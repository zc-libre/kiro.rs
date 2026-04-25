//! Region 三级配置：region / authRegion / apiRegion

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionConfig {
    #[serde(default = "default_region")]
    pub region: String,

    /// Auth Region（用于 Token 刷新），未配置时回退到 region
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_region: Option<String>,

    /// API Region（用于 API 请求），未配置时回退到 region
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_region: Option<String>,
}

impl Default for RegionConfig {
    fn default() -> Self {
        Self {
            region: default_region(),
            auth_region: None,
            api_region: None,
        }
    }
}

impl RegionConfig {
    pub fn effective_auth(&self) -> &str {
        self.auth_region.as_deref().unwrap_or(&self.region)
    }

    pub fn effective_api(&self) -> &str {
        self.api_region.as_deref().unwrap_or(&self.region)
    }
}

fn default_region() -> String {
    "us-east-1".to_string()
}
