//! Feature flags：负载均衡模式 + thinking 提取开关

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlags {
    /// 负载均衡模式（"priority" 或 "balanced"）
    #[serde(default = "default_load_balancing_mode")]
    pub load_balancing_mode: String,

    /// 是否开启非流式响应的 thinking 块提取（默认 true）
    #[serde(default = "default_extract_thinking")]
    pub extract_thinking: bool,
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self {
            load_balancing_mode: default_load_balancing_mode(),
            extract_thinking: default_extract_thinking(),
        }
    }
}

fn default_load_balancing_mode() -> String {
    "priority".to_string()
}

fn default_extract_thinking() -> bool {
    true
}
