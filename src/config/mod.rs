//! 应用配置（serde flatten 子结构，JSON 仍为单层 camelCase）
//!
//! Breaking Change：相比 master 删除了 `countTokensApiUrl` / `countTokensApiKey` /
//! `countTokensAuthType` 三字段。旧字段加载时被 serde 默认忽略，首次 [`Config::save`]
//! 后字段从文件中消失。

pub mod admin;
pub mod endpoint;
pub mod feature;
pub mod kiro;
pub mod net;
pub mod proxy;
pub mod region;

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use admin::AdminConfig;
pub use endpoint::EndpointConfig;
pub use feature::FeatureFlags;
pub use kiro::KiroIdentity;
pub use net::NetConfig;
pub use proxy::GlobalProxyConfig;
pub use region::RegionConfig;

use crate::domain::error::ConfigError;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(flatten)]
    pub net: NetConfig,

    #[serde(flatten)]
    pub region: RegionConfig,

    #[serde(flatten)]
    pub kiro: KiroIdentity,

    #[serde(flatten)]
    pub proxy: GlobalProxyConfig,

    #[serde(flatten)]
    pub admin: AdminConfig,

    #[serde(flatten)]
    pub endpoint: EndpointConfig,

    #[serde(flatten)]
    pub features: FeatureFlags,

    /// 对外 Anthropic 路由的客户端鉴权 key
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    #[serde(skip)]
    config_path: Option<PathBuf>,
}

impl Config {
    pub fn default_config_path() -> &'static str {
        "config.json"
    }

    /// 有效 Auth Region（用于 Token 刷新）
    pub fn effective_auth_region(&self) -> &str {
        self.region.effective_auth()
    }

    /// 有效 API Region（用于 API 请求）
    pub fn effective_api_region(&self) -> &str {
        self.region.effective_api()
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self {
                config_path: Some(path.to_path_buf()),
                ..Self::default()
            });
        }
        let content = fs::read_to_string(path)?;
        let mut config: Config = serde_json::from_str(&content)?;
        config.config_path = Some(path.to_path_buf());
        Ok(config)
    }

    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        let path = self
            .config_path
            .as_deref()
            .ok_or_else(|| ConfigError::Validation("配置文件路径未知，无法保存配置".into()))?;
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    const FIXTURE_MIN: &str = include_str!("tests/fixtures/config_minimum.json");
    const FIXTURE_FULL: &str = include_str!("tests/fixtures/config_full.json");
    const FIXTURE_LEGACY: &str =
        include_str!("tests/fixtures/config_legacy_with_count_tokens_api.json");

    fn tmp_path(tag: &str) -> PathBuf {
        let id = uuid::Uuid::new_v4();
        std::env::temp_dir().join(format!("kiro-rs-config-test-{tag}-{id}.json"))
    }

    #[test]
    fn load_minimum_uses_defaults() {
        let cfg: Config = serde_json::from_str(FIXTURE_MIN).expect("parse minimum");
        assert_eq!(cfg.net.host, "127.0.0.1");
        assert_eq!(cfg.net.port, 8080);
        assert_eq!(cfg.region.region, "us-east-1");
        assert_eq!(cfg.endpoint.default_endpoint, "ide");
        assert!(cfg.features.extract_thinking);
        assert_eq!(cfg.features.load_balancing_mode, "priority");
    }

    #[test]
    fn load_full_populates_all_groups() {
        let cfg: Config = serde_json::from_str(FIXTURE_FULL).expect("parse full");
        assert_eq!(cfg.net.host, "127.0.0.1");
        assert_eq!(cfg.net.port, 8990);
        assert_eq!(cfg.region.region, "us-east-1");
        assert_eq!(cfg.region.auth_region.as_deref(), Some("us-east-1"));
        assert_eq!(cfg.region.api_region.as_deref(), Some("us-east-1"));
        assert!(cfg.admin.admin_api_key.is_some());
        assert_eq!(
            cfg.proxy.proxy_url.as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(cfg.proxy.proxy_username.as_deref(), Some("user"));
        assert_eq!(cfg.proxy.proxy_password.as_deref(), Some("pass"));
        assert_eq!(cfg.kiro.kiro_version, "0.9.2");
        assert!(cfg.kiro.machine_id.is_some());
        assert_eq!(cfg.endpoint.default_endpoint, "ide");
        assert!(cfg.api_key.is_some());
    }

    #[test]
    fn load_legacy_with_count_tokens_api_silently_drops_three_fields() {
        let cfg: Config =
            serde_json::from_str(FIXTURE_LEGACY).expect("legacy parse should succeed");
        assert!(cfg.admin.admin_api_key.is_some());
        let serialized = serde_json::to_string(&cfg).expect("serialize");
        assert!(!serialized.contains("countTokensApiUrl"));
        assert!(!serialized.contains("countTokensApiKey"));
        assert!(!serialized.contains("countTokensAuthType"));
    }

    #[test]
    fn legacy_load_then_save_drops_count_tokens_fields_in_file() {
        let path = tmp_path("legacy-save");
        fs::write(&path, FIXTURE_LEGACY).unwrap();
        let cfg = Config::load(&path).expect("load");
        cfg.save().expect("save");
        let after = fs::read_to_string(&path).unwrap();
        assert!(
            !after.contains("countTokensApiUrl")
                && !after.contains("countTokensApiKey")
                && !after.contains("countTokensAuthType"),
            "save 后老字段必须消失：{after}"
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn config_default_roundtrip_is_equivalent() {
        let cfg = Config::default();
        let s = serde_json::to_string(&cfg).expect("serialize default");
        let back: Config = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(cfg.net.host, back.net.host);
        assert_eq!(cfg.net.port, back.net.port);
        assert_eq!(cfg.region.region, back.region.region);
        assert_eq!(
            cfg.endpoint.default_endpoint,
            back.endpoint.default_endpoint
        );
        assert_eq!(cfg.features.extract_thinking, back.features.extract_thinking);
        assert_eq!(
            cfg.features.load_balancing_mode,
            back.features.load_balancing_mode
        );
    }

    #[test]
    fn effective_auth_region_falls_back_to_region() {
        let cfg = Config::default();
        assert_eq!(cfg.effective_auth_region(), "us-east-1");
        assert_eq!(cfg.effective_api_region(), "us-east-1");
    }

    #[test]
    fn effective_auth_region_uses_override_when_set() {
        let mut cfg = Config::default();
        cfg.region.auth_region = Some("us-east-2".to_string());
        cfg.region.api_region = Some("eu-west-1".to_string());
        assert_eq!(cfg.effective_auth_region(), "us-east-2");
        assert_eq!(cfg.effective_api_region(), "eu-west-1");
    }
}
