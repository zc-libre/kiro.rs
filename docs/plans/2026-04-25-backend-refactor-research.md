# Research: kiro-rs 后端整体重构（2026-04-25）

> 研究范围限定为 `src/` 中除 `admin_ui/` 之外的全部 Rust 代码（前端面板 `admin-ui/` 不动）。

## Problem Statement

`kiro-rs` 是一个 **Anthropic Claude API ↔ Kiro / AWS CodeWhisperer 的协议转换代理**：客户端按 Anthropic 格式发请求，后端选择凭据 → 调上游 → 解 AWS Event Stream → 转回 Anthropic SSE。

当前在 `refactor/v2` 分支，仓主希望对后端整个重写——重新设计 framework / 解析 / 请求 / 管理四大块——以彻底解决以下系统性问题：

- **模块耦合严重**：解析 / 请求 / 管理互相依赖，改一处牵连一片，边界不清
- **扩展新协议 / 源困难**：增加协议解析或订阅源类型时需在多处改动
- **并发 / 状态管理混乱**：异步任务、共享状态、配置所有权不清晰
- **代码重复 / 调用链过长**：相似逻辑重复实现，函数嵌套层次过多

## Requirements

| 项 | 要求 |
| - | - |
| 兼容契约 | **必须保持**：① `config.json` / `credentials.json` 格式；② 前后端 HTTP API（`/v1`、`/cc/v1`、`/api/admin/*`、`/admin`）。 |
| 自由度 | 内部模块边界、抽象、错误体系、文件组织 全部可推翻。 |
| 起点 | `refactor/v2` 刚从 master 切出，未做实质改动，可整体重写。 |
| 节奏 | **一次性完成**——整体重构后单次合入主干。 |
| 工程 | 遵守 KISS / YAGNI / SOLID / DRY；优先复用项目现有亮点（Endpoint trait、EventStream Decoder、Credential 单测）。 |

## Findings

### 1. 项目地图（约 14 100 行 Rust，36 个 .rs 文件）

```
src/
├── main.rs                       219    手动 wire 一切（Args/Config/Cred/Endpoints/Provider/Routes）
├── http_client.rs                117    reqwest 工厂 + ProxyConfig（数据简单）
├── token.rs                      245    count_tokens（含全局静态 + sync 中跑 async）
├── debug.rs / test.rs                   调试与测试
├── common/auth.rs                 41    API Key 提取 + 常量时间比较
├── model/
│   ├── arg.rs                     14    clap Args
│   └── config.rs                 242    扁平 Config（30+ 字段一把抓）
├── anthropic/                            对外 Anthropic 兼容层
│   ├── router.rs                  73    /v1, /cc/v1
│   ├── middleware.rs              78    AppState (api_key + Provider + extract_thinking)
│   ├── handlers.rs               938    ⚠ post_messages / post_messages_cc 双胞胎
│   ├── converter.rs            1 777    ⚠ 顶层函数堆砌，无 trait 抽象
│   ├── stream.rs               1 989    ⚠ 三个 Context + thinking 字符串扫描
│   ├── websearch.rs              761    WebSearch 工具特化路径
│   └── types.rs                  283    DTO（含 system 字段 string|array 自定义解码）
├── kiro/                                 上游领域核心
│   ├── provider.rs               519    ⚠ call_api_with_retry / call_mcp_with_retry 双胞胎
│   ├── token_manager.rs        2 599    ⚠⚠ 单文件巨石，1 376 行 impl
│   ├── machine_id.rs             282    含全局静态 fallback 缓存
│   ├── endpoint/                         ✅ 已有 trait 抽象
│   │   ├── mod.rs                133    KiroEndpoint trait + RequestContext
│   │   └── ide.rs                169    唯一实现
│   ├── parser/                           ✅ 设计较完整
│   │   ├── decoder.rs            337    四态状态机 + 容错恢复
│   │   ├── frame.rs / header.rs / crc.rs / error.rs
│   │   └── mod.rs                 10
│   └── model/
│       ├── credentials.rs        873    ✅ 数据模型 + 大量单测
│       ├── token_refresh.rs       46
│       ├── usage_limits.rs       202
│       ├── events/{base,assistant,tool_use,context_usage}.rs
│       └── requests/{conversation,kiro,tool}.rs
├── admin/                                管理 API
│   ├── router.rs                  56    10 个端点
│   ├── handlers.rs               142    薄 HTTP 包装
│   ├── service.rs                457    ⚠ 4 个 classify_*_error 用字符串匹配
│   ├── types.rs                  259    DTO
│   ├── middleware.rs              50    Admin Key 鉴权
│   └── error.rs                   64    ✅ 结构化错误 enum + 状态码映射
└── admin_ui/                             嵌入 SPA（不在重构范围）
    └── router.rs                 109    rust-embed + SPA fallback
```

### 2. 数据流（按一次 `POST /v1/messages` 流式请求）

```
Client
  │ Anthropic 请求体 (JSON)
  ▼
anthropic::auth_middleware  ── x-api-key / Bearer
  ▼
anthropic::handlers::post_messages
  │  override_thinking_from_model_name()
  │  has_web_search_tool()  ──→ websearch::handle_websearch_request (旁路)
  │  convert_request()      ──→ converter.rs 顶层函数群
  │  serde_json::to_string(KiroRequest)
  │  token::count_all_tokens()  ⚠ block_in_place + block_on
  ▼
KiroProvider::call_api_stream
  │  call_api_with_retry(stream=true)        ── 重试上限 9 次
  │   ├── token_manager.acquire_context()    ── 选凭据 + 刷 token + 自愈
  │   │   ├── select_next_credential()       ── balanced / priority
  │   │   ├── try_ensure_token()             ── 双重检查锁定 + refresh
  │   │   └── refresh_social_token / refresh_idc_token
  │   ├── endpoint_for() / client_for()      ── client 缓存（按 ProxyConfig 维度）
  │   ├── endpoint.transform_api_body()      ── inject profileArn
  │   ├── endpoint.decorate_api()            ── headers (UA / x-amz-* / Authorization)
  │   └── request.send().await
  │
  │  状态码分类（200 / 400 / 401|403 / 402+monthly / 408|429|5xx / 其他）
  │  报告：report_success / report_failure / report_quota_exhausted
  │  失效则 force_refresh_token_for + 重试
  ▼
StreamContext::new_with_thinking → generate_initial_events
create_sse_stream (stream::unfold + tokio::select!)
  │ 每 chunk: EventStreamDecoder.feed → decode_iter → Event::from_frame
  │           StreamContext.process_kiro_event() → Vec<SseEvent>
  │ 每 25 s: ping
  ▼
Anthropic SSE 流回 Client
```

`/cc/v1/messages` 的差异仅在最后一段：用 `BufferedStreamContext` 缓冲全部事件，等到流结束（拿到 `contextUsageEvent`）再批量发 `message_start`，确保 `input_tokens` 准确。前 700+ 行的处理代码与 `/v1` 几乎一模一样。

### 3. 复杂度爆点（按严重程度）

#### 爆点 A · `MultiTokenManager` 巨石（2 599 行 / 1 376 行 impl）

`src/kiro/token_manager.rs:510` `pub struct MultiTokenManager` 一个对象包揽 5 类职责：

| 职责 | 关键方法 |
| - | - |
| 凭据池查询 | `total_count`, `available_count`, `snapshot`, `cache_dir`, `config` |
| 选凭据 | `acquire_context`, `select_next_credential`, `switch_to_next`, `select_highest_priority` |
| 状态记录 | `report_success`, `report_failure`, `report_quota_exhausted`, `report_refresh_failure`, `report_refresh_token_invalid` |
| Token 刷新 | `try_ensure_token`, `force_refresh_token_for` + 模块级 `refresh_token` / `refresh_social_token` / `refresh_idc_token` |
| 凭据增删改 | `add_credential`, `delete_credential`, `set_disabled`, `set_priority`, `reset_and_enable`, `set_load_balancing_mode` |
| 元数据查询 | `get_usage_limits_for`, `get_load_balancing_mode` |
| 持久化 | `persist_credentials`, `load_stats` / `save_stats` (统计) + `persist_load_balancing_mode` |

锁体系：`parking_lot::Mutex<Vec<CredentialEntry>>` + `parking_lot::Mutex<u64> current_id` + `parking_lot::Mutex<String> load_balancing_mode` + `parking_lot::Mutex<Option<Instant>> last_stats_save_at` + `tokio::Mutex<()> refresh_lock` + `AtomicBool stats_dirty`——5 把锁混用，并且 `acquire_context` 内出现 *"获取 entries 锁后再 bail，要小心 available_count 二次取锁会死锁"* 的注释（见 `:822` 行），说明锁的边界已经成为认知负担。

`refresh_social_token` (`:142`) 与 `refresh_idc_token` (`:225`) 在 HTTP 构造、错误判定、字段更新上大量重复。

#### 爆点 B · `KiroProvider` 双胞胎（519 行）

`src/kiro/provider.rs`：

- `call_mcp_with_retry` (`:129`) ≈ 140 行
- `call_api_with_retry` (`:279`) ≈ 200 行

两者结构几乎完全一致：尝试上限 → `acquire_context` → endpoint → request → 状态码分类 → 报告 → 退避。差异仅在 `endpoint.api_url` vs `endpoint.mcp_url`、`transform_api_body` vs `transform_mcp_body`。

状态码到行为的策略硬编码在长长的 `if status.as_u16() == ... { ... }` 链里，扩展新策略需改这一大段。错误识别靠 `endpoint.is_monthly_request_limit(body)` 等基于响应体字符串匹配。

#### 爆点 C · Handler 双胞胎 + SSE 闭包

`src/anthropic/handlers.rs`：

- `post_messages` (`:178`) ≈ 120 行
- `post_messages_cc` (`:690`) ≈ 120 行

差异点几乎仅有最后一段 `handle_stream_request` ↔ `handle_stream_request_buffered`。WebSearch 旁路 / thinking 覆写 / 序列化 / token 估算 全部在 handler 自己。

`create_sse_stream` (`:345`) 用 `stream::unfold + tokio::select!` 把"读 chunk / 解码 / 处理事件 / 发 ping"塞进闭包，状态机难以独立测试或复用。

错误映射 `map_provider_error` (`:31`) 用字符串 `err_str.contains("CONTENT_LENGTH_EXCEEDS_THRESHOLD")` 识别上游错误——与上游错误格式强耦合。

#### 爆点 D · `converter.rs` (1 777) 与 `stream.rs` (1 989)

- **converter.rs**：纯顶层函数堆砌——`normalize_*`, `extract_*`, `is_*`, `collect_*`, `create_*`, `convert_*`, `process_*`, `validate_*`, `remove_*`, `shorten_*`, `map_*`, `build_*`, `merge_*`、`generate_*`、`has_*`——共 24 个 fn，没有 trait/struct 把它们组织起来，调用链很难追。
- **stream.rs**：三个状态结构 `SseStateManager` (`:279`) / `StreamContext` (`:513`) / `BufferedStreamContext` (`:1142`)，外加 5 个 thinking 标签字符串扫描函数（`find_real_thinking_*`, `is_quote_char`, `find_char_boundary`）。`StreamContext::xx` impl 跨 595 行。事件状态机和字符串扫描状态机混在同一文件。

#### 爆点 E · Admin 服务的字符串匹配错误

`src/admin/service.rs` 4 个 `classify_*_error` (`:370` / `:380` / `:418` / `:447`) 全部依赖 `msg.contains("不存在")` / `msg.contains("error trying to connect")` / `msg.contains("已被截断")`——把错误分类的契约绑在 anyhow 字符串上。**而 `admin/error.rs` 自己用 thiserror 风格的 enum 已经做对了**——本质问题是 token_manager / provider 那一层没有暴露结构化错误，迫使 admin 这一层做反向解析。

#### 爆点 F · 跨切面隐患

| 位置 | 问题 |
| - | - |
| `token.rs:33` `OnceLock<CountTokensConfig>` | 全局静态注入的 count_tokens 配置 |
| `token.rs:118` `block_in_place + block_on` | 在 axum sync handler 路径里同步阻塞跑 async，会卡住 worker 线程 |
| `machine_id.rs:17` `OnceLock<Mutex<HashMap>>` | 全局静态 fallback 缓存 |
| `model/config.rs:23` `Config` | 30+ 字段平铺一层（HTTP/Region/Token/Proxy/Endpoint/外部 API/Admin 全混） |
| `provider.rs:39` `client_cache: Mutex<HashMap<Option<ProxyConfig>, Client>>` | 缓存 key 是整个 ProxyConfig，正确但侵入式 |

### 4. 现有可复用的"亮点"

| 文件 | 价值 |
| - | - |
| `src/kiro/endpoint/mod.rs` | `KiroEndpoint` trait + `RequestContext<'a>` 是已有的合理抽象，新增端点（如 cli/codeWhisperer-future）只要实现 trait |
| `src/kiro/parser/decoder.rs` | 四态状态机 + 容错恢复 + 单测，独立性好，重构时整体保留 |
| `src/kiro/model/credentials.rs` | `KiroCredentials` 数据模型（含 30+ 单元测试）和 `effective_*` 多级回退方法逻辑清晰，可直接保留 |
| `src/http_client.rs` | `ProxyConfig` + `build_client` 工厂，正交简单，可直接保留 |
| `src/common/auth.rs` | API Key 提取 + 常量时间比较，正交简单 |
| `src/admin/error.rs` | `AdminServiceError` enum + `status_code` / `into_response` 是结构化错误的范本 |

### 5. 兼容性契约清单（重构必须满足）

#### HTTP API

```
GET    /v1/models
POST   /v1/messages
POST   /v1/messages/count_tokens
POST   /cc/v1/messages
POST   /cc/v1/messages/count_tokens

GET    /api/admin/credentials
POST   /api/admin/credentials
DELETE /api/admin/credentials/:id
POST   /api/admin/credentials/:id/disabled
POST   /api/admin/credentials/:id/priority
POST   /api/admin/credentials/:id/reset
POST   /api/admin/credentials/:id/refresh
GET    /api/admin/credentials/:id/balance
GET    /api/admin/config/load-balancing
PUT    /api/admin/config/load-balancing

GET    /admin                    (SPA + 静态资源 fallback)
GET    /admin/{*file}
```

请求体 / 响应体的 JSON 结构、字段命名（camelCase）、错误 envelope 格式（`{"error": {"type", "message"}}`）必须保持。

#### 配置文件

- `config.json`：见 `model/config.rs` Config 全部 camelCase 字段（30+），`endpoints: HashMap<String, serde_json::Value>` 的端点扩展点。
- `credentials.json`：单对象 / 数组双格式（`CredentialsConfig::Single | Multiple` untagged），凭据字段 23 个（含 OAuth、IdC、API Key、Region、代理、endpoint），`priority=0` 时不序列化等行为。
- 配置文件运行时回写：① 凭据补 ID/machineId 后回写；② 多凭据 token 刷新后回写；③ 负载均衡模式变更回写到 config.json；④ 余额缓存回写到 cache_dir。

#### 上游契约（不可变）

- AWS CodeWhisperer `generateAssistantResponse` 端点 + AWS Event Stream 协议
- Kiro Auth `https://prod.{region}.auth.desktop.kiro.dev/refreshToken`（Social）
- AWS SSO OIDC `https://oidc.{region}.amazonaws.com/token`（IdC）
- AWS `getUsageLimits` 端点
- 各端点的 User-Agent / x-amz-* header 形式

### 6. 依赖（外部 crate）

`axum 0.8` · `tokio 1` · `reqwest 0.12 (rustls-tls + native-tls 可选)` · `tower-http 0.6` · `serde 1` / `serde_json 1` · `tracing` / `tracing-subscriber` · `anyhow 1` · `parking_lot 0.12` · `subtle 2.6` · `bytes 1` · `futures 0.3` · `chrono 0.4` · `uuid 1.10` · `fastrand 2` · `sha2 0.10` · `hex 0.4` · `crc 3` · `urlencoding 2` · `clap 4.5` · `rust-embed 8` · `mime_guess 2`

---

## Open Questions（重构启动前需要对齐）

1. **错误体系策略**——`anyhow::Error` 全栈贯穿好，还是分层用 `thiserror` 派生结构化错误（`KiroError` / `TokenError` / `RefreshError` / `ProviderError`）？我的初步判断是后者，因为爆点 E 完全是字符串识别错误的结果。
2. **Provider / TokenManager 是否合并**——两者目前耦合极强（Provider 持有 `Arc<MultiTokenManager>` 并调用 7+ 方法）。重构时是合并到一个新的 `KiroClient`（领域服务）还是仍保留二层结构？
3. **count_tokens 的同步阻塞如何处理**——彻底改成 `async fn count_all_tokens(...)`，让 handler `.await`？还是把"远程 API 模式"做成可选 trait 实现，本地模式仍保持同步？
4. **配置结构**——是否打散 `Config` 为 `NetConfig / RegionConfig / KiroConfig / TokenCountConfig / ProxyConfig / AdminConfig`，再用 `#[serde(flatten)]` 保持 JSON 兼容？还是为了更稳妥保持平铺，仅做 helper method（`net()`, `region()`, ...）？
5. **统一/分离 RetryPolicy**——爆点 B 的 200 行重复，是抽象成一个 `RetryPolicy` trait 给 Provider 注入，还是直接抽到一个 `RequestExecutor::execute_with_retry(req: F, policy: Policy)` 通用函数？
6. **流式状态机**——`StreamContext` / `BufferedStreamContext` / `SseStateManager` 是否合并为单个 `EventReducer`，"buffered vs streaming" 改为外层 SSE Sink 的两种 collect 策略？
7. **凭据池层级**——是否把 `MultiTokenManager` 拆为四个对象：`CredentialStore`（持久化 + 增删改）、`CredentialSelector`（trait + Priority/Balanced 实现）、`CredentialState`（失败计数、自愈）、`TokenRefresher`（trait + Social/Idc/ApiKey 三策略）？

---

## Recommendations（候选重构方向，待对齐）

> 这些是**初步候选**，不是决定。需要根据上面的 Open Questions 对齐后才能定型。

### R1 · 分层（Hexagonal-style）

```
src/
├── main.rs                   入口（解析 args + 启动 wiring）
├── config/                   配置（含 v2 子结构 + 兼容旧 JSON）
├── domain/                   核心抽象（trait/types）
│   ├── credential.rs         Credential 数据 + 角色
│   ├── token.rs              TokenSource / RefreshOutcome
│   ├── endpoint.rs           KiroEndpoint trait（保留现有）
│   ├── retry.rs              RetryPolicy + RetryDecision
│   ├── error.rs              结构化错误层级
│   └── conversation.rs       Anthropic ↔ Kiro 协议中间表示
├── infra/                    实现
│   ├── http/                 reqwest client 工厂、ProxyConfig
│   ├── parser/               AWS Event Stream（保留）
│   ├── refresher/            social/idc/api_key 三种 TokenRefresher 实现
│   ├── storage/              凭据 / 统计 / 余额缓存的 file-backed 仓储
│   └── endpoint/             Kiro 端点实现（保留 ide）
├── service/                  用例编排
│   ├── credential_pool.rs    Store + Selector + State 组合
│   ├── kiro_client.rs        executor + retry + endpoint + refresher
│   ├── conversation.rs       request 转换、event 归约
│   └── admin.rs              管理用例
└── interface/
    ├── http/
    │   ├── anthropic/        v1 + cc/v1 路由 + handler + DTO
    │   ├── admin/            管理路由 + handler + DTO
    │   └── ui/               admin_ui 嵌入（保留）
    └── auth.rs
```

### R2 · 拆分 `MultiTokenManager`

| 新组件 | 职责 | 依赖 |
| - | - | - |
| `CredentialStore` | 增删改、持久化（凭据 / 统计 / 负载均衡）、加载 | `infra::storage` |
| `CredentialSelector` (trait) | 选凭据，实现：`PrioritySelector`, `BalancedSelector` | 只读 store |
| `CredentialState` | 失败计数、禁用、自愈逻辑 | store |
| `TokenRefresher` (trait) | `refresh(&Credential) -> RefreshOutcome`，实现：`SocialRefresher`, `IdcRefresher`, `ApiKeyPassthrough` | `infra::http` |
| `CredentialPool` | 把上面四个组合成 `acquire_context` / `report_*` 的 facade | 全部 |

### R3 · 拆分 `KiroProvider`

```
KiroClient
  ├── RequestExecutor          通用 send + 退避 + 故障转移（不区分 api/mcp）
  ├── RetryPolicy (trait)      状态码 → RetryDecision { Retry / FailoverCredential / DisableCredential / ForceRefresh / Fail }
  ├── KiroEndpoint (保留)
  └── CredentialPool (R2)
```

`call_api` / `call_mcp` 不再各写一套，而是 `executor.execute(EndpointKind::Api, body)` / `executor.execute(EndpointKind::Mcp, body)`。

### R4 · Anthropic 协议层

| 新组件 | 职责 |
| - | - |
| `ProtocolConverter` | `AnthropicRequest → KiroRequest` 转换；把 converter.rs 的 24 个 fn 按职责切到 `tools.rs` / `messages.rs` / `thinking.rs` 子模块 |
| `EventReducer` | 接收 `kiro::Event`，产出 `AnthropicSseEvent`；替代 `StreamContext` |
| `ThinkingExtractor` | thinking 标签状态机，独立可测，stream 与 non-stream 共用 |
| `SseDelivery` (trait) | `Live` 实现立即推送、`Buffered` 实现等结束后批量推送；替代双胞胎 handler |

### R5 · 错误体系

```rust
// crate-level
pub enum KiroError { Http(...), Endpoint(...), Quota(...), Auth(...), Refresh(RefreshError), ... }
pub enum RefreshError { TokenInvalid, RateLimited, Network(...), ... }
pub enum ProviderError { AllCredentialsExhausted, ContextWindowFull, InputTooLong, ... }
```

`AdminServiceError` 直接 `From<ProviderError>` / `From<KiroError>`，丢掉所有 `msg.contains(...)`。

### R6 · 全局静态消除

- `CountTokensConfig` 走 `AppState`（构造时注入），count_tokens 改 `async fn`
- `FALLBACK_MACHINE_IDS` 移到 `CredentialPool` 持有的字段
- `block_in_place` 整个删掉

### R7 · Config 分组（serde flatten 保兼容）

```rust
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(flatten)] pub net: NetConfig,
    #[serde(flatten)] pub region: RegionConfig,
    #[serde(flatten)] pub kiro: KiroIdentity,
    #[serde(flatten)] pub proxy: ProxyConfig,
    #[serde(flatten)] pub admin: AdminConfig,
    #[serde(flatten)] pub count_tokens: CountTokensConfig,
    #[serde(flatten)] pub endpoint: EndpointConfig,
    #[serde(flatten)] pub features: FeatureFlags,
    #[serde(default)] pub endpoints: HashMap<String, serde_json::Value>,
}
```

JSON 仍是单层平铺，但代码侧职责清晰，`Config::region.effective_auth_region()` 等方法语义自然。

---

## Next Step

Open Questions 是否能先回答 1-3 个关键的（**问题 1 错误体系 + 问题 2 Provider/Manager 边界 + 问题 7 凭据池拆分**）？这三个对齐后，就能输出一份可执行的实施计划（`docs/plans/2026-04-25-backend-refactor-plan.md`），按 R1-R7 的方向落地。
