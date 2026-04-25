//! 使用额度查询数据模型（迁移自 `kiro::model::usage_limits`）
//!
//! Anthropic Admin 端的余额查询直接使用此结构。

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageLimitsResponse {
    #[serde(default)]
    pub next_date_reset: Option<f64>,

    #[serde(default)]
    pub subscription_info: Option<SubscriptionInfo>,

    #[serde(default)]
    pub usage_breakdown_list: Vec<UsageBreakdown>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionInfo {
    #[serde(default)]
    pub subscription_title: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct UsageBreakdown {
    #[serde(default)]
    pub current_usage: i64,

    #[serde(default)]
    pub current_usage_with_precision: f64,

    #[serde(default)]
    pub bonuses: Vec<Bonus>,

    #[serde(default)]
    pub free_trial_info: Option<FreeTrialInfo>,

    #[serde(default)]
    pub next_date_reset: Option<f64>,

    #[serde(default)]
    pub usage_limit: i64,

    #[serde(default)]
    pub usage_limit_with_precision: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bonus {
    #[serde(default)]
    pub current_usage: f64,

    #[serde(default)]
    pub usage_limit: f64,

    #[serde(default)]
    pub status: Option<String>,
}

impl Bonus {
    pub fn is_active(&self) -> bool {
        self.status
            .as_deref()
            .map(|s| s == "ACTIVE")
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct FreeTrialInfo {
    #[serde(default)]
    pub current_usage: i64,

    #[serde(default)]
    pub current_usage_with_precision: f64,

    #[serde(default)]
    pub free_trial_expiry: Option<f64>,

    #[serde(default)]
    pub free_trial_status: Option<String>,

    #[serde(default)]
    pub usage_limit: i64,

    #[serde(default)]
    pub usage_limit_with_precision: f64,
}

impl FreeTrialInfo {
    pub fn is_active(&self) -> bool {
        self.free_trial_status
            .as_deref()
            .map(|s| s == "ACTIVE")
            .unwrap_or(false)
    }
}

impl UsageLimitsResponse {
    pub fn subscription_title(&self) -> Option<&str> {
        self.subscription_info
            .as_ref()
            .and_then(|info| info.subscription_title.as_deref())
    }

    fn primary_breakdown(&self) -> Option<&UsageBreakdown> {
        self.usage_breakdown_list.first()
    }

    pub fn usage_limit(&self) -> f64 {
        let Some(breakdown) = self.primary_breakdown() else {
            return 0.0;
        };
        let mut total = breakdown.usage_limit_with_precision;
        if let Some(trial) = &breakdown.free_trial_info
            && trial.is_active() {
                total += trial.usage_limit_with_precision;
            }
        for bonus in &breakdown.bonuses {
            if bonus.is_active() {
                total += bonus.usage_limit;
            }
        }
        total
    }

    pub fn current_usage(&self) -> f64 {
        let Some(breakdown) = self.primary_breakdown() else {
            return 0.0;
        };
        let mut total = breakdown.current_usage_with_precision;
        if let Some(trial) = &breakdown.free_trial_info
            && trial.is_active() {
                total += trial.current_usage_with_precision;
            }
        for bonus in &breakdown.bonuses {
            if bonus.is_active() {
                total += bonus.current_usage;
            }
        }
        total
    }
}
