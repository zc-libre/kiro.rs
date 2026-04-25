//! 协议层：Anthropic ↔ Kiro 转换、流式归约、推送策略、token 估算

#![allow(dead_code)]

pub mod converter;
pub mod delivery;
pub mod reducer;
pub mod stream;
pub mod thinking;
pub mod tokens;
pub mod websearch;

pub use converter::{ConversionError, ConversionResult, convert_request, get_context_window_size, map_model};
pub use delivery::{BufferedDelivery, DeliveryMode, LiveDelivery};
pub use reducer::{EventReducer, SseEvent};
pub use thinking::extract_thinking_from_complete_text;
pub use tokens::{count_all_tokens, count_tokens, estimate_output_tokens};
