//! The chats a transcript can name — `@chat:` references in prose and the
//! sources of orchestration reports — resolved from the workspace the client
//! already holds. The layout thread gets a fresh directory whenever the
//! workspace moves and rebuilds only the rows whose references now read
//! differently (a title, a harness, a status word).
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

    /// The child a delegation started, only where the evidence names one:
    /// its request key (the engine derives the task id from it) is
    /// authoritative — a keyed call whose child isn't here stays unlinked —
    /// else a title exactly one of `parent`'s children carries, when the
    /// caller saw that title on exactly one keyless call. Order is never
    /// evidence: a failed or not-yet-created delegation borrows no child.
    pub fn delegated(
        &self,
        parent: &str,
        call: &zeron_proto::chat_refs::DelegateCall,
        title_is_unique_call: bool,
    ) -> Option<&ChatLabel> {
        if let Some(key) = call.request_id.as_deref() {
            return self.chats.get(&zeron_proto::chat_refs::managed_task_id(parent, key));
        }
        if !title_is_unique_call {
            return None;
        }
        let title = call.title.as_deref()?.trim();
        let mut named = self
            .chats
            .values()
            .filter(|chat| chat.parent.as_deref() == Some(parent) && chat.title == title);
        match (named.next(), named.next()) {
            (Some(chat), None) => Some(chat),
            _ => None,
        }
    }

    /// Every chat whose parent is `parent`, hashed: delegation lines rebuild
    /// when a child arrives, is renamed or moves.
    pub fn children_fingerprint(&self, parent: &str) -> u64 {
        let mut children: Vec<&ChatLabel> = self.chats.values().filter(|c| c.parent.as_deref() == Some(parent)).collect();
        children.sort_by(|a, b| a.id.cmp(&b.id));
        let mut h = std::collections::hash_map::DefaultHasher::new();
        children.hash(&mut h);
        h.finish()
    }

    /// What `written` ids resolve to, hashed: rows naming them rebuild when
    /// this moves.
    pub fn fingerprint<'a>(&self, written: impl IntoIterator<Item = &'a str>) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for id in written {
            id.hash(&mut h);
            self.resolve(id).hash(&mut h);
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
        }
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
