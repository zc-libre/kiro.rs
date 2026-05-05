//! Kiro IDE 端点（迁移自 `kiro::endpoint::ide`）
//!
//! - API: `https://q.{api_region}.amazonaws.com/generateAssistantResponse`
//! - MCP: `https://q.{api_region}.amazonaws.com/mcp`

use uuid::Uuid;

use crate::domain::endpoint::{KiroEndpoint, RequestContext};

pub const IDE_ENDPOINT_NAME: &str = "ide";

pub struct IdeEndpoint;

impl IdeEndpoint {
    pub fn new() -> Self {
        Self
    }

    fn api_region<'a>(&self, ctx: &'a RequestContext<'_>) -> &'a str {
        ctx.credentials.effective_api_region(ctx.config)
    }

    fn host(&self, ctx: &RequestContext<'_>) -> String {
        format!("q.{}.amazonaws.com", self.api_region(ctx))
    }

    fn x_amz_user_agent(&self, ctx: &RequestContext<'_>) -> String {
        format!(
            "aws-sdk-js/1.0.34 KiroIDE-{}-{}",
            ctx.config.kiro.kiro_version, ctx.machine_id
        )
    }

    fn user_agent(&self, ctx: &RequestContext<'_>) -> String {
        format!(
            "aws-sdk-js/1.0.34 ua/2.1 os/{} lang/js md/nodejs#{} api/codewhispererstreaming#1.0.34 m/E KiroIDE-{}-{}",
            ctx.config.kiro.system_version,
            ctx.config.kiro.node_version,
            ctx.config.kiro.kiro_version,
            ctx.machine_id
        )
    }
}

impl Default for IdeEndpoint {
    fn default() -> Self {
        Self::new()
    }
}

impl KiroEndpoint for IdeEndpoint {
    fn name(&self) -> &'static str {
        IDE_ENDPOINT_NAME
    }

    fn api_url(&self, ctx: &RequestContext<'_>) -> String {
        format!(
            "https://q.{}.amazonaws.com/generateAssistantResponse",
            self.api_region(ctx)
        )
    }

    fn mcp_url(&self, ctx: &RequestContext<'_>) -> String {
        format!("https://q.{}.amazonaws.com/mcp", self.api_region(ctx))
    }

    fn api_headers(&self, ctx: &RequestContext<'_>) -> Vec<(String, String)> {
        let mut h = vec![
            ("x-amzn-codewhisperer-optout".into(), "true".into()),
            ("x-amzn-kiro-agent-mode".into(), "vibe".into()),
            ("x-amz-user-agent".into(), self.x_amz_user_agent(ctx)),
            ("user-agent".into(), self.user_agent(ctx)),
            ("host".into(), self.host(ctx)),
            ("amz-sdk-invocation-id".into(), Uuid::new_v4().to_string()),
            ("amz-sdk-request".into(), "attempt=1; max=3".into()),
            ("Authorization".into(), format!("Bearer {}", ctx.token)),
        ];
        if ctx.credentials.is_api_key_credential() {
            h.push(("tokentype".into(), "API_KEY".into()));
        }
        h
    }

    fn mcp_headers(&self, ctx: &RequestContext<'_>) -> Vec<(String, String)> {
        let mut h = vec![
            ("x-amz-user-agent".into(), self.x_amz_user_agent(ctx)),
            ("user-agent".into(), self.user_agent(ctx)),
            ("host".into(), self.host(ctx)),
            ("amz-sdk-invocation-id".into(), Uuid::new_v4().to_string()),
            ("amz-sdk-request".into(), "attempt=1; max=3".into()),
            ("Authorization".into(), format!("Bearer {}", ctx.token)),
        ];
        if let Some(ref arn) = ctx.credentials.profile_arn {
            h.push(("x-amzn-kiro-profile-arn".into(), arn.clone()));
        }
        if ctx.credentials.is_api_key_credential() {
            h.push(("tokentype".into(), "API_KEY".into()));
        }
        h
    }

    fn transform_api_body(&self, body: &str, ctx: &RequestContext<'_>) -> String {
        inject_profile_arn(body, &ctx.credentials.profile_arn)
    }
}

fn inject_profile_arn(request_body: &str, profile_arn: &Option<String>) -> String {
    if let Some(arn) = profile_arn
        && let Ok(mut json) = serde_json::from_str::<serde_json::Value>(request_body)
    {
        json["profileArn"] = serde_json::Value::String(arn.clone());
        if let Ok(body) = serde_json::to_string(&json) {
            return body;
        }
    }
    request_body.to_string()
}

#[cfg(test)]
mod tests {
    use super::inject_profile_arn;
    use serde_json::Value;

    #[test]
    fn test_inject_profile_arn_with_some() {
        let body = r#"{"conversationState":{"conversationId":"c1"}}"#;
        let arn = Some("arn:aws:codewhisperer:us-east-1:123:profile/ABC".to_string());
        let result = inject_profile_arn(body, &arn);
        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            json["profileArn"],
            "arn:aws:codewhisperer:us-east-1:123:profile/ABC"
        );
        assert_eq!(json["conversationState"]["conversationId"], "c1");
    }

    #[test]
    fn test_inject_profile_arn_with_none() {
        let body = r#"{"conversationState":{"conversationId":"c1"}}"#;
        let result = inject_profile_arn(body, &None);
        let json: Value = serde_json::from_str(&result).unwrap();
        assert!(json.get("profileArn").is_none());
        assert_eq!(json["conversationState"]["conversationId"], "c1");
    }

    #[test]
    fn test_inject_profile_arn_overwrites_existing() {
        let body = r#"{"conversationState":{},"profileArn":"old-arn"}"#;
        let arn = Some("new-arn".to_string());
        let result = inject_profile_arn(body, &arn);
        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["profileArn"], "new-arn");
    }

    #[test]
    fn test_inject_profile_arn_invalid_json() {
        let body = "not-valid-json";
        let arn = Some("arn:test".to_string());
        let result = inject_profile_arn(body, &arn);
        assert_eq!(result, "not-valid-json");
    }
}
