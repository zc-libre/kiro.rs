//! Kiro 端点抽象（迁移自 `kiro::endpoint`）
//!
//! 不同 Kiro 端点（如 `ide` / `cli`）在 URL、请求头、请求体上存在差异，
//! 但共享凭据池、Token 刷新、重试逻辑和 AWS event-stream 响应解码。

use reqwest::RequestBuilder;

use crate::config::Config;
use crate::domain::credential::Credential;

/// 装饰请求时可用的上下文
pub struct RequestContext<'a> {
    pub credentials: &'a Credential,
    /// 有效的 access token（API Key 凭据下即 kiroApiKey）
    pub token: &'a str,
    /// 当前凭据对应的 machineId
    pub machine_id: &'a str,
    /// 全局配置
    pub config: &'a Config,
}

/// Kiro 端点
pub trait KiroEndpoint: Send + Sync {
    fn name(&self) -> &'static str;
    fn api_url(&self, ctx: &RequestContext<'_>) -> String;
    fn mcp_url(&self, ctx: &RequestContext<'_>) -> String;
    fn decorate_api(&self, req: RequestBuilder, ctx: &RequestContext<'_>) -> RequestBuilder;
    fn decorate_mcp(&self, req: RequestBuilder, ctx: &RequestContext<'_>) -> RequestBuilder;
    fn transform_api_body(&self, body: &str, ctx: &RequestContext<'_>) -> String;
    fn transform_mcp_body(&self, body: &str, _ctx: &RequestContext<'_>) -> String {
        body.to_string()
    }
}
