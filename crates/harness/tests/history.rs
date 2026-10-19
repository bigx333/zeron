#![cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use zeron_harness::{CodexHarness, Harness};
use zeron_proto::{HarnessId, ProviderPoint};

#[tokio::test]
async fn native_history_fork_sends_exact_boundary_and_never_starts_a_model_turn() {
    let temp = tempfile::tempdir().unwrap();
    let exe = temp.path().join("codex-history");
    std::fs::write(
        &exe,
        r##"#!/bin/sh
read -r line || exit 1
printf '%s\n' "$line" >> history-wire.log
printf '%s\n' '{"id":1,"result":{}}'
read -r line || exit 1
printf '%s\n' "$line" >> history-wire.log
read -r line || exit 1
printf '%s\n' "$line" >> history-wire.log
case "$line" in *'"method":"thread/fork"'*) ;; *) exit 2;; esac
case "$line" in *'"lastTurnId":"saved-turn"'*) ;; *) exit 3;; esac
printf '%s\n' '{"id":2,"result":{"thread":{"id":"independent-fork","historyMode":"paginated"}}}'
read -r line || exit 1
printf '%s\n' "$line" >> history-wire.log
case "$line" in *'"method":"thread/turns/list"'*) ;; *) exit 4;; esac
printf '%s\n' '{"id":3,"result":{"data":[{"id":"saved-turn","status":"completed"}]}}'
while read -r line; do printf '%s\n' "$line" >> history-wire.log; done
"##,
    )
    .unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    let harness = CodexHarness::new().with_executable(exe.clone());
    let source = ProviderPoint {
        harness: HarnessId::Codex,
        session_id: "source-thread".into(),
        cwd: temp.path().display().to_string(),
        turn_id: Some("saved-turn".into()),
    };
    assert_eq!(
        harness.fork_session(&source).await.unwrap(),
        "independent-fork"
    );
    let wire = std::fs::read_to_string(temp.path().join("history-wire.log")).unwrap();
    assert!(wire.contains("\"threadId\":\"source-thread\""));
    assert!(wire.contains("\"excludeTurns\":true"));
    assert!(!wire.contains("turn/start"));
    assert!(!wire.contains("thread/rollback"));
    assert_eq!(source.session_id, "source-thread");
    let fixture = std::fs::read_to_string(&exe).unwrap();
    let wrong_boundary = fixture.replace(
        "\"id\":\"saved-turn\",\"status\"",
        "\"id\":\"later-turn\",\"status\"",
    );
    assert_ne!(fixture, wrong_boundary);
    std::fs::write(&exe, wrong_boundary).unwrap();
    assert!(
        harness
            .fork_session(&source)
            .await
            .unwrap_err()
            .to_string()
            .contains("did not honor")
    );
}

#[tokio::test]
async fn missing_native_boundary_is_rejected_before_a_process_is_started() {
    let harness = CodexHarness::new().with_executable("/missing-history-fixture");
    let source = ProviderPoint {
        harness: HarnessId::Codex,
        session_id: "source".into(),
        cwd: "/tmp".into(),
        turn_id: None,
    };
    assert!(
        harness
            .fork_session(&source)
            .await
            .unwrap_err()
            .to_string()
            .contains("completed provider turn")
    );
}

#[tokio::test]
async fn missing_native_rollout_uses_full_portable_history_instead_of_the_resume_delta() {
    use futures::StreamExt;
    use zeron_proto::{AgentEvent, DoneStatus, RunRequest, SandboxLevel};
    let exe = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-codex.sh");
    let harness = CodexHarness::new().with_executable(exe);
    let (_steer, steering) = tokio::sync::mpsc::channel(1);
    let controls = zeron_harness::RunControls {
        realtime: None,
        account_pool: None,
        execution_lease: None,
        request_input: Box::new(|_| {
            let (_tx, rx) = tokio::sync::oneshot::channel();
            rx
        }),
        steering,
        interrupt: zeron_harness::CancellationToken::new(),
        turn: Default::default(),
    };
    let request = RunRequest {
        prompt: "scenario:resumed-full delta-only".into(),
        harness: Some(HarnessId::Codex),
        model: None,
        reasoning: None,
        model_options: Default::default(),
        cwd: "/tmp".into(),
        sandbox: SandboxLevel::ReadOnly,
        auto_approve: false,
        resume: Some("resume-fail".into()),
        attachments: vec![],
        worktree: None,
        mcp: None,
    };
    let stream = harness
        .run_with_resume_context(
            request,
            controls,
            Some("scenario:resumed-full full-history: PINEAPPLE".into()),
        )
        .await
        .unwrap();
    let events = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        stream.map(Result::unwrap).collect::<Vec<_>>(),
    )
    .await
    .unwrap();
    assert!(events.iter().any(|event| matches!(event, AgentEvent::SessionStarted { session_id, .. } if session_id == "th-fresh")));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::Done {
            status: DoneStatus::Completed,
            ..
        }
    )));
}
