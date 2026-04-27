//! conversation 流处理过程中的致命错误（fail-fast）
//!
//! 一旦产生 [`FatalKiroError`]，handler 必须立刻终止流，向客户端发送一条标准
//! Anthropic `event: error` SSE（流式）或返回 502 + ErrorResponse（非流式），
//! 不再补 `message_stop` 等正常收尾事件——因为流的内容完整性已不可信。
//!
//! 与 [`crate::domain::error::KiroError`] 不同，本类型只承载 SSE 转换层的失败，
//! 不重叠上游 HTTP / 凭据 / 网络错误。

use http::StatusCode;
use thiserror::Error;

/// SSE 流转换过程中的致命错误
#[derive(Debug, Error)]
pub enum FatalKiroError {
    /// 解码缓冲区溢出（`EventStreamDecoder::feed` 失败）
    #[error("decoder buffer overflow: {0}")]
    BufferOverflow(String),

    /// 帧级解码失败（CRC 错位 / 缓冲区损坏 / 累计错误超阈值）
    #[error("event stream decode failed: {0}")]
    DecodeFailed(String),

    /// 帧 → Event 反序列化失败（协议层契约不一致）
    #[error("event payload parse failed: {0}")]
    EventParseFailed(String),

    /// 上游显式 Error 事件
    #[error("upstream error: {error_code}: {error_message}")]
    UpstreamError {
        error_code: String,
        error_message: String,
    },

    /// 上游显式 Exception 事件（不含 ContentLengthExceededException，那个是 max_tokens 正常停止）
    #[error("upstream exception: {exception_type}: {message}")]
    UpstreamException {
        exception_type: String,
        message: String,
    },

    /// 上游响应字节流读取失败（reqwest 层）
    #[error("upstream body stream read failed: {0}")]
    UpstreamBodyRead(String),
}

impl FatalKiroError {
    /// 用于结构化日志的稳定标签
    pub fn kind(&self) -> &'static str {
        match self {
            Self::BufferOverflow(_) => "buffer_overflow",
            Self::DecodeFailed(_) => "decode_failed",
            Self::EventParseFailed(_) => "event_parse_failed",
            Self::UpstreamError { .. } => "upstream_error",
            Self::UpstreamException { .. } => "upstream_exception",
            Self::UpstreamBodyRead(_) => "upstream_body_read",
        }
    }

    /// 暴露给客户端的 message
    ///
    /// - 上游 Error/Exception：透传上游语义，便于客户端排错。
    /// - 本地解码层错误：返回泛化文案，避免内部细节泄漏；完整 Display 走 `tracing::error!`。
    pub fn client_message(&self) -> String {
        match self {
            Self::UpstreamError {
                error_code,
                error_message,
            } => format!("upstream error: {error_code}: {error_message}"),
            Self::UpstreamException {
                exception_type,
                message,
            } => format!("upstream exception: {exception_type}: {message}"),
            Self::BufferOverflow(_) => "Stream decode failed: buffer overflow".into(),
            Self::DecodeFailed(_) => "Stream decode failed".into(),
            Self::EventParseFailed(_) => "Stream decode failed: event parse error".into(),
            Self::UpstreamBodyRead(_) => "Upstream body read failed".into(),
        }
    }

    /// Anthropic 响应中的 `error.type` 字段
    pub fn anthropic_error_type(&self) -> &'static str {
        "api_error"
    }

    /// 非流式响应使用的 HTTP 状态码
    pub fn http_status(&self) -> StatusCode {
        StatusCode::BAD_GATEWAY
    }
}

/// 判断上游 `Event::Exception` 是否需要按 fatal 处理。
///
/// `ContentLengthExceededException` 是正常停止信号（映射到 `stop_reason = "max_tokens"`），
/// 其余 exception_type 一律视为 fatal——内容已不可信，必须终止流并通知客户端。
///
/// 在 stream / non-stream 两条路径共用，避免分叉。
pub fn is_fatal_exception(exception_type: &str) -> bool {
    exception_type != "ContentLengthExceededException"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_returns_stable_tag() {
        assert_eq!(
            FatalKiroError::BufferOverflow("x".into()).kind(),
            "buffer_overflow"
        );
        assert_eq!(
            FatalKiroError::DecodeFailed("x".into()).kind(),
            "decode_failed"
        );
        assert_eq!(
            FatalKiroError::EventParseFailed("x".into()).kind(),
            "event_parse_failed"
        );
        assert_eq!(
            FatalKiroError::UpstreamError {
                error_code: "C".into(),
                error_message: "M".into()
            }
            .kind(),
            "upstream_error"
        );
        assert_eq!(
            FatalKiroError::UpstreamException {
                exception_type: "T".into(),
                message: "M".into()
            }
            .kind(),
            "upstream_exception"
        );
        assert_eq!(
            FatalKiroError::UpstreamBodyRead("x".into()).kind(),
            "upstream_body_read"
        );
    }

    #[test]
    fn client_message_passes_through_upstream_payload() {
        let err = FatalKiroError::UpstreamError {
            error_code: "RateLimited".into(),
            error_message: "too many requests".into(),
        };
        let msg = err.client_message();
        assert!(msg.contains("RateLimited"), "got: {msg}");
        assert!(msg.contains("too many requests"), "got: {msg}");
    }

    #[test]
    fn client_message_redacts_local_decode_internals() {
        // 内部细节（如缓冲区大小、字节坐标）不应出现在对外 message 中
        let err = FatalKiroError::BufferOverflow("size 16777217 > max 16777216".into());
        let msg = err.client_message();
        assert!(!msg.contains("16777216"), "internal detail leaked: {msg}");
        assert!(msg.contains("buffer overflow"), "got: {msg}");
    }

    #[test]
    fn http_status_is_bad_gateway() {
        let err = FatalKiroError::DecodeFailed("x".into());
        assert_eq!(err.http_status(), StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn anthropic_error_type_is_api_error() {
        let err = FatalKiroError::DecodeFailed("x".into());
        assert_eq!(err.anthropic_error_type(), "api_error");
    }

    #[test]
    fn is_fatal_exception_excludes_content_length_exceeded() {
        assert!(!is_fatal_exception("ContentLengthExceededException"));
        assert!(is_fatal_exception("ThrottlingException"));
        assert!(is_fatal_exception("InternalServerException"));
        assert!(is_fatal_exception(""));
    }
}
