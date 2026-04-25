//! Domain layer：抽象 trait + 数据 + 错误
//!
//! Phase 1 阶段为占位，真实实现见 Phase 2-4。

#![allow(dead_code)]

pub mod credential;
pub mod endpoint;
pub mod error;
pub mod event;
pub mod retry;
pub mod selector;
pub mod token;
