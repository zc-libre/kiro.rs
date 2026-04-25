//! Domain-layer 错误体系
//!
//! 分层 thiserror 错误：取代旧 anyhow + msg.contains() 字符串匹配模式。

use http::StatusCode;
use thiserror::Error;

/// Token 刷新过程中的错误
#[derive(Debug, Error)]
pub enum RefreshError {
    #[error("refresh token invalid (invalid_grant)")]
    TokenInvalid,
    #[error("refresh rate limited")]
    RateLimited,
    #[error("refresh unauthorized")]
    Unauthorized,
    #[error("refresh forbidden")]
    Forbidden,
    #[error("refresh network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("refresh server error: HTTP {0}")]
    ServerError(StatusCode),
    #[error("refresh malformed response: {0}")]
    MalformedResponse(#[from] serde_json::Error),
}

/// 上游请求错误
#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("all credentials exhausted (available {available} / total {total})")]
    AllCredentialsExhausted { available: usize, total: usize },
    #[error("Context window is full")]
    ContextWindowFull,
    #[error("input too long")]
    InputTooLong,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("upstream HTTP {status}: {body}")]
    UpstreamHttp { status: u16, body: String },
    #[error("endpoint resolution: {0}")]
    EndpointResolution(String),
}

/// 配置文件错误
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("config parse: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("config validation: {0}")]
    Validation(String),
}

/// crate-level 顶层错误
#[derive(Debug, Error)]
pub enum KiroError {
    #[error("refresh: {0}")]
    Refresh(#[from] RefreshError),
    #[error("provider: {0}")]
    Provider(#[from] ProviderError),
    #[error("endpoint: {0}")]
    Endpoint(String),
    #[error("network: {0}")]
    Network(#[from] reqwest::Error),
    #[error("config: {0}")]
    Config(#[from] ConfigError),
    #[error("storage: {0}")]
    Storage(std::io::Error),
    #[error("decode: {0}")]
    Decode(String),
}

impl KiroError {
    pub fn kind(&self) -> &'static str {
        match self {
            KiroError::Refresh(RefreshError::Network(_)) => "network",
            KiroError::Refresh(_) => "refresh",
            KiroError::Provider(_) => "provider",
            KiroError::Endpoint(_) => "endpoint",
            KiroError::Network(_) => "network",
            KiroError::Config(_) => "config",
            KiroError::Storage(_) => "storage",
            KiroError::Decode(_) => "decode",
        }
    }

    pub fn http_status_hint(&self) -> StatusCode {
        match self {
            KiroError::Provider(ProviderError::ContextWindowFull) => StatusCode::BAD_REQUEST,
            KiroError::Provider(ProviderError::InputTooLong) => StatusCode::BAD_REQUEST,
            KiroError::Provider(ProviderError::BadRequest(_)) => StatusCode::BAD_REQUEST,
            KiroError::Provider(ProviderError::EndpointResolution(_)) => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            KiroError::Provider(ProviderError::AllCredentialsExhausted { .. }) => {
                StatusCode::BAD_GATEWAY
            }
            KiroError::Refresh(RefreshError::TokenInvalid) => StatusCode::BAD_GATEWAY,
            _ => StatusCode::BAD_GATEWAY,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_error_token_invalid_display_contains_invalid_grant() {
        let s = RefreshError::TokenInvalid.to_string();
        assert!(s.contains("invalid_grant"), "got: {}", s);
    }

    #[test]
    fn provider_error_all_credentials_exhausted_display_includes_numbers() {
        let s = ProviderError::AllCredentialsExhausted {
            available: 1,
            total: 4,
        }
        .to_string();
        assert!(s.contains('1') && s.contains('4'), "got: {}", s);
    }

    #[test]
    fn provider_error_context_window_full_display() {
        let s = ProviderError::ContextWindowFull.to_string();
        assert!(s.contains("Context window is full"), "got: {}", s);
    }

    #[test]
    fn kiro_error_refresh_token_invalid_kind() {
        let e: KiroError = RefreshError::TokenInvalid.into();
        assert_eq!(e.kind(), "refresh");
    }

    #[test]
    fn from_refresh_error_to_kiro_error_compiles() {
        fn assert_from<T, U: From<T>>() {}
        assert_from::<RefreshError, KiroError>();
    }

    #[test]
    fn from_provider_error_to_kiro_error_compiles() {
        fn assert_from<T, U: From<T>>() {}
        assert_from::<ProviderError, KiroError>();
    }

    #[test]
    fn from_reqwest_error_to_kiro_error_compiles() {
        fn assert_from<T, U: From<T>>() {}
        assert_from::<reqwest::Error, KiroError>();
    }

    #[test]
    fn http_status_hint_context_window_full_is_400() {
        let e: KiroError = ProviderError::ContextWindowFull.into();
        assert_eq!(e.http_status_hint(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn http_status_hint_endpoint_resolution_is_503() {
        let e: KiroError = ProviderError::EndpointResolution("ide".into()).into();
        assert_eq!(e.http_status_hint(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn http_status_hint_default_is_502() {
        let e = KiroError::Endpoint("oops".into());
        assert_eq!(e.http_status_hint(), StatusCode::BAD_GATEWAY);
    }
}
