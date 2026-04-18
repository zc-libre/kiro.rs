# Kiro 端点分层重构 —— 综合研究报告

研究日期：2026-04-18  
研究范围：协议层（KiroEndpoint trait）、路由层（endpoint 注册表）、调度层（KiroProvider / MultiTokenManager）的现状梳理与重构影响  
目标受众：规划阶段决策者（无需阅读原始两份研究文档即可理解）

---

## 一、重构目标（引用）

本重构遵循三层分离架构，目标如下：

### 1.1 协议层 — KiroEndpoint trait 核心化

**核心接口简化**：当前 10 个分散方法（URL 返回、header 装饰、body 变换、错误判断）折叠为两个方法 + 两个数据枚举：

```rust
pub trait KiroEndpoint: Send + Sync {
    fn name(&self) -> &'static str;
    
    fn build_request(
        &self,
        client: &reqwest::Client,
        ctx: &RequestContext<'_>,
        req: &KiroRequest<'_>,
    ) -> anyhow::Result<RequestBuilder>;
    
    fn classify_error(
        &self,
        status: u16,
        body: &str,
    ) -> EndpointErrorKind;
}
```

**新增枚举**：

```rust
pub enum KiroRequest<'a> {
    GenerateAssistant { body: &'a str, stream: bool },
    Mcp { body: &'a str },
    UsageLimits,
}

pub enum EndpointErrorKind {
    MonthlyQuotaExhausted,
    BearerTokenInvalid,
    BadRequest,
    ClientError,
    Transient,
    Unknown,
}
```

**RequestContext 简化**：删除 `machine_id` 字段（由 endpoint 内部调用 `machine_id::generate_from_credentials`），保留：

```rust
pub struct RequestContext<'a> {
    pub credentials: &'a KiroCredentials,
    pub token: &'a str,
    pub config: &'a Config,
}
```

### 1.2 路由层 — 单一注册表 EndpointRegistry

```rust
pub struct EndpointRegistry {
    endpoints: HashMap<String, Arc<dyn KiroEndpoint>>,
    default: String,
}

impl EndpointRegistry {
    pub fn new(
        endpoints: HashMap<String, Arc<dyn KiroEndpoint>>,
        default: String,
    ) -> anyhow::Result<Self>;
    
    pub fn resolve(&self, credentials: &KiroCredentials) -> Arc<dyn KiroEndpoint>;
    pub fn contains(&self, name: &str) -> bool;
    pub fn names(&self) -> Vec<&str>;
}
```

**作用**：替代三处副本（main.rs、KiroProvider、AdminService），提供统一的端点解析与验证。

### 1.3 调度层 — CallContext 增强 + 重试循环简化

**CallContext 新增字段**：

```rust
pub struct CallContext {
    pub id: u64,
    pub credentials: KiroCredentials,
    pub token: String,
    pub endpoint: Arc<dyn KiroEndpoint>,  // ← 新增
}
```

**MultiTokenManager 改造**：
- 持有 `Arc<EndpointRegistry>`
- `acquire_context()` / `acquire_context_for()` 在返回 CallContext 时同时解析并注入 endpoint 字段

**KiroProvider 改造**：
- 丢弃 endpoints HashMap，仅保留 client_cache + 重试循环
- 重试循环核心逻辑简化为：`ctx.endpoint.build_request(...).send() → ctx.endpoint.classify_error(...)`
- 两个 `call_api_with_retry` / `call_mcp_with_retry` 方法可合并为泛型方法

**get_usage_limits_for 改造**：
- 改为 `acquire_context_for(id) → build_request(UsageLimits) → send().json()`
- 消除硬编码 IDE 格式，支持多端点额度获取

---

## 二、当前代码事实清单

### 2.1 协议层现状

#### 2.1.1 KiroEndpoint trait 定义

**文件位置**：`src/kiro/endpoint/mod.rs` 第 14-45 行

**当前方法集合**（10 个）：
- `name(&self) -> &'static str` — 端点标识符
- `api_url(&self, ctx: &RequestContext) -> String` — API 端点 URL
- `mcp_url(&self, ctx: &RequestContext) -> String` — MCP 端点 URL
- `decorate_api(&self, req: RequestBuilder, ctx) -> RequestBuilder` — API 请求装饰（header）
- `decorate_mcp(&self, req: RequestBuilder, ctx) -> RequestBuilder` — MCP 请求装饰
- `transform_api_body(&self, body: &str, ctx) -> String` — API body 变换
- `transform_mcp_body(&self, body: &str, ctx) -> String` — MCP body 变换（默认实现）
- `is_monthly_request_limit(&self, body: &str) -> bool` — 月度配额判断（默认实现）
- `is_bearer_token_invalid(&self, body: &str) -> bool` — Bearer token 失效判断（默认实现）

**特点**：无 `Self::associate_type`；装饰和转换均基于 `&str` body；错误判断依赖响应文本解析

#### 2.1.2 RequestContext 定义与构造

**定义位置**：`src/kiro/endpoint/mod.rs` 第 62-71 行

```rust
pub struct RequestContext<'a> {
    pub credentials: &'a KiroCredentials,
    pub token: &'a str,
    pub machine_id: &'a str,        // ← 当前包含，重构时删除
    pub config: &'a Config,
}
```

**构造点**（两处）：
1. **API 调用**：`src/kiro/provider.rs` 第 305-310 行（`call_api_with_retry` 内）
2. **MCP 调用**：`src/kiro/provider.rs` 第 158-163 行（`call_mcp_with_retry` 内）

**machine_id 来源**：
```rust
let machine_id = machine_id::generate_from_credentials(&ctx.credentials, config);
```
调用在 provider.rs 第 146 行和 304 行；每次重试都重新生成一次

#### 2.1.3 IDE 实现 (IdeEndpoint)

**文件位置**：`src/kiro/endpoint/ide.rs` 第 57-111 行

**关键实现**：
- `name()` 返回常量 `IDE_ENDPOINT_NAME = "ide"`（第 15 行）
- `api_url()` 返回 `https://q.{api_region}.amazonaws.com/generateAssistantResponse`
- `mcp_url()` 返回 `https://q.{api_region}.amazonaws.com/mcp`
- `decorate_api()` 添加 9 个头部（x-amzn-codewhisperer-optout、x-amz-user-agent、user-agent、host、amz-sdk-invocation-id、amz-sdk-request、Authorization、tokentype）
- `decorate_mcp()` 添加 6 个头部（x-amz-user-agent、user-agent、host、amz-sdk-invocation-id、amz-sdk-request、Authorization、x-amzn-kiro-profile-arn、tokentype）
- `transform_api_body()` 调用 `inject_profile_arn()` 将 profile_arn 注入 JSON 根对象
- `transform_mcp_body()` 不覆盖，使用默认实现（直接返回）

#### 2.1.4 CLI 实现

**存在性**：不存在

**证据**：
- 文件系统扫描：`src/kiro/endpoint/` 仅含 `mod.rs` 和 `ide.rs`
- `impl KiroEndpoint` 全局搜索仅返回 1 处（ide.rs）
- mod.rs 文档注释提到 "如 `ide` / `cli`" 为潜在候选，非实现事实

**待确认**：最近提交 `35a7c93 refactor: 抽象 Kiro 端点 trait，支持按凭据切换 ide/cli` 的评论与实际代码不符；是否该提交计划包含 cli 但尚未开始实现

#### 2.1.5 辅助函数

**default_is_monthly_request_limit()**（mod.rs 第 76-97 行）：
- 查找字符串 "MONTHLY_REQUEST_COUNT" 或 JSON 字段 `reason == "MONTHLY_REQUEST_COUNT"`
- 支持顶层 `reason` 和嵌套 `error.reason` 两种格式

**default_is_bearer_token_invalid()**（mod.rs 第 100-102 行）：
- 查找字符串 "The bearer token included in the request is invalid"

**inject_profile_arn()**（ide.rs 第 114-124 行）：
- 将 profile_arn 注入请求体 JSON 根对象
- profile_arn 为 None 或 JSON 解析失败时返回原 body

#### 2.1.6 machine_id 模块

**文件位置**：`src/kiro/machine_id.rs`

**主函数 generate_from_credentials()**（第 54-86 行）：
```rust
pub fn generate_from_credentials(credentials: &KiroCredentials, config: &Config) -> String
```

**派生优先级**（顺序）：
1. 凭据级 `machineId`（若配置且合法）
2. 全局 `config.machineId`（若配置且合法）
3. 根据凭据类型派生：API Key `sha256("KiroAPIKey/" + key)`，OAuth `sha256("KotlinNativeAPI/" + refreshToken)`
4. 兜底缓存：`sha256("KiroFallback/" + uuid)`（按凭据 ID 进程内缓存）

**调用成本**：
- 第 1-2 级：廉价（直接返回）
- 第 3 级：中等（SHA256 哈希）
- 第 4 级：廉价但依赖缓存（首次随机，后续复用）

**调用点**：provider.rs 第 304 行（API）、第 146 行（MCP）

### 2.2 路由层现状（散落状态）

#### 2.2.1 副本 1：main.rs 端点注册表

**文件位置**：`src/main.rs` 第 100-127 行

```rust
let mut endpoints: HashMap<String, Arc<dyn KiroEndpoint>> = HashMap::new();
{
    let ide = IdeEndpoint::new();
    endpoints.insert(ide.name().to_string(), Arc::new(ide));
}

// 校验默认端点存在
if !endpoints.contains_key(&config.default_endpoint) {
    tracing::error!("默认端点 \"{}\" 未注册", config.default_endpoint);
    std::process::exit(1);
}

// 校验所有凭据声明的端点都已注册
for cred in &credentials_list {
    let name = cred
        .endpoint
        .as_deref()
        .unwrap_or(&config.default_endpoint);
    if !endpoints.contains_key(name) {
        tracing::error!(
            "凭据 id={:?} 指定了未知端点 \"{}\"（已注册: {:?}）",
            cred.id, name,
            endpoints.keys().collect::<Vec<_>>()
        );
        std::process::exit(1);
    }
}

let endpoint_names: Vec<String> = endpoints.keys().cloned().collect();
```

**特点**：
- 硬编码仅包含 IdeEndpoint
- 进行启动时校验（default_endpoint、凭据 endpoint）
- 传递给 KiroProvider（第 147 行）和 AdminService（第 181 行）

#### 2.2.2 副本 2：KiroProvider 端点注册表

**文件位置**：`src/kiro/provider.rs` 第 33-46 行

```rust
pub struct KiroProvider {
    token_manager: Arc<MultiTokenManager>,
    global_proxy: Option<ProxyConfig>,
    client_cache: Mutex<HashMap<Option<ProxyConfig>, Client>>,
    tls_backend: TlsBackend,
    endpoints: HashMap<String, Arc<dyn KiroEndpoint>>,
    default_endpoint: String,
}
```

**解析方法**（第 97-109 行）：
```rust
fn endpoint_for(
    &self,
    credentials: &KiroCredentials,
) -> anyhow::Result<Arc<dyn KiroEndpoint>> {
    let name = credentials
        .endpoint
        .as_deref()
        .unwrap_or(&self.default_endpoint);
    self.endpoints
        .get(name)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("未知端点: {}", name))
}
```

**特点**：
- 从 main.rs 接收并存储为成员
- 每次调用均 `.cloned()`（可优化为 `Arc<dyn _>` 本身可 Clone）
- 无运行时动态注册机制

#### 2.2.3 副本 3：AdminService 端点集合

**文件位置**：`src/kiro/admin_service.rs` 第 40, 58 行

```rust
pub struct AdminService {
    token_manager: Arc<MultiTokenManager>,
    balance_cache: Mutex<BalanceCache>,
    cache_path: Option<PathBuf>,
    known_endpoints: HashSet<String>,  // ← 副本 3
}

pub fn new(
    token_manager: Arc<MultiTokenManager>,
    known_endpoints: impl IntoIterator<Item = String>,
) -> Self {
    Self {
        token_manager,
        balance_cache: Mutex::new(balance_cache),
        cache_path,
        known_endpoints: known_endpoints.into_iter().collect(),
    }
}
```

**用途**：add_credential 时校验端点合法性（第 202-211 行）

**特点**：
- 仅存储端点名称，不含实现
- 从 main.rs 的 `endpoint_names` 转换而来

#### 2.2.4 端点校验逻辑（AdminService.add_credential）

**文件位置**：`src/kiro/admin_service.rs` 第 197-212 行

```rust
pub async fn add_credential(
    &self,
    req: AddCredentialRequest,
) -> Result<AddCredentialResponse, AdminServiceError> {
    // 校验端点名：未指定则默认合法，指定则必须已注册
    if let Some(ref name) = req.endpoint {
        if !self.known_endpoints.contains(name) {
            let mut known: Vec<&str> =
                self.known_endpoints.iter().map(|s| s.as_str()).collect();
            known.sort();
            return Err(AdminServiceError::InvalidCredential(format!(
                "未知端点 \"{}\"，已注册端点: {:?}",
                name, known
            )));
        }
    }
    // ... 后续逻辑
}
```

**特点**：纯字符串匹配，无上游验证

### 2.3 调度层现状

#### 2.3.1 CallContext 定义

**文件位置**：`src/kiro/token_manager.rs` 第 540-548 行

```rust
#[derive(Clone)]
pub struct CallContext {
    pub id: u64,
    pub credentials: KiroCredentials,
    pub token: String,
}
```

**当前缺失**：无 `endpoint` 字段

#### 2.3.2 MultiTokenManager 构造与主要方法

**字段**（第 510-529 行）：
- `config: Config`
- `proxy: Option<ProxyConfig>`
- `entries: Mutex<Vec<CredentialEntry>>`
- `current_id: Mutex<u64>`
- `refresh_lock: TokioMutex<()>`
- `credentials_path: Option<PathBuf>`
- `is_multiple_format: bool`
- `load_balancing_mode: Mutex<String>`
- `last_stats_save_at: Mutex<Option<Instant>>`
- `stats_dirty: AtomicBool`

**当前缺失**：无 `Arc<EndpointRegistry>` 字段

**acquire_context()**（第 755-850 行）：
- 支持 `model` 参数过滤（Opus 需付费）
- 支持 priority / balanced 负载均衡
- 自动刷新过期/即将过期 Token

**try_ensure_token()**（第 884-969 行）：
- API Key 凭据直接返回
- OAuth 凭据检查过期状态，需要时刷新
- 返回 CallContext（id / credentials / token）

**force_refresh_token_for()**（第 1840-1874 行）：
- 无条件刷新；API Key 凭据会 bail
- 每凭据无缓存，每次调用都刷新
- 被 provider.rs 调用（API 第 411 行、MCP 第 225 行）

#### 2.3.3 KiroProvider 重试循环

##### API 调用重试（call_api_with_retry）

**文件位置**：`src/kiro/provider.rs` 第 279-491 行

**入口**（第 114-121 行）：
```rust
pub async fn call_api(&self, request_body: &str) -> anyhow::Result<reqwest::Response> {
    self.call_api_with_retry(request_body, false).await
}

pub async fn call_api_stream(&self, request_body: &str) -> anyhow::Result<reqwest::Response> {
    self.call_api_with_retry(request_body, true).await
}
```

**三段式构造**（第 293-331 行）：
```rust
// 凭据获取
let ctx = self.token_manager.acquire_context(model.as_deref()).await?;
let config = self.token_manager.config();
let machine_id = machine_id::generate_from_credentials(&ctx.credentials, config);

// endpoint 动态解析
let endpoint = self.endpoint_for(&ctx.credentials)?;

let rctx = RequestContext {
    credentials: &ctx.credentials,
    token: &ctx.token,
    machine_id: &machine_id,
    config,
};

// URL / Body / Header 段
let url = endpoint.api_url(&rctx);
let body = endpoint.transform_api_body(request_body, &rctx);
let base = self.client_for(&ctx.credentials)?.post(&url)
    .body(body)
    .header("content-type", "application/json")
    .header("Connection", "close");
let request = endpoint.decorate_api(base, &rctx);
```

**重试策略**：
- 总重试次数 = min(凭据数量 × MAX_RETRIES_PER_CREDENTIAL, MAX_TOTAL_RETRIES)
- MAX_RETRIES_PER_CREDENTIAL = 3，MAX_TOTAL_RETRIES = 9

**错误分支**：

1. **402 + 月度配额**（第 364-390 行）：
   - 调用 `endpoint.is_monthly_request_limit(&body)` + `status == 402`
   - 调用 `token_manager.report_quota_exhausted(ctx.id)` 禁用凭据
   - 不重试，继续下一凭据或返回错误

2. **401/403 + Bearer token 失效**（第 398-435 行）：
   - 调用 `endpoint.is_bearer_token_invalid(&body)`
   - 调用 `token_manager.force_refresh_token_for(ctx.id)` 强制刷新（每凭据一次）
   - 刷新成功则重试，失败则 `report_failure()`

3. **400 Bad Request**（第 392-395 行）：
   - 直接返回，不重试、不计入凭据失败

4. **429/408/5xx 瞬态错误**（第 437-457 行）：
   - 重试（Sleep + 计入失败计数，但不禁用凭据）

##### MCP 调用重试（call_mcp_with_retry）

**文件位置**：`src/kiro/provider.rs` 第 129-271 行

**结构**：与 API 循环相同，主要差异：
- 凭据获取：`acquire_context(None)`（无模型过滤）
- URL：`endpoint.mcp_url()`
- Body：`endpoint.transform_mcp_body()`
- Header：`endpoint.decorate_mcp()`

**错误分支**（第 204-254 行）：同 API 循环，分支逻辑完全重复

#### 2.3.4 get_usage_limits_for 硬编码痕迹

**文件位置**：`src/kiro/token_manager.rs` 第 1533-1639 行

```rust
pub async fn get_usage_limits_for(&self, id: u64) -> anyhow::Result<UsageLimitsResponse> {
    let credentials = { /* 获取凭据 */ };
    
    let token = if credentials.is_api_key_credential() {
        // API Key 凭据直接使用
        credentials.kiro_api_key.clone()?
    } else {
        // OAuth 凭据需刷新
        if self.should_refresh_token(&credentials) {
            let refreshed = refresh_token(&credentials, &self.config, ...).await?;
            // ...
        }
        credentials.access_token.clone()?
    };
    
    // ★ 调用 get_usage_limits 函数（纯 token_manager 级）
    let usage_limits = get_usage_limits(&credentials, &self.config, &token, effective_proxy.as_ref()).await?;
    
    Ok(usage_limits)
}
```

**get_usage_limits 函数**（第 323-393 行）：
- 硬编码 IDE 端点特定的 Host、URL 参数、User-Agent 格式
- 未通过 endpoint trait 抽象，无法按凭据切换不同端点的额度获取方式

### 2.4 trait 调用分布

#### Provider.rs 调用（9 处）

| 方法 | 行号 | 上下文 |
|------|------|--------|
| `endpoint.mcp_url()` | 165 | call_mcp_with_retry |
| `endpoint.transform_mcp_body()` | 166 | call_mcp_with_retry |
| `endpoint.decorate_mcp()` | 174 | call_mcp_with_retry |
| `endpoint.is_monthly_request_limit()` | 205 | call_mcp_with_retry |
| `endpoint.is_bearer_token_invalid()` | 222 | call_mcp_with_retry |
| `endpoint.api_url()` | 322 | call_api_with_retry |
| `endpoint.transform_api_body()` | 323 | call_api_with_retry |
| `endpoint.decorate_api()` | 331 | call_api_with_retry |
| `endpoint.is_monthly_request_limit()` | 364 | call_api_with_retry |
| `endpoint.is_bearer_token_invalid()` | 408 | call_api_with_retry |

**调用特点**：所有调用均在 provider.rs 内部，通过 `endpoint_for()` 取得 Arc 后立即使用

---

## 三、现状缺陷映射到目标改动

| 序号 | 现状问题 | 重构目标解决方案 | 影响的文件 & 行号 |
|------|---------|-----------------|-------------------|
| 1 | **三段式 URL/body/header 胶水** — 当前 URL、body 变换、header 装饰分散在三个 trait 方法中，provider 必须手动串联（api_url → transform_api_body → decorate_api），逻辑分散难以维护 | 合并为单一 `build_request(ctx, req: KiroRequest) -> RequestBuilder`；endpoint 内部负责完整的请求构造；provider 无需关心细节 | provider.rs 305-331, 158-174 |
| 2 | **402/401/403 字符串匹配长链条** — `is_monthly_request_limit()` 和 `is_bearer_token_invalid()` 分别通过字符串查找判断；provider 则通过 status 码 + trait 调用的组合判断；逻辑分散在三层（status 码 + 函数 + 字符串） | 新增 `classify_error(status, body) -> EndpointErrorKind` 枚举；endpoint 返回语义化的错误类型，provider 直接 match 枚举分支，无需重复的 status + 字符串判断 | provider.rs 364-435, endpoint/mod.rs 76-102 |
| 3 | **三处 endpoint 注册表副本** — main.rs、KiroProvider、AdminService 各持有一份 endpoint 信息（HashMap、HashSet），修改时必须同步三处，易出现不一致；无统一的注册/解析接口 | 新增 `EndpointRegistry` struct（`new()`、`resolve()`、`contains()`、`names()`）；main.rs 构建一次，以 Arc 形式共享给 Provider、TokenManager、AdminService；单一来源 | main.rs 100-127, provider.rs 43/79, admin_service.rs 40/58 |
| 4 | **CallContext 缺 endpoint 字段** — Provider 每次请求都要 `endpoint_for(credentials)` 查询一遍；TokenManager 无法提前解析 endpoint，导致 endpoint 信息无法流向调度链 | 在 CallContext 中新增 `endpoint: Arc<dyn KiroEndpoint>` 字段；TokenManager.acquire_context() 时同时调用 `registry.resolve(credentials)` 填充；Provider 无需再调用 endpoint_for | token_manager.rs 540-548, provider.rs 293-310 |
| 5 | **get_usage_limits_for 硬编码 IDE 格式** — get_usage_limits 函数硬编码了 IDE 端点特定的 Host/URL/User-Agent；无法按凭据切换不同端点的额度获取方式 | 改为 `acquire_context_for(id) → build_request(KiroRequest::UsageLimits) → send().json()`；每个 endpoint 在 build_request 中实现自己的额度请求构造，无 IDE 硬编码 | token_manager.rs 1533-1639, 323-393 |
| 6 | **machine_id 在 provider/token_manager 的重复计算** — Provider 的 API/MCP 重试循环各调用一次 `machine_id::generate_from_credentials()`（第 146、304 行）；目前未在 TokenManager 中计算，导致每次请求都重新生成 | RequestContext 删除 machine_id 字段；endpoint.build_request 内部调用 `machine_id::generate_from_credentials()` 一次；无缓存但内部调用成本低（多数情况走缓存路径 1-2 级） | provider.rs 146, 304; endpoint/mod.rs 162-177 (新) |
| 7 | **API / MCP 两个几乎重复的 call_*_with_retry** — call_api_with_retry 和 call_mcp_with_retry 逻辑 80% 重复（重试策略、凭据轮转、错误处理仅在 endpoint 方法名和 KiroRequest 类型上有差异） | 实现泛型 `call_with_retry<R: RequestType>(req: &R) -> Response`；或在 RequestContext 中嵌入 KiroRequest 枚举，单一重试循环处理 API/MCP；减少代码重复 | provider.rs 129-271, 279-491 |

---

## 四、开放问题 / 需规划阶段裁决的事项

### 4.1 CLI endpoint 实现状态

**描述**：最近提交 `35a7c93` 的评论提及 "支持按凭据切换 ide/cli"，但文件系统中仅存在 IdeEndpoint，无 CliEndpoint 实现。

**现象**：
- 代码现状：仅 `src/kiro/endpoint/ide.rs`；无 cli.rs
- 提交评论：暗示 CLI 支持已就位
- 不一致：提交可能计划包含但尚未完成

**规划决策待定**：
- CLI endpoint 是否需要在本轮重构中实现，还是作为后续功能？
- 若不实现，是否需要在 main.rs 注册表中预留占位符？

### 4.2 EndpointErrorKind 的 BadRequest vs ClientError 边界

**描述**：当前代码通过 status code 区分错误：
- `if status == 400` → 直接返回（BadRequest）
- `if status.is_client_error()` → 其他 4xx（如 401/403/402 经特殊处理）

**问题**：
- `EndpointErrorKind::BadRequest` 应仅对应 400，还是 4xx 通用？
- `EndpointErrorKind::ClientError` 是否应覆盖其他 4xx（含 401/402/403）后经过强制刷新/配额检查的最终结果？

**规划决策待定**：
- `classify_error()` 是否应同时接收 status code，以在 body 无明确错误标记时降级到 status-only 判断？
- 或者 endpoint 实现承诺在所有 4xx 情况下都从 body 中提取准确的语义信息？

### 4.3 RequestContext 是否应保留 machine_id 字段

**描述**：当前 RequestContext 包含 machine_id，供 endpoint 的 decorate 和 transform 方法使用。重构目标建议删除，改为 endpoint 内部调用。

**权衡**：

| 选项 | 优势 | 劣势 |
|------|------|------|
| 删除（当前方案） | RequestContext 更纯粹（仅凭据/token/配置）；无重复计算 | endpoint.build_request 每次调用 machine_id 函数；生成成本（虽有缓存） |
| 保留 | 性能无额外成本；RequestContext 已预算好 machine_id | 层级划分不清（RequestContext 不应包含端点计算结果）；machine_id 在 provider 计算后再传入 |

**规划决策待定**：
- 是否应优先保留 machine_id 性能，还是优先设计纯度？
- 如保留，machine_id 的计算时机与缓存策略如何设计？

### 4.4 validate_credential 校验应在 registry 还是保留在 admin_service

**描述**：当前 AdminService.add_credential 校验凭据的 endpoint 字段是否在已注册端点中。

**现象**：
- 校验逻辑在 admin_service.rs 第 202-211 行
- EndpointRegistry 可以提供 `contains(name)` 方法支持此校验

**规划决策待定**：
- EndpointRegistry.contains() 是否应作为公开 API（暴露为 registry.contains(name)）？
- 校验逻辑是否应迁移到 registry（新增 validate_endpoint_name 方法），还是保留在 admin_service 中调用 registry.contains？
- 是否需要在 registry 中新增额外的验证逻辑（如与凭据类型的兼容性检查）？

### 4.5 合并泛型 call_with_retry 的可行性

**描述**：API 和 MCP 的重试循环几乎完全相同，仅在请求类型和调用的 endpoint 方法上有差异。

**现象**：
- call_api_with_retry（第 279-491 行）和 call_mcp_with_retry（第 129-271 行）重复率 ~80%
- 差异点：
  - acquire_context 是否传入 model 参数
  - endpoint 方法（api_url vs mcp_url；decorate_api vs decorate_mcp；transform_api_body vs transform_mcp_body）

**规划决策待定**：
- KiroRequest 枚举是否足以作为参数，抽象出单一 `call_with_retry<R: Into<KiroRequest>>(req: R)` 方法？
- 重试语义（429/5xx 重试逻辑、402 配额处理、401/403 强制刷新）是否对 API 和 MCP 完全一致？
- 如可合并，是否值得引入泛型复杂度，还是保留两个具体实现？

---

## 五、验证与测试基线 —— 回归清单

本重构必须保持以下既有行为，建议在测试中验证：

### 5.1 重试策略

- [ ] **429 / 408 / 5xx 自动重试**：瞬态错误应触发 sleep + 重试，不应禁用凭据
- [ ] **重试次数上限**：min(凭据数 × MAX_RETRIES_PER_CREDENTIAL, MAX_TOTAL_RETRIES) = min(凭据数 × 3, 9)
- [ ] **重试延迟**：基于尝试次数的指数退避（Self::retry_delay(attempt)）

### 5.2 凭据轮转与禁用

- [ ] **402 + 月度配额**：禁用凭据、切换下一可用凭据（按优先级）
- [ ] **401/403 + Bearer token 失效**：强制刷新（每凭据一次机会）；刷新失败后禁用
- [ ] **400 Bad Request**：直接返回，不重试、不计入失败计数（因通常表示请求体错误）
- [ ] **连续失败 3 次**：凭据被禁用，切换到优先级最高的可用凭据

### 5.3 Token 刷新

- [ ] **API Key 凭据**：无刷新流程，直接使用 kiroApiKey
- [ ] **OAuth 凭据**：过期或即将过期时自动刷新（try_ensure_token）
- [ ] **force_refresh_token_for**：无条件刷新；API Key 凭据应返回错误；每凭据仅一次机会（provider 层面跟踪 force_refreshed Set）

### 5.4 负载均衡

- [ ] **priority 模式**：优先使用 current_id，不可用时自动切换到优先级最高的可用凭据
- [ ] **balanced 模式**：每次请求都重新选择最优凭据（调用 select_next_credential）

### 5.5 端点路由

- [ ] **凭据指定 endpoint**：优先使用指定端点；未指定则使用默认端点
- [ ] **启动时校验**：default_endpoint 和所有凭据声明的 endpoint 必须已注册；缺失时进程退出
- [ ] **运行时查询**：endpoint_for(credentials) 返回对应的 Arc<dyn KiroEndpoint>；未注册时返回错误（不应中断流程，应继续轮转凭据）

### 5.6 月度额度与配额管理

- [ ] **get_usage_limits_for**：获取指定凭据的剩余额度
- [ ] **get_usage_limits**：当前硬编码 IDE 格式；重构后应通过 endpoint.build_request 泛化

---

## 六、关键引用索引

本综合报告基于两份输入研究文档的内容整合，以下是规划者可直接回溯的章节对应：

### 源文档 1：/home/hank9999/kiro.rs/docs/plans/2026-04-18-kiro-endpoint-layered-refactor-codebase.md

- **协议层现状**：当前 KiroEndpoint trait（第 9-56 行）、IDE 实现（第 59-124 行）、CLI 不存在证据（第 145-154 行）
- **RequestContext 定义与构造**：定义（第 158-180 行）、两处构造点（第 182-213 行）
- **machine_id 模块**：公共函数、调用成本分析（第 217-277 行）
- **辅助函数与常量**：默认错误判断函数、IDE 辅助函数（第 281-315 行）
- **trait 调用分布**：provider.rs 9 处调用清单（第 319-342 行）
- **重构影响范围总结**：待折叠方法、新枚举驱动、改造点（第 346-387 行）

### 源文档 2：/home/hank9999/kiro.rs/docs/plans/2026-04-18-kiro-endpoint-layered-refactor-integration.md

- **KiroProvider 结构**：字段定义、构造签名、endpoint 解析方法（第 9-60 行）
- **API 调用重试循环**：概览、三段式构造、错误分支详析（第 63-206 行）
- **MCP 调用重试循环**：与 API 循环差异、错误分支（第 209-270 行）
- **get_usage_limits_for**：硬编码痕迹（第 273-312 行）
- **MultiTokenManager 结构**：关键字段、CallContext 定义、当前缺失（第 315-359 行）
- **acquire_context / try_ensure_token**：签名、实现细节（第 362-502 行）
- **三个 endpoint 副本**：main.rs、KiroProvider、AdminService 的具体位置与特点（第 506-595 行）
- **admin_service endpoint 校验逻辑**：add_credential 中的验证（第 598-643 行）
- **token_manager 的关键方法**：force_refresh_token_for、report_quota_exhausted、report_failure 等精确签名（第 646-908 行）
- **总结与影响范围**：当前数据流、重构后期望结构、影响表格、前置条件检查清单（第 922-976 行）

---

## 七、综合总结

### 当前状态摘要

**协议层**：KiroEndpoint trait 存在但方法分散（10 个）；仅有 IDE 实现，CLI 计划但未实现。

**路由层**：endpoint 注册表存在但散落为三份副本（main.rs / KiroProvider / AdminService），修改时易出现不一致；无统一解析接口。

**调度层**：CallContext 缺少 endpoint 字段，导致 Provider 每次请求都要重新查询；machine_id 在 Provider 中重复计算；get_usage_limits_for 硬编码 IDE 格式；两个重试循环代码重复率高。

### 重构收益

1. **协议层简化**：10 个方法折叠为 2 个方法 + 2 个枚举，逻辑内聚，易于扩展新端点
2. **路由层统一**：单一 EndpointRegistry，消除副本维护成本，提升可靠性
3. **调度层优化**：CallContext 增强，Provider 无需重复查询；重试循环可泛化；硬编码逐步消除
4. **代码质量**：减少重复、提升层级划分清晰度、降低维护成本

### 实施建议

规划者应基于本研究报告的**开放问题**（4.1-4.5）进行决策，确认以下关键点后再编制详细计划：
- CLI 实现时机与范围
- EndpointErrorKind 枚举的精确定义与 status code 处理策略
- RequestContext 的 machine_id 字段决策
- 校验逻辑的最终归属
- 泛型重试循环的可行性与必要性

本报告已包含所有必要的代码位置、签名、行号引用，规划者可直接据此拆解为可执行的任务单。
