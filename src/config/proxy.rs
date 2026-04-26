//! 全局代理配置

use serde::{Deserialize, Serialize};

use crate::infra::http::client::ProxyConfig;

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

impl GlobalProxyConfig {
    /// 把全局代理配置转换为 [`ProxyConfig`]；`proxy_url` 为空返回 `None`。
    /// `username/password` 必须同时提供才会附加 basic auth。
    pub fn to_proxy_config(&self) -> Option<ProxyConfig> {
        let url = self.proxy_url.as_deref()?;
        let mut proxy = ProxyConfig::new(url);
        if let (Some(u), Some(pw)) = (
            self.proxy_username.as_deref(),
            self.proxy_password.as_deref(),
        ) {
            proxy = proxy.with_auth(u, pw);
        }
        Some(proxy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_proxy_config_returns_none_when_url_missing() {
        let cfg = GlobalProxyConfig::default();
        assert_eq!(cfg.to_proxy_config(), None);
    }

    #[test]
    fn to_proxy_config_includes_auth_when_username_password_present() {
        let cfg = GlobalProxyConfig {
            proxy_url: Some("http://proxy:3128".into()),
            proxy_username: Some("user".into()),
            proxy_password: Some("pwd".into()),
        };
        let expected = ProxyConfig::new("http://proxy:3128").with_auth("user", "pwd");
        assert_eq!(cfg.to_proxy_config(), Some(expected));
    }

    #[test]
    fn to_proxy_config_omits_auth_when_either_missing() {
        let only_user = GlobalProxyConfig {
            proxy_url: Some("http://proxy:3128".into()),
            proxy_username: Some("user".into()),
            proxy_password: None,
        };
        assert_eq!(
            only_user.to_proxy_config(),
            Some(ProxyConfig::new("http://proxy:3128"))
        );

        let only_pwd = GlobalProxyConfig {
            proxy_url: Some("http://proxy:3128".into()),
            proxy_username: None,
            proxy_password: Some("pwd".into()),
        };
        assert_eq!(
            only_pwd.to_proxy_config(),
            Some(ProxyConfig::new("http://proxy:3128"))
        );
    }
}
