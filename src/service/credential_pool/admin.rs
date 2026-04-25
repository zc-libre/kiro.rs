//! AdminPoolError + AdminSnapshot：CredentialPool admin 操作的错误与视图类型
//!
//! 真实方法实现见 `pool.rs` 末尾的 admin 扩展 impl 块。

use thiserror::Error;

use crate::domain::error::{ConfigError, RefreshError};

/// CredentialPool admin 操作错误
///
/// 取代旧 anyhow + msg.contains() 字符串匹配模式：admin service 直接通过 1-1
/// 映射转换为 AdminServiceError。
#[derive(Debug, Error)]
pub enum AdminPoolError {
    #[error("凭据不存在: {0}")]
    NotFound(u64),

    #[error("凭据 #{0} 因配置无效被禁用，请修正配置后重启服务")]
    DisabledByInvalidConfig(u64),

    #[error("只能删除已禁用的凭据（请先禁用凭据 #{0}）")]
    NotDisabled(u64),

    #[error("凭据已存在（refreshToken 重复）")]
    DuplicateRefreshToken,

    #[error("凭据已存在（kiroApiKey 重复）")]
    DuplicateApiKey,

    #[error("缺少 refreshToken")]
    MissingRefreshToken,

    #[error("refreshToken 为空")]
    EmptyRefreshToken,

    #[error(
        "refreshToken 已被截断（长度: {0} 字符）。\
         这通常是 Kiro IDE 为了防止凭证被第三方工具使用而故意截断的。"
    )]
    TruncatedRefreshToken(usize),

    #[error("缺少 kiroApiKey")]
    MissingApiKey,

    #[error("kiroApiKey 为空")]
    EmptyApiKey,

    #[error("API Key 凭据不支持刷新 Token")]
    ApiKeyNotRefreshable,

    #[error("Token 刷新失败: {0}")]
    Refresh(#[from] RefreshError),

    #[error("配置持久化失败: {0}")]
    Config(#[from] ConfigError),

    #[error("上游 HTTP {status}: {body}")]
    UpstreamHttp { status: u16, body: String },

    #[error("上游网络错误: {0}")]
    Network(String),

    #[error("无效模式: {0}")]
    InvalidMode(String),
}

/// 单条凭据 admin 视图
#[derive(Debug, Clone)]
pub struct AdminEntrySnapshot {
    pub id: u64,
    pub priority: u32,
    pub disabled: bool,
    pub failure_count: u32,
    pub auth_method: Option<String>,
    pub has_profile_arn: bool,
    pub expires_at: Option<String>,
    pub refresh_token_hash: Option<String>,
    pub api_key_hash: Option<String>,
    pub masked_api_key: Option<String>,
    pub email: Option<String>,
    pub success_count: u64,
    pub last_used_at: Option<String>,
    pub has_proxy: bool,
    pub proxy_url: Option<String>,
    pub refresh_failure_count: u32,
    pub disabled_reason: Option<String>,
    pub endpoint: Option<String>,
}

/// AdminService.get_all_credentials 的数据视图
#[derive(Debug, Clone)]
pub struct AdminSnapshot {
    pub entries: Vec<AdminEntrySnapshot>,
    pub current_id: u64,
    pub total: usize,
    pub available: usize,
}
