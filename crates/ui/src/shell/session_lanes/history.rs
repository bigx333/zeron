//! History in the lanes card: the checkpoints the engine saves after each
//! finished turn (conversation, provider session and, when captured, the
//! checkout), with restore in place. A restore keeps where you were as a
//! recovery point, so the section offers Undo right after one, and keeps
//! recovery points folded for inspection.
//!
//! Every word comes from what the engine returned; the only local state is
//! a request in flight and the last restore's receipt.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use zeron_proto::{Chat, ChatIndicator};

use super::tasks::{DETAIL_INDENT, fact, millis, note, on_activate, quiet_button};
use super::*;
use crate::motion;

use zeron_rpc::methods::{CHAT_CHECKPOINTS, CREATE_CHAT_CHECKPOINT, ROLLBACK_CHAT};

/// Checkpoints shown before "Show more" pages the rest.
const HISTORY_ROWS: usize = 5;
/// The detail's height before its first paint measures it.
const DETAIL_ESTIMATE: f32 = 84.0;

// ── Model ──────────────────────────────────────────────────────────────

/// One saved point, as the engine reports it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Checkpoint {
    pub id: String,
    #[serde(default)]
    pub message_id: Option<String>,
    pub title: String,
    #[serde(alias = "createdAtMs")]
    pub created_at: i64,
    /// Saved by a restore: where the chat was just before it.
    #[serde(default)]
    pub recovery: bool,
    /// `native` resumes the provider session; `portable` replays a copy.
    #[serde(default)]
    pub provider_strategy: String,
    /// Whether the checkout was captured, so a restore moves files too.
    #[serde(default)]
    pub files_available: bool,
    #[serde(default)]
    pub files_changed: Option<u32>,
    #[serde(default = "restorable_default")]
    pub restorable: bool,
    #[serde(default)]
    pub blocked_reason: Option<String>,
}

fn restorable_default() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RollbackReceipt {
    recovery_checkpoint_id: String,
}

/// What the provider does on restore, in plain words.
pub(super) fn strategy_text(strategy: &str) -> Option<&'static str> {
    match strategy {
        "native" => Some("Resumes the provider session at this point."),
        "portable" => Some("Continues from a copy of the conversation."),
        _ => None,
    }
}

/// What a restore does, said before it happens.
pub(super) fn restore_blurb(checkpoint: &Checkpoint) -> &'static str {
    match (checkpoint.recovery, checkpoint.files_available) {
        (true, true) => {
            "The conversation and the checkout return to where they were before the restore. Where you are now is saved as a recovery point."
        }
        (true, false) => {
            "The conversation returns to where it was before the restore. Where you are now is saved as a recovery point."
        }
        (false, true) => {
            "The conversation and the checkout go back to just after this turn. Where you are now is saved as a recovery point."
        }
        (false, false) => {
            "The conversation goes back to just after this turn; no files were saved here, so the checkout stays as it is. Where you are now is saved as a recovery point."
        }
    }
}

/// Why Restore can't run right now, the engine's word first.
/// What a restore depends on beyond the checkpoint itself.
#[derive(Debug, Clone, Copy)]
struct RestoreGate {
    status: ChatIndicator,
    online: bool,
    /// A managed task under this chat (any depth) still owes it a result.
    tasks_busy: bool,
}

/// `tasks_busy`: a managed task under this chat is still unsettled.
pub(super) fn blocked_reason(
    checkpoint: &Checkpoint,
    status: ChatIndicator,
    online: bool,
    tasks_busy: bool,
) -> Option<String> {
    if !online {
        return Some("Its device is offline.".into());
    }
    if matches!(
        status,
        ChatIndicator::Working | ChatIndicator::AwaitingInput
    ) {
        return Some("Finish or stop the current turn first.".into());
    }
    if tasks_busy {
        return Some("Finish or stop this chat's tasks first.".into());
    }
    (!checkpoint.restorable).then(|| {
        checkpoint
            .blocked_reason
            .clone()
            .unwrap_or_else(|| "This point can't be restored.".into())
    })
}

/// How deep managed delegation goes; the walk stops there.
const TASK_DEPTH: usize = 4;

/// The managed tasks under `chat_id`, at any depth up to [`TASK_DEPTH`],
/// archived ones included (the engine's restore gate counts them).
pub(super) fn descendant_tasks(
    state: &AppState,
    chat_id: &str,
) -> Vec<(String, zeron_proto::DelegatedTask)> {
    let mut found = Vec::new();
    let mut frontier = vec![chat_id.to_owned()];
    for _ in 0..TASK_DEPTH {
        let mut next = Vec::new();
        for chat in &state.chats {
            let Some(parent) = chat.parent_chat_id.as_deref() else {
                continue;
            };
            // Archived tasks count: archiving neither stops their work nor
            // waives an undelivered result.
            if !frontier.iter().any(|id| id == parent) {
                continue;
            }
            if let Some(task) = chat.orchestration.as_ref().and_then(|o| o.task.as_ref()) {
                found.push((chat.id.clone(), task.clone()));
                next.push(chat.id.clone());
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    found
}

/// What decides whether a restore may run — each task's status, delivery and
/// stop — sorted by id. Never its times: streaming must not re-read.
pub(super) fn availability(tasks: &[(String, zeron_proto::DelegatedTask)]) -> String {
    let mut parts: Vec<String> = tasks
        .iter()
        .map(|(id, task)| {
            format!(
                "{id}:{:?}:{}:{}",
                task.status, task.delivered, task.cancel_requested
            )
        })
        .collect();
    parts.sort();
    parts.join(",")
}

/// Whether the chat moved on after a restore, so its Undo would now mean
/// something else: a turn's checkpoint newer than the recovery point that
/// restore kept. Both times are the host's, so a viewer's clock skew can't
/// retire Undo early or keep it forever; until the recovery point is listed,
/// Undo stays.
pub(super) fn receipt_retired(list: &[Checkpoint], recovery: &str) -> bool {
    let Some(kept) = list.iter().find(|c| c.id == recovery) else {
        return false;
    };
    list.iter()
        .any(|c| !c.recovery && c.created_at > kept.created_at)
}

fn ago(ms: i64, now: DateTime<Utc>) -> String {
    match zeron_proto::view::format_time_ago(millis(ms), now).as_str() {
        "now" => "just now".into(),
        ago => format!("{ago} ago"),
    }
}

/// Load state of the open chat's history.
#[derive(Debug, Default)]
enum Load {
    #[default]
    Idle,
    Loading,
    Ready(Vec<Checkpoint>),
    Failed(SharedString),
    /// The host's engine predates checkpoints: the section stays away.
    Unsupported,
}

/// The last restore, for "Restored … Undo".
#[derive(Debug, Clone)]
struct Receipt {
    title: SharedString,
    recovery: String,
    undone: bool,
}

/// History's view state, inside [`LaneState`] so it follows the open chat.
#[derive(Default)]
pub(in crate::shell) struct HistoryState {
    /// The chat, its last message, and its tasks' availability the list
    /// was read at: a finished turn or a task settling reads it again.
    key: Option<(String, Option<DateTime<Utc>>, String)>,
    load: Load,
    /// Ids seen once listed, so a checkpoint arriving later fades in.
    seen: HashSet<String>,
    first_listed: bool,
    open: HashSet<String>,
    confirm: Option<String>,
    restoring: Option<String>,
    /// One request id per checkpoint until it lands, so a retry dedupes.
    requests: HashMap<String, String>,
    errors: HashMap<String, SharedString>,
    saving: bool,
    save_error: Option<SharedString>,
    recovery_open: bool,
    shown: usize,
    receipt: Option<Receipt>,
    receipt_epoch: u64,
}

impl HistoryState {
    fn list(&self) -> &[Checkpoint] {
        match &self.load {
            Load::Ready(list) => list,
            _ => &[],
        }
    }

    fn find(&self, id: &str) -> Option<&Checkpoint> {
        self.list().iter().find(|c| c.id == id)
    }
}

impl Shell {
    /// Read the open chat's checkpoints once per visit and again after each
    /// finished turn (a new one is saved then).
    fn ensure_history(&mut self, chat: &Chat, cx: &mut Context<Self>) {
        let tasks = availability(&descendant_tasks(self.state.read(cx), &chat.id));
        let key = (chat.id.clone(), chat.last_message_at, tasks);
        let history = &mut self.session_lanes.history;
        if history.key.as_ref() == Some(&key)
            || matches!(history.load, Load::Loading | Load::Unsupported)
                && history
                    .key
                    .as_ref()
                    .is_some_and(|(id, _, _)| *id == chat.id)
        {
            return;
        }
        // Checkpoints land at turn end: a running turn's messages don't
        // re-read the list (the read after it finishes picks them up).
        let busy = matches!(
            self.state.read(cx).display_status_for(chat, Utc::now()),
            ChatIndicator::Working | ChatIndicator::AwaitingInput
        );
        let history = &mut self.session_lanes.history;
        if busy && matches!(history.load, Load::Ready(_)) {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let history = &mut self.session_lanes.history;
        history.key = Some(key.clone());
        if !matches!(history.load, Load::Ready(_)) {
            history.load = Load::Loading;
        }
        let params = serde_json::json!({
            "chatId": chat.id,
            "targetDeviceId": chat.device_id,
        });
        cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call_as::<Vec<Checkpoint>>(CHAT_CHECKPOINTS, params)
                .await;
            let _ = this.update(cx, |this, cx| {
                let history = &mut this.session_lanes.history;
                if history.key.as_ref() != Some(&key) {
                    return;
                }
                history.load = match result {
                    Ok(list) => {
                        // The chat moved on after the restore: Undo would
                        // now mean something else, so the receipt retires.
                        if history
                            .receipt
                            .as_ref()
                            .is_some_and(|receipt| receipt_retired(&list, &receipt.recovery))
                        {
                            history.receipt = None;
                        }
                        if !history.first_listed {
                            history.first_listed = true;
                            history.seen.extend(list.iter().map(|c| c.id.clone()));
                        }
                        Load::Ready(list)
                    }
                    Err(zeron_rpc::RpcError::UnknownMethod(_)) => Load::Unsupported,
                    Err(error) if error.to_string().starts_with("unknown method") => {
                        Load::Unsupported
                    }
                    Err(error) => Load::Failed(error.to_string().into()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn reload_history(&mut self, cx: &mut Context<Self>) {
        self.session_lanes.history.key = None;
        cx.notify();
    }

    /// The History section: heading (with Save), the last restore's receipt,
    /// the checkpoints newest first, recovery points folded.
    pub(super) fn render_history(
        &mut self,
        chat: &Chat,
        view: gpui::EntityId,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        self.ensure_history(chat, cx);
        if matches!(self.session_lanes.history.load, Load::Unsupported) {
            return Vec::new();
        }
        let now = Utc::now();
        let gate = {
            let state = self.state.read(cx);
            RestoreGate {
                status: state.display_status_for(chat, now),
                online: state.device_online(&chat.device_id, now),
                // The engine refuses while managed work under this chat is
                // unsettled (any depth); this says so up front.
                tasks_busy: descendant_tasks(state, &chat.id)
                    .iter()
                    .any(|(_, task)| !super::tasks::settled(task)),
            }
        };
        let online = gate.online;

        let mut rows = vec![self.render_history_heading(chat, online, theme, cx)];
        if let Some(error) = self.session_lanes.history.save_error.clone() {
            rows.push(
                div()
                    .px(px(8.0))
                    .pb(px(4.0))
                    .child(note(error, theme.danger))
                    .into_any_element(),
            );
        }
        rows.extend(self.render_receipt(chat, view, theme, cx));

        let list = match &self.session_lanes.history.load {
            Load::Idle | Load::Loading => {
                rows.push(
                    popover::picker_row(theme, false, false, "session-lanes-history-loading")
                        .cursor_default()
                        .child(popover::picker_icon_slot(
                            loaders::mini_mono_spinner(
                                "session-lanes-history-loading",
                                1.5,
                                theme.text_muted,
                                view,
                                cx,
                            )
                            .into_any_element(),
                        ))
                        .child(div().text_color(theme.text_muted).child("Loading history…"))
                        .into_any_element(),
                );
                return rows;
            }
            Load::Failed(error) => {
                let error = error.clone();
                rows.push(
                    div()
                        .px(px(8.0))
                        .pb(px(2.0))
                        .child(note(
                            format!("Couldn't load history. {error}"),
                            theme.danger,
                        ))
                        .into_any_element(),
                );
                let accent = theme.accent;
                rows.push(
                    on_activate(
                        popover::picker_row(theme, false, false, "session-lanes-history-retry")
                            .id("session-lanes-history-retry")
                            .role(gpui::Role::Button)
                            .tab_index(0)
                            .border_1()
                            .border_color(gpui::transparent_black())
                            .px(px(7.0))
                            .focus_visible(move |style| style.border_color(accent))
                            .child(popover::picker_icon_slot(
                                icon(icons::REFRESH)
                                    .size(px(14.0))
                                    .text_color(theme.text_muted)
                                    .into_any_element(),
                            ))
                            .child("Retry"),
                        cx,
                        |this, cx| {
                            this.session_lanes.history.load = Load::Idle;
                            this.reload_history(cx);
                        },
                    )
                    .into_any_element(),
                );
                return rows;
            }
            Load::Ready(list) => list.clone(),
            Load::Unsupported => return Vec::new(),
        };
        let (recovery, points): (Vec<_>, Vec<_>) = list.into_iter().partition(|c| c.recovery);
        if points.is_empty() && recovery.is_empty() {
            rows.push(
                div()
                    .px(px(8.0))
                    .py(px(6.0))
                    .child(note(
                        "A checkpoint is saved after each finished turn.",
                        theme.text_muted,
                    ))
                    .into_any_element(),
            );
            return rows;
        }
        let shown = HISTORY_ROWS.max(self.session_lanes.history.shown);
        let hidden = points.len().saturating_sub(shown);
        for checkpoint in points.into_iter().take(shown) {
            rows.push(self.render_checkpoint(checkpoint, chat, gate, view, theme, cx));
        }
        if hidden > 0 {
            rows.push(self.render_show_more(
                "session-lanes-history-more",
                hidden,
                theme,
                cx,
                |lanes| {
                    lanes.history.shown = HISTORY_ROWS.max(lanes.history.shown) + PAGE_ROWS;
                },
            ));
        }
        if !recovery.is_empty() {
            let open = self.session_lanes.history.recovery_open;
            rows.push(self.render_recovery_fold(recovery.len(), open, view, theme, cx));
            if open {
                for checkpoint in recovery {
                    rows.push(self.render_checkpoint(checkpoint, chat, gate, view, theme, cx));
                }
            }
        }
        rows
    }

    fn render_history_heading(
        &mut self,
        chat: &Chat,
        online: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let saving = self.session_lanes.history.saving;
        let accent = theme.accent;
        let enabled = online && !saving;
        let save = div()
            .id("session-lanes-history-save")
            .debug_selector(|| "session-lanes-history-save".into())
            .role(gpui::Role::Button)
            .aria_label("Save a checkpoint now")
            .size(px(22.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .border_1()
            .border_color(gpui::transparent_black())
            .tooltip(crate::settings::widgets::text_tooltip(if online {
                "Save a checkpoint now"
            } else {
                "Its device is offline"
            }))
            .when(!enabled, |button| button.opacity(0.5))
            .when(enabled, |button| {
                button
                    .tab_index(0)
                    .cursor_pointer()
                    .hover(|style| style.bg(crate::theme::ink(0.08)))
                    .focus_visible(move |style| style.border_color(accent))
            })
            .child(if saving {
                loaders::mini_mono_spinner(
                    "session-lanes-history-saving",
                    1.5,
                    theme.text_muted,
                    cx.entity_id(),
                    cx,
                )
                .into_any_element()
            } else {
                icon(icons::PLUS)
                    .size(px(14.0))
                    .text_color(theme.text_muted)
                    .into_any_element()
            });
        let save = if enabled {
            let chat = chat.clone();
            on_activate(save, cx, move |this, cx| this.save_checkpoint(&chat, cx))
                .into_any_element()
        } else {
            save.into_any_element()
        };
        div()
            .flex()
            .items_center()
            .pr(px(2.0))
            .child(
                div()
                    .flex_1()
                    .child(popover::menu_heading(theme, "History")),
            )
            .child(save)
            .into_any_element()
    }

    /// "Restored to … [Undo]", easing in once; Undo restores the recovery
    /// point the restore kept (which keeps one of its own).
    fn render_receipt(
        &mut self,
        chat: &Chat,
        view: gpui::EntityId,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let history = &self.session_lanes.history;
        let receipt = history.receipt.clone()?;
        let epoch = history.receipt_epoch;
        let restoring = history.restoring.as_deref() == Some(receipt.recovery.as_str());
        let error = history.errors.get(&receipt.recovery).cloned();
        let text = if receipt.undone {
            "Undone. The restore is kept as a recovery point.".to_owned()
        } else {
            format!("Restored to \u{201c}{}\u{201d}.", receipt.title)
        };
        let mut line = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(
                icon(icons::CHECK)
                    .size(px(11.0))
                    .flex_none()
                    .text_color(theme.success),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(crate::typography::ui_rems(12.0))
                    .line_height(px(16.0))
                    .text_color(theme.text)
                    .child(SharedString::from(text)),
            );
        if !receipt.undone {
            let undo = quiet_button(
                "session-lanes-history-undo".into(),
                if restoring { "Undoing…" } else { "Undo" }.into(),
                theme.accent,
                !restoring,
                theme,
            );
            line = line.child(if restoring {
                undo.into_any_element()
            } else {
                let chat = chat.clone();
                let recovery = receipt.recovery.clone();
                on_activate(undo, cx, move |this, cx| {
                    this.rollback(&chat, recovery.clone(), true, cx)
                })
                .into_any_element()
            });
        }
        let reduced = motion::reduced_motion(cx);
        let card = div()
            .id(SharedString::from(format!(
                "session-lanes-history-receipt-{epoch}"
            )))
            .mx(px(4.0))
            .mb(px(4.0))
            .px(px(8.0))
            .py(px(6.0))
            .rounded(px(popover::MENU_ITEM_RADIUS))
            .bg(theme.success.opacity(0.08))
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(line)
            .children(error.map(|text| note(text, theme.danger)));
        let _ = view;
        Some(if reduced {
            card.into_any_element()
        } else {
            card.with_animation(
                SharedString::from(format!("session-lanes-history-receipt-in-{epoch}")),
                motion::MENU_IN.animation(),
                |el, t| el.opacity(t).relative().top(px(-3.0 * (1.0 - t))),
            )
            .into_any_element()
        })
    }

    fn render_recovery_fold(
        &mut self,
        count: usize,
        open: bool,
        view: gpui::EntityId,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let turn = motion::state_t(
            &format!("lanes-history-recovery-{view}"),
            open,
            motion::COLLAPSE,
            motion::reduced_motion(cx),
        );
        let accent = theme.accent;
        let label = format!(
            "{count} recovery point{}",
            if count == 1 { "" } else { "s" }
        );
        on_activate(
            popover::picker_row(theme, false, false, "session-lanes-history-recovery")
                .id("session-lanes-history-recovery")
                .role(gpui::Role::Button)
                .aria_label(SharedString::from(label.clone()))
                .aria_expanded(open)
                .tab_index(0)
                .border_1()
                .border_color(gpui::transparent_black())
                .px(px(7.0))
                .focus_visible(move |style| style.border_color(accent))
                .child(popover::picker_icon_slot(
                    icon(icons::ALT_ARROW_RIGHT)
                        .size(px(12.0))
                        .text_color(theme.text_muted)
                        .with_transformation(gpui::Transformation::rotate(gpui::percentage(
                            0.25 * turn,
                        )))
                        .into_any_element(),
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(theme.text_muted)
                        .child(SharedString::from(label)),
                ),
            cx,
            |this, cx| {
                let history = &mut this.session_lanes.history;
                history.recovery_open = !history.recovery_open;
                cx.notify();
            },
        )
        .into_any_element()
    }

    /// One checkpoint: its row (expand to inspect; hover reveals Restore)
    /// and, while open, its detail with the restore confirm in place.
    #[allow(clippy::too_many_arguments)]
    fn render_checkpoint(
        &mut self,
        checkpoint: Checkpoint,
        chat: &Chat,
        gate: RestoreGate,
        view: gpui::EntityId,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = format!("session-lanes-checkpoint-{}", checkpoint.id);
        let group = SharedString::from(format!("{key}-group"));
        let reduced = motion::reduced_motion(cx);
        let history = &mut self.session_lanes.history;
        let open = history.open.contains(&checkpoint.id)
            || history.confirm.as_deref() == Some(checkpoint.id.as_str());
        // A checkpoint that arrives while the card is up fades in once.
        let arriving = history.seen.insert(checkpoint.id.clone()) && !reduced;
        let reveal = motion::state_t(
            &format!("lanes-checkpoint-{view}-{}", checkpoint.id),
            open,
            motion::COLLAPSE,
            reduced,
        );
        let blocked = blocked_reason(&checkpoint, gate.status, gate.online, gate.tasks_busy);
        let accent = theme.accent;

        // Hover or focus reveals Restore beside the time.
        let restore_id = checkpoint.id.clone();
        let restore_tile = div()
            .id(SharedString::from(format!("{key}-restore")))
            .role(gpui::Role::Button)
            .aria_label("Restore to here")
            .absolute()
            .right_0()
            .top(px(-2.0))
            .size(px(18.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .bg(theme.ink(0.06))
            .invisible()
            .group_hover(group.clone(), |style| style.visible())
            .tooltip(crate::settings::widgets::text_tooltip(
                blocked.clone().unwrap_or_else(|| "Restore to here".into()),
            ))
            .when(blocked.is_some(), |tile| tile.opacity(0.5))
            .child(
                icon(icons::RESTART)
                    .size(px(11.0))
                    .text_color(theme.text_muted.opacity(0.8)),
            );
        let restore_tile = if blocked.is_some() {
            restore_tile.into_any_element()
        } else {
            on_activate(restore_tile, cx, move |this, cx| {
                let history = &mut this.session_lanes.history;
                history.confirm = Some(restore_id.clone());
                history.errors.remove(&restore_id);
                cx.notify();
            })
            .into_any_element()
        };
        let trailing = div()
            .relative()
            .flex_none()
            .min_w(px(18.0))
            .flex()
            .justify_end()
            .child(
                time_label(
                    zeron_proto::view::format_time_ago(millis(checkpoint.created_at), Utc::now())
                        .into(),
                    theme,
                )
                .id(SharedString::from(format!("{key}-time")))
                .group_hover(group.clone(), |style| style.invisible()),
            )
            .child(restore_tile);

        let toggle_id = checkpoint.id.clone();
        let header = on_activate(
            popover::picker_row(theme, false, false, key.clone())
                .id(SharedString::from(key.clone()))
                .group(group)
                .role(gpui::Role::Button)
                .aria_label(SharedString::from(format!(
                    "{}, saved {}",
                    checkpoint.title,
                    ago(checkpoint.created_at, Utc::now())
                )))
                .aria_expanded(open)
                .tab_index(0)
                .border_1()
                .border_color(gpui::transparent_black())
                .px(px(7.0))
                .focus_visible(move |style| style.border_color(accent))
                .child(popover::picker_icon_slot(
                    icon(if checkpoint.recovery {
                        icons::RESET
                    } else {
                        icons::CLOCK_CIRCLE
                    })
                    .size(px(13.0))
                    .text_color(theme.text_muted)
                    .into_any_element(),
                ))
                .child(row_title(
                    format!("{key}-title"),
                    checkpoint.title.clone().into(),
                ))
                .child(trailing),
            cx,
            move |this, cx| {
                let history = &mut this.session_lanes.history;
                if !history.open.remove(&toggle_id) {
                    history.open.insert(toggle_id.clone());
                } else if history.confirm.as_deref() == Some(toggle_id.as_str()) {
                    history.confirm = None;
                }
                cx.notify();
            },
        );

        let detail = (reveal > 0.001).then(|| {
            let content = self.render_checkpoint_detail(&checkpoint, chat, blocked, theme, cx);
            self.lane_disclosure(
                format!("checkpoint:{}", checkpoint.id),
                reveal,
                DETAIL_ESTIMATE,
                content,
                cx,
            )
        });
        let plate = div()
            .id(SharedString::from(format!("{key}-plate")))
            .w_full()
            .flex()
            .flex_col()
            .rounded(px(popover::MENU_ITEM_RADIUS))
            .bg(theme.ink(0.035 * reveal))
            .child(header)
            .children(detail);
        if arriving {
            plate
                .with_animation(
                    SharedString::from(format!("{key}-arrive")),
                    motion::FADE_IN.animation(),
                    |el, t| el.opacity(t).relative().top(px(4.0 * (1.0 - t))),
                )
                .into_any_element()
        } else {
            plate.into_any_element()
        }
    }

    fn render_checkpoint_detail(
        &mut self,
        checkpoint: &Checkpoint,
        chat: &Chat,
        blocked: Option<String>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let key = format!("session-lanes-checkpoint-{}", checkpoint.id);
        let history = &self.session_lanes.history;
        let confirming = history.confirm.as_deref() == Some(checkpoint.id.as_str());
        let restoring = history.restoring.as_deref() == Some(checkpoint.id.as_str());
        let busy_elsewhere = history.restoring.is_some() && !restoring;
        let error = history.errors.get(&checkpoint.id).cloned();

        let saved = div()
            .text_size(crate::typography::ui_rems(12.0))
            .text_color(theme.text_faint)
            .child(SharedString::from(if checkpoint.recovery {
                format!(
                    "Kept {}, before a restore",
                    ago(checkpoint.created_at, Utc::now())
                )
            } else {
                format!("Saved {}", ago(checkpoint.created_at, Utc::now()))
            }));
        let mut facts: Vec<SharedString> = Vec::new();
        facts.push(if checkpoint.files_available {
            match checkpoint.files_changed {
                Some(0) => "Checkout unchanged since".into(),
                Some(1) => "1 file differs".into(),
                Some(n) => format!("{n} files differ").into(),
                None => "Checkout saved".into(),
            }
        } else {
            "Conversation only".into()
        });
        match checkpoint.provider_strategy.as_str() {
            "native" => facts.push("Native resume".into()),
            "portable" => facts.push("Portable".into()),
            _ => {}
        }
        let facts = div()
            .flex()
            .flex_wrap()
            .gap(px(4.0))
            .children(facts.into_iter().map(|label| fact(label, theme)));
        let strategy =
            strategy_text(&checkpoint.provider_strategy).map(|text| note(text, theme.text_muted));

        let mut column = div()
            .w_full()
            .flex_none()
            .pl(px(DETAIL_INDENT))
            .pr(px(8.0))
            .pt(px(2.0))
            .pb(px(8.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(saved)
            .child(facts)
            .children(strategy);

        if confirming && blocked.is_none() {
            // The question says what moves and what is kept, then the row
            // holds only the answer — the safe one keeps focus.
            column = column.child(note(restore_blurb(checkpoint), theme.text));
            let keep = on_activate(
                quiet_button(
                    format!("{key}-restore-button").into(),
                    "Keep current".into(),
                    theme.text,
                    true,
                    theme,
                ),
                cx,
                |this, cx| {
                    this.session_lanes.history.confirm = None;
                    cx.notify();
                },
            );
            let chat = chat.clone();
            let id = checkpoint.id.clone();
            let confirm = on_activate(
                quiet_button(
                    format!("{key}-restore-confirm").into(),
                    "Restore".into(),
                    theme.accent,
                    true,
                    theme,
                ),
                cx,
                move |this, cx| this.rollback(&chat, id.clone(), false, cx),
            );
            column = column.child(
                div()
                    .id(SharedString::from(format!("{key}-confirm-row")))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                        if event.keystroke.key == "escape" {
                            cx.stop_propagation();
                            this.session_lanes.history.confirm = None;
                            cx.notify();
                        }
                    }))
                    .child(div().flex_1())
                    .child(keep)
                    .child(confirm),
            );
            return column.children(error.map(|text| note(text, theme.danger)));
        }

        if let Some(reason) = blocked.clone() {
            column = column.child(note(reason, theme.warning_muted));
        }
        column = column.children(error.clone().map(|text| note(text, theme.danger)));
        let label: SharedString = if restoring {
            "Restoring…".into()
        } else if error.is_some() {
            "Try again".into()
        } else if checkpoint.recovery {
            "Restore this point".into()
        } else {
            "Restore to here".into()
        };
        let enabled = blocked.is_none() && !restoring && !busy_elsewhere;
        let button = quiet_button(
            format!("{key}-restore-button").into(),
            label,
            theme.text,
            enabled,
            theme,
        )
        .when(restoring, |button| {
            button.child(loaders::mini_mono_spinner(
                format!("{key}-restoring"),
                1.5,
                theme.text_muted,
                cx.entity_id(),
                cx,
            ))
        });
        let button = if enabled {
            let id = checkpoint.id.clone();
            on_activate(button, cx, move |this, cx| {
                let history = &mut this.session_lanes.history;
                history.confirm = Some(id.clone());
                cx.notify();
            })
            .into_any_element()
        } else {
            button.into_any_element()
        };
        column.child(div().flex().items_center().child(button))
    }

    /// Restore `checkpoint`. The engine keeps where the chat is now as a
    /// recovery point and returns it, which Undo restores in turn.
    fn rollback(&mut self, chat: &Chat, checkpoint: String, undo: bool, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let history = &mut self.session_lanes.history;
        if history.restoring.is_some() {
            return;
        }
        let title: SharedString = history
            .find(&checkpoint)
            .map(|c| c.title.clone())
            .unwrap_or_default()
            .into();
        history.confirm = None;
        history.errors.remove(&checkpoint);
        history.restoring = Some(checkpoint.clone());
        let request = history
            .requests
            .entry(checkpoint.clone())
            .or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        let params = serde_json::json!({
            "chatId": chat.id,
            "checkpointId": checkpoint,
            "clientRequestId": request,
            "targetDeviceId": chat.device_id,
        });
        let origin = chat.id.clone();
        cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call_as::<RollbackReceipt>(ROLLBACK_CHAT, params)
                .await;
            let _ = this.update(cx, |this, cx| {
                // Another chat opened meanwhile: its History is not this
                // restore's (an Undo there would target the wrong chat).
                if this.session_lanes.chat_id != origin {
                    return;
                }
                let history = &mut this.session_lanes.history;
                history.restoring = None;
                match result {
                    Ok(receipt) => {
                        history.requests.remove(&checkpoint);
                        history.open.remove(&checkpoint);
                        history.receipt_epoch += 1;
                        history.receipt = Some(match (undo, history.receipt.take()) {
                            (true, Some(previous)) => Receipt {
                                undone: true,
                                recovery: receipt.recovery_checkpoint_id,
                                ..previous
                            },
                            _ => Receipt {
                                title,
                                recovery: receipt.recovery_checkpoint_id,
                                undone: false,
                            },
                        });
                        this.reload_history(cx);
                    }
                    Err(error) => {
                        history
                            .errors
                            .insert(checkpoint, format!("Couldn't restore. {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn save_checkpoint(&mut self, chat: &Chat, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let history = &mut self.session_lanes.history;
        if history.saving {
            return;
        }
        history.saving = true;
        history.save_error = None;
        let params = serde_json::json!({
            "chatId": chat.id,
            "targetDeviceId": chat.device_id,
        });
        let origin = chat.id.clone();
        cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call_as::<Checkpoint>(CREATE_CHAT_CHECKPOINT, params)
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.session_lanes.chat_id != origin {
                    return;
                }
                let history = &mut this.session_lanes.history;
                history.saving = false;
                match result {
                    Ok(_) => this.reload_history(cx),
                    Err(error) => {
                        history.save_error =
                            Some(format!("Couldn't save a checkpoint. {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

/// Native task-lanes fixture only: stage History at once — its checkpoints,
/// which are open, a pending restore confirm, a restore error, the last
/// restore's receipt, and the recovery fold.
#[cfg(feature = "appshots-fixture")]
impl Shell {
    pub fn fixture_history(
        &mut self,
        checkpoints: serde_json::Value,
        open: &[&str],
        confirm: Option<&str>,
        error: Option<(&str, &str)>,
        receipt: Option<(&str, &str)>,
        recovery_open: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.state.read(cx).selected_chat_row().cloned() else {
            return;
        };
        self.session_lanes.follow(&chat.id);
        let history = &mut self.session_lanes.history;
        let list: Vec<Checkpoint> = serde_json::from_value(checkpoints).unwrap_or_default();
        history.seen.extend(list.iter().map(|c| c.id.clone()));
        history.first_listed = true;
        history.load = Load::Ready(list);
        let tasks = availability(&descendant_tasks(self.state.read(cx), &chat.id));
        let history = &mut self.session_lanes.history;
        history.key = Some((chat.id.clone(), chat.last_message_at, tasks));
        history.open = open.iter().map(|id| (*id).to_owned()).collect();
        history.confirm = confirm.map(str::to_owned);
        history.errors.clear();
        if let Some((id, message)) = error {
            history
                .errors
                .insert(id.to_owned(), message.to_owned().into());
        }
        history.receipt = receipt.map(|(title, recovery)| Receipt {
            title: title.to_owned().into(),
            recovery: recovery.to_owned(),
            undone: false,
        });
        history.recovery_open = recovery_open;
        self.show_session_info(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkpoint(json: serde_json::Value) -> Checkpoint {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn checkpoints_read_the_agreed_wire_shape() {
        let c = checkpoint(serde_json::json!({
            "id": "c1", "messageId": "m1", "title": "Wire the lanes card",
            "createdAtMs": 5, "recovery": false, "providerStrategy": "native",
            "filesAvailable": true, "filesChanged": 12, "restorable": false,
            "blockedReason": "Stop tasks working in this checkout first",
        }));
        assert_eq!(c.created_at, 5);
        assert_eq!(c.files_changed, Some(12));
        assert_eq!(
            blocked_reason(&c, ChatIndicator::Idle, true, false).as_deref(),
            Some("Stop tasks working in this checkout first")
        );
        // Older hosts that omit the new fields still read as restorable.
        let old = checkpoint(serde_json::json!({"id": "c2", "title": "t", "createdAt": 1}));
        assert!(old.restorable && !old.files_available);
        assert_eq!(blocked_reason(&old, ChatIndicator::Idle, true, false), None);
    }

    #[test]
    fn restore_is_blocked_truthfully() {
        let c = checkpoint(serde_json::json!({"id": "c", "title": "t", "createdAt": 1}));
        assert_eq!(
            blocked_reason(&c, ChatIndicator::Working, true, false).as_deref(),
            Some("Finish or stop the current turn first.")
        );
        assert_eq!(
            blocked_reason(&c, ChatIndicator::AwaitingInput, true, false).as_deref(),
            Some("Finish or stop the current turn first.")
        );
        assert_eq!(
            blocked_reason(&c, ChatIndicator::Idle, false, false).as_deref(),
            Some("Its device is offline.")
        );
        assert_eq!(
            blocked_reason(&c, ChatIndicator::Completed, true, false),
            None
        );
        // Unsettled managed work blocks before the engine is asked.
        assert_eq!(
            blocked_reason(&c, ChatIndicator::Idle, true, true).as_deref(),
            Some("Finish or stop this chat's tasks first.")
        );
    }

    #[test]
    fn nested_task_settling_changes_the_availability_key() {
        let task = |parent: &str, status: &str, extra: serde_json::Value| {
            let mut t = serde_json::json!({
                "id": "t", "parentChatId": parent, "role": "general", "isolation": "worktree",
                "status": status, "prompt": "", "requestFingerprint": "f",
                "createdAt": 0, "updatedAt": 0,
            });
            for (k, v) in extra.as_object().unwrap() {
                t[k] = v.clone();
            }
            t
        };
        let chat = |id: &str, parent: Option<&str>, task: Option<serde_json::Value>| -> Chat {
            serde_json::from_value(serde_json::json!({
                "id": id, "deviceId": "dev", "archived": false, "createdAt": Utc::now(),
                "parentChatId": parent, "orchestration": task.map(|t| serde_json::json!({"task": t})),
            }))
            .unwrap()
        };
        let mut state = AppState::new();
        // Cancelled parent task, grandchild still running.
        state.apply_chats(vec![
            chat("main", None, None),
            chat(
                "a",
                Some("main"),
                Some(task(
                    "main",
                    "cancelled",
                    serde_json::json!({"cancelRequested": true}),
                )),
            ),
            chat(
                "b",
                Some("a"),
                Some(task("a", "running", serde_json::json!({}))),
            ),
        ]);
        let before = descendant_tasks(&state, "main");
        assert_eq!(before.len(), 2, "the grandchild counts");
        assert!(
            before
                .iter()
                .any(|(_, t)| !crate::shell::session_lanes::tasks::settled(t))
        );
        let key = availability(&before);
        // Its timestamps moving changes nothing; its settling does.
        let mut later = before.clone();
        later[1].1.updated_at = 99;
        assert_eq!(availability(&later), key);
        later[1].1.status = zeron_proto::TaskStatus::Cancelled;
        later[1].1.cancel_requested = true;
        assert_ne!(availability(&later), key);
    }

    #[test]
    fn undo_retires_on_the_hosts_clock_not_the_viewers() {
        let point = |id: &str, at: i64, recovery: bool| {
            checkpoint(serde_json::json!({
                "id": id, "title": id, "createdAtMs": at, "recovery": recovery,
            }))
        };
        // The host's clock runs far behind the viewer's: nothing here is
        // newer than "now" on the viewer, yet the turn after the restore is.
        let list = vec![point("old", 100, false), point("kept", 200, true)];
        assert!(!receipt_retired(&list, "kept"), "no turn since the restore");
        let moved_on = vec![
            point("next", 300, false),
            point("old", 100, false),
            point("kept", 200, true),
        ];
        assert!(receipt_retired(&moved_on, "kept"));
        // Until the recovery point is listed, Undo stays.
        assert!(!receipt_retired(&moved_on, "not-listed-yet"));
        // Other recovery points never retire it.
        let restores = vec![point("again", 400, true), point("kept", 200, true)];
        assert!(!receipt_retired(&restores, "kept"));
    }

    #[test]
    fn the_confirm_says_what_moves_and_what_is_kept() {
        let mut c = checkpoint(serde_json::json!({"id": "c", "title": "t", "createdAt": 1}));
        assert!(restore_blurb(&c).contains("checkout stays as it is"));
        c.files_available = true;
        assert!(restore_blurb(&c).contains("conversation and the checkout go back"));
        c.recovery = true;
        assert!(restore_blurb(&c).contains("before the restore"));
        assert!(restore_blurb(&c).ends_with("saved as a recovery point."));
        assert_eq!(
            strategy_text("native"),
            Some("Resumes the provider session at this point.")
        );
        assert_eq!(
            strategy_text("portable"),
            Some("Continues from a copy of the conversation.")
        );
        assert_eq!(strategy_text("other"), None);
    }
}
