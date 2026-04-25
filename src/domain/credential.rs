//! Credential 数据模型（占位，Phase 2 完整迁移自 `kiro::model::credentials::KiroCredentials`）

#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    Social,
    Idc,
    ApiKey,
}

/// 凭据数据（Phase 2 才填全字段）
#[derive(Debug, Clone)]
pub struct Credential {
    pub id: u64,
    pub auth: AuthMethod,
}
