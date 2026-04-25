//! CredentialSelector trait + CredentialView（占位，Phase 2 实现 Priority/Balanced）

#![allow(dead_code)]

use crate::domain::credential::Credential;

/// 凭据状态只读视图（Phase 2 由 service::credential_pool::state 拼装）
#[derive(Debug)]
pub struct CredentialStateView {
    pub disabled: bool,
}

#[derive(Debug)]
pub struct CredentialStatsView {
    pub success_count: u64,
}

/// 选凭据时的只读组合视图
///
/// `select` 在 store/state/stats 三把锁持有期内构造，禁止跨 `.await`、禁止再获取其他锁。
pub struct CredentialView<'a> {
    pub id: u64,
    pub credential: &'a Credential,
    pub state: &'a CredentialStateView,
    pub stats: &'a CredentialStatsView,
}

pub trait CredentialSelector: Send + Sync {
    /// 必须同步、纯计算
    fn select(&self, candidates: &[CredentialView<'_>], model: Option<&str>) -> Option<u64>;
}
