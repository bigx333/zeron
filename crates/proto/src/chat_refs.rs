//! References between chats in prose: `@chat:<id>` mentions agents write,
//! the attribution headers agent-to-agent messages carry, and the text
//! header older hosts wrote for a delegated task's result. Shared by the
//! engine (which converts headers into transcript activity), the desktop
//! transcript and the mobile layout core, so every surface agrees on what
//! is a reference.

use std::ops::Range;

use crate::orchestration::TaskStatus;

/// The prefix of a chat mention in prose.
pub const CHAT_REF_PREFIX: &str = "@chat:";

/// The shortest id prefix a mention may abbreviate to. Matches the 8-char
/// prefix agents and the CLI print.
pub const CHAT_REF_MIN_PREFIX: usize = 8;

/// `@chat:<id>` for a chat id, the form agents are told to write.
pub fn chat_ref(chat_id: &str) -> String {
    format!("{CHAT_REF_PREFIX}{chat_id}")
}

/// The chat id the engine gives a managed task delegated with a
/// `client_request_id` (the CLI's `--request-id`). Lets a transcript link a
/// `delegate_task` call to its child without a registry round trip.
pub fn managed_task_id(parent_chat_id: &str, client_request_id: &str) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("zeron:orchestration:{parent_chat_id}:{client_request_id}").as_bytes(),
    )
    .to_string()
}

/// One `@chat:` mention. `range` covers the whole written token (prefix,
/// id and any trailing ellipsis) so a pill can stand in for all of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatRef<'a> {
    pub range: Range<usize>,
    /// The written id: a full chat id or a prefix of one. Resolution
    /// (exact, then unique prefix) belongs to whoever knows the chats.
    pub chat_id: &'a str,
    /// The id is shorter than a full UUID.
    pub abbreviated: bool,
}

fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, &c)| match i {
            8 | 13 | 18 | 23 => c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

fn id_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')
}

/// Every `@chat:` mention in `text`.
///
/// - The prefix starts the text or follows a character that isn't
///   alphanumeric, so `foo@chat:…` (an address) never matches.
/// - The id is a full UUID, or at least [`CHAT_REF_MIN_PREFIX`] hex digits
///   with optional dashes, optionally followed by `...` or `…`.
/// - The token ends the text or is followed by a character that can't
///   continue an id; a longer slug (`<id>-draft`, `<id>z`) is not a mention.
///
/// A still-streaming UUID matches as its prefix and grows into the full id,
/// naming the same chat throughout, so a pill never flips identity mid-write.
pub fn chat_refs(text: &str) -> Vec<ChatRef<'_>> {
    if !text.contains(CHAT_REF_PREFIX) {
        return Vec::new();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(found) = text[at..].find(CHAT_REF_PREFIX) {
        let start = at + found;
        let id_start = start + CHAT_REF_PREFIX.len();
        at = id_start;
        if start > 0 && bytes[start - 1].is_ascii_alphanumeric() {
            continue;
        }
        let run_end = text[id_start..]
            .find(|c: char| !c.is_ascii_hexdigit() && c != '-')
            .map_or(text.len(), |rel| id_start + rel);
        // Trailing dashes are punctuation ("see @chat:abcd1234-"), not id.
        let id_end = id_start + text[id_start..run_end].trim_end_matches('-').len();
        let id = &text[id_start..id_end];
        let hex_lead = id.bytes().take_while(u8::is_ascii_hexdigit).count();
        if hex_lead < CHAT_REF_MIN_PREFIX {
            continue;
        }
        let ellipsis = ["...", "…"]
            .iter()
            .find(|dots| text[id_end..].starts_with(**dots))
            .map_or(0, |dots| dots.len());
        let end = id_end + ellipsis;
        // The token is bounded after its ellipsis, or after the whole run
        // (trimmed dashes included) when it has none.
        let boundary = if ellipsis > 0 { end } else { run_end };
        if boundary < text.len() && id_byte(bytes[boundary]) {
            continue;
        }
        let full = is_uuid(id);
        if !full && id.len() > 36 {
            continue;
        }
        out.push(ChatRef {
            range: start..end,
            chat_id: id,
            abbreviated: !full,
        });
        at = end;
    }
    out
}

/// How an attributed message names its sender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMessage<'a> {
    /// The sender's title as written (or its id prefix when untitled).
    pub sender_label: &'a str,
    /// The sender's full chat id. `None` for an external controller, which
    /// isn't a chat.
    pub sender_chat_id: Option<&'a str>,
    /// An MCP client outside Zeron rather than another chat.
    pub external: bool,
    /// Everything after the header's blank line.
    pub body: &'a str,
}

/// The header an agent-to-agent message carries, so the receiving agent
/// can reply and the transcript can attribute it. Pairs with
/// [`parse_agent_message`].
pub fn agent_message(sender_title: Option<&str>, sender_chat_id: &str, text: &str) -> String {
    let label = sender_title
        .map(|title| title.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| sender_chat_id.chars().take(CHAT_REF_MIN_PREFIX).collect());
    format!(
        "[Message from Zeron chat {label} ({CHAT_REF_PREFIX}{sender_chat_id}). \
         Reply with the Zeron `send_message` tool or `zeron chat tell {sender_chat_id} <message>`.]\n\n{text}"
    )
}

/// Decode an attributed message's header. Accepts every form Zeron has
/// written, so old transcripts attribute too:
/// - `[Message from Zeron chat <label> (@chat:<id>). Reply …]` (current)
/// - `[Message from Zeron chat <title> (<id8>). Reply to it with the Zeron
///   \`send_message\` tool, chat <id>.]` (MCP before `@chat:`)
/// - `[Message from external controller <label>]`
pub fn parse_agent_message(text: &str) -> Option<AgentMessage<'_>> {
    if let Some(rest) = text.strip_prefix("[Message from external controller ") {
        let (label, body) = rest.split_once("]\n\n")?;
        return (!label.is_empty() && !label.contains('\n')).then_some(AgentMessage {
            sender_label: label,
            sender_chat_id: None,
            external: true,
            body,
        });
    }
    let rest = text.strip_prefix("[Message from Zeron chat ")?;
    let (header, body) = rest.split_once("]\n\n")?;
    if header.contains('\n') {
        return None;
    }
    // Current: the label is followed by ` (@chat:<id>)`.
    if let Some((label, tail)) = header.split_once(&format!(" ({CHAT_REF_PREFIX}")) {
        let id_len = tail.find(')')?;
        let id = &tail[..id_len];
        if id.is_empty() || !id.bytes().all(id_byte) {
            return None;
        }
        return Some(AgentMessage {
            sender_label: label,
            sender_chat_id: Some(id),
            external: false,
            body,
        });
    }
    // MCP before `@chat:`: the reply instruction ends with the full id.
    let (label, id) =
        header.rsplit_once(". Reply to it with the Zeron `send_message` tool, chat ")?;
    let id = id.strip_suffix('.')?;
    if id.is_empty() || !id.bytes().all(id_byte) {
        return None;
    }
    let short: String = id.chars().take(CHAT_REF_MIN_PREFIX).collect();
    let label = label
        .strip_suffix(&format!(" ({short})"))
        .filter(|name| !name.is_empty())
        .unwrap_or(label);
    Some(AgentMessage {
        sender_label: label,
        sender_chat_id: Some(id),
        external: false,
        body,
    })
}

/// A delegated task's result as hosts before typed activity wrote it into
/// the parent: a user message with a bracketed header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyTaskResult<'a> {
    pub task_id: &'a str,
    pub child_chat_id: &'a str,
    /// `None` for a status label this build doesn't know.
    pub status: Option<TaskStatus>,
    pub body: &'a str,
}

/// Decode `[Delegated task <id> finished: <Status>. Child chat: <id>. This is
/// an agent result, not a new user instruction.]\n\n<result>`.
pub fn parse_legacy_task_result(text: &str) -> Option<LegacyTaskResult<'_>> {
    let rest = text.strip_prefix("[Delegated task ")?;
    let (header, body) = rest.split_once("]\n\n")?;
    let header =
        header.strip_suffix(". This is an agent result, not a new user instruction.")?;
    let (task_id, rest) = header.split_once(" finished: ")?;
    let (label, child_chat_id) = rest.rsplit_once(". Child chat: ")?;
    if [task_id, child_chat_id]
        .iter()
        .any(|id| id.is_empty() || !id.bytes().all(id_byte))
    {
        return None;
    }
    Some(LegacyTaskResult {
        task_id,
        child_chat_id,
        status: task_status_from_label(label),
        body,
    })
}

/// A managed delegation as a transcript shows it: Zeron's `delegate_task`
/// tool (any harness's MCP spelling) or the agent CLI's `zeron chat spawn`.
/// Carries only the short identifiers that link the call to its child chat
/// and label it, never the brief.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DelegateCall {
    pub title: Option<String>,
    /// `client_request_id` / `--request-id`: with the parent's id this
    /// yields the child's id ([`managed_task_id`]).
    pub request_id: Option<String>,
    pub role: Option<String>,
    pub isolation: Option<String>,
    pub harness: Option<String>,
    pub model: Option<String>,
}

/// Input keys a doc keeps on a `delegate_task` call (see
/// `zeron_doc::sanitize_tool_call`). Nested `target` keeps only these.
pub const DELEGATE_INPUT_KEEP: [&str; 4] = ["title", "client_request_id", "role", "isolation"];
pub const DELEGATE_TARGET_KEEP: [&str; 2] = ["harness", "model"];

/// The Zeron MCP tool a call names, whatever the harness's spelling:
/// `Mcp { server: "zeron", tool }`, `mcp__zeron__<tool>`, `zeron_<tool>`.
pub fn zeron_tool(call: &crate::ToolCall) -> Option<&str> {
    match call {
        crate::ToolCall::Mcp { server, tool, .. } => {
            let server = server.to_ascii_lowercase();
            (server == "zeron" || server.starts_with("zeron-") || server.starts_with("zeron_"))
                .then_some(tool.as_str())
                .or_else(|| zeron_tool_name(tool))
        }
        crate::ToolCall::Unknown { name, .. } => zeron_tool_name(name),
        _ => None,
    }
}

fn zeron_tool_name(name: &str) -> Option<&str> {
    ["mcp__zeron__", "zeron__", "zeron_", "zeron."]
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix))
        .filter(|tool| !tool.is_empty())
}

/// The delegation a call performs, if it is one.
pub fn delegate_call(call: &crate::ToolCall) -> Option<DelegateCall> {
    if zeron_tool(call) == Some("delegate_task") {
        let input = match call {
            crate::ToolCall::Mcp { input, .. } | crate::ToolCall::Unknown { input, .. } => {
                input.as_ref()
            }
            _ => None,
        };
        fn raw<'a>(value: Option<&'a serde_json::Value>, key: &str) -> Option<&'a str> {
            value
                .and_then(|v| v.get(key))
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.trim().is_empty())
        }
        let field = |value: Option<&serde_json::Value>, key: &str| {
            raw(value, key).map(|s| s.trim().to_owned())
        };
        let target = input.and_then(|v| v.get("target"));
        return Some(DelegateCall {
            title: field(input, "title"),
            // The engine derives the child's id from the key's exact bytes:
            // only labels are tidied.
            request_id: raw(input, "client_request_id").map(str::to_owned),
            role: field(input, "role"),
            isolation: field(input, "isolation"),
            harness: field(target, "harness"),
            model: field(target, "model"),
        });
    }
    let crate::ToolCall::Exec { command } = call else {
        return None;
    };
    cli_spawn(command)
}

/// `zeron chat spawn --prompt … --request-id … --title …` anywhere in a
/// command line (after `cd … &&`, behind an absolute path).
fn cli_spawn(command: &str) -> Option<DelegateCall> {
    if !command.contains("chat") || !command.contains("spawn") {
        return None;
    }
    let words = shell_words(command);
    // The program starts a command: the line, an operator or an env prefix.
    let starts_command = |ix: usize| {
        ix == 0 || {
            let prev = words[ix - 1].as_str();
            matches!(prev, "&&" | "||" | ";" | "|" | "&")
                || (prev.contains('=') && !prev.starts_with('-') && starts_command_env(&words, ix - 1))
        }
    };
    let at = (0..words.len().saturating_sub(2)).find(|&ix| {
        (words[ix] == "zeron" || words[ix].ends_with("/zeron"))
            && words[ix + 1] == "chat"
            && words[ix + 2] == "spawn"
            && starts_command(ix)
    })?;
    let mut call = DelegateCall::default();
    let mut args = words[at + 3..].iter();
    while let Some(word) = args.next() {
        if matches!(word.as_str(), "&&" | "||" | ";" | "|") {
            break;
        }
        let (flag, inline) = match word.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_owned())),
            _ => (word.as_str(), None),
        };
        let slot = match flag {
            "--title" => &mut call.title,
            "--request-id" => &mut call.request_id,
            "--role" => &mut call.role,
            "--isolation" => &mut call.isolation,
            "--harness" => &mut call.harness,
            "--model" => &mut call.model,
            _ => continue,
        };
        *slot = inline
            .or_else(|| args.next().cloned())
            .filter(|value| !value.trim().is_empty());
    }
    Some(call)
}

/// `words[ix]` is one of the `NAME=value` assignments leading a command.
fn starts_command_env(words: &[String], ix: usize) -> bool {
    let mut at = ix;
    loop {
        if at == 0 || matches!(words[at - 1].as_str(), "&&" | "||" | ";" | "|" | "&") {
            return true;
        }
        let prev = &words[at - 1];
        if !(prev.contains('=') && !prev.starts_with('-')) {
            return false;
        }
        at -= 1;
    }
}

/// POSIX-ish word splitting: quotes, backslash escapes and the control
/// operators, which are their own words. Enough to read flags back out of a
/// command an agent wrote; never used to run anything.
fn shell_words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    word.push(c);
                }
            }
            '"' => {
                in_word = true;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' if matches!(chars.peek(), Some('"' | '\\' | '$' | '`')) => {
                            word.extend(chars.next());
                        }
                        c => word.push(c),
                    }
                }
            }
            '\\' => {
                in_word = true;
                word.extend(chars.next().filter(|c| *c != '\n'));
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            ';' | '|' | '&' => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
                let mut op = c.to_string();
                if (c == '|' || c == '&') && chars.peek() == Some(&c) {
                    op.extend(chars.next());
                }
                words.push(op);
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

/// The inverse of [`TaskStatus::label`].
pub fn task_status_from_label(label: &str) -> Option<TaskStatus> {
    use TaskStatus::*;
    [
        Queued,
        Running,
        Waiting,
        WaitingForChildren,
        Completed,
        Failed,
        Cancelled,
        Interrupted,
    ]
    .into_iter()
    .find(|status| status.label() == label)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "3f6b2a18-9c4d-4e5f-8a7b-1c2d3e4f5a6b";

    fn ids(text: &str) -> Vec<(&str, &str, bool)> {
        chat_refs(text)
            .into_iter()
            .map(|m| (&text[m.range.clone()], m.chat_id, m.abbreviated))
            .collect()
    }

    #[test]
    fn full_ids_and_prefixes_match_at_token_boundaries() {
        let text = format!("Started @chat:{ID} to review.");
        assert_eq!(ids(&text), [(&*format!("@chat:{ID}"), ID, false)]);
        // Upper case, parentheses, two in a line.
        let up = ID.to_uppercase();
        let text = format!("(@chat:{up}) and @chat:{ID}");
        assert_eq!(ids(&text).len(), 2);
        assert_eq!(ids(&text)[0].1, up);
        // The prefixes agents type, with or without an ellipsis.
        assert_eq!(
            ids("Launched @chat:ba6888c8... and @chat:deadbeef… then @chat:0fd51606."),
            [
                ("@chat:ba6888c8...", "ba6888c8", true),
                ("@chat:deadbeef…", "deadbeef", true),
                ("@chat:0fd51606", "0fd51606", true),
            ]
        );
        // A trailing dash is punctuation.
        assert_eq!(ids("see @chat:ba6888c8-"), [("@chat:ba6888c8", "ba6888c8", true)]);
        // A UUID mid-stream names its prefix; the same chat once complete.
        assert_eq!(
            ids("@chat:3f6b2a18-9c4d"),
            [("@chat:3f6b2a18-9c4d", "3f6b2a18-9c4d", true)]
        );
    }

    #[test]
    fn non_mentions_stay_text() {
        for text in [
            format!("foo@chat:{ID}"),
            format!("@chat:{ID}z"),
            format!("@chat:{ID}_x"),
            "@chat:ba6888c".to_owned(),
            "@chat:ba6888c8...x".to_owned(),
            "@chat:ba6888c8z".to_owned(),
            "@chat:".to_owned(),
            "@chat:release-notes".to_owned(),
            format!("@chat:{ID}{ID}"),
        ] {
            assert!(chat_refs(&text).is_empty(), "{text}");
        }
    }

    #[test]
    fn agent_headers_round_trip_in_every_written_form() {
        let wire = agent_message(Some("  Review\nthe  merge "), ID, "look at this");
        let parsed = parse_agent_message(&wire).unwrap();
        assert_eq!(parsed.sender_label, "Review the merge");
        assert_eq!(parsed.sender_chat_id, Some(ID));
        assert_eq!(parsed.body, "look at this");
        assert!(!parsed.external);
        // Untitled senders are labelled with the id prefix.
        let untitled = agent_message(None, ID, "hi");
        assert_eq!(parse_agent_message(&untitled).unwrap().sender_label, "3f6b2a18");
        // The MCP header before `@chat:` carried the full id at its end.
        let legacy = format!(
            "[Message from Zeron chat Main (3f6b2a18). Reply to it with the Zeron `send_message` tool, chat {ID}.]\n\nbody here"
        );
        let parsed = parse_agent_message(&legacy).unwrap();
        assert_eq!(parsed.sender_label, "Main");
        assert_eq!(parsed.sender_chat_id, Some(ID));
        assert_eq!(parsed.body, "body here");
        let bare = format!(
            "[Message from Zeron chat 3f6b2a18. Reply to it with the Zeron `send_message` tool, chat {ID}.]\n\nb"
        );
        assert_eq!(parse_agent_message(&bare).unwrap().sender_label, "3f6b2a18");
        // PR #608's CLI header.
        let cli = format!(
            "[Message from Zeron chat Reviewer (@chat:{ID}). Reply with `zeron chat tell 3f6b2a18 <message>`.]\n\nok"
        );
        assert_eq!(parse_agent_message(&cli).unwrap().sender_chat_id, Some(ID));
        let external = "[Message from external controller CI bot]\n\nship it";
        let parsed = parse_agent_message(external).unwrap();
        assert!(parsed.external);
        assert_eq!(parsed.sender_label, "CI bot");
        assert_eq!(parsed.sender_chat_id, None);
        for text in [
            "hello",
            "[Message from Zeron chat x]\n\nno reply tail",
            "mid [Message from Zeron chat a (@chat:abc). Reply]\n\nb",
            "[Message from Zeron chat a (@chat:a/b). Reply]\n\nb",
        ] {
            assert_eq!(parse_agent_message(text), None, "{text}");
        }
    }

    #[test]
    fn legacy_task_results_parse_with_their_status() {
        let text = format!(
            "[Delegated task {ID} finished: Completed. Child chat: {ID}. This is an agent result, not a new user instruction.]\n\nAll green."
        );
        let parsed = parse_legacy_task_result(&text).unwrap();
        assert_eq!(parsed.task_id, ID);
        assert_eq!(parsed.child_chat_id, ID);
        assert_eq!(parsed.status, Some(TaskStatus::Completed));
        assert_eq!(parsed.body, "All green.");
        let waiting = text.replace("Completed", "Waiting for tasks");
        assert_eq!(
            parse_legacy_task_result(&waiting).unwrap().status,
            Some(TaskStatus::WaitingForChildren)
        );
        let unknown = text.replace("Completed", "Evaporated");
        assert_eq!(parse_legacy_task_result(&unknown).unwrap().status, None);
        assert!(parse_legacy_task_result("[Delegated task x finished: Completed.]\n\nb").is_none());
    }

    #[test]
    fn delegations_are_recognised_in_every_spelling() {
        use crate::ToolCall;
        let input = serde_json::json!({
            "task": "the whole brief",
            "title": "Audit the budget",
            "client_request_id": "audit-1",
            "role": "research",
            "target": {"harness": "codex", "model": "gpt-5.3-codex", "reasoning": "high"}
        });
        let expected = DelegateCall {
            title: Some("Audit the budget".into()),
            request_id: Some("audit-1".into()),
            role: Some("research".into()),
            isolation: None,
            harness: Some("codex".into()),
            model: Some("gpt-5.3-codex".into()),
        };
        for call in [
            ToolCall::Mcp {
                server: "zeron".into(),
                tool: "delegate_task".into(),
                input: Some(input.clone()),
            },
            ToolCall::Unknown {
                name: "mcp__zeron__delegate_task".into(),
                input: Some(input.clone()),
            },
            ToolCall::Unknown {
                name: "zeron_delegate_task".into(),
                input: Some(input.clone()),
            },
        ] {
            assert_eq!(delegate_call(&call), Some(expected.clone()), "{call:?}");
        }
        assert_eq!(
            delegate_call(&ToolCall::Mcp {
                server: "github".into(),
                tool: "delegate_task".into(),
                input: Some(input),
            }),
            None
        );
        let cli = ToolCall::Exec {
            command: "cd /repo && zeron chat spawn --prompt 'Read the \"lanes\" card' \
                      --request-id=audit-1 --title \"Audit the budget\" --role research \
                      --harness codex --model gpt-5.3-codex --wait | tail -n 5"
                .into(),
        };
        assert_eq!(delegate_call(&cli), Some(DelegateCall { ..expected }));
        let path = ToolCall::Exec {
            command: "/usr/local/bin/zeron chat spawn --prompt x".into(),
        };
        assert_eq!(delegate_call(&path), Some(DelegateCall::default()));
        let env = ToolCall::Exec {
            command: "ZERON_CHAT_ID=abc zeron chat spawn --title T".into(),
        };
        assert_eq!(delegate_call(&env).unwrap().title.as_deref(), Some("T"));
        // Only an invocation counts, not the words in an argument.
        for command in ["zeron chat tell abc hi", "echo zeron chat spawn", "ls"] {
            let call = ToolCall::Exec {
                command: command.into(),
            };
            assert_eq!(delegate_call(&call), None, "{command}");
        }
    }

    #[test]
    fn managed_task_ids_match_the_engine_derivation() {
        let id = managed_task_id("parent", "review-1");
        assert_eq!(id, managed_task_id("parent", "review-1"));
        assert_ne!(id, managed_task_id("parent", "review-2"));
        assert!(is_uuid(&id));
    }

    #[test]
    fn request_keys_keep_their_exact_bytes() {
        use crate::ToolCall;
        // The engine accepts any key that isn't blank and derives the child
        // from its raw bytes, so a padded key names a different child than
        // its trimmed spelling. Titles are labels and are tidied.
        let padded = " round-1 ";
        let mcp = ToolCall::Mcp {
            server: "zeron".into(),
            tool: "delegate_task".into(),
            input: Some(serde_json::json!({
                "client_request_id": padded,
                "title": "  Review round 1 ",
                "role": " review ",
            })),
        };
        let call = delegate_call(&mcp).unwrap();
        assert_eq!(call.request_id.as_deref(), Some(padded));
        assert_eq!(call.title.as_deref(), Some("Review round 1"));
        assert_eq!(call.role.as_deref(), Some("review"));
        assert_eq!(
            managed_task_id("parent", call.request_id.as_deref().unwrap()),
            managed_task_id("parent", padded)
        );
        assert_ne!(
            managed_task_id("parent", padded),
            managed_task_id("parent", padded.trim())
        );
        let cli = ToolCall::Exec {
            command: "zeron chat spawn --prompt x --request-id ' round-1 ' --title 'Review'".into(),
        };
        assert_eq!(delegate_call(&cli).unwrap().request_id.as_deref(), Some(padded));
        let inline = ToolCall::Exec {
            command: "zeron chat spawn --prompt x --request-id=\"\tround-1\"".into(),
        };
        assert_eq!(delegate_call(&inline).unwrap().request_id.as_deref(), Some("\tround-1"));
        // A blank key is no key (the engine refuses it; nothing is derived).
        for blank in [serde_json::json!("   "), serde_json::json!("")] {
            let call = ToolCall::Mcp {
                server: "zeron".into(),
                tool: "delegate_task".into(),
                input: Some(serde_json::json!({"client_request_id": blank})),
            };
            assert_eq!(delegate_call(&call).unwrap().request_id, None);
        }
        let blank_cli = ToolCall::Exec {
            command: "zeron chat spawn --prompt x --request-id '  '".into(),
        };
        assert_eq!(delegate_call(&blank_cli).unwrap().request_id, None);
    }
}
