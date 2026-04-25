//! 全局代理配置

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalProxyConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_username: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_password: Option<String>,
}
