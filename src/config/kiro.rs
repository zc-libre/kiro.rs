//! Kiro 客户端身份指纹（kiroVersion / machineId / systemVersion / nodeVersion）

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KiroIdentity {
    #[serde(default = "default_kiro_version")]
    pub kiro_version: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_id: Option<String>,

    #[serde(default = "default_system_version")]
    pub system_version: String,

    #[serde(default = "default_node_version")]
    pub node_version: String,
}

impl Default for KiroIdentity {
    fn default() -> Self {
        Self {
            kiro_version: default_kiro_version(),
            machine_id: None,
            system_version: default_system_version(),
            node_version: default_node_version(),
        }
    }
}

fn default_kiro_version() -> String {
    "0.11.107".to_string()
}

fn default_system_version() -> String {
    const SYSTEM_VERSIONS: &[&str] = &["darwin#24.6.0", "win32#10.0.22631"];
    SYSTEM_VERSIONS[fastrand::usize(..SYSTEM_VERSIONS.len())].to_string()
}

fn default_node_version() -> String {
    "22.22.0".to_string()
}
