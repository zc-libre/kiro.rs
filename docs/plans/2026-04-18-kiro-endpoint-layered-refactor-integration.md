# Kiro 端点分层重构 —— 当前集成现状研究报告

日期：2026-04-18  
研究范围：KiroProvider / MultiTokenManager / AdminService 中 endpoint 注册表、调用链路、错误处理的现状  
研究目标：为三层重构（Protocol / Routing / Scheduling）提供基线和集成点确认

---

## 一、KiroProvider 当前结构

### 字段定义（第 33-46 行）

```rust
pub struct KiroProvider {
    token_manager: Arc<MultiTokenManager>,
    global_proxy: Option<ProxyConfig>,
    client_cache: Mutex<HashMap<Option<ProxyConfig>, Client>>,
    tls_backend: TlsBackend,
    /// 端点实现注册表（key: endpoint 名称）
    endpoints: HashMap<String, Arc<dyn KiroEndpoint>>,
    /// 默认端点名称（凭据未指定 endpoint 时使用）
    default_endpoint: String,
}
```

### 构造签名（第 56-82 行）

```rust
pub fn with_proxy(
    token_manager: Arc<MultiTokenManager>,
    proxy: Option<ProxyConfig>,
    endpoints: HashMap<String, Arc<dyn KiroEndpoint>>,
    default_endpoint: String,
) -> Self
```

**关键约束：** 构造时断言 `default_endpoint` 必须在 endpoints 注册表中存在（第 62-66 行）

### endpoint 解析方法（第 97-109 行）

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

**当前耦合点：** 
- 无法在运行时更换 endpoints 注册表（HashMap 为 immutable）
- endpoint 名称解析纯本地，无额外上游验证

---

## 二、Provider API 调用重试循环（call_api_with_retry）

### 调用入口（第 114-121 行）

```rust
pub async fn call_api(&self, request_body: &str) -> anyhow::Result<reqwest::Response> {
    self.call_api_with_retry(request_body, false).await
}

pub async fn call_api_stream(&self, request_body: &str) -> anyhow::Result<reqwest::Response> {
    self.call_api_with_retry(request_body, true).await
}
```

### 重试循环概览（第 279-491 行）

**重试策略：** 
- 总重试次数 = min(凭据数量 × MAX_RETRIES_PER_CREDENTIAL, MAX_TOTAL_RETRIES)
- MAX_RETRIES_PER_CREDENTIAL = 3，MAX_TOTAL_RETRIES = 9

### endpoint 获取与 URL/Body/Header 三段式构造（第 293-331 行）

```rust
// 凭据获取（含 Token 自动刷新）
let ctx = match self.token_manager.acquire_context(model.as_deref()).await {
    Ok(c) => c,
    Err(e) => { /* 继续重试 */ },
};

let config = self.token_manager.config();
let machine_id = machine_id::generate_from_credentials(&ctx.credentials, config);

// ★ endpoint 动态解析
let endpoint = match self.endpoint_for(&ctx.credentials) {
    Ok(e) => e,
    Err(e) => {
        last_error = Some(e);
        self.token_manager.report_failure(ctx.id);
        continue;
    }
};

let rctx = RequestContext {
    credentials: &ctx.credentials,
    token: &ctx.token,
    machine_id: &machine_id,
    config,
};

// ★ URL 段
let url = endpoint.api_url(&rctx);
// ★ Body 段（端点特有变换）
let body = endpoint.transform_api_body(request_body, &rctx);

// ★ Header 段
let base = self
    .client_for(&ctx.credentials)?
    .post(&url)
    .body(body)
    .header("content-type", "application/json")
    .header("Connection", "close");
// ★ 端点装饰（追加 Authorization、host 等）
let request = endpoint.decorate_api(base, &rctx);
```

### 错误分支详析

#### 402 Payment Required + 额度用尽（第 364-390 行）

```rust
if status.as_u16() == 402 && endpoint.is_monthly_request_limit(&body) {
    tracing::warn!("API 请求失败（额度已用尽，禁用凭据并切换，尝试 {}/{}）: {} {}", ...);
    
    let has_available = self.token_manager.report_quota_exhausted(ctx.id);
    if !has_available {
        anyhow::bail!("{} API 请求失败（所有凭据已用尽）: {} {}", api_type, status, body);
    }
    
    last_error = Some(anyhow::anyhow!("{} API 请求失败: {} {}", api_type, status, body));
    continue;
}
```

**endpoint trait 调用：** `is_monthly_request_limit(&body)` → 判断是否为月度配额用尽  
**token_manager 调用：** `report_quota_exhausted(ctx.id)` → 禁用凭据、切换下一张

#### 401/403 凭据问题与强制刷新（第 398-435 行）

```rust
if matches!(status.as_u16(), 401 | 403) {
    tracing::warn!("API 请求失败（可能为凭据错误，尝试 {}/{}）: {} {}", ...);

    // ★ endpoint.is_bearer_token_invalid 判断
    if endpoint.is_bearer_token_invalid(&body) && !force_refreshed.contains(&ctx.id) {
        force_refreshed.insert(ctx.id);
        tracing::info!("凭据 #{} token 疑似被上游失效，尝试强制刷新", ctx.id);
        
        // ★ token_manager.force_refresh_token_for 强制刷新
        if self.token_manager.force_refresh_token_for(ctx.id).await.is_ok() {
            tracing::info!("凭据 #{} token 强制刷新成功，重试请求", ctx.id);
            continue;
        }
        tracing::warn!("凭据 #{} token 强制刷新失败，计入失败", ctx.id);
    }

    let has_available = self.token_manager.report_failure(ctx.id);
    if !has_available {
        anyhow::bail!("{} API 请求失败（所有凭据已用尽）: {} {}", api_type, status, body);
    }
    last_error = Some(anyhow::anyhow!("{} API 请求失败: {} {}", api_type, status, body));
    continue;
}
```

**endpoint trait 调用：** `is_bearer_token_invalid(&body)` → 判断是否为 bearer token 失效  
**token_manager 调用：** 
- `force_refresh_token_for(ctx.id)` → 无条件刷新（每凭据一次机会）
- `report_failure(ctx.id)` → 失败计数 +1，达到阈值后禁用

#### 400 Bad Request（第 392-395 行）

```rust
if status.as_u16() == 400 {
    anyhow::bail!("{} API 请求失败: {} {}", api_type, status, body);
}
```

**处理：** 直接返回，不重试、不计入凭据失败

#### 429/408/5xx 瞬态错误（第 437-457 行）

```rust
if matches!(status.as_u16(), 408 | 429) || status.is_server_error() {
    tracing::warn!("API 请求失败（上游瞬态错误，尝试 {}/{}）: {} {}", ...);
    last_error = Some(anyhow::anyhow!("{} API 请求失败: {} {}", api_type, status, body));
    if attempt + 1 < max_retries {
        sleep(Self::retry_delay(attempt)).await;
    }
    continue;
}
```

**处理：** 重试但不禁用/切换凭据（避免瞬态错误锁死凭据）

---

## 三、Provider MCP 调用重试循环（call_mcp_with_retry）

### 重试循环概览（第 129-271 行）

与 API 循环结构相同，主要差异：

1. **凭据获取：** `acquire_context(None)` —— MCP 调用无模型过滤
2. **endpoint 调用：** 
   - URL：`endpoint.mcp_url(&rctx)` 
   - Body：`endpoint.transform_mcp_body(request_body, &rctx)`
   - Header：`endpoint.decorate_mcp(base, &rctx)`

### 错误分支（第 204-254 行）

#### 402 + 月度额度（第 205-212 行）

```rust
if status.as_u16() == 402 && endpoint.is_monthly_request_limit(&body) {
    let has_available = self.token_manager.report_quota_exhausted(ctx.id);
    if !has_available {
        anyhow::bail!("MCP 请求失败（所有凭据已用尽）: {} {}", status, body);
    }
    last_error = Some(anyhow::anyhow!("MCP 请求失败: {} {}", status, body));
    continue;
}
```

#### 400（第 214-217 行）

```rust
if status.as_u16() == 400 {
    anyhow::bail!("MCP 请求失败: {} {}", status, body);
}
```

#### 401/403 + 强制刷新（第 220-238 行）

```rust
if matches!(status.as_u16(), 401 | 403) {
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
        anyhow::bail!("MCP 请求失败（所有凭据已用尽）: {} {}", status, body);
    }
    last_error = Some(anyhow::anyhow!("MCP 请求失败: {} {}", status, body));
    continue;
}
```

#### 408/429/5xx（第 241-254 行）

瞬态错误重试，同 API 循环

---

## 四、Provider get_usage_limits_for 的 IDE 格式硬编码痕迹

**当前状态：** get_usage_limits 是 token_manager 的模块级函数（第 323-393 行），不在 provider 中。  
**硬编码痕迹：** 注册表中仅有 `IdeEndpoint` 一个实现。

### 主要函数（src/kiro/token_manager.rs 第 1533-1639 行）

```rust
pub async fn get_usage_limits_for(&self, id: u64) -> anyhow::Result<UsageLimitsResponse> {
    // 获取凭据
    let credentials = { /* ... */ };

    // API Key 凭据直接使用 kiro_api_key，无需刷新
    let token = if credentials.is_api_key_credential() {
        // ...
    } else {
        // 需要刷新则刷新（含双重检查锁）
        // ...
    };

    // 重新获取凭据（刷新后可能已更新）
    let credentials = { /* ... */ };
    let effective_proxy = credentials.effective_proxy(self.proxy.as_ref());
    
    // ★ 调用 get_usage_limits 获取额度（纯 token_manager 级函数，未经 endpoint）
    let usage_limits = get_usage_limits(&credentials, &self.config, &token, effective_proxy.as_ref()).await?;

    // 更新订阅等级到凭据
    if let Some(subscription_title) = usage_limits.subscription_title() {
        // ...
    }

    Ok(usage_limits)
}
```

**当前问题：** 
- `get_usage_limits` 函数（第 323-393 行）硬编码了 IDE 端点特定的 Host、URL 参数、User-Agent 格式
- 没有通过 endpoint.trait 抽象，无法按凭据切换不同端点的额度获取方式

---

## 五、MultiTokenManager 当前结构

### 关键字段（第 510-529 行）

```rust
pub struct MultiTokenManager {
    config: Config,
    proxy: Option<ProxyConfig>,
    /// 凭据条目列表
    entries: Mutex<Vec<CredentialEntry>>,
    /// 当前活动凭据 ID
    current_id: Mutex<u64>,
    /// Token 刷新锁，确保同一时间只有一个刷新操作
    refresh_lock: TokioMutex<()>,
    /// 凭据文件路径（用于回写）
    credentials_path: Option<PathBuf>,
    /// 是否为多凭据格式（数组格式才回写）
    is_multiple_format: bool,
    /// 负载均衡模式（运行时可修改）
    load_balancing_mode: Mutex<String>,
    /// 最近一次统计持久化时间（用于 debounce）
    last_stats_save_at: Mutex<Option<Instant>>,
    /// 统计数据是否有未落盘更新
    stats_dirty: AtomicBool,
}
```

**当前缺失：** 无 endpoints 注册表存储（由 KiroProvider 持有）

### CallContext 定义（第 540-548 行）

```rust
#[derive(Clone)]
pub struct CallContext {
    /// 凭据 ID（用于 report_success/report_failure）
    pub id: u64,
    /// 凭据信息（用于构建请求头）
    pub credentials: KiroCredentials,
    /// 访问 Token
    pub token: String,
}
```

**当前缺失字段：** `endpoint: Arc<dyn KiroEndpoint>` —— 新层级需要在此添加

---

## 六、acquire_context / acquire_context_for 当前签名与实现

### acquire_context（第 755-850 行）

```rust
pub async fn acquire_context(&self, model: Option<&str>) -> anyhow::Result<CallContext> {
    let total = self.total_count();
    let max_attempts = (total * MAX_FAILURES_PER_CREDENTIAL as usize).max(1);
    let mut attempt_count = 0;

    loop {
        if attempt_count >= max_attempts {
            anyhow::bail!("所有凭据均无法获取有效 Token（可用: {}/{}）", ...);
        }

        let (id, credentials) = {
            let is_balanced = self.load_balancing_mode.lock().as_str() == "balanced";

            // balanced 模式：每次请求都重新均衡选择
            // priority 模式：优先使用 current_id 指向的凭据
            let current_hit = if is_balanced {
                None
            } else {
                let entries = self.entries.lock();
                let current_id = *self.current_id.lock();
                entries
                    .iter()
                    .find(|e| e.id == current_id && !e.disabled)
                    .map(|e| (e.id, e.credentials.clone()))
            };

            if let Some(hit) = current_hit {
                hit
            } else {
                let mut best = self.select_next_credential(model);
                // 没有可用凭据自愈逻辑...
                // 更新 current_id...
                (new_id, new_creds)
            }
        };

        // 尝试获取/刷新 Token
        match self.try_ensure_token(id, &credentials).await {
            Ok(ctx) => {
                return Ok(ctx);
            }
            Err(e) => {
                // Token 刷新失败计入重试计数
                attempt_count += 1;
                if !has_available {
                    anyhow::bail!("所有凭据均已禁用（0/{}）", total);
                }
            }
        }
    }
}
```

**当前特性：** 
- 支持 `model` 参数过滤凭据（Opus 模型需付费）
- 支持 priority / balanced 两种负载均衡模式
- 自动刷新过期/即将过期的 Token（via try_ensure_token）

**未来添加点：** 返回 CallContext 时需同时返回 resolved endpoint（从 KiroProvider 的注册表）

### try_ensure_token（第 884-969 行）

```rust
async fn try_ensure_token(
    &self,
    id: u64,
    credentials: &KiroCredentials,
) -> anyhow::Result<CallContext> {
    // API Key 凭据直接使用 kiro_api_key，无需刷新
    if credentials.is_api_key_credential() {
        let token = credentials
            .kiro_api_key
            .clone()
            .ok_or_else(|| anyhow::anyhow!("API Key 凭据缺少 kiroApiKey"))?;
        return Ok(CallContext {
            id,
            credentials: credentials.clone(),
            token,
        });
    }

    // 第一次检查（无锁）：快速判断是否需要刷新
    let needs_refresh = is_token_expired(credentials) || is_token_expiring_soon(credentials);

    let creds = if needs_refresh {
        // 获取刷新锁，确保同一时间只有一个刷新操作
        let _guard = self.refresh_lock.lock().await;

        // 第二次检查：获取锁后重新读取凭据
        let current_creds = { /* 从 entries 重新读取 */ };

        if is_token_expired(&current_creds) || is_token_expiring_soon(&current_creds) {
            // 确实需要刷新
            let effective_proxy = current_creds.effective_proxy(self.proxy.as_ref());
            let new_creds =
                refresh_token(&current_creds, &self.config, effective_proxy.as_ref()).await?;

            // 更新凭据
            { /* entries.iter_mut.find.credentials = new_creds */ }

            // 回写凭据到文件
            if let Err(e) = self.persist_credentials() {
                tracing::warn!("Token 刷新后持久化失败（不影响本次请求）: {}", e);
            }

            new_creds
        } else {
            // 其他请求已经完成刷新
            current_creds
        }
    } else {
        credentials.clone()
    };

    let token = creds
        .access_token
        .clone()
        .ok_or_else(|| anyhow::anyhow!("没有可用的 accessToken"))?;

    // 重置刷新失败计数
    {
        let mut entries = self.entries.lock();
        if let Some(entry) = entries.iter_mut().find(|e| e.id == id) {
            entry.refresh_failure_count = 0;
        }
    }

    Ok(CallContext {
        id,
        credentials: creds,
        token,
    })
}
```

**无 acquire_context_for 方法**（当前代码中无此方法，仅 acquire_context 支持模型过滤）

---

## 七、Endpoint 注册表的三个副本（当前状态）

### 副本 1：main.rs 中的端点注册表（第 100-127 行）

**位置：** `/home/hank9999/kiro.rs/src/main.rs` 第 100-127 行

```rust
// 构建端点注册表
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
            cred.id,
            name,
            endpoints.keys().collect::<Vec<_>>()
        );
        std::process::exit(1);
    }
}

let endpoint_names: Vec<String> = endpoints.keys().cloned().collect();
```

**特点：** 
- 硬编码仅包含 IdeEndpoint（无运行时动态注册机制）
- 构建后传递给 KiroProvider（第 147 行）和 AdminService（第 181 行）
- endpoint_names 仅传递给 AdminService，未传给 token_manager

### 副本 2：KiroProvider 中的端点注册表（第 43, 79 行）

```rust
pub struct KiroProvider {
    // ...
    endpoints: HashMap<String, Arc<dyn KiroEndpoint>>,
    default_endpoint: String,
}
```

**特点：**
- 从 main.rs 接收并存储为成员字段
- 每个 Provider 持有完整副本（不共享）
- endpoint_for 方法用于按凭据名称查询

### 副本 3：AdminService 中的已知端点集合（第 40, 58 行）

```rust
pub struct AdminService {
    // ...
    /// 已注册的端点名称集合（用于 add_credential 校验）
    known_endpoints: HashSet<String>,
}

impl AdminService {
    pub fn new(
        token_manager: Arc<MultiTokenManager>,
        known_endpoints: impl IntoIterator<Item = String>,
    ) -> Self {
        // ...
        Self {
            token_manager,
            balance_cache: Mutex::new(balance_cache),
            cache_path,
            known_endpoints: known_endpoints.into_iter().collect(),
        }
    }
}
```

**特点：**
- 从 main.rs 的 `endpoint_names` 转换而来（main.rs 第 181 行）
- 仅存储 HashSet<String>（端点名称），不含实现
- 用于 add_credential 校验（第 202-211 行）

---

## 八、admin_service 中 endpoint 校验逻辑

### add_credential 中的端点验证（第 197-212 行）

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

    // 构建凭据对象
    let email = req.email.clone();
    let new_cred = KiroCredentials {
        // ...
        endpoint: req.endpoint,
        // ...
    };

    // 调用 token_manager 添加凭据
    let credential_id = self
        .token_manager
        .add_credential(new_cred)
        .await
        .map_err(|e| self.classify_add_error(e))?;

    // ...
}
```

**特点：**
- 若凭据指定 endpoint，必须在 known_endpoints 中
- 若未指定，由 token_manager.add_credential 处理（默认为 None）
- 端点验证纯粹是字符串匹配，无上游检验

---

## 九、force_refresh_token_for / report_quota_exhausted 等精确签名

### force_refresh_token_for（第 1840-1874 行）

```rust
pub async fn force_refresh_token_for(&self, id: u64) -> anyhow::Result<()> {
    let credentials = {
        let entries = self.entries.lock();
        entries
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.credentials.clone())
            .ok_or_else(|| anyhow::anyhow!("凭据不存在: {}", id))?
    };

    // 获取刷新锁防止并发刷新
    let _guard = self.refresh_lock.lock().await;

    // 无条件调用 refresh_token（会对 API Key 凭据 bail）
    let effective_proxy = credentials.effective_proxy(self.proxy.as_ref());
    let new_creds =
        refresh_token(&credentials, &self.config, effective_proxy.as_ref()).await?;

    // 更新 entries 中对应凭据
    {
        let mut entries = self.entries.lock();
        if let Some(entry) = entries.iter_mut().find(|e| e.id == id) {
            entry.credentials = new_creds;
            entry.refresh_failure_count = 0;
        }
    }

    // 持久化
    if let Err(e) = self.persist_credentials() {
        tracing::warn!("强制刷新 Token 后持久化失败: {}", e);
    }

    tracing::info!("凭据 #{} Token 已强制刷新", id);
    Ok(())
}
```

**特点：**
- 无条件刷新（不检查 is_token_expired）
- API Key 凭据调用会 bail "API Key 凭据不支持刷新"
- 被 provider.rs 第 411 行（API 循环）和 225 行（MCP 循环）调用
- 被 admin_service.rs 第 305 行（Admin API force_refresh_token）调用

### report_quota_exhausted（第 1211-1253 行）

```rust
pub fn report_quota_exhausted(&self, id: u64) -> bool {
    let result = {
        let mut entries = self.entries.lock();
        let mut current_id = self.current_id.lock();

        let entry = match entries.iter_mut().find(|e| e.id == id) {
            Some(e) => e,
            None => return entries.iter().any(|e| !e.disabled),
        };

        if entry.disabled {
            return entries.iter().any(|e| !e.disabled);
        }

        entry.disabled = true;
        entry.disabled_reason = Some(DisabledReason::QuotaExceeded);
        entry.last_used_at = Some(Utc::now().to_rfc3339());
        // 设为阈值，便于在管理面板中直观看到该凭据已不可用
        entry.failure_count = MAX_FAILURES_PER_CREDENTIAL;

        tracing::error!("凭据 #{} 额度已用尽（MONTHLY_REQUEST_COUNT），已被禁用", id);

        // 切换到优先级最高的可用凭据
        if let Some(next) = entries
            .iter()
            .filter(|e| !e.disabled)
            .min_by_key(|e| e.credentials.priority)
        {
            *current_id = next.id;
            tracing::info!("已切换到凭据 #{}（优先级 {}）", next.id, next.credentials.priority);
            true
        } else {
            tracing::error!("所有凭据均已禁用！");
            false
        }
    };
    self.save_stats_debounced();
    result
}
```

**返回值：** `bool` —— 是否还有可用凭据

### report_failure（第 1152-1203 行）

```rust
pub fn report_failure(&self, id: u64) -> bool {
    let result = {
        let mut entries = self.entries.lock();
        let mut current_id = self.current_id.lock();

        let entry = match entries.iter_mut().find(|e| e.id == id) {
            Some(e) => e,
            None => return entries.iter().any(|e| !e.disabled),
        };

        if entry.disabled {
            return entries.iter().any(|e| !e.disabled);
        }

        entry.failure_count += 1;
        entry.last_used_at = Some(Utc::now().to_rfc3339());
        let failure_count = entry.failure_count;

        tracing::warn!(
            "凭据 #{} API 调用失败（{}/{}）",
            id,
            failure_count,
            MAX_FAILURES_PER_CREDENTIAL
        );

        if failure_count >= MAX_FAILURES_PER_CREDENTIAL {
            entry.disabled = true;
            entry.disabled_reason = Some(DisabledReason::TooManyFailures);
            tracing::error!("凭据 #{} 已连续失败 {} 次，已被禁用", id, failure_count);

            // 切换到优先级最高的可用凭据
            if let Some(next) = entries
                .iter()
                .filter(|e| !e.disabled)
                .min_by_key(|e| e.credentials.priority)
            {
                *current_id = next.id;
                tracing::info!(
                    "已切换到凭据 #{}（优先级 {}）",
                    next.id,
                    next.credentials.priority
                );
            } else {
                tracing::error!("所有凭据均已禁用！");
            }
        }

        entries.iter().any(|e| !e.disabled)
    };
    self.save_stats_debounced();
    result
}
```

**MAX_FAILURES_PER_CREDENTIAL = 3**

### report_refresh_failure（第 1259-1316 行）

```rust
pub fn report_refresh_failure(&self, id: u64) -> bool {
    let result = {
        let mut entries = self.entries.lock();
        let mut current_id = self.current_id.lock();

        let entry = match entries.iter_mut().find(|e| e.id == id) {
            Some(e) => e,
            None => return entries.iter().any(|e| !e.disabled),
        };

        if entry.disabled {
            return entries.iter().any(|e| !e.disabled);
        }

        entry.last_used_at = Some(Utc::now().to_rfc3339());
        entry.refresh_failure_count += 1;
        let refresh_failure_count = entry.refresh_failure_count;

        tracing::warn!(
            "凭据 #{} Token 刷新失败（{}/{}）",
            id,
            refresh_failure_count,
            MAX_FAILURES_PER_CREDENTIAL
        );

        if refresh_failure_count < MAX_FAILURES_PER_CREDENTIAL {
            return entries.iter().any(|e| !e.disabled);
        }

        entry.disabled = true;
        entry.disabled_reason = Some(DisabledReason::TooManyRefreshFailures);

        tracing::error!(
            "凭据 #{} Token 已连续刷新失败 {} 次，已被禁用",
            id,
            refresh_failure_count
        );

        if let Some(next) = entries
            .iter()
            .filter(|e| !e.disabled)
            .min_by_key(|e| e.credentials.priority)
        {
            *current_id = next.id;
            tracing::info!(
                "已切换到凭据 #{}（优先级 {}）",
                next.id,
                next.credentials.priority
            );
            true
        } else {
            tracing::error!("所有凭据均已禁用！");
            false
        }
    };
    self.save_stats_debounced();
    result
}
```

### report_refresh_token_invalid（第 1322-1364 行）

```rust
pub fn report_refresh_token_invalid(&self, id: u64) -> bool {
    let result = {
        let mut entries = self.entries.lock();
        let mut current_id = self.current_id.lock();

        let entry = match entries.iter_mut().find(|e| e.id == id) {
            Some(e) => e,
            None => return entries.iter().any(|e| !e.disabled),
        };

        if entry.disabled {
            return entries.iter().any(|e| !e.disabled);
        }

        entry.last_used_at = Some(Utc::now().to_rfc3339());
        entry.disabled = true;
        entry.disabled_reason = Some(DisabledReason::InvalidRefreshToken);

        tracing::error!(
            "凭据 #{} refreshToken 已失效 (invalid_grant)，已立即禁用",
            id
        );

        if let Some(next) = entries
            .iter()
            .filter(|e| !e.disabled)
            .min_by_key(|e| e.credentials.priority)
        {
            *current_id = next.id;
            tracing::info!(
                "已切换到凭据 #{}（优先级 {}）",
                next.id,
                next.credentials.priority
            );
            true
        } else {
            tracing::error!("所有凭据均已禁用！");
            false
        }
    };
    self.save_stats_debounced();
    result
}
```

**特点：** 立即禁用（无累计重试），因 refreshToken 已永久失效（invalid_grant）

### get_usage_limits_for（第 1533-1639 行）

**已在第四部分详细阐述**

### add_credential（第 1654-1768 行）

**已在第七部分详细阐述**

---

## 十、总结：三个 endpoint 副本的协调关系和重构影响

### 当前数据流

```
main.rs 构建 endpoints HashMap 
    ↓
    ├→ KiroProvider.endpoints (完整副本)
    │   └→ endpoint_for(credentials) 用于请求时查询
    │
    └→ endpoint_names Vec 
        └→ AdminService.known_endpoints (HashSet)
            └→ add_credential 时校验端点合法性
```

### 重构后的期望结构

```
EndpointRegistry {
    endpoints: HashMap<String, Arc<dyn KiroEndpoint>>,
    default: String,
}

main.rs 构建 EndpointRegistry
    ↓
    ├→ KiroProvider (持有 Arc<EndpointRegistry>)
    │   └→ endpoint_for → registry.resolve(credentials)
    │
    ├→ MultiTokenManager (持有 Arc<EndpointRegistry>)
    │   └→ acquire_context 时返回 endpoint（嵌入 CallContext）
    │
    └→ AdminService (持有 Arc<EndpointRegistry>)
        └→ add_credential 时通过 registry.contains(name) 校验
```

### 影响范围

| 组件 | 当前 | 重构后 | 影响 |
|------|------|--------|------|
| KiroProvider | 持有 endpoints HashMap | 持有 Arc<EndpointRegistry> | 字段类型变化，endpoint_for 逻辑迁移到 registry.resolve |
| MultiTokenManager | 不持有 endpoint 信息 | 持有 Arc<EndpointRegistry> | 新增字段，acquire_context 返回值增加 endpoint 字段 |
| CallContext | 3 字段（id/credentials/token） | 4 字段（+ endpoint） | 提供者侧：provider 或 token_manager 负责注入；消费侧：call_api_with_retry 不再调用 endpoint_for |
| AdminService | 持有 known_endpoints: HashSet | 持有 Arc<EndpointRegistry> | 字段类型变化，endpoint 验证逻辑由字符串检查升级为 registry.contains |
| admin_service.get_usage_limits_for | 无 endpoint 抽象 | 使用 registry 中对应的 endpoint.trait（待实现） | 解耦 IDE 硬编码，支持多端点 |

---

## 十一、重构前置条件检查清单

- [ ] 确认 endpoint trait 需要添加 `get_usage_limits_url` / `decorate_usage_limits` 等方法（或继续保持 token_manager 级函数）
- [ ] 确认 CallContext.endpoint 的所有权模型（Arc<dyn KiroEndpoint> vs &'a dyn KiroEndpoint）
- [ ] 确认 EndpointRegistry 是否需要支持运行时动态注册（当前无此需求）
- [ ] 确认 Admin API 中 endpoint 字段的增删改权限（当前仅在 add_credential 时校验）
- [ ] 确认是否需要为 endpoint 添加 REST API 端点（如 GET /api/admin/endpoints）

