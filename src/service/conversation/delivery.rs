//! SseDelivery：Live / Buffered 两种推送策略
//!
//! 当前实现委托给 [`super::stream::StreamContext`] 与 [`super::stream::BufferedStreamContext`]。
//! Phase 4 不重写 1989 行 stream.rs；Phase 7 优化时可把这两个 Context 拆为独立 trait 实现。

#![allow(dead_code, unused_imports)]

pub use super::stream::{BufferedStreamContext as BufferedDelivery, StreamContext as LiveDelivery};

/// SseDelivery 策略类型枚举（与 handler 参数化匹配）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryMode {
    /// 实时推送：每收到事件立即推 SSE
    Live,
    /// 缓冲推送：等流结束后批量推送（修正 input_tokens 后）
    Buffered,
}
