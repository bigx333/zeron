//! Delegated tasks in the lanes card: the chats this one handed work to,
//! with their true status, role and checkout, the result each reported, and
//! whether that result was sent back here. A row expands in place to
//! inspect, and a running task stops from there.
//!
//! Also here: the lineage row that leads back up (a task's delegating chat,
//! a fork's source) and a fork's merge-back strip.
//!
//! Every status word comes from the task the engine wrote
//! (`Chat.orchestration`); the only local state is a request in flight,
//! until the next chats frame confirms it.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;

use zeron_proto::{
    Chat, ChatIndicator, ContextTransfer, DelegatedTask, TaskIsolation, TaskRole, TaskStatus,
};

use crate::motion::{self, MotionSpec};

use super::*;

/// The detail's height before its first paint measures it.
const DETAIL_ESTIMATE: f32 = 92.0;
/// The detail lines up with the row's title: row inset, icon slot, gap.
pub(super) const DETAIL_INDENT: f32 = 32.0;
/// The one unprompted moment: a task reaching its end washes its row once.
pub(super) const SETTLE: MotionSpec = MotionSpec::new(1100, motion::EASE_OUT_SOFT);
/// How long a mark swap takes when a task's status moves.
const MARK_SWAP: MotionSpec = motion::GLYPH_STATE;
/// Summary lines shown inline; the task's own transcript holds the rest.
const SUMMARY_LINES: usize = 4;

// ── Model ──────────────────────────────────────────────────────────────

/// How a task's status reads at a glance — its mark in the icon slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mark {
    Queued,
    Working,
    WaitingOnTasks,
    NeedsInput,
    Stopping,
    /// Completed, result not yet sent to the parent.
    Ready,
    /// Completed and its result sent.
    Sent,
    Failed,
    Cancelled,
    Interrupted,
}

impl Mark {
    fn terminal(self) -> bool {
        matches!(
            self,
            Self::Ready | Self::Sent | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}

/// `stopping`: a stop this window asked for that no chats frame has
/// confirmed yet.
pub(super) fn mark_for(task: &DelegatedTask, stopping: bool) -> Mark {
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

/// The status word in the detail.
pub(super) fn status_text(task: &DelegatedTask, stopping: bool) -> &'static str {
    match mark_for(task, stopping) {
        Mark::Stopping => "Stopping…",
        _ => task.status.label(),
    }
}

/// Where the result stands, once there is one. `delivered` means the
/// parent's result command applied — it may still be held for the parent's
/// turn boundary, so it reads "sent", never "read".
pub(super) fn delivery_text(task: &DelegatedTask) -> Option<&'static str> {
    if !task.status.is_terminal() {
        return None;
    }
    Some(if task.delivered {
        "Sent to this chat"
    } else if task.cancel_requested {
        // A stopped task's result is deliberately not delivered.
        "Stopped from this chat"
    } else {
        "Result ready, not sent yet"
    })
}

/// Ended with nothing left owed here: sent, or stopped (a stop suppresses
/// delivery).
pub(super) fn settled(task: &DelegatedTask) -> bool {
    task.status.is_terminal() && (task.delivered || task.cancel_requested)
}

/// What the detail says when the status alone leaves a question open.
pub(super) fn outcome_note(task: &DelegatedTask, stopping: bool) -> Option<&'static str> {
    let has_summary = task
        .summary
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty());
    match mark_for(task, stopping) {
        Mark::Queued => Some("Waiting to start."),
        Mark::NeedsInput => Some("It's waiting on an answer. Open it to reply."),
        Mark::WaitingOnTasks => Some("Waiting for its own tasks to finish."),
        Mark::Stopping => Some("Stopping its current turn."),
        Mark::Failed if !has_summary => Some("It stopped without a result."),
        Mark::Interrupted if !has_summary => Some("It was interrupted before finishing."),
        Mark::Cancelled if !has_summary => Some("Cancelled before it reported a result."),
        Mark::Ready | Mark::Sent if !has_summary => Some("Finished without a summary."),
        _ => None,
    }
}

pub(super) fn role_label(role: TaskRole) -> Option<&'static str> {
    match role {
        TaskRole::Implementation => Some("Implementation"),
        TaskRole::Research => Some("Research"),
        TaskRole::Review => Some("Review"),
        TaskRole::Design => Some("Design"),
        TaskRole::Test => Some("Test"),
        TaskRole::General => None,
    }
}

pub(super) fn isolation_label(isolation: TaskIsolation) -> &'static str {
    match isolation {
        TaskIsolation::Shared => "Shared checkout",
        TaskIsolation::Worktree => "Own worktree",
    }
}

pub(super) fn millis(ms: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or_else(Utc::now)
}

/// "for 4m" while it runs, "4m ago" once it ended.
pub(super) fn elapsed_phrase(task: &DelegatedTask, now: DateTime<Utc>) -> String {
    if task.status.is_terminal() {
        match zeron_proto::view::format_time_ago(millis(task.updated_at), now).as_str() {
            "now" => "just now".into(),
            ago => format!("{ago} ago"),
        }
    } else {
        match zeron_proto::view::format_time_ago(millis(task.created_at), now).as_str() {
            "now" => "just started".into(),
            ago => format!("for {ago}"),
        }
    }
}

/// A delegated task of the open chat, as its lane lists it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TaskRow {
    pub chat_id: String,
    pub device_id: String,
    pub title: SharedString,
    pub task: DelegatedTask,
    pub harness: Option<zeron_proto::HarnessId>,
    pub model: Option<SharedString>,
    /// Whether the task's host is reachable — a stop has to land there.
    pub online: bool,
    /// Its own live tasks: a task can delegate in turn.
    pub tasks: usize,
}

fn task_of(chat: &Chat) -> Option<&DelegatedTask> {
    chat.orchestration.as_ref()?.task.as_ref()
}

pub(super) fn is_task(chat: &Chat) -> bool {
    task_of(chat).is_some()
}

/// A task names itself from its title, else its brief's first line.
fn task_title(chat: &Chat, task: &DelegatedTask) -> String {
    chat.title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .or_else(|| {
            let line = task.prompt.lines().find(|l| !l.trim().is_empty())?.trim();
            let mut out: String = line.chars().take(60).collect();
            if line.chars().count() > 60 {
                out.push('…');
            }
            Some(out)
        })
        .unwrap_or_else(|| "Task".into())
}

/// The live tasks `chat_id` delegated.
pub(super) fn task_rows(state: &AppState, chat_id: &str, now: DateTime<Utc>) -> Vec<TaskRow> {
    let live_tasks_of = |id: &str| {
        state
            .chats
            .iter()
            .filter(|c| !c.archived && c.parent_chat_id.as_deref() == Some(id) && is_task(c))
            .count()
    };
    state
        .chats
        .iter()
        .filter(|chat| !chat.archived && chat.parent_chat_id.as_deref() == Some(chat_id))
        .filter_map(|chat| {
            let task = task_of(chat)?;
            Some(TaskRow {
                chat_id: chat.id.clone(),
                device_id: chat.device_id.clone(),
                title: task_title(chat, task).into(),
                task: task.clone(),
                harness: chat.config.as_ref().map(|config| config.harness),
                model: chat
                    .config
                    .as_ref()
                    .and_then(|config| config.model.clone())
                    .map(SharedString::from),
                online: state.device_online(&chat.device_id, now),
                tasks: live_tasks_of(&chat.id),
            })
        })
        .collect()
}

/// The task lane: what still owes this chat something rides it, the rest
/// folds away.
#[derive(Debug, Default, PartialEq)]
pub(super) struct TaskLane {
    /// Unfinished, or finished with the result not sent yet; oldest first,
    /// so the longest-running leads and new tasks join at the foot.
    pub active: Vec<TaskRow>,
    /// Ended and sent (or stopped), most recently updated first.
    pub finished: Vec<TaskRow>,
}

impl TaskLane {
    pub fn new(rows: Vec<TaskRow>) -> Self {
        let (mut finished, mut active): (Vec<_>, Vec<_>) =
            rows.into_iter().partition(|row| settled(&row.task));
        active.sort_by_key(|row| (row.task.created_at, row.chat_id.clone()));
        finished.sort_by_key(|row| std::cmp::Reverse(row.task.updated_at));
        Self { active, finished }
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty() && self.finished.is_empty()
    }
}

/// Which way the open chat came from, for the row that leads back up.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Lineage {
    pub label: &'static str,
    pub target: String,
    /// `None` when the chat it came from is gone.
    pub title: Option<SharedString>,
}

pub(super) fn lineage_for(state: &AppState, chat: &Chat) -> Option<Lineage> {
    let orchestration = chat.orchestration.as_ref()?;
    let (label, target) = if let Some(task) = &orchestration.task {
        ("Delegated by", task.parent_chat_id.clone())
    } else {
        (
            "Forked from",
            orchestration.fork.as_ref()?.source_chat_id.clone(),
        )
    };
    let title = state
        .chats
        .iter()
        .find(|c| c.id == target && !c.archived)
        .map(|source| match task_of(source) {
            Some(task) => task_title(source, task),
            None => child_chat_title(source),
        })
        .map(SharedString::from);
    Some(Lineage {
        label,
        target,
        title,
    })
}

/// The latest merge of `fork` into the chat whose transfers these are.
pub(super) fn latest_merge<'a>(
    transfers: &'a [ContextTransfer],
    fork: &str,
) -> Option<&'a ContextTransfer> {
    transfers
        .iter()
        .filter(|t| t.kind == "merge" && t.source_chat_id == fork)
        .max_by_key(|t| t.created_at)
}

// ── View state ─────────────────────────────────────────────────────────

/// A task's last seen mark, for the swap and settle when it moves.
#[derive(Debug, Clone, Copy)]
struct Seen {
    mark: Mark,
    epoch: u64,
    changed: Option<(Mark, Instant)>,
    settled: Option<Instant>,
}

/// What the task lane has unfolded, measured, or has in flight. Lives in
/// [`LaneState`], so it follows the open chat.
#[derive(Default)]
pub(in crate::shell) struct TaskLaneState {
    open: HashSet<String>,
    heights: HashMap<String, f32>,
    seen: HashMap<String, Seen>,
    /// The task whose Stop waits for its confirm.
    confirm: Option<String>,
    /// Stops in flight, then asked for until a frame shows them.
    stopping: HashSet<String>,
    requested: HashSet<String>,
    errors: HashMap<String, SharedString>,
    finished_open: bool,
    shown_finished: usize,
    /// The fork whose merge strip is open.
    merge_open: Option<String>,
    merging: HashSet<String>,
    merged: HashSet<String>,
    merge_errors: HashMap<String, SharedString>,
    /// The open chat's transfers: `None` until asked, then the chat they
    /// belong to and what came back.
    transfers: Option<Vec<ContextTransfer>>,
    /// The chat and its last message the transfers were read at: a merge
    /// lands as a message in the parent, so a new one reads them again.
    transfers_key: Option<(String, Option<DateTime<Utc>>)>,
}

impl TaskLaneState {
    /// Record `mark` for `id`: a move restarts the swap, and a first arrival
    /// at an end starts the settle. The first sight of a task moves nothing.
    fn observe(&mut self, id: &str, mark: Mark, now: Instant) -> Seen {
        let seen = self.seen.entry(id.to_owned()).or_insert(Seen {
            mark,
            epoch: 0,
            changed: None,
            settled: None,
        });
        if seen.mark != mark {
            if mark.terminal() && !seen.mark.terminal() {
                seen.settled = Some(now);
            }
            seen.changed = Some((seen.mark, now));
            seen.epoch += 1;
            seen.mark = mark;
        }
        *seen
    }

    /// A stop asked for is confirmed once the task carries it, or ended.
    fn reconcile(&mut self, row: &TaskRow) {
        if row.task.cancel_requested || row.task.status.is_terminal() {
            self.requested.remove(&row.chat_id);
        }
        if row.task.status.is_terminal() {
            if self.confirm.as_deref() == Some(row.chat_id.as_str()) {
                self.confirm = None;
            }
            self.errors.remove(&row.chat_id);
        }
    }

    fn stop_pending(&self, id: &str) -> bool {
        self.stopping.contains(id) || self.requested.contains(id)
    }
}

// ── Elements ───────────────────────────────────────────────────────────

fn dot(color: gpui::Hsla) -> AnyElement {
    div()
        .size(px(6.0))
        .rounded_full()
        .bg(color)
        .into_any_element()
}

fn mark_element(
    key: String,
    mark: Mark,
    view: gpui::EntityId,
    theme: &Theme,
    cx: &mut App,
) -> AnyElement {
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
        Mark::NeedsInput => dot(spaces::status_dot_color(
            ChatIndicator::AwaitingInput,
            theme,
        )),
        Mark::Ready => icon(icons::CHECK)
            .size(px(12.0))
            .text_color(spaces::status_dot_color(ChatIndicator::Completed, theme))
            .into_any_element(),
        Mark::Sent => icon(icons::CHECK)
            .size(px(12.0))
            .text_color(theme.text_muted)
            .into_any_element(),
        Mark::Failed => dot(spaces::status_dot_color(ChatIndicator::Errored, theme)),
        Mark::Cancelled => icon(icons::CLOSE)
            .size(px(10.0))
            .text_color(theme.text_faint)
            .into_any_element(),
        Mark::Interrupted => dot(theme.warning.opacity(0.7)),
    }
}

/// The status tone of the detail's status word.
fn mark_tone(mark: Mark, theme: &Theme) -> gpui::Hsla {
    match mark {
        Mark::NeedsInput => theme.accent,
        Mark::Ready => theme.success,
        Mark::Failed => theme.danger,
        Mark::Stopping | Mark::Interrupted => theme.warning,
        _ => theme.text_muted,
    }
}

/// The settle's wash, by how the task ended.
fn settle_tone(mark: Mark, theme: &Theme) -> gpui::Hsla {
    match mark {
        Mark::Ready | Mark::Sent => theme.success.opacity(0.12),
        Mark::Failed => theme.danger.opacity(0.10),
        _ => theme.ink(0.06),
    }
}

/// A quiet 22px text button, the session card's tile language; `tone`
/// colors its label.
pub(super) fn quiet_button(
    id: SharedString,
    label: SharedString,
    tone: gpui::Hsla,
    enabled: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let accent = theme.accent;
    div()
        .id(id.clone())
        .debug_selector(move || id.to_string())
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .h(px(22.0))
        .px(px(8.0))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(5.0))
        .rounded(px(5.0))
        .border_1()
        .border_color(gpui::transparent_black())
        .text_size(crate::typography::ui_rems(12.0))
        .text_color(tone)
        .bg(theme.ink(0.05))
        .when(enabled, |button| {
            button
                .tab_index(0)
                .cursor_pointer()
                .hover(|style| style.bg(crate::theme::ink(0.09)))
                .focus_visible(move |style| style.border_color(accent))
        })
        .when(!enabled, |button| button.opacity(0.5).cursor_default())
        .child(label)
}

/// A passive fact: faint text on a faint plate, never button chrome.
pub(super) fn fact(label: SharedString, theme: &Theme) -> gpui::Div {
    div()
        .h(px(18.0))
        .px(px(6.0))
        .flex_none()
        .flex()
        .items_center()
        .rounded(px(5.0))
        .bg(theme.ink(0.06))
        .text_size(crate::typography::ui_rems(11.0))
        .text_color(theme.text_muted)
        .child(label)
}

/// One note line of the detail, in `color`.
pub(super) fn note(text: impl Into<SharedString>, color: gpui::Hsla) -> gpui::Div {
    div()
        .w_full()
        .text_size(crate::typography::ui_rems(12.0))
        .line_height(px(16.0))
        .text_color(color)
        .child(text.into())
}

/// Click and Enter/Space run `run`; the row's own click never fires under it.
pub(super) fn on_activate(
    element: gpui::Stateful<gpui::Div>,
    cx: &mut Context<Shell>,
    run: impl Fn(&mut Shell, &mut Context<Shell>) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let run = Rc::new(run);
    let key_run = run.clone();
    element
        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
            cx.stop_propagation();
            run(this, cx);
        }))
        .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                key_run(this, cx);
            }
        }))
}

impl Shell {
    /// `content` eased open to `reveal` (0..1): laid out at its natural
    /// height inside a clipped frame whose height animates; a paint-time
    /// probe records that height under `measure_key`, so the first frame
    /// uses `estimate` and the next the truth. At rest it keeps its natural
    /// height, so content that grows while open is never clipped.
    pub(super) fn lane_disclosure(
        &mut self,
        measure_key: String,
        reveal: f32,
        estimate: f32,
        content: gpui::Div,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let measured = self
            .session_lanes
            .tasks
            .heights
            .get(&measure_key)
            .copied()
            .unwrap_or(estimate);
        let shell = cx.weak_entity();
        let probe = gpui::canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let height = f32::from(bounds.size.height);
                window.defer(cx, move |_, cx| {
                    shell
                        .update(cx, |this, cx| {
                            let heights = &mut this.session_lanes.tasks.heights;
                            if heights
                                .get(&measure_key)
                                .is_none_or(|known| (known - height).abs() > 0.5)
                            {
                                heights.insert(measure_key, height);
                                cx.notify();
                            }
                        })
                        .ok();
                });
            },
        )
        .absolute()
        .inset_0();
        let content = content.relative().child(probe);
        let frame = div().w_full().flex_none().overflow_hidden();
        if reveal >= 1.0 {
            frame.child(content).into_any_element()
        } else {
            frame
                .h(px(measured * reveal))
                .opacity(0.35 + 0.65 * reveal)
                .child(content.top(px(-3.0 * (1.0 - reveal))))
                .into_any_element()
        }
    }

    // ── Lineage ────────────────────────────────────────────────────────

    /// "Delegated by …" / "Forked from …": the way back up, as the card's
    /// first row. A task's parent can be a task itself, so the row is the
    /// same at every depth.
    pub(super) fn render_lineage_row(
        &mut self,
        chat: &Chat,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let lineage = lineage_for(self.state.read(cx), chat)?;
        let key = format!("session-lanes-lineage-{}", chat.id);
        let row = popover::picker_row(theme, false, false, key.clone())
            .id(SharedString::from(key.clone()))
            .debug_selector(|| "session-lanes-lineage".into())
            .child(popover::picker_icon_slot(
                icon(icons::ARROW_LEFT)
                    .size(px(13.0))
                    .text_color(theme.text_muted)
                    .into_any_element(),
            ))
            .child(
                div()
                    .flex_none()
                    .text_color(theme.text_muted)
                    .child(lineage.label),
            );
        Some(match lineage.title {
            Some(title) => {
                let target = lineage.target.clone();
                on_activate(
                    row.role(gpui::Role::Link)
                        .aria_label(SharedString::from(format!("{} {title}", lineage.label)))
                        .tab_index(0)
                        .border_1()
                        .border_color(gpui::transparent_black())
                        .px(px(7.0))
                        .focus_visible({
                            let accent = theme.accent;
                            move |style| style.border_color(accent)
                        }),
                    cx,
                    move |this, cx| {
                        this.close_session_info_overlay(cx);
                        this.open_chat(target.clone(), cx);
                    },
                )
                .child(row_title(format!("{key}-title"), title))
                .into_any_element()
            }
            None => row
                .cursor_default()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(theme.text_faint)
                        .child("a chat that's no longer here"),
                )
                .into_any_element(),
        })
    }

    // ── Tasks ──────────────────────────────────────────────────────────

    /// The Tasks section: heading, the active rows, the finished fold. Empty
    /// when the chat delegated nothing.
    pub(super) fn render_task_lane(
        &mut self,
        lane: TaskLane,
        view: gpui::EntityId,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        if lane.is_empty() {
            return Vec::new();
        }
        let mut rows = vec![popover::menu_heading(theme, "Tasks").into_any_element()];
        let TaskLane { active, finished } = lane;
        for row in &active {
            self.session_lanes.tasks.reconcile(row);
        }
        for row in &finished {
            self.session_lanes.tasks.reconcile(row);
        }
        for row in active {
            rows.push(self.render_task(row, view, theme, cx));
        }
        if !finished.is_empty() {
            let open = self.session_lanes.tasks.finished_open;
            rows.push(self.render_task_fold(finished.len(), open, view, theme, cx));
            if open {
                let shown = LaneState::shown(self.session_lanes.tasks.shown_finished);
                let hidden = finished.len().saturating_sub(shown);
                for row in finished.into_iter().take(shown) {
                    rows.push(self.render_task(row, view, theme, cx));
                }
                if hidden > 0 {
                    rows.push(self.render_show_more(
                        "session-lanes-tasks-more",
                        hidden,
                        theme,
                        cx,
                        |lanes| {
                            lanes.tasks.shown_finished =
                                LaneState::shown(lanes.tasks.shown_finished) + PAGE_ROWS;
                        },
                    ));
                }
            }
        }
        rows
    }

    fn render_task_fold(
        &mut self,
        count: usize,
        open: bool,
        view: gpui::EntityId,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let turn = motion::state_t(
            &format!("lanes-task-fold-{view}"),
            open,
            motion::COLLAPSE,
            motion::reduced_motion(cx),
        );
        let accent = theme.accent;
        let fold = popover::picker_row(theme, false, false, "session-lanes-tasks-finished")
            .id("session-lanes-tasks-finished")
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(format!("{count} finished tasks")))
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
                    .child(SharedString::from(format!("{count} finished"))),
            );
        on_activate(fold, cx, |this, cx| {
            let tasks = &mut this.session_lanes.tasks;
            tasks.finished_open = !tasks.finished_open;
            cx.notify();
        })
        .into_any_element()
    }

    fn toggle_task(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let tasks = &mut self.session_lanes.tasks;
        if !tasks.open.remove(chat_id) {
            tasks.open.insert(chat_id.to_owned());
        } else if tasks.confirm.as_deref() == Some(chat_id) {
            // Folding the row backs out of a pending confirm.
            tasks.confirm = None;
        }
        cx.notify();
    }

    /// One task: its row, and under it the detail while expanded.
    fn render_task(
        &mut self,
        row: TaskRow,
        view: gpui::EntityId,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = format!("session-lanes-task-{}", row.chat_id);
        let group = SharedString::from(format!("{key}-group"));
        let reduced = motion::reduced_motion(cx);
        let tasks = &mut self.session_lanes.tasks;
        let open = tasks.open.contains(&row.chat_id);
        let stopping = tasks.stop_pending(&row.chat_id);
        let mark = mark_for(&row.task, stopping);
        let seen = tasks.observe(&row.chat_id, mark, Instant::now());
        let reveal = motion::state_t(
            &format!("lanes-task-{view}-{}", row.chat_id),
            open,
            motion::COLLAPSE,
            reduced,
        );

        // ── The mark, swapping when the status moves.
        let slot = {
            let current = mark_element(key.clone(), mark, view, theme, cx);
            let swap = seen
                .changed
                .filter(|(_, at)| !reduced && at.elapsed() < MARK_SWAP.total());
            match swap {
                Some((previous, _)) => {
                    let old = mark_element(format!("{key}-was"), previous, view, theme, cx);
                    let layer = |child: AnyElement| {
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(child)
                    };
                    div()
                        .relative()
                        .size(px(16.0))
                        .child(layer(old).with_animation(
                            SharedString::from(format!("{key}-mark-out-{}", seen.epoch)),
                            MARK_SWAP.animation(),
                            |el, t| el.opacity(1.0 - t).top(px(-3.0 * t)),
                        ))
                        .child(layer(current).with_animation(
                            SharedString::from(format!("{key}-mark-in-{}", seen.epoch)),
                            MARK_SWAP.animation(),
                            |el, t| el.opacity(t).top(px(3.0 * (1.0 - t))),
                        ))
                        .into_any_element()
                }
                None => current,
            }
        };

        // ── Trailing: the time, swapped for the open arrow on hover.
        let time = zeron_proto::view::format_time_ago(millis(row.task.updated_at), Utc::now());
        let open_id = row.chat_id.clone();
        let arrow = div()
            .id(SharedString::from(format!("{key}-open")))
            .debug_selector({
                let key = key.clone();
                move || format!("{key}-open")
            })
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
            .tooltip(crate::settings::widgets::text_tooltip("Open task"))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.close_session_info_overlay(cx);
                this.open_child_chat_tab(&open_id, cx);
            }))
            .child(
                icon(icons::ARROW_UP_RIGHT)
                    .size(px(11.0))
                    .text_color(theme.text_muted.opacity(0.8)),
            );
        let trailing = div()
            .relative()
            .flex_none()
            .min_w(px(18.0))
            .flex()
            .justify_end()
            .child(
                time_label(time.into(), theme)
                    .id(SharedString::from(format!("{key}-time")))
                    .group_hover(group.clone(), |style| style.invisible()),
            )
            .child(arrow);

        let accent = theme.accent;
        let toggle_id = row.chat_id.clone();
        let header = popover::picker_row(theme, false, false, key.clone())
            .id(SharedString::from(key.clone()))
            .debug_selector({
                let key = key.clone();
                move || key.clone()
            })
            .group(group.clone())
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(format!(
                "{}, {}",
                row.title,
                status_text(&row.task, stopping)
            )))
            .aria_expanded(open)
            .tab_index(0)
            .border_1()
            .border_color(gpui::transparent_black())
            .px(px(7.0))
            .focus_visible(move |style| style.border_color(accent))
            .child(popover::picker_icon_slot(slot))
            .children(row.harness.map(|harness| harness_mark(harness, theme)))
            .child(row_title(format!("{key}-title"), row.title.clone()))
            .child(trailing);
        let header = on_activate(header, cx, move |this, cx| this.toggle_task(&toggle_id, cx));

        // ── The detail, measured, eased open.
        let detail = (reveal > 0.001).then(|| {
            let measured = self
                .session_lanes
                .tasks
                .heights
                .get(&row.chat_id)
                .copied()
                .unwrap_or(DETAIL_ESTIMATE);
            let content = self.render_task_detail(&row, mark, stopping, reduced, theme, cx);
            let shell = cx.weak_entity();
            let measure_id = row.chat_id.clone();
            let probe = gpui::canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let height = f32::from(bounds.size.height);
                    window.defer(cx, move |_, cx| {
                        shell
                            .update(cx, |this, cx| {
                                let heights = &mut this.session_lanes.tasks.heights;
                                let known = heights.get(&measure_id).copied();
                                if known.is_none_or(|known| (known - height).abs() > 0.5) {
                                    heights.insert(measure_id, height);
                                    cx.notify();
                                }
                            })
                            .ok();
                    });
                },
            )
            .absolute()
            .inset_0();
            let content = content.relative().child(probe);
            let frame = div().w_full().flex_none().overflow_hidden();
            if reveal >= 1.0 {
                // At rest it takes its natural height, so a summary that
                // lands while it is open grows it rather than clipping.
                frame.child(content).into_any_element()
            } else {
                frame
                    .h(px(measured * reveal))
                    .opacity(0.35 + 0.65 * reveal)
                    .child(content.top(px(-3.0 * (1.0 - reveal))))
                    .into_any_element()
            }
        });

        // ── Row and detail share one plate while open; an end washes it.
        let settling = seen
            .settled
            .filter(|at| !reduced && at.elapsed() < SETTLE.total())
            .map(|_| settle_tone(mark, theme));
        let plate = div()
            .id(SharedString::from(format!("{key}-plate")))
            .w_full()
            .flex()
            .flex_col()
            .rounded(px(popover::MENU_ITEM_RADIUS))
            .bg(theme.ink(0.035 * reveal))
            .child(header)
            .children(detail);
        match settling {
            Some(tone) => plate
                .with_animation(
                    SharedString::from(format!("{key}-settle-{}", seen.epoch)),
                    SETTLE.animation(),
                    move |el, t| el.bg(tone.opacity(1.0 - t)),
                )
                .into_any_element(),
            None => plate.into_any_element(),
        }
    }

    fn render_task_detail(
        &mut self,
        row: &TaskRow,
        mark: Mark,
        stopping: bool,
        reduced: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let key = format!("session-lanes-task-{}", row.chat_id);
        let task = &row.task;
        let now = Utc::now();

        // Status in its tone, the time beside it.
        let status = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .text_size(crate::typography::ui_rems(12.0))
            .child(
                div()
                    .flex_none()
                    .text_color(mark_tone(mark, theme))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(crate::roll_text::roll_text(
                        format!("{key}-status"),
                        status_text(task, stopping).into(),
                        reduced,
                    )),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.text_faint)
                    .child(SharedString::from(elapsed_phrase(task, now))),
            );

        let mut facts: Vec<SharedString> = Vec::new();
        facts.extend(role_label(task.role).map(SharedString::from));
        facts.push(isolation_label(task.isolation).into());
        facts.extend(row.model.clone());
        if row.tasks > 0 {
            facts.push(
                format!(
                    "{} task{}",
                    row.tasks,
                    if row.tasks == 1 { "" } else { "s" }
                )
                .into(),
            );
        }
        let facts = div()
            .flex()
            .flex_wrap()
            .gap(px(4.0))
            .children(facts.into_iter().map(|label| fact(label, theme)));

        let summary = task
            .summary
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|summary| {
                div()
                    .w_full()
                    .text_size(crate::typography::ui_rems(12.0))
                    .line_height(px(17.0))
                    .text_color(if mark == Mark::Failed {
                        theme.danger_muted
                    } else {
                        theme.text.opacity(0.85)
                    })
                    .line_clamp(SUMMARY_LINES)
                    .text_ellipsis()
                    .child(SharedString::from(summary.to_owned()))
            });
        let outcome = outcome_note(task, stopping).map(|text| note(text, theme.text_muted));
        // The record keeps a bounded summary; say when it was cut.
        let shortened = (task.summary_truncated && summary.is_some()).then(|| {
            note(
                "Summary shortened. Open the task to read the full result.",
                theme.text_faint,
            )
        });

        let delivery = delivery_text(task).map(|text| {
            div()
                .flex()
                .items_center()
                .gap(px(5.0))
                .text_size(crate::typography::ui_rems(12.0))
                .text_color(theme.text_muted)
                .child(if settled(task) {
                    icon(icons::CHECK)
                        .size(px(11.0))
                        .text_color(theme.text_muted)
                        .into_any_element()
                } else {
                    div()
                        .size(px(5.0))
                        .rounded_full()
                        .bg(theme.busy.opacity(0.7))
                        .into_any_element()
                })
                .child(crate::roll_text::roll_text(
                    format!("{key}-delivery"),
                    text.into(),
                    reduced,
                ))
        });

        let error = self
            .session_lanes
            .tasks
            .errors
            .get(&row.chat_id)
            .cloned()
            .map(|text| note(text, theme.danger));

        div()
            .w_full()
            .flex_none()
            .pl(px(DETAIL_INDENT))
            .pr(px(8.0))
            .pt(px(2.0))
            .pb(px(8.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(status)
            .child(facts)
            .children(summary)
            .children(shortened)
            .children(outcome)
            .children(delivery)
            .children(error)
            .child(self.render_task_actions(row, mark, theme, cx))
    }

    /// Open task, Open in main view, and Stop with its in-place confirm.
    fn render_task_actions(
        &mut self,
        row: &TaskRow,
        mark: Mark,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let key = format!("session-lanes-task-{}", row.chat_id);
        let tasks = &self.session_lanes.tasks;
        let confirming = tasks.confirm.as_deref() == Some(row.chat_id.as_str());
        let in_flight = tasks.stopping.contains(&row.chat_id);
        let failed_before = tasks.errors.contains_key(&row.chat_id);
        let can_stop = !row.task.status.is_terminal()
            && !row.task.cancel_requested
            && !tasks.requested.contains(&row.chat_id);

        let open_id = row.chat_id.clone();
        let primary = if mark == Mark::NeedsInput {
            theme.accent
        } else {
            theme.text
        };
        let open = on_activate(
            quiet_button(
                format!("{key}-open-task").into(),
                "Open task".into(),
                primary,
                true,
                theme,
            ),
            cx,
            move |this, cx| {
                this.close_session_info_overlay(cx);
                this.open_child_chat_tab(&open_id, cx);
            },
        );
        // A task's own tasks show in ITS lanes card: the main view is how a
        // nested task is reached.
        let main_id = row.chat_id.clone();
        let accent = theme.accent;
        let main = on_activate(
            div()
                .id(SharedString::from(format!("{key}-open-main")))
                .role(gpui::Role::Button)
                .aria_label("Open in main view")
                .size(px(22.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .border_1()
                .border_color(gpui::transparent_black())
                .tab_index(0)
                .cursor_pointer()
                .hover(|style| style.bg(crate::theme::ink(0.08)))
                .focus_visible(move |style| style.border_color(accent))
                .tooltip(crate::settings::widgets::text_tooltip("Open in main view"))
                .child(
                    icon(icons::EXPAND_ARROWS)
                        .size(px(13.0))
                        .text_color(theme.text_muted),
                ),
            cx,
            move |this, cx| {
                this.close_session_info_overlay(cx);
                this.open_chat(main_id.clone(), cx);
            },
        );

        let mut actions = div()
            .id(SharedString::from(format!("{key}-actions")))
            .flex()
            .items_center()
            .gap(px(4.0));
        // While Stop waits for its answer, the row holds only the question;
        // otherwise the ways in lead and Stop sits apart at the far end.
        if !(can_stop && confirming && !in_flight) {
            actions = actions.child(open).child(main).child(div().flex_1());
        }

        // The Stop slot keeps its element id through the confirm, so focus
        // lands on "Keep running" — the safe answer — after a keyboard press.
        let stop_id: SharedString = format!("{key}-stop").into();
        if in_flight {
            actions = actions.child(
                quiet_button(stop_id, "Stopping…".into(), theme.warning, false, theme).child(
                    loaders::mini_mono_spinner(
                        format!("{key}-stop-spin"),
                        1.5,
                        theme.warning,
                        cx.entity_id(),
                        cx,
                    ),
                ),
            );
        } else if can_stop && confirming {
            let keep = on_activate(
                quiet_button(stop_id, "Keep running".into(), theme.text, true, theme),
                cx,
                |this, cx| {
                    this.session_lanes.tasks.confirm = None;
                    cx.notify();
                },
            );
            let target = row.clone();
            let confirm = on_activate(
                quiet_button(
                    format!("{key}-stop-confirm").into(),
                    "Stop".into(),
                    theme.danger,
                    true,
                    theme,
                ),
                cx,
                move |this, cx| this.stop_task(&target, cx),
            );
            actions = actions
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        cx.stop_propagation();
                        this.session_lanes.tasks.confirm = None;
                        cx.notify();
                    }
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .child("Stop it?"),
                )
                .child(keep)
                .child(confirm);
        } else if can_stop {
            let label = if failed_before {
                "Try again"
            } else {
                "Stop task"
            };
            let id = row.chat_id.clone();
            let button = quiet_button(stop_id, label.into(), theme.text_muted, row.online, theme);
            actions = actions.child(if row.online {
                on_activate(button, cx, move |this, cx| {
                    this.session_lanes.tasks.confirm = Some(id.clone());
                    cx.notify();
                })
                // Forks and side chats it started are independent and keep running.
                .tooltip(crate::settings::widgets::text_tooltip(
                    "Stops this task and the tasks it started",
                ))
                .into_any_element()
            } else {
                button
                    .tooltip(crate::settings::widgets::text_tooltip(
                        "Its device is offline",
                    ))
                    .into_any_element()
            });
        }
        actions
    }

    /// Ask the task's host to stop it. The row reads "Stopping…" from here
    /// until the task itself says so, then ends where the engine puts it.
    fn stop_task(&mut self, row: &TaskRow, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let chat_id = row.chat_id.clone();
        let tasks = &mut self.session_lanes.tasks;
        tasks.confirm = None;
        tasks.errors.remove(&chat_id);
        if !tasks.stopping.insert(chat_id.clone()) {
            return;
        }
        let params = serde_json::json!({
            "parentChatId": row.task.parent_chat_id,
            "taskId": row.task.id,
            // A task runs on its parent's host; route there.
            "targetDeviceId": row.device_id,
        });
        let origin = self.session_lanes.chat_id.clone();
        cx.spawn(async move |this, cx| {
            let result = engine.client().call(methods::TASK_CANCEL, params).await;
            let _ = this.update(cx, |this, cx| {
                // The card moved to another chat: its lanes started fresh.
                if this.session_lanes.chat_id != origin {
                    return;
                }
                let tasks = &mut this.session_lanes.tasks;
                tasks.stopping.remove(&chat_id);
                match result {
                    Ok(_) => {
                        tasks.requested.insert(chat_id);
                    }
                    Err(error) => {
                        tasks
                            .errors
                            .insert(chat_id, format!("Couldn't stop the task. {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    // ── Fork merge-back ────────────────────────────────────────────────

    /// Read the open chat's transfers for "Merged": once per visit and
    /// again whenever a message lands in it (a merge from anywhere — MCP, a
    /// phone — arrives as one). An engine without them leaves the local
    /// flag to speak.
    pub(super) fn ensure_chat_transfers(&mut self, chat: &Chat, cx: &mut Context<Self>) {
        let key = (chat.id.clone(), chat.last_message_at);
        if self.session_lanes.tasks.transfers_key.as_ref() == Some(&key) {
            return;
        }
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.session_lanes.tasks.transfers_key = Some(key.clone());
        let params = serde_json::json!({
            "chatId": chat.id,
            "targetDeviceId": chat.device_id,
        });
        cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call_as::<Vec<ContextTransfer>>(methods::CHAT_TRANSFERS, params)
                .await;
            let _ = this.update(cx, |this, cx| {
                let tasks = &mut this.session_lanes.tasks;
                // Only the read for the current chat and message lands.
                if let Ok(list) = result
                    && tasks.transfers_key.as_ref() == Some(&key)
                {
                    tasks.transfers = Some(list);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn fork_merged_at(&self, fork: &str) -> Option<Option<DateTime<Utc>>> {
        let tasks = &self.session_lanes.tasks;
        let recorded = tasks
            .transfers
            .as_deref()
            .and_then(|list| latest_merge(list, fork))
            .map(|t| Some(millis(t.created_at)));
        recorded.or_else(|| tasks.merged.contains(fork).then_some(None))
    }

    /// The merge tile in a fork's row, or "Merged" once it has been.
    pub(super) fn render_fork_merge_tile(
        &mut self,
        row: &ChildChatRow,
        group: SharedString,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !row.fork || !row.same_device {
            return None;
        }
        let key = format!("session-lanes-chat-{}-merge", row.chat_id);
        let merged = self.fork_merged_at(&row.chat_id).is_some();
        let open = self.session_lanes.tasks.merge_open.as_deref() == Some(row.chat_id.as_str());
        let tooltip = if row.busy {
            "Finish the fork's turn first"
        } else {
            "Merge into this chat"
        };
        let accent = theme.accent;
        let id = row.chat_id.clone();
        let tile = div()
            .id(SharedString::from(key.clone()))
            .debug_selector({
                let key = key.clone();
                move || key.clone()
            })
            .role(gpui::Role::Button)
            .aria_label(tooltip)
            .aria_expanded(open)
            .size(px(22.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .border_1()
            .border_color(gpui::transparent_black())
            .when(!open, |tile| {
                tile.invisible().group_hover(group, |style| style.visible())
            })
            .when(open, |tile| tile.bg(crate::theme::ink(0.08)))
            .when(row.busy, |tile| tile.opacity(0.5))
            .when(!row.busy, |tile| {
                tile.tab_index(0)
                    .cursor_pointer()
                    .hover(|style| style.bg(crate::theme::ink(0.08)))
                    .focus_visible(move |style| style.visible().border_color(accent))
            })
            .tooltip(crate::settings::widgets::text_tooltip(tooltip))
            .child(
                icon(icons::RETURN)
                    .size(px(13.0))
                    .text_color(theme.text_muted),
            );
        let tile = if row.busy {
            tile
        } else {
            on_activate(tile, cx, move |this, cx| {
                let tasks = &mut this.session_lanes.tasks;
                tasks.merge_open = if tasks.merge_open.as_deref() == Some(id.as_str()) {
                    None
                } else {
                    Some(id.clone())
                };
                tasks.merge_errors.remove(&id);
                cx.notify();
            })
        };
        let merged_mark = merged.then(|| {
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(3.0))
                .text_size(crate::typography::ui_rems(11.0))
                .text_color(theme.text_faint)
                .child(
                    icon(icons::CHECK)
                        .size(px(10.0))
                        .text_color(theme.text_faint),
                )
                .child("Merged")
        });
        Some(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(4.0))
                .children(merged_mark)
                .child(tile)
                .into_any_element(),
        )
    }

    /// The confirm strip under a fork's row, eased open like a task's detail.
    pub(super) fn render_fork_merge_strip(
        &mut self,
        row: &ChildChatRow,
        view: gpui::EntityId,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !row.fork || !row.same_device {
            return None;
        }
        let open = self.session_lanes.tasks.merge_open.as_deref() == Some(row.chat_id.as_str());
        let reduced = motion::reduced_motion(cx);
        let reveal = motion::state_t(
            &format!("lanes-merge-{view}-{}", row.chat_id),
            open,
            motion::COLLAPSE,
            reduced,
        );
        if reveal <= 0.001 {
            return None;
        }
        let key = format!("session-lanes-chat-{}-merge", row.chat_id);
        let tasks = &self.session_lanes.tasks;
        let merging = tasks.merging.contains(&row.chat_id);
        let error = tasks.merge_errors.get(&row.chat_id).cloned();
        let blurb = match self.fork_merged_at(&row.chat_id) {
            Some(Some(at)) => format!(
                "Sends the fork's latest response here as context. Last merged {}.",
                match zeron_proto::view::format_time_ago(at, Utc::now()).as_str() {
                    "now" => "just now".to_owned(),
                    ago => format!("{ago} ago"),
                }
            ),
            _ => "Sends the fork's latest response here as context.".to_owned(),
        };
        let fork = row.chat_id.clone();
        let device = row.device_id.clone();
        let merge = quiet_button(
            format!("{key}-confirm").into(),
            if merging { "Merging…" } else { "Merge" }.into(),
            theme.accent,
            !merging && !row.busy,
            theme,
        );
        let merge = if merging || row.busy {
            merge
        } else {
            on_activate(merge, cx, move |this, cx| {
                this.merge_fork(fork.clone(), device.clone(), cx)
            })
        };
        let dismiss_id = row.chat_id.clone();
        let dismiss = on_activate(
            quiet_button(
                format!("{key}-dismiss").into(),
                "Not now".into(),
                theme.text_muted,
                true,
                theme,
            ),
            cx,
            move |this, cx| {
                let tasks = &mut this.session_lanes.tasks;
                if tasks.merge_open.as_deref() == Some(dismiss_id.as_str()) {
                    tasks.merge_open = None;
                }
                cx.notify();
            },
        );
        let measure_key = format!("merge:{}", row.chat_id);
        let measured = self
            .session_lanes
            .tasks
            .heights
            .get(&measure_key)
            .copied()
            .unwrap_or(64.0);
        let shell = cx.weak_entity();
        let probe = gpui::canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let height = f32::from(bounds.size.height);
                window.defer(cx, move |_, cx| {
                    shell
                        .update(cx, |this, cx| {
                            let heights = &mut this.session_lanes.tasks.heights;
                            if heights
                                .get(&measure_key)
                                .is_none_or(|known| (known - height).abs() > 0.5)
                            {
                                heights.insert(measure_key, height);
                                cx.notify();
                            }
                        })
                        .ok();
                });
            },
        )
        .absolute()
        .inset_0();
        let content = div()
            .id(SharedString::from(format!("{key}-strip")))
            .relative()
            .w_full()
            .flex_none()
            .pl(px(DETAIL_INDENT))
            .pr(px(8.0))
            .pt(px(2.0))
            .pb(px(8.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    this.session_lanes.tasks.merge_open = None;
                    cx.notify();
                }
            }))
            .child(note(blurb, theme.text_muted))
            .children(error.map(|text| note(text, theme.danger)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(merge)
                    .child(dismiss),
            )
            .child(probe);
        let frame = div()
            .w_full()
            .flex_none()
            .overflow_hidden()
            .rounded(px(popover::MENU_ITEM_RADIUS))
            .bg(theme.ink(0.035 * reveal));
        Some(if reveal >= 1.0 {
            frame.child(content).into_any_element()
        } else {
            frame
                .h(px(measured * reveal))
                .opacity(0.35 + 0.65 * reveal)
                .child(content.top(px(-3.0 * (1.0 - reveal))))
                .into_any_element()
        })
    }

    /// Merge `fork`'s latest response into its parent (the open chat), on
    /// the fork's host — the engine refuses across devices.
    fn merge_fork(&mut self, fork: String, device: String, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let tasks = &mut self.session_lanes.tasks;
        tasks.merge_errors.remove(&fork);
        if !tasks.merging.insert(fork.clone()) {
            return;
        }
        let params = serde_json::json!({
            "sourceChatId": fork,
            "targetDeviceId": device,
        });
        let origin = self.session_lanes.chat_id.clone();
        cx.spawn(async move |this, cx| {
            let result = engine.client().call(methods::MERGE_CHAT, params).await;
            let _ = this.update(cx, |this, cx| {
                if this.session_lanes.chat_id != origin {
                    return;
                }
                let tasks = &mut this.session_lanes.tasks;
                tasks.merging.remove(&fork);
                match result {
                    Ok(_) => {
                        if tasks.merge_open.as_deref() == Some(fork.as_str()) {
                            tasks.merge_open = None;
                        }
                        tasks.merged.insert(fork);
                        // Re-read the transfers for the recorded time.
                        tasks.transfers_key = None;
                    }
                    Err(error) => {
                        tasks
                            .merge_errors
                            .insert(fork, format!("Couldn't merge. {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

/// Native task-lanes fixture only: stage the lane's view state at once —
/// unfolded tasks, the finished fold, a pending stop confirm, a stop error,
/// a fork's open merge strip — and show the card.
#[cfg(feature = "appshots-fixture")]
impl Shell {
    pub fn fixture_task_lanes(
        &mut self,
        open: &[&str],
        finished: bool,
        confirm: Option<&str>,
        error: Option<(&str, &str)>,
        merge: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        // The card follows the selected chat; stage its state under that id.
        let Some(chat) = self.state.read(cx).selected_chat.clone() else {
            return;
        };
        self.session_lanes.follow(&chat);
        let tasks = &mut self.session_lanes.tasks;
        tasks.open = open.iter().map(|id| (*id).to_owned()).collect();
        tasks.finished_open = finished;
        tasks.confirm = confirm.map(str::to_owned);
        tasks.errors.clear();
        if let Some((id, message)) = error {
            tasks
                .errors
                .insert(id.to_owned(), message.to_owned().into());
        }
        tasks.merge_open = merge.map(str::to_owned);
        self.show_session_info(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(status: TaskStatus) -> DelegatedTask {
        serde_json::from_value(serde_json::json!({
            "id": "t", "parentChatId": "main", "role": "research", "isolation": "worktree",
            "status": status, "prompt": "Audit the handoff budget\nmore", "requestFingerprint": "f",
            "createdAt": 0, "updatedAt": 0,
        }))
        .unwrap()
    }

    fn task_chat(id: &str, parent: &str, task: DelegatedTask, created: i64) -> Chat {
        let mut task = task;
        task.created_at = created;
        task.updated_at = created;
        serde_json::from_value(serde_json::json!({
            "id": id, "deviceId": "dev", "archived": false, "createdAt": Utc::now(),
            "parentChatId": parent, "orchestration": { "task": task },
        }))
        .unwrap()
    }

    #[test]
    fn marks_follow_the_task_truthfully() {
        let mut t = task(TaskStatus::Running);
        assert_eq!(mark_for(&t, false), Mark::Working);
        // A stop asked for reads Stopping until the task ends.
        assert_eq!(mark_for(&t, true), Mark::Stopping);
        t.cancel_requested = true;
        assert_eq!(mark_for(&t, false), Mark::Stopping);
        assert_eq!(status_text(&t, false), "Stopping…");
        t.status = TaskStatus::Cancelled;
        assert_eq!(mark_for(&t, false), Mark::Cancelled);
        assert_eq!(status_text(&t, false), "Cancelled");
        t.cancel_requested = false;
        t.status = TaskStatus::Completed;
        assert_eq!(mark_for(&t, false), Mark::Ready);
        t.delivered = true;
        assert_eq!(mark_for(&t, false), Mark::Sent);
    }

    #[test]
    fn delivery_reads_sent_only_once_applied() {
        let mut t = task(TaskStatus::Running);
        assert_eq!(delivery_text(&t), None);
        t.status = TaskStatus::Completed;
        assert_eq!(delivery_text(&t), Some("Result ready, not sent yet"));
        t.delivered = true;
        assert_eq!(delivery_text(&t), Some("Sent to this chat"));
        assert_eq!(outcome_note(&t, false), Some("Finished without a summary."));
        t.summary = Some("Found two regressions.".into());
        assert_eq!(outcome_note(&t, false), None);
    }

    #[test]
    fn a_stopped_task_owes_nothing_and_folds_away() {
        let mut t = task(TaskStatus::Running);
        t.cancel_requested = true;
        assert!(!settled(&t), "still stopping");
        t.status = TaskStatus::Cancelled;
        // Delivery is suppressed for a stop: never "not sent yet".
        assert!(settled(&t));
        assert_eq!(delivery_text(&t), Some("Stopped from this chat"));
        // Even one that completed as the stop landed owes no result.
        t.status = TaskStatus::Completed;
        assert_eq!(mark_for(&t, false), Mark::Sent);
        let mut state = AppState::new();
        state.apply_chats(vec![
            task_chat("x", "main", t.clone(), 1),
            task_chat("y", "main", task(TaskStatus::Running), 2),
        ]);
        let lane = TaskLane::new(task_rows(&state, "main", Utc::now()));
        assert_eq!(lane.active.len(), 1);
        assert_eq!(lane.finished[0].chat_id, "x");
    }

    #[test]
    fn the_lane_keeps_owed_results_up_and_folds_the_sent() {
        let mut state = AppState::new();
        let mut sent = task(TaskStatus::Completed);
        sent.delivered = true;
        let mut nested = task(TaskStatus::Running);
        nested.parent_chat_id = "b".into();
        state.apply_chats(vec![
            serde_json::from_value(serde_json::json!({
                "id": "main", "deviceId": "dev", "archived": false, "createdAt": Utc::now(),
            }))
            .unwrap(),
            task_chat("a", "main", task(TaskStatus::Running), 30),
            task_chat("b", "main", task(TaskStatus::Queued), 10),
            task_chat("c", "main", task(TaskStatus::Completed), 20),
            task_chat("d", "main", sent, 5),
            task_chat("e", "b", nested, 40),
        ]);
        let lane = TaskLane::new(task_rows(&state, "main", Utc::now()));
        let ids = |rows: &[TaskRow]| rows.iter().map(|r| r.chat_id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&lane.active), ["b", "c", "a"]);
        assert_eq!(ids(&lane.finished), ["d"]);
        // The brief's first line names an untitled task.
        assert_eq!(lane.active[0].title.as_ref(), "Audit the handoff budget");
        // A task that delegated in turn counts its own tasks.
        assert_eq!(lane.active[0].tasks, 1);
        // A nested task leads back to the task that delegated it.
        let e = state.chats.iter().find(|c| c.id == "e").unwrap().clone();
        let lineage = lineage_for(&state, &e).unwrap();
        assert_eq!(lineage.label, "Delegated by");
        assert_eq!(lineage.target, "b");
        assert_eq!(lineage.title.as_deref(), Some("Audit the handoff budget"));
    }

    #[test]
    fn a_status_move_swaps_once_and_an_end_settles_once() {
        let mut lanes = TaskLaneState::default();
        let now = Instant::now();
        let first = lanes.observe("a", Mark::Working, now);
        assert!(first.changed.is_none() && first.settled.is_none());
        let done = lanes.observe("a", Mark::Ready, now);
        assert_eq!(done.changed.map(|(was, _)| was), Some(Mark::Working));
        assert!(done.settled.is_some());
        let sent = lanes.observe("a", Mark::Sent, now);
        assert_eq!(sent.epoch, 2);
        // Ready → Sent is not a second ending.
        assert_eq!(sent.settled, done.settled);
        // A task first seen finished never replays its ending.
        assert!(lanes.observe("b", Mark::Failed, now).settled.is_none());
    }

    #[test]
    fn merges_are_read_per_fork() {
        let transfer = |id: &str, source: &str, at: i64| ContextTransfer {
            id: id.into(),
            kind: "merge".into(),
            source_chat_id: source.into(),
            source_message_id: None,
            target_harness: None,
            created_at: at,
            status: "queued".into(),
            content: String::new(),
            truncated: false,
        };
        let list = [
            transfer("1", "f", 1),
            transfer("2", "f", 5),
            transfer("3", "g", 9),
        ];
        assert_eq!(latest_merge(&list, "f").map(|t| t.id.as_str()), Some("2"));
        assert!(latest_merge(&list, "h").is_none());
    }
}
