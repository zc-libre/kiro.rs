//! RetryPolicy + RetryDecision（占位，Phase 3 实现 DefaultRetryPolicy）

#![allow(dead_code)]

use std::time::Duration;

use http::StatusCode;

use crate::domain::error::ProviderError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisabledReason {
    QuotaExceeded,
    InvalidRefreshToken,
    InvalidConfig,
    TooManyFailures,
}

#[derive(Debug)]
pub enum RetryDecision {
    Retry { backoff: Duration },
    FailoverCredential,
    ForceRefresh,
    DisableCredential(DisabledReason),
    Fail(ProviderError),
    Success,
}

pub trait RetryPolicy: Send + Sync {
    fn decide(&self, status: StatusCode, body: &str, attempt: usize) -> RetryDecision;
}
