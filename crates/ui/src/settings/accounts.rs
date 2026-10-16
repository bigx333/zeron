//! Settings → Providers / accounts (feature-inventory §1.9): provider cards
//! (Claude Code, Codex, Cursor, Antigravity) with account rows — email, plan
//! badge, Active, usage meters (indigo → amber ≥80% → red ≥95%, reset time),
//! Switch / Forget — plus the one sign-in dialog every provider shares (the
//! browser finishes the login on a loopback callback; Claude's paste-code
//! step is its fallback) and account-shaped loading skeletons. Zeron
//! retargets devices from the settings sidebar (`targetDeviceId` passthrough;
//! a remote device's callback is forwarded to this one engine-side).
//!
//! The accounts RPC surface is being implemented engine-side in parallel —
//! every call here surfaces failures as inline UI states rather than assuming
//! the methods exist.

use chrono::{DateTime, Utc};
use gpui::{
    AnyElement, Context, Entity, Hsla, SharedString, Subscription, Task, Window, div, prelude::*,
    px,
};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use zeron_proto::{
    AccountPoolAccountStatus, AccountPoolProviderConfig, AccountPoolProviderStatus,
    AccountPoolRoutingMode, AccountPoolStatus, AgentAccount, AgentAccountsSnapshot, AgentLoginMode,
    AgentLoginPoll, AgentLoginStart, AgentLoginStatus, HarnessId,
};
use zeron_rpc::methods;

use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::motion;
use crate::popover::{self, Loadable};
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

// ---------------------------------------------------------------------------
// Pure: usage meters + labels
// ---------------------------------------------------------------------------

pub const USAGE_WARN_FRACTION: f32 = 0.80;
pub const USAGE_CRITICAL_FRACTION: f32 = 0.95;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageLevel {
    /// < 80% — indigo.
    Normal,
    /// ≥ 80% — amber.
    Warn,
    /// ≥ 95% — red.
    Critical,
}

/// Threshold classification of a usage fraction. Pure.
pub fn usage_level(fraction: f32) -> UsageLevel {
    if fraction >= USAGE_CRITICAL_FRACTION {
        UsageLevel::Critical
    } else if fraction >= USAGE_WARN_FRACTION {
        UsageLevel::Warn
    } else {
        UsageLevel::Normal
    }
}

/// Usage meter columns: window label, reset time, and the meters' overall
/// measure — long enough to read at a glance, short of a full-width gauge.
/// Compact account list geometry: every row shares one usage column and one
/// action slot, so meters and buttons line up down the list however many
/// accounts there are.
const USAGE_LABEL_WIDTH: f32 = 52.0;
const USAGE_BAR_WIDTH: f32 = 88.0;
const USAGE_PERCENT_WIDTH: f32 = 34.0;
const USAGE_COLUMN_WIDTH: f32 = USAGE_LABEL_WIDTH + USAGE_BAR_WIDTH + USAGE_PERCENT_WIDTH + 16.0;
const ACCOUNT_ACTION_WIDTH: f32 = 112.0;

pub fn usage_color(level: UsageLevel, theme: &Theme) -> Hsla {
    match level {
        UsageLevel::Normal => theme.accent,
        UsageLevel::Warn => theme.warning,
        UsageLevel::Critical => theme.danger,
    }
}

/// Why a `ListAgentAccounts` load is happening. Pure input to
/// [`force_usage_for`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadTrigger {
    /// Page construction — the visit's first list.
    Mount,
    /// "Click to retry" after a failed load — still the visit's first
    /// successful list.
    Retry,
    /// The explicit Refresh button.
    Refresh,
    /// After a completed add-account login flow.
    PostLogin,
    /// After Switch/Forget succeeds.
    PostAction,
}

/// Whether a load should ask the engine to probe usage (`forceUsage`). The
/// engine only hits the provider when forced; non-forced lists serve the 60s
/// usage cache or nothing (engine/src/agent_accounts.rs module docs — the
/// design expects the UI to force "on page mount/refresh"). The visit's first
/// list (mount, or retry after a failure) must force, or every first open
/// renders "Usage unavailable" until a manual Refresh — the old app fetched
/// usage on every list. Post-Switch/Forget lists ride the still-warm cache.
pub fn force_usage_for(trigger: LoadTrigger) -> bool {
    match trigger {
        LoadTrigger::Mount | LoadTrigger::Retry | LoadTrigger::Refresh | LoadTrigger::PostLogin => {
            true
        }
        LoadTrigger::PostAction => false,
    }
}

/// Whether a login URL may be opened. Remote logins relay their URL from
/// another device (or from a CLI's output there), so only web sign-in pages
/// and loopback callbacks pass — never `file://` or custom schemes. Pure.
pub fn is_openable_login_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    match scheme.to_ascii_lowercase().as_str() {
        "https" => true,
        "http" => {
            let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
            let authority = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            let host = match authority.strip_prefix('[') {
                Some(v6) => v6.split(']').next().unwrap_or(""),
                None => authority.split(':').next().unwrap_or(""),
            };
            matches!(host, "localhost" | "127.0.0.1" | "::1")
        }
        _ => false,
    }
}

fn open_login_url(url: &str, cx: &mut gpui::App) {
    if is_openable_login_url(url) {
        cx.open_url(url);
    } else {
        tracing::warn!("refused to open a non-web sign-in URL");
    }
}

/// Compact absolute reset moment (zeron settings.agents.tsx `formatReset`):
/// a local clock time ("3:45 PM") when it lands within ~22h, a short weekday
/// ("Mon") within a week, else month + day ("Sep 14") — a weekday is noise
/// when the window is a Codex free-tier MONTHLY reset weeks out. The caller
/// prefixes "resets ". Pure given `now`.
pub fn format_reset(resets_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Option<String> {
    Some(format!("resets {}", format_moment(resets_at?, now)))
}

/// [`format_reset`]'s moment alone ("3:45 PM", "Mon", "Sep 14"), for copy
/// that phrases it differently ("Limited until 3:45 PM"). Pure given `now`.
pub fn format_moment(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    use chrono::Local;
    let local = at.with_timezone(&Local);
    if at.signed_duration_since(now).num_hours() < 22 {
        local.format("%-I:%M %p").to_string()
    } else if at.signed_duration_since(now).num_hours() < 24 * 7 {
        local.format("%a").to_string()
    } else {
        local.format("%b %-d").to_string()
    }
}

/// Which windows the two meter lines draw: all of them when they fit,
/// else the first (the session) and the most-used of the rest — a family
/// weekly limit ("Week (opus)") that binds outranks the shared week. Ties
/// keep the engine's order. Pure.
pub fn meter_windows(windows: &[zeron_proto::AgentUsageWindow]) -> Vec<usize> {
    if windows.len() <= 2 {
        return (0..windows.len()).collect();
    }
    let binding = (1..windows.len())
        .reduce(|best, ix| {
            if windows[ix].used_fraction > windows[best].used_fraction {
                ix
            } else {
                best
            }
        })
        .unwrap_or(1);
    vec![0, binding]
}

/// A meter's short label: a family window reads as its family ("Week
/// (opus)" → "Opus") so it fits the label column. Pure.
pub fn meter_label(label: &str) -> String {
    match label
        .strip_prefix("Week (")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        Some(family) => capitalized(family),
        None => label.to_string(),
    }
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// The usage meters' tooltip — "Session: resets 3:45 PM · Week: resets Thu"
/// — naming the percent of each window the meters leave out, so hidden
/// family limits stay discoverable. Pure.
pub(crate) fn usage_tooltip(
    windows: &[zeron_proto::AgentUsageWindow],
    shown: &[usize],
    now: DateTime<Utc>,
) -> String {
    windows
        .iter()
        .enumerate()
        .map(|(ix, window)| {
            let label = if shown.contains(&ix) {
                window.label.clone()
            } else {
                format!(
                    "{} {}%",
                    window.label,
                    (window.used_fraction.clamp(0.0, 1.0) * 100.0).round() as u32
                )
            };
            match format_reset(window.resets_at, now) {
                Some(reset) => format!("{label}: {reset}"),
                None => label,
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A pool row's meta line: [`widgets::meta_line`]'s look on ONE line, so
/// every row is the same height (and the same drag slot) at any width. The
/// pool status leads and keeps its words; the tail (plan, org, usage note)
/// truncates when the pane is narrow.
fn pool_meta_line(theme: &Theme, fragments: Vec<AnyElement>) -> gpui::Div {
    let mut line = div()
        .mt(px(Theme::TEXT_STACK_GAP))
        .min_w_0()
        .max_w_full()
        .overflow_hidden()
        .flex()
        .flex_row()
        .flex_nowrap()
        .items_center()
        .gap_x(px(8.0))
        .text_size(crate::typography::ui_rems(widgets::ROW_DESCRIPTION_SIZE))
        .line_height(crate::typography::ui_rems(16.0))
        .text_color(theme.text_muted);
    let last = fragments.len().saturating_sub(1);
    for (ix, fragment) in fragments.into_iter().enumerate() {
        if ix > 0 {
            line = line.child(
                div()
                    .flex_none()
                    .text_color(theme.text_muted.opacity(0.3))
                    .child(SharedString::from("·")),
            );
        }
        line = line.child(if ix == last {
            div().min_w_0().truncate().child(fragment)
        } else {
            div().flex_none().whitespace_nowrap().child(fragment)
        });
    }
    line
}

/// One pool settings row, in the harness "Update policy" anatomy: title and
/// muted meta on the left, the control on the right edge.
fn pool_setting_row(
    theme: &Theme,
    first: bool,
    title: &'static str,
    meta: String,
    control: impl IntoElement,
) -> gpui::Div {
    div()
        .min_h(px(52.0))
        .py(px(10.0))
        .when(!first, |el| {
            el.border_t_1().border_color(widgets::row_divider(theme))
        })
        .flex()
        .flex_row()
        .items_center()
        .gap(px(16.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(widgets::row_title(theme, title))
                .child(widgets::meta_line(
                    theme,
                    vec![div().child(SharedString::from(meta)).into_any_element()],
                )),
        )
        .child(control)
}

/// The usage column's meter lines ([`meter_windows`]), shared by Settings and
/// the composer's accounts card.
pub(crate) fn render_usage_meters(
    windows: &[zeron_proto::AgentUsageWindow],
    theme: &Theme,
) -> Vec<AnyElement> {
    meter_windows(windows)
        .into_iter()
        .map(|ix| render_usage_meter(&windows[ix], theme))
        .collect()
}

/// The providers zeron can sign into, in display order: (harness, name, CLI
/// command — named in the empty-state copy, zeron settings.agents.tsx
/// `PROVIDERS`). Every agent with a login of its own is here; what each one
/// supports is documented engine-side (`agent_accounts` module docs).
pub const PROVIDERS: [(HarnessId, &str, &str); 9] = [
    (HarnessId::ClaudeCode, "Claude Code", "claude"),
    (HarnessId::Codex, "Codex", "codex"),
    (HarnessId::Cursor, "Cursor", "cursor-agent"),
    (HarnessId::Antigravity, "Antigravity", "Antigravity"),
    (HarnessId::Grok, "Grok", "grok login"),
    (HarnessId::Devin, "Devin", "devin auth login"),
    (HarnessId::Opencode, "OpenCode", "opencode auth login"),
    (HarnessId::Pi, "Pi", "pi"),
    (HarnessId::Hermes, "Hermes", "hermes auth add"),
];

/// Whether `harness` has an Accounts section (and sign-in flow). Pure.
pub fn signs_in(harness: HarnessId) -> bool {
    PROVIDERS
        .iter()
        .any(|(provider, _, _)| *provider == harness)
}

pub(crate) fn provider_name(harness: HarnessId) -> &'static str {
    PROVIDERS
        .iter()
        .find(|(provider, _, _)| *provider == harness)
        .map_or("this provider", |(_, name, _)| name)
}

/// Whether the provider reports plan usage. One that doesn't shows no meters
/// and no "usage unavailable" note — there is nothing missing. Pure.
pub fn reports_usage(harness: HarnessId) -> bool {
    harness != HarnessId::Antigravity
}

/// Providers whose agent holds exactly ONE login (Antigravity): once it is
/// connected there is nothing to add — signing in again only re-confirms it.
pub fn keeps_one_login(harness: HarnessId) -> bool {
    harness == HarnessId::Antigravity
}

/// Whether zeron switches this agent's logins. Hermes rotates through its
/// own credential pool (listed, never reordered); Antigravity keeps one.
pub fn switches_accounts(harness: HarnessId) -> bool {
    !matches!(harness, HarnessId::Hermes | HarnessId::Antigravity)
}

/// A standing note under a provider's Accounts label, for an agent whose
/// accounts work differently. Hermes owns its credential pool: zeron lists
/// it and adds to it through Hermes' own CLI, but never switches or removes
/// its entries. Pure.
pub fn provider_note(harness: HarnessId) -> Option<&'static str> {
    match harness {
        HarnessId::Hermes => Some(
            "Hermes manages its own credential pool and rotates through it. Accounts added \
             here go through `hermes auth add`; remove one with `hermes auth remove`.",
        ),
        _ => None,
    }
}

/// One way to add an account: agents that keep a login PER model provider
/// (OpenCode, Pi, Hermes) sign in to a named provider; the rest have one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginOption {
    /// The engine's `provider` param (`None` = the agent's only login).
    pub provider: Option<&'static str>,
    /// Who the user signs in to.
    pub label: &'static str,
}

/// The sign-ins zeron offers for `harness`, in button order. Pure.
pub fn login_options(harness: HarnessId) -> Vec<LoginOption> {
    let option = |provider, label| LoginOption {
        provider: Some(provider),
        label,
    };
    match harness {
        HarnessId::Opencode => vec![
            option("openai", "ChatGPT"),
            option("github-copilot", "GitHub Copilot"),
        ],
        HarnessId::Pi => vec![option("openai-codex", "ChatGPT")],
        HarnessId::Hermes => vec![
            option("openai-codex", "ChatGPT"),
            option("nous", "Nous Portal"),
        ],
        _ => vec![LoginOption {
            provider: None,
            label: provider_name(harness),
        }],
    }
}

/// The list's last row: connect the first account, or add another. Pure.
pub fn add_account_label(harness: HarnessId, empty: bool) -> String {
    if empty {
        format!("Connect a {} account", provider_name(harness))
    } else {
        "Add account".to_string()
    }
}

/// [`add_account_label`] for one [`LoginOption`]: a per-provider sign-in
/// names its provider ("Connect ChatGPT", "Add GitHub Copilot account"). Pure.
pub fn add_option_label(harness: HarnessId, option: LoginOption, empty: bool) -> String {
    match option.provider {
        None => add_account_label(harness, empty),
        Some(_) if empty => format!("Connect {}", option.label),
        Some(_) => format!("Add {} account", option.label),
    }
}

/// What the sign-in dialog says while the browser finishes — one sentence
/// per provider, same shape. Pure.
fn login_copy(harness: HarnessId, provider: Option<&str>) -> &'static str {
    match (harness, provider) {
        (HarnessId::ClaudeCode, _) => {
            "Finish signing in to Claude in your browser. The new login is saved next to \
             your current one — nothing changes until you switch."
        }
        (HarnessId::Codex, _) => {
            "Finish signing in to ChatGPT in your browser. The new login is saved next to \
             your current one — nothing changes until you switch."
        }
        (HarnessId::Cursor, _) => {
            "Finish signing in to Cursor in your browser. This mints a zeron-named API key \
             you can revoke any time from Cursor's dashboard."
        }
        (HarnessId::Antigravity, _) => {
            "Finish signing in to Google in your browser. Antigravity keeps one login on \
             this device; if it is already signed in, this just confirms it."
        }
        (HarnessId::Grok, _) => {
            "Finish signing in to Grok in your browser — approve the code shown below. The \
             new login is saved next to your current one — nothing changes until you switch."
        }
        (HarnessId::Devin, _) => {
            "Finish signing in to Devin in your browser. The new login is saved next to your \
             current one — nothing changes until you switch."
        }
        (HarnessId::Opencode, Some("github-copilot")) => {
            "Finish signing in to GitHub in your browser — enter the code shown below. The \
             new login is saved next to your current one — nothing changes until you switch."
        }
        (HarnessId::Opencode | HarnessId::Pi, _) => {
            "Finish signing in to ChatGPT in your browser. The agent gets its own login, saved \
             next to any current one — nothing changes until you switch."
        }
        (HarnessId::Hermes, _) => {
            "Finish signing in in your browser — enter the code shown below. Hermes adds the \
             login to its own credential pool and rotates through it itself."
        }
        _ => "Finish signing in in your browser.",
    }
}

/// Accounts of one provider, in the engine's order (slot creation). No
/// active-first re-sort: switching accounts must not move the switched-to
/// card — the Active badge already says which one is live, and a list that
/// reshuffles under the click reads as broken. Pure.
pub fn provider_accounts(
    snapshot: &AgentAccountsSnapshot,
    harness: HarnessId,
) -> Vec<&AgentAccount> {
    snapshot
        .accounts
        .iter()
        .filter(|a| a.harness == harness)
        .collect()
}

/// The optimistic half of a switch: `account` becomes the live login of its
/// group — its agent, or for agents that keep a login per model provider,
/// that provider — and every other group keeps its own. Pure.
pub fn mark_switched(snapshot: &mut AgentAccountsSnapshot, account: &AgentAccount) {
    for row in snapshot.accounts.iter_mut() {
        if row.harness == account.harness && row.provider == account.provider {
            row.active = row.id == account.id;
        }
    }
}

/// A Claude API key saved for the pool. It has no CLI login to switch to —
/// it only ever serves pooled requests. Pure.
pub fn is_api_key_slot(account: &AgentAccount) -> bool {
    account.harness == HarnessId::ClaudeCode
        && account.auth_kind == Some(zeron_proto::AgentAuthKind::ApiKey)
}

/// Whether the row can become the CLI's live login. Pure.
pub fn can_switch_live(account: &AgentAccount) -> bool {
    !account.active && account.switchable && !is_api_key_slot(account)
}

/// Whether clicking the row switches the live login: pool off, any other
/// switchable login does; pool on, the list is an order to arrange and
/// switching lives in the row's menu. Pure.
pub fn row_click_switches(account: &AgentAccount, pool_on: bool) -> bool {
    can_switch_live(account) && !pool_on
}

// ---------------------------------------------------------------------------
// Pure: account pool
// ---------------------------------------------------------------------------
//
// The pool is a mode of a provider's account list, not a second list: pool
// off, the rows behave as they always have; pool on, the same rows become an
// ordered pool with a participation switch each, and the ring marks the
// account a new routed request would use. The engine computes every status;
// the UI only words and colors it.

/// Whether `harness` can route its sessions through the account pool. Pure.
pub fn pools_accounts(harness: HarnessId) -> bool {
    matches!(harness, HarnessId::ClaudeCode | HarnessId::Codex)
}

pub fn pool_config(
    snapshot: &AgentAccountsSnapshot,
    harness: HarnessId,
) -> Option<&AccountPoolProviderConfig> {
    match harness {
        HarnessId::ClaudeCode => Some(&snapshot.pool.config.claude),
        HarnessId::Codex => Some(&snapshot.pool.config.codex),
        _ => None,
    }
}

pub fn pool_config_mut(
    snapshot: &mut AgentAccountsSnapshot,
    harness: HarnessId,
) -> Option<&mut AccountPoolProviderConfig> {
    match harness {
        HarnessId::ClaudeCode => Some(&mut snapshot.pool.config.claude),
        HarnessId::Codex => Some(&mut snapshot.pool.config.codex),
        _ => None,
    }
}

/// Whether `harness` routes through the pool on this device. Pure.
pub fn pool_enabled(snapshot: &AgentAccountsSnapshot, harness: HarnessId) -> bool {
    pool_config(snapshot, harness).is_some_and(|config| config.enabled)
}

pub fn pool_provider(
    snapshot: &AgentAccountsSnapshot,
    harness: HarnessId,
) -> Option<&AccountPoolProviderStatus> {
    snapshot
        .pool
        .providers
        .iter()
        .find(|provider| provider.harness == harness)
}

pub fn pool_account<'a>(
    snapshot: &'a AgentAccountsSnapshot,
    account_id: &str,
) -> Option<&'a AccountPoolAccountStatus> {
    snapshot
        .pool
        .accounts
        .iter()
        .find(|status| status.account_id == account_id)
}

/// The account the pool would use for a new request now, when routing is on.
pub fn pool_next_id(snapshot: &AgentAccountsSnapshot, harness: HarnessId) -> Option<&str> {
    pool_enabled(snapshot, harness)
        .then(|| pool_provider(snapshot, harness)?.next_account_id.as_deref())
        .flatten()
}

/// The provider's pool order: its rows, which the engine returns in pool
/// priority order whether or not the pool is on. Pure.
pub fn pool_order(snapshot: &AgentAccountsSnapshot, harness: HarnessId) -> Vec<String> {
    provider_accounts(snapshot, harness)
        .into_iter()
        .filter(|account| {
            pool_config(snapshot, harness).is_none_or(|config| {
                config.account_order.contains(&account.id)
                    || config.account_order.is_empty()
                        && pool_account(snapshot, &account.id).is_none_or(|status| status.poolable)
            })
        })
        .map(|account| account.id.clone())
        .collect()
}

/// The optimistic half of a reorder: the provider's rows take `order` in
/// place — every other provider's rows keep their slots — and the config
/// records it. Accounts `order` doesn't name keep their relative order at
/// the end. Pure.
pub fn apply_pool_order(
    snapshot: &mut AgentAccountsSnapshot,
    harness: HarnessId,
    order: &[String],
) {
    let slots: Vec<usize> = snapshot
        .accounts
        .iter()
        .enumerate()
        .filter(|(_, account)| account.harness == harness)
        .map(|(ix, _)| ix)
        .collect();
    let mut rows: Vec<AgentAccount> = slots
        .iter()
        .map(|&ix| snapshot.accounts[ix].clone())
        .collect();
    rows.sort_by_key(|row| {
        order
            .iter()
            .position(|id| *id == row.id)
            .unwrap_or(usize::MAX)
    });
    for (slot, row) in slots.into_iter().zip(rows) {
        snapshot.accounts[slot] = row;
    }
    if let Some(config) = pool_config_mut(snapshot, harness) {
        config.account_order = order.to_vec();
    }
}

/// A pointer-free reorder from a row's `⋯` menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolMove {
    Top,
    Up,
    Down,
}

impl PoolMove {
    pub fn label(self) -> &'static str {
        match self {
            PoolMove::Top => "Move to top",
            PoolMove::Up => "Move up",
            PoolMove::Down => "Move down",
        }
    }

    /// Where the row at `ix` of `count` lands, or `None` at the list's edge
    /// (the menu hides that entry). Pure.
    pub fn target(self, ix: usize, count: usize) -> Option<usize> {
        match self {
            PoolMove::Top | PoolMove::Up if ix == 0 => None,
            PoolMove::Top => Some(0),
            PoolMove::Up => Some(ix - 1),
            PoolMove::Down => (ix + 1 < count).then_some(ix + 1),
        }
    }
}

/// `order` with the id at `from` moved to `to`. Pure.
pub fn moved(order: &[String], from: usize, to: usize) -> Vec<String> {
    let mut order = order.to_vec();
    if from < order.len() {
        let id = order.remove(from);
        order.insert(to.min(order.len()), id);
    }
    order
}

/// How a pool status fragment is colored — the engine's status, read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolTone {
    Accent,
    Muted,
    Warning,
    Danger,
}

pub fn pool_tone_color(tone: PoolTone, theme: &Theme) -> Hsla {
    match tone {
        PoolTone::Accent => theme.accent,
        PoolTone::Muted => theme.text_muted,
        PoolTone::Warning => theme.warning,
        PoolTone::Danger => theme.danger,
    }
}

/// The one pool fragment a row's meta line carries, or `None` when the row
/// has nothing to say (a ready account in priority order). `excluded` is
/// the config's say — it wins over a status the engine computed before the
/// switch flipped. Pure given `now`.
pub fn pool_status_fragment(
    status: Option<&AccountPoolAccountStatus>,
    excluded: bool,
    mode: AccountPoolRoutingMode,
    now: DateTime<Utc>,
) -> Option<(String, PoolTone)> {
    if excluded {
        return Some(("Not in pool".into(), PoolTone::Muted));
    }
    let status = status?;
    let until = |prefix: &str, fallback: &str| match status.until {
        Some(at) => format!("{prefix} {}", format_moment(at, now)),
        None => fallback.to_string(),
    };
    Some(match status.status {
        AccountPoolStatus::Next => (
            match mode {
                AccountPoolRoutingMode::WeeklyReset => "Next",
                AccountPoolRoutingMode::Sequential => "Current",
            }
            .into(),
            PoolTone::Accent,
        ),
        AccountPoolStatus::Ready => match mode {
            // The sort key in this mode — worth showing.
            AccountPoolRoutingMode::WeeklyReset => (
                format!("Week {}", format_reset(status.weekly_reset_at, now)?),
                PoolTone::Muted,
            ),
            AccountPoolRoutingMode::Sequential => return None,
        },
        AccountPoolStatus::Limited => (until("Limited until", "Limited"), PoolTone::Warning),
        AccountPoolStatus::FamilyLimited => {
            let family = status
                .family
                .as_deref()
                .map_or_else(|| "Model".to_string(), capitalized);
            let mut label = format!("{family} limit");
            if let Some(reset) = format_reset(status.until, now) {
                label.push_str(&format!(" · {reset}"));
            }
            (label, PoolTone::Warning)
        }
        AccountPoolStatus::Held => (
            until("Rate limited · retry", "Rate limited"),
            PoolTone::Warning,
        ),
        AccountPoolStatus::NeedsSignIn => ("Needs sign-in".into(), PoolTone::Danger),
        AccountPoolStatus::LiveExpired => {
            ("Skipped · live token expired".into(), PoolTone::Warning)
        }
        AccountPoolStatus::Excluded => ("Not in pool".into(), PoolTone::Muted),
        AccountPoolStatus::Unsupported => ("Can't join the pool".into(), PoolTone::Muted),
    })
}

/// The longer story behind a status fragment, for its tooltip. Pure.
pub fn pool_status_tooltip(status: &AccountPoolAccountStatus) -> Option<String> {
    match status.status {
        AccountPoolStatus::LiveExpired => Some(
            "The pool never refreshes the live login's token. Use the CLI once or switch \
             the live login."
                .into(),
        ),
        AccountPoolStatus::NeedsSignIn => status.error.clone(),
        AccountPoolStatus::Unsupported => status.unpoolable_reason.clone(),
        _ => None,
    }
}

/// "alice@example.com" → "alice@…": enough to tell accounts apart in a
/// summary line that also carries counts and a reset. Pure.
pub fn short_email(email: &str) -> String {
    match email.split_once('@') {
        Some((local, _)) => format!("{local}@…"),
        None => email.to_string(),
    }
}

fn account_label(account: &AgentAccount) -> String {
    account
        .email
        .clone()
        .or_else(|| account.display_name.clone())
        .unwrap_or_else(|| "Unknown account".into())
}

/// The pool switch row's live summary: "3 of 4 accounts · Next: alice@… ·
/// week resets Thu" (priority order: "· Current: alice@…"). Pure given `now`.
pub fn pool_summary(
    snapshot: &AgentAccountsSnapshot,
    harness: HarnessId,
    now: DateTime<Utc>,
) -> String {
    let accounts = provider_accounts(snapshot, harness);
    let Some(config) = pool_config(snapshot, harness) else {
        return String::new();
    };
    let members = accounts
        .iter()
        .filter(|account| !config.disabled_account_ids.contains(&account.id))
        .filter(|account| pool_account(snapshot, &account.id).is_none_or(|s| s.poolable))
        .count();
    let mut summary = format!(
        "{members} of {} account{}",
        accounts.len(),
        if accounts.len() == 1 { "" } else { "s" }
    );
    let next = pool_next_id(snapshot, harness)
        .and_then(|id| accounts.iter().find(|account| account.id == id));
    if let Some(next) = next {
        let label = match config.routing_mode {
            AccountPoolRoutingMode::WeeklyReset => "Next",
            AccountPoolRoutingMode::Sequential => "Current",
        };
        summary.push_str(&format!(
            " · {label}: {}",
            short_email(&account_label(next))
        ));
        if config.routing_mode == AccountPoolRoutingMode::WeeklyReset
            && let Some(reset) = pool_account(snapshot, &next.id)
                .and_then(|status| format_reset(status.weekly_reset_at, now))
        {
            summary.push_str(&format!(" · week {reset}"));
        }
    }
    summary
}

/// The routing modes, in menu order: (mode, label, what it does).
pub const POOL_MODES: [(AccountPoolRoutingMode, &str, &str); 2] = [
    (
        AccountPoolRoutingMode::WeeklyReset,
        "Soonest weekly reset",
        "Use the account whose weekly limit resets first; return to it as soon as it's usable.",
    ),
    (
        AccountPoolRoutingMode::Sequential,
        "Priority order",
        "Stay on one account until it's limited, then move down the list.",
    ),
];

/// "Move on at" presets.
pub const POOL_THRESHOLDS: [f32; 4] = [0.90, 0.95, 0.98, 1.0];

/// The threshold menu: the presets plus the engine's value when it isn't
/// one, and which entry is selected. Pure.
pub fn threshold_options(current: f32) -> (Vec<f32>, usize) {
    let mut options = POOL_THRESHOLDS.to_vec();
    let selected = match options
        .iter()
        .position(|preset| (preset - current).abs() < 0.0005)
    {
        Some(ix) => ix,
        None => {
            options.push(current);
            options.len() - 1
        }
    };
    (options, selected)
}

pub fn threshold_label(threshold: f32) -> String {
    format!("{}%", (threshold * 100.0).round() as u32)
}

/// The static note under an enabled pool's rows. Pure.
pub fn pool_footnote(harness: HarnessId, mode: AccountPoolRoutingMode) -> String {
    let mut note = match mode {
        AccountPoolRoutingMode::WeeklyReset => {
            "Order breaks ties and ranks accounts with no known reset. Changes apply from each \
             chat's next turn."
        }
        AccountPoolRoutingMode::Sequential => "Requests use the first ready account from the top.",
    }
    .to_string();
    if harness == HarnessId::Codex {
        note.push_str(" Voice chats always use the live login.");
    }
    note
}

/// One mini meter line of the usage column: label, a short bar, percent.
/// The reset moment rides the row's tooltip instead of taking a column.
pub(crate) fn render_usage_meter(
    window: &zeron_proto::AgentUsageWindow,
    theme: &Theme,
) -> AnyElement {
    let fraction = window.used_fraction.clamp(0.0, 1.0);
    let level = usage_level(fraction);
    let fill = usage_color(level, theme).opacity(match level {
        UsageLevel::Normal => 0.8,
        _ => 0.9,
    });
    div()
        .h(px(16.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .text_size(crate::typography::ui_rems(11.5))
        .child(
            div()
                .w(px(USAGE_LABEL_WIDTH))
                .flex_none()
                .truncate()
                .text_color(theme.text_muted)
                .child(SharedString::from(meter_label(&window.label))),
        )
        .child(
            div()
                .w(px(USAGE_BAR_WIDTH))
                // Full length in the usual column; shorter (never gone) when
                // a pool row's column narrows.
                .flex_shrink(1.0)
                .min_w(px(28.0))
                .h(px(4.0))
                .rounded_full()
                .overflow_hidden()
                .bg(theme.wash(0.08))
                .when(fraction > 0.0, |el| {
                    el.child(
                        div()
                            .h_full()
                            // A 1.5% floor keeps tiny non-zero usage
                            // visible (zeron `max(used, 1.5)%`).
                            .w(gpui::relative(fraction.max(0.015)))
                            .rounded_full()
                            .bg(fill),
                    )
                }),
        )
        .child(
            div()
                .w(px(USAGE_PERCENT_WIDTH))
                .flex_none()
                .text_right()
                .text_color(match level {
                    UsageLevel::Normal => theme.text_muted,
                    _ => usage_color(level, theme),
                })
                .child(SharedString::from(format!(
                    "{}%",
                    (fraction * 100.0).round() as u32
                ))),
        )
        .into_any_element()
}

// ---------------------------------------------------------------------------
// Entity
// ---------------------------------------------------------------------------

/// One sign-in, whichever provider: started, then waiting on the browser
/// (or, Claude's fallback, on a pasted code), until it lands or fails.
struct LoginFlow {
    harness: HarnessId,
    /// The model provider signed in to, for per-provider agents.
    provider: Option<&'static str>,
    /// Which start this is — replies to a cancelled or superseded attempt
    /// must not touch the current one.
    attempt: u64,
    /// Known once StartAgentLogin replied.
    login_id: Option<String>,
    /// The sign-in page, once known (the start reply, or a later poll).
    url: Option<String>,
    step: LoginStep,
}

#[derive(Debug, Clone, PartialEq)]
enum LoginStep {
    /// Starting, then waiting for the browser to reach the loopback callback.
    Browser { message: Option<SharedString> },
    /// Claude's fallback when no loopback port could be bound.
    PasteCode {
        submitting: bool,
        error: Option<SharedString>,
    },
    /// The start, a poll, or the provider failed — shown with Retry.
    Failed { message: SharedString },
}

impl LoginFlow {
    fn title(&self) -> String {
        let option = login_options(self.harness)
            .into_iter()
            .find(|option| option.provider == self.provider);
        match option {
            Some(LoginOption {
                provider: Some(_),
                label,
            }) => format!("Sign in to {label} for {}", provider_name(self.harness)),
            _ => format!("Sign in to {}", provider_name(self.harness)),
        }
    }

    /// The Browser step's progress line.
    fn status(&self) -> SharedString {
        match &self.step {
            LoginStep::Browser {
                message: Some(message),
            } => message.clone(),
            LoginStep::Browser { .. } if self.url.is_some() => {
                "Waiting for you to finish in the browser…".into()
            }
            _ => "Starting sign-in…".into(),
        }
    }
}

/// The last accounts list per target device (`None` = this device), shared
/// by every accounts view so a re-opened Settings paints instantly and
/// revalidates in place instead of flashing a skeleton.
#[derive(Default)]
pub(crate) struct AccountsSnapshotCache(
    pub(crate) std::collections::HashMap<Option<String>, AgentAccountsSnapshot>,
);

impl gpui::Global for AccountsSnapshotCache {}

pub struct AccountsPage {
    state: Entity<AppState>,
    embedded: bool,
    embedded_harness: Option<HarnessId>,
    scroll: widgets::PageScroll,
    /// Which device's logins are shown; `None` = this device (no passthrough).
    /// Retargeted by the page-header device switcher (zeron parity: the
    /// accounts RPCs are relay-forwardable, CLI logins are per-device).
    target_device: Option<String>,
    device_select: widgets::SelectState,
    snapshot: Loadable<AgentAccountsSnapshot>,
    /// A list is in flight over an already-painted snapshot.
    refreshing: bool,
    /// Account id with an in-flight Switch/Forget.
    busy_account: Option<String>,
    /// The account whose `⋯` actions menu is open.
    row_menu: popover::Popup<String>,
    login: Option<LoginFlow>,
    login_attempts: u64,
    error: Option<SharedString>,
    code_input: Entity<ComposerInput>,
    load_task: Option<Task<()>>,
    action_task: Option<Task<()>>,
    poll_task: Option<Task<()>>,
    /// The pool's Routing / Move-on-at dropdowns, per pooled provider.
    pool_selects: HashMap<(HarnessId, PoolSelect), widgets::SelectState>,
    /// The last confirmed list under a chain of in-flight pool changes: a
    /// refusal restores it (and every row slides back to where it was).
    pool_baseline: Option<AgentAccountsSnapshot>,
    pool_seq: u64,
    pool_task: Option<Task<()>>,
    pool_queue: VecDeque<(HarnessId, AccountPoolProviderConfig)>,
    /// Providers whose pool controls are leaving (the pool was switched
    /// off), and since when — they play `menu_out` before unmounting.
    pool_closing: HashMap<HarnessId, Instant>,
    /// A row dragged by pointer.
    pool_drag: Option<PoolDragState>,
    /// A row picked up from the keyboard (Space), moved with ↑/↓.
    pool_grab: Option<PoolGrab>,
    /// Rows that just changed slot without a pointer: account id → (start
    /// offset in px, epoch), slid to rest over `TAB_SLIDE`.
    pool_slides: HashMap<String, (f32, Instant)>,
    /// The "Add API key" dialog (Claude Code's pool only), while open.
    api_key: Option<ApiKeyFlow>,
    api_key_input: Entity<ComposerInput>,
    api_label_input: Entity<ComposerInput>,
    api_key_task: Option<Task<()>>,
    _pool_poll: Task<()>,
    _observe: Subscription,
    _code_events: Subscription,
    _api_key_events: Subscription,
}

/// The "Add API key" dialog's state. The key itself lives only in its
/// input until submitted — never in the page, the cache, or an error.
#[derive(Debug, Clone, Default, PartialEq)]
struct ApiKeyFlow {
    submitting: bool,
    error: Option<SharedString>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum PoolSelect {
    Routing,
    Threshold,
}

/// One row's height in a pool list (`min_h` 56 + 2×2 wrapper padding + the
/// divider): the unit of drag slots and reorder slides.
const POOL_ROW_SLOT: f32 = 61.0;
/// The order slot before the avatar (position digit ↔ grip).
const POOL_ORDER_WIDTH: f32 = 20.0;
/// The account row's gap between columns; a pool column fading in or out
/// cancels its share of it so the neighbours glide instead of jumping.
const ROW_GAP: f32 = 14.0;
const POOL_EXCLUDED_OPACITY: f32 = 0.55;
/// A pool row's floors on a narrow pane: the identity column keeps an
/// email's worth of room; the meters keep label, a short bar, and percent.
const POOL_IDENTITY_MIN_WIDTH: f32 = 96.0;
const POOL_USAGE_MIN_WIDTH: f32 = USAGE_LABEL_WIDTH + USAGE_PERCENT_WIDTH + 16.0 + 28.0;
/// Background revalidation while the pool is on: no provider calls (usage
/// stays cached), but statuses move as requests flow.
const POOL_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// A pool row being dragged; scoped to its provider so a drag can't land
/// in another provider's list.
pub struct PoolDragPayload {
    harness: HarnessId,
    from: usize,
}

/// Where a dragged row would land (the queue's drag state).
struct PoolDragState {
    harness: HarnessId,
    from: usize,
    over: usize,
    prev_over: usize,
    /// When `over` last changed: rows in the path slide from `prev_over`.
    moved_at: Instant,
    pointer: f32,
    grab: f32,
}

/// Eased 0..1 progress of a reorder slide started at `started`.
fn slide_progress(started: Instant) -> f32 {
    let total = motion::TAB_SLIDE.total().mul_f32(motion::speed_scale());
    if total.is_zero() {
        return 1.0;
    }
    motion::TAB_SLIDE.progress(started.elapsed().as_secs_f32() / total.as_secs_f32())
}

/// Top of a dragged row's lifted copy: under the pointer where it was
/// grabbed, kept within the list (the queue's `queue_lifted_top`). Pure.
fn pool_lifted_top(pointer: f32, grab: f32, count: usize) -> f32 {
    (pointer - grab).clamp(0.0, count.saturating_sub(1) as f32 * POOL_ROW_SLOT)
}

/// A keyboard pick-up: the order to restore on Escape, and the live one.
struct PoolGrab {
    harness: HarnessId,
    id: String,
    original: Vec<String>,
    order: Vec<String>,
}

/// The invisible cursor ghost of a pool drag: the real row stays in the
/// list and moves between slots.
struct PoolGhost;

impl Render for PoolGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// Paint offsets of row `ix` while `from` is dragged over `over` (the
/// queue's reorder treatment): rows in the path slide into the gap. Pure.
fn pool_drag_offsets(ix: usize, from: usize, prev_over: usize, over: usize) -> (f32, f32) {
    use crate::terminal::panel::slide_offset;
    (
        slide_offset(ix, from, prev_over) * POOL_ROW_SLOT,
        slide_offset(ix, from, over) * POOL_ROW_SLOT,
    )
}

/// Per-row pool presentation, resolved once per provider list.
#[derive(Clone)]
struct PoolRowView {
    /// Routing is on (vs. the chrome fading out after it turned off).
    on: bool,
    reorderable: bool,
    /// 0..1 reveal of the order slot and participation switch.
    chrome: f32,
    mode: AccountPoolRoutingMode,
    status: Option<AccountPoolAccountStatus>,
    excluded: bool,
    next: bool,
    position: usize,
    count: usize,
    grabbed: bool,
    /// The copy of a dragged row that follows the pointer.
    lifted: bool,
    /// Paint offset (px) of a row sliding to its slot.
    offset: f32,
    /// The dragged row's own slot, held open while its lifted copy moves.
    hidden: bool,
}

impl AccountsPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        Self::new_with_layout(state, false, None, None, cx)
    }

    pub fn new_embedded(
        state: Entity<AppState>,
        target_device: Option<String>,
        harness: HarnessId,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_layout(state, true, target_device, Some(harness), cx)
    }

    fn new_with_layout(
        state: Entity<AppState>,
        embedded: bool,
        target_device: Option<String>,
        embedded_harness: Option<HarnessId>,
        cx: &mut Context<Self>,
    ) -> Self {
        let observe = cx.observe(&state, |_, _, cx| cx.notify());
        let code_input = cx.new(|cx| ComposerInput::new("Paste the authorization code", cx));
        let code_events = cx.subscribe(&code_input, |this: &mut Self, _, event, cx| {
            if matches!(event, ComposerInputEvent::Submitted) {
                this.submit_code(cx);
            }
        });
        let api_key_input = cx.new(|cx| ComposerInput::new("sk-ant-…", cx));
        let api_label_input = cx.new(|cx| ComposerInput::new("Label (optional)", cx));
        let api_key_events = cx.subscribe(&api_key_input, |this: &mut Self, _, event, cx| {
            if matches!(event, ComposerInputEvent::Submitted) {
                this.submit_api_key(cx);
            }
        });
        let mut page = Self {
            state,
            embedded,
            embedded_harness,
            scroll: widgets::PageScroll::default(),
            target_device,
            device_select: widgets::SelectState::default(),
            snapshot: Loadable::Idle,
            refreshing: false,
            busy_account: None,
            row_menu: popover::Popup::default(),
            login: None,
            login_attempts: 0,
            error: None,
            code_input,
            load_task: None,
            action_task: None,
            poll_task: None,
            pool_selects: HashMap::new(),
            pool_baseline: None,
            pool_seq: 0,
            pool_task: None,
            pool_queue: VecDeque::new(),
            pool_closing: HashMap::new(),
            pool_drag: None,
            pool_grab: None,
            pool_slides: HashMap::new(),
            api_key: None,
            api_key_input,
            api_label_input,
            api_key_task: None,
            _api_key_events: api_key_events,
            _pool_poll: cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(POOL_POLL_INTERVAL).await;
                    let alive = this.update(cx, |page, cx| {
                        let pooled = page.snapshot.ready().is_some_and(|snapshot| {
                            [HarnessId::ClaudeCode, HarnessId::Codex]
                                .into_iter()
                                .filter(|h| page.embedded_harness.is_none_or(|e| e == *h))
                                .any(|h| pool_enabled(snapshot, h))
                        });
                        if pooled && !page.refreshing && page.pool_baseline.is_none() {
                            page.load_with(false, true, cx);
                        }
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            }),
            _observe: observe,
            _code_events: code_events,
        };
        // Paint from the last list (this session) or the engine's persisted
        // usage, then force one probe to update in place — the skeleton only
        // ever shows on a genuinely first load.
        page.load(force_usage_for(LoadTrigger::Mount), cx);
        page
    }

    /// Retarget the page at another device's logins: every accounts RPC is
    /// relay-forwardable, so the whole page — list, usage probes, switch,
    /// forget, login flows — follows the passthrough.
    pub(crate) fn set_target_device(&mut self, target: Option<String>, cx: &mut Context<Self>) {
        widgets::close_select(self, |page: &mut Self| &mut page.device_select, cx);
        if self.target_device == target {
            cx.notify();
            return;
        }
        self.target_device = target;
        // A different device = a different accounts world: drop in-flight
        // login/action state and reload with a forced usage probe (the new
        // device's cache is cold).
        self.login = None;
        self.busy_account = None;
        self.error = None;
        self.pool_baseline = None;
        self.pool_seq += 1;
        self.pool_queue.clear();
        self.pool_task = None;
        self.pool_drag = None;
        self.pool_grab = None;
        self.load(force_usage_for(LoadTrigger::Mount), cx);
    }

    pub(crate) fn set_embedded_harness(&mut self, harness: HarnessId, cx: &mut Context<Self>) {
        if self.embedded_harness != Some(harness) {
            self.embedded_harness = Some(harness);
            self.login = None;
            self.error = None;
            cx.notify();
        }
    }

    /// Visual fixtures: paint a synthetic list (no engine, no provider
    /// calls) and optionally open one row's `⋯` menu.
    #[doc(hidden)]
    pub fn fixture_snapshot(
        &mut self,
        snapshot: AgentAccountsSnapshot,
        open_menu: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        self.load_task = None;
        self.refreshing = false;
        self.error = None;
        self.snapshot = Loadable::Ready(snapshot);
        match open_menu {
            Some(id) => self.row_menu.open(id.to_string()),
            None => self.row_menu = popover::Popup::default(),
        }
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn embedded_harness(&self) -> Option<HarnessId> {
        self.embedded_harness
    }

    /// Params with the `targetDeviceId` passthrough merged in.
    fn params(&self, value: serde_json::Value) -> serde_json::Value {
        let mut value = value;
        if let (Some(target), Some(object)) = (&self.target_device, value.as_object_mut()) {
            object.insert("targetDeviceId".into(), serde_json::json!(target));
        }
        value
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    /// The page-header device switcher (zeron device-switcher.tsx): a quiet
    /// trigger — platform glyph · name · presence dot · sort glyph — opening a
    /// dropdown of every registered device. Selecting one retargets the page.
    fn render_device_switcher(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        use crate::icons::{self, icon};
        let (mut devices, local_id) = {
            let s = self.state.read(cx);
            (s.devices.clone(), s.local_device_id.clone())
        };
        // Stable row order (registration time, then id) — zeron's switcher
        // sorts the same way so rows never reshuffle on heartbeats.
        devices.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        let effective = self.target_device.clone().or_else(|| local_id.clone());
        let platform_glyph = |platform: &str| match platform {
            "macos" | "darwin" => icons::LAPTOP,
            "ios" | "android" => icons::SMARTPHONE,
            _ => icons::MONITOR,
        };
        // Local device = no passthrough (calls stay direct).
        let mut targets: Vec<Option<String>> = Vec::new();
        let mut options = Vec::new();
        for device in &devices {
            let is_local = local_id.as_deref() == Some(device.id.as_str());
            let glyph = platform_glyph(&device.platform);
            let muted = theme.text_muted;
            let option = widgets::SelectOption::new(device.name.clone()).leading(move || {
                icon(glyph)
                    .size(px(16.0))
                    .flex_none()
                    .text_color(muted)
                    .into_any_element()
            });
            options.push(if is_local {
                option.detail("You")
            } else {
                option
            });
            targets.push((!is_local).then(|| device.id.clone()));
        }
        let selected = match devices
            .iter()
            .position(|d| Some(d.id.as_str()) == effective.as_deref())
        {
            Some(ix) => ix,
            // Not registered (yet): keep the current target reachable.
            None => {
                let muted = theme.text_muted;
                options.push(widgets::SelectOption::new("This device").leading(move || {
                    icon(icons::LAPTOP)
                        .size(px(16.0))
                        .flex_none()
                        .text_color(muted)
                        .into_any_element()
                }));
                targets.push(self.target_device.clone());
                options.len() - 1
            }
        };
        widgets::select(
            "accounts-device-switcher",
            "Device",
            theme,
            |page: &mut Self| &mut page.device_select,
        )
        .options(options, selected)
        .menu_width(260.0)
        .heading("Devices")
        .on_select(move |page, ix, _, cx| {
            if let Some(target) = targets.get(ix) {
                page.set_target_device(target.clone(), cx);
            }
        })
        .render(&self.device_select, cx)
        .into_any_element()
    }

    /// Stale-while-revalidate: a painted snapshot stays on screen while the
    /// list runs. With nothing to paint yet, a forced load first takes the
    /// engine's plain list (last known usage, no network) so rows appear at
    /// once, then the probed list replaces it.
    fn load(&mut self, force_usage: bool, cx: &mut Context<Self>) {
        self.load_with(force_usage, false, cx);
    }

    /// [`Self::load`]; a `quiet` one (the pool's background revalidation)
    /// shows no refreshing spinner.
    fn load_with(&mut self, force_usage: bool, quiet: bool, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.snapshot = Loadable::Error("Engine not connected".into());
            return;
        };
        let key = self.target_device.clone();
        if !matches!(self.snapshot, Loadable::Ready(_)) {
            self.snapshot = match cx
                .try_global::<AccountsSnapshotCache>()
                .and_then(|cache| cache.0.get(&key))
            {
                Some(cached) => Loadable::Ready(cached.clone()),
                None => Loadable::Loading,
            };
        }
        let paint_first = force_usage && !matches!(self.snapshot, Loadable::Ready(_));
        self.refreshing = !quiet;
        let params = self.params(serde_json::json!({ "forceUsage": force_usage }));
        let plain = self.params(serde_json::json!({ "forceUsage": false }));
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let fetch = |params: serde_json::Value| {
                let engine = engine.clone();
                async move {
                    engine
                        .client()
                        .call(methods::LIST_AGENT_ACCOUNTS, params)
                        .await
                        .map_err(|err| err.to_string())
                        .and_then(|value| {
                            serde_json::from_value::<AgentAccountsSnapshot>(value)
                                .map_err(|err| err.to_string())
                        })
                }
            };
            if paint_first && let Ok(snapshot) = fetch(plain).await {
                let key = key.clone();
                this.update(cx, |page, cx| {
                    cx.default_global::<AccountsSnapshotCache>()
                        .0
                        .insert(key, snapshot.clone());
                    page.snapshot = Loadable::Ready(snapshot);
                    cx.notify();
                })
                .ok();
            }
            let result = fetch(params).await;
            this.update(cx, |page, cx| {
                page.refreshing = false;
                match result {
                    // A pool change in flight owns the list: its reply is
                    // fresher, and this one would flick its switches back.
                    Ok(_) if page.pool_baseline.is_some() => {}
                    Ok(snapshot) => {
                        cx.default_global::<AccountsSnapshotCache>()
                            .0
                            .insert(key, snapshot.clone());
                        page.snapshot = Loadable::Ready(snapshot);
                    }
                    // A failed revalidation keeps the painted rows.
                    Err(err) if matches!(page.snapshot, Loadable::Ready(_)) => {
                        page.error = Some(err.into());
                    }
                    Err(err) => page.snapshot = Loadable::Error(err),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Switch / Forget an account, optimistically: the row changes at once
    /// and the engine's reply (the fresh list) replaces it — no reload, no
    /// spinner, no flicker. A refusal restores the previous list.
    fn account_action(
        &mut self,
        method: &'static str,
        account: &AgentAccount,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let previous = self.snapshot.ready().cloned();
        if let Loadable::Ready(snapshot) = &mut self.snapshot {
            if method == methods::ACTIVATE_AGENT_ACCOUNT {
                mark_switched(snapshot, account);
            } else {
                snapshot.accounts.retain(|row| row.id != account.id);
            }
        }
        self.busy_account = Some(account.id.clone());
        self.error = None;
        // Tolerant param shape: both `id` and `accountId` plus the harness.
        let params = self.params(serde_json::json!({
            "id": account.id,
            "accountId": account.id,
            "harness": account.harness,
        }));
        let key = self.target_device.clone();
        self.action_task = Some(cx.spawn(async move |this, cx| {
            let result = engine.client().call(method, params).await;
            this.update(cx, |page, cx| {
                page.busy_account = None;
                match result.map(serde_json::from_value::<AgentAccountsSnapshot>) {
                    Ok(Ok(snapshot)) => {
                        cx.default_global::<AccountsSnapshotCache>()
                            .0
                            .insert(key, snapshot.clone());
                        page.snapshot = Loadable::Ready(snapshot);
                    }
                    // An older engine's reply without the list: fetch it.
                    Ok(Err(_)) => page.load(force_usage_for(LoadTrigger::PostAction), cx),
                    Err(err) => {
                        if let Some(previous) = previous {
                            page.snapshot = Loadable::Ready(previous);
                        }
                        page.error = Some(format!("{err}").into());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    // ---- account pool ----

    /// Keep edits optimistic, but send them in order. Dropping an RPC waiter
    /// cannot undo a mutation that the engine has already committed.
    fn configure_pool(
        &mut self,
        harness: HarnessId,
        edit: impl FnOnce(&mut AgentAccountsSnapshot),
        cx: &mut Context<Self>,
    ) {
        let Loadable::Ready(snapshot) = &mut self.snapshot else {
            return;
        };
        if self.pool_baseline.is_none() {
            self.pool_baseline = Some(snapshot.clone());
        }
        edit(snapshot);
        let Some(config) = pool_config(snapshot, harness).cloned() else {
            return;
        };
        cx.default_global::<AccountsSnapshotCache>()
            .0
            .insert(self.target_device.clone(), snapshot.clone());
        self.error = None;
        self.pool_queue.push_back((harness, config));
        if self.pool_task.is_none() {
            self.send_next_pool_change(cx);
        }
        cx.notify();
    }

    fn send_next_pool_change(&mut self, cx: &mut Context<Self>) {
        let Some((harness, config)) = self.pool_queue.pop_front() else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.fail_pool_change("Engine not connected".into(), cx);
            return;
        };
        let params = self.params(serde_json::json!({ "harness": harness, "config": config }));
        let plain = self.params(serde_json::json!({}));
        let seq = self.pool_seq;
        self.pool_task = Some(cx.spawn(async move |this, cx| {
            let decode = |value| {
                serde_json::from_value::<AgentAccountsSnapshot>(value)
                    .map_err(|err| err.to_string())
            };
            let result = engine
                .client()
                .call(methods::CONFIGURE_ACCOUNT_POOL, params)
                .await
                .map_err(|err| err.to_string())
                .and_then(decode);
            let (snapshot, recovered_pool, error) = match result {
                Ok(snapshot) => (Some(snapshot), None, None),
                Err(error) => {
                    // Configuration may have committed before account discovery
                    // failed. Read the pool independently of that discovery.
                    let pool = engine
                        .client()
                        .call(methods::GET_ACCOUNT_POOL, plain)
                        .await
                        .ok()
                        .and_then(|value| {
                            serde_json::from_value::<zeron_proto::AccountPoolSnapshot>(value).ok()
                        });
                    (None, pool, Some(error))
                }
            };
            this.update(cx, |page, cx| {
                if page.pool_seq != seq {
                    return;
                }
                page.pool_task = None;
                let confirmed = snapshot.or_else(|| {
                    // Keep current usage and unrelated account data. If the read
                    // also failed, retain the pending view rather than claim an
                    // old baseline is authoritative after an ambiguous write.
                    let mut current = cx
                        .default_global::<AccountsSnapshotCache>()
                        .0
                        .get(&page.target_device)
                        .cloned()
                        .or_else(|| page.snapshot.ready().cloned())?;
                    if let Some(pool) = recovered_pool {
                        current.pool = pool;
                        for harness in [HarnessId::ClaudeCode, HarnessId::Codex] {
                            let order = pool_config(&current, harness)?.account_order.clone();
                            apply_pool_order(&mut current, harness, &order);
                        }
                    }
                    Some(current)
                });
                if let Some(mut confirmed) = confirmed {
                    // Bypass mutations have their own serialized composer queue.
                    // This provider edit must not overwrite a newer chat flag
                    // with one captured by its earlier snapshot/read response.
                    if let Some(current) = cx
                        .default_global::<AccountsSnapshotCache>()
                        .0
                        .get(&page.target_device)
                    {
                        confirmed.pool.config.bypassed_chat_ids =
                            current.pool.config.bypassed_chat_ids.clone();
                    }
                    page.pool_baseline = (!page.pool_queue.is_empty()).then(|| confirmed.clone());
                    for (harness, config) in &page.pool_queue {
                        apply_pool_order(&mut confirmed, *harness, &config.account_order);
                        if let Some(target) = pool_config_mut(&mut confirmed, *harness) {
                            *target = config.clone();
                        }
                    }
                    if let Some(current) = page.snapshot.ready() {
                        let moves: Vec<_> = [HarnessId::ClaudeCode, HarnessId::Codex]
                            .into_iter()
                            .map(|h| (pool_order(current, h), pool_order(&confirmed, h)))
                            .collect();
                        for (before, after) in moves {
                            page.slide_rows(&before, &after);
                        }
                    }
                    cx.default_global::<AccountsSnapshotCache>()
                        .0
                        .insert(page.target_device.clone(), confirmed.clone());
                    page.snapshot = Loadable::Ready(confirmed);
                }
                if let Some(error) = error {
                    page.error = Some(error.into());
                }
                if !page.pool_queue.is_empty() {
                    page.send_next_pool_change(cx);
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Undo every unconfirmed pool change along the path it came: rows slide
    /// back to their slots, switches travel back, then the toast.
    fn fail_pool_change(&mut self, error: String, cx: &mut Context<Self>) {
        self.pool_queue.clear();
        self.pool_task = None;
        if let (Some(baseline), Some(current)) =
            (self.pool_baseline.take(), self.snapshot.ready().cloned())
        {
            for harness in [HarnessId::ClaudeCode, HarnessId::Codex] {
                let before = pool_order(&current, harness);
                let after = pool_order(&baseline, harness);
                self.slide_rows(&before, &after);
            }
            cx.default_global::<AccountsSnapshotCache>()
                .0
                .insert(self.target_device.clone(), baseline.clone());
            self.snapshot = Loadable::Ready(baseline);
        }
        self.error = Some(error.into());
        cx.notify();
    }

    /// Rows that changed slot between `before` and `after` start where they
    /// were drawn and slide home ([`motion::TAB_SLIDE`]).
    fn slide_rows(&mut self, before: &[String], after: &[String]) {
        let now = Instant::now();
        for (to, id) in after.iter().enumerate() {
            if let Some(from) = before.iter().position(|other| other == id)
                && from != to
            {
                let offset = (from as f32 - to as f32) * POOL_ROW_SLOT;
                self.pool_slides.insert(id.clone(), (offset, now));
            }
        }
    }

    /// A row's remaining slide offset, if it is still travelling.
    fn slide_offset(&self, id: &str, reduced: bool) -> f32 {
        match self.pool_slides.get(id) {
            Some((from, started)) if !reduced => motion::lerp(*from, 0.0, slide_progress(*started)),
            _ => 0.0,
        }
    }

    /// Whether any reorder slide is mid-flight (→ keep frames coming).
    fn pool_motion_active(&self) -> bool {
        self.pool_slides
            .values()
            .any(|(_, started)| slide_progress(*started) < 1.0)
            || self
                .pool_drag
                .as_ref()
                .is_some_and(|drag| slide_progress(drag.moved_at) < 1.0)
    }

    fn set_pool_enabled(&mut self, harness: HarnessId, enabled: bool, cx: &mut Context<Self>) {
        if enabled {
            self.pool_closing.remove(&harness);
        } else {
            // The Routing / Move-on-at rows play `menu_out`, then unmount.
            self.pool_closing.insert(harness, Instant::now());
            let span = motion::MENU_OUT.total();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(span).await;
                this.update(cx, |page, cx| {
                    page.pool_closing.remove(&harness);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        if self
            .pool_grab
            .as_ref()
            .is_some_and(|grab| grab.harness == harness)
        {
            self.pool_grab = None;
        }
        self.configure_pool(
            harness,
            |snapshot| {
                if let Some(config) = pool_config_mut(snapshot, harness) {
                    config.enabled = enabled;
                }
            },
            cx,
        );
    }

    fn set_pool_mode(
        &mut self,
        harness: HarnessId,
        mode: AccountPoolRoutingMode,
        cx: &mut Context<Self>,
    ) {
        self.configure_pool(
            harness,
            |snapshot| {
                if let Some(config) = pool_config_mut(snapshot, harness) {
                    config.routing_mode = mode;
                }
            },
            cx,
        );
    }

    fn set_pool_threshold(&mut self, harness: HarnessId, threshold: f32, cx: &mut Context<Self>) {
        self.configure_pool(
            harness,
            |snapshot| {
                if let Some(config) = pool_config_mut(snapshot, harness) {
                    config.switch_threshold = threshold;
                }
            },
            cx,
        );
    }

    fn set_pool_member(
        &mut self,
        harness: HarnessId,
        account_id: &str,
        included: bool,
        cx: &mut Context<Self>,
    ) {
        let account_id = account_id.to_string();
        self.configure_pool(
            harness,
            |snapshot| {
                if let Some(config) = pool_config_mut(snapshot, harness) {
                    config.disabled_account_ids.retain(|id| *id != account_id);
                    if !included {
                        config.disabled_account_ids.push(account_id);
                    }
                }
            },
            cx,
        );
    }

    /// Commit a new order. `slide`: animate rows from their drawn slots (a
    /// drag has already moved them; menu moves haven't).
    fn commit_pool_order(
        &mut self,
        harness: HarnessId,
        order: Vec<String>,
        slide: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(before) = self.snapshot.ready().map(|s| pool_order(s, harness)) else {
            return;
        };
        if before == order {
            return;
        }
        if slide {
            self.slide_rows(&before, &order);
        }
        self.configure_pool(
            harness,
            move |snapshot| apply_pool_order(snapshot, harness, &order),
            cx,
        );
    }

    /// Move to top / up / down, from a row's menu.
    fn move_pool_row(
        &mut self,
        harness: HarnessId,
        account_id: &str,
        mv: PoolMove,
        cx: &mut Context<Self>,
    ) {
        let Some(order) = self.snapshot.ready().map(|s| pool_order(s, harness)) else {
            return;
        };
        let Some(from) = order.iter().position(|id| id == account_id) else {
            return;
        };
        if let Some(to) = mv.target(from, order.len()) {
            self.commit_pool_order(harness, moved(&order, from, to), true, cx);
        }
    }

    /// The order a provider's rows draw in: a keyboard pick-up's live order,
    /// else the list's.
    fn display_accounts<'a>(
        &self,
        snapshot: &'a AgentAccountsSnapshot,
        harness: HarnessId,
    ) -> Vec<&'a AgentAccount> {
        let mut rows = provider_accounts(snapshot, harness);
        if let Some(grab) = self
            .pool_grab
            .as_ref()
            .filter(|grab| grab.harness == harness)
        {
            rows.sort_by_key(|row| {
                grab.order
                    .iter()
                    .position(|id| *id == row.id)
                    .unwrap_or(usize::MAX)
            });
        }
        rows
    }

    /// The order handle's keyboard: Space (or Enter) picks the row up, ↑/↓
    /// move it, Space drops and commits, Escape puts everything back.
    /// Returns whether the key was handled.
    fn pool_key(
        &mut self,
        harness: HarnessId,
        account_id: &str,
        key: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let grabbed = self
            .pool_grab
            .as_ref()
            .is_some_and(|grab| grab.harness == harness && grab.id == account_id);
        match key {
            "space" | "enter" if grabbed => {
                if let Some(grab) = self.pool_grab.take()
                    && grab.order != grab.original
                {
                    // The rows already slid while moving.
                    self.commit_pool_order(harness, grab.order, false, cx);
                }
            }
            "space" | "enter" => {
                let Some(order) = self.snapshot.ready().map(|s| pool_order(s, harness)) else {
                    return false;
                };
                self.pool_grab = Some(PoolGrab {
                    harness,
                    id: account_id.to_string(),
                    original: order.clone(),
                    order,
                });
            }
            "up" | "down" if grabbed => {
                let Some(grab) = self.pool_grab.as_mut() else {
                    return false;
                };
                let mv = if key == "up" {
                    PoolMove::Up
                } else {
                    PoolMove::Down
                };
                let Some(from) = grab.order.iter().position(|id| id == account_id) else {
                    return false;
                };
                if let Some(to) = mv.target(from, grab.order.len()) {
                    let next = moved(&grab.order, from, to);
                    let before = std::mem::replace(&mut grab.order, next.clone());
                    self.slide_rows(&before, &next);
                }
            }
            "escape" if grabbed => {
                self.cancel_pool_grab();
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    /// Escape on a picked-up row: back to the order it had.
    fn cancel_pool_grab(&mut self) -> bool {
        let Some(grab) = self.pool_grab.take() else {
            return false;
        };
        self.slide_rows(&grab.order, &grab.original);
        true
    }

    /// Track the drop slot and the pointer while a row is dragged.
    fn update_pool_drag_over(
        &mut self,
        harness: HarnessId,
        from: usize,
        over: usize,
        pointer: f32,
        cx: &mut Context<Self>,
    ) {
        match &mut self.pool_drag {
            Some(drag) if drag.harness == harness && drag.from == from => {
                if drag.over != over {
                    drag.prev_over = drag.over;
                    drag.over = over;
                    drag.moved_at = Instant::now();
                }
                if drag.pointer != pointer {
                    drag.pointer = pointer;
                    cx.notify();
                }
            }
            _ => {
                self.pool_grab = None;
                self.pool_drag = Some(PoolDragState {
                    harness,
                    from,
                    over,
                    prev_over: from,
                    moved_at: Instant::now(),
                    pointer,
                    grab: (pointer - from as f32 * POOL_ROW_SLOT).clamp(0.0, POOL_ROW_SLOT),
                });
                cx.notify();
            }
        }
    }

    /// Drop: the lifted row settles into its slot and the order commits.
    fn drop_pool_row(&mut self, harness: HarnessId, from: usize, cx: &mut Context<Self>) {
        let Some(drag) = self.pool_drag.take().filter(|drag| drag.harness == harness) else {
            cx.notify();
            return;
        };
        let Some(order) = self.snapshot.ready().map(|s| pool_order(s, harness)) else {
            return;
        };
        let to = drag.over.min(order.len().saturating_sub(1));
        if let Some(id) = order.get(from) {
            let top = pool_lifted_top(drag.pointer, drag.grab, order.len());
            self.pool_slides.insert(
                id.clone(),
                (top - to as f32 * POOL_ROW_SLOT, Instant::now()),
            );
        }
        self.commit_pool_order(harness, moved(&order, from, to), false, cx);
        cx.notify();
    }

    fn cancel_pool_drag(&mut self, cx: &mut Context<Self>) {
        if self.pool_drag.take().is_some() {
            cx.notify();
        }
    }

    // ---- add-account flows ----

    /// Start (or Retry) a sign-in: the same dialog for every provider.
    fn start_login(
        &mut self,
        harness: HarnessId,
        provider: Option<&'static str>,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.login_attempts += 1;
        let attempt = self.login_attempts;
        self.login = Some(LoginFlow {
            harness,
            provider,
            attempt,
            login_id: None,
            url: None,
            step: LoginStep::Browser { message: None },
        });
        self.poll_task = None;
        self.error = None;
        let mut params = serde_json::json!({ "harness": harness });
        if let (Some(provider), Some(object)) = (provider, params.as_object_mut()) {
            object.insert("provider".into(), serde_json::json!(provider));
        }
        let params = self.params(params);
        self.action_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::START_AGENT_LOGIN, params)
                .await
                .map_err(|e| e.to_string())
                .and_then(|value| {
                    serde_json::from_value::<AgentLoginStart>(value).map_err(|e| e.to_string())
                });
            this.update(cx, |page, cx| page.apply_start(attempt, result, cx))
                .ok();
        }));
        cx.notify();
    }

    /// Fold a StartAgentLogin reply into the flow `attempt` (if still open).
    fn apply_start(
        &mut self,
        attempt: u64,
        result: Result<AgentLoginStart, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(flow) = self.login.as_mut().filter(|flow| flow.attempt == attempt) else {
            return;
        };
        match result {
            Ok(start) => {
                if !start.url.is_empty() {
                    open_login_url(&start.url, cx);
                    flow.url = Some(start.url.clone());
                }
                flow.login_id = Some(start.login_id);
                match start.mode {
                    AgentLoginMode::PasteCode => {
                        flow.step = LoginStep::PasteCode {
                            submitting: false,
                            error: None,
                        };
                        self.code_input
                            .update(cx, |input, cx| input.set_text("", cx));
                    }
                    AgentLoginMode::Browser => self.spawn_poll(cx),
                }
            }
            Err(error) => {
                flow.step = LoginStep::Failed {
                    message: format!("Couldn't start the sign-in: {error}").into(),
                };
            }
        }
        cx.notify();
    }

    fn submit_code(&mut self, cx: &mut Context<Self>) {
        let Some(LoginFlow {
            login_id: Some(login_id),
            step: LoginStep::PasteCode { submitting, .. },
            attempt,
            ..
        }) = &mut self.login
        else {
            return;
        };
        if *submitting {
            return;
        }
        let code = self.code_input.read(cx).text().trim().to_string();
        if code.is_empty() {
            return;
        }
        let (login_id, attempt) = (login_id.clone(), *attempt);
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        *submitting = true;
        let params = self.params(serde_json::json!({ "loginId": login_id, "code": code }));
        self.action_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::COMPLETE_AGENT_LOGIN, params)
                .await;
            this.update(cx, |page, cx| {
                let Some(flow) = page.login.as_mut().filter(|f| f.attempt == attempt) else {
                    return;
                };
                match result {
                    Ok(_) => {
                        page.login = None;
                        page.load(force_usage_for(LoadTrigger::PostLogin), cx);
                    }
                    Err(err) => {
                        flow.step = LoginStep::PasteCode {
                            submitting: false,
                            error: Some(format!("{err}").into()),
                        };
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// The browser-wait poll loop: PollAgentLogin every 1.5s until the login
    /// lands or fails.
    fn spawn_poll(&mut self, cx: &mut Context<Self>) {
        let Some(LoginFlow {
            login_id: Some(login_id),
            attempt,
            ..
        }) = &self.login
        else {
            return;
        };
        let attempt = *attempt;
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let params = self.params(serde_json::json!({ "loginId": login_id }));
        self.poll_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(1500))
                    .await;
                let result = engine
                    .client()
                    .call(methods::POLL_AGENT_LOGIN, params.clone())
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|value| {
                        serde_json::from_value::<AgentLoginPoll>(value)
                            .map_err(|_| "malformed reply".to_string())
                    });
                match this.update(cx, |page, cx| page.apply_poll(attempt, result, cx)) {
                    Ok(false) => {}
                    Ok(true) | Err(_) => break,
                }
            }
        }));
    }

    /// Fold one poll into the flow `attempt`. `true` once polling is over —
    /// the login landed (the list reloads and shows it), failed (the dialog
    /// says why and offers Retry), or the dialog is gone. Never a silent
    /// reset: every ending either shows the account or an error.
    fn apply_poll(
        &mut self,
        attempt: u64,
        result: Result<AgentLoginPoll, String>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(flow) = self.login.as_mut().filter(|flow| flow.attempt == attempt) else {
            return true;
        };
        let finished = match result {
            Ok(poll) => match poll.status {
                AgentLoginStatus::Done => {
                    self.login = None;
                    self.load(force_usage_for(LoadTrigger::PostLogin), cx);
                    true
                }
                AgentLoginStatus::Error => {
                    flow.step = LoginStep::Failed {
                        message: poll
                            .message
                            .unwrap_or_else(|| "The sign-in failed.".to_string())
                            .into(),
                    };
                    true
                }
                AgentLoginStatus::Pending => {
                    // A page learned after the start (Antigravity prints its
                    // own once its server is up): open it once.
                    if let Some(url) = poll.url.filter(|url| flow.url.as_ref() != Some(url)) {
                        open_login_url(&url, cx);
                        flow.url = Some(url);
                    }
                    flow.step = LoginStep::Browser {
                        message: poll.message.map(Into::into),
                    };
                    false
                }
            },
            Err(error) => {
                flow.step = LoginStep::Failed {
                    message: format!("Lost track of the sign-in: {error}").into(),
                };
                true
            }
        };
        cx.notify();
        finished
    }

    fn cancel_login(&mut self, cx: &mut Context<Self>) {
        let login_id = match self.login.take() {
            // A failed login is already over engine-side.
            Some(LoginFlow {
                login_id: Some(login_id),
                step: LoginStep::Browser { .. } | LoginStep::PasteCode { .. },
                ..
            }) => Some(login_id),
            _ => None,
        };
        self.poll_task = None;
        if let (Some(login_id), Some(engine)) = (login_id, self.state.read(cx).engine().cloned()) {
            let params = self.params(serde_json::json!({ "loginId": login_id }));
            self.action_task = Some(cx.spawn(async move |_, _| {
                if let Err(err) = engine
                    .client()
                    .call(methods::CANCEL_AGENT_LOGIN, params)
                    .await
                {
                    tracing::debug!(error = %err, "CancelAgentLogin failed (best-effort)");
                }
            }));
        }
        cx.notify();
    }

    /// Escape that reached Settings unclaimed cancels an open login (or
    /// API-key dialog, or a picked-up pool row) first, so it never closes
    /// Settings under them. Returns whether it did.
    pub(crate) fn dismiss_on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        if self.api_key.is_some() {
            self.close_api_key(cx);
            return true;
        }
        if self.cancel_pool_grab() {
            cx.notify();
            return true;
        }
        if self.login.is_none() {
            return false;
        }
        self.cancel_login(cx);
        true
    }

    // ---- pool API keys ----

    fn open_api_key(&mut self, cx: &mut Context<Self>) {
        self.api_key = Some(ApiKeyFlow::default());
        self.api_key_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.api_label_input
            .update(cx, |input, cx| input.set_text("", cx));
        cx.notify();
    }

    /// Close the dialog and drop the typed key with it.
    fn close_api_key(&mut self, cx: &mut Context<Self>) {
        self.api_key = None;
        self.api_key_task = None;
        self.api_key_input
            .update(cx, |input, cx| input.set_text("", cx));
        self.api_label_input
            .update(cx, |input, cx| input.set_text("", cx));
        cx.notify();
    }

    /// Save the key as a pool-only Claude account. The engine's reply (the
    /// fresh list) shows the new row; a refusal stays in the dialog.
    fn submit_api_key(&mut self, cx: &mut Context<Self>) {
        if self.api_key.as_ref().is_none_or(|flow| flow.submitting) {
            return;
        }
        let api_key = self.api_key_input.read(cx).text().trim().to_string();
        if api_key.is_empty() {
            return;
        }
        let label = self.api_label_input.read(cx).text().trim().to_string();
        let engine = self.state.read(cx).engine().cloned();
        let Some(flow) = self.api_key.as_mut() else {
            return;
        };
        let Some(engine) = engine else {
            flow.error = Some("Engine not connected".into());
            cx.notify();
            return;
        };
        flow.submitting = true;
        flow.error = None;
        let mut params = serde_json::json!({ "harness": HarnessId::ClaudeCode, "apiKey": api_key });
        if let (false, Some(object)) = (label.is_empty(), params.as_object_mut()) {
            object.insert("label".into(), serde_json::json!(label));
        }
        let params = self.params(params);
        let key = self.target_device.clone();
        self.api_key_task = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(methods::ADD_ACCOUNT_POOL_API_KEY, params)
                .await
                .map_err(|err| err.to_string())
                .and_then(|value| {
                    serde_json::from_value::<AgentAccountsSnapshot>(value)
                        .map_err(|err| err.to_string())
                });
            this.update(cx, |page, cx| {
                match result {
                    Ok(snapshot) => {
                        cx.default_global::<AccountsSnapshotCache>()
                            .0
                            .insert(key, snapshot.clone());
                        page.snapshot = Loadable::Ready(snapshot);
                        page.close_api_key(cx);
                    }
                    Err(err) => {
                        if let Some(flow) = page.api_key.as_mut() {
                            flow.submitting = false;
                            flow.error = Some(format!("Couldn't add the key: {err}").into());
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// "Add API key" beside the sign-in action — Claude Code only, and only
    /// while its pool is on: a key serves pooled requests and nothing else,
    /// so a pool-off list stays exactly as it was.
    fn render_add_api_key(
        &self,
        harness: HarnessId,
        snapshot: &AgentAccountsSnapshot,
        theme: &Theme,
        standalone: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if harness != HarnessId::ClaudeCode || !pool_enabled(snapshot, harness) {
            return None;
        }
        let label = "Add API key";
        let button = if standalone {
            widgets::ghost_action(theme).id("add-pool-api-key").child(
                crate::icons::icon(crate::icons::ADD_CIRCLE)
                    .size(px(16.0))
                    .text_color(theme.text_muted),
            )
        } else {
            widgets::action_button(theme, widgets::ActionTone::Quiet)
                .id("add-pool-api-key")
                .child(
                    crate::icons::icon(crate::icons::PLUS)
                        .size(px(14.0))
                        .text_color(theme.text_muted),
                )
        };
        Some(
            motion::fade_quick(
                "add-pool-api-key-reveal",
                button
                    .tab_index(0)
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .tooltip(widgets::text_tooltip(
                        "Pool an Anthropic API key — used only for pooled requests",
                    ))
                    .focus_visible(|s| s.border_2().border_color(theme.accent).opacity(1.0))
                    .on_click(cx.listener(|page, _, _, cx| page.open_api_key(cx)))
                    .child(SharedString::from(label)),
            )
            .into_any_element(),
        )
    }

    /// The "Add API key" dialog: the sign-in dialog's anatomy with the
    /// paste-code step's monospace field, plus an optional label.
    fn render_api_key_dialog(
        &mut self,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let flow = self.api_key.clone()?;
        let theme = Theme::of(cx).for_popup();
        let action = |id: &'static str, tone: widgets::ActionTone, label: &'static str| {
            widgets::text_action(&theme, tone, label)
                .id(id)
                .tab_index(0)
                .role(gpui::Role::Button)
                .focus_visible(|s| s.border_2().border_color(theme.accent).opacity(1.0))
        };
        let title = "Add a Claude API key";
        let card = popover::dialog_card(&theme)
            .id("add-api-key-card")
            .role(gpui::Role::Dialog)
            .aria_label(title)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.close_api_key(cx);
                    cx.stop_propagation();
                }
            }))
            .child(popover::dialog_title(&theme, title))
            .child(div().mt(px(8.0)).child(popover::dialog_body(
                &theme,
                "The key joins the Claude Code account pool and is only used for pooled \
                 requests — Claude Code's own login is untouched. It stays on this device.",
            )))
            .child(
                div()
                    .mt(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        popover::dialog_field(self.api_key_input.clone().into_any_element())
                            .font_family(theme.font_mono.clone())
                            .text_size(crate::typography::ui_rems(13.0)),
                    )
                    .child(
                        popover::dialog_field(self.api_label_input.clone().into_any_element())
                            .text_size(crate::typography::ui_rems(13.0)),
                    )
                    .when_some(flow.error.clone(), |el, message| {
                        el.child(
                            div()
                                .text_size(crate::typography::ui_rems(12.0))
                                .text_color(theme.danger_muted.opacity(0.9))
                                .child(message),
                        )
                    }),
            )
            .child(
                div()
                    .mt(px(16.0))
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        action("api-key-cancel", widgets::ActionTone::Quiet, "Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.close_api_key(cx))),
                    )
                    .child(
                        action(
                            "api-key-submit",
                            widgets::ActionTone::Solid,
                            if flow.submitting {
                                "Adding…"
                            } else {
                                "Add key"
                            },
                        )
                        .when(flow.submitting, |el| el.opacity(0.5))
                        .on_click(cx.listener(|this, _, _, cx| this.submit_api_key(cx))),
                    ),
            )
            .into_any_element();
        Some(popover::modal("add-api-key-dialog", viewport, card))
    }

    fn retry_login(&mut self, cx: &mut Context<Self>) {
        if let Some((harness, provider)) = self.login.as_ref().map(|f| (f.harness, f.provider)) {
            self.start_login(harness, provider, cx);
        }
    }

    // ---- render pieces ----

    /// The quiet note an account shows in its meta line instead of meters.
    /// Render hook for the usage data flow: a reason the probe came back
    /// empty belongs here. A provider without usage gets no note at all.
    fn render_usage_missing(&self, account: &AgentAccount, _theme: &Theme) -> Option<AnyElement> {
        if !reports_usage(account.harness) {
            return None;
        }
        let note = if !account.switchable && switches_accounts(account.harness) {
            match account.harness {
                // Keychain denied: the login is there, its secret isn't.
                HarnessId::ClaudeCode => "Credentials unavailable".to_string(),
                // An opaque token whose account couldn't be looked up.
                _ => "Couldn't identify this login".to_string(),
            }
        } else if let Some(reason) = &account.usage_error {
            // The engine's reason ("Rate limited by Anthropic — retrying in
            // 2m", "Signed out — sign in again"), not a bare shrug.
            reason.clone()
        } else if self.refreshing {
            "Checking usage…".to_string()
        } else {
            "Usage unavailable".to_string()
        };
        Some(div().child(SharedString::from(note)).into_any_element())
    }

    /// Trailing mark on an expanded provider's "Accounts" label: the app's
    /// mini loader while a list is in flight over painted rows, else nothing.
    fn render_accounts_status(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !(self.refreshing && matches!(self.snapshot, Loadable::Ready(_))) {
            return None;
        }
        let theme = Theme::of(cx).for_settings_surface();
        Some(
            crate::loaders::mini_mono_spinner(
                "accounts-refreshing",
                1.5,
                theme.text_muted,
                cx.entity_id(),
                cx,
            )
            .into_any_element(),
        )
    }

    fn close_row_menu(&mut self, cx: &mut Context<Self>) {
        if self.row_menu.begin_close() {
            popover::reap_popup(cx, |page: &mut Self| &mut page.row_menu);
        }
        cx.notify();
    }

    /// One account row. The list is a single choice, so the row itself says
    /// which account is live: its avatar wears an accent ring and its meta
    /// line reads "In use". Any other row switches when clicked; every row
    /// ends in the same quiet `⋯` menu (Switch / Remove), so the trailing
    /// edge never changes shape between states.
    ///
    /// In pool mode (`pool` with `on`) the same row is a pool member: a
    /// position slot leads it, a participation switch precedes the `⋯`, the
    /// ring marks the account the pool uses next (the live login says so in
    /// its meta line instead), and a click no longer switches the live login
    /// — that lives in the menu, beside Move up/down. Every column keeps its
    /// width between states, so a row never changes height.
    #[allow(clippy::too_many_arguments)]
    fn render_account_row(
        &self,
        account: &AgentAccount,
        ix: usize,
        first: bool,
        pool: Option<&PoolRowView>,
        theme: &Theme,
        now: DateTime<Utc>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::motion;
        let email: SharedString = account_label(account).into();
        let harness = account.harness;
        let pool_on = pool.is_some_and(|pool| pool.on);
        let lifted = pool.is_some_and(|pool| pool.lifted);
        let can_switch = can_switch_live(account);
        let click_switches = row_click_switches(account, pool_on);
        let switch_account = account.clone();
        let reduced = motion::reduced_motion(cx);
        // Pool rows are keyed by account, so a reorder moves the row — its
        // focus and fades with it — instead of re-skinning a slot.
        let row_key: SharedString = match pool {
            Some(_) => format!("account-row-{}", account.id).into(),
            None => format!("account-row-{ix}").into(),
        };
        let hover_key = format!("{row_key}-hover");

        // Personal Claude orgs are named "<email>'s Organization" — noise
        // right under the email itself.
        let organization = account.organization.clone().filter(|org| {
            account
                .email
                .as_deref()
                .is_none_or(|email| !org.starts_with(email))
        });
        let mut meta: Vec<AnyElement> = [account.plan_label.clone(), organization]
            .into_iter()
            .flatten()
            .map(|fragment| div().child(SharedString::from(fragment)).into_any_element())
            .collect();
        // A pool row's status already explains a sign-in problem; the
        // usage note would only repeat it.
        let mut usage_note = true;
        match pool {
            Some(pool) if pool.on => {
                let fragment =
                    pool_status_fragment(pool.status.as_ref(), pool.excluded, pool.mode, now);
                // The pool's words lead the line; plan and org follow and
                // are what a narrow pane truncates.
                let details = std::mem::take(&mut meta);
                usage_note = !pool.status.as_ref().is_some_and(|status| {
                    matches!(
                        status.status,
                        AccountPoolStatus::NeedsSignIn
                            | AccountPoolStatus::LiveExpired
                            | AccountPoolStatus::Unsupported
                    )
                });
                if let Some((label, tone)) = fragment {
                    let tooltip = pool
                        .status
                        .as_ref()
                        .filter(|_| !pool.excluded)
                        .and_then(pool_status_tooltip);
                    let fragment = div()
                        .id(SharedString::from(format!("pool-status-{}", account.id)))
                        .text_color(pool_tone_color(tone, theme))
                        .when_some(tooltip, |el, tip| el.tooltip(widgets::text_tooltip(tip)))
                        .child(SharedString::from(label.clone()));
                    // Keyed by the wording: only a fragment that changed fades.
                    meta.push(
                        motion::fade_quick(
                            SharedString::from(format!("pool-status-{}-{label}", account.id)),
                            fragment,
                        )
                        .into_any_element(),
                    );
                }
                if account.active {
                    meta.push(
                        div()
                            .child(SharedString::from("Live login"))
                            .into_any_element(),
                    );
                }
                meta.extend(details);
            }
            _ if account.active => meta.push(
                div()
                    .text_color(theme.accent)
                    .child(SharedString::from("In use"))
                    .into_any_element(),
            ),
            _ => {}
        }
        // An account left out of the pool dims — avatar, text, meters — and
        // brightens again, over the hover fade's timing.
        let dim = match pool {
            Some(pool) => motion::state_t(
                &format!("pool-dim-{}", account.id),
                pool.on && pool.excluded,
                motion::HOVER_FADE,
                reduced,
            ),
            None => 0.0,
        };
        let content_opacity = motion::lerp(1.0, POOL_EXCLUDED_OPACITY, dim);
        // Meters XOR the reason they're missing — the reason joins the meta
        // line, so a failed probe never changes the row's height.
        // A pool row's usage column gives way on a narrow pane (its bars
        // shorten) before the identity column drops below readable.
        let usage_column = |column: gpui::Div| {
            column
                .w(px(USAGE_COLUMN_WIDTH))
                .map(|el| match pool {
                    Some(_) => el.flex_shrink(1.0).min_w(px(POOL_USAGE_MIN_WIDTH)),
                    None => el.flex_none(),
                })
                .when(content_opacity < 1.0, |el| el.opacity(content_opacity))
        };
        let usage: AnyElement = if account.usage_windows.is_empty() {
            if usage_note {
                meta.extend(self.render_usage_missing(account, theme));
            }
            usage_column(div()).into_any_element()
        } else {
            let shown = meter_windows(&account.usage_windows);
            let tooltip = usage_tooltip(&account.usage_windows, &shown, now);
            usage_column(div())
                .id(SharedString::from(format!("{row_key}-usage")))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .tooltip(widgets::text_tooltip(tooltip))
                .children(render_usage_meters(&account.usage_windows, theme))
                .into_any_element()
        };

        // Avatar: the account's initial on a soft disc; the marked account
        // (the live login, or the pool's next) gets an accent ring with a
        // hairline of air between ring and disc. In a pooled provider the
        // mark crossfades between rows rather than cutting.
        let marked = match pool {
            Some(pool) if pool.on => pool.next,
            _ => account.active,
        };
        let ring = match pool {
            Some(_) => motion::state_t(
                &format!("account-ring-{}", account.id),
                marked,
                motion::GLYPH_STATE,
                reduced,
            ),
            None => f32::from(u8::from(marked)),
        };
        let initial: SharedString = email
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".into())
            .into();
        let avatar = div()
            .flex_none()
            .size(px(32.0))
            .rounded_full()
            .border(px(1.5))
            .border_color(motion::mix(gpui::transparent_black(), theme.accent, ring))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .size(px(25.0))
                    .rounded_full()
                    .bg(theme.wash(0.09))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(crate::typography::ui_rems(11.5))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(motion::mix(theme.text_muted, theme.text, ring))
                    .child(initial),
            );

        let menu_open = !lifted && self.row_menu.get() == Some(&account.id);
        let menu_account = account.clone();
        let menu_trigger_id = account.id.clone();
        let mut more = widgets::action_button(theme, widgets::ActionTone::Quiet)
            .id(SharedString::from(format!("{row_key}-more")))
            .w(px(28.0))
            .px_0()
            .justify_center()
            .when(menu_open, |el| el.bg(theme.glass_hover()))
            .tab_index(0)
            .role(gpui::Role::Button)
            .aria_label(format!("Actions for {email}"))
            .tooltip(widgets::text_tooltip("Account actions"))
            .aria_expanded(menu_open)
            .focus_visible(|s| s.border_2().border_color(theme.accent))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |page, _, _, _| {
                    page.row_menu
                        .note_trigger_press_matching(|open| *open == menu_trigger_id);
                }),
            )
            .on_click(cx.listener(move |page, _, _, cx| {
                cx.stop_propagation();
                if page.row_menu.take_press_was_open() {
                    page.close_row_menu(cx);
                } else {
                    page.row_menu.open(menu_account.id.clone());
                    cx.notify();
                }
            }))
            .child(
                crate::icons::icon(crate::icons::MORE_HORIZONTAL)
                    .size(px(16.0))
                    .text_color(theme.text_muted),
            );
        if menu_open {
            let popup = Theme::of(cx).for_popup();
            let switch_from_menu = account.clone();
            let forget_account = account.clone();
            let moves: Vec<(PoolMove, usize)> = match pool {
                Some(pool) if pool.on && pool.reorderable => {
                    [PoolMove::Top, PoolMove::Up, PoolMove::Down]
                        .into_iter()
                        .filter_map(|mv| mv.target(pool.position, pool.count).map(|to| (mv, to)))
                        // "Move to top" from the second row is just "Move up".
                        .filter(|(mv, to)| !(*mv == PoolMove::Top && *to + 1 == pool.position))
                        .collect()
                }
                _ => Vec::new(),
            };
            let needs_sign_in = pool_on
                && pool
                    .and_then(|pool| pool.status.as_ref())
                    .is_some_and(|status| status.status == AccountPoolStatus::NeedsSignIn)
                && !is_api_key_slot(account);
            let menu = popover::popover_card(&popup)
                .w(px(if pool_on { 224.0 } else { 208.0 }))
                .flex()
                .flex_col()
                .on_mouse_down_out(cx.listener(|page, _, _, cx| page.close_row_menu(cx)))
                .children(moves.into_iter().map(|(mv, _)| {
                    let id = account.id.clone();
                    let key = format!("account-menu-{mv:?}-{}", account.id);
                    popover::menu_row(&popup, false, key.clone())
                        .id(SharedString::from(key))
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.close_row_menu(cx);
                            page.move_pool_row(harness, &id, mv, cx);
                        }))
                        .child(SharedString::from(mv.label()))
                }))
                .when(pool_on, |menu| {
                    menu.child(
                        popover::menu_row(&popup, false, format!("account-menu-refresh-{ix}"))
                            .id(("account-menu-refresh", ix))
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.close_row_menu(cx);
                                page.load(force_usage_for(LoadTrigger::Refresh), cx);
                            }))
                            .child(SharedString::from("Refresh usage")),
                    )
                })
                .when(needs_sign_in, |menu| {
                    menu.child(
                        popover::menu_row(&popup, false, format!("account-menu-sign-in-{ix}"))
                            .id(("account-menu-sign-in", ix))
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.close_row_menu(cx);
                                page.start_login(harness, None, cx);
                            }))
                            .child(SharedString::from("Sign in again")),
                    )
                })
                .when(can_switch, |menu| {
                    menu.child(
                        popover::menu_row(&popup, false, format!("account-menu-switch-{ix}"))
                            .id(("account-menu-switch", ix))
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.close_row_menu(cx);
                                page.account_action(
                                    methods::ACTIVATE_AGENT_ACCOUNT,
                                    &switch_from_menu,
                                    cx,
                                );
                            }))
                            .child(SharedString::from(if pool_on {
                                "Switch live login to this account"
                            } else {
                                "Switch to this account"
                            })),
                    )
                })
                .when(account.switchable, |menu| {
                    menu.child(
                        popover::menu_row(&popup, false, format!("account-menu-remove-{ix}"))
                            .id(("account-menu-remove", ix))
                            .text_color(popup.danger_muted)
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.close_row_menu(cx);
                                page.account_action(
                                    methods::FORGET_AGENT_ACCOUNT,
                                    &forget_account,
                                    cx,
                                );
                            }))
                            .child(SharedString::from("Remove account")),
                    )
                })
                .into_any_element();
            more = more.relative().child(popover::anchored_menu_below_end(
                format!("account-menu-{ix}"),
                menu,
                self.row_menu.closing_since(),
            ));
        }
        // Accounts with nothing to offer (an unreadable live login,
        // Antigravity's single login) keep the slot for alignment only. A
        // pool member always has its order moves.
        let has_actions = account.switchable || pool_on;

        // Pool chrome: the order slot and the participation switch. Each
        // grows from zero width while it fades in (and cancels its share of
        // the row gap), so the avatar and meters glide rather than jump.
        let chrome = pool.map_or(0.0, |pool| pool.chrome);
        let order_slot = pool.filter(|pool| pool.chrome > 0.0).map(|pool| {
            let grip = if pool.grabbed {
                1.0
            } else if pool.on && pool.reorderable && !lifted {
                motion::hover_t(&hover_key)
            } else {
                0.0
            };
            let digit: SharedString = if pool.excluded || !pool.reorderable {
                "–".into()
            } else {
                (pool.position + 1).to_string().into()
            };
            let label = format!(
                "Reorder {email}, position {} of {}",
                pool.position + 1,
                pool.count
            );
            let id = account.id.clone();
            let selector = format!("pool-handle-{}", account.id);
            div()
                .id(SharedString::from(format!("pool-handle-{}", account.id)))
                .debug_selector(move || selector)
                .flex_none()
                .relative()
                .h(px(32.0))
                .w(px(POOL_ORDER_WIDTH * chrome))
                .mr(px(-ROW_GAP * (1.0 - chrome)))
                .opacity(chrome)
                .rounded(px(6.0))
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .opacity(1.0 - grip)
                        .child(motion::fade_quick(
                            SharedString::from(format!("pool-digit-{}-{digit}", account.id)),
                            div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(crate::typography::ui_rems(11.5))
                                .text_color(theme.text_muted)
                                .child(digit),
                        )),
                )
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(grip)
                        .child(
                            crate::icons::icon(crate::icons::DRAG_HANDLE)
                                .size(px(14.0))
                                .text_color(if pool.grabbed {
                                    theme.accent
                                } else {
                                    theme.text_muted
                                }),
                        ),
                )
                .when(pool.on && pool.reorderable && !lifted, |slot| {
                    slot.cursor(gpui::CursorStyle::OpenHand)
                        .tab_index(0)
                        .role(gpui::Role::Button)
                        .aria_label(label)
                        .tooltip(widgets::text_tooltip(
                            "Drag to reorder, or press Space and use ↑/↓",
                        ))
                        .focus_visible(|s| s.border_2().border_color(theme.accent))
                        .on_key_down(cx.listener(move |page, event: &gpui::KeyDownEvent, _, cx| {
                            if !event.is_held
                                && page.pool_key(harness, &id, event.keystroke.key.as_str(), cx)
                            {
                                cx.stop_propagation();
                            }
                        }))
                })
        });
        let member_switch = pool.filter(|pool| pool.chrome > 0.0).map(|pool| {
            let poolable = pool.status.as_ref().is_none_or(|status| status.poolable);
            let included = !pool.excluded && poolable;
            let interactive = pool.on && poolable && !lifted;
            let reason = pool
                .status
                .as_ref()
                .and_then(|status| status.unpoolable_reason.clone())
                .unwrap_or_else(|| "This login can't join the pool".into());
            let id = account.id.clone();
            let key_id = account.id.clone();
            widgets::toggle_switch(theme, included, format!("pool-member-{}", account.id))
                .id(SharedString::from(format!("pool-member-{}", account.id)))
                .w(px(widgets::SWITCH_WIDTH * chrome))
                .ml(px(-ROW_GAP * (1.0 - chrome)))
                .overflow_hidden()
                .opacity(chrome * if poolable { 1.0 } else { 0.55 })
                .when(!poolable, |el| el.tooltip(widgets::text_tooltip(reason)))
                .when(interactive, |el| {
                    el.cursor_pointer()
                        .tab_index(0)
                        .role(gpui::Role::Switch)
                        .aria_label(format!("Use {email} in the account pool"))
                        .aria_toggled(if included {
                            gpui::Toggled::True
                        } else {
                            gpui::Toggled::False
                        })
                        .focus_visible(|s| s.border_2().border_color(theme.accent).opacity(1.0))
                        .on_click(cx.listener(move |page, _, _, cx| {
                            cx.stop_propagation();
                            page.set_pool_member(harness, &id, !included, cx);
                        }))
                        .on_key_down(cx.listener(move |page, event: &gpui::KeyDownEvent, _, cx| {
                            if !event.is_held
                                && matches!(event.keystroke.key.as_str(), "enter" | "space")
                            {
                                page.set_pool_member(harness, &key_id, !included, cx);
                                cx.stop_propagation();
                            }
                        }))
                })
        });

        let body = div()
            .id(row_key.clone())
            .mx(px(-10.0))
            .px(px(10.0))
            .py(px(10.0))
            .min_h(px(56.0))
            .rounded(px(10.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(ROW_GAP))
            .when(click_switches, |row| {
                row.cursor_pointer()
                    .bg(motion::hover_blend(
                        &hover_key,
                        theme.wash(0.0),
                        theme.wash(0.04),
                    ))
                    .on_hover(motion::hover_listener(hover_key.clone()))
                    .tooltip(widgets::text_tooltip("Switch to this account"))
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.account_action(methods::ACTIVATE_AGENT_ACCOUNT, &switch_account, cx);
                    }))
            })
            // A pool row answers the pointer with the same quiet wash, but
            // a click no longer switches the live login: it's a list to
            // order, so the whole row drags (the queue's full-row hitbox).
            .when(
                pool_on && pool.is_some_and(|pool| pool.reorderable) && !lifted,
                |row| {
                    row.bg(motion::hover_blend(
                        &hover_key,
                        theme.wash(0.0),
                        theme.wash(0.04),
                    ))
                    .on_hover(motion::hover_listener(hover_key.clone()))
                    .on_drag(
                        PoolDragPayload { harness, from: ix },
                        |_payload, _point, _, cx| {
                            cx.stop_propagation();
                            cx.new(|_| PoolGhost)
                        },
                    )
                },
            )
            .when(lifted, |row| row.cursor(gpui::CursorStyle::ClosedHand))
            .children(order_slot)
            .child(avatar.when(content_opacity < 1.0, |el| el.opacity(content_opacity)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .when(pool.is_some(), |el| el.min_w(px(POOL_IDENTITY_MIN_WIDTH)))
                    .when(content_opacity < 1.0, |el| el.opacity(content_opacity))
                    .child(widgets::row_title(theme, email.clone()).truncate())
                    .when(!meta.is_empty(), |el| {
                        el.child(match pool {
                            Some(_) => pool_meta_line(theme, meta),
                            None => widgets::meta_line(theme, meta),
                        })
                    }),
            )
            .child(usage)
            .children(member_switch)
            .child(
                div()
                    .w(px(28.0))
                    .flex_none()
                    .when(has_actions, |slot| slot.child(more)),
            );
        let offset = pool.map_or(0.0, |pool| pool.offset);
        div()
            .debug_selector(move || format!("{row_key}-slot"))
            .when(!self.embedded, |el| el.mx(px(16.0)))
            .when(!first, |el| {
                el.border_t_1().border_color(widgets::row_divider(theme))
            })
            .py(px(2.0))
            .when(offset != 0.0, |el| el.relative().top(px(offset)))
            // The dragged row's own slot stays open; its lifted copy moves.
            .when(pool.is_some_and(|pool| pool.hidden), |el| el.opacity(0.0))
            .child(body)
            .into_any_element()
    }

    /// A ghost of [`Self::render_account_row`] for an expanded provider:
    /// same geometry, so loaded rows land without a layout jump.
    fn render_embedded_skeleton(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        use crate::motion;
        let delta = motion::pulse_delta(&motion::ZERON_PULSE, cx.entity_id(), cx);
        let ghost = |w: f32, h: f32| {
            div()
                .flex_none()
                .w(px(w))
                .h(px(h))
                .rounded(px(4.0))
                .bg(theme.wash(0.07))
        };
        let row = |first: bool, dim: bool| {
            div()
                .min_h(px(56.0))
                .py(px(10.0))
                .when(!first, |el| {
                    el.border_t_1().border_color(widgets::row_divider(theme))
                })
                .when(dim, |el| el.opacity(0.6))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(16.0))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(ghost(176.0, 11.0))
                        .child(ghost(48.0, 9.0)),
                )
                .child(
                    div()
                        .w(px(USAGE_COLUMN_WIDTH))
                        .flex_none()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .children((0..2).map(|_| {
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(ghost(USAGE_LABEL_WIDTH - 12.0, 8.0))
                                .child(
                                    div()
                                        .w(px(USAGE_BAR_WIDTH))
                                        .h(px(4.0))
                                        .rounded_full()
                                        .bg(theme.wash(0.06)),
                                )
                        })),
                )
                .child(div().w(px(ACCOUNT_ACTION_WIDTH)).flex_none())
        };
        div()
            .child(row(true, false))
            .child(row(false, true))
            .opacity(0.55 + 0.35 * motion::pulse_wave(delta))
            .into_any_element()
    }

    /// The sign-in dialog — one layout for every provider: what to do in the
    /// browser, a way back to the page, then the progress line (or, on
    /// failure, the reason in the same place), with Cancel — or Close and
    /// Retry — on the right. Claude's paste-code fallback swaps the progress
    /// line for the code field.
    fn render_login_dialog(
        &mut self,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let theme = Theme::of(cx).for_popup();
        let red_text = theme.danger_muted.opacity(0.9); // red-300
        let login = self.login.as_ref()?;
        let title = login.title();
        let failed = matches!(login.step, LoginStep::Failed { .. });
        let action = |id: &'static str, tone: widgets::ActionTone, label: &'static str| {
            widgets::text_action(&theme, tone, label)
                .id(id)
                .tab_index(0)
                .role(gpui::Role::Button)
                .focus_visible(|s| s.border_2().border_color(theme.accent).opacity(1.0))
        };
        let copy = match login.step {
            LoginStep::PasteCode { .. } => {
                "Your browser opened Claude's sign-in page. Approve access, then paste the \
                 code Anthropic shows you below. Your current login is untouched until you \
                 switch."
            }
            _ => login_copy(login.harness, login.provider),
        };
        // "Reopen the sign-in page" (zeron: `text-[12px]
        // text-muted-foreground/60 hover:underline`), once there is a page.
        let reopen = login.url.clone().filter(|_| !failed).map(|url| {
            div()
                .id("login-open-url")
                .mt(px(6.0))
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text_muted)
                .truncate()
                .cursor_pointer()
                .hover(|s| s.text_color(theme.text))
                .tab_index(0)
                .role(gpui::Role::Button)
                .focus_visible(|s| s.border_2().border_color(theme.accent).opacity(1.0))
                .on_click(cx.listener(move |_, _, _, cx| open_login_url(&url, cx)))
                .child(SharedString::from("Reopen the sign-in page"))
        });
        let error_line = |message: SharedString| {
            div()
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(red_text)
                .child(message)
        };
        let middle: AnyElement = match &login.step {
            LoginStep::Browser { .. } => div()
                .mt(px(16.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(crate::loaders::gradient_spinner(
                    "login-poll",
                    &theme,
                    3.0,
                    cx.entity_id(),
                    cx,
                ))
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(12.5))
                        .text_color(theme.text_muted)
                        .child(login.status()),
                )
                .into_any_element(),
            LoginStep::PasteCode { error, .. } => div()
                .mt(px(12.0))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    popover::dialog_field(self.code_input.clone().into_any_element())
                        .font_family(theme.font_mono.clone())
                        .text_size(crate::typography::ui_rems(13.0)),
                )
                .when_some(error.clone(), |el, message| el.child(error_line(message)))
                .into_any_element(),
            LoginStep::Failed { message } => div()
                .mt(px(16.0))
                .child(error_line(message.clone()))
                .into_any_element(),
        };
        let buttons = div()
            .mt(px(16.0))
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(8.0));
        let buttons = match &login.step {
            LoginStep::Browser { .. } => buttons.child(
                action("login-cancel", widgets::ActionTone::Quiet, "Cancel")
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_login(cx))),
            ),
            LoginStep::PasteCode { submitting, .. } => {
                let submitting = *submitting;
                buttons
                    .child(
                        action("login-cancel", widgets::ActionTone::Quiet, "Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_login(cx))),
                    )
                    .child(
                        action(
                            "login-submit-code",
                            widgets::ActionTone::Solid,
                            if submitting {
                                "Verifying…"
                            } else {
                                "Add account"
                            },
                        )
                        .when(submitting, |el| el.opacity(0.5))
                        .on_click(cx.listener(|this, _, _, cx| this.submit_code(cx))),
                    )
            }
            LoginStep::Failed { .. } => buttons
                .child(
                    action("login-cancel", widgets::ActionTone::Quiet, "Close")
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_login(cx))),
                )
                .child(
                    action("login-retry", widgets::ActionTone::Solid, "Retry")
                        .on_click(cx.listener(|this, _, _, cx| this.retry_login(cx))),
                ),
        };
        let card = popover::dialog_card(&theme)
            .id("add-account-card")
            .role(gpui::Role::Dialog)
            .aria_label(title.clone())
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.cancel_login(cx);
                    cx.stop_propagation();
                }
            }))
            .child(popover::dialog_title(&theme, &title))
            .child(div().mt(px(8.0)).child(popover::dialog_body(&theme, copy)))
            .children(reopen)
            .child(middle)
            .child(buttons)
            .into_any_element();
        Some(popover::modal("add-account-dialog", viewport, card))
    }

    /// A ghost account row (zeron settings.agents.tsx `SkeletonRow`): avatar,
    /// email line, two usage-meter ghosts, a badge — same geometry as the real
    /// row so loaded data lands without a layout jump. `dim` fades row two.
    fn render_skeleton_row(
        &self,
        _id: (&'static str, usize),
        dim: bool,
        first: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::motion;
        let delta = motion::pulse_delta(&motion::ZERON_PULSE, cx.entity_id(), cx);
        let ghost = |w: gpui::Length, h: f32, round_full: bool| {
            div()
                .w(w)
                .h(px(h))
                .flex_none()
                .map(|el| {
                    if round_full {
                        el.rounded_full()
                    } else {
                        el.rounded(px(4.0))
                    }
                })
                .bg(crate::theme::ink(0.05))
        };
        let meters = div()
            .mt(px(8.0))
            .flex()
            .flex_col()
            .gap(px(7.0))
            .children((0..2).map(|_| {
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(ghost(px(48.0).into(), 9.0, false))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(56.0))
                            .max_w(px(230.0))
                            .h(px(5.0))
                            .rounded_full()
                            .bg(crate::theme::ink(0.04)),
                    )
                    .child(ghost(px(64.0).into(), 9.0, false))
            }));
        let inner = div()
            .flex()
            .flex_row()
            .items_stretch()
            .gap(px(12.0))
            .child(
                div()
                    .flex_none()
                    .self_center()
                    .size(px(32.0))
                    .rounded_full()
                    .bg(crate::theme::ink(0.05)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(ghost(px(176.0).into(), 13.0, false).max_w(gpui::relative(0.6)))
                    .child(meters),
            )
            .child(div().flex_none().flex().flex_col().items_end().child(ghost(
                px(64.0).into(),
                21.0,
                true,
            )));
        div()
            .px(px(20.0))
            .py(px(14.0))
            .when(!first, |el| el.border_t_1().border_color(theme.border))
            .when(dim, |el| el.opacity(0.6))
            .child(inner.opacity(0.55 + 0.35 * motion::pulse_wave(delta)))
            .into_any_element()
    }
}

impl popover::ScrollRailHost for AccountsPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl AccountsPage {
    /// A provider's account rows, pool-aware, in `container` — which becomes
    /// the drop target of a pool drag — with the dragged row's lifted copy
    /// painted over the rows sliding beneath it. Settings → Accounts and an
    /// expanded agent share this; only `self.embedded` margins differ.
    fn render_rows(
        &self,
        harness: HarnessId,
        snapshot: &AgentAccountsSnapshot,
        container: gpui::Div,
        theme: &Theme,
        now: DateTime<Utc>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let accounts = self.display_accounts(snapshot, harness);
        let count = if pools_accounts(harness) {
            pool_order(snapshot, harness).len()
        } else {
            accounts.len()
        };
        let reduced = motion::reduced_motion(cx);
        let config = pool_config(snapshot, harness);
        let on = config.is_some_and(|config| config.enabled);
        // Read every frame for a pooled provider (even while off) so the
        // morph sees the flip and the chrome grows rather than pops.
        let chrome = match config {
            Some(_) => motion::state_t(
                &format!("pool-chrome-{harness:?}"),
                on,
                motion::COLLAPSE,
                reduced,
            ),
            None => 0.0,
        };
        let next = pool_next_id(snapshot, harness);
        let drag = self
            .pool_drag
            .as_ref()
            .filter(|drag| on && drag.harness == harness && drag.from < count);
        let view = |ix: usize, account: &AgentAccount, lifted: bool| {
            let config = config?;
            let drag_offset = match drag {
                Some(drag) if !lifted && ix != drag.from => {
                    let (start, target) =
                        pool_drag_offsets(ix, drag.from, drag.prev_over, drag.over);
                    if reduced {
                        target
                    } else {
                        motion::lerp(start, target, slide_progress(drag.moved_at))
                    }
                }
                _ => 0.0,
            };
            Some(PoolRowView {
                on,
                reorderable: config.account_order.contains(&account.id),
                chrome,
                mode: config.routing_mode,
                status: pool_account(snapshot, &account.id).cloned(),
                excluded: config.disabled_account_ids.contains(&account.id),
                next: next == Some(account.id.as_str()),
                position: ix,
                count,
                grabbed: self
                    .pool_grab
                    .as_ref()
                    .is_some_and(|grab| grab.harness == harness && grab.id == account.id),
                lifted,
                offset: if lifted {
                    0.0
                } else {
                    drag_offset + self.slide_offset(&account.id, reduced)
                },
                hidden: !lifted && drag.is_some_and(|drag| drag.from == ix),
            })
        };
        let rows: Vec<AnyElement> = accounts
            .iter()
            .enumerate()
            .map(|(ix, account)| {
                let pool = view(ix, account, false);
                self.render_account_row(account, ix, ix == 0, pool.as_ref(), theme, now, cx)
            })
            .collect();
        // The dragged row, lifted: it follows the pointer like a dragged
        // queue row, frosted and shadowed a level above the list.
        let lifted = drag.and_then(|drag| {
            let account = accounts.get(drag.from)?;
            let pool = view(drag.from, account, true);
            let row =
                self.render_account_row(account, drag.from, true, pool.as_ref(), theme, now, cx);
            let card = div()
                .rounded(px(10.0))
                .bg(popover::surface_bg(theme))
                .shadow_md()
                .child(
                    div()
                        .rounded(px(10.0))
                        .bg(crate::theme::wash(0.06))
                        .child(row),
                );
            Some(
                div()
                    .id(SharedString::from(format!("pool-lifted-row-{harness:?}")))
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(pool_lifted_top(drag.pointer, drag.grab, count)))
                    .child(crate::frost::frosted(10.0, crate::frost::MENU_BLUR, card)),
            )
        });
        let container = container.relative().children(rows).children(lifted);
        if !on {
            return container;
        }
        container
            .on_drag_move::<PoolDragPayload>(cx.listener(
                move |page, event: &gpui::DragMoveEvent<PoolDragPayload>, _, cx| {
                    let payload = event.drag(cx);
                    if payload.harness != harness {
                        return;
                    }
                    let from = payload.from;
                    let rel_y = f32::from(event.event.position.y) - f32::from(event.bounds.top());
                    let over = crate::terminal::panel::drop_index(rel_y, POOL_ROW_SLOT, count);
                    page.update_pool_drag_over(harness, from, over, rel_y, cx);
                },
            ))
            .on_drop::<PoolDragPayload>(cx.listener(
                move |page, payload: &PoolDragPayload, _, cx| {
                    if payload.harness == harness {
                        page.drop_pool_row(harness, payload.from, cx);
                    } else {
                        page.cancel_pool_drag(cx);
                    }
                },
            ))
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|page, _, _, cx| page.cancel_pool_drag(cx)),
            )
    }

    /// The pool controls above a pooled provider's rows: settings rows in
    /// the "Update policy" anatomy (title + meta left, control right). The
    /// switch row always shows; Routing and Move-on-at reveal with the
    /// agent details' `menu_in` while the pool is on and leave with
    /// `menu_out`. A blocking notice from the engine follows as a warning
    /// strip. `None` for providers that don't pool.
    fn render_pool_block(
        &self,
        harness: HarnessId,
        snapshot: &AgentAccountsSnapshot,
        theme: &Theme,
        now: DateTime<Utc>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let config = pool_config(snapshot, harness)?.clone();
        let on = config.enabled;
        let has_accounts = !provider_accounts(snapshot, harness).is_empty();
        let interactive = has_accounts || on;
        let name = provider_name(harness);
        let summary = if on {
            pool_summary(snapshot, harness, now)
        } else {
            format!(
                "Spread {name} sessions across your saved accounts. Off: sessions use the live \
                 login."
            )
        };
        let switch = widgets::toggle_switch(theme, on, format!("pool-{harness:?}"))
            .id(SharedString::from(format!("pool-switch-{harness:?}")))
            .when(!interactive, |el| {
                el.opacity(0.55)
                    .tooltip(widgets::text_tooltip("Add an account first"))
            })
            .when(interactive, |el| {
                el.cursor_pointer()
                    .tab_index(0)
                    .role(gpui::Role::Switch)
                    .aria_label(format!("{name} account pool"))
                    .aria_toggled(if on {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .focus_visible(|s| s.border_2().border_color(theme.accent).opacity(1.0))
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.set_pool_enabled(harness, !on, cx);
                    }))
                    .on_key_down(cx.listener(move |page, event: &gpui::KeyDownEvent, _, cx| {
                        if !event.is_held
                            && matches!(event.keystroke.key.as_str(), "enter" | "space")
                        {
                            page.set_pool_enabled(harness, !on, cx);
                            cx.stop_propagation();
                        }
                    }))
            });
        let block = div().flex().flex_col().child(pool_setting_row(
            theme,
            true,
            "Account pool",
            summary,
            switch,
        ));

        let closing = self
            .pool_closing
            .get(&harness)
            .filter(|_| !on)
            .map(|since| {
                let total = motion::MENU_OUT.total().as_secs_f32();
                motion::MENU_OUT.progress(since.elapsed().as_secs_f32() / total.max(f32::EPSILON))
            })
            .filter(|t| *t < 1.0);
        let controls = (on || closing.is_some()).then(|| {
            let closed = widgets::SelectState::default();
            let mode_ix = POOL_MODES
                .iter()
                .position(|(mode, ..)| *mode == config.routing_mode)
                .unwrap_or(0);
            let routing = widgets::select(
                format!("pool-routing-{harness:?}"),
                "Routing",
                theme,
                move |page: &mut Self| {
                    page.pool_selects
                        .entry((harness, PoolSelect::Routing))
                        .or_default()
                },
            )
            .options(
                POOL_MODES
                    .iter()
                    .map(|(_, label, _)| widgets::SelectOption::new(*label)),
                mode_ix,
            )
            // Fits the longest label, so switching never resizes the trigger.
            .width(176.0)
            .on_select(move |page, ix, _, cx| {
                if let Some((mode, ..)) = POOL_MODES.get(ix) {
                    page.set_pool_mode(harness, *mode, cx);
                }
            })
            .render(
                self.pool_selects
                    .get(&(harness, PoolSelect::Routing))
                    .unwrap_or(&closed),
                cx,
            );
            let (thresholds, threshold_ix) = threshold_options(config.switch_threshold);
            let threshold_values = thresholds.clone();
            let threshold = widgets::select(
                format!("pool-threshold-{harness:?}"),
                "Move on at",
                theme,
                move |page: &mut Self| {
                    page.pool_selects
                        .entry((harness, PoolSelect::Threshold))
                        .or_default()
                },
            )
            .options(
                thresholds
                    .iter()
                    .map(|value| widgets::SelectOption::new(threshold_label(*value))),
                threshold_ix,
            )
            .width(96.0)
            .on_select(move |page, ix, _, cx| {
                if let Some(value) = threshold_values.get(ix) {
                    page.set_pool_threshold(harness, *value, cx);
                }
            })
            .render(
                self.pool_selects
                    .get(&(harness, PoolSelect::Threshold))
                    .unwrap_or(&closed),
                cx,
            );
            let content = div()
                .flex()
                .flex_col()
                .child(pool_setting_row(
                    theme,
                    false,
                    "Routing",
                    POOL_MODES[mode_ix].2.to_string(),
                    routing,
                ))
                .child(pool_setting_row(
                    theme,
                    false,
                    "Move on at",
                    "An account is skipped once any of its limits reaches this much use."
                        .to_string(),
                    threshold,
                ));
            let id = SharedString::from(format!("pool-controls-{harness:?}"));
            match closing {
                Some(t) => motion::menu_out(id, t, content).into_any_element(),
                None => motion::menu_in(id, content).into_any_element(),
            }
        });
        let notice = pool_provider(snapshot, harness)
            .and_then(|provider| provider.notice.clone())
            .filter(|_| on)
            .map(|notice| {
                motion::fade_quick(
                    SharedString::from(format!("pool-notice-{harness:?}-{notice}")),
                    widgets::warning_strip(theme, notice).mt(px(4.0)),
                )
            });
        Some(block.children(controls).children(notice).into_any_element())
    }

    /// The quiet note under an enabled pool's rows.
    fn render_pool_footnote(
        &self,
        harness: HarnessId,
        snapshot: &AgentAccountsSnapshot,
        theme: &Theme,
    ) -> Option<AnyElement> {
        let config = pool_config(snapshot, harness).filter(|config| config.enabled)?;
        Some(
            motion::fade_quick(
                SharedString::from(format!("pool-footnote-{harness:?}")),
                div()
                    .mt(px(8.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(pool_footnote(
                        harness,
                        config.routing_mode,
                    ))),
            )
            .into_any_element(),
        )
    }

    fn render_embedded_provider(
        &self,
        harness: HarnessId,
        theme: &Theme,
        now: DateTime<Utc>,
        dialog: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let refreshing = self.refreshing || matches!(self.snapshot, Loadable::Loading);
        let content: AnyElement =
            match &self.snapshot {
                Loadable::Idle | Loadable::Loading => self.render_embedded_skeleton(theme, cx),
                Loadable::Error(message) => widgets::error_strip(theme, message.clone())
                    .mt(px(4.0))
                    .id("accounts-inline-retry")
                    .role(gpui::Role::Button)
                    .aria_label("Retry loading accounts")
                    .tab_index(0)
                    .cursor_pointer()
                    .focus_visible(|s| s.border_2().border_color(theme.accent))
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.load(force_usage_for(LoadTrigger::Retry), cx)
                    }))
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex_none()
                            .text_color(theme.text_muted)
                            .child(SharedString::from("Retry")),
                    )
                    .into_any_element(),
                Loadable::Ready(snapshot) => {
                    let empty = provider_accounts(snapshot, harness).is_empty();
                    let api_key = self.render_add_api_key(harness, snapshot, theme, false, cx);
                    // Adding is the list's last row, not a header button —
                    // it sits where the next account will appear. A
                    // one-login agent that is connected has nothing to add.
                    let add_row = (empty || !keeps_one_login(harness)).then(|| {
                        div()
                            .py(px(8.0))
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .gap(px(4.0))
                            .when(!empty, |el| {
                                el.border_t_1().border_color(widgets::row_divider(theme))
                            })
                            .children(login_options(harness).into_iter().enumerate().map(
                                |(ix, option)| {
                                    let label = add_option_label(harness, option, empty);
                                    widgets::action_button(theme, widgets::ActionTone::Quiet)
                                        .id(("accounts-add", ix))
                                        .when(ix == 0, |el| el.ml(px(-10.0)))
                                        .role(gpui::Role::Button)
                                        .aria_label(label.clone())
                                        .tab_index(0)
                                        .focus_visible(|s| s.border_2().border_color(theme.accent))
                                        .on_click(cx.listener(move |page, _, _, cx| {
                                            page.start_login(harness, option.provider, cx)
                                        }))
                                        .child(
                                            crate::icons::icon(crate::icons::PLUS)
                                                .size(px(14.0))
                                                .text_color(theme.text_muted),
                                        )
                                        .child(SharedString::from(label))
                                },
                            ))
                            .children(api_key)
                    });
                    div()
                        .flex()
                        .flex_col()
                        .children(
                            self.render_pool_block(harness, snapshot, theme, now, cx)
                                .map(|block| div().mb(px(4.0)).child(block)),
                        )
                        .child(self.render_rows(
                            harness,
                            snapshot,
                            div().flex().flex_col(),
                            theme,
                            now,
                            cx,
                        ))
                        .children(add_row)
                        .children(self.render_pool_footnote(harness, snapshot, theme))
                        .into_any_element()
                }
            };
        div()
            .id("accounts-embedded")
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .child(widgets::details_label(theme, "Accounts"))
                    .children(
                        self.render_accounts_status(cx)
                            .map(|status| div().ml(px(6.0)).flex_none().child(status)),
                    )
                    .child(div().flex_1())
                    .child(
                        widgets::action_button(theme, widgets::ActionTone::Quiet)
                            .id("accounts-refresh")
                            // Optically align the glyph with the badges'
                            // right edge below.
                            .mr(px(-8.0))
                            .w(px(32.0))
                            .px_0()
                            .justify_center()
                            .when(refreshing, |el| el.opacity(0.5))
                            .role(gpui::Role::Button)
                            .aria_label("Refresh accounts")
                            .tooltip(widgets::text_tooltip("Refresh usage"))
                            .tab_index(0)
                            .focus_visible(|s| s.border_2().border_color(theme.accent))
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.load(force_usage_for(LoadTrigger::Refresh), cx)
                            }))
                            .child(
                                crate::icons::icon(crate::icons::REFRESH)
                                    .size(px(14.0))
                                    .text_color(theme.text_muted),
                            ),
                    ),
            )
            .children(match &self.snapshot {
                Loadable::Ready(snapshot) => snapshot
                    .warnings
                    .iter()
                    .filter(|warning| warning.harness == harness)
                    .map(|warning| {
                        widgets::warning_strip(theme, warning.message.clone()).mt(px(4.0))
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .when_some(provider_note(harness), |el, note| {
                el.child(
                    div()
                        .mt(px(4.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(note)),
                )
            })
            .child(content)
            .when_some(dialog, |el, dialog| el.child(dialog))
            .into_any_element()
    }
}

impl Render for AccountsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::toast::take_error(&mut self.error, cx);
        let theme = Theme::of(cx).for_settings_surface();
        let now = Utc::now();
        // A drop outside every list ends gpui's drag without our `on_drop`:
        // never leave a slot held open.
        if self.pool_drag.is_some() && !cx.has_active_drag() {
            self.pool_drag = None;
        }
        if self.pool_motion_active() {
            window.request_animation_frame();
        }
        let dialog = match self.render_login_dialog(window.viewport_size(), cx) {
            Some(dialog) => Some(dialog),
            None => self.render_api_key_dialog(window.viewport_size(), cx),
        };
        if let Some(harness) = self.embedded_harness {
            return self.render_embedded_provider(harness, &theme, now, dialog, cx);
        }
        let refreshing = self.refreshing || matches!(self.snapshot, Loadable::Loading);
        let account_count = self
            .snapshot
            .ready()
            .map(|s| s.accounts.len())
            .filter(|&n| n > 0);

        let provider_icon = crate::pickers::harness_brand_icon;
        // Brand mark inside a 24px centered box (zeron: `grid size-6
        // place-items-center [&_svg]:size-4`).
        let provider_mark = |harness: HarnessId, theme: &Theme| {
            let (mark, tint) = provider_icon(harness);
            div()
                .flex_none()
                .size(px(24.0))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    crate::icons::icon(mark)
                        .size(px(16.0))
                        .text_color(tint.unwrap_or(theme.text_muted)),
                )
        };

        // One section per provider (zeron settings.agents.tsx `ProviderSection`):
        // brand header + Add account, then the account rows card.
        let sections: Vec<AnyElement> = match &self.snapshot {
            Loadable::Idle | Loadable::Loading => PROVIDERS
                .into_iter()
                .map(|(harness, name, _cli)| {
                    let skeleton_id = match harness {
                        HarnessId::Codex => "accounts-skeleton-codex",
                        HarnessId::Cursor => "accounts-skeleton-cursor",
                        HarnessId::Antigravity => "accounts-skeleton-antigravity",
                        HarnessId::Grok => "accounts-skeleton-grok",
                        HarnessId::Devin => "accounts-skeleton-devin",
                        HarnessId::Opencode => "accounts-skeleton-opencode",
                        HarnessId::Pi => "accounts-skeleton-pi",
                        HarnessId::Hermes => "accounts-skeleton-hermes",
                        _ => "accounts-skeleton-claude",
                    };
                    div()
                        .mt(px(24.0))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(provider_mark(harness, &theme))
                                .child(
                                    div()
                                        .text_size(crate::typography::ui_rems(14.0))
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(theme.text)
                                        .child(SharedString::from(name)),
                                ),
                        )
                        .child(
                            // Ghost rows shaped like real ones (row two dimmed)
                            // so the card keeps its size while data develops.
                            widgets::section_card(&theme)
                                .mt(px(8.0))
                                .child(self.render_skeleton_row(
                                    (skeleton_id, 0),
                                    false,
                                    true,
                                    &theme,
                                    cx,
                                ))
                                .child(self.render_skeleton_row(
                                    (skeleton_id, 1),
                                    true,
                                    false,
                                    &theme,
                                    cx,
                                )),
                        )
                        .into_any_element()
                })
                .collect(),
            Loadable::Error(message) => {
                let message = message.clone();
                vec![
                    widgets::error_strip(&theme, message)
                        .id("accounts-load-error")
                        .cursor_pointer()
                        .tab_index(0)
                        .role(gpui::Role::Button)
                        .focus_visible(|s| s.border_2().border_color(theme.accent).opacity(1.0))
                        .on_click(cx.listener(|this, _, _, cx| {
                            // Retry IS the visit's first successful list — force usage.
                            this.load(force_usage_for(LoadTrigger::Retry), cx)
                        }))
                        .child(
                            div()
                                .mt(px(4.0))
                                .text_size(crate::typography::ui_rems(11.5))
                                .text_color(theme.text_muted)
                                .child(SharedString::from("Click to retry")),
                        )
                        .into_any_element(),
                ]
            }
            Loadable::Ready(snapshot) => {
                let snapshot = snapshot.clone();
                PROVIDERS
                    .into_iter()
                    .map(|(harness, name, cli)| {
                        let accounts = provider_accounts(&snapshot, harness);
                        // EVERY warning renders its own strip (zeron maps them).
                        let warnings: Vec<String> = snapshot
                            .warnings
                            .iter()
                            .filter(|w| w.harness == harness)
                            .map(|w| w.message.clone())
                            .collect();
                        let add_id: SharedString = format!("add-account-{name}").into();
                        let empty = accounts.is_empty();
                        let api_key = self.render_add_api_key(harness, &snapshot, &theme, true, cx);
                        let can_add = empty || !keeps_one_login(harness);
                        let card = widgets::section_card(&theme).mt(px(8.0));
                        let empty_copy = match harness {
                            // Cursor's app login is SEPARATE from `cursor-agent
                            // login` — pointing at the CLI would send users to a
                            // sign-in that does not light this up. Antigravity
                            // has no CLI login at all.
                            HarnessId::Cursor | HarnessId::Antigravity => format!(
                                "{name} isn't connected on this device — connect it to run \
                                 {name} sessions."
                            ),
                            _ => format!(
                                "No {name} login detected on this device — sign in \
                                 with \u{201C}{cli}\u{201D} or add an account."
                            ),
                        };
                        let card = if empty {
                            card.child(
                                div()
                                    .px(px(16.0))
                                    .py(px(16.0))
                                    .text_size(crate::typography::ui_rems(12.0))
                                    .text_color(theme.text_muted)
                                    .child(SharedString::from(empty_copy)),
                            )
                        } else {
                            self.render_rows(harness, &snapshot, card, &theme, now, cx)
                        };
                        div()
                            .mt(px(16.0))
                            .p(px(16.0))
                            .rounded(px(12.0))
                            .bg(crate::theme::wash(0.035))
                            .border_1()
                            .border_color(theme.border)
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(provider_mark(harness, &theme))
                                    .child(
                                        div()
                                            .text_size(crate::typography::ui_rems(14.0))
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(theme.text)
                                            .child(SharedString::from(name)),
                                    )
                                    .child(div().flex_1())
                                    .when(can_add, |header| {
                                        header.children(
                                            login_options(harness).into_iter().enumerate().map(
                                                |(ix, option)| {
                                                    let label =
                                                        add_option_label(harness, option, empty);
                                                    widgets::ghost_action(&theme)
                                                        .id((add_id.clone(), ix))
                                                        .tab_index(0)
                                                        .role(gpui::Role::Button)
                                                        .focus_visible(|s| {
                                                            s.border_2()
                                                                .border_color(theme.accent)
                                                                .opacity(1.0)
                                                        })
                                                        .on_click(cx.listener(
                                                            move |this, _, _, cx| {
                                                                this.start_login(
                                                                    harness,
                                                                    option.provider,
                                                                    cx,
                                                                );
                                                            },
                                                        ))
                                                        .child(
                                                            crate::icons::icon(
                                                                crate::icons::ADD_CIRCLE,
                                                            )
                                                            .size(px(16.0))
                                                            .text_color(theme.text_muted),
                                                        )
                                                        .child(SharedString::from(label))
                                                },
                                            ),
                                        )
                                    })
                                    .children(api_key),
                            )
                            .children(
                                warnings
                                    .into_iter()
                                    .map(|warning| widgets::warning_strip(&theme, warning)),
                            )
                            .children(provider_note(harness).map(|note| {
                                div()
                                    .mt(px(8.0))
                                    .text_size(crate::typography::ui_rems(12.0))
                                    .text_color(theme.text_muted)
                                    .child(SharedString::from(note))
                            }))
                            .children(
                                self.render_pool_block(harness, &snapshot, &theme, now, cx)
                                    .map(|block| div().mt(px(4.0)).child(block)),
                            )
                            .child(card)
                            .children(self.render_pool_footnote(harness, &snapshot, &theme))
                            .into_any_element()
                    })
                    .collect()
            }
        };

        let scrollbar = popover::rail(self, "accounts-page-scrollbar", &theme, cx);
        div()
            .id("accounts-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(16.0, true, true, div()
                    .id("accounts-page")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll.scroll)
                    .child(
                        widgets::page_column()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .flex_wrap()
                                    .items_center()
                                    .gap(px(10.0))
                                    .child(widgets::page_header(&theme, "Accounts", account_count))
                                    .child(div().flex_1())
                                    .child(
                                        // `text-[12.5px]` + leading 16px Refresh icon,
                                        // dimmed while a refresh is in flight (zeron
                                        // `disabled:opacity-50`).
                                        widgets::ghost_action(&theme)
                                            .id("accounts-refresh")
                                            .flex_none()
                                            .text_size(crate::typography::ui_rems(12.5))
                                            .when(refreshing, |el| el.opacity(0.5))
                                            .tab_index(0)
.role(gpui::Role::Button)
.focus_visible(|s| s.border_2().border_color(theme.accent).opacity(1.0))
.on_click(cx.listener(|this, _, _, cx| {
                                                this.load(
                                                    force_usage_for(LoadTrigger::Refresh),
                                                    cx,
                                                )
                                            }))
                                            .child(
                                                crate::icons::icon(crate::icons::REFRESH)
                                                    .size(px(16.0))
                                                    .text_color(theme.text_muted),
                                            )
                                            .child(SharedString::from("Refresh")),
                                    )
                                    .child(self.render_device_switcher(&theme, cx)),
                            )
                            .children(sections)
                            // Footer note (zeron: `mt-6 text-[12px] leading-relaxed
                            // text-muted-foreground/60`).
                            .child(
                                div()
                                    .mt(px(24.0))
                                    .text_size(crate::typography::ui_rems(12.0))
                                    .line_height(px(19.0))
                                    .text_color(theme.text_muted)
                                    .child(SharedString::from(
                                        "Account changes affect new sessions. On macOS, Claude Code may take about 30 seconds to pick up a switched account.",
                                    )),
                            ),
                    )).fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
            .when_some(dialog, |el, dialog| el.child(dialog))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_web_and_loopback_login_urls_open() {
        assert!(is_openable_login_url(
            "https://claude.com/cai/oauth/authorize?x=1"
        ));
        assert!(is_openable_login_url("http://localhost:1455/auth/callback"));
        assert!(is_openable_login_url("http://127.0.0.1:8080/"));
        assert!(is_openable_login_url("http://[::1]:9000/cb"));
        assert!(!is_openable_login_url("http://evil.example/login"));
        assert!(!is_openable_login_url("http://localhost.evil.example/"));
        assert!(!is_openable_login_url("http://localhost@evil.example/"));
        assert!(!is_openable_login_url("file:///etc/passwd"));
        assert!(!is_openable_login_url("javascript:alert(1)"));
        assert!(!is_openable_login_url("vscode://extension/x"));
    }

    use super::*;
    use chrono::TimeDelta;

    #[test]
    fn first_load_of_a_visit_forces_the_usage_probe() {
        // The engine only probes usage when forced (M5c); without forcing on
        // mount, the first Accounts open always rendered "Usage unavailable".
        assert!(force_usage_for(LoadTrigger::Mount));
        // A retry after a failed load is still the visit's first successful
        // list — same requirement.
        assert!(force_usage_for(LoadTrigger::Retry));
        // Explicit refresh and a just-completed login always re-probe.
        assert!(force_usage_for(LoadTrigger::Refresh));
        assert!(force_usage_for(LoadTrigger::PostLogin));
        // Switch/Forget re-lists ride the still-warm 60s cache.
        assert!(!force_usage_for(LoadTrigger::PostAction));
    }

    #[test]
    fn usage_thresholds_match_zeron() {
        assert_eq!(usage_level(0.0), UsageLevel::Normal);
        assert_eq!(usage_level(0.79), UsageLevel::Normal);
        assert_eq!(usage_level(0.80), UsageLevel::Warn);
        assert_eq!(usage_level(0.94), UsageLevel::Warn);
        assert_eq!(usage_level(0.95), UsageLevel::Critical);
        assert_eq!(usage_level(1.0), UsageLevel::Critical);
    }

    #[test]
    fn usage_colors_map_to_theme_accents() {
        let theme = Theme::dark();
        assert_eq!(usage_color(UsageLevel::Normal, &theme), theme.accent);
        assert_eq!(usage_color(UsageLevel::Warn, &theme), theme.warning);
        assert_eq!(usage_color(UsageLevel::Critical, &theme), theme.danger);
    }

    #[test]
    fn reset_formatting_is_absolute() {
        use chrono::Local;
        let now = Utc::now();
        assert_eq!(format_reset(None, now), None);
        // Within ~22h: a local clock time ("resets 3:45 PM").
        let soon = now + TimeDelta::minutes(125);
        assert_eq!(
            format_reset(Some(soon), now),
            Some(format!(
                "resets {}",
                soon.with_timezone(&Local).format("%-I:%M %p")
            ))
        );
        // Within a week: a short weekday ("resets Mon").
        let later = now + TimeDelta::days(3);
        assert_eq!(
            format_reset(Some(later), now),
            Some(format!(
                "resets {}",
                later.with_timezone(&Local).format("%a")
            ))
        );
        // Beyond a week (Codex free tier resets ~monthly): month + day
        // ("resets Sep 14") — a weekday 4 weeks out carries no information.
        let monthly = now + TimeDelta::days(26);
        assert_eq!(
            format_reset(Some(monthly), now),
            Some(format!(
                "resets {}",
                monthly.with_timezone(&Local).format("%b %-d")
            ))
        );
    }

    #[test]
    fn usage_tooltip_lists_every_window() {
        let now = Utc::now();
        let soon = now + TimeDelta::minutes(125);
        let windows = [
            zeron_proto::AgentUsageWindow {
                label: "Session".into(),
                used_fraction: 0.07,
                resets_at: Some(soon),
            },
            zeron_proto::AgentUsageWindow {
                label: "Week".into(),
                used_fraction: 0.44,
                resets_at: None,
            },
        ];
        assert_eq!(
            usage_tooltip(&windows, &[0, 1], now),
            format!("Session: {} · Week", format_reset(Some(soon), now).unwrap())
        );
        assert_eq!(usage_tooltip(&[], &[], now), "");
    }

    #[test]
    fn every_sign_in_provider_shares_one_accounts_flow() {
        for harness in [
            HarnessId::ClaudeCode,
            HarnessId::Codex,
            HarnessId::Cursor,
            HarnessId::Antigravity,
            HarnessId::Grok,
            HarnessId::Devin,
        ] {
            assert!(signs_in(harness), "{harness:?}");
            assert!(add_account_label(harness, true).starts_with("Connect a "));
            assert_eq!(add_account_label(harness, false), "Add account");
            assert!(login_copy(harness, None).starts_with("Finish signing in to "));
            // One login per agent: one add button, no provider param.
            let options = login_options(harness);
            assert_eq!(options.len(), 1, "{harness:?}");
            assert_eq!(options[0].provider, None);
            assert_eq!(add_option_label(harness, options[0], false), "Add account");
        }
        assert_eq!(
            add_account_label(HarnessId::Antigravity, true),
            "Connect a Antigravity account"
        );
        // Every agent with a login of its own has an Accounts section.
        for harness in [HarnessId::Opencode, HarnessId::Pi, HarnessId::Hermes] {
            assert!(signs_in(harness), "{harness:?}");
            assert!(reports_usage(harness));
            for option in login_options(harness) {
                assert!(option.provider.is_some(), "{harness:?} names its provider");
                assert!(login_copy(harness, option.provider).starts_with("Finish signing in"));
            }
        }
        assert!(!signs_in(HarnessId::Mock));
        // Antigravity has no quota to show and keeps a single login.
        assert!(!reports_usage(HarnessId::Antigravity) && reports_usage(HarnessId::Codex));
        assert!(keeps_one_login(HarnessId::Antigravity) && !keeps_one_login(HarnessId::Cursor));
        // Hermes rotates its own pool; zeron never switches it, and says so.
        assert!(!switches_accounts(HarnessId::Hermes) && switches_accounts(HarnessId::Grok));
        assert!(provider_note(HarnessId::Hermes).is_some_and(|n| n.contains("hermes auth add")));
        assert!(provider_note(HarnessId::Grok).is_none());
    }

    #[test]
    fn per_provider_agents_offer_one_sign_in_per_provider() {
        let opencode = login_options(HarnessId::Opencode);
        let labels: Vec<_> = opencode.iter().map(|o| o.label).collect();
        assert_eq!(labels, ["ChatGPT", "GitHub Copilot"]);
        assert_eq!(
            add_option_label(HarnessId::Opencode, opencode[0], true),
            "Connect ChatGPT"
        );
        assert_eq!(
            add_option_label(HarnessId::Opencode, opencode[1], false),
            "Add GitHub Copilot account"
        );
        let providers: Vec<_> = login_options(HarnessId::Hermes)
            .iter()
            .map(|o| o.provider.unwrap())
            .collect();
        assert_eq!(providers, ["openai-codex", "nous"]);
        let flow = LoginFlow {
            provider: Some("openai-codex"),
            ..waiting(HarnessId::Pi, 1)
        };
        assert_eq!(flow.title(), "Sign in to ChatGPT for Pi");
        assert_eq!(waiting(HarnessId::Grok, 1).title(), "Sign in to Grok");
    }

    #[test]
    fn a_switch_only_moves_the_live_login_within_its_provider_group() {
        let row = |id: &str, harness, provider: Option<&str>, active| AgentAccount {
            id: id.into(),
            harness,
            email: None,
            plan_label: None,
            active,
            usage_windows: vec![],
            usage_fetched_at: None,
            usage_error: None,
            display_name: None,
            organization: None,
            auth_kind: None,
            switchable: true,
            saved_at: None,
            provider: provider.map(str::to_string),
        };
        let mut snapshot = AgentAccountsSnapshot {
            pool: Default::default(),
            accounts: vec![
                row("gpt-a", HarnessId::Opencode, Some("openai"), true),
                row("gpt-b", HarnessId::Opencode, Some("openai"), false),
                row("copilot", HarnessId::Opencode, Some("github-copilot"), true),
                row("grok-a", HarnessId::Grok, None, true),
            ],
            warnings: vec![],
        };
        let target = snapshot.accounts[1].clone();
        mark_switched(&mut snapshot, &target);
        let live: Vec<&str> = snapshot
            .accounts
            .iter()
            .filter(|a| a.active)
            .map(|a| a.id.as_str())
            .collect();
        assert_eq!(live, ["gpt-b", "copilot", "grok-a"]);
    }

    fn page(cx: &mut gpui::TestAppContext) -> gpui::WindowHandle<AccountsPage> {
        use gpui::AppContext;
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            crate::settings::init(Default::default(), dir.path(), cx);
            gpui_base::init(cx);
            cx.set_global(crate::theme::Theme::default());
        });
        cx.add_window(|_, cx| {
            let state = cx.new(|_| crate::state::AppState::new());
            AccountsPage::new_embedded(state, None, HarnessId::Antigravity, cx)
        })
    }

    fn waiting(harness: HarnessId, attempt: u64) -> LoginFlow {
        LoginFlow {
            harness,
            provider: None,
            attempt,
            login_id: Some("login-1".into()),
            url: None,
            step: LoginStep::Browser { message: None },
        }
    }

    fn poll(status: AgentLoginStatus, message: Option<&str>, url: Option<&str>) -> AgentLoginPoll {
        AgentLoginPoll {
            status,
            message: message.map(str::to_string),
            url: url.map(str::to_string),
            callback_port: None,
        }
    }

    #[gpui::test]
    fn a_sign_in_ends_with_the_account_or_an_error_never_a_silent_reset(
        cx: &mut gpui::TestAppContext,
    ) {
        let window = page(cx);
        window
            .update(cx, |page, _, cx| {
                // Starting: the dialog's progress line before any page.
                page.login = Some(waiting(HarnessId::Antigravity, 1));
                let flow = page.login.as_ref().unwrap();
                assert_eq!(flow.title(), "Sign in to Antigravity");
                assert_eq!(flow.status().as_ref(), "Starting sign-in…");
                // The page arrives with a poll: opened once, then waited on.
                let url = "https://accounts.google.com/o/oauth2/auth?x=1";
                assert!(!page.apply_poll(
                    1,
                    Ok(poll(AgentLoginStatus::Pending, None, Some(url))),
                    cx
                ));
                let flow = page.login.as_ref().unwrap();
                assert_eq!(flow.url.as_deref(), Some(url));
                assert_eq!(
                    flow.status().as_ref(),
                    "Waiting for you to finish in the browser…"
                );
                // A reply for an older attempt never touches this one.
                assert!(page.apply_poll(0, Ok(poll(AgentLoginStatus::Done, None, None)), cx));
                assert!(page.login.is_some());
                // An already-signed-in agent finishes at once: the dialog
                // closes and the list reloads to show the account.
                assert!(page.apply_poll(1, Ok(poll(AgentLoginStatus::Done, None, None)), cx));
                assert!(page.login.is_none());

                // A failure stays on screen, in the dialog, with Retry.
                page.login = Some(waiting(HarnessId::Codex, 2));
                assert!(page.apply_poll(
                    2,
                    Ok(poll(
                        AgentLoginStatus::Error,
                        Some("Port 1455 is in use"),
                        None
                    )),
                    cx
                ));
                assert_eq!(
                    page.login.as_ref().unwrap().step,
                    LoginStep::Failed {
                        message: "Port 1455 is in use".into()
                    }
                );
                // So does a lost poll or a start that never got going.
                page.login = Some(waiting(HarnessId::Cursor, 3));
                assert!(page.apply_poll(3, Err("engine gone".into()), cx));
                assert!(matches!(
                    &page.login.as_ref().unwrap().step,
                    LoginStep::Failed { message } if message.contains("engine gone")
                ));
                page.login = Some(waiting(HarnessId::ClaudeCode, 4));
                page.apply_start(4, Err("no network".into()), cx);
                assert!(matches!(
                    &page.login.as_ref().unwrap().step,
                    LoginStep::Failed { message } if message.contains("Couldn't start the sign-in")
                ));
                // Closing a failed dialog clears it.
                page.cancel_login(cx);
                assert!(page.login.is_none());
            })
            .unwrap();
    }

    /// A per-provider agent's section renders its rows (a live login, a
    /// saved one, an unidentified one) and one add button per provider,
    /// and a device-code sign-in's dialog shows the code line.
    #[gpui::test]
    fn per_provider_accounts_render_with_one_add_button_per_provider(
        cx: &mut gpui::TestAppContext,
    ) {
        let window = page(cx);
        let row = |id: &str, email: &str, active, switchable| AgentAccount {
            id: id.into(),
            harness: HarnessId::Opencode,
            email: Some(email.into()),
            plan_label: Some("ChatGPT Plus".into()),
            active,
            usage_windows: vec![zeron_proto::AgentUsageWindow {
                label: "Session".into(),
                used_fraction: 0.4,
                resets_at: None,
            }],
            usage_fetched_at: None,
            usage_error: None,
            display_name: None,
            organization: None,
            auth_kind: None,
            switchable,
            saved_at: None,
            provider: Some("openai".into()),
        };
        window
            .update(cx, |page, _, cx| {
                page.set_embedded_harness(HarnessId::Opencode, cx);
                page.snapshot = Loadable::Ready(AgentAccountsSnapshot {
                    pool: Default::default(),
                    accounts: vec![
                        row("a", "a@example.com", true, true),
                        row("b", "b@example.com", false, true),
                        AgentAccount {
                            usage_windows: vec![],
                            provider: Some("github-copilot".into()),
                            ..row("c", "GitHub account", true, false)
                        },
                    ],
                    warnings: vec![],
                });
                page.login = Some(LoginFlow {
                    provider: Some("github-copilot"),
                    url: Some("https://github.com/login/device".into()),
                    step: LoginStep::Browser {
                        message: Some("Enter the code ABCD-1234 on GitHub.".into()),
                    },
                    ..waiting(HarnessId::Opencode, 1)
                });
                assert_eq!(
                    page.login.as_ref().unwrap().status().as_ref(),
                    "Enter the code ABCD-1234 on GitHub."
                );
                assert_eq!(
                    page.login.as_ref().unwrap().title(),
                    "Sign in to GitHub Copilot for OpenCode"
                );
            })
            .unwrap();
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
    }

    #[gpui::test]
    fn claudes_paste_fallback_joins_the_same_dialog(cx: &mut gpui::TestAppContext) {
        let window = page(cx);
        window
            .update(cx, |page, _, cx| {
                page.login = Some(LoginFlow {
                    login_id: None,
                    ..waiting(HarnessId::ClaudeCode, 1)
                });
                page.apply_start(
                    1,
                    Ok(AgentLoginStart {
                        login_id: "login-1".into(),
                        url: "https://claude.ai/oauth/authorize?code=true".into(),
                        mode: AgentLoginMode::PasteCode,
                        callback_port: None,
                    }),
                    cx,
                );
                let flow = page.login.as_ref().unwrap();
                assert_eq!(flow.title(), "Sign in to Claude Code");
                assert_eq!(flow.login_id.as_deref(), Some("login-1"));
                assert_eq!(
                    flow.step,
                    LoginStep::PasteCode {
                        submitting: false,
                        error: None
                    }
                );
            })
            .unwrap();
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
    }

    #[test]
    fn provider_grouping_keeps_engine_order_even_when_active_is_later() {
        let account = |id: &str, harness: HarnessId, active: bool| AgentAccount {
            id: id.into(),
            harness,
            email: None,
            plan_label: None,
            active,
            usage_windows: vec![],
            usage_fetched_at: None,
            usage_error: None,
            display_name: None,
            organization: None,
            auth_kind: None,
            switchable: true,
            saved_at: None,
            provider: None,
        };
        let snapshot = AgentAccountsSnapshot {
            pool: Default::default(),
            accounts: vec![
                account("c1", HarnessId::ClaudeCode, false),
                account("x1", HarnessId::Codex, false),
                account("c2", HarnessId::ClaudeCode, true),
            ],
            warnings: vec![],
        };
        let claude = provider_accounts(&snapshot, HarnessId::ClaudeCode);
        let ids: Vec<&str> = claude.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            ["c1", "c2"],
            "engine (creation) order holds — switching must not move a card"
        );
        assert_eq!(provider_accounts(&snapshot, HarnessId::Codex).len(), 1);
        assert!(provider_accounts(&snapshot, HarnessId::Cursor).is_empty());
    }

    // ---- account pool ----

    fn pool_row(id: &str, harness: HarnessId, active: bool, used: &[(&str, f32)]) -> AgentAccount {
        AgentAccount {
            id: id.into(),
            harness,
            email: Some(format!("{id}@example.com")),
            plan_label: Some("Max".into()),
            active,
            usage_windows: used
                .iter()
                .map(|(label, fraction)| zeron_proto::AgentUsageWindow {
                    label: (*label).into(),
                    used_fraction: *fraction,
                    resets_at: None,
                })
                .collect(),
            usage_fetched_at: None,
            usage_error: None,
            display_name: None,
            organization: None,
            auth_kind: None,
            switchable: true,
            saved_at: None,
            provider: None,
        }
    }

    fn pool_status(id: &str, status: AccountPoolStatus) -> AccountPoolAccountStatus {
        AccountPoolAccountStatus {
            account_id: id.into(),
            status,
            until: None,
            family: None,
            weekly_reset_at: None,
            poolable: true,
            unpoolable_reason: None,
            held_until: None,
            error: None,
            last_used_at: None,
            in_flight: 0,
        }
    }

    /// Claude a/b/c (b live) around one Codex row; pool on for Claude with
    /// b next and c excluded.
    fn pooled_snapshot(enabled: bool) -> AgentAccountsSnapshot {
        let mut snapshot = AgentAccountsSnapshot {
            accounts: vec![
                pool_row(
                    "a",
                    HarnessId::ClaudeCode,
                    false,
                    &[("Session", 0.2), ("Week", 0.4)],
                ),
                pool_row("x", HarnessId::Codex, true, &[("5h", 0.1)]),
                pool_row(
                    "b",
                    HarnessId::ClaudeCode,
                    true,
                    &[("Session", 0.1), ("Week", 0.3)],
                ),
                pool_row("c", HarnessId::ClaudeCode, false, &[]),
            ],
            warnings: vec![],
            pool: Default::default(),
        };
        snapshot.pool.config.claude.enabled = enabled;
        snapshot.pool.config.claude.account_order = vec!["a".into(), "b".into(), "c".into()];
        snapshot.pool.config.claude.disabled_account_ids = vec!["c".into()];
        snapshot.pool.providers = vec![AccountPoolProviderStatus {
            harness: HarnessId::ClaudeCode,
            serving: true,
            next_account_id: Some("b".into()),
            next_account_ids: Default::default(),
            notice: None,
        }];
        snapshot.pool.accounts = vec![
            pool_status("a", AccountPoolStatus::Ready),
            pool_status("b", AccountPoolStatus::Next),
            pool_status("c", AccountPoolStatus::Excluded),
        ];
        snapshot
    }

    #[test]
    fn pool_statuses_read_as_one_colored_meta_fragment() {
        use AccountPoolRoutingMode::{Sequential, WeeklyReset};
        let now = Utc::now();
        let soon = now + TimeDelta::minutes(90);
        let thu = now + TimeDelta::days(3);
        let fragment = |status: AccountPoolAccountStatus, mode| {
            pool_status_fragment(Some(&status), false, mode, now)
        };
        let with = |status, f: &dyn Fn(&mut AccountPoolAccountStatus)| {
            let mut status = pool_status("a", status);
            f(&mut status);
            status
        };
        assert_eq!(
            fragment(pool_status("a", AccountPoolStatus::Next), WeeklyReset),
            Some(("Next".into(), PoolTone::Accent))
        );
        assert_eq!(
            fragment(pool_status("a", AccountPoolStatus::Next), Sequential),
            Some(("Current".into(), PoolTone::Accent))
        );
        // Ready shows its weekly reset only where that is the sort key.
        let ready = with(AccountPoolStatus::Ready, &|s| s.weekly_reset_at = Some(thu));
        assert_eq!(
            fragment(ready.clone(), WeeklyReset),
            Some((
                format!("Week {}", format_reset(Some(thu), now).unwrap()),
                PoolTone::Muted
            ))
        );
        assert_eq!(fragment(ready, Sequential), None);
        assert_eq!(
            fragment(pool_status("a", AccountPoolStatus::Ready), WeeklyReset),
            None,
            "no known reset, nothing to say"
        );
        assert_eq!(
            fragment(
                with(AccountPoolStatus::Limited, &|s| s.until = Some(soon)),
                WeeklyReset
            ),
            Some((
                format!("Limited until {}", format_moment(soon, now)),
                PoolTone::Warning
            ))
        );
        assert_eq!(
            fragment(pool_status("a", AccountPoolStatus::Limited), WeeklyReset),
            Some(("Limited".into(), PoolTone::Warning))
        );
        assert_eq!(
            fragment(
                with(AccountPoolStatus::FamilyLimited, &|s| {
                    s.family = Some("opus".into());
                    s.until = Some(thu);
                }),
                WeeklyReset
            ),
            Some((
                format!("Opus limit · {}", format_reset(Some(thu), now).unwrap()),
                PoolTone::Warning
            ))
        );
        assert_eq!(
            fragment(
                with(AccountPoolStatus::Held, &|s| s.until = Some(soon)),
                WeeklyReset
            ),
            Some((
                format!("Rate limited · retry {}", format_moment(soon, now)),
                PoolTone::Warning
            ))
        );
        assert_eq!(
            fragment(
                pool_status("a", AccountPoolStatus::NeedsSignIn),
                WeeklyReset
            ),
            Some(("Needs sign-in".into(), PoolTone::Danger))
        );
        let expired = pool_status("a", AccountPoolStatus::LiveExpired);
        assert_eq!(
            fragment(expired.clone(), WeeklyReset),
            Some(("Skipped · live token expired".into(), PoolTone::Warning))
        );
        assert!(pool_status_tooltip(&expired).is_some_and(|tip| tip.contains("never refreshes")));
        // The config's exclusion wins over a status computed before it.
        assert_eq!(
            pool_status_fragment(
                Some(&pool_status("a", AccountPoolStatus::Next)),
                true,
                WeeklyReset,
                now
            ),
            Some(("Not in pool".into(), PoolTone::Muted))
        );
        let theme = Theme::dark();
        assert_eq!(pool_tone_color(PoolTone::Accent, &theme), theme.accent);
        assert_eq!(pool_tone_color(PoolTone::Warning, &theme), theme.warning);
        assert_eq!(pool_tone_color(PoolTone::Danger, &theme), theme.danger);
        assert_eq!(pool_tone_color(PoolTone::Muted, &theme), theme.text_muted);
    }

    #[test]
    fn meters_draw_the_session_and_the_binding_weekly_window() {
        let window = |label: &str, used| zeron_proto::AgentUsageWindow {
            label: label.into(),
            used_fraction: used,
            resets_at: None,
        };
        // Two windows: exactly as before.
        assert_eq!(
            meter_windows(&[window("Session", 0.1), window("Week", 0.2)]),
            [0, 1]
        );
        // A family limit that binds replaces the shared week on line two…
        let windows = [
            window("Session", 0.1),
            window("Week", 0.4),
            window("Week (opus)", 0.9),
            window("Week (sonnet)", 0.2),
        ];
        assert_eq!(meter_windows(&windows), [0, 2]);
        assert_eq!(meter_label("Week (opus)"), "Opus");
        assert_eq!(meter_label("Week"), "Week");
        // …and the windows left out still show in the tooltip, with percent.
        let now = Utc::now();
        assert_eq!(
            usage_tooltip(&windows, &[0, 2], now),
            "Session · Week 40% · Week (opus) · Week (sonnet) 20%"
        );
        // Ties keep the engine's order: the shared week.
        assert_eq!(
            meter_windows(&[
                window("Session", 0.1),
                window("Week", 0.5),
                window("Week (opus)", 0.5)
            ]),
            [0, 1]
        );
    }

    #[test]
    fn reorders_move_one_row_and_keep_other_providers_in_place() {
        assert_eq!(PoolMove::Top.target(2, 3), Some(0));
        assert_eq!(PoolMove::Up.target(2, 3), Some(1));
        assert_eq!(PoolMove::Down.target(2, 3), None);
        assert_eq!(PoolMove::Up.target(0, 3), None);
        assert_eq!(PoolMove::Top.target(0, 3), None);
        assert_eq!(PoolMove::Down.target(0, 3), Some(1));
        let order: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        assert_eq!(moved(&order, 2, 0), ["c", "a", "b"]);
        assert_eq!(moved(&order, 0, 2), ["b", "c", "a"]);
        assert_eq!(moved(&order, 1, 1), order);

        let mut snapshot = pooled_snapshot(true);
        apply_pool_order(&mut snapshot, HarnessId::ClaudeCode, &moved(&order, 2, 0));
        let ids: Vec<&str> = snapshot.accounts.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["c", "x", "a", "b"], "Codex keeps its slot");
        assert_eq!(snapshot.pool.config.claude.account_order, ["c", "a", "b"]);
        assert!(snapshot.pool.config.codex.account_order.is_empty());
    }

    #[test]
    fn the_pool_switch_row_summarizes_members_and_next() {
        let now = Utc::now();
        let mut snapshot = pooled_snapshot(true);
        assert_eq!(
            pool_summary(&snapshot, HarnessId::ClaudeCode, now),
            "2 of 3 accounts · Next: b@…"
        );
        let thu = now + TimeDelta::days(3);
        snapshot.pool.accounts[1].weekly_reset_at = Some(thu);
        assert_eq!(
            pool_summary(&snapshot, HarnessId::ClaudeCode, now),
            format!(
                "2 of 3 accounts · Next: b@… · week {}",
                format_reset(Some(thu), now).unwrap()
            )
        );
        snapshot.pool.config.claude.routing_mode = AccountPoolRoutingMode::Sequential;
        assert_eq!(
            pool_summary(&snapshot, HarnessId::ClaudeCode, now),
            "2 of 3 accounts · Current: b@…"
        );
        // A login that can't join doesn't count as a member.
        snapshot.pool.accounts[0].poolable = false;
        assert!(pool_summary(&snapshot, HarnessId::ClaudeCode, now).starts_with("1 of 3"));
        // Off, nothing is "next".
        snapshot.pool.config.claude.enabled = false;
        assert_eq!(pool_next_id(&snapshot, HarnessId::ClaudeCode), None);
    }

    #[test]
    fn pool_controls_cover_presets_and_engine_values() {
        assert_eq!(threshold_options(0.98), (POOL_THRESHOLDS.to_vec(), 2));
        let (options, selected) = threshold_options(0.85);
        assert_eq!(options.len(), 5);
        assert_eq!(threshold_label(options[selected]), "85%");
        assert!(pools_accounts(HarnessId::ClaudeCode) && pools_accounts(HarnessId::Codex));
        assert!(!pools_accounts(HarnessId::Cursor) && !pools_accounts(HarnessId::Opencode));
        assert!(
            pool_footnote(HarnessId::Codex, AccountPoolRoutingMode::WeeklyReset)
                .ends_with("Voice chats always use the live login.")
        );
        assert!(
            !pool_footnote(HarnessId::ClaudeCode, AccountPoolRoutingMode::WeeklyReset)
                .contains("Voice")
        );
        assert_eq!(short_email("alice@example.com"), "alice@…");
    }

    #[test]
    fn api_key_slots_pool_but_never_become_the_live_login() {
        let mut key = pool_row("k", HarnessId::ClaudeCode, false, &[]);
        key.auth_kind = Some(zeron_proto::AgentAuthKind::ApiKey);
        assert!(is_api_key_slot(&key) && !can_switch_live(&key));
        let oauth = pool_row("o", HarnessId::ClaudeCode, false, &[]);
        assert!(!is_api_key_slot(&oauth) && can_switch_live(&oauth));
        // Codex API-key logins are ordinary CLI logins.
        let mut codex = pool_row("x", HarnessId::Codex, false, &[]);
        codex.auth_kind = Some(zeron_proto::AgentAuthKind::ApiKey);
        assert!(can_switch_live(&codex));
    }

    #[test]
    fn a_row_click_switches_only_while_the_pool_is_off() {
        let saved = pool_row("a", HarnessId::ClaudeCode, false, &[]);
        let live = pool_row("b", HarnessId::ClaudeCode, true, &[]);
        assert!(
            row_click_switches(&saved, false),
            "pool off: today's click-to-switch"
        );
        assert!(
            !row_click_switches(&live, false),
            "the live row has nothing to switch to"
        );
        assert!(
            !row_click_switches(&saved, true),
            "pool on: switching moves to the menu"
        );
        let mut key = saved.clone();
        key.auth_kind = Some(zeron_proto::AgentAuthKind::ApiKey);
        assert!(!row_click_switches(&key, false));
    }

    fn pool_page(
        cx: &mut gpui::TestAppContext,
        snapshot: AgentAccountsSnapshot,
    ) -> (gpui::Entity<AccountsPage>, &mut gpui::VisualTestContext) {
        use gpui::AppContext;
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            crate::settings::init(Default::default(), dir.path(), cx);
            gpui_base::init(cx);
            cx.set_global(crate::theme::Theme::default());
        });
        let (page, cx) = cx.add_window_view(|_, cx| {
            let state = cx.new(|_| crate::state::AppState::new());
            AccountsPage::new_embedded(state, None, HarnessId::ClaudeCode, cx)
        });
        page.update(cx, |page, cx| {
            page.snapshot = Loadable::Ready(snapshot);
            page.refreshing = false;
            cx.notify();
        });
        cx.run_until_parked();
        (page, cx)
    }

    /// Pool off, a Claude row is today's row (no order slot); pool on, the
    /// slot is there at full width — at once under reduced motion — and no
    /// row changes height.
    #[gpui::test]
    fn the_pool_toggle_reveals_chrome_without_moving_a_row(cx: &mut gpui::TestAppContext) {
        let (page, cx) = pool_page(cx, pooled_snapshot(false));
        cx.update(|_, cx| crate::motion::set_reduced_motion(cx, true));
        cx.simulate_resize(gpui::size(px(900.0), px(700.0)));
        page.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("pool-handle-a").is_none(),
            "pool off: today's row"
        );
        let heights_off: Vec<_> = [
            "account-row-a-slot",
            "account-row-b-slot",
            "account-row-c-slot",
        ]
        .map(|selector| cx.debug_bounds(selector).unwrap().size.height)
        .to_vec();

        page.update(cx, |page, cx| {
            if let Loadable::Ready(snapshot) = &mut page.snapshot {
                snapshot.pool.config.claude.enabled = true;
            }
            cx.notify();
        });
        cx.run_until_parked();
        let handle = cx.debug_bounds("pool-handle-a").expect("order slot");
        assert_eq!(
            handle.size.width,
            px(POOL_ORDER_WIDTH),
            "reduced motion: final state at once"
        );
        let heights_on: Vec<_> = [
            "account-row-a-slot",
            "account-row-b-slot",
            "account-row-c-slot",
        ]
        .map(|selector| cx.debug_bounds(selector).unwrap().size.height)
        .to_vec();
        assert_eq!(heights_off, heights_on);
        let order = page.read_with(cx, |page, _| {
            page.display_accounts(page.snapshot.ready().unwrap(), HarnessId::ClaudeCode)
                .iter()
                .map(|a| a.id.clone())
                .collect::<Vec<_>>()
        });
        assert_eq!(order, ["a", "b", "c"], "turning the pool on never reorders");
    }

    /// Space picks a row up, ↓ moves it, Escape puts it back; a refused
    /// change (here: no engine) restores the list and toasts.
    #[gpui::test]
    fn keyboard_reorder_and_refused_changes_restore_the_list(cx: &mut gpui::TestAppContext) {
        let (page, cx) = pool_page(cx, pooled_snapshot(true));
        page.update(cx, |page, cx| {
            let claude = HarnessId::ClaudeCode;
            let order = |page: &AccountsPage| -> Vec<String> {
                page.display_accounts(page.snapshot.ready().unwrap(), claude)
                    .iter()
                    .map(|a| a.id.clone())
                    .collect()
            };
            assert!(page.pool_key(claude, "a", "space", cx));
            assert!(page.pool_key(claude, "a", "down", cx));
            assert_eq!(order(page), ["b", "a", "c"]);
            assert!(page.pool_slides.contains_key("a") && page.pool_slides.contains_key("b"));
            assert!(page.pool_key(claude, "a", "escape", cx));
            assert_eq!(order(page), ["a", "b", "c"]);
            assert!(page.pool_grab.is_none());
            // Keys on a row that isn't picked up pass through.
            assert!(!page.pool_key(claude, "a", "down", cx));

            // Optimistic member switch, refused: back to excluded, toast.
            page.set_pool_member(claude, "c", true, cx);
            let config = &page.snapshot.ready().unwrap().pool.config.claude;
            assert_eq!(config.disabled_account_ids, ["c"]);
            assert!(page.error.is_some());
            assert!(page.pool_baseline.is_none());

            // A refused reorder slides back to the confirmed order.
            page.error = None;
            page.move_pool_row(claude, "c", PoolMove::Top, cx);
            assert_eq!(order(page), ["a", "b", "c"]);
            assert!(page.error.is_some());
        });
        cx.run_until_parked();
    }

    /// Pool rows render for Claude (status, switches, footnote) in both
    /// layouts' shared renderer; Cursor gets no pool block.
    #[gpui::test]
    fn pool_rows_render_and_other_providers_stay_plain(cx: &mut gpui::TestAppContext) {
        let mut snapshot = pooled_snapshot(true);
        snapshot.pool.providers[0].notice =
            Some("No pooled account can take requests — sessions use the live login.".into());
        snapshot
            .accounts
            .push(pool_row("cur", HarnessId::Cursor, true, &[]));
        let (page, cx) = pool_page(cx, snapshot);
        cx.run_until_parked();
        assert!(cx.debug_bounds("pool-handle-b").is_some());
        page.update(cx, |page, cx| {
            let snapshot = page.snapshot.ready().unwrap().clone();
            let theme = Theme::dark();
            assert!(
                page.render_pool_block(HarnessId::Cursor, &snapshot, &theme, Utc::now(), cx)
                    .is_none()
            );
            assert!(
                page.render_pool_footnote(HarnessId::ClaudeCode, &snapshot, &theme)
                    .is_some()
            );
            page.set_embedded_harness(HarnessId::Cursor, cx);
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("pool-handle-cur").is_none());
    }
    #[gpui::test]
    fn committed_pool_changes_survive_a_later_rejected_edit(cx: &mut gpui::TestAppContext) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let (out, mut requests) = tokio::sync::mpsc::channel::<String>(32);
        let (replies, inbound) = tokio::sync::mpsc::channel::<String>(32);
        let (page, cx) = pool_page(cx, pooled_snapshot(false));
        page.update(cx, |page, cx| {
            page.state.update(cx, |state, _| {
                state.set_test_engine(crate::state::EngineHandle::from_test_client(
                    zeron_rpc::RpcClient::new(out, inbound),
                ))
            });
            page.set_pool_enabled(HarnessId::ClaudeCode, true, cx);
            page.configure_pool(
                HarnessId::ClaudeCode,
                |snapshot| snapshot.pool.config.claude.switch_threshold = 0.95,
                cx,
            );
        });
        let pump = |cx: &mut gpui::VisualTestContext| {
            for _ in 0..20 {
                runtime.block_on(async {
                    tokio::task::yield_now().await;
                });
                cx.run_until_parked();
            }
        };
        pump(cx);
        let first: zeron_rpc::ClientFrame =
            serde_json::from_str(&requests.try_recv().unwrap()).unwrap();
        assert_eq!(
            first.method.as_deref(),
            Some(methods::CONFIGURE_ACCOUNT_POOL)
        );
        assert!(
            requests.try_recv().is_err(),
            "later edits must not cancel or overtake the first RPC"
        );
        let mut confirmed = pooled_snapshot(true);
        confirmed.pool.config.claude.switch_threshold = 0.98;
        replies
            .try_send(
                serde_json::to_string(&zeron_rpc::ServerFrame {
                    id: first.id,
                    ok: Some(serde_json::to_value(&confirmed).unwrap()),
                    ..Default::default()
                })
                .unwrap(),
            )
            .unwrap();
        pump(cx);
        let second: zeron_rpc::ClientFrame =
            serde_json::from_str(&requests.try_recv().unwrap()).unwrap();
        assert_eq!(
            second.method.as_deref(),
            Some(methods::CONFIGURE_ACCOUNT_POOL)
        );
        assert_eq!(
            second.params["config"]["switchThreshold"],
            serde_json::json!(0.95_f32)
        );
        replies
            .try_send(
                serde_json::to_string(&zeron_rpc::ServerFrame {
                    id: second.id,
                    err: Some("Rejected second edit".into()),
                    ..Default::default()
                })
                .unwrap(),
            )
            .unwrap();
        pump(cx);
        let reconcile: zeron_rpc::ClientFrame =
            serde_json::from_str(&requests.try_recv().unwrap()).unwrap();
        assert_eq!(reconcile.method.as_deref(), Some(methods::GET_ACCOUNT_POOL));
        replies
            .try_send(
                serde_json::to_string(&zeron_rpc::ServerFrame {
                    id: reconcile.id,
                    ok: Some(serde_json::to_value(&confirmed.pool).unwrap()),
                    ..Default::default()
                })
                .unwrap(),
            )
            .unwrap();
        pump(cx);
        page.read_with(cx, |page, cx| {
            let config = &page.snapshot.ready().unwrap().pool.config.claude;
            assert!(
                config.enabled,
                "the acknowledged first mutation must remain committed"
            );
            assert_eq!(config.switch_threshold, 0.98);
            assert!(page.pool_baseline.is_none());
            assert!(page.pool_queue.is_empty());
            assert_eq!(
                cx.global::<AccountsSnapshotCache>().0[&None]
                    .pool
                    .config
                    .claude,
                *config
            );
        });
        // A later write commits but its discovery phase fails. The pool-only
        // read restores committed routing/order without replacing newer usage.
        page.update(cx, |page, cx| {
            page.configure_pool(
                HarnessId::ClaudeCode,
                |snapshot| {
                    snapshot.pool.config.claude.enabled = false;
                    apply_pool_order(
                        snapshot,
                        HarnessId::ClaudeCode,
                        &["c".into(), "a".into(), "b".into()],
                    );
                },
                cx,
            );
            cx.default_global::<AccountsSnapshotCache>()
                .0
                .get_mut(&None)
                .unwrap()
                .accounts
                .iter_mut()
                .find(|row| row.id == "a")
                .unwrap()
                .usage_windows[0]
                .used_fraction = 0.55;
            cx.default_global::<AccountsSnapshotCache>()
                .0
                .get_mut(&None)
                .unwrap()
                .pool
                .config
                .bypassed_chat_ids
                .push("another-chat".into());
        });
        pump(cx);
        let committed: zeron_rpc::ClientFrame =
            serde_json::from_str(&requests.try_recv().unwrap()).unwrap();
        assert_eq!(
            committed.method.as_deref(),
            Some(methods::CONFIGURE_ACCOUNT_POOL)
        );
        assert_eq!(committed.params["config"]["enabled"], false);
        replies
            .try_send(
                serde_json::to_string(&zeron_rpc::ServerFrame {
                    id: committed.id,
                    err: Some("Configuration saved; account discovery failed".into()),
                    ..Default::default()
                })
                .unwrap(),
            )
            .unwrap();
        pump(cx);
        let recovery: zeron_rpc::ClientFrame =
            serde_json::from_str(&requests.try_recv().unwrap()).unwrap();
        assert_eq!(recovery.method.as_deref(), Some(methods::GET_ACCOUNT_POOL));
        confirmed.pool.config.claude.enabled = false;
        confirmed.pool.config.claude.account_order = vec!["c".into(), "a".into(), "b".into()];
        replies
            .try_send(
                serde_json::to_string(&zeron_rpc::ServerFrame {
                    id: recovery.id,
                    ok: Some(serde_json::to_value(&confirmed.pool).unwrap()),
                    ..Default::default()
                })
                .unwrap(),
            )
            .unwrap();
        pump(cx);
        page.read_with(cx, |page, cx| {
            let snapshot = page.snapshot.ready().unwrap();
            assert!(!snapshot.pool.config.claude.enabled);
            assert_eq!(pool_order(snapshot, HarnessId::ClaudeCode), ["c", "a", "b"]);
            assert_eq!(
                snapshot
                    .accounts
                    .iter()
                    .find(|row| row.id == "a")
                    .unwrap()
                    .usage_windows[0]
                    .used_fraction,
                0.55
            );
            assert!(
                !cx.global::<AccountsSnapshotCache>().0[&None]
                    .pool
                    .config
                    .claude
                    .enabled
            );
            assert!(page.pool_baseline.is_none());
            assert!(
                snapshot
                    .pool
                    .config
                    .bypassed_chat_ids
                    .contains(&"another-chat".into())
            );
            assert!(
                crate::toast::messages(cx)
                    .iter()
                    .any(|message| message.contains("account discovery failed"))
            );
        });
    }

    #[test]
    fn an_unsaved_unsupported_row_is_not_part_of_the_reorder_rpc() {
        let mut snapshot = pooled_snapshot(true);
        snapshot
            .accounts
            .push(pool_row("unreadable", HarnessId::ClaudeCode, true, &[]));
        let mut status = pool_status("unreadable", AccountPoolStatus::Unsupported);
        status.poolable = false;
        snapshot.pool.accounts.push(status);
        assert_eq!(
            pool_order(&snapshot, HarnessId::ClaudeCode),
            ["a", "b", "c"]
        );
        let order = vec!["b".into(), "a".into(), "c".into()];
        apply_pool_order(&mut snapshot, HarnessId::ClaudeCode, &order);
        assert_eq!(snapshot.pool.config.claude.account_order, order);
        assert!(
            snapshot
                .accounts
                .iter()
                .any(|account| account.id == "unreadable")
        );
    }
}
