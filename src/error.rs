//! crate-level 错误 re-export
//!
//! 实际定义在 [`crate::domain::error`]。

pub use crate::domain::error::{ConfigError, KiroError, ProviderError, RefreshError};
