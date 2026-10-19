//! A chat named somewhere else: a `@chat:` reference in prose, the child an
//! activity row reports from, the task a spawn line started. Every surface
//! draws it the same way — the chat's harness mark, its live title and its
//! live status — and resolves it at render time, so a rename or a status
//! change is a repaint, never a rebuild.
//!
//! Also home to the task [`Mark`]: how a delegated task's status reads at a
//! glance, shared by the lanes card, the transcript and the sidebar.

use std::hash::{Hash, Hasher};

use chrono::Utc;
use gpui::{AnyElement, App, EntityId, SharedString, div, prelude::*, px};

use zeron_proto::{Chat, ChatIndicator, DelegatedTask, HarnessId, TaskStatus};

use crate::icons::{self, icon};
use crate::loaders;
use crate::state::AppState;
use crate::theme::Theme;

// ── Task and chat marks ────────────────────────────────────────────────

/// How a chat's status reads at a glance — its mark in the icon slot. A
/// managed task reads from its task record; any other chat from its live
/// indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Queued,
    Working,
    WaitingOnTasks,
    NeedsInput,
    Stopping,
    /// Completed, result not yet sent to the parent (or, for a plain chat,
    /// finished and not seen yet).
    Ready,
    /// Completed and its result sent.
    Sent,
    Failed,
    Cancelled,
    Interrupted,
    /// A plain chat at rest.
    Idle,
}

impl Mark {
    pub(crate) fn terminal(self) -> bool {
        matches!(
            self,
            Self::Ready | Self::Sent | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }

    /// Spoken status, for assistive tech and tooltips.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Working => "Working",
            Self::WaitingOnTasks => "Waiting for tasks",
            Self::NeedsInput => "Needs input",
            Self::Stopping => "Stopping",
            Self::Ready => "Finished",
            Self::Sent => "Finished",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
            Self::Interrupted => "Interrupted",
            Self::Idle => "Idle",
        }
    }
}

/// `stopping`: a stop this window asked for that no chats frame has
/// confirmed yet.
pub(crate) fn mark_for(task: &DelegatedTask, stopping: bool) -> Mark {
    if (task.cancel_requested || stopping) && !task.status.is_terminal() {
        return Mark::Stopping;
    }
    match task.status {
        TaskStatus::Queued => Mark::Queued,
        TaskStatus::Running => Mark::Working,
        TaskStatus::Waiting => Mark::NeedsInput,
        TaskStatus::WaitingForChildren => Mark::WaitingOnTasks,
        // A stop suppresses delivery: nothing is owed any more.
        TaskStatus::Completed if task.delivered || task.cancel_requested => Mark::Sent,
        TaskStatus::Completed => Mark::Ready,
        TaskStatus::Failed => Mark::Failed,
        TaskStatus::Cancelled => Mark::Cancelled,
        TaskStatus::Interrupted => Mark::Interrupted,
    }
}

/// A plain chat's mark from its display indicator.
pub(crate) fn mark_for_indicator(indicator: ChatIndicator) -> Mark {
    match indicator {
        ChatIndicator::Working => Mark::Working,
        ChatIndicator::AwaitingInput => Mark::NeedsInput,
        ChatIndicator::Errored => Mark::Failed,
        ChatIndicator::Completed => Mark::Ready,
        ChatIndicator::Idle => Mark::Idle,
    }
}

fn dot(color: gpui::Hsla) -> AnyElement {
    div()
        .size(px(6.0))
        .rounded_full()
        .bg(color)
        .into_any_element()
}

pub(crate) fn mark_element(
    key: String,
    mark: Mark,
    view: EntityId,
    theme: &Theme,
    cx: &mut App,
) -> AnyElement {
    use crate::shell::spaces::status_dot_color;
    match mark {
        Mark::Queued => div()
            .size(px(7.0))
            .rounded_full()
            .border_1()
            .border_color(theme.text_muted.opacity(0.7))
            .into_any_element(),
        Mark::Working => {
            loaders::mini_glyph_spinner(format!("{key}-working"), 2.0, theme.glyph, view, cx)
                .into_any_element()
        }
        Mark::WaitingOnTasks => {
            loaders::mini_mono_spinner(format!("{key}-children"), 2.0, theme.text_muted, view, cx)
                .into_any_element()
        }
        Mark::Stopping => {
            loaders::mini_mono_spinner(format!("{key}-stopping"), 2.0, theme.warning, view, cx)
                .into_any_element()
        }
        Mark::NeedsInput => dot(status_dot_color(ChatIndicator::AwaitingInput, theme)),
        Mark::Ready => icon(icons::CHECK)
            .size(px(12.0))
            .text_color(status_dot_color(ChatIndicator::Completed, theme))
            .into_any_element(),
        Mark::Sent => icon(icons::CHECK)
            .size(px(12.0))
            .text_color(theme.text_muted)
            .into_any_element(),
        Mark::Failed => dot(status_dot_color(ChatIndicator::Errored, theme)),
        Mark::Cancelled => icon(icons::CLOSE)
            .size(px(10.0))
            .text_color(theme.text_faint)
            .into_any_element(),
        Mark::Interrupted => dot(theme.warning.opacity(0.7)),
        Mark::Idle => dot(status_dot_color(ChatIndicator::Idle, theme)),
    }
}

/// The status tone of a status word.
pub(crate) fn mark_tone(mark: Mark, theme: &Theme) -> gpui::Hsla {
    match mark {
        Mark::NeedsInput => theme.accent,
        Mark::Ready => theme.success,
        Mark::Failed => theme.danger,
        Mark::Stopping | Mark::Interrupted => theme.warning,
        _ => theme.text_muted,
    }
}

// ── Resolution ─────────────────────────────────────────────────────────

/// A chat resolved for display. `known == false`: the id has no row here
/// (deleted, another account's, or never real) — the pill reads "Unknown
/// chat" and opens nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRef {
    /// The full id when known; the written id otherwise.
    pub chat_id: SharedString,
    pub title: SharedString,
    pub harness: Option<HarnessId>,
    pub model: Option<SharedString>,
    pub mark: Mark,
    pub known: bool,
    pub archived: bool,
}

impl ChatRef {
    pub(crate) fn unknown(written: &str) -> Self {
        Self {
            chat_id: written.to_owned().into(),
            title: "Unknown chat".into(),
            harness: None,
            model: None,
            mark: Mark::Idle,
            known: false,
            archived: false,
        }
    }

    pub(crate) fn of(state: &AppState, chat: &Chat) -> Self {
        let mark = match chat.orchestration.as_ref().and_then(|o| o.task.as_ref()) {
            Some(task) => mark_for(task, false),
            None => mark_for_indicator(state.display_status_for(chat, Utc::now())),
        };
        Self {
            chat_id: chat.id.clone().into(),
            title: chat_title(chat).into(),
            harness: chat.config.as_ref().map(|config| config.harness),
            model: chat
                .config
                .as_ref()
                .and_then(|config| config.model.clone())
                .map(SharedString::from),
            mark,
            known: true,
            archived: chat.archived,
        }
    }

    /// Resolve a written id: exact, else a unique prefix (case-insensitive,
    /// at least [`zeron_proto::chat_refs::CHAT_REF_MIN_PREFIX`] long).
    pub(crate) fn resolve(state: &AppState, written: &str) -> Self {
        resolve_chat(state, written)
            .map(|chat| Self::of(state, chat))
            .unwrap_or_else(|| Self::unknown(written))
    }

    /// Accessible name: "{title}, {status}".
    pub(crate) fn spoken(&self) -> String {
        if self.known {
            format!("{}, {}", self.title, self.mark.label())
        } else {
            self.title.to_string()
        }
    }
}

/// The chat a written id names, if exactly one does.
pub(crate) fn resolve_chat<'a>(state: &'a AppState, written: &str) -> Option<&'a Chat> {
    let written = written.trim();
    if written.is_empty() {
        return None;
    }
    if let Some(chat) = state.chats.iter().find(|chat| chat.id == written) {
        return Some(chat);
    }
    if written.len() < zeron_proto::chat_refs::CHAT_REF_MIN_PREFIX {
        return None;
    }
    let mut matches = state.chats.iter().filter(|chat| {
        chat.id.len() >= written.len()
            && chat.id.is_char_boundary(written.len())
            && chat.id[..written.len()].eq_ignore_ascii_case(written)
    });
    match (matches.next(), matches.next()) {
        (Some(chat), None) => Some(chat),
        _ => None,
    }
}

/// A chat's title, or what stands in until one is generated.
pub(crate) fn chat_title(chat: &Chat) -> String {
    chat.title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .or_else(|| {
            chat.orchestration
                .as_ref()
                .and_then(|o| o.task.as_ref())
                .and_then(|task| {
                    let line = task.prompt.lines().find(|l| !l.trim().is_empty())?.trim();
                    let mut out: String = line.chars().take(60).collect();
                    if line.chars().count() > 60 {
                        out.push('…');
                    }
                    Some(out)
                })
        })
        .or_else(|| chat.last_message_preview.clone())
        .map(|title| title.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|| "New chat".into())
}

/// The harness names a title may repeat in parentheses. A pill draws the
/// harness mark beside the title, so "(Codex)" beside the Codex mark is
/// noise.
fn harness_title_names(harness: HarnessId) -> &'static [&'static str] {
    use HarnessId::*;
    match harness {
        ClaudeCode => &["Claude Code", "Claude"],
        Codex => &["Codex"],
        Cursor => &["Cursor"],
        Devin => &["Devin"],
        Grok => &["Grok"],
        Hermes => &["Hermes"],
        Pi => &["Pi"],
        Opencode => &["OpenCode"],
        _ => &[],
    }
}

/// The title a pill draws: a trailing parenthesised name of the chat's own
/// harness is dropped (the mark says it), unless nothing else is left.
pub(crate) fn display_title(title: &str, harness: Option<HarnessId>) -> &str {
    let Some(harness) = harness else {
        return title;
    };
    let Some(inside) = title.trim_end().strip_suffix(')') else {
        return title;
    };
    let Some(open) = inside.rfind('(') else {
        return title;
    };
    let name = inside[open + 1..].trim();
    if !harness_title_names(harness)
        .iter()
        .any(|known| known.eq_ignore_ascii_case(name))
    {
        return title;
    }
    let stripped = title[..open].trim_end();
    if stripped.is_empty() { title } else { stripped }
}

/// Hash what changes a pill's LAYOUT — which chat a written id names, its
/// title and harness. Status is painted live and stays out, so a status
/// change never remeasures a row.
pub(crate) fn layout_fingerprint<'a>(
    state: &AppState,
    written: impl IntoIterator<Item = &'a str>,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for id in written {
        id.hash(&mut hasher);
        match resolve_chat(state, id) {
            Some(chat) => {
                chat.id.hash(&mut hasher);
                chat_title(chat).hash(&mut hasher);
                chat.config.as_ref().map(|c| c.harness as u8).hash(&mut hasher);
            }
            None => 0u8.hash(&mut hasher),
        }
    }
    hasher.finish()
}

// ── Elements ───────────────────────────────────────────────────────────

/// The harness a chat runs on, as the sidebar's brand mark.
pub(crate) fn harness_mark(harness: HarnessId, size: f32, theme: &Theme) -> gpui::Svg {
    let (path, tint) = crate::pickers::harness_brand_icon(harness);
    icon(path)
        .size(px(size))
        .flex_none()
        .text_color(tint.unwrap_or(theme.text_muted).opacity(0.8))
}

/// Height of a pill inside a row (activity lines, spawn lines).
pub(crate) const PILL_HEIGHT: f32 = 22.0;

/// A chat pill for rows: harness mark, the live title (dissolving at its
/// right edge when it runs long) and, when `with_mark`, the live status
/// mark. The caller adds identity, hover and click.
pub(crate) fn pill(
    key: &str,
    chat: &ChatRef,
    with_mark: bool,
    view: EntityId,
    theme: &Theme,
    cx: &mut App,
) -> gpui::Div {
    let title = SharedString::from(display_title(&chat.title, chat.harness).to_owned());
    div()
        .h(px(PILL_HEIGHT))
        .max_w(px(320.0))
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(5.0))
        .pl(px(6.0))
        .pr(px(if with_mark && chat.known { 5.0 } else { 7.0 }))
        .rounded(px(Theme::CONTROL_RADIUS))
        .bg(theme.ink(0.055))
        .text_color(if chat.known {
            theme.text
        } else {
            theme.text_muted
        })
        .when_some(chat.harness.filter(|_| chat.known), |el, harness| {
            el.child(harness_mark(harness, 12.0, theme))
        })
        .when(!chat.known, |el| {
            el.child(
                icon(icons::BOT)
                    .size(px(12.0))
                    .flex_none()
                    .text_color(theme.text_faint),
            )
        })
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(title),
        )
        .when(with_mark && chat.known, |el| {
            el.child(
                div()
                    .flex_none()
                    .size(px(13.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(mark_element(format!("pill-{key}"), chat.mark, view, theme, cx)),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(id: &str, title: Option<&str>) -> Chat {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "deviceId": "dev",
            "archived": false,
            "createdAt": Utc::now(),
            "title": title,
            "config": {"harness": "codex", "model": "gpt-5.3-codex", "sandbox": "workspace-write"},
        }))
        .unwrap()
    }

    #[test]
    fn written_ids_resolve_exactly_or_by_a_unique_prefix() {
        let mut state = AppState::new();
        state
            .chats
            .push(chat("3f6b2a18-9c4d-4e5f-8a7b-1c2d3e4f5a6b", Some("Audit (Codex)")));
        state
            .chats
            .push(chat("3f6b2a18-1111-4e5f-8a7b-1c2d3e4f5a6b", None));
        state
            .chats
            .push(chat("ba6888c8-0000-4e5f-8a7b-1c2d3e4f5a6b", Some("Review")));
        let exact = ChatRef::resolve(&state, "3f6b2a18-9c4d-4e5f-8a7b-1c2d3e4f5a6b");
        assert!(exact.known);
        assert_eq!(display_title(&exact.title, exact.harness), "Audit");
        assert_eq!(exact.mark, Mark::Idle);
        assert_eq!(
            ChatRef::resolve(&state, "BA6888C8").chat_id.as_ref(),
            "ba6888c8-0000-4e5f-8a7b-1c2d3e4f5a6b"
        );
        // Ambiguous and too-short prefixes resolve nothing.
        assert!(!ChatRef::resolve(&state, "3f6b2a18").known);
        assert!(!ChatRef::resolve(&state, "ba6888").known);
        assert_eq!(ChatRef::resolve(&state, "nope-nope").title.as_ref(), "Unknown chat");
        // Untitled chats read as new.
        assert_eq!(
            ChatRef::resolve(&state, "3f6b2a18-1111").title.as_ref(),
            "New chat"
        );
    }

    #[test]
    fn a_tasks_mark_comes_from_its_record() {
        let mut state = AppState::new();
        let mut child = chat("c1c1c1c1-0000", Some("Child"));
        child.orchestration = Some(serde_json::from_value(serde_json::json!({
            "task": {
                "id": "c1c1c1c1-0000", "parentChatId": "p", "role": "review",
                "isolation": "shared", "status": "waiting_for_children", "prompt": "x",
                "requestFingerprint": "f", "createdAt": 1, "updatedAt": 2
            }
        })).unwrap());
        state.chats.push(child);
        assert_eq!(ChatRef::resolve(&state, "c1c1c1c1-0000").mark, Mark::WaitingOnTasks);
    }

    #[test]
    fn the_layout_fingerprint_ignores_status_but_not_titles() {
        let mut state = AppState::new();
        state.chats.push(chat("c1c1c1c1-0000", Some("Child")));
        let base = layout_fingerprint(&state, ["c1c1c1c1"]);
        state.begin_pending_send("c1c1c1c1-0000", "m1", Utc::now());
        assert_eq!(base, layout_fingerprint(&state, ["c1c1c1c1"]));
        state.chats[0].title = Some("Renamed".into());
        assert_ne!(base, layout_fingerprint(&state, ["c1c1c1c1"]));
    }

    #[test]
    fn display_title_strips_only_the_chats_own_harness() {
        let codex = Some(HarnessId::Codex);
        assert_eq!(display_title("Audit the sync layer (Codex)", codex), "Audit the sync layer");
        assert_eq!(display_title("Fix the queue (codex)", codex), "Fix the queue");
        assert_eq!(
            display_title("Audit (Codex)", Some(HarnessId::Devin)),
            "Audit (Codex)"
        );
        assert_eq!(display_title("Retry (draft)", codex), "Retry (draft)");
        assert_eq!(display_title("(Codex)", codex), "(Codex)");
    }
}
