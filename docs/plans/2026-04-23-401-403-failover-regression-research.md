# 401/403 凭据故障转移语义回归 - 研究报告

> 研究日期：2026-04-23  
> 研究分支：refactor/kiro-endpoint  
> 对照基线：master（commit 35a7c93）

---

## 1. 当前实现（新版）

### 1.1 EndpointErrorKind 变体定义

**文件**：`src/kiro/endpoint/mod.rs:105-119`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointErrorKind {
    /// 402 + 月度配额用尽标记：禁用凭据 + 故障转移
    MonthlyQuotaExhausted,
    /// 401/403 + bearer token 失效标记：强制刷新 token（每凭据一次机会）
    BearerTokenInvalid,
    /// 400 Bad Request：直接 bail，不重试、不计入失败
    BadRequest,
    /// 其他未分类 4xx（经 401/402/403 特殊分支处理后的剩余 4xx）：bail
    ClientError,
    /// 408/429/5xx：瞬态错误，sleep + 重试，不禁用
    Transient,
    /// 兜底：当作可重试瞬态错误
    Unknown,
}
```

注意：共 6 个变体，**没有覆盖"无标记 401/403"的专用变体**。

### 1.2 classify_error 默认实现（决策树）

**文件**：`src/kiro/endpoint/mod.rs:146-163`

```
classify_error(status, body):
  if status==402 && body 含 MONTHLY_REQUEST_COUNT 标记
      → MonthlyQuotaExhausted
  if status∈{401,403} && body 含 "bearer token included in the request is invalid"
      → BearerTokenInvalid
  if status==400
      → BadRequest
  if status∈{408,429} || status∈[500,600)
      → Transient
  if status∈[400,500)     ← 此处兜底 4xx，包括无标记的 401/403/402
      → ClientError
  else
      → Unknown
```

关键路径：无标记的 401（如 `{}`）先检查 bearer 标记失败，然后落入 `(400..500).contains(&status)` → 返回 `ClientError`。

### 1.3 provider match 分支行为表

**文件**：`src/kiro/provider.rs:210-325`

| EndpointErrorKind | provider 行为 | report_failure？ | bail？ | continue？ |
|---|---|---|---|---|
| `MonthlyQuotaExhausted` | report_quota_exhausted + 故障转移 | 否（走 quota 路径） | 若无可用凭据则 bail | 有可用凭据则 continue |
| `BearerTokenInvalid` | 尝试 force_refresh → 若失败则 report_failure | 是（force_refresh 失败后） | 若无可用凭据则 bail | 有可用凭据则 continue |
| `BadRequest` | 直接 bail | 否 | 是 | 否 |
| `ClientError` | **直接 bail**（`anyhow::bail!`） | **否** | **是** | **否** |
| `Transient` | sleep + continue | 否 | 否 | 是 |
| `Unknown` | sleep + continue | 否 | 否 | 是 |

**关键问题**：`ClientError` arm 直接 bail，完全绕过 `report_failure` 和凭据轮转。

---

## 2. 旧 master 行为（重构前）

**基线 commit**：`35a7c93`（master 分支最后一个重构前 commit：`refactor: 抽象 Kiro 端点 trait，支持按凭据切换 ide/cli`）

### 2.1 call_api_with_retry 中的 401/403 处理

**文件**（旧版）：`src/kiro/provider.rs`（从 `git show 35a7c93:src/kiro/provider.rs` 读取）

旧版 call_api_with_retry 对 401/403 的处理（完整代码块）：

```rust
// 401/403 - 更可能是凭据/权限问题：计入失败并允许故障转移
if matches!(status.as_u16(), 401 | 403) {
    tracing::warn!(
        "API 请求失败（可能为凭据错误，尝试 {}/{}）: {} {}",
        attempt + 1,
        max_retries,
        status,
        body
    );

    // token 被上游失效：先尝试 force-refresh，每凭据仅一次机会
    if endpoint.is_bearer_token_invalid(&body) && !force_refreshed.contains(&ctx.id) {
        force_refreshed.insert(ctx.id);
        tracing::info!("凭据 #{} token 疑似被上游失效，尝试强制刷新", ctx.id);
        if self.token_manager.force_refresh_token_for(ctx.id).await.is_ok() {
            tracing::info!("凭据 #{} token 强制刷新成功，重试请求", ctx.id);
            continue;
        }
        tracing::warn!("凭据 #{} token 强制刷新失败，计入失败", ctx.id);
    }

    let has_available = self.token_manager.report_failure(ctx.id);
    if !has_available {
        anyhow::bail!(
            "{} API 请求失败（所有凭据已用尽）: {} {}",
            api_type,
            status,
            body
        );
    }

    last_error = Some(anyhow::anyhow!(
        "{} API 请求失败: {} {}",
        api_type,
        status,
        body
    ));
    continue;
}
```

### 2.2 call_mcp_with_retry 中的 401/403 处理（旧版）

旧版 call_mcp_with_retry 对 401/403 的处理与 call_api_with_retry 完全对称：

```rust
// 401/403 凭据问题
if matches!(status.as_u16(), 401 | 403) {
    // token 被上游失效：先尝试 force-refresh，每凭据仅一次机会
    if endpoint.is_bearer_token_invalid(&body) && !force_refreshed.contains(&ctx.id) {
        force_refreshed.insert(ctx.id);
        // ... force_refresh 逻辑
    }

    let has_available = self.token_manager.report_failure(ctx.id);
    if !has_available {
        anyhow::bail!("MCP 请求失败（所有凭据已用尽）: {} {}", status, body);
    }
    last_error = Some(anyhow::anyhow!("MCP 请求失败: {} {}", status, body));
    continue;
}
```

### 2.3 旧版决策逻辑摘要

旧版用简单的 `if matches!(status.as_u16(), 401 | 403)` 检查，**先**于 `status.is_client_error()` 兜底分支。逻辑：

1. **收到 401/403**（无论 body 内容）：
   - 若 body 含 bearer 失效标记 且 尚未 force_refresh 过 → 先尝试强制刷新 token；刷新成功则 continue
   - 无论 bearer 标记与否，最终都调用 `report_failure(ctx.id)`
   - `report_failure` 内部：累计 failure_count；达到阈值（`MAX_FAILURES_PER_CREDENTIAL`，即 3 次）则禁用凭据并切换

2. **凭据禁用计数器位置**：`src/kiro/token_manager.rs:1147-1198`，字段 `entry.failure_count`，阈值 3 次。

---

## 3. 行为差异对照

| HTTP 状态 | body 场景 | 旧版行为（35a7c93） | 新版行为（当前） | 预期行为 |
|---|---|---|---|---|
| 401 | 含 bearer 标记 | force_refresh 尝试；失败则 report_failure + continue | BearerTokenInvalid → force_refresh 尝试；失败则 report_failure + continue | 与旧版一致 ✓ |
| 401 | **不含 bearer 标记**（如 IAM 拒绝、订阅不匹配） | report_failure + continue（累计 3 次禁用） | **ClientError → bail！绕过 report_failure** | 应 report_failure + continue |
| 403 | 含 bearer 标记 | force_refresh 尝试；失败则 report_failure + continue | BearerTokenInvalid → force_refresh 尝试；失败则 report_failure + continue | 与旧版一致 ✓ |
| 403 | **不含 bearer 标记**（如 profile_arn 授权不足） | report_failure + continue（累计 3 次禁用） | **ClientError → bail！绕过 report_failure** | 应 report_failure + continue |
| 402 | 含 MONTHLY_REQUEST_COUNT | report_quota_exhausted + continue | MonthlyQuotaExhausted → report_quota_exhausted + continue | 与旧版一致 ✓ |
| 402 | 不含标记 | is_client_error() → bail! | ClientError → bail! | 一致 ✓（非额度问题，bail 合理） |
| 400 | 任意 | bail! | BadRequest → bail! | 与旧版一致 ✓ |
| 404 | 任意 | is_client_error() → bail! | ClientError → bail! | 与旧版一致 ✓ |
| 429 | 任意 | sleep + continue | Transient → sleep + continue | 与旧版一致 ✓ |
| 500 | 任意 | sleep + continue | Transient → sleep + continue | 与旧版一致 ✓ |

**回归场景汇总**：`401/403 + 无 bearer 标记 body` → 旧版故障转移（report_failure），新版直接 bail（ClientError）。

---

## 4. 测试现状

### 4.1 已有测试清单

**文件**：`src/kiro/endpoint/mod.rs`（测试在同文件 `#[cfg(test)] mod tests` 内）

| 测试函数名 | 行号 | 断言内容 |
|---|---|---|
| `test_classify_error_monthly_quota_exhausted` | 252-258 | `classify_error(402, body含标记)` → `MonthlyQuotaExhausted` |
| `test_classify_error_bearer_token_invalid_401` | 262-268 | `classify_error(401, bearer标记body)` → `BearerTokenInvalid` |
| `test_classify_error_bearer_token_invalid_403` | 272-278 | `classify_error(403, bearer标记body)` → `BearerTokenInvalid` |
| `test_classify_error_bad_request` | 282-287 | `classify_error(400, "{}")` → `BadRequest` |
| `test_classify_error_transient_429_500` | 291-298 | `classify_error({429,408,500,502,599}, "{}")` → `Transient` |
| `test_classify_error_402_without_marker_is_client_error` | 301-307 | `classify_error(402, "{}")` → `ClientError` |
| `test_classify_error_404_is_client_error` | 310-316 | `classify_error(404, "{}")` → `ClientError` |
| **`test_classify_error_401_without_marker_is_client_error`** | **319-326** | **`classify_error(401, "{}")` → `ClientError`**（锁定了回归行为） |
| `test_classify_error_200_is_unknown` | 329-332 | `classify_error(200, "{}")` → `Unknown` |

### 4.2 测试缺口

1. **`test_classify_error_401_without_marker_is_client_error`（mod.rs:319-326）**：这个测试反向锁定了回归行为——它断言 `401 + 无标记` 应返回 `ClientError`，但正确行为是应触发 `report_failure`（相当于一个新变体或 provider 层特殊处理）。修复后此测试必须同步修改。

2. **缺少 403 无标记的测试**：目前没有 `classify_error(403, "{}")` 对应的测试。旧版该场景走 report_failure，新版走 ClientError→bail，同样是回归。

3. **provider 层集成测试完全缺失**：`src/kiro/provider.rs` 没有任何测试覆盖 classify_error 分类到 provider match 分支的端到端行为，无法在单元测试层检测 `ClientError → bail` 绕过 `report_failure` 的问题。

4. **无标记 401/403 对应的凭据隔离行为未测试**：token_manager.rs 有 `report_failure` 的单元测试（2229-2250 行），但没有测试"provider 收到无标记 401 后是否调用 report_failure"这条链路。

---

## 5. 调用链

### 5.1 classify_error 调用方

**唯一调用点**：`src/kiro/provider.rs:210`

```rust
match endpoint.classify_error(status.as_u16(), &body) {
```

注：`src/admin/service.rs:113,126,133` 有 `self.classify_error(...)` 调用，但这是 `AdminService` 自己的私有方法（`src/admin/service.rs:370`），与 `KiroEndpoint::classify_error` **不同**，不相关。

### 5.2 EndpointErrorKind 消费方

**唯一消费点**：`src/kiro/provider.rs:211-324`，在 `call_with_retry` 的 match 语句中：

- `MonthlyQuotaExhausted` → 210 行 match → 211 行 arm
- `BadRequest` → 239 行 arm
- `BearerTokenInvalid` → 242 行 arm
- `Transient` → 281 行 arm
- `ClientError` → 301 行 arm
- `Unknown` → 304 行 arm

`EndpointErrorKind` 未被 `src/kiro/endpoint/ide.rs` 或其他文件直接消费（ide.rs 没有 override `classify_error`，使用 trait 默认实现）。

---

## 6. 修复方案分析

### 方案 A：新增 EndpointErrorKind::Unauthorized 变体

**改动范围**：

1. `src/kiro/endpoint/mod.rs`：
   - 在 `EndpointErrorKind` 枚举新增变体 `Unauthorized`（或 `CredentialFailure`），附文档说明语义
   - `classify_error` 默认实现：在检查 bearer 标记失败后、进入 `(400..500)` 兜底前，对 `401 | 403` 返回 `Unauthorized`：
     ```rust
     if matches!(status, 401 | 403) {
         return EndpointErrorKind::Unauthorized;
     }
     ```
   - 修改 `test_classify_error_401_without_marker_is_client_error`：期望改为 `Unauthorized`
   - 新增 `test_classify_error_403_without_marker_is_unauthorized`

2. `src/kiro/provider.rs`：
   - 在 match 语句新增 `EndpointErrorKind::Unauthorized` arm，语义与旧版 401/403 块一致：report_failure + continue

3. `src/kiro/endpoint/mod.rs`（Debug 测试）：
   - `test_endpoint_error_kind_debug`：新增 `Unauthorized` 变体的 Debug 断言

**优点**：
- 类型安全：`Unauthorized` 变体明确表达"无标记 401/403"的语义，match 枚举不完整时编译器会报错
- 关注点分离清晰：分类（endpoint 层）与处置（provider 层）各司其职
- 未来端点可 override `classify_error` 返回 `Unauthorized` 而 provider 行为自动对齐

**缺点**：
- 改动面略大：需要改 enum 定义 + classify_error + provider match + 3 处测试
- `BearerTokenInvalid` 与 `Unauthorized` 在语义上有重叠（都是 401/403），需要在文档中明确区分（有标记走 BearerTokenInvalid，无标记走 Unauthorized）

### 方案 B：provider ClientError arm 内对 401/403 兜底

在 `src/kiro/provider.rs:301-303` 的 `ClientError` arm 内加判断：

```rust
EndpointErrorKind::ClientError => {
    // 兜底：无标记 401/403 也走凭据失败路径（与旧版行为一致）
    // 注：有标记的 401/403 已由 BearerTokenInvalid arm 处理
    if matches!(status.as_u16(), 401 | 403) {
        let has_available = self.token_manager.report_failure(ctx.id);
        if !has_available {
            anyhow::bail!("{}失败（所有凭据已用尽）: {} {}", kind_label, status, body);
        }
        last_error = Some(anyhow::anyhow!("{}失败: {} {}", kind_label, status, body));
        continue;
    }
    anyhow::bail!("{}失败: {} {}", kind_label, status, body);
}
```

**改动范围**：
1. `src/kiro/provider.rs`：仅修改 `ClientError` arm
2. 测试：`test_classify_error_401_without_marker_is_client_error` 不需要修改（`classify_error` 行为不变）；但需要新增 provider 层集成测试验证 `401 + ClientError` 正确调用 report_failure

**优点**：
- 改动极小，只触动 provider.rs 的 7 行
- `classify_error` 语义不变，`test_classify_error_401_without_marker_is_client_error` 无需修改
- 修复风险最低，回归面最小

**缺点**：
- 架构上有漏洞：`ClientError` arm 内部检查 status 违背了分层原则（endpoint 层负责分类，provider 层负责处置，方案 B 让 provider 再做一次状态码判断）
- `status` 变量在 match 块内仍可访问（`let status = response.status()` 在 match 之前），技术上可行，但属于"为修复漏洞而绕过抽象"的模式
- 不利于未来新端点扩展：新端点若有自己的 401/403 语义（非凭据问题），无法通过 override `classify_error` 来控制 provider 行为

### 推荐方案 + 理由

**推荐方案 A（新增 `EndpointErrorKind::Unauthorized` 变体）**。

理由：
1. 该重构的核心目标就是让 endpoint 层负责分类、provider 层负责处置——方案 B 在 provider 里二次检查 status 是对该目标的妥协
2. 改动量虽略多，但每处改动都是明确、自解释的；enum exhaustive match 保证未来不会再有遗漏
3. 旧版 401/403 处理有独立的 `tracing::warn!`（"可能为凭据错误"），方案 A 可以在 `Unauthorized` arm 保留该日志，语义更准确
4. `BearerTokenInvalid` 与 `Unauthorized` 的区分在文档和变体名称上都清晰：有标记=token 失效（force_refresh 有意义），无标记=权限/订阅问题（直接 report_failure）

---

## 7. plan.md 漂移

### 具体表述与行号

**文件**：`docs/plans/2026-04-18-kiro-endpoint-layered-refactor-plan.md:170`

```
4. **错误分支顺序**：402 → 400 → 401/403 → 瞬态 → 其他 4xx → 兜底，顺序一致
```

该行位于"设计讨论：API/MCP 重试循环合并（决策 5）"→"可合并项"小节，原意是描述重构前两个循环（MCP/API）可合并的共同特征，作为合并决策的依据。

### 漂移说明

该表述描述的是**重构前**两个循环"错误分支顺序一致"（因此可合并）。但重构后（阶段 4.1+4.2，commit `6f13323`），`call_with_retry` 的分支顺序已经变成：

```
MonthlyQuotaExhausted → BadRequest → BearerTokenInvalid → Transient → ClientError → Unknown
```

**转换关系**：
- 原 `402 → 400 → 401/403 → 瞬态 → 其他4xx → 兜底`
- 新 `MonthlyQuotaExhausted → BadRequest → BearerTokenInvalid → Transient → ClientError → Unknown`

在枚举化之后，"401/403"已不再是一个独立分支——有标记的 401/403 走 `BearerTokenInvalid`，无标记的 401/403 落入 `ClientError`（这正是回归所在）。

### 修订建议

plan.md:170 应修订为：

```
4. **错误分支顺序（重构前）**：402 → 400 → 401/403 → 瞬态 → 其他 4xx → 兜底，两循环顺序一致。
   重构后枚举化为：MonthlyQuotaExhausted → BadRequest → BearerTokenInvalid → Transient → ClientError → Unknown。
   注意：原"401/403"分支在枚举化时被拆分，无标记的 401/403 落入 ClientError，存在行为回归（见研究报告 2026-04-23）。
```

---

## 附录：关键文件路径索引

| 文件 | 关键行 | 内容 |
|---|---|---|
| `src/kiro/endpoint/mod.rs` | 105-119 | `EndpointErrorKind` 枚举定义 |
| `src/kiro/endpoint/mod.rs` | 146-163 | `classify_error` 默认实现（决策树） |
| `src/kiro/endpoint/mod.rs` | 206-209 | `default_is_bearer_token_invalid` 标记检测 |
| `src/kiro/endpoint/mod.rs` | 319-326 | `test_classify_error_401_without_marker_is_client_error`（需修改） |
| `src/kiro/provider.rs` | 210-325 | `call_with_retry` match 分支（新版） |
| `src/kiro/provider.rs` | 301-303 | `ClientError => bail!`（回归点） |
| `src/kiro/token_manager.rs` | 1147-1198 | `report_failure` 实现（含计数器和禁用逻辑） |
| `docs/plans/2026-04-18-kiro-endpoint-layered-refactor-plan.md` | 170 | 漂移表述"错误分支顺序一致" |
