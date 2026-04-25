//! Admin API 鉴权配置

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminConfig {
    /// Admin API 密钥（可选，启用 Admin API 功能）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_api_key: Option<String>,
}
