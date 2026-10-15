//! Native account pool. Saved CLI slots remain the only credential registry.
//! The loopback capability is host-only; settings and observations contain no secrets.
use std::collections::{HashMap, HashSet};
use std::sync::Weak;

use serde::{Deserialize, Serialize};
use zeron_harness::{AccountPoolRoute, CancellationToken};
use zeron_proto::{
    AccountPoolAccountStatus, AccountPoolConfig, AccountPoolProviderConfig,
    AccountPoolProviderStatus, AccountPoolRoutingMode, AccountPoolSnapshot, AccountPoolStatus,
};

use super::*;

mod http;
#[cfg(test)]
mod tests;

const PIN_TTL: i64 = 30 * 60 * 1000;
const MAX_PINS: usize = 4096;

#[derive(Debug)]
enum SecretError {
    Authentication(&'static str),
    Temporary(&'static str),
}
impl From<&'static str> for SecretError {
    fn from(message: &'static str) -> Self {
        Self::Authentication(message)
    }
}
impl SecretError {
    fn message(&self) -> &'static str {
        match self {
            Self::Authentication(message) | Self::Temporary(message) => message,
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Observation {
    windows: Vec<AgentUsageWindow>,
    observed_at: i64,
    held_until: Option<i64>,
    error: Option<String>,
    error_credentials: Option<String>,
    error_at: i64,
    last_used_at: Option<i64>,
    codex_window_labels: HashMap<String, String>,
    codex_windows: HashMap<String, AgentUsageWindow>,
    replaced_window_labels: HashSet<String>,
    /// Reset metadata is useful for routing even without measured utilization.
    quota_resets: HashMap<String, DateTime<Utc>>,
    #[serde(skip)]
    in_flight: usize,
}

#[derive(Clone, Serialize, Deserialize)]
struct Pin {
    account_id: String,
    used_at: i64,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct RoutingState {
    observations: HashMap<String, Observation>,
    active: HashMap<String, String>,
    pins: HashMap<String, Pin>,
}

struct Hub {
    port: u16,
    token: String,
}

#[derive(Clone)]
struct Endpoints {
    claude: String,
    codex: String,
    openai: String,
    codex_refresh: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            claude: "https://api.anthropic.com".into(),
            codex: "https://chatgpt.com/backend-api/codex".into(),
            openai: "https://api.openai.com/v1".into(),
            codex_refresh: "https://auth.openai.com/oauth/token".into(),
        }
    }
}

pub(super) struct PoolState {
    config: Mutex<AccountPoolConfig>,
    state: Mutex<RoutingState>,
    hub: tokio::sync::Mutex<Option<Hub>>,
    cancel: CancellationToken,
    refreshes: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    refresh_tasks: Mutex<Option<tokio_util::task::TaskTracker>>,
    refresh_retry_at: Mutex<HashMap<String, i64>>,
    pending_credentials: Mutex<HashMap<String, Slot>>,
    endpoints: Mutex<Endpoints>,
    http: reqwest::Client,
    last_background_probe: Mutex<HashMap<HarnessId, i64>>,
    last_persist: Mutex<Option<Instant>>,
}

impl PoolState {
    pub(super) fn new(root: &Path) -> Self {
        let config: AccountPoolConfig = std::fs::read(root.join("pool.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .filter(|config: &AccountPoolConfig| {
                validate_config(&config.claude).is_ok() && validate_config(&config.codex).is_ok()
            })
            .unwrap_or_default();
        let state = std::fs::read(root.join("pool-state.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            config: Mutex::new(config),
            state: Mutex::new(state),
            hub: tokio::sync::Mutex::new(None),
            cancel: CancellationToken::new(),
            refreshes: Mutex::new(HashMap::new()),
            refresh_tasks: Mutex::new(Some(tokio_util::task::TaskTracker::new())),
            refresh_retry_at: Mutex::new(HashMap::new()),
            pending_credentials: Mutex::new(HashMap::new()),
            endpoints: Mutex::new(Endpoints::default()),
            last_background_probe: Mutex::new(HashMap::new()),
            last_persist: Mutex::new(None),
            http: reqwest::Client::builder()
                .http1_only()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(300))
                .build()
                .expect("account pool HTTP client"),
        }
    }

    pub(super) fn shutdown(&self) {
        self.cancel.cancel();
    }
}

fn provider(config: &AccountPoolConfig, harness: HarnessId) -> Option<&AccountPoolProviderConfig> {
    match harness {
        HarnessId::ClaudeCode => Some(&config.claude),
        HarnessId::Codex => Some(&config.codex),
        _ => None,
    }
}

fn validate_config(config: &AccountPoolProviderConfig) -> Result<(), EngineError> {
    if !config.switch_threshold.is_finite()
        || config.switch_threshold <= 0.0
        || config.switch_threshold > 1.0
    {
        return Err(EngineError::Other(
            "Switch threshold must be greater than 0 and at most 1".into(),
        ));
    }
    for ids in [&config.account_order, &config.disabled_account_ids] {
        if ids.len() > 512 || ids.iter().collect::<HashSet<_>>().len() != ids.len() {
            return Err(EngineError::Other(
                "Account lists must contain at most 512 unique IDs".into(),
            ));
        }
    }
    Ok(())
}

impl AgentAccounts {
    pub(super) fn record_claude_pool_probe(
        &self,
        slot: &Slot,
        body: &serde_json::Value,
        snapshot: &UsageSnapshot,
    ) {
        let mut state = lock(&self.inner.pool.state);
        let observation = state.observations.entry(slot.id.clone()).or_default();
        let previous = self.pool_windows(slot, observation);
        let labels: HashSet<_> = snapshot.windows.iter().map(|w| w.label.clone()).collect();
        observation.replaced_window_labels.extend(
            previous
                .into_iter()
                .map(|w| w.label)
                .filter(|label| !labels.contains(label)),
        );
        observation
            .replaced_window_labels
            .retain(|label| !labels.contains(label));
        // Full probes replace the complete metadata set, so unrelated later
        // headers cannot revive an older shared or family reset date.
        observation.quota_resets.clear();
        for (label, _, reset) in claude_usage_buckets(body) {
            if let Some(reset) = reset {
                observation
                    .quota_resets
                    .entry(label)
                    .and_modify(|current| *current = (*current).min(reset))
                    .or_insert(reset);
            }
        }
        for window in &snapshot.windows {
            if let Some(reset) = window.resets_at {
                observation.quota_resets.insert(window.label.clone(), reset);
            }
        }
        observation.windows = snapshot.windows.clone();
        observation.observed_at = now_ms();
        self.persist_pool_state(&state, false);
    }

    pub(super) fn codex_pool_labels(&self, slot: &Slot) -> HashMap<String, String> {
        lock(&self.inner.pool.state)
            .observations
            .get(&slot.id)
            .map(|observation| observation.codex_window_labels.clone())
            .unwrap_or_default()
    }

    pub(super) fn record_codex_pool_labels(
        &self,
        slot: &Slot,
        body: &serde_json::Value,
        previous: &HashMap<String, String>,
    ) {
        let mut state = lock(&self.inner.pool.state);
        let observation = state.observations.entry(slot.id.clone()).or_default();
        for (key, bucket, fallback) in [
            ("primary_window", "primary", "Primary"),
            ("secondary_window", "secondary", "Secondary"),
        ] {
            let window = body.pointer(&format!("/rate_limit/{key}"));
            if window.is_none_or(serde_json::Value::is_null) {
                if let Some(old) = observation.codex_window_labels.remove(bucket) {
                    observation.replaced_window_labels.insert(old);
                }
                observation.codex_windows.remove(bucket);
                observation.quota_resets.remove(&format!("codex:{bucket}"));
                continue;
            }
            let window = window.unwrap();
            let label = window["limit_window_seconds"]
                .as_i64()
                .filter(|span| *span > 0)
                .map(codex_window_label)
                .map(str::to_owned)
                .or_else(|| previous.get(bucket).cloned())
                .unwrap_or_else(|| fallback.into());
            if let Some(old) = observation
                .codex_window_labels
                .insert(bucket.into(), label.clone())
                && old != label
            {
                observation.replaced_window_labels.insert(old);
            }
            observation.replaced_window_labels.remove(&label);
            observation.codex_windows.remove(bucket);
            observation.quota_resets.remove(&format!("codex:{bucket}"));
            if let Some(reset) = parse_when(window.get("reset_at")) {
                observation
                    .quota_resets
                    .insert(format!("codex:{bucket}"), reset);
            }
            if let Some(used) = window.get("used_percent").and_then(quota_number) {
                observation.codex_windows.insert(
                    bucket.into(),
                    AgentUsageWindow {
                        label,
                        used_fraction: (used / 100.0).clamp(0.0, 1.0) as f32,
                        resets_at: parse_when(window.get("reset_at")),
                    },
                );
            }
        }
        self.persist_pool_state(&state, false);
    }

    pub(crate) async fn drain_pool_refreshes(&self) {
        self.inner.pool.cancel.cancel();
        let tasks = lock(&self.inner.pool.refresh_tasks).take();
        if let Some(tasks) = tasks {
            tasks.close();
            tasks.wait().await;
        }
    }

    pub(super) fn pending_pool_slot(&self, harness: HarnessId, id: &str) -> Option<Slot> {
        lock(&self.inner.pool.pending_credentials)
            .get(id)
            .filter(|slot| slot.harness == harness)
            .cloned()
    }

    pub(super) fn merge_pending_pool_slots(&self, harness: HarnessId, slots: &mut Vec<Slot>) {
        for pending in lock(&self.inner.pool.pending_credentials)
            .values()
            .filter(|slot| slot.harness == harness)
        {
            if let Some(slot) = slots.iter_mut().find(|slot| slot.id == pending.id) {
                *slot = pending.clone();
            } else {
                slots.push(pending.clone());
            }
        }
    }

    pub(super) fn forget_pending_pool_slot(&self, id: &str) {
        lock(&self.inner.pool.pending_credentials).remove(id);
    }

    pub(super) fn save_refreshed_pool_slot(&self, slot: &Slot) {
        if self.write_slot(slot).is_err() {
            // A single-use refresh token must not disappear on a transient
            // filesystem failure. Serve it from memory until persistence recovers.
            lock(&self.inner.pool.pending_credentials).insert(slot.id.clone(), slot.clone());
            tracing::warn!("refreshed account credentials are awaiting persistence");
        }
    }

    pub(super) fn retry_pool_credential_saves(&self) {
        let slots: Vec<_> = lock(&self.inner.pool.pending_credentials)
            .values()
            .cloned()
            .collect();
        for slot in slots {
            let _ = self.write_slot(&slot);
        }
    }

    pub(super) fn order_pool_accounts(&self, accounts: &mut [AgentAccount]) {
        let mut settings = lock(&self.inner.pool.config);
        let mut changed = false;
        for harness in [HarnessId::ClaudeCode, HarnessId::Codex] {
            let config = if harness == HarnessId::ClaudeCode {
                &mut settings.claude
            } else {
                &mut settings.codex
            };
            let ids: HashSet<_> = self
                .read_slots(harness)
                .into_iter()
                .map(|slot| slot.id)
                .collect();
            let before = config.account_order.clone();
            config.account_order.retain(|id| ids.contains(id));
            config.disabled_account_ids.retain(|id| ids.contains(id));
            for account in accounts
                .iter()
                .filter(|a| a.harness == harness && ids.contains(&a.id))
            {
                if !config.account_order.contains(&account.id) {
                    config.account_order.push(account.id.clone());
                }
            }
            changed |= before != config.account_order;
            let mut ordered: Vec<_> = accounts
                .iter()
                .filter(|a| a.harness == harness)
                .cloned()
                .collect();
            ordered.sort_by_key(|a| {
                config
                    .account_order
                    .iter()
                    .position(|id| id == &a.id)
                    .unwrap_or(usize::MAX)
            });
            for (account, ordered) in accounts
                .iter_mut()
                .filter(|a| a.harness == harness)
                .zip(ordered)
            {
                *account = ordered;
            }
        }
        if changed && self.save_pool_config(&settings).is_err() {
            tracing::warn!("could not persist account pool order");
        }
    }

    pub(super) fn apply_pool_usage(&self, accounts: &mut [AgentAccount]) {
        let state = lock(&self.inner.pool.state);
        for account in accounts {
            if let Some(observation) = state.observations.get(&account.id)
                && observation.observed_at >= account.usage_fetched_at.unwrap_or(0)
            {
                account
                    .usage_windows
                    .retain(|w| !observation.replaced_window_labels.contains(&w.label));
                if account.harness == HarnessId::Codex && !observation.codex_windows.is_empty() {
                    account.usage_windows.retain(|w| {
                        !observation
                            .codex_window_labels
                            .values()
                            .any(|label| *label == w.label)
                    });
                    for bucket in ["primary", "secondary"] {
                        if let Some(window) = observation.codex_windows.get(bucket) {
                            account.usage_windows.push(window.clone());
                        }
                    }
                } else {
                    for window in &observation.windows {
                        account.usage_windows.retain(|w| w.label != window.label);
                        account.usage_windows.push(window.clone());
                    }
                }
                if !observation.windows.is_empty() {
                    account.usage_fetched_at = Some(observation.observed_at);
                }
            }
        }
    }

    pub(super) fn include_unsaved_pool_statuses(snapshot: &mut AgentAccountsSnapshot) {
        for account in &snapshot.accounts {
            if matches!(account.harness, HarnessId::ClaudeCode | HarnessId::Codex)
                && !snapshot
                    .pool
                    .accounts
                    .iter()
                    .any(|status| status.account_id == account.id)
            {
                snapshot.pool.accounts.push(AccountPoolAccountStatus {
                    account_id: account.id.clone(),
                    status: AccountPoolStatus::Unsupported,
                    until: None,
                    family: None,
                    weekly_reset_at: None,
                    poolable: false,
                    unpoolable_reason: Some(
                        "This live login has no saved readable credentials".into(),
                    ),
                    held_until: None,
                    error: None,
                    last_used_at: None,
                    in_flight: 0,
                });
            }
        }
    }
    pub fn pool_snapshot(&self) -> AccountPoolSnapshot {
        let pool = &self.inner.pool;
        let config = lock(&pool.config).clone();
        let mut snapshot = AccountPoolSnapshot {
            config: config.clone(),
            ..Default::default()
        };
        let now = now_ms();
        for harness in [HarnessId::ClaudeCode, HarnessId::Codex] {
            let settings = provider(&config, harness).unwrap();
            let next_account_ids: HashMap<_, _> = ["other", "opus", "sonnet", "fable", "haiku"]
                .into_iter()
                .filter_map(|family| {
                    self.pool_candidates(harness, family, None, &HashSet::new(), now)
                        .first()
                        .map(|slot| (family.to_owned(), slot.id.clone()))
                })
                .collect();
            let next = next_account_ids.get("other").cloned();
            let state = lock(&pool.state);
            let slots = self.read_slots(harness);
            let serving = slots
                .iter()
                .any(|s| has_secret(s) && !settings.disabled_account_ids.contains(&s.id));
            for slot in &slots {
                let observation = state
                    .observations
                    .get(&slot.id)
                    .cloned()
                    .unwrap_or_default();
                let windows = self.pool_windows(slot, &observation);
                let limit = windows
                    .iter()
                    .filter(|w| applies(w, "other") && w.used_fraction >= settings.switch_threshold)
                    .filter(|w| w.resets_at.is_none_or(|at| at.timestamp_millis() > now))
                    .filter_map(|w| w.resets_at)
                    .max();
                let family_limit = windows.iter().find(|w| {
                    w.label.starts_with("Week (")
                        && w.used_fraction >= settings.switch_threshold
                        && w.resets_at.is_none_or(|at| at.timestamp_millis() > now)
                });
                let status = if !has_secret(slot) {
                    AccountPoolStatus::Unsupported
                } else if settings.disabled_account_ids.contains(&slot.id) {
                    AccountPoolStatus::Excluded
                } else if observation
                    .error
                    .as_ref()
                    .is_some_and(|e| e.starts_with("Live login expired"))
                {
                    AccountPoolStatus::LiveExpired
                } else if observation.error.is_some() {
                    AccountPoolStatus::NeedsSignIn
                } else if observation.held_until.is_some_and(|at| at > now) {
                    AccountPoolStatus::Held
                } else if !eligible(&windows, "other", settings.switch_threshold, now) {
                    AccountPoolStatus::Limited
                } else if next.as_ref() == Some(&slot.id) {
                    AccountPoolStatus::Next
                } else if family_limit.is_some() {
                    AccountPoolStatus::FamilyLimited
                } else {
                    AccountPoolStatus::Ready
                };
                snapshot.accounts.push(AccountPoolAccountStatus {
                    account_id: slot.id.clone(),
                    status,
                    until: if status == AccountPoolStatus::Held {
                        observation
                            .held_until
                            .and_then(DateTime::from_timestamp_millis)
                    } else if status == AccountPoolStatus::FamilyLimited {
                        family_limit.and_then(|w| w.resets_at)
                    } else {
                        limit
                    },
                    family: family_limit.map(|w| {
                        w.label
                            .trim_start_matches("Week (")
                            .trim_end_matches(')')
                            .to_owned()
                    }),
                    weekly_reset_at: weekly_reset(
                        &self.pool_routing_windows(slot, &observation),
                        "other",
                        now,
                    )
                    .and_then(DateTime::from_timestamp_millis),
                    poolable: has_secret(slot),
                    unpoolable_reason: (!has_secret(slot))
                        .then(|| "This login has no readable credentials".into()),
                    held_until: observation.held_until,
                    error: observation.error,
                    last_used_at: observation.last_used_at,
                    in_flight: observation.in_flight,
                });
            }
            snapshot.providers.push(AccountPoolProviderStatus {
                harness, serving, next_account_id: next.clone(), next_account_ids,
                notice: if settings.enabled && !serving { Some("No pooled account can take requests — sessions use the live login.".into()) }
                    else if settings.enabled && next.is_none() { Some("Every pooled account is limited or unavailable — requests resume when an account is ready.".into()) }
                    else { None },
            });
        }
        snapshot
    }

    pub async fn configure_pool(
        &self,
        harness: HarnessId,
        config: AccountPoolProviderConfig,
    ) -> Result<AgentAccountsSnapshot, EngineError> {
        validate_config(&config)?;
        if !matches!(harness, HarnessId::ClaudeCode | HarnessId::Codex) {
            return Err(EngineError::Other(
                "Account pooling supports Claude Code and Codex".into(),
            ));
        }
        let _ops = self.inner.ops.lock().await;
        // Import the CLI's current login before validating membership.
        self.list_locked(false).await?;
        let slots = self.read_slots(harness);
        let ids: HashSet<_> = slots.iter().map(|slot| &slot.id).collect();
        if config
            .account_order
            .iter()
            .chain(&config.disabled_account_ids)
            .any(|id| !ids.contains(id))
            || !config.account_order.is_empty() && config.account_order.len() != ids.len()
        {
            return Err(EngineError::Other(
                "Pool configuration names an unknown account".into(),
            ));
        }
        {
            let mut settings = lock(&self.inner.pool.config);
            let mut next = settings.clone();
            match harness {
                HarnessId::ClaudeCode => next.claude = config,
                HarnessId::Codex => next.codex = config,
                _ => unreachable!(),
            }
            self.save_pool_config(&next)?;
            *settings = next;
        }
        self.list_locked(false).await
    }

    pub async fn set_pool_bypass(&self, chat_id: &str, bypassed: bool) -> Result<(), EngineError> {
        if chat_id.is_empty() || chat_id.len() > 512 {
            return Err(EngineError::Other("Invalid chat ID".into()));
        }
        let mut settings = lock(&self.inner.pool.config);
        let mut next = settings.clone();
        next.bypassed_chat_ids.retain(|id| id != chat_id);
        if bypassed {
            if next.bypassed_chat_ids.len() >= 4096 {
                return Err(EngineError::Other("Too many bypassed chats".into()));
            }
            next.bypassed_chat_ids.push(chat_id.into());
        }
        self.save_pool_config(&next)?;
        *settings = next;
        Ok(())
    }

    /// Add a key to the private slot store without changing the CLI's login.
    pub async fn add_pool_api_key(
        &self,
        harness: HarnessId,
        api_key: &str,
        label: Option<&str>,
    ) -> Result<AgentAccountsSnapshot, EngineError> {
        if harness != HarnessId::ClaudeCode {
            return Err(EngineError::Other(
                "API-key entry supports Claude Code".into(),
            ));
        }
        let key = api_key.trim();
        if key.is_empty() || key.len() > 8192 || key.chars().any(char::is_control) {
            return Err(EngineError::Other("Enter a valid API key".into()));
        }
        let label = label
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Anthropic API key");
        if label.len() > 256 {
            return Err(EngineError::Other("Account label is too long".into()));
        }
        let _ops = self.inner.ops.lock().await;
        let digest = Sha256::digest(key.as_bytes());
        let account_key = format!("api-key:{}", crate::repos::hex(&digest));
        self.write_slot(&Slot {
            id: slot_id_for(harness, &account_key),
            harness,
            account_key,
            profile: SlotProfile {
                email: label.into(),
                display_name: Some(label.into()),
                organization: None,
                plan: Some("API key".into()),
                auth_kind: AgentAuthKind::ApiKey,
            },
            credentials: serde_json::json!({"apiKey":key}),
            claude_config: None,
            saved_at: now_ms(),
            created_at: None,
            store_key: None,
        })?;
        self.list_locked(false).await
    }

    fn save_pool_config(&self, config: &AccountPoolConfig) -> Result<(), EngineError> {
        let root = self.inner.config.private_root()?;
        write_file_atomic(
            &root.join("pool.json"),
            &serde_json::to_vec_pretty(config).map_err(|e| EngineError::Other(e.to_string()))?,
            true,
        )
    }

    fn save_pool_state(&self, state: &RoutingState) {
        self.persist_pool_state(state, true);
    }

    fn persist_pool_state(&self, state: &RoutingState, force: bool) {
        let mut last = lock(&self.inner.pool.last_persist);
        if !force && last.is_some_and(|at| at.elapsed() < Duration::from_secs(2)) {
            return;
        }
        let result = self
            .inner
            .config
            .private_root()
            .map_err(EngineError::from)
            .and_then(|root| {
                write_file_atomic(
                    &root.join("pool-state.json"),
                    &serde_json::to_vec(state).map_err(|e| EngineError::Other(e.to_string()))?,
                    true,
                )
            });
        if result.is_err() {
            tracing::warn!("could not persist account pool observations");
        } else {
            *last = Some(Instant::now());
        }
    }

    pub(super) fn flush_pool(&self) {
        self.save_pool_state(&lock(&self.inner.pool.state));
    }

    /// Called only on the owning device. Never sends the capability over RPC.
    pub async fn pool_route(
        &self,
        harness: HarnessId,
        chat_id: &str,
    ) -> Result<Option<AccountPoolRoute>, EngineError> {
        {
            let config = lock(&self.inner.pool.config);
            if provider(&config, harness).is_none_or(|p| !p.enabled)
                || config.bypassed_chat_ids.iter().any(|id| id == chat_id)
            {
                return Ok(None);
            }
            let settings = provider(&config, harness).unwrap();
            if !self
                .read_slots(harness)
                .iter()
                .any(|s| !settings.disabled_account_ids.contains(&s.id) && has_secret(s))
            {
                return Ok(None);
            }
        }
        self.refresh_pool_in_background(harness);
        let mut hub = self.inner.pool.hub.lock().await;
        if hub.is_none() {
            let listener =
                match tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await {
                    Ok(listener) => listener,
                    Err(_) => {
                        tracing::warn!("could not start account pool listener; using live login");
                        return Ok(None);
                    }
                };
            let port = listener.local_addr()?.port();
            let token = random_url_token();
            http::start(
                listener,
                Arc::downgrade(&self.inner),
                token.clone(),
                self.inner.pool.cancel.clone(),
            );
            *hub = Some(Hub { port, token });
        }
        let hub = hub.as_ref().unwrap();
        let path = if harness == HarnessId::ClaudeCode {
            "claude"
        } else {
            "codex/v1"
        };
        Ok(Some(AccountPoolRoute {
            base_url: format!("http://127.0.0.1:{}/{path}", hub.port),
            token: hub.token.clone(),
        }))
    }

    fn refresh_pool_in_background(&self, harness: HarnessId) {
        let now = now_ms();
        let mut last = lock(&self.inner.pool.last_background_probe);
        if last.get(&harness).is_some_and(|at| now - at < 30_000) {
            return;
        }
        let stale = {
            let usage = lock(&self.inner.usage);
            self.read_slots(harness).iter().any(|slot| {
                let entry = usage.get(&usage_key(harness, &slot.account_key));
                entry
                    .and_then(|e| e.fetched_at)
                    .is_none_or(|at| now - at >= 5 * 60 * 1000)
                    || entry.and_then(|e| e.usage.as_ref()).is_some_and(|u| {
                        u.windows
                            .iter()
                            .any(|w| w.resets_at.is_some_and(|at| at.timestamp_millis() <= now))
                    })
            })
        };
        if !stale {
            return;
        }
        last.insert(harness, now);
        let accounts = self.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = accounts.inner.pool.cancel.cancelled() => {},
                _ = accounts.refresh_pool_usage(harness, false) => {},
            }
        });
    }

    async fn refresh_pool_usage(&self, harness: HarnessId, exhausted_only: bool) {
        // Refresh the snapshot of a CLI-owned login without keeping the
        // operation gate during any provider quota requests.
        {
            let _ops = self.inner.ops.lock().await;
            self.retry_pool_credential_saves();
            let detected = match harness {
                HarnessId::ClaudeCode => self.detect_claude().await.0,
                HarnessId::Codex => self.detect_codex(),
                _ => None,
            };
            if let Some(detected) = detected {
                let _ = self.snapshot_detected(harness, &detected);
            }
        }
        let threshold = provider(&lock(&self.inner.pool.config), harness)
            .unwrap()
            .switch_threshold;
        let slots = self.read_slots(harness);
        let targets: Vec<_> = slots
            .iter()
            .filter(|slot| {
                if !exhausted_only {
                    return true;
                }
                let observation = lock(&self.inner.pool.state)
                    .observations
                    .get(&slot.id)
                    .cloned()
                    .unwrap_or_default();
                observation.error.is_some()
                    || observation.held_until.is_some_and(|at| at > now_ms())
                    || self
                        .pool_windows(slot, &observation)
                        .iter()
                        .any(|w| w.used_fraction >= threshold)
            })
            .map(|slot| (harness, slot, true))
            .collect();
        self.refresh_usage_scoped(&targets, false).await;
    }

    fn pool_windows(&self, slot: &Slot, state: &Observation) -> Vec<AgentUsageWindow> {
        let usage = lock(&self.inner.usage);
        let cached = usage.get(&usage_key(slot.harness, &slot.account_key));
        let mut windows = cached
            .and_then(|entry| entry.usage.as_ref())
            .map(|s| s.windows.clone())
            .unwrap_or_default();
        if cached.and_then(|e| e.fetched_at).unwrap_or(0) <= state.observed_at {
            windows.retain(|w| !state.replaced_window_labels.contains(&w.label));
            if slot.harness == HarnessId::Codex && !state.codex_windows.is_empty() {
                windows.retain(|w| {
                    !state
                        .codex_window_labels
                        .values()
                        .any(|label| *label == w.label)
                });
                for bucket in ["primary", "secondary"] {
                    if let Some(window) = state.codex_windows.get(bucket) {
                        windows.push(window.clone());
                    }
                }
                return windows;
            }
            for window in &state.windows {
                windows.retain(|w| w.label != window.label);
                windows.push(window.clone());
            }
        }
        windows
    }

    fn pool_routing_windows(
        &self,
        slot: &Slot,
        observation: &Observation,
    ) -> Vec<AgentUsageWindow> {
        let mut windows = self.pool_windows(slot, observation);
        for (key, reset) in &observation.quota_resets {
            let label = match key.strip_prefix("codex:") {
                Some(bucket) => observation
                    .codex_window_labels
                    .get(bucket)
                    .map(String::as_str)
                    .unwrap_or(if bucket == "primary" {
                        "Primary"
                    } else {
                        "Secondary"
                    }),
                None => key,
            };
            if !windows
                .iter()
                .any(|window| window.label == label && window.resets_at == Some(*reset))
            {
                windows.push(AgentUsageWindow {
                    label: label.into(),
                    used_fraction: 0.0,
                    resets_at: Some(*reset),
                });
            }
        }
        windows
    }

    fn pool_candidates(
        &self,
        harness: HarnessId,
        family: &str,
        session: Option<&str>,
        attempted: &HashSet<String>,
        now: i64,
    ) -> Vec<Slot> {
        let live = match harness {
            HarnessId::ClaudeCode => {
                read_json(&self.inner.config.claude_config_file).and_then(|cfg| {
                    str_field(&cfg["oauthAccount"], "accountUuid")
                        .or_else(|| str_field(&cfg["oauthAccount"], "emailAddress"))
                })
            }
            HarnessId::Codex => self.detect_codex().map(|detected| detected.account_key),
            _ => None,
        };
        let config = lock(&self.inner.pool.config);
        let Some(settings) = provider(&config, harness) else {
            return Vec::new();
        };
        let mut state = lock(&self.inner.pool.state);
        state.pins.retain(|_, pin| now - pin.used_at < PIN_TTL);
        let mut candidates: Vec<_> = self
            .read_slots(harness)
            .into_iter()
            .filter_map(|slot| {
                if settings.disabled_account_ids.contains(&slot.id)
                    || attempted.contains(&slot.id)
                    || !has_secret(&slot)
                {
                    return None;
                }
                let observation = state.observations.entry(slot.id.clone()).or_default();
                let fingerprint = credentials_fingerprint(&slot.credentials);
                let probe_recovered = lock(&self.inner.usage)
                    .get(&usage_key(harness, &slot.account_key))
                    .is_some_and(|entry| {
                        entry.error.is_none()
                            && entry.fetched_at.is_some_and(|at| at > observation.error_at)
                            && entry.usage_credentials.as_deref() == Some(fingerprint.as_str())
                            && observation
                                .error_credentials
                                .as_deref()
                                .is_none_or(|failed| failed == fingerprint)
                    });
                if observation
                    .error_credentials
                    .as_ref()
                    .is_some_and(|old| old != &fingerprint)
                    || probe_recovered
                    || observation
                        .error
                        .as_deref()
                        .is_some_and(|error| error.starts_with("Live login expired"))
                        && live.as_deref() != Some(slot.account_key.as_str())
                {
                    observation.error = None;
                    observation.error_credentials = None;
                }
                if observation.error.is_some() || observation.held_until.is_some_and(|at| at > now)
                {
                    return None;
                }
                let windows = self.pool_routing_windows(&slot, observation);
                eligible(&windows, family, settings.switch_threshold, now)
                    .then_some((slot, windows))
            })
            .collect();
        candidates.sort_by_key(|(slot, windows)| {
            let priority = settings
                .account_order
                .iter()
                .position(|id| id == &slot.id)
                .unwrap_or(settings.account_order.len());
            let reset = match settings.routing_mode {
                AccountPoolRoutingMode::WeeklyReset => {
                    weekly_reset(windows, family, now).unwrap_or(i64::MAX)
                }
                AccountPoolRoutingMode::Sequential => 0,
            };
            (
                reset,
                priority,
                slot.created_at.unwrap_or(slot.saved_at),
                slot.id.clone(),
            )
        });
        if settings.routing_mode == AccountPoolRoutingMode::Sequential {
            let key = session.map(|id| format!("{}:{id}", harness_slug(harness)));
            let anchor = key
                .as_ref()
                .and_then(|k| state.pins.get(k))
                .map(|p| &p.account_id)
                .or_else(|| state.active.get(harness_slug(harness)));
            if let Some(index) =
                anchor.and_then(|id| candidates.iter().position(|(s, _)| &s.id == id))
            {
                candidates.rotate_left(index);
            } else if let Some(anchor_index) =
                anchor.and_then(|id| settings.account_order.iter().position(|s| s == id))
            {
                let index = candidates
                    .iter()
                    .position(|(slot, _)| {
                        settings
                            .account_order
                            .iter()
                            .position(|id| id == &slot.id)
                            .is_some_and(|index| index > anchor_index)
                    })
                    .unwrap_or(0);
                candidates.rotate_left(index);
            }
        }
        candidates.into_iter().map(|(slot, _)| slot).collect()
    }

    fn accept_pool_account(&self, slot: &Slot, family: &str, session: Option<&str>) {
        let fingerprint = credentials_fingerprint(&slot.credentials);
        if self
            .read_slot(slot.harness, &slot.id)
            .is_none_or(|current| credentials_fingerprint(&current.credentials) != fingerprint)
        {
            return;
        }
        let settings = lock(&self.inner.pool.config);
        let config = provider(&settings, slot.harness).unwrap();
        let mut state = lock(&self.inner.pool.state);
        let now = now_ms();
        let observation = state.observations.entry(slot.id.clone()).or_default();
        observation.last_used_at = Some(now);
        if observation
            .error_credentials
            .as_deref()
            .is_none_or(|failed| failed == fingerprint)
        {
            observation.error = None;
            observation.error_credentials = None;
        }
        if config.routing_mode == AccountPoolRoutingMode::Sequential {
            // A family-only detour must not replace a healthy shared binding.
            let prior = session
                .and_then(|s| {
                    state
                        .pins
                        .get(&format!("{}:{s}", harness_slug(slot.harness)))
                })
                .map(|p| p.account_id.clone())
                .or_else(|| state.active.get(harness_slug(slot.harness)).cloned());
            let family_detour = prior
                .and_then(|id| self.read_slot(slot.harness, &id))
                .is_some_and(|old| {
                    let windows = self.pool_windows(
                        &old,
                        &state.observations.get(&old.id).cloned().unwrap_or_default(),
                    );
                    eligible(&windows, "other", config.switch_threshold, now)
                        && !eligible(&windows, family, config.switch_threshold, now)
                });
            if !family_detour {
                state
                    .active
                    .insert(harness_slug(slot.harness).into(), slot.id.clone());
                if let Some(session) = session {
                    state.pins.insert(
                        format!("{}:{session}", harness_slug(slot.harness)),
                        Pin {
                            account_id: slot.id.clone(),
                            used_at: now,
                        },
                    );
                }
            }
        }
        while state.pins.len() > MAX_PINS {
            let oldest = state
                .pins
                .iter()
                .min_by_key(|(_, p)| p.used_at)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                state.pins.remove(&id);
            }
        }
        self.persist_pool_state(
            &state,
            config.routing_mode == AccountPoolRoutingMode::Sequential,
        );
    }

    async fn pool_secret(&self, slot: &Slot, rejected: bool) -> Result<Slot, SecretError> {
        // Healthy requests never wait behind Settings' live-login detection or
        // probes. A rejected access token takes the serialized repair path.
        let current = self
            .read_slot(slot.harness, &slot.id)
            .ok_or("Account was removed")?;
        let live = self.live_account_key(slot.harness, None).await.as_deref()
            == Some(&current.account_key);
        let changed = credentials_fingerprint(&current.credentials)
            != credentials_fingerprint(&slot.credentials);
        let due = token_expiry(&current)
            .is_some_and(|at| at <= now_ms() + if live { 0 } else { 5 * 60 * 1000 });
        let refresh_backoff = lock(&self.inner.pool.refresh_retry_at)
            .get(&slot.id)
            .is_some_and(|at| *at > now_ms());
        if !due && (!rejected || changed)
            || !rejected
                && refresh_backoff
                && token_expiry(&current).is_some_and(|at| at > now_ms())
        {
            return Ok(current);
        }
        if refresh_backoff {
            return Err(SecretError::Temporary("Account refresh is backing off"));
        }
        // Dropping a downstream request may cancel its waiter, but must never
        // cancel a refresh after the provider has consumed a single-use token.
        let accounts = self.clone();
        let slot = slot.clone();
        let task = {
            let tasks = lock(&self.inner.pool.refresh_tasks);
            let Some(tasks) = tasks.as_ref() else {
                return Err(SecretError::Temporary("Account pool is shutting down"));
            };
            tasks.spawn(async move {
                let result = accounts.pool_secret_once(&slot, rejected).await;
                if let Err(SecretError::Authentication(message)) = &result {
                    http::mark_error(&accounts, &slot, message).await;
                }
                result
            })
        };
        task.await
            .map_err(|_| SecretError::Temporary("Account refresh task failed"))?
    }

    async fn pool_secret_once(&self, slot: &Slot, rejected: bool) -> Result<Slot, SecretError> {
        let refresh = lock(&self.inner.pool.refreshes)
            .entry(slot.id.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _single = refresh.lock().await;
        let _ops = self.inner.ops.lock().await;
        let mut current = self
            .read_slot(slot.harness, &slot.id)
            .ok_or("Account was removed")?;
        let active =
            self.live_account_key(slot.harness, None).await.as_deref() == Some(&slot.account_key);
        if active {
            // The CLI owns this refresh-token family. Re-read it, never rotate it.
            let detected = match slot.harness {
                HarnessId::ClaudeCode => {
                    *lock(&self.inner.claude_credentials) = None;
                    self.detect_claude().await.0
                }
                HarnessId::Codex => self.detect_codex(),
                _ => None,
            }
            .ok_or("Live account credentials are unavailable")?;
            if detected.account_key != current.account_key {
                return Err("Live account changed".into());
            }
            self.snapshot_detected(slot.harness, &detected)
                .map_err(|_| SecretError::Temporary("Could not save live account snapshot"))?;
            current.credentials = detected
                .credentials
                .ok_or("Live account credentials are unavailable")?;
            if token_expiry(&current).is_some_and(|at| at <= now_ms()) {
                return Err("Live login expired; sign in again in the CLI".into());
            }
            return Ok(current);
        }
        let changed = credentials_fingerprint(&current.credentials)
            != credentials_fingerprint(&slot.credentials);
        let due = token_expiry(&current).is_some_and(|at| at <= now_ms() + 5 * 60 * 1000);
        if !due && (!rejected || changed) {
            return Ok(current);
        }
        if lock(&self.inner.pool.refresh_retry_at)
            .get(&slot.id)
            .is_some_and(|at| *at > now_ms())
        {
            if !rejected && token_expiry(&current).is_some_and(|at| at > now_ms()) {
                return Ok(current);
            }
            return Err(SecretError::Temporary("Account refresh is backing off"));
        }
        let fallback = current.clone();
        let result = async {
            match slot.harness {
                HarnessId::ClaudeCode if current.credentials.get("claudeAiOauth").is_some() => {
                    self.refresh_claude_slot_once(&current)
                        .await
                        .map_err(|error| match error {
                            ProbeError::Unauthorized { .. } | ProbeError::NoCredentials { .. } => {
                                SecretError::Authentication("Needs sign-in")
                            }
                            _ => SecretError::Temporary("Account refresh temporarily unavailable"),
                        })?;
                    self.read_slot(slot.harness, &slot.id)
                        .ok_or(SecretError::Temporary(
                            "Refreshed account could not be read",
                        ))
                }
                HarnessId::Codex if current.credentials.get("tokens").is_some() => {
                    let tokens = &current.credentials["tokens"];
                    let refresh =
                        str_field(tokens, "refresh_token").ok_or("Account has no refresh token")?;
                    let url = lock(&self.inner.pool.endpoints).codex_refresh.clone();
                    let result = self
                        .inner
                        .http
                        .post(url)
                        .json(&serde_json::json!({
                            "grant_type":"refresh_token", "refresh_token":refresh,
                            "client_id":"app_EMoamEEZ73f0CkXaXp7hrann",
                        }))
                        .send()
                        .await
                        .map_err(|_| SecretError::Temporary("Account refresh connection failed"))?;
                    if !result.status().is_success() {
                        return Err(if matches!(result.status().as_u16(), 400 | 401 | 403) {
                            SecretError::Authentication("Needs sign-in")
                        } else {
                            SecretError::Temporary("Account refresh temporarily unavailable")
                        });
                    }
                    let body: serde_json::Value = result
                        .json()
                        .await
                        .map_err(|_| SecretError::Temporary("Invalid refresh response"))?;
                    let access = str_field(&body, "access_token")
                        .ok_or(SecretError::Temporary("Invalid refresh response"))?;
                    current.credentials["tokens"]["access_token"] = access.into();
                    for key in ["refresh_token", "id_token"] {
                        if let Some(value) = str_field(&body, key) {
                            current.credentials["tokens"][key] = value.into();
                        }
                    }
                    current.credentials["last_refresh"] = Utc::now().to_rfc3339().into();
                    current.saved_at = now_ms();
                    self.save_refreshed_pool_slot(&current);
                    Ok(current)
                }
                _ => Ok(current),
            }
        }
        .await;
        match result {
            Err(SecretError::Temporary(message)) => {
                lock(&self.inner.pool.refresh_retry_at).insert(slot.id.clone(), now_ms() + 30_000);
                if !rejected && token_expiry(&fallback).is_some_and(|at| at > now_ms()) {
                    Ok(fallback)
                } else {
                    Err(SecretError::Temporary(message))
                }
            }
            result => {
                if result.is_ok() {
                    lock(&self.inner.pool.refresh_retry_at).remove(&slot.id);
                }
                result
            }
        }
    }
}

fn has_secret(slot: &Slot) -> bool {
    match slot.harness {
        HarnessId::ClaudeCode => {
            str_field(&slot.credentials["claudeAiOauth"], "accessToken").is_some()
                || str_field(&slot.credentials, "apiKey").is_some()
        }
        HarnessId::Codex => {
            str_field(&slot.credentials["tokens"], "access_token").is_some()
                || str_field(&slot.credentials, "OPENAI_API_KEY").is_some()
        }
        _ => false,
    }
}

fn token_expiry(slot: &Slot) -> Option<i64> {
    if slot.harness == HarnessId::ClaudeCode {
        // Some Claude CLI releases wrote zero for freshly issued tokens.
        // Unknown metadata is not evidence that a live login expired.
        slot.credentials["claudeAiOauth"]["expiresAt"]
            .as_i64()
            .filter(|at| *at > 0)
    } else {
        str_field(&slot.credentials["tokens"], "access_token")
            .and_then(|token| jwt_claims(&token))
            .and_then(|claims| claims["exp"].as_i64())
            .map(|seconds| seconds.saturating_mul(1000))
    }
}

pub(super) fn model_family(model: &str) -> &'static str {
    zeron_proto::model_family(model)
}

fn applies(window: &AgentUsageWindow, family: &str) -> bool {
    !window.label.starts_with("Week (")
        || window
            .label
            .eq_ignore_ascii_case(&format!("Week ({family})"))
}

fn eligible(windows: &[AgentUsageWindow], family: &str, threshold: f32, now: i64) -> bool {
    !windows.iter().any(|w| {
        applies(w, family)
            && w.used_fraction >= threshold
            && w.resets_at.is_none_or(|at| at.timestamp_millis() > now)
    })
}

fn weekly_reset(windows: &[AgentUsageWindow], family: &str, now: i64) -> Option<i64> {
    let scoped = format!("Week ({family})");
    [scoped.as_str(), "Week"].into_iter().find_map(|label| {
        windows
            .iter()
            .filter(|w| w.label.eq_ignore_ascii_case(label))
            .filter_map(|w| w.resets_at)
            .map(|at| at.timestamp_millis())
            .filter(|at| *at > now)
            .min()
    })
}
