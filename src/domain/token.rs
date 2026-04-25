//! TokenSource trait（占位，Phase 2 实现 Social/Idc/ApiKey 三种）

#![allow(dead_code)]

use crate::domain::credential::Credential;
use crate::domain::error::RefreshError;

/// Token 刷新结果
#[derive(Debug, Clone)]
pub struct RefreshOutcome {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub profile_arn: Option<String>,
    pub expires_at: Option<String>,
}

/// Token 刷新策略
pub trait TokenSource: Send + Sync {
    fn refresh(
        &self,
        cred: &Credential,
    ) -> impl std::future::Future<Output = Result<RefreshOutcome, RefreshError>> + Send;
}
