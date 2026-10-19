//! A chat's tasks and side chats under its sidebar row. Children never take
//! a top-level row (they are the parent's workers, not sessions the person
//! started), but the work they carry is visible where the person looks: a
//! slim line under the parent says how many there are and how many are
//! running, and unfolds into their rows — nested tasks indented under the
//! task that delegated them, four levels at most.
//!
//! Status words and marks come from the task record and the live session,
//! the same as the lanes card and the transcript's pills.

use chrono::{DateTime, Utc};
use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};

use zeron_proto::Chat;

use super::Shell;
use crate::chat_pill::{self, Mark};
use crate::motion;
use crate::state::AppState;
use crate::theme::Theme;

/// Deepest nesting drawn — the delegation limit.
const MAX_DEPTH: usize = 4;
/// The summary line under a parent with children.
pub(super) const SUMMARY_HEIGHT: f32 = 22.0;
/// One child row.
const CHILD_HEIGHT: f32 = 26.0;
const CHILD_GAP: f32 = 2.0;
/// Indent per nesting level, past the parent's text column.
const INDENT: f32 = 12.0;
/// Where the tree's guide sits, under the parent's status column.
const GUIDE_X: f32 = 15.0;

/// One child in a parent's flattened subtree.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TreeNode {
    pub chat_id: String,
    pub title: SharedString,
    pub harness: Option<zeron_proto::HarnessId>,
    pub mark: Mark,
    pub task: bool,
    pub depth: usize,
    pub time: SharedString,
}

/// What the collapsed line counts.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct TreeCounts {
    pub tasks: usize,
    pub side_chats: usize,
    pub running: usize,
    pub needs_input: usize,
}

fn live_children<'a>(state: &'a AppState, parent: &str) -> Vec<&'a Chat> {
    let mut children: Vec<&Chat> = state
        .chats
        .iter()
        .filter(|chat| !chat.archived && chat.parent_chat_id.as_deref() == Some(parent))
        .collect();
    // Tasks still owed lead (oldest first, as in the lanes card); the rest
    // follow by recent activity.
    let owed = |chat: &Chat| {
        chat.orchestration
            .as_ref()
            .and_then(|o| o.task.as_ref())
            .is_some_and(|task| !(task.status.is_terminal() && (task.delivered || task.cancel_requested)))
    };
    children.sort_by(|a, b| {
        owed(b).cmp(&owed(a)).then_with(|| {
            if owed(a) {
                a.created_at.cmp(&b.created_at)
            } else {
                b.last_message_at
                    .unwrap_or(b.created_at)
                    .cmp(&a.last_message_at.unwrap_or(a.created_at))
            }
        })
    });
    children
}

/// `parent`'s live subtree, depth-first, to [`MAX_DEPTH`].
pub(super) fn subtree(state: &AppState, parent: &str, now: DateTime<Utc>) -> Vec<TreeNode> {
    fn walk(state: &AppState, parent: &str, depth: usize, now: DateTime<Utc>, out: &mut Vec<TreeNode>) {
        if depth > MAX_DEPTH {
            return;
        }
        for chat in live_children(state, parent) {
            let reference = chat_pill::ChatRef::of(state, chat);
            out.push(TreeNode {
                chat_id: chat.id.clone(),
                title: reference.title.clone(),
                harness: reference.harness,
                mark: reference.mark,
                task: chat.orchestration.as_ref().is_some_and(|o| o.task.is_some()),
                depth,
                time: zeron_proto::view::format_time_ago(
                    chat.last_message_at.unwrap_or(chat.created_at),
                    now,
                )
                .into(),
            });
            walk(state, &chat.id, depth + 1, now, out);
        }
    }
    let mut out = Vec::new();
    walk(state, parent, 1, now, &mut out);
    out
}

pub(super) fn counts(nodes: &[TreeNode]) -> TreeCounts {
    let mut counts = TreeCounts::default();
    for node in nodes {
        if node.depth == 1 {
            if node.task {
                counts.tasks += 1;
            } else {
                counts.side_chats += 1;
            }
        }
        match node.mark {
            Mark::Working | Mark::WaitingOnTasks | Mark::Stopping | Mark::Queued => {
                counts.running += 1
            }
            Mark::NeedsInput => counts.needs_input += 1,
            _ => {}
        }
    }
    counts
}

/// What the line names: "3 tasks", "1 task, 2 side chats". Running and
/// waiting counts ride beside it as pills.
pub(super) fn label(counts: TreeCounts) -> String {
    let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut what = Vec::new();
    if counts.tasks > 0 {
        what.push(plural(counts.tasks, "task", "tasks"));
    }
    if counts.side_chats > 0 {
        what.push(plural(counts.side_chats, "side chat", "side chats"));
    }
    what.join(", ")
}

/// The line spoken whole: "3 tasks · 1 running · 1 needs input".
pub(super) fn summary(counts: TreeCounts) -> String {
    let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut what = Vec::new();
    if counts.tasks > 0 {
        what.push(plural(counts.tasks, "task", "tasks"));
    }
    if counts.side_chats > 0 {
        what.push(plural(counts.side_chats, "side chat", "side chats"));
    }
    let mut out = what.join(", ");
    if counts.running > 0 {
        out.push_str(&format!(" · {} running", counts.running));
    }
    if counts.needs_input > 0 {
        out.push_str(&format!(" · {} needs input", counts.needs_input));
    }
    out
}

impl Shell {
    /// The tree block under `parent`'s row and its height this frame, or
    /// `None` when the chat has no live children.
    pub(super) fn sidebar_tree_block(
        &mut self,
        parent: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<(f32, AnyElement)> {
        let now = Utc::now();
        let (nodes, selected) = {
            let state = self.state.read(cx);
            (subtree(state, parent, now), state.selected_chat.clone())
        };
        if nodes.is_empty() {
            return None;
        }
        let counts = counts(&nodes);
        let open = self.sidebar_open_trees.contains(parent);
        let motion_key = format!("tree:{parent}");
        let full = nodes.len() as f32 * (CHILD_HEIGHT + CHILD_GAP);
        let tween = self
            .sidebar_disclosure_motion
            .get(&motion_key)
            .copied()
            .filter(|motion| motion.animating() && !motion::reduced_motion(cx));
        let body_height = tween.map_or(if open { full } else { 0.0 }, |t| t.current());
        let animating = tween.is_some();
        let view = cx.entity_id();
        let toggle_parent = parent.to_owned();
        let chevron = self.sidebar_disclosure_chevron(&motion_key, open, theme);
        let summary_line = div()
            .id(SharedString::from(format!("sidebar-tree-{parent}")))
            .h(px(SUMMARY_HEIGHT))
            .ml(px(GUIDE_X - 6.0))
            .pr(px(8.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .rounded(px(Theme::CONTROL_RADIUS))
            .cursor_pointer()
            .text_size(crate::typography::ui_rems(11.0))
            .text_color(theme.text_muted)
            .hover(|s| s.bg(theme.ink(0.04)).text_color(theme.text))
            .tooltip(crate::settings::widgets::text_tooltip(summary(counts)))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                let key = format!("tree:{toggle_parent}");
                let was_open = this.sidebar_open_trees.contains(&toggle_parent);
                let full = {
                    let state = this.state.read(cx);
                    subtree(state, &toggle_parent, Utc::now()).len() as f32
                        * (CHILD_HEIGHT + CHILD_GAP)
                };
                this.begin_sidebar_disclosure_motion(
                    &key,
                    if was_open { full } else { 0.0 },
                    if was_open { 0.0 } else { full },
                );
                if was_open {
                    this.sidebar_open_trees.remove(&toggle_parent);
                } else {
                    this.sidebar_open_trees.insert(toggle_parent.clone());
                }
                cx.notify();
            }))
            .child(chevron)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(SharedString::from(label(counts))),
            )
            // What's live stays counted whether the tree is open or not:
            // running work as the sidebar's own running pill, open
            // questions as an accent dot with their count.
            .when(counts.running > 0, |el| {
                el.child(crate::running_pill::running_pill(
                    format!("sidebar-tree-{parent}-running"),
                    counts.running as u32,
                    theme,
                ))
            })
            .when(counts.needs_input > 0, |el| {
                el.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .text_color(theme.accent)
                        .child(
                            div().size(px(6.0)).rounded_full().bg(
                                crate::shell::spaces::status_dot_color(
                                    zeron_proto::ChatIndicator::AwaitingInput,
                                    theme,
                                ),
                            ),
                        )
                        .child(SharedString::from(counts.needs_input.to_string())),
                )
            });
        let rows = (body_height > 0.0).then(|| {
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(CHILD_GAP))
                .pt(px(CHILD_GAP))
                .children(nodes.iter().map(|node| {
                    let chat_id = node.chat_id.clone();
                    let is_selected = selected.as_deref() == Some(node.chat_id.as_str());
                    let mark = chat_pill::mark_element(
                        format!("sidebar-tree-mark-{}", node.chat_id),
                        node.mark,
                        view,
                        theme,
                        cx,
                    );
                    div()
                        .id(SharedString::from(format!("sidebar-tree-row-{}", node.chat_id)))
                        .h(px(CHILD_HEIGHT))
                        .flex_none()
                        .ml(px(GUIDE_X + INDENT * (node.depth - 1) as f32))
                        .pl(px(6.0))
                        .pr(px(8.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(7.0))
                        .rounded(px(Theme::CONTROL_RADIUS))
                        .cursor_pointer()
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(if is_selected { theme.text } else { theme.text_muted })
                        .when(is_selected, |el| el.bg(theme.ink(0.06)))
                        .hover(|s| s.bg(theme.ink(0.045)).text_color(theme.text))
                        .tooltip(crate::settings::widgets::text_tooltip(format!(
                            "{}, {}",
                            node.title,
                            node.mark.label()
                        )))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.open_chat(chat_id.clone(), cx);
                        }))
                        .child(
                            div()
                                .flex_none()
                                .size(px(13.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(mark),
                        )
                        .when_some(node.harness, |el, harness| {
                            el.child(chat_pill::harness_mark(harness, 12.0, theme))
                        })
                        .child(super::sidebar_faded_label(
                            format!("sidebar-tree-title-{}", node.chat_id).into(),
                            true,
                            div().line_height(px(16.0)).child(SharedString::from(
                                chat_pill::display_title(&node.title, node.harness).to_owned(),
                            )),
                        ))
                        .child(
                            div()
                                .flex_none()
                                .text_size(crate::typography::ui_rems(11.0))
                                .text_color(match node.mark {
                                    Mark::NeedsInput => theme.accent,
                                    Mark::Failed => theme.danger,
                                    _ => theme.text_faint,
                                })
                                .child(match node.mark {
                                    Mark::NeedsInput => SharedString::from("Input"),
                                    Mark::Failed => SharedString::from("Failed"),
                                    Mark::Working => SharedString::from("Working"),
                                    _ => node.time.clone(),
                                }),
                        )
                }))
        });
        let body = div()
            .w_full()
            .h(px(body_height))
            .overflow_hidden()
            .children(rows)
            .when(animating, |el| {
                el.relative().child(
                    gpui::canvas(
                        |_, _, _| (),
                        move |_, _, window, _| window.on_next_frame(move |_, cx| cx.notify(view)),
                    )
                    .absolute()
                    .inset_0(),
                )
            });
        Some((
            SUMMARY_HEIGHT + body_height,
            div()
                .w_full()
                .flex()
                .flex_col()
                .child(summary_line)
                .child(body)
                .into_any_element(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(id: &str, parent: Option<&str>, task: Option<&str>) -> Chat {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "deviceId": "dev",
            "archived": false,
            "createdAt": Utc::now(),
            "title": id,
            "parentChatId": parent,
            "orchestration": task.map(|status| serde_json::json!({"task": {
                "id": id, "parentChatId": parent, "role": "general", "isolation": "shared",
                "status": status, "prompt": "x", "requestFingerprint": "f",
                "createdAt": 1, "updatedAt": 1
            }})),
        }))
        .unwrap()
    }

    #[test]
    fn the_tree_nests_tasks_and_counts_what_runs() {
        let mut state = AppState::new();
        state.chats = vec![
            chat("main", None, None),
            chat("audit", Some("main"), Some("running")),
            chat("review", Some("main"), Some("waiting")),
            chat("side", Some("main"), None),
            chat("deep", Some("audit"), Some("queued")),
            chat("other", None, None),
        ];
        let nodes = subtree(&state, "main", Utc::now());
        let order: Vec<(&str, usize)> = nodes.iter().map(|n| (n.chat_id.as_str(), n.depth)).collect();
        assert_eq!(
            order,
            [("audit", 1), ("deep", 2), ("review", 1), ("side", 1)]
        );
        let counts = counts(&nodes);
        assert_eq!(counts.tasks, 2);
        assert_eq!(counts.side_chats, 1);
        assert_eq!(counts.running, 2, "working and queued");
        assert_eq!(counts.needs_input, 1);
        assert_eq!(summary(counts), "2 tasks, 1 side chat · 2 running · 1 needs input");
        assert!(subtree(&state, "other", Utc::now()).is_empty());
    }
}

#[cfg(feature = "appshots-fixture")]
impl Shell {
    /// Unfold the named parents' trees, settled.
    pub fn fixture_sidebar_trees(&mut self, open: &[&str], cx: &mut Context<Self>) {
        self.sidebar_open_trees = open.iter().map(|id| (*id).to_owned()).collect();
        self.sidebar_disclosure_motion
            .retain(|key, _| !key.starts_with("tree:"));
        cx.notify();
    }
}
