//! KiroEndpoint trait（占位，Phase 2 完整迁移自 `kiro::endpoint`）

#![allow(dead_code)]

use crate::domain::credential::Credential;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointKind {
    Api,
    Mcp,
}

/// 单次请求的上下文
pub struct RequestContext<'a> {
    pub credential: &'a Credential,
    pub token: &'a str,
    pub machine_id: &'a str,
}

/// Kiro 端点抽象
pub trait KiroEndpoint: Send + Sync {
    fn name(&self) -> &'static str;
    fn url(&self, kind: EndpointKind, ctx: &RequestContext<'_>) -> String;
    fn transform_body(&self, kind: EndpointKind, body: &str, ctx: &RequestContext<'_>) -> String;
    fn is_monthly_request_limit(&self, body: &str) -> bool {
        body.contains("MONTHLY_REQUEST_COUNT")
    }
    fn is_bearer_token_invalid(&self, body: &str) -> bool {
        body.contains("The bearer token included in the request is invalid")
    }
}
