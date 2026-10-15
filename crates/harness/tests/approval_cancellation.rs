//! Withdrawn native approvals must release the engine bridge's receiver.
#![cfg(unix)]

use futures::StreamExt;
use std::{path::PathBuf, time::Duration};
use tokio::sync::{mpsc, oneshot};
use zeron_harness::{
    CancellationToken, ClaudeHarness, CodexHarness, Harness, RunControls, SteerMessage,
};
use zeron_proto::{AgentEvent, DoneStatus, RunRequest, SandboxLevel, UserInputAnswer};

#[tokio::test]
async fn withdrawn_approvals_release_only_the_matching_input_and_send_no_reply() {
    // Isolate the opt-in environment setting from all other fixture tests.
    if std::env::var_os("ZERON_TEST_WITHDRAWAL_CHILD").is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "withdrawn_approvals_release_only_the_matching_input_and_send_no_reply",
                "--nocapture",
            ])
            .env("ZERON_TEST_WITHDRAWAL_CHILD", "1")
            .env("ZERON_APPROVAL_MODE", "prompt")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    let fixture = |name: &str| {
        use std::os::unix::fs::PermissionsExt;
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    };
    let harnesses: Vec<Box<dyn Harness>> = vec![
        Box::new(ClaudeHarness::new().with_executable(fixture("fake-claude.sh"))),
        Box::new(CodexHarness::new().with_executable(fixture("fake-codex.sh"))),
    ];
    for harness in harnesses {
        tokio::time::timeout(Duration::from_secs(10), async {
            let dir = tempfile::tempdir().unwrap();
            let (asked, mut questions) = mpsc::unbounded_channel();
            let (steer, steering) = mpsc::channel(4);
            let controls = RunControls {
                realtime: None,
                execution_lease: None,
                request_input: Box::new(move |questions| {
                    let (tx, rx) = oneshot::channel();
                    asked.send((questions, tx)).unwrap();
                    rx
                }),
                steering,
                interrupt: CancellationToken::new(),
                turn: Default::default(),
            };
            let mut stream = harness
                .run(
                    RunRequest {
                        prompt: "scenario:withdraw-approval".into(),
                        cwd: dir.path().to_string_lossy().into(),
                        harness: None,
                        model: None,
                        reasoning: None,
                        model_options: Default::default(),
                        sandbox: SandboxLevel::WorkspaceWrite,
                        auto_approve: false,
                        attachments: vec![],
                        worktree: None,
                        resume: None,
                        mcp: None,
                    },
                    controls,
                )
                .await
                .unwrap();
            let first = questions.recv().await.unwrap();
            let second = questions.recv().await.unwrap();
            let ((_, mut withdrawn), (pending, response)) = if first.0[0].question.contains("first")
            {
                (first, second)
            } else {
                (second, first)
            };
            assert!(pending[0].question.contains("second"));
            assert!(!withdrawn.is_closed());
            assert!(!response.is_closed());
            steer
                .send(SteerMessage::text("withdraw-first"))
                .await
                .unwrap();
            loop {
                let event = stream.next().await.expect("provider continues").unwrap();
                if matches!(event, AgentEvent::TextDelta { ref text } if text == "withdrawn") {
                    break;
                }
            }
            // This is the closure the engine uses to remove its request and
            // emit InputResolved; the other approval must remain answerable.
            withdrawn.closed().await;
            assert!(!response.is_closed(), "unrelated approval was cancelled");
            response
                .send(vec![UserInputAnswer {
                    question_id: pending[0].id.clone(),
                    labels: vec!["Yes".into()],
                }])
                .unwrap();
            let events = stream.map(|event| event.unwrap()).collect::<Vec<_>>().await;
            assert!(
                matches!(
                    events.last(),
                    Some(AgentEvent::Done {
                        status: DoneStatus::Completed,
                        ..
                    })
                ),
                "{}: {events:?}",
                harness.display_name()
            );
        })
        .await
        .expect("approval withdrawal completed without a stale input");
    }
}
