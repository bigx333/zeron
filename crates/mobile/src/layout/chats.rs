//! The chats a transcript can name — `@chat:` references in prose and the
//! sources of orchestration reports — resolved from the workspace the client
//! already holds. The layout thread gets a fresh directory whenever the
//! workspace moves and rebuilds only the rows whose references now read
//! differently: a pill or a report follows its chat's title and harness; a
//! delegation line also follows its task's status (its spinner and tone).
//!
//! Taps reach the platform as `zeron-chat:<full id>` links; the platform
//! opens that chat like any other session.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// The link scheme a tappable chat reference carries.
pub const CHAT_LINK_SCHEME: &str = "zeron-chat:";

/// How a named chat reads right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ChatStatus {
    Working,
    NeedsInput,
    Failed,
    Finished,
    Idle,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ChatLabel {
    pub id: String,
    pub title: String,
    /// Wire harness id (`claude-code`).
    pub harness: Option<String>,
    pub status: ChatStatus,
    pub parent: Option<String>,
    /// A managed task (it has a task record), not a plain side chat.
    pub task: bool,
}

impl ChatLabel {
    /// The pill's icon: the harness brand mark (`brand-<harness>`; hosts
    /// that don't know one draw nothing), else the agent glyph.
    pub fn icon(&self) -> String {
        match &self.harness {
            Some(harness) => format!("brand-{harness}"),
            None => "tool-bot".into(),
        }
    }

    pub fn link(&self) -> String {
        format!("{CHAT_LINK_SCHEME}{}", self.id)
    }

    /// The title without a trailing "(Codex)" naming its own harness — the
    /// mark beside it already says so.
    pub fn display_title(&self) -> &str {
        let title = self.title.as_str();
        let Some(harness) = self.harness.as_deref() else {
            return title;
        };
        let names: &[&str] = match harness {
            "claude-code" => &["Claude Code", "Claude"],
            "codex" => &["Codex"],
            "cursor" => &["Cursor"],
            "devin" => &["Devin"],
            "grok" => &["Grok"],
            "hermes" => &["Hermes"],
            "pi" => &["Pi"],
            "opencode" => &["OpenCode"],
            _ => &[],
        };
        let Some(inside) = title.trim_end().strip_suffix(')') else {
            return title;
        };
        let Some(open) = inside.rfind('(') else {
            return title;
        };
        let name = inside[open + 1..].trim();
        if !names.iter().any(|known| known.eq_ignore_ascii_case(name)) {
            return title;
        }
        let stripped = title[..open].trim_end();
        if stripped.is_empty() { title } else { stripped }
    }
}

/// Every chat the workspace knows, by id.
#[derive(Debug, Clone, Default)]
pub(crate) struct ChatDirectory {
    chats: HashMap<String, ChatLabel>,
}

impl ChatDirectory {
    pub fn from_workspace(ws: &zeron_client::WorkspaceSnapshot) -> Self {
        Self::from_rows(ws.sessions.values().map(|row| row.as_ref()))
    }

    pub fn from_rows<'a>(rows: impl Iterator<Item = &'a zeron_client::SessionRow>) -> Self {
        use zeron_proto::ChatIndicator as I;
        let chats = rows
            .map(|row| {
                let task = row.task.as_ref();
                let status = match (task.map(|t| t.status.as_str()), row.indicator) {
                    (Some("waiting"), _) | (_, I::AwaitingInput) => ChatStatus::NeedsInput,
                    (Some("failed"), _) => ChatStatus::Failed,
                    (Some("running" | "queued" | "waiting_for_children"), _) | (_, I::Working) => {
                        ChatStatus::Working
                    }
                    (Some("completed"), _) | (_, I::Completed) => ChatStatus::Finished,
                    (_, I::Errored) => ChatStatus::Failed,
                    _ => ChatStatus::Idle,
                };
                let label = ChatLabel {
                    id: row.id.clone(),
                    title: row.title.split_whitespace().collect::<Vec<_>>().join(" "),
                    harness: row.harness.clone(),
                    status,
                    parent: row.parent_chat_id.clone(),
                    task: task.is_some(),
                };
                (row.id.clone(), label)
            })
            .collect();
        Self { chats }
    }

    #[cfg(test)]
    pub fn from_labels(labels: Vec<ChatLabel>) -> Self {
        Self {
            chats: labels.into_iter().map(|l| (l.id.clone(), l)).collect(),
        }
    }

    /// Exact id, else a unique case-insensitive prefix of at least
    /// [`zeron_proto::chat_refs::CHAT_REF_MIN_PREFIX`] characters.
    pub fn resolve(&self, written: &str) -> Option<&ChatLabel> {
        if let Some(label) = self.chats.get(written) {
            return Some(label);
        }
        if written.len() < zeron_proto::chat_refs::CHAT_REF_MIN_PREFIX {
            return None;
        }
        let mut found = self.chats.values().filter(|label| {
            label.id.len() >= written.len()
                && label.id.is_char_boundary(written.len())
                && label.id[..written.len()].eq_ignore_ascii_case(written)
        });
        match (found.next(), found.next()) {
            (Some(label), None) => Some(label),
            _ => None,
        }
    }

    /// The task a delegation started, only where the evidence names one:
    /// its request key (the engine derives the task id from its exact
    /// bytes) is authoritative — a keyed call whose task isn't here stays
    /// unlinked, and one that reported an error still names the task its
    /// key created — else a title exactly one of `parent`'s managed tasks
    /// carries, when the caller saw that title on exactly one keyless,
    /// unfailed call. Side chats are never delegations, and order is never
    /// evidence: a not-yet-created delegation borrows no child.
    pub fn delegated(
        &self,
        parent: &str,
        call: &zeron_proto::chat_refs::DelegateCall,
        title_is_unique_call: bool,
    ) -> Option<&ChatLabel> {
        if let Some(key) = call.request_id.as_deref() {
            return self
                .chats
                .get(&zeron_proto::chat_refs::managed_task_id(parent, key))
                .filter(|chat| chat.task);
        }
        if !title_is_unique_call {
            return None;
        }
        let title = call.title.as_deref()?.trim();
        let mut named = self
            .chats
            .values()
            .filter(|chat| chat.task && chat.parent.as_deref() == Some(parent) && chat.title == title);
        match (named.next(), named.next()) {
            (Some(chat), None) => Some(chat),
            _ => None,
        }
    }

    /// `parent`'s managed tasks as delegation lines draw them (title,
    /// harness, status), hashed: those lines rebuild when a task arrives,
    /// is renamed or changes status.
    pub fn children_fingerprint(&self, parent: &str) -> u64 {
        let mut children: Vec<&ChatLabel> = self
            .chats
            .values()
            .filter(|c| c.task && c.parent.as_deref() == Some(parent))
            .collect();
        children.sort_by(|a, b| a.id.cmp(&b.id));
        let mut h = std::collections::hash_map::DefaultHasher::new();
        children.hash(&mut h);
        h.finish()
    }

    /// What `written` ids resolve to as pills and report headers draw them
    /// (which chat, its title and harness), hashed: rows naming them rebuild
    /// when this moves. Status isn't drawn there, so it stays out.
    pub fn fingerprint<'a>(&self, written: impl IntoIterator<Item = &'a str>) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for id in written {
            id.hash(&mut h);
            self.resolve(id)
                .map(|chat| (&chat.id, &chat.title, &chat.harness))
                .hash(&mut h);
        }
        h.finish()
    }
}

/// The chat ids an entry names: report sources and `@chat:` references in
/// its text (including the headers older hosts wrote).
pub(crate) fn entry_chat_refs(entry: &zeron_doc::schema::SessionMessageEntry) -> Vec<String> {
    use zeron_doc::parts::MessagePart;
    let mut out = Vec::new();
    for part in &entry.parts {
        match part {
            MessagePart::Text { text, .. } => {
                if let Some(result) = zeron_proto::chat_refs::parse_legacy_task_result(text) {
                    out.push(result.child_chat_id.to_owned());
                } else if let Some(message) = zeron_proto::chat_refs::parse_agent_message(text)
                    && let Some(sender) = message.sender_chat_id
                {
                    out.push(sender.to_owned());
                }
                out.extend(
                    zeron_proto::chat_refs::chat_refs(text)
                        .into_iter()
                        .map(|r| r.chat_id.to_owned()),
                );
            }
            MessagePart::Activity(activity) => {
                out.push(activity.source_chat_id.clone());
                out.extend(
                    zeron_proto::chat_refs::chat_refs(&activity.body)
                        .into_iter()
                        .map(|r| r.chat_id.to_owned()),
                );
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(id: &str, title: &str, harness: Option<&str>) -> ChatLabel {
        ChatLabel {
            id: id.into(),
            title: title.into(),
            harness: harness.map(Into::into),
            status: ChatStatus::Idle,
            parent: None,
            task: false,
        }
    }

    fn child(id: &str, title: &str, task: bool) -> ChatLabel {
        ChatLabel {
            parent: Some("parent".into()),
            task,
            ..label(id, title, None)
        }
    }

    fn keyless(title: &str) -> zeron_proto::chat_refs::DelegateCall {
        zeron_proto::chat_refs::DelegateCall {
            title: Some(title.into()),
            ..Default::default()
        }
    }

    #[test]
    fn keyless_titles_name_managed_tasks_never_side_chats() {
        // A side chat titled like the delegation is there first: it is not
        // the delegation's child, whatever its title.
        let side_only = ChatDirectory::from_labels(vec![child("side", "Audit", false)]);
        assert!(side_only.delegated("parent", &keyless("Audit"), true).is_none());
        // Once the task arrives it is the one match; the side chat still
        // doesn't make the title ambiguous.
        let both = ChatDirectory::from_labels(vec![child("side", "Audit", false), child("task", "Audit", true)]);
        assert_eq!(both.delegated("parent", &keyless("Audit"), true).unwrap().id, "task");
        // A title two keyless calls share names nobody.
        assert!(both.delegated("parent", &keyless("Audit"), false).is_none());
        // A side chat's change never rebuilds delegation lines; a task's does.
        let renamed_side = ChatDirectory::from_labels(vec![child("side", "Audit v2", false), child("task", "Audit", true)]);
        assert_eq!(both.children_fingerprint("parent"), renamed_side.children_fingerprint("parent"));
        let mut working = child("task", "Audit", true);
        working.status = ChatStatus::Working;
        let working = ChatDirectory::from_labels(vec![child("side", "Audit", false), working]);
        assert_ne!(both.children_fingerprint("parent"), working.children_fingerprint("parent"));
    }

    #[test]
    fn a_request_key_names_its_own_task_even_after_an_error() {
        // A wait-mode call can fail after its task exists (the watch ended);
        // the key still names exactly that task.
        let key = " round-1 ";
        let id = zeron_proto::chat_refs::managed_task_id("parent", key);
        let dir = ChatDirectory::from_labels(vec![child(&id, "Review", true), child("other", "Review", true)]);
        let call = zeron_proto::chat_refs::DelegateCall {
            request_id: Some(key.into()),
            title: Some("Review".into()),
            ..Default::default()
        };
        assert_eq!(dir.delegated("parent", &call, false).unwrap().id, id);
        // The trimmed spelling is a different key.
        let trimmed = zeron_proto::chat_refs::DelegateCall {
            request_id: Some(key.trim().into()),
            ..call.clone()
        };
        assert!(dir.delegated("parent", &trimmed, true).is_none());
    }

    #[test]
    fn reference_fingerprints_follow_what_pills_draw_not_status() {
        let idle = ChatDirectory::from_labels(vec![label("ba6888c8-0000", "Review", Some("codex"))]);
        let mut busy = label("ba6888c8-0000", "Review", Some("codex"));
        busy.status = ChatStatus::Working;
        let busy = ChatDirectory::from_labels(vec![busy]);
        assert_eq!(idle.fingerprint(["ba6888c8"]), busy.fingerprint(["ba6888c8"]));
        let moved = ChatDirectory::from_labels(vec![label("ba6888c8-0000", "Review", Some("claude-code"))]);
        assert_ne!(idle.fingerprint(["ba6888c8"]), moved.fingerprint(["ba6888c8"]));
    }

    #[test]
    fn references_resolve_exactly_or_by_unique_prefix() {
        let dir = ChatDirectory::from_labels(vec![
            label("3f6b2a18-9c4d", "Audit (Codex)", Some("codex")),
            label("3f6b2a18-1111", "Other", None),
            label("ba6888c8-0000", "Review", Some("claude-code")),
        ]);
        assert_eq!(dir.resolve("3f6b2a18-9c4d").unwrap().display_title(), "Audit");
        assert_eq!(dir.resolve("BA6888C8").unwrap().id, "ba6888c8-0000");
        assert!(dir.resolve("3f6b2a18").is_none(), "ambiguous");
        assert!(dir.resolve("ba6888").is_none(), "too short");
        assert_eq!(dir.resolve("ba6888c8").unwrap().icon(), "brand-claude-code");
        assert_eq!(dir.resolve("3f6b2a18-1111").unwrap().icon(), "tool-bot");
        let before = dir.fingerprint(["ba6888c8"]);
        let renamed = ChatDirectory::from_labels(vec![label("ba6888c8-0000", "Review v2", None)]);
        assert_ne!(before, renamed.fingerprint(["ba6888c8"]));
    }
}
