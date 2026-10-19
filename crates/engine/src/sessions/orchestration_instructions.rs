//! Provider-independent fallback for harnesses without a system prompt field.
//! Keep app guidance out of the user's stored message and native commands.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn prepare(
    prompt: &str,
    harness: HarnessId,
    sent: Option<&AtomicBool>,
) -> Option<String> {
    // These providers receive the same app guidance through their native
    // instruction channel on both start and resume, never inside user text.
    if matches!(harness, HarnessId::ClaudeCode | HarnessId::Codex) {
        return None;
    }
    if sent.is_none_or(|sent| sent.load(Ordering::Acquire)) || native_command(prompt, harness) {
        return None;
    }
    Some(format!(
        "<zeron_orchestration_instructions>\n{}\n</zeron_orchestration_instructions>\n\n<user_request>\n{prompt}\n</user_request>",
        zeron_proto::ORCHESTRATION_INSTRUCTIONS.trim()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guidance_requires_tools_and_waits_for_a_normal_prompt() {
        let sent = AtomicBool::new(false);
        for harness in [HarnessId::Codex, HarnessId::ClaudeCode] {
            assert!(prepare("hello", harness, Some(&sent)).is_none());
        }
        for harness in [HarnessId::Opencode, HarnessId::Cursor] {
            assert!(prepare("hello", harness, None).is_none());
            assert!(prepare("/compact", harness, Some(&sent)).is_none());
            assert!(!sent.load(Ordering::Acquire));
            let wire = prepare("review my changes", harness, Some(&sent)).unwrap();
            assert!(wire.contains("<zeron_orchestration_instructions>"));
            assert!(wire.contains("<user_request>\nreview my changes\n</user_request>"));
            assert!(wire.contains("For each delegated review round"));
        }
        sent.store(true, Ordering::Release);
        assert!(prepare("next turn", HarnessId::Codex, Some(&sent)).is_none());
    }

    #[tokio::test]
    async fn portable_bootstrap_excludes_messages_after_the_current_request() {
        let temp = tempfile::tempdir().unwrap();
        let core = crate::EngineCore::assemble(
            temp.path(),
            Arc::new(HarnessRegistry::new()),
            HarnessId::Mock,
            None,
        )
        .unwrap();
        let handle = core.doc_host.open("chat").unwrap();
        handle
            .write_user_message("prior", "earlier context", 1)
            .unwrap();
        let sender = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        handle
            .write_user_message(
                "report",
                &zeron_proto::chat_refs::agent_message(
                    Some("Reviewer"),
                    sender,
                    "The activity context must survive provider switching",
                ),
                2,
            )
            .unwrap();
        let report = handle
            .doc()
            .read_entries()
            .unwrap()
            .into_iter()
            .find(|e| e.id == "report")
            .unwrap();
        assert_eq!(report.role, zeron_doc::MessageRole::System);
        assert!(
            matches!(report.parts.first(), Some(zeron_doc::MessagePart::Activity(activity))
            if activity.source_chat_id == sender && activity.body == "The activity context must survive provider switching")
        );
        handle
            .write_user_message("current", "current request", 3)
            .unwrap();
        handle
            .write_user_message("future", "queued future request", 4)
            .unwrap();
        let prompt = core
            .sessions
            .inner
            .fork_history_prompt(
                "chat",
                handle.doc(),
                HarnessId::Codex,
                "current request",
                Some("current"),
                ProviderSession::Fresh,
            )
            .unwrap();
        assert!(prompt.contains("earlier context"));
        assert!(prompt.contains("The activity context must survive provider switching"));
        assert!(prompt.contains(sender));
        assert!(!prompt.contains("queued future request"));
        assert!(prompt.ends_with("\n\ncurrent request"));
        assert!(
            !handle.doc().context_transfers().unwrap()[0]
                .content
                .contains("current request")
        );
        core.shutdown().await;
    }
}
