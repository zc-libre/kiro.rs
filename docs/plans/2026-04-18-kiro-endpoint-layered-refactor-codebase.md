# Kiro 端点协议层当前状态研究

研究时间：2026-04-18  
目标：为 KiroEndpoint trait 分层重构做准备  
重构目标：引入 `KiroRequest`、`EndpointErrorKind` 枚举，将多个狭义方法折叠为 `build_request()` + `classify_error()`

---

## 当前 KiroEndpoint trait 方法清单

位置：`src/kiro/endpoint/mod.rs`（21-57 行）

```rust
pub trait KiroEndpoint: Send + Sync {
    /// fn name(&self) -> &'static str
    /// 端点名称，对应 credentials.endpoint / config.defaultEndpoint 的取值
    
    /// fn api_url(&self, ctx: &RequestContext<'_>) -> String
    /// 返回 API 端点 URL 字符串
    
    /// fn mcp_url(&self, ctx: &RequestContext<'_>) -> String
    /// 返回 MCP 端点 URL 字符串（用于 WebSearch 等工具调用）
    
    /// fn decorate_api(&self, req: RequestBuilder, ctx: &RequestContext<'_>) -> RequestBuilder
    /// 装饰 API 请求的端点特有 header（Authorization、host、user-agent 等）
    /// 前置条件：Provider 已设置 URL、content-type、Connection 和 body
    
    /// fn decorate_mcp(&self, req: RequestBuilder, ctx: &RequestContext<'_>) -> RequestBuilder
    /// 装饰 MCP 请求的端点特有 header
    
    /// fn transform_api_body(&self, body: &str, ctx: &RequestContext<'_>) -> String
    /// 对已序列化的 API 请求体做端点特有加工（如注入 profileArn）
    
    /// fn transform_mcp_body(&self, body: &str, _ctx: &RequestContext<'_>) -> String
    /// 对已序列化的 MCP 请求体做端点特有加工
    /// 默认实现：直接返回原始 body（body.to_string()）
    
    /// fn is_monthly_request_limit(&self, body: &str) -> bool
    /// 判断响应体是否表示"月度配额用尽"
    /// 默认实现：使用 default_is_monthly_request_limit()
    
    /// fn is_bearer_token_invalid(&self, body: &str) -> bool
    /// 判断响应体是否表示"上游 bearer token 失效"（触发强制刷新）
    /// 默认实现：使用 default_is_bearer_token_invalid()
}
```

**方法总数**：10 个（其中 2 个有默认实现）

**关键特征**：
- 无 `Self::associate_type`
- URL 方法返回 `String`
- 装饰方法接收 `RequestBuilder` 并返回 `RequestBuilder`
- body 变换和错误判断都是 `&self + &str -> result`
- 两个识别方法依赖 response body 文本内容

---

## ide.rs 实现要点

位置：`src/kiro/endpoint/ide.rs`（57-111 行的 `impl KiroEndpoint for IdeEndpoint`）

### 端点名称
- **实现**：`name()` 返回 `IDE_ENDPOINT_NAME`（常量"ide"）

### URL 构造

**api_url()**
```
https://q.{api_region}.amazonaws.com/generateAssistantResponse
```
- `api_region` 来自 `ctx.credentials.effective_api_region(ctx.config)`
- 对应 AWS CodeWhisperer 生成接口

**mcp_url()**
```
https://q.{api_region}.amazonaws.com/mcp
```
- 同一个主机下的 `/mcp` 路径

### Header 装饰

**decorate_api()** 添加头部（lines 73-88）：
```
x-amzn-codewhisperer-optout: true
x-amzn-kiro-agent-mode: vibe
x-amz-user-agent: aws-sdk-js/1.0.34 KiroIDE-{kiro_version}-{machine_id}
user-agent: aws-sdk-js/1.0.34 ua/2.1 os/{system_version} lang/js md/nodejs#{node_version} api/codewhispererstreaming#1.0.34 m/E KiroIDE-{kiro_version}-{machine_id}
host: q.{api_region}.amazonaws.com
amz-sdk-invocation-id: {uuid-v4}
amz-sdk-request: attempt=1; max=3
Authorization: Bearer {token}
tokentype: API_KEY (仅限 API Key 凭据)
```

**decorate_mcp()** 添加头部（lines 90-106）：
```
x-amz-user-agent: aws-sdk-js/1.0.34 KiroIDE-{kiro_version}-{machine_id}
user-agent: aws-sdk-js/1.0.34 ua/2.1 os/{system_version} lang/js md/nodejs#{node_version} api/codewhispererstreaming#1.0.34 m/E KiroIDE-{kiro_version}-{machine_id}
host: q.{api_region}.amazonaws.com
amz-sdk-invocation-id: {uuid-v4}
amz-sdk-request: attempt=1; max=3
Authorization: Bearer {token}
x-amzn-kiro-profile-arn: {profile_arn} (如果存在)
tokentype: API_KEY (仅限 API Key 凭据)
```

### Body 变换

**transform_api_body()** 调用 `inject_profile_arn()` helper（lines 108-124）：
- 将 `ctx.credentials.profile_arn` 注入到 JSON 根对象的 `profileArn` 字段
- 如果 profile_arn 为 None，直接返回原 body
- 如果 JSON 解析失败，返回原 body（容错）

**transform_mcp_body()** 不覆盖，使用 trait 默认实现

### 特征常数和辅助方法

**IDE_ENDPOINT_NAME**（line 15）
```rust
pub const IDE_ENDPOINT_NAME: &str = "ide";
```

**IdeEndpoint 内部辅助方法**（未暴露）：
- `api_region()` - 获取生效的 API Region
- `host()` - 生成 `q.{api_region}.amazonaws.com`
- `x_amz_user_agent()` - 生成 x-amz-user-agent 字符串
- `user_agent()` - 生成 User-Agent 字符串（包含系统版本、Node 版本）

### 常见参数源

从 `RequestContext` 获取：
- `ctx.credentials.effective_api_region(ctx.config)` - API Region
- `ctx.credentials.profile_arn` - ProfileArn（可选）
- `ctx.token` - Bearer Token
- `ctx.machine_id` - 设备指纹（用于 UA）

从 `Config` 获取：
- `config.kiro_version` - Kiro 版本字符串
- `config.system_version` - 系统版本（用于 User-Agent）
- `config.node_version` - Node 版本（用于 User-Agent）

---

## cli.rs 实现情况

**存在性**：不存在

**证据**：
1. 文件系统扫描：`src/kiro/endpoint/` 目录仅含 `mod.rs` 和 `ide.rs`（见 Glob 结果）
2. impl 搜索：全局搜索 `impl KiroEndpoint` 仅返回 `src/kiro/endpoint/ide.rs`（1 处）
3. 注释参考：`mod.rs` 第 3 行文档评论提到 "如 `ide` / `cli`"，但仅作为潜在候选

**结论**：CLI 端点实现不存在，只有 IDE 端点（这与最近一次提交 `35a7c93 refactor: 抽象 Kiro 端点 trait，支持按凭据切换 ide/cli` 的评论相悖；需要确认该提交是否创建了 cli 模块）

---

## RequestContext 当前定义与构造点

### 定义

位置：`src/kiro/endpoint/mod.rs`（62-71 行）

```rust
pub struct RequestContext<'a> {
    /// 当前凭据
    pub credentials: &'a KiroCredentials,
    
    /// 有效的 access token（API Key 凭据下即 kiroApiKey）
    pub token: &'a str,
    
    /// 当前凭据对应的 machineId
    pub machine_id: &'a str,
    
    /// 全局配置
    pub config: &'a Config,
}
```

**特征**：纯引用结构体，避免无谓 clone（生命周期 `'a`）

### 构造点

**构造 1**：API 调用重试循环（src/kiro/provider.rs, 315-320 行）
```rust
let rctx = RequestContext {
    credentials: &ctx.credentials,
    token: &ctx.token,
    machine_id: &machine_id,
    config,
};
```
- 在 `call_api_with_retry()` 方法内
- 每次重试都重新构造一个新的 RequestContext

**构造 2**：MCP 调用重试循环（src/kiro/provider.rs, 158-163 行）
```rust
let rctx = RequestContext {
    credentials: &ctx.credentials,
    token: &ctx.token,
    machine_id: &machine_id,
    config,
};
```
- 在 `call_mcp_with_retry()` 方法内
- 同样为每次重试重新构造

**machine_id 来源**：
```rust
let machine_id = machine_id::generate_from_credentials(&ctx.credentials, config);
```
- 在 RequestContext 构造前调用（src/kiro/provider.rs, 146 行和 304 行）
- 返回值是 `String`，然后被引用

---

## machine_id 模块公共 API 与调用成本

位置：`src/kiro/machine_id.rs`

### 核心公共函数

**generate_from_credentials()**（lines 54-86）
```rust
pub fn generate_from_credentials(credentials: &KiroCredentials, config: &Config) -> String
```

**语义**：根据凭证信息生成唯一的 Machine ID  
**返回**：64 字符十六进制字符串

**优先级**（顺序执行）：
1. 凭据级 `machineId`（若配置且格式合法）
2. 全局 `config.machineId`（若配置且格式合法）
3. 根据凭据类型派生（互斥两条路）：
   - API Key 凭据：`sha256("KiroAPIKey/" + kiroApiKey)`
   - OAuth 凭据：`sha256("KotlinNativeAPI/" + refreshToken)`
4. 兜底：`sha256("KiroFallback/" + uuid)`（按凭据 ID 进程内缓存）

**调用成本**：
- **廉价**：配置值直接返回（第 1-2 级，仅 normalize 校验）
- **中等**：SHA256 哈希派生（第 3 级）
- **廉价但依赖缓存**：兜底机制（第 4 级，首次生成一个随机 UUID，后续复用）

### 辅助函数（内部/私有）

**normalize_machine_id()** （lines 24-43）
- 验证和标准化 machine_id 格式
- 支持 64 字符十六进制和 UUID 格式（去掉连字符后补齐到 64 字符）
- 返回 `Option<String>`

**fallback_machine_id()** （lines 94-109）
- 为缺失派生材料的凭据生成兜底 machineId
- 使用全局 `FALLBACK_MACHINE_IDS` HashMap 按 `credentials.id` 缓存
- 首次生成时发出 warn 日志
- 进程重启会重新随机

**sha256_hex()** （lines 112-117）
- SHA256 哈希实现，返回十六进制字符串
- 调用内部使用（via `sha2` crate）

### 全局状态

**FALLBACK_MACHINE_IDS** （lines 14-17）
```rust
static FALLBACK_MACHINE_IDS: OnceLock<Mutex<HashMap<Option<u64>, String>>> = OnceLock::new();
```
- 兜底 machineId 缓存
- 按凭据 `id` 分桶
- 进程生命周期内稳定，不持久化

### 调用点

**provider.rs**：
- `call_api_with_retry()` 第 304 行
- `call_mcp_with_retry()` 第 146 行

调用频率：每次 API/MCP 请求都会调用一次（在获取 token 后、构造 RequestContext 前）

---

## 辅助函数/常量

### 默认错误判断函数（mod.rs）

**default_is_monthly_request_limit()** （lines 76-97）
```rust
pub fn default_is_monthly_request_limit(body: &str) -> bool
```
- 查找字符串 "MONTHLY_REQUEST_COUNT" 或 JSON 字段 `reason == "MONTHLY_REQUEST_COUNT"`
- 支持顶层 `reason` 和嵌套 `error.reason` 两种格式
- 返回 bool

**default_is_bearer_token_invalid()** （lines 100-102）
```rust
pub fn default_is_bearer_token_invalid(body: &str) -> bool
```
- 查找字符串 "The bearer token included in the request is invalid"
- 返回 bool

### IdeEndpoint 辅助函数（ide.rs）

**inject_profile_arn()** （lines 114-124）
```rust
fn inject_profile_arn(request_body: &str, profile_arn: &Option<String>) -> String
```
- 将 profile_arn 注入到请求体 JSON 根对象
- 如果 profile_arn 为 None 或 JSON 解析失败，返回原 body
- 用于 transform_api_body() 内部

### 常量

**IDE_ENDPOINT_NAME** （ide.rs, line 15）
```rust
pub const IDE_ENDPOINT_NAME: &str = "ide";
```

---

## trait 方法被哪些模块调用（仅 file:line 清单，不做分析）

### provider.rs（9 处调用）

| 方法 | 行号 | 方法 | 行号 |
|------|------|------|------|
| `endpoint.mcp_url()` | 165 | `endpoint.api_url()` | 322 |
| `endpoint.transform_mcp_body()` | 166 | `endpoint.transform_api_body()` | 323 |
| `endpoint.decorate_mcp()` | 174 | `endpoint.decorate_api()` | 331 |
| `endpoint.is_monthly_request_limit()` | 205 | `endpoint.is_monthly_request_limit()` | 364 |
| `endpoint.is_bearer_token_invalid()` | 222 | | |
| `endpoint.is_bearer_token_invalid()` | 408 | | |

**调用上下文**：
- MCP 调用：`call_mcp_with_retry()` （lines 129-271）
- API 调用：`call_api_with_retry()` （lines 279-491）

**调用方向**：
- `endpoint_for()` 方法选择实现（line 97-109）
- 获取的 `Arc<dyn KiroEndpoint>` 被立即调用

### 其他模块

无其他模块直接调用 trait 方法。trait 是 `provider.rs` 的内部消费者。

---

## 重构影响范围总结

### 待折叠的方法

当前分散的方法需要整合为两个核心方法：

1. **build_request()** 可能取代：
   - `api_url()`
   - `mcp_url()`
   - `decorate_api()`
   - `decorate_mcp()`
   - `transform_api_body()`
   - `transform_mcp_body()`

2. **classify_error()** 可能取代：
   - `is_monthly_request_limit()`
   - `is_bearer_token_invalid()`

### 新枚举的驱动

**KiroRequest** 需要编码当前由调用端区分的请求类型：
```
GenerateAssistant { body: &'a str, stream: bool }  // API 调用 (api_url, decorate_api, transform_api_body)
Mcp { body: &'a str }                              // MCP 调用 (mcp_url, decorate_mcp, transform_mcp_body)
UsageLimits                                        // 可能的额外请求类型？
```

**EndpointErrorKind** 需要编码当前的错误分类逻辑：
```
MonthlyQuotaExhausted          // is_monthly_request_limit() + 402 status
BearerTokenInvalid            // is_bearer_token_invalid() + 401/403 status
BadRequest                     // 400 status
ClientError                    // 4xx 其他
Transient                      // 408/429/5xx status
Unknown                        // 其他
```

### 改造点

**provider.rs** 的 `call_api_with_retry()` 和 `call_mcp_with_retry()` 需要重写以使用新接口。

---

## 附注

本研究基于当前 master 分支代码（最近提交 35a7c93）。CLI 端点的不存在建议该提交可能是计划的（trait 抽象已就位），但实现尚未开始。
