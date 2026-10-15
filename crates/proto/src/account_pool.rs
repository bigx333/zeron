//! Device-local account routing. These types contain no credentials or hub tokens.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccountPoolRoutingMode {
    #[default]
    WeeklyReset,
    Sequential,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct AccountPoolProviderConfig {
    pub enabled: bool,
    pub routing_mode: AccountPoolRoutingMode,
    pub switch_threshold: f32,
    pub account_order: Vec<String>,
    pub disabled_account_ids: Vec<String>,
}

impl Default for AccountPoolProviderConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            routing_mode: AccountPoolRoutingMode::WeeklyReset,
            switch_threshold: 0.98,
            account_order: Vec::new(),
            disabled_account_ids: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct AccountPoolConfig {
    pub claude: AccountPoolProviderConfig,
    pub codex: AccountPoolProviderConfig,
    pub bypassed_chat_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AccountPoolSnapshot {
    pub config: AccountPoolConfig,
    pub providers: Vec<AccountPoolProviderStatus>,
    pub accounts: Vec<AccountPoolAccountStatus>,
}

impl AccountPoolSnapshot {
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
            && self.accounts.is_empty()
            && self.config == AccountPoolConfig::default()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountPoolProviderStatus {
    pub harness: crate::HarnessId,
    pub serving: bool,
    pub next_account_id: Option<String>,
    #[serde(default)]
    pub next_account_ids: std::collections::HashMap<String, String>,
    pub notice: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccountPoolStatus {
    Next,
    Ready,
    Limited,
    FamilyLimited,
    Held,
    NeedsSignIn,
    LiveExpired,
    Excluded,
    Unsupported,
}

/// Shared model-family vocabulary for quota routing and the composer meter.
pub fn model_family(model: &str) -> &'static str {
    let model = model.to_ascii_lowercase();
    ["fable", "sonnet", "opus", "haiku"]
        .into_iter()
        .find(|family| model.contains(family))
        .unwrap_or("other")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountPoolAccountStatus {
    pub account_id: String,
    pub status: AccountPoolStatus,
    pub until: Option<DateTime<Utc>>,
    pub family: Option<String>,
    pub weekly_reset_at: Option<DateTime<Utc>>,
    pub poolable: bool,
    pub unpoolable_reason: Option<String>,
    pub held_until: Option<i64>,
    pub error: Option<String>,
    pub last_used_at: Option<i64>,
    pub in_flight: usize,
}
