# Plan: 401/403 凭据故障转移语义回归修复 (2026-04-23)

## Summary

重构 `refactor/kiro-endpoint` 分支将原本位于 `call_api_with_retry` / `call_mcp_with_retry` 的 `if matches!(status, 401 | 403)` 独立分支枚举化为 `EndpointErrorKind`，但**遗漏了无 bearer 标记 401/403 的故障转移路径**：当前这类响应落入 `ClientError` arm 直接 `bail!`，完全绕过 `report_failure`。本计划按用户批准的**方案 A**，新增 `EndpointErrorKind::Unauthorized` 变体，在 `classify_error` 默认实现中为无 bearer 标记的 401/403 返回该变体，并在 `provider.rs` match 中新增对应 arm（复用旧版行为：`warn!` → `report_failure` → continue），同步修订相关单元测试与 plan.md 漂移表述。

## Stakes Classification

**Level**: Medium

**Rationale**：
- 改动核心错误处理路径（`classify_error` 决策树 + `call_with_retry` match），影响所有 Kiro 请求（API/MCP/UsageLimits）
- 改动面较小：1 个 enum 变体 + 1 个 match arm + 3~4 个单元测试；调用链单点（provider 只有一处 `match endpoint.classify_error(...)`）
- 类型系统（Rust enum 非穷尽匹配检查）保证不会漏改 match arm
- 无外部接口变更，无数据迁移，回滚成本低

## Context

**Research**: [docs/plans/2026-04-23-401-403-failover-regression-research.md](./2026-04-23-401-403-failover-regression-research.md)

**Affected Areas**:
- `src/kiro/endpoint/mod.rs`：`EndpointErrorKind` 枚举、`classify_error` 默认实现、相关单元测试
- `src/kiro/provider.rs`：`call_with_retry` 的 match 分支
- `docs/plans/2026-04-18-kiro-endpoint-layered-refactor-plan.md`：一处描述性表述修订

**不涉及**：
- `src/kiro/token_manager.rs`（`report_failure`、failure_count 阈值逻辑不变）
- 其他端点实现（ide.rs 未 override `classify_error`，自动继承修复）
- `force_refresh_token_for` 内部实现
- `MAX_FAILURES_PER_CREDENTIAL` 阈值

## Success Criteria

- [ ] `classify_error(401, "{}")` 返回 `EndpointErrorKind::Unauthorized`
- [ ] `classify_error(403, "{}")` 返回 `EndpointErrorKind::Unauthorized`
- [ ] `classify_error(401, bearer-标记-body)` 仍返回 `EndpointErrorKind::BearerTokenInvalid`（无回归）
- [ ] `classify_error(403, bearer-标记-body)` 仍返回 `EndpointErrorKind::BearerTokenInvalid`（无回归）
- [ ] `classify_error(404, "{}")` 仍返回 `EndpointErrorKind::ClientError`（无回归）
- [ ] `classify_error(402, "{}")` 仍返回 `EndpointErrorKind::ClientError`（无回归）
- [ ] provider 的 match 语句新增 `Unauthorized` arm，语义：`warn!` → `report_failure(ctx.id)` → 若无可用凭据则 bail，否则 continue
- [ ] provider 的 `Unauthorized` arm **不**包含 `force_refresh` 逻辑（该路径已由 `BearerTokenInvalid` arm 覆盖）
- [ ] `cargo build` 通过
- [ ] `cargo test` 全部通过
- [ ] `cargo clippy --all-targets`（若项目使用）无新增警告
- [ ] plan.md:170 漂移表述已修订
- [ ] 分支顺序合理：`Unauthorized` 放在 `BearerTokenInvalid` 之后、`Transient` 之前（与旧版"401/403 独立分支"位置对应）

## 前置状态验证

改动前必须确认：

- [ ] 当前分支：`refactor/kiro-endpoint`（`git branch --show-current`）
- [ ] 工作区干净或仅有可接受的未跟踪文件（`git status`）
- [ ] 基线编译通过：`cargo build`
- [ ] 基线测试通过：`cargo test`（特别是现有 9 个 `test_classify_error_*`）
- [ ] 研究文档存在：`docs/plans/2026-04-23-401-403-failover-regression-research.md`

## Implementation Steps

### Phase 1：扩展 `EndpointErrorKind` 枚举（endpoint 层分类）

#### Step 1.1：新增 `Unauthorized` 变体定义

- **Files**: `src/kiro/endpoint/mod.rs:105-119`
- **Action**: 在 `enum EndpointErrorKind` 中新增 `Unauthorized` 变体，附文档注释与 `BearerTokenInvalid` 形成清晰区分
- **Before**（mod.rs:105-119，精确现状）：
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
- **After**：
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum EndpointErrorKind {
      /// 402 + 月度配额用尽标记：禁用凭据 + 故障转移
      MonthlyQuotaExhausted,
      /// 401/403 + bearer token 失效标记：强制刷新 token（每凭据一次机会）
      BearerTokenInvalid,
      /// 401/403 无 bearer 失效标记：凭据/权限问题（如 IAM 拒绝、profile_arn
      /// 授权不足、订阅不匹配等）。计入 `report_failure`（累计达阈值禁用凭据），
      /// 然后故障转移到下一个可用凭据。与 `BearerTokenInvalid` 的区别：
      /// 后者是 token 本身失效（可尝试 force_refresh 恢复），前者是凭据
      /// 无法获得对应资源的访问权限（刷新 token 无意义）。
      Unauthorized,
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
- **Verify**: `cargo build` 预期**失败**（provider.rs match 将因 non-exhaustive 报错），`cargo check --lib` 报 `E0004: non-exhaustive patterns: \`EndpointErrorKind::Unauthorized\` not covered`（这是预期信号，将在 Phase 2 修复）
- **Complexity**: Small

#### Step 1.2：在 `classify_error` 默认实现中为无标记 401/403 返回 `Unauthorized`

- **Files**: `src/kiro/endpoint/mod.rs:146-163`
- **Action**: 在 `matches!(status, 401 | 403) && default_is_bearer_token_invalid(body)` 分支之后、`status == 400` 分支之前，新增对 `401 | 403` 的独立分支，返回 `Unauthorized`
- **Before**（mod.rs:146-163，精确现状）：
  ```rust
  fn classify_error(&self, status: u16, body: &str) -> EndpointErrorKind {
      if status == 402 && default_is_monthly_request_limit(body) {
          return EndpointErrorKind::MonthlyQuotaExhausted;
      }
      if matches!(status, 401 | 403) && default_is_bearer_token_invalid(body) {
          return EndpointErrorKind::BearerTokenInvalid;
      }
      if status == 400 {
          return EndpointErrorKind::BadRequest;
      }
      if matches!(status, 408 | 429) || (500..600).contains(&status) {
          return EndpointErrorKind::Transient;
      }
      if (400..500).contains(&status) {
          return EndpointErrorKind::ClientError;
      }
      EndpointErrorKind::Unknown
  }
  ```
- **After**：
  ```rust
  fn classify_error(&self, status: u16, body: &str) -> EndpointErrorKind {
      if status == 402 && default_is_monthly_request_limit(body) {
          return EndpointErrorKind::MonthlyQuotaExhausted;
      }
      if matches!(status, 401 | 403) && default_is_bearer_token_invalid(body) {
          return EndpointErrorKind::BearerTokenInvalid;
      }
      if matches!(status, 401 | 403) {
          return EndpointErrorKind::Unauthorized;
      }
      if status == 400 {
          return EndpointErrorKind::BadRequest;
      }
      if matches!(status, 408 | 429) || (500..600).contains(&status) {
          return EndpointErrorKind::Transient;
      }
      if (400..500).contains(&status) {
          return EndpointErrorKind::ClientError;
      }
      EndpointErrorKind::Unknown
  }
  ```
- **Verify**: `cargo check --lib`（仍会在 provider.rs 报 non-exhaustive；endpoint/mod.rs 本身无新错误）
- **Complexity**: Small

### Phase 2：provider 消费端新增 `Unauthorized` arm

#### Step 2.1：在 `call_with_retry` match 中新增 `Unauthorized` arm

- **Files**: `src/kiro/provider.rs:210-325`
- **Action**: 在 `EndpointErrorKind::BearerTokenInvalid` arm 之后（即原 mod.rs 中的 `BearerTokenInvalid => { ... continue; }` 块结束后）、`EndpointErrorKind::Transient` arm 之前，新增 `EndpointErrorKind::Unauthorized` arm。语义参考旧版 `call_api_with_retry` 的 401/403 块（研究文档 §2.1），但**不重复** force_refresh（该路径已在 `BearerTokenInvalid` arm 处理）。
- **新增位置**：`src/kiro/provider.rs:280` 附近（`BearerTokenInvalid` arm 的 `continue;` 之后，`EndpointErrorKind::Transient => {` 之前）
- **新增代码**（与现有分支的 `kind_label` / `last_error` / `force_refreshed` 命名保持一致）：
  ```rust
  EndpointErrorKind::Unauthorized => {
      tracing::warn!(
          "{}失败（可能为凭据错误，尝试 {}/{}）: {} {}",
          kind_label,
          attempt + 1,
          max_retries,
          status,
          body
      );

      let has_available = self.token_manager.report_failure(ctx.id);
      if !has_available {
          anyhow::bail!(
              "{}失败（所有凭据已用尽）: {} {}",
              kind_label,
              status,
              body
          );
      }

      last_error = Some(anyhow::anyhow!(
          "{}失败: {} {}",
          kind_label,
          status,
          body
      ));
      continue;
  }
  ```
- **注意事项**：
  1. 日志文案"可能为凭据错误"与 `BearerTokenInvalid` arm（provider.rs:244）完全一致；旧版即用同一措辞，复用保持语义稳定
  2. **不**包含 `force_refreshed.insert` / `force_refresh_token_for` 调用：无 bearer 标记意味着 token 本身并非被上游失效，刷新 token 无意义；与旧版行为一致（旧版 force_refresh 包裹在 `if endpoint.is_bearer_token_invalid(&body) && !force_refreshed.contains(&ctx.id)` 内部，在新版枚举化后这条路径由 `BearerTokenInvalid` arm 专属处理）
  3. arm 顺序：`MonthlyQuotaExhausted → BadRequest → BearerTokenInvalid → Unauthorized → Transient → ClientError → Unknown`（新 arm 紧随 `BearerTokenInvalid`，与旧版"401/403 独立分支"在循环中的位置对应）
- **Verify**:
  - `cargo build` 通过（不再有 non-exhaustive 警告/错误）
  - `cargo test --lib kiro::provider` 通过（即使无新测试也应编译）
  - `cargo clippy --all-targets` 无新警告
- **Complexity**: Small

### Phase 3：单元测试修改与新增（endpoint 层）

#### Step 3.1：修改 `test_classify_error_401_without_marker_is_client_error` → `..._is_unauthorized`

- **Files**: `src/kiro/endpoint/mod.rs:318-326`
- **Action**: 该测试当前锁定了回归行为（期望 `ClientError`）。将函数名重命名为 `test_classify_error_401_without_marker_is_unauthorized`、注释与断言同步更新
- **Before**（mod.rs:318-326，精确现状）：
  ```rust
  #[test]
  fn test_classify_error_401_without_marker_is_client_error() {
      // 401 但 body 不含 bearer 失效标记 → 走普通 4xx arm
      let probe = ProbeEndpoint;
      assert_eq!(
          probe.classify_error(401, "{}"),
          EndpointErrorKind::ClientError
      );
  }
  ```
- **After**：
  ```rust
  #[test]
  fn test_classify_error_401_without_marker_is_unauthorized() {
      // 401 但 body 不含 bearer 失效标记 → 凭据/权限问题，走 Unauthorized（触发 report_failure）
      let probe = ProbeEndpoint;
      assert_eq!(
          probe.classify_error(401, "{}"),
          EndpointErrorKind::Unauthorized
      );
  }
  ```
- **Test case**: `(401, "{}")` → `Unauthorized`
- **Verify**: `cargo test --lib kiro::endpoint::tests::test_classify_error_401_without_marker_is_unauthorized` 通过
- **Complexity**: Small

#### Step 3.2：新增 `test_classify_error_403_without_marker_is_unauthorized`

- **Files**: `src/kiro/endpoint/mod.rs`（与 Step 3.1 相邻，紧随其后）
- **Action**: 新增与 401 对称的 403 测试，填补研究文档 §4.2 指出的"缺少 403 无标记测试"缺口
- **新增代码**：
  ```rust
  #[test]
  fn test_classify_error_403_without_marker_is_unauthorized() {
      // 403 但 body 不含 bearer 失效标记 → 凭据/权限问题，走 Unauthorized（触发 report_failure）
      let probe = ProbeEndpoint;
      assert_eq!(
          probe.classify_error(403, "{}"),
          EndpointErrorKind::Unauthorized
      );
  }
  ```
- **Test case**: `(403, "{}")` → `Unauthorized`
- **Verify**: `cargo test --lib kiro::endpoint::tests::test_classify_error_403_without_marker_is_unauthorized` 通过
- **Complexity**: Small

#### Step 3.3：新增 `test_classify_error_401_with_iam_body_is_unauthorized`

- **Files**: `src/kiro/endpoint/mod.rs`（与 Step 3.2 相邻）
- **Action**: 用真实 AWS 风格的拒绝 body 验证决策树：body 含 `message` 但不含 `The bearer token included in the request is invalid` 标记时应为 `Unauthorized`，防止未来误调用 `default_is_bearer_token_invalid` 逻辑时回归
- **新增代码**：
  ```rust
  #[test]
  fn test_classify_error_401_with_iam_body_is_unauthorized() {
      // body 有业务错误消息但不含 bearer 失效标记 → 仍走 Unauthorized
      let probe = ProbeEndpoint;
      let body = r#"{"message":"User is not authorized to perform this action"}"#;
      assert_eq!(
          probe.classify_error(401, body),
          EndpointErrorKind::Unauthorized
      );
  }
  ```
- **Test case**: `(401, r#"{"message":"User is not authorized to perform this action"}"#)` → `Unauthorized`
- **Verify**: `cargo test --lib kiro::endpoint::tests::test_classify_error_401_with_iam_body_is_unauthorized` 通过
- **Complexity**: Small

#### Step 3.4：更新 `test_endpoint_error_kind_debug` 覆盖新变体

- **Files**: `src/kiro/endpoint/mod.rs:419-427`
- **Action**: 在现有的 Debug 断言中新增 `Unauthorized` 分支
- **Before**（mod.rs:419-427，精确现状）：
  ```rust
  #[test]
  fn test_endpoint_error_kind_debug() {
      assert!(format!("{:?}", EndpointErrorKind::MonthlyQuotaExhausted).contains("Monthly"));
      assert!(format!("{:?}", EndpointErrorKind::BearerTokenInvalid).contains("Bearer"));
      assert!(format!("{:?}", EndpointErrorKind::BadRequest).contains("Bad"));
      assert!(format!("{:?}", EndpointErrorKind::ClientError).contains("Client"));
      assert!(format!("{:?}", EndpointErrorKind::Transient).contains("Transient"));
      assert!(format!("{:?}", EndpointErrorKind::Unknown).contains("Unknown"));
  }
  ```
- **After**：
  ```rust
  #[test]
  fn test_endpoint_error_kind_debug() {
      assert!(format!("{:?}", EndpointErrorKind::MonthlyQuotaExhausted).contains("Monthly"));
      assert!(format!("{:?}", EndpointErrorKind::BearerTokenInvalid).contains("Bearer"));
      assert!(format!("{:?}", EndpointErrorKind::Unauthorized).contains("Unauthorized"));
      assert!(format!("{:?}", EndpointErrorKind::BadRequest).contains("Bad"));
      assert!(format!("{:?}", EndpointErrorKind::ClientError).contains("Client"));
      assert!(format!("{:?}", EndpointErrorKind::Transient).contains("Transient"));
      assert!(format!("{:?}", EndpointErrorKind::Unknown).contains("Unknown"));
  }
  ```
- **Verify**: `cargo test --lib kiro::endpoint::tests::test_endpoint_error_kind_debug` 通过
- **Complexity**: Small

#### Step 3.5：整体回归

- **Files**: N/A
- **Action**: 运行全部 endpoint 层测试
- **Verify**: `cargo test --lib kiro::endpoint` 全部通过；`test_classify_error_402_without_marker_is_client_error`（mod.rs:300-307）、`test_classify_error_404_is_client_error`（mod.rs:309-316）、`test_classify_error_bearer_token_invalid_401/403`（mod.rs:261-278）无回归
- **Complexity**: Small

### Phase 4：provider 层集成测试（可选，视基础设施而定）

#### Step 4.1：评估 provider 层测试基础设施

- **Files**: 读取 `src/kiro/provider.rs` 末尾 `#[cfg(test)] mod tests`（若存在）
- **Action**: 检查 provider.rs 是否已有 `#[cfg(test)] mod tests` 或 mock `KiroEndpoint`/`TokenManager` 的工具。研究文档 §4.2 指出"provider 层集成测试完全缺失"
- **Decision**：
  - 若**存在** mock 基础设施：继续 Step 4.2 新增端到端测试
  - 若**不存在**：跳过 Step 4.2（遵循 YAGNI，不为此修复引入新 mock 框架），在 provider.rs 新增 arm 处加行内注释说明"此路径暂无单元测试覆盖，依赖 endpoint 层 classify_error 测试 + 人工走查"
- **Verify**: 明确决定后记录到实施报告
- **Complexity**: Small

#### Step 4.2：（条件性）新增 `Unauthorized → report_failure` 端到端测试

- **前置**: Step 4.1 决定继续
- **Files**: `src/kiro/provider.rs`（新增测试模块）或新文件
- **Action**: 构造 mock endpoint 返回 `classify_error → Unauthorized`，mock token_manager 计数 `report_failure` 调用次数；断言调用次数 == 重试次数（而非 0）
- **Test cases**:
  - `401 无标记 body` → `report_failure` 被调用 且 **未** `bail`（有可用凭据情况）
  - `401 无标记 body` 且 单凭据耗尽 → `report_failure` 被调用 且 `bail`
  - `403 无标记 body` → 与 401 对称
- **Verify**: `cargo test --lib kiro::provider` 通过新测试
- **Complexity**: Medium（受 mock 基础设施复杂度影响）
- **YAGNI 提示**：若项目实施成本高，本步骤可延后至后续工作项

### Phase 5：修订 plan.md 漂移

#### Step 5.1：更新 2026-04-18 plan.md:170 表述

- **Files**: `docs/plans/2026-04-18-kiro-endpoint-layered-refactor-plan.md:170`
- **Action**: 将单行表述扩展为多行，点明重构前后两种顺序，并标注当前已识别的回归问题
- **Before**（plan.md:170，精确现状）：
  ```
  4. **错误分支顺序**：402 → 400 → 401/403 → 瞬态 → 其他 4xx → 兜底，顺序一致
  ```
- **After**（参见研究文档 §7 修订建议，并根据修复后真实顺序同步更新为 7 个分支）：
  ```
  4. **错误分支顺序（重构前）**：402 → 400 → 401/403 → 瞬态 → 其他 4xx → 兜底，两循环顺序一致。
     重构后枚举化为：MonthlyQuotaExhausted → BadRequest → BearerTokenInvalid → Unauthorized → Transient → ClientError → Unknown。
     注意：原"401/403"分支在枚举化时曾被拆分导致无标记 401/403 落入 ClientError 回归（见研究报告 2026-04-23），已通过新增 Unauthorized 变体修复。
  ```
- **Verify**: 人工核对 diff，确保表述准确且保留"可合并项"原意
- **Complexity**: Small

### Phase 6：最终验证

#### Step 6.1：完整构建与测试

- **Files**: N/A
- **Action**: 依次执行：
  1. `cargo build`
  2. `cargo test`
  3. `cargo clippy --all-targets`（若项目有 clippy 配置）
- **Verify**:
  - `cargo build`：无错误无警告（至少不引入新警告）
  - `cargo test`：所有测试通过，尤其包括 9 个 `test_classify_error_*`、`test_endpoint_error_kind_debug` 和新增的 2~3 个测试
  - `cargo clippy`：无新警告
- **Complexity**: Small

#### Step 6.2：手工走查

- **Files**: `src/kiro/provider.rs:210-325`（修改后全量）、`src/kiro/endpoint/mod.rs:105-163`
- **Action**: 对照研究文档 §1.3 行为表与 §2.1 旧版逻辑，逐 arm 核对：
  - [ ] `Unauthorized` arm 调用 `report_failure`（而非 `report_quota_exhausted`）
  - [ ] `Unauthorized` arm 无 `force_refresh_token_for` 调用
  - [ ] `Unauthorized` arm 在 `BearerTokenInvalid` 之后、`Transient` 之前
  - [ ] 所有 arm 的 `kind_label` / `attempt+1` / `max_retries` / `last_error = Some(anyhow::anyhow!(...))` 模式保持一致
  - [ ] `classify_error` 决策树顺序：`402+标记 → 401/403+标记 → 401/403 → 400 → 408/429/5xx → 4xx → Unknown`
  - [ ] mod.rs `EndpointErrorKind` 文档注释明确区分 `BearerTokenInvalid` 与 `Unauthorized`
- **Verify**: checklist 全勾
- **Complexity**: Small

## Test Strategy

### Automated Tests

| Test Case | Type | Input | Expected Output |
|---|---|---|---|
| `test_classify_error_401_without_marker_is_unauthorized`（修改） | Unit | `classify_error(401, "{}")` | `EndpointErrorKind::Unauthorized` |
| `test_classify_error_403_without_marker_is_unauthorized`（新增） | Unit | `classify_error(403, "{}")` | `EndpointErrorKind::Unauthorized` |
| `test_classify_error_401_with_iam_body_is_unauthorized`（新增） | Unit | `classify_error(401, r#"{"message":"User is not authorized..."}"#)` | `EndpointErrorKind::Unauthorized` |
| `test_classify_error_bearer_token_invalid_401`（现有，回归） | Unit | `classify_error(401, "The bearer token included in the request is invalid")` | `EndpointErrorKind::BearerTokenInvalid` |
| `test_classify_error_bearer_token_invalid_403`（现有，回归） | Unit | `classify_error(403, "The bearer token included in the request is invalid")` | `EndpointErrorKind::BearerTokenInvalid` |
| `test_classify_error_402_without_marker_is_client_error`（现有，回归） | Unit | `classify_error(402, "{}")` | `EndpointErrorKind::ClientError` |
| `test_classify_error_404_is_client_error`（现有，回归） | Unit | `classify_error(404, "{}")` | `EndpointErrorKind::ClientError` |
| `test_classify_error_monthly_quota_exhausted`（现有，回归） | Unit | `classify_error(402, {"reason":"MONTHLY_REQUEST_COUNT"})` | `EndpointErrorKind::MonthlyQuotaExhausted` |
| `test_classify_error_transient_429_500`（现有，回归） | Unit | `classify_error({408,429,500,502,599}, "{}")` | `EndpointErrorKind::Transient` |
| `test_classify_error_bad_request`（现有，回归） | Unit | `classify_error(400, "{}")` | `EndpointErrorKind::BadRequest` |
| `test_classify_error_200_is_unknown`（现有，回归） | Unit | `classify_error(200, "{}")` | `EndpointErrorKind::Unknown` |
| `test_endpoint_error_kind_debug`（扩展） | Unit | `format!("{:?}", EndpointErrorKind::Unauthorized)` | 包含 `"Unauthorized"` |
| （条件性）provider-层 `Unauthorized → report_failure` | Integration | 401 无标记响应 | `report_failure` 被调用 ≥1 次，无 bail |

### Manual Verification

- [ ] `git diff src/kiro/endpoint/mod.rs` 仅包含：`Unauthorized` 变体 + `classify_error` 新分支 + 2 个新测试 + 1 个测试重命名 + `test_endpoint_error_kind_debug` 扩展
- [ ] `git diff src/kiro/provider.rs` 仅包含：新 match arm（约 20 行），无其他意外改动
- [ ] `git diff docs/plans/2026-04-18-kiro-endpoint-layered-refactor-plan.md` 仅第 170 行附近表述修订
- [ ] 阅读 `Unauthorized` 变体的文档注释，确保与 `BearerTokenInvalid` 在语义上清晰区分
- [ ] 对照旧版 `call_api_with_retry` 401/403 块（研究文档 §2.1），确认新 arm 的日志文案、`report_failure` 调用、`last_error` 赋值、`continue` 均对齐

## Risks and Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| `Unauthorized` arm 与 `BearerTokenInvalid` 语义重叠导致未来维护者混淆 | 中：可能在新端点 override `classify_error` 时选错变体 | `EndpointErrorKind` 枚举注释明确区分：有 bearer 标记→token 失效（可刷新）；无标记→权限问题（直接 report_failure） |
| arm 顺序错位（`Unauthorized` 放错位置） | 低：枚举 match 无顺序依赖，但影响代码阅读性 | 手工走查（Step 6.2）确认顺序；代码审查关注点明确列出 |
| `report_failure` 累计计数与旧版不一致 | 低：本计划不修改 token_manager | 不改动 `report_failure` 与 `MAX_FAILURES_PER_CREDENTIAL`（=3）；Step 6.2 核对日志文案确保触发频次一致 |
| 其他端点（如未来可能新增的 `cli.rs`）override `classify_error` 后未覆盖 `Unauthorized` | 低：Rust 编译器 non-exhaustive 检查兜底 | 依赖类型系统；若新端点 override，编译器会强制处理所有变体 |
| provider 层集成测试缺口遗留 | 中：未来可能再发生类似回归无法被单测捕获 | Step 4.1 显式评估；若基础设施不足，记录到后续工作项，不强行引入 |
| plan.md 修订影响历史归档计划的完整性 | 极低 | 仅添加"重构后"与"回归已修复"两段说明，保留原"重构前"表述 |

## Rollback Strategy

- **未 commit 状态**：`git checkout -- src/kiro/endpoint/mod.rs src/kiro/provider.rs docs/plans/2026-04-18-kiro-endpoint-layered-refactor-plan.md`
- **已 commit 状态**：`git revert <commit-sha>`（本修复预期 1~2 个 commit，revert 成本低）
- **部分阶段完成**：`git stash` 暂存当前进度后回滚至上一个稳定 commit
- **回归判据**：若 Step 6.1 `cargo test` 出现任何 `test_classify_error_*` 现有测试失败（非 Step 3.1 修改的那个），立即回滚并重新检视 Phase 1~2 改动

## 代码审查关注点

1. **`Unauthorized` 变体文档注释**：是否清晰说明与 `BearerTokenInvalid` 的区别？（token 失效 vs 权限问题）
2. **provider.rs `Unauthorized` arm**：
   - tracing 日志文案是否与旧版 `call_api_with_retry` 401/403 块一致（"可能为凭据错误"）？
   - 是否**无** `force_refresh` 调用？（force_refresh 专属于 `BearerTokenInvalid`）
   - `report_failure` 调用 + `has_available` 判定 + `last_error` 赋值 + `continue` 四段式是否完整？
3. **arm 顺序**：`BearerTokenInvalid → Unauthorized → Transient`，与旧版"401/403 独立分支"位置对应
4. **新增测试命名**：`test_classify_error_<status>_<body_case>_is_<kind>` 与现有命名风格一致
5. **plan.md 漂移修订**：是否保留"重构前顺序一致"的原意，同时标注"重构后 + 回归已修复"？
6. **决策树顺序**：`classify_error` 中 `Unauthorized` 分支必须在 `BearerTokenInvalid` 之后（否则带 bearer 标记的 401/403 会被错判为 `Unauthorized`，绕过 force_refresh）——这是正确性关键，审查必查

## Status

- [ ] Plan approved
- [ ] Implementation started
- [ ] Implementation complete
