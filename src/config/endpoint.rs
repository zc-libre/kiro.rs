//! 端点配置：默认端点 + endpoints map

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointConfig {
    /// 默认端点名称（凭据未显式指定 endpoint 时使用，默认 "ide"）
    #[serde(default = "default_endpoint")]
    pub default_endpoint: String,

    /// 端点特定的配置（键为端点名，值为该端点自由定义的参数对象）
    #[serde(default)]
    pub endpoints: HashMap<String, serde_json::Value>,
}

impl Default for EndpointConfig {
    fn default() -> Self {
        Self {
            default_endpoint: default_endpoint(),
            endpoints: HashMap::new(),
        }
    }
}

fn default_endpoint() -> String {
    "ide".to_string()
}
