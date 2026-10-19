//! Managed delegations in a tool group: Zeron's `delegate_task` (any
//! harness's MCP spelling) and the agent CLI's `zeron chat spawn`. They join
//! the provider's own subagent spawns as one always-visible spawn group —
//! a running child stays in sight without opening a fold — and each line
//! names its child chat with the live pill: harness mark, title, status.
//! The whole line opens the child beside this chat.
//!
//! A call learns its child only from evidence that names one child: its
//! request key (the engine derives the task id from it) — authoritative, so
//! a keyed call whose child isn't there stays a placeholder — else a title
//! that exactly one of this chat's tasks and exactly one keyless call carry.
//! Order is never evidence: a failed or not-yet-created delegation must not
//! borrow another child.

use super::*;

use zeron_proto::chat_refs::{DelegateCall, delegate_call, managed_task_id};

use crate::chat_pill::{self, ChatRef, Mark};

/// The delegation a tool item performs, if any.
pub(super) fn delegation(tool: &ToolItem) -> Option<DelegateCall> {
    (tool.kind == ToolItemKind::Call)
        .then(|| delegate_call(&tool.call))
        .flatten()
}

/// Pair each delegation in `rows` with its child chat, where the evidence
/// names exactly one (see the module docs).
pub(super) fn link_delegations(
    rows: &[Row],
    parent: &str,
    chats: &[zeron_proto::Chat],
) -> HashMap<String, SharedString> {
    let tasks: Vec<&zeron_proto::Chat> = chats
        .iter()
        .filter(|chat| {
            chat.parent_chat_id.as_deref() == Some(parent)
                && chat.orchestration.as_ref().is_some_and(|o| o.task.is_some())
        })
        .collect();
    let calls: Vec<(&ToolItem, DelegateCall)> = rows
        .iter()
        .filter_map(|row| match &row.kind {
            RowKind::ToolGroup { tools, .. } => Some(tools.iter()),
            _ => None,
        })
        .flatten()
        .filter_map(|tool| Some((tool, delegation(tool)?)))
        .collect();
    // Keyless titles: a title two calls share names neither of them.
    let mut keyless_titles: HashMap<&str, usize> = HashMap::new();
    for (tool, call) in &calls {
        if call.request_id.is_none()
            && !tool.is_error
            && let Some(title) = call.title.as_deref()
        {
            *keyless_titles.entry(title.trim()).or_default() += 1;
        }
    }
    let mut links = HashMap::new();
    for (tool, call) in &calls {
        let child = match call.request_id.as_deref() {
            Some(key) => {
                let id = managed_task_id(parent, key);
                tasks.iter().find(|chat| chat.id == id).copied()
            }
            None if tool.is_error => None,
            None => call.title.as_deref().map(str::trim).and_then(|title| {
                if keyless_titles.get(title) != Some(&1) {
                    return None;
                }
                let mut named = tasks
                    .iter()
                    .filter(|chat| chat.title.as_deref().map(str::trim) == Some(title));
                match (named.next(), named.next()) {
                    (Some(chat), None) => Some(*chat),
                    _ => None,
                }
            }),
        };
        if let Some(chat) = child {
            links.insert(tool.part_id.clone(), SharedString::from(chat.id.clone()));
        }
    }
    links
}

impl Transcript {
    /// Re-pair delegations when the rows or the chats moved.
    pub(super) fn refresh_delegate_links(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let key = (self.viewport_layout_revision, state.link_roots_revision, self.rows.len());
        if self.delegate_links_key == Some(key) {
            return;
        }
        self.delegate_links_key = Some(key);
        self.delegate_links = match self.chat_id.as_deref() {
            Some(parent) => link_delegations(&self.rows, parent, &state.chats),
            None => HashMap::new(),
        };
    }

    /// One delegation line: the card a native spawn wears, naming its child
    /// with the live pill instead of the call's text.
    pub(super) fn render_delegation(
        &mut self,
        tool: &ToolItem,
        call: &DelegateCall,
        key: SharedString,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity_id();
        let linked = self.delegate_links.get(&tool.part_id).cloned();
        let child = {
            let state = self.state.read(cx);
            linked
                .as_deref()
                .map(|id| ChatRef::resolve(state, id))
                .filter(|chat| chat.known)
        };
        // Until the child is known the line names what was asked for; the
        // call in flight is the first live cue.
        let pill_chat = child.clone().unwrap_or_else(|| {
            let mut placeholder = ChatRef::unknown(&tool.part_id);
            placeholder.title = call
                .title
                .clone()
                .unwrap_or_else(|| "Task".into())
                .into();
            placeholder
        });
        let starting = !tool.resolved && !tool.is_error;
        let failed = tool.is_error || child.as_ref().is_some_and(|c| c.mark == Mark::Failed);
        let facts: Vec<&'static str> = [
            call.role.as_deref().and_then(role_word),
            call.isolation
                .as_deref()
                .filter(|isolation| *isolation == "worktree")
                .map(|_| "Own worktree"),
        ]
        .into_iter()
        .flatten()
        .collect();
        let model = child
            .as_ref()
            .and_then(|c| c.model.clone())
            .or_else(|| call.model.clone().map(SharedString::from));
        let open_id = child.as_ref().map(|c| c.chat_id.clone());
        let spoken = format!(
            "Delegated {}{}",
            pill_chat.spoken(),
            if starting { ", starting" } else { "" }
        );
        div()
            .h(px(CHIP_HEIGHT))
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .child(
                div()
                    .id(key.clone())
                    .h(px(CHIP_CARD_HEIGHT))
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .overflow_hidden()
                    .rounded(px(9.0))
                    .border_1()
                    .border_color(crate::theme::hairline(0.07))
                    .bg(crate::theme::ink(0.03))
                    .text_size(px(TOOL_LABEL_SIZE))
                    .line_height(px(TOOL_LABEL_LINE_HEIGHT))
                    .text_color(theme.text_muted)
                    .tooltip(crate::settings::widgets::text_tooltip(spoken))
                    .when_some(open_id, |card, chat_id| {
                        card.cursor_pointer()
                            .hover(|s| s.bg(crate::theme::ink(0.05)))
                            .on_click(cx.listener(move |_, _, window, cx| {
                                cx.stop_propagation();
                                cx.emit(TranscriptEvent::OpenChat {
                                    chat_id: chat_id.to_string(),
                                    main_view: window.modifiers().platform,
                                });
                            }))
                    })
                    .child(
                        div()
                            .size(px(18.0))
                            .flex_none()
                            .rounded(px(5.0))
                            .bg(crate::theme::ink(0.08))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                crate::icons::icon(crate::icons::BOT)
                                    .size(px(12.0))
                                    .text_color(if failed { theme.danger } else { theme.text_muted }),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(if failed { theme.danger } else { theme.text_muted })
                            .child("Delegated"),
                    )
                    .child(chat_pill::pill(
                        &format!("{key}-pill"),
                        &pill_chat,
                        child.is_some(),
                        view,
                        theme,
                        cx,
                    ))
                    .children(facts.into_iter().map(|fact| {
                        div()
                            .flex_none()
                            .text_color(theme.text_faint)
                            .child(fact)
                    }))
                    .when_some(model, |el, model| {
                        el.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_color(theme.text_faint)
                                .child(model),
                        )
                    })
                    .child(div().flex_1())
                    .when(starting && child.is_none(), |el| {
                        el.child(crate::loaders::mini_glyph_spinner(
                            format!("{key}-starting"),
                            2.0,
                            theme.glyph,
                            view,
                            cx,
                        ))
                    })
                    .when(child.is_some(), |el| {
                        el.child(
                            crate::icons::icon(crate::icons::ARROW_UP_RIGHT)
                                .size(px(12.0))
                                .flex_none()
                                .text_color(theme.text_faint),
                        )
                    }),
            )
            .into_any_element()
    }
}

fn role_word(role: &str) -> Option<&'static str> {
    Some(match role {
        "implementation" => "Implementation",
        "research" => "Research",
        "review" => "Review",
        "design" => "Design",
        "test" => "Test",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(id: &str, title: Option<&str>, created_ms: i64) -> zeron_proto::Chat {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "deviceId": "dev",
            "archived": false,
            "createdAt": chrono::DateTime::<chrono::Utc>::from_timestamp_millis(created_ms).unwrap(),
            "title": title,
            "parentChatId": "parent",
            "orchestration": {"task": {
                "id": id, "parentChatId": "parent", "role": "general", "isolation": "shared",
                "status": "running", "prompt": "x", "requestFingerprint": "f",
                "createdAt": created_ms, "updatedAt": created_ms
            }}
        }))
        .unwrap()
    }

    fn delegate(part: &str, input: serde_json::Value) -> ToolItem {
        ToolItem {
            part_id: part.into(),
            call: ToolCall::Mcp {
                server: "zeron".into(),
                tool: "delegate_task".into(),
                input: Some(input),
            },
            is_error: false,
            resolved: true,
            detail: None,
            invocation: None,
            output_ref: None,
            output_bytes: None,
            diff_ref: None,
            subagent_ref: None,
            subagent_status: None,
            subagent_tail: None,
            kind: ToolItemKind::Call,
            images: Arc::from(Vec::new()),
        }
    }

    fn group(tools: Vec<ToolItem>) -> Vec<Row> {
        vec![Row {
            id: "e#g0".into(),
            version: 0,
            turn_start: true,
            kind: RowKind::ToolGroup {
                summary: "".into(),
                tools: Arc::new(tools),
                auto_open: false,
                worked_secs: None,
                compact_shell: false,
            },
            entry_id: "e".into(),
            timestamp: None,
            copy_text: None,
            compact_fold: None,
        }]
    }

    #[test]
    fn delegations_link_by_request_key_or_a_unique_title() {
        let keyed = managed_task_id("parent", "audit-1");
        let chats = vec![
            chat("titled", Some("Review the RPC"), 2),
            chat(&keyed, Some("Audit"), 1),
            chat("other", None, 3),
        ];
        let links = link_delegations(
            &group(vec![
                delegate("p-keyed", serde_json::json!({"client_request_id": "audit-1"})),
                delegate("p-titled", serde_json::json!({"title": "Review the RPC"})),
                delegate("p-plain", serde_json::json!({})),
            ]),
            "parent",
            &chats,
        );
        assert_eq!(links["p-keyed"].as_ref(), keyed);
        assert_eq!(links["p-titled"].as_ref(), "titled");
        // Order is never evidence: the untitled call names no one.
        assert!(!links.contains_key("p-plain"));
    }

    #[test]
    fn a_request_key_is_authoritative_and_ambiguity_links_nothing() {
        // An earlier task titled "Review" exists; a keyed spawn of the same
        // title failed before its child was created.
        let chats = vec![
            chat("old-review", Some("Review"), 1),
            chat("twin-a", Some("Twin"), 2),
            chat("twin-b", Some("Twin"), 3),
            chat("solo", Some("Solo"), 4),
        ];
        let mut failed = delegate(
            "p-failed",
            serde_json::json!({"client_request_id": "review-2", "title": "Review"}),
        );
        failed.is_error = true;
        let mut failed_keyless = delegate("p-failed-keyless", serde_json::json!({"title": "Solo"}));
        failed_keyless.is_error = true;
        let links = link_delegations(
            &group(vec![
                failed,
                delegate("p-pending", serde_json::json!({"client_request_id": "review-3", "title": "Review"})),
                failed_keyless,
                delegate("p-twin", serde_json::json!({"title": "Twin"})),
                delegate("p-solo-1", serde_json::json!({"title": "Solo"})),
                delegate("p-solo-2", serde_json::json!({"title": "Solo"})),
            ]),
            "parent",
            &chats,
        );
        // Keyed calls never fall back to a title.
        assert!(!links.contains_key("p-failed"));
        assert!(!links.contains_key("p-pending"));
        assert!(!links.contains_key("p-failed-keyless"));
        // Two children share the title; two calls share the other.
        assert!(!links.contains_key("p-twin"));
        assert!(!links.contains_key("p-solo-1") && !links.contains_key("p-solo-2"));
    }

    #[test]
    fn spawn_groups_hold_delegations_with_native_spawns() {
        let tool = delegate("p", serde_json::json!({"title": "x"}));
        assert!(is_agent_tool(&tool));
        assert!(delegation(&tool).is_some());
        let cli = ToolItem {
            call: ToolCall::Exec {
                command: "zeron chat spawn --prompt hi --title Audit".into(),
            },
            ..tool
        };
        assert!(is_agent_tool(&cli));
        assert_eq!(delegation(&cli).unwrap().title.as_deref(), Some("Audit"));
    }
}
