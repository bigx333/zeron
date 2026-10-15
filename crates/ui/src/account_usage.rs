//! The composer footer's ring cluster. The plan-usage ring shows how much of
//! the active account's rate limit the session's harness has used, beside
//! the context ring. Clicking it opens the harness's accounts — each with its
//! usage meters — and clicking one switches to it, the same `ActivateAgentAccount` Settings →
//! Accounts runs. Both views share [`AccountsSnapshotCache`], so a switch in
//! either shows up in the other.
//!
//! When the harness routes through the account pool and this chat isn't
//! bypassing it, the ring combines its enabled accounts' usage, the card
//! lists the pool (statuses, no click-to-switch), and its last row is the
//! per-chat "Use account pool in this chat" switch.
use std::time::{Duration, Instant};

use gpui::{
    Context, Entity, IntoElement, Render, SharedString, Subscription, Task, Window, div,
    prelude::*, px,
};
use zeron_proto::{AccountPoolStatus, AgentAccount, AgentAccountsSnapshot, HarnessId};

use crate::motion::{self, IndicatorSlide};
use zeron_rpc::methods;

use crate::popover;
use crate::settings::accounts::{
    self, AccountsSnapshotCache, UsageLevel, reports_usage, signs_in, usage_color, usage_level,
};
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

/// Forced probes hit the provider; the engine throttles them too, but the
/// ring re-probes on every hover, so don't even ask more often than this.
const FORCE_MIN_INTERVAL: Duration = Duration::from_secs(30);
/// Background re-probe while a composer is alive: usage moves as turns run.
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// While the pool indicator is visible: an unforced list (no provider calls)
/// so combined usage and statuses move as requests flow.
const POOL_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// The binding limit: the most-used window of the account. Pure.
pub fn used_fraction(account: &AgentAccount) -> Option<f32> {
    account
        .usage_windows
        .iter()
        .map(|window| window.used_fraction.clamp(0.0, 1.0))
        .reduce(f32::max)
}

/// The harness's live account, when it reports usage. Pure.
pub fn active_account(
    snapshot: &AgentAccountsSnapshot,
    harness: HarnessId,
) -> Option<&AgentAccount> {
    snapshot
        .accounts
        .iter()
        .find(|account| account.harness == harness && account.active)
}

/// Whether `chat` on `harness` routes through the account pool: routing is
/// on for the harness, the pool is serving (with no readable member the
/// engine falls back to the live login), and the chat hasn't opted out.
/// Pure.
pub fn pooled_for_chat(
    snapshot: &AgentAccountsSnapshot,
    harness: HarnessId,
    chat: Option<&str>,
) -> bool {
    accounts::pool_enabled(snapshot, harness)
        && accounts::pool_provider(snapshot, harness).is_none_or(|provider| provider.serving)
        && !chat_bypassed(snapshot, chat)
}

/// The binding limit for a request of `family`: the shared windows plus
/// that family's own weekly window — an Opus-only limit doesn't redden a
/// Sonnet chat's ring. Pure.
pub fn family_used_fraction(account: &AgentAccount, family: &str) -> Option<f32> {
    family_used_fraction_at(account, family, chrono::Utc::now())
}

fn family_used_fraction_at(
    account: &AgentAccount,
    family: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<f32> {
    account
        .usage_windows
        .iter()
        .filter(|window| match window.label.strip_prefix("Week (") {
            Some(rest) => rest.trim_end_matches(')').eq_ignore_ascii_case(family),
            None => true,
        })
        .filter(|window| window.used_fraction.is_finite())
        .map(|window| {
            if window.resets_at.is_some_and(|reset| reset <= now) {
                0.0
            } else {
                window.used_fraction.clamp(0.0, 1.0)
            }
        })
        .reduce(f32::max)
}

/// Combined normalized use of the enabled, supported pool members. Providers
/// report fractions, not comparable quota capacities, so each measured account
/// contributes equally. Its most-used applicable window binds its capacity;
/// exhausted accounts stay in the total, and unknown usage never counts as 0%.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PoolUsage {
    pub fraction: Option<f32>,
    pub measured: usize,
    pub members: usize,
}

pub fn pool_usage(
    snapshot: &AgentAccountsSnapshot,
    harness: HarnessId,
    model: Option<&str>,
) -> PoolUsage {
    let Some(config) = accounts::pool_config(snapshot, harness) else {
        return PoolUsage::default();
    };
    let family = zeron_proto::model_family(model.unwrap_or_default());
    let mut usage = PoolUsage::default();
    let mut sum = 0.0f64;
    for account in &snapshot.accounts {
        if account.harness != harness
            || config.disabled_account_ids.contains(&account.id)
            || accounts::pool_account(snapshot, &account.id).is_some_and(|status| {
                !status.poolable
                    || matches!(
                        status.status,
                        AccountPoolStatus::Excluded | AccountPoolStatus::Unsupported
                    )
            })
        {
            continue;
        }
        usage.members += 1;
        if let Some(fraction) = family_used_fraction(account, family) {
            sum += f64::from(fraction);
            usage.measured += 1;
        }
    }
    if usage.measured > 0 {
        usage.fraction = Some((sum / usage.measured as f64) as f32);
    }
    usage
}

fn pool_usage_tooltip(usage: PoolUsage, harness: HarnessId) -> String {
    let heading = match usage.fraction {
        Some(fraction) => format!(
            "{} pool · {}% combined use",
            accounts::provider_name(harness),
            (fraction * 100.0).round() as u32,
        ),
        None => format!("{} pool · usage unknown", accounts::provider_name(harness)),
    };
    format!(
        "{heading}\n{} of {} enabled accounts report usage.\nAverage of each measured account's most-used limit for this model.",
        usage.measured, usage.members,
    )
}

fn chat_bypassed(snapshot: &AgentAccountsSnapshot, chat: Option<&str>) -> bool {
    chat.is_some_and(|chat| {
        snapshot
            .pool
            .config
            .bypassed_chat_ids
            .iter()
            .any(|id| id == chat)
    })
}

/// The account the pool would use next for `model`'s family — the engine
/// computes each family's pick. A family missing from a populated map has
/// no eligible account; an engine without per-family picks reads the
/// generic one. `None` when routing is off or nothing can take a request.
/// Pure.
pub fn pool_next_for_model<'a>(
    snapshot: &'a AgentAccountsSnapshot,
    harness: HarnessId,
    model: Option<&str>,
) -> Option<&'a str> {
    if !accounts::pool_enabled(snapshot, harness) {
        return None;
    }
    let provider = accounts::pool_provider(snapshot, harness)?;
    if provider.next_account_ids.is_empty() {
        return provider.next_account_id.as_deref();
    }
    let family = zeron_proto::model_family(model.unwrap_or_default());
    provider.next_account_ids.get(family).map(String::as_str)
}

/// What the footer ring shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RingReading {
    /// `None`: no measured usage (API keys or logins without a probe)
    /// — the chip says "Pool" rather than claiming 0%.
    pub fraction: Option<f32>,
    /// No pooled account can take a request: warn without changing its measured use.
    pub exhausted: bool,
}

/// The ring's reading: for a pooled chat, combined use across enabled accounts
/// for the chat's model (red when none can take a request); else the live
/// account's. While the harness pools, the ring
/// never disappears — its card holds the per-chat switch back. Pure.
pub fn ring_reading(
    snapshot: &AgentAccountsSnapshot,
    harness: HarnessId,
    chat: Option<&str>,
    model: Option<&str>,
) -> Option<RingReading> {
    if pooled_for_chat(snapshot, harness, chat) {
        return Some(RingReading {
            fraction: pool_usage(snapshot, harness, model).fraction,
            exhausted: pool_next_for_model(snapshot, harness, model).is_none(),
        });
    }
    match active_account(snapshot, harness).and_then(used_fraction) {
        Some(fraction) => Some(RingReading {
            fraction: Some(fraction),
            exhausted: false,
        }),
        None => accounts::pool_enabled(snapshot, harness).then_some(RingReading {
            fraction: None,
            exhausted: false,
        }),
    }
}

/// Loads (and switches) the accounts of the composer's target device.
pub struct AccountUsage {
    state: Entity<AppState>,
    /// `None` = this device, as in [`AccountsSnapshotCache`].
    target: Option<String>,
    harness: Option<HarnessId>,
    /// The composer's chat — the per-chat pool bypass is keyed by it.
    chat: Option<String>,
    /// The composer's model: combined usage considers its family's limits
    /// (an Opus-only limit doesn't consume a Sonnet chat's capacity).
    model: Option<String>,
    /// The ring's drawn fraction, tweened when the reading changes (a new
    /// source — live account ↔ combined pool — or fresh usage).
    ring: Option<(IndicatorSlide, f32)>,
    /// Once any list has been asked for the current target.
    loaded: bool,
    last_forced: Option<Instant>,
    error: Option<SharedString>,
    load_task: Option<Task<()>>,
    action_task: Option<Task<()>>,
    bypass_task: Option<Task<()>>,
    popup: popover::Popup<FooterCard>,
    _poll: Task<()>,
    _cache: Subscription,
    _state: Subscription,
}

impl AccountUsage {
    /// Capture the production footer and popover without an engine or credentials.
    #[cfg(feature = "account-pool-fixture")]
    pub(crate) fn fixture_snapshot(
        &mut self,
        snapshot: AgentAccountsSnapshot,
        open: bool,
        cx: &mut Context<Self>,
    ) {
        self.loaded = true;
        cx.default_global::<AccountsSnapshotCache>()
            .0
            .insert(None, snapshot);
        self.popup = popover::Popup::default();
        if open {
            self.popup.open(FooterCard::Accounts);
        }
        cx.notify();
    }

    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let poll = cx.spawn(async move |this, cx| {
            let every = (POLL_INTERVAL.as_secs() / POOL_POLL_INTERVAL.as_secs()).max(1);
            let mut tick = 0u64;
            loop {
                cx.background_executor().timer(POOL_POLL_INTERVAL).await;
                tick += 1;
                if this
                    .update(cx, |usage, cx| {
                        if usage.harness.is_none() {
                            return;
                        }
                        if tick.is_multiple_of(every) {
                            usage.load(true, cx);
                        } else if usage.harness.is_some_and(|harness| {
                            usage
                                .snapshot(cx)
                                .is_some_and(|snapshot| accounts::pool_enabled(snapshot, harness))
                        }) {
                            usage.load(false, cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            _state: cx.observe(&state, |_, _, cx| cx.notify()),
            state,
            target: None,
            harness: None,
            chat: None,
            model: None,
            ring: None,
            loaded: false,
            last_forced: None,
            error: None,
            load_task: None,
            action_task: None,
            bypass_task: None,
            popup: popover::Popup::default(),
            _poll: poll,
            // Settings → Accounts writes the same cache.
            _cache: cx.observe_global::<AccountsSnapshotCache>(|_, cx| cx.notify()),
        }
    }

    /// Point at the session's harness and device; loads on first sight of a
    /// device. Cheap enough to call every render.
    pub fn track(
        &mut self,
        harness: Option<HarnessId>,
        target: Option<String>,
        chat: Option<String>,
        model: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let harness = harness.filter(|h| signs_in(*h) && reports_usage(*h));
        self.harness = harness;
        self.chat = chat;
        self.model = model;
        if self.target != target {
            self.target = target;
            self.loaded = false;
            self.last_forced = None;
            self.error = None;
        }
        if harness.is_some() && !self.loaded {
            self.loaded = true;
            self.load(true, cx);
        }
    }

    fn snapshot<'a>(&self, cx: &'a gpui::App) -> Option<&'a AgentAccountsSnapshot> {
        cx.try_global::<AccountsSnapshotCache>()?
            .0
            .get(&self.target)
    }

    fn params(&self, mut value: serde_json::Value) -> serde_json::Value {
        if let (Some(target), Some(object)) = (&self.target, value.as_object_mut()) {
            object.insert("targetDeviceId".into(), serde_json::json!(target));
        }
        value
    }

    /// Plain list first when nothing is cached (the engine's persisted usage
    /// paints at once), then the forced probe replaces it.
    fn load(&mut self, force_usage: bool, cx: &mut Context<Self>) {
        if force_usage {
            if self
                .last_forced
                .is_some_and(|at| at.elapsed() < FORCE_MIN_INTERVAL)
            {
                return;
            }
            self.last_forced = Some(Instant::now());
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let key = self.target.clone();
        let paint_first = force_usage && self.snapshot(cx).is_none();
        let plain = self.params(serde_json::json!({ "forceUsage": false }));
        let params = self.params(serde_json::json!({ "forceUsage": force_usage }));
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let fetch = |params: serde_json::Value| {
                let engine = engine.clone();
                async move {
                    engine
                        .client()
                        .call(methods::LIST_AGENT_ACCOUNTS, params)
                        .await
                        .ok()
                        .and_then(|value| {
                            serde_json::from_value::<AgentAccountsSnapshot>(value).ok()
                        })
                }
            };
            let store = |snapshot: AgentAccountsSnapshot, cx: &mut gpui::AsyncApp| {
                let key = key.clone();
                this.update(cx, |_, cx| {
                    cx.default_global::<AccountsSnapshotCache>()
                        .0
                        .insert(key, snapshot);
                })
                .ok();
            };
            if paint_first && let Some(snapshot) = fetch(plain).await {
                store(snapshot, cx);
            }
            if let Some(snapshot) = fetch(params).await {
                store(snapshot, cx);
            }
        }));
    }

    /// Switch optimistically, like Settings → Accounts: the rows flip at once
    /// and the engine's reply replaces them; a refusal restores the list.
    fn switch(&mut self, account: &AgentAccount, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let key = self.target.clone();
        let cache = &mut cx.default_global::<AccountsSnapshotCache>().0;
        let previous = cache.get(&key).cloned();
        if let Some(snapshot) = cache.get_mut(&key) {
            // Per-provider agents keep one live login per provider: only the
            // switched-to row's group changes.
            accounts::mark_switched(snapshot, account);
        }
        self.error = None;
        let params = self.params(serde_json::json!({
            "id": account.id,
            "accountId": account.id,
            "harness": account.harness,
        }));
        self.action_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::ACTIVATE_AGENT_ACCOUNT, params)
                .await;
            this.update(cx, |usage, cx| {
                match result.map(serde_json::from_value::<AgentAccountsSnapshot>) {
                    Ok(Ok(snapshot)) => {
                        cx.default_global::<AccountsSnapshotCache>()
                            .0
                            .insert(key, snapshot);
                    }
                    Ok(Err(_)) => usage.load(false, cx),
                    Err(err) => {
                        if let Some(previous) = previous {
                            cx.default_global::<AccountsSnapshotCache>()
                                .0
                                .insert(key, previous);
                        }
                        usage.error = Some(err.to_string().into());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// The ring's reading ([`ring_reading`]) for this chat.
    fn reading(&self, cx: &gpui::App) -> Option<RingReading> {
        ring_reading(
            self.snapshot(cx)?,
            self.harness?,
            self.chat.as_deref(),
            self.model.as_deref(),
        )
    }

    fn pooled(&self, cx: &gpui::App) -> bool {
        match (self.snapshot(cx), self.harness) {
            (Some(snapshot), Some(harness)) => {
                pooled_for_chat(snapshot, harness, self.chat.as_deref())
            }
            _ => false,
        }
    }

    /// The per-chat pool switch, optimistically: the cache flips at once
    /// (every view of this device sees it) and the engine's reply replaces
    /// it; a refusal restores it. Applies from the chat's next turn.
    fn set_bypass(&mut self, bypassed: bool, cx: &mut Context<Self>) {
        let Some(chat) = self.chat.clone() else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let key = self.target.clone();
        let cache = &mut cx.default_global::<AccountsSnapshotCache>().0;
        let previous = cache.get(&key).cloned();
        if let Some(snapshot) = cache.get_mut(&key) {
            let ids = &mut snapshot.pool.config.bypassed_chat_ids;
            ids.retain(|id| *id != chat);
            if bypassed {
                ids.push(chat.clone());
            }
        }
        self.error = None;
        let params = self.params(serde_json::json!({ "chatId": chat, "bypassed": bypassed }));
        self.bypass_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::SET_ACCOUNT_POOL_BYPASS, params)
                .await;
            this.update(cx, |usage, cx| {
                match result.map(serde_json::from_value::<AgentAccountsSnapshot>) {
                    Ok(Ok(snapshot)) => {
                        cx.default_global::<AccountsSnapshotCache>()
                            .0
                            .insert(key, snapshot);
                    }
                    Ok(Err(_)) => usage.load(false, cx),
                    Err(err) => {
                        if let Some(previous) = previous {
                            cx.default_global::<AccountsSnapshotCache>()
                                .0
                                .insert(key, previous);
                        }
                        usage.error = Some(err.to_string().into());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// A trigger click: open this ring's popover, or close it when the
    /// press found it open (the card's mouse-down-out already began that
    /// close — see `Popup::note_trigger_press`).
    fn toggle(&mut self, card: FooterCard, cx: &mut Context<Self>) {
        if self.popup.take_press_was_open() || self.popup.as_open() == Some(&card) {
            self.dismiss(cx);
            return;
        }
        if card == FooterCard::Accounts {
            // Opening the card is the moment someone cares: re-probe.
            self.load(true, cx);
        }
        self.popup.open(card);
        cx.notify();
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.popup.begin_close() {
            popover::reap_popup(cx, |usage: &mut Self| &mut usage.popup);
        }
        cx.notify();
    }

    /// A footer ring that opens `card` above itself, right-aligned like the
    /// model picker.
    fn trigger(
        &self,
        chip: gpui::Stateful<gpui::Div>,
        card: FooterCard,
        content: impl FnOnce(&Self, &mut Context<Self>) -> gpui::Div,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let chip = chip
            .relative()
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |usage, _, _, _| {
                    usage
                        .popup
                        .note_trigger_press_matching(|open| *open == card);
                }),
            )
            .on_click(cx.listener(move |usage, _, _, cx| usage.toggle(card, cx)));
        if self.popup.get() != Some(&card) {
            return chip.into_any_element();
        }
        let content = content(self, cx)
            .on_mouse_down_out(cx.listener(|usage, _, _, cx| usage.dismiss(cx)))
            .into_any_element();
        chip.child(popover::anchored_menu_above_end(
            card.id(),
            content,
            self.popup.closing_since(),
        ))
        .into_any_element()
    }

    fn accounts_card(&self, cx: &mut Context<Self>) -> gpui::Div {
        let theme = &Theme::of(cx).for_popup();
        let harness = self.harness;
        let now = chrono::Utc::now();
        let snapshot = self.snapshot(cx).cloned();
        let rows: Vec<AgentAccount> = match (harness, &snapshot) {
            (Some(harness), Some(snapshot)) => accounts::provider_accounts(snapshot, harness)
                .into_iter()
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        let pool_on = match (harness, &snapshot) {
            (Some(harness), Some(snapshot)) => accounts::pool_enabled(snapshot, harness),
            _ => false,
        };
        let bypassed = snapshot
            .as_ref()
            .is_some_and(|snapshot| chat_bypassed(snapshot, self.chat.as_deref()));
        let pooled = match (harness, &snapshot) {
            (Some(harness), Some(snapshot)) => {
                pooled_for_chat(snapshot, harness, self.chat.as_deref())
            }
            _ => false,
        };
        let config = match (harness, &snapshot) {
            (Some(harness), Some(snapshot)) => accounts::pool_config(snapshot, harness).cloned(),
            _ => None,
        };
        let next = match (harness, &snapshot) {
            (Some(harness), Some(snapshot)) => {
                pool_next_for_model(snapshot, harness, self.model.as_deref()).map(str::to_string)
            }
            _ => None,
        };
        let title = format!(
            "{} accounts",
            harness.map_or("Agent", accounts::provider_name)
        );
        // Pooled: "Account pool" and the routing mode, quietly.
        let heading = match config.as_ref().filter(|_| pooled) {
            Some(config) => div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(popover::menu_heading(theme, "Account pool"))
                .child(
                    div()
                        .px(px(8.0))
                        .pt(px(2.0))
                        .text_size(crate::typography::ui_rems(11.5))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(
                            accounts::POOL_MODES
                                .iter()
                                .find(|(mode, ..)| *mode == config.routing_mode)
                                .map_or("", |(_, label, _)| *label),
                        )),
                ),
            None => popover::menu_heading(theme, &title),
        };
        popover::popover_card(theme)
            .w(px(400.0))
            .flex()
            .flex_col()
            .child(heading)
            .children(rows.iter().enumerate().map(|(ix, account)| {
                let email: SharedString = account
                    .email
                    .clone()
                    .or_else(|| account.display_name.clone())
                    .unwrap_or_else(|| "Unknown account".into())
                    .into();
                // A pooled chat's card shows the pool; switching the CLI's
                // live login from here would not change where it routes.
                let can_switch = !pooled && accounts::can_switch_live(account);
                let mut meta: Vec<gpui::AnyElement> = account
                    .plan_label
                    .iter()
                    .map(|plan| {
                        div()
                            .child(SharedString::from(plan.clone()))
                            .into_any_element()
                    })
                    .collect();
                let fragment = config.as_ref().filter(|_| pooled).and_then(|config| {
                    // "Next" follows this chat's model family, like the ring.
                    let status = snapshot
                        .as_ref()
                        .and_then(|s| accounts::pool_account(s, &account.id))
                        .cloned()
                        .map(|mut status| {
                            let is_next = next.as_deref() == Some(account.id.as_str());
                            match status.status {
                                AccountPoolStatus::Next if !is_next => {
                                    status.status = AccountPoolStatus::Ready
                                }
                                AccountPoolStatus::Ready if is_next => {
                                    status.status = AccountPoolStatus::Next
                                }
                                _ => {}
                            }
                            status
                        });
                    accounts::pool_status_fragment(
                        status.as_ref(),
                        config.disabled_account_ids.contains(&account.id),
                        config.routing_mode,
                        now,
                    )
                });
                if let Some((label, tone)) = fragment {
                    meta.push(
                        motion::fade_quick(
                            SharedString::from(format!("footer-pool-{}-{label}", account.id)),
                            div()
                                .text_color(accounts::pool_tone_color(tone, theme))
                                .child(SharedString::from(label)),
                        )
                        .into_any_element(),
                    );
                } else if account.active && !pooled {
                    meta.push(
                        div()
                            .text_color(theme.accent)
                            .child(SharedString::from("In use"))
                            .into_any_element(),
                    );
                } else if let Some(reason) = account
                    .usage_error
                    .clone()
                    .filter(|_| account.usage_windows.is_empty())
                {
                    meta.push(div().child(SharedString::from(reason)).into_any_element());
                }
                let marked = if pooled {
                    next.as_deref() == Some(account.id.as_str())
                } else {
                    account.active
                };
                let switch_to = account.clone();
                let shown = accounts::meter_windows(&account.usage_windows);
                popover::menu_row(theme, marked, format!("account-usage-row-{ix}"))
                    .id(("account-usage-row", ix))
                    .when(!can_switch, |row| row.cursor_default())
                    .when(can_switch, |row| {
                        row.on_click(cx.listener(move |usage, _, _, cx| {
                            usage.switch(&switch_to, cx);
                        }))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.5))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child(email),
                            )
                            .when(!meta.is_empty(), |el| {
                                el.child(crate::settings::widgets::meta_line(theme, meta))
                            }),
                    )
                    .child(
                        div()
                            .id(("account-usage-meters", ix))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .when(!account.usage_windows.is_empty(), |meters| {
                                meters.tooltip(widgets::text_tooltip(accounts::usage_tooltip(
                                    &account.usage_windows,
                                    &shown,
                                    now,
                                )))
                            })
                            .children(accounts::render_usage_meters(&account.usage_windows, theme)),
                    )
            }))
            // The per-chat opt-out: wherever the harness pools, this chat
            // can keep to the live login (and come back).
            .when(pool_on && self.chat.is_some(), |card| {
                card.child(
                    div()
                        .mx(px(8.0))
                        .my(px(4.0))
                        .border_t_1()
                        .border_color(widgets::row_divider(theme)),
                )
                .child(
                    div()
                        .px(px(8.0))
                        .py(px(6.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(12.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(theme.text)
                                        .child(SharedString::from("Use account pool in this chat")),
                                )
                                .child(widgets::meta_line(
                                    theme,
                                    vec![
                                        div()
                                            .child(SharedString::from(
                                                "Applies from the next turn.",
                                            ))
                                            .into_any_element(),
                                    ],
                                )),
                        )
                        .child(
                            widgets::toggle_switch(theme, !bypassed, "account-pool-chat")
                                .id("account-pool-chat-switch")
                                .cursor_pointer()
                                .tab_index(0)
                                .role(gpui::Role::Switch)
                                .aria_label("Use account pool in this chat")
                                .aria_toggled(if bypassed {
                                    gpui::Toggled::False
                                } else {
                                    gpui::Toggled::True
                                })
                                .focus_visible(|s| {
                                    s.border_2().border_color(theme.accent).opacity(1.0)
                                })
                                .on_click(cx.listener(move |usage, _, _, cx| {
                                    usage.set_bypass(!bypassed, cx);
                                }))
                                .on_key_down(cx.listener(
                                    move |usage, event: &gpui::KeyDownEvent, _, cx| {
                                        if !event.is_held
                                            && matches!(
                                                event.keystroke.key.as_str(),
                                                "enter" | "space"
                                            )
                                        {
                                            usage.set_bypass(!bypassed, cx);
                                            cx.stop_propagation();
                                        }
                                    },
                                )),
                        ),
                )
            })
            .children(self.error.clone().map(|error| {
                div()
                    .px(px(8.0))
                    .py(px(4.0))
                    .text_size(px(12.0))
                    .text_color(theme.danger)
                    .child(error)
            }))
    }
}

/// Which footer ring's popover is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FooterCard {
    Accounts,
    Context,
}

impl FooterCard {
    fn id(self) -> &'static str {
        match self {
            FooterCard::Accounts => "account-usage-menu",
            FooterCard::Context => "context-usage-menu",
        }
    }
}

/// The footer's ring cluster: account usage (accent arc, so the two read
/// apart), then context occupancy. Each opens its popover on click.
impl Render for AccountUsage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let context = self.state.read(cx).context_usage;
        let reading = self.reading(cx);
        // The arc travels to a new reading instead of snapping — a change of
        // source (live account ↔ the combined pool) included.
        let drawn = reading.map(|reading| {
            let target = reading.fraction.unwrap_or(0.0);
            let now = Instant::now();
            let reduced = motion::reduced_motion(cx);
            let (ring, drawn_target) = self
                .ring
                .get_or_insert_with(|| (IndicatorSlide::at(target, now), target));
            if *drawn_target != target {
                *drawn_target = target;
                ring.retarget(target, reduced, now);
            }
            if ring.animating_at(now) {
                window.request_animation_frame();
            }
            ring.value_at(now)
        });
        if reading.is_none() {
            self.ring = None;
        }
        let account = reading.zip(drawn).map(|(reading, fraction)| {
            let level = match reading.fraction {
                _ if reading.exhausted => UsageLevel::Critical,
                Some(fraction) => usage_level(fraction),
                None => UsageLevel::Normal,
            };
            let chip = crate::context_usage::ring_chip(
                "account-usage",
                fraction,
                usage_color(level, &theme),
                match level {
                    UsageLevel::Normal => theme.text_muted,
                    _ => usage_color(level, &theme),
                },
                match reading.fraction {
                    Some(fraction) => format!("{}%", (fraction * 100.0).round() as u32),
                    None => "Pool".to_string(),
                },
                self.popup.get() == Some(&FooterCard::Accounts),
                &theme,
            )
            .when(self.pooled(cx), |chip| {
                let harness = self.harness.expect("pooled usage has a harness");
                let usage = pool_usage(
                    self.snapshot(cx).expect("pooled usage has a snapshot"),
                    harness,
                    self.model.as_deref(),
                );
                let mut tooltip = pool_usage_tooltip(usage, harness);
                if reading.exhausted {
                    tooltip.push_str("\nNo pooled account is ready for this model.");
                }
                chip.tooltip(widgets::text_tooltip(tooltip))
            });
            self.trigger(
                chip,
                FooterCard::Accounts,
                |usage, cx| usage.accounts_card(cx),
                cx,
            )
        });
        let context = crate::context_usage::has_window(context).then(|| {
            let chip = crate::context_usage::chip(
                context,
                self.popup.get() == Some(&FooterCard::Context),
                &theme,
            );
            self.trigger(
                chip,
                FooterCard::Context,
                move |_, cx| crate::context_usage::card(context, &Theme::of(cx).for_popup()),
                cx,
            )
        });
        div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .children(account)
            .children(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeron_proto::AgentUsageWindow;

    fn account(harness: HarnessId, active: bool, used: &[f32]) -> AgentAccount {
        serde_json::from_value(serde_json::json!({
            "id": format!("{harness:?}-{active}"),
            "harness": harness,
            "email": null,
            "planLabel": null,
            "active": active,
            "switchable": true,
            "usageWindows": used
                .iter()
                .map(|fraction| AgentUsageWindow {
                    label: "5h".into(),
                    used_fraction: *fraction,
                    resets_at: None,
                })
                .collect::<Vec<_>>(),
        }))
        .unwrap()
    }

    #[test]
    fn ring_shows_the_most_used_window() {
        assert_eq!(used_fraction(&account(HarnessId::Codex, true, &[])), None);
        assert_eq!(
            used_fraction(&account(HarnessId::Codex, true, &[0.12, 0.64])),
            Some(0.64)
        );
        assert_eq!(
            used_fraction(&account(HarnessId::Codex, true, &[1.4])),
            Some(1.0)
        );
    }

    fn pooled(enabled: bool) -> AgentAccountsSnapshot {
        let mut live = account(HarnessId::ClaudeCode, true, &[0.3]);
        live.id = "live".into();
        let mut next: AgentAccount = account(HarnessId::ClaudeCode, false, &[]);
        next.id = "next".into();
        next.usage_windows = [("Session", 0.2), ("Week", 0.4), ("Week (opus)", 0.97)]
            .into_iter()
            .map(|(label, used)| AgentUsageWindow {
                label: label.into(),
                used_fraction: used,
                resets_at: None,
            })
            .collect();
        let mut snapshot = AgentAccountsSnapshot {
            accounts: vec![live, next],
            warnings: Vec::new(),
            pool: Default::default(),
        };
        snapshot.pool.config.claude.enabled = enabled;
        snapshot.pool.providers = vec![zeron_proto::AccountPoolProviderStatus {
            harness: HarnessId::ClaudeCode,
            serving: true,
            next_account_id: Some("next".into()),
            next_account_ids: Default::default(),
            notice: None,
        }];
        snapshot
    }

    #[test]
    fn a_pooled_chat_reads_combined_usage_for_its_model_family() {
        let claude = HarnessId::ClaudeCode;
        let reading = |snapshot: &AgentAccountsSnapshot, chat, model| {
            ring_reading(snapshot, claude, chat, model).map(|r| {
                (
                    (r.fraction.unwrap_or(-1.0) * 1000.0).round() as i32,
                    r.exhausted,
                )
            })
        };
        // Pool off: the live account, as always.
        assert_eq!(
            reading(&pooled(false), Some("c1"), None),
            Some((300, false))
        );
        let mut snapshot = pooled(true);
        // Both enabled accounts contribute; an Opus-only limit doesn't
        // inflate the Sonnet total.
        assert_eq!(
            reading(&snapshot, Some("c1"), Some("claude-sonnet-4-5")),
            Some((350, false))
        );
        assert_eq!(
            reading(&snapshot, Some("c1"), Some("claude-opus-4-1")),
            Some((635, false))
        );
        // Per-family picks: a populated map without the family means no
        // eligible account — red, while retaining the actual combined use.
        snapshot.pool.providers[0]
            .next_account_ids
            .insert("sonnet".into(), "next".into());
        assert_eq!(
            reading(&snapshot, Some("c1"), Some("claude-opus-4-1")),
            Some((635, true))
        );
        assert_eq!(
            pool_next_for_model(&snapshot, claude, Some("claude-sonnet-4-5")),
            Some("next")
        );
        // A bypassed chat keeps the live login.
        snapshot.pool.config.bypassed_chat_ids = vec!["c1".into()];
        assert_eq!(reading(&snapshot, Some("c1"), None), Some((300, false)));
        assert!(!pooled_for_chat(&snapshot, claude, Some("c1")));
        assert!(pooled_for_chat(&snapshot, claude, Some("c2")));
        // A pool with no readable member isn't serving: the live login runs.
        snapshot.pool.providers[0].serving = false;
        assert_eq!(reading(&snapshot, Some("c2"), None), Some((300, false)));
        // One unknown member never counts as 0%: the remaining account
        // contributes its measured use. Entirely unknown usage keeps the
        // trigger without claiming a percentage (-1000 here = unknown).
        snapshot.pool.providers[0].serving = true;
        snapshot.accounts[1].usage_windows.clear();
        assert_eq!(
            reading(&snapshot, Some("c2"), Some("claude-sonnet-4-5")),
            Some((300, false))
        );
        snapshot.accounts[0].usage_windows.clear();
        assert_eq!(
            reading(&snapshot, Some("c2"), Some("claude-sonnet-4-5")),
            Some((-1000, false))
        );
        assert_eq!(reading(&snapshot, Some("c1"), None), Some((-1000, false)));
        snapshot.pool.config.claude.enabled = false;
        assert_eq!(
            reading(&snapshot, Some("c1"), None),
            None,
            "no pool, no usage: no ring"
        );
    }

    #[test]
    fn combined_pool_use_does_not_follow_the_next_account() {
        let mut snapshot = pooled(true);
        snapshot.accounts[0].usage_windows[0].used_fraction = 0.2;
        snapshot.accounts[1].usage_windows = vec![AgentUsageWindow {
            label: "Session".into(),
            used_fraction: 0.8,
            resets_at: None,
        }];
        let before = ring_reading(&snapshot, HarnessId::ClaudeCode, Some("c"), None);
        snapshot.pool.providers[0].next_account_id = Some("live".into());
        assert_eq!(
            ring_reading(&snapshot, HarnessId::ClaudeCode, Some("c"), None),
            before,
        );
        assert_eq!(before.unwrap().fraction, Some(0.5));
    }

    #[test]
    fn claude_and_codex_have_separate_combined_pool_totals() {
        let mut snapshot = pooled(true);
        snapshot.pool.config.codex.enabled = true;
        let mut first = account(HarnessId::Codex, true, &[0.1]);
        first.id = "codex-first".into();
        let mut second = account(HarnessId::Codex, false, &[0.7]);
        second.id = "codex-second".into();
        snapshot.accounts.extend([first, second]);
        snapshot
            .pool
            .providers
            .push(zeron_proto::AccountPoolProviderStatus {
                harness: HarnessId::Codex,
                serving: true,
                next_account_id: Some("codex-first".into()),
                next_account_ids: Default::default(),
                notice: None,
            });
        let codex = ring_reading(&snapshot, HarnessId::Codex, Some("c"), Some("gpt-5.4"))
            .unwrap()
            .fraction
            .unwrap();
        let claude = ring_reading(&snapshot, HarnessId::ClaudeCode, Some("c"), Some("sonnet"))
            .unwrap()
            .fraction
            .unwrap();
        assert!((codex - 0.4).abs() < 0.0001);
        assert!((claude - 0.35).abs() < 0.0001);
    }

    #[test]
    fn combined_pool_use_ignores_other_providers_and_excluded_or_unsupported_members() {
        let mut snapshot = pooled(true);
        snapshot
            .accounts
            .push(account(HarnessId::Codex, true, &[1.0]));
        snapshot.pool.config.claude.disabled_account_ids = vec!["next".into()];
        let usage = pool_usage(&snapshot, HarnessId::ClaudeCode, None);
        assert_eq!(usage.fraction, Some(0.3));
        assert_eq!((usage.measured, usage.members), (1, 1));
        snapshot.pool.config.claude.disabled_account_ids.clear();
        snapshot.pool.accounts = vec![zeron_proto::AccountPoolAccountStatus {
            account_id: "next".into(),
            status: AccountPoolStatus::Unsupported,
            until: None,
            family: None,
            weekly_reset_at: None,
            poolable: false,
            unpoolable_reason: Some("Unreadable credentials".into()),
            held_until: None,
            error: None,
            last_used_at: None,
            in_flight: 0,
        }];
        assert_eq!(pool_usage(&snapshot, HarnessId::ClaudeCode, None), usage);
    }

    #[test]
    fn combined_pool_use_retains_exhausted_accounts_and_each_accounts_binding_limit() {
        let mut snapshot = pooled(true);
        snapshot.accounts[0].usage_windows = [0.0, 1.0]
            .into_iter()
            .map(|used_fraction| AgentUsageWindow {
                label: "Session".into(),
                used_fraction,
                resets_at: None,
            })
            .collect();
        snapshot.accounts[1].usage_windows = [1.0, 0.0]
            .into_iter()
            .map(|used_fraction| AgentUsageWindow {
                label: "Week".into(),
                used_fraction,
                resets_at: None,
            })
            .collect();
        snapshot.pool.providers[0].next_account_id = None;
        assert_eq!(
            ring_reading(&snapshot, HarnessId::ClaudeCode, Some("c"), None),
            Some(RingReading {
                fraction: Some(1.0),
                exhausted: true,
            }),
        );
        snapshot.accounts[0].usage_windows[1].used_fraction = 0.2;
        assert_eq!(
            pool_usage(&snapshot, HarnessId::ClaudeCode, None).fraction,
            Some(0.6),
            "a limited member's consumed quota remains in the combined total",
        );
    }

    #[test]
    fn combined_pool_use_reports_partial_or_unknown_measurements() {
        let mut snapshot = pooled(true);
        snapshot.accounts[1].usage_windows.clear();
        let usage = pool_usage(&snapshot, HarnessId::ClaudeCode, None);
        assert_eq!(usage.fraction, Some(0.3));
        assert_eq!((usage.measured, usage.members), (1, 2));
        let tooltip = pool_usage_tooltip(usage, HarnessId::ClaudeCode);
        assert!(tooltip.contains("30% combined use"));
        assert!(tooltip.contains("1 of 2 enabled accounts report usage"));
        snapshot.accounts[0].usage_windows.clear();
        let usage = pool_usage(&snapshot, HarnessId::ClaudeCode, None);
        assert_eq!(usage.fraction, None);
        assert_eq!((usage.measured, usage.members), (0, 2));
        assert!(pool_usage_tooltip(usage, HarnessId::ClaudeCode).contains("usage unknown"));
    }

    #[test]
    fn combined_pool_use_releases_expired_limits() {
        let mut snapshot = pooled(true);
        let now = chrono::Utc::now();
        snapshot.accounts[0].usage_windows[0].used_fraction = 1.0;
        snapshot.accounts[0].usage_windows[0].resets_at = Some(now - chrono::TimeDelta::seconds(1));
        snapshot.accounts[1].usage_windows = vec![AgentUsageWindow {
            label: "Week".into(),
            used_fraction: 0.8,
            resets_at: Some(now + chrono::TimeDelta::hours(1)),
        }];
        assert_eq!(
            pool_usage(&snapshot, HarnessId::ClaudeCode, None).fraction,
            Some(0.4),
        );
    }

    #[test]
    fn active_account_is_scoped_to_the_harness() {
        let snapshot = AgentAccountsSnapshot {
            pool: Default::default(),
            accounts: vec![
                account(HarnessId::ClaudeCode, true, &[0.3]),
                account(HarnessId::Codex, false, &[0.1]),
                account(HarnessId::Codex, true, &[0.2]),
            ],
            warnings: Vec::new(),
        };
        assert_eq!(
            active_account(&snapshot, HarnessId::Codex).map(|a| a.id.as_str()),
            Some("Codex-true")
        );
        assert!(active_account(&snapshot, HarnessId::Cursor).is_none());
    }
}
