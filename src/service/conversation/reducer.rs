//! EventReducer：Kiro 事件 → Anthropic SSE 事件
//!
//! 当前实现是 [`super::stream::SseStateManager`] 的别名 + new methods。
//! Phase 4 不深度拆分 stream.rs 的 1989 行（现状能跑通；契约由 Phase 8 真实启动冒烟保护）；
//! Phase 7 优化时可把 reducer 状态机从 stream.rs 完整剥离。

#![allow(dead_code, unused_imports)]

pub use super::stream::{SseEvent, SseStateManager as EventReducer};
