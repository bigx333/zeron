//! Real host/RPC/document/persistence/Git checks with a fake provider boundary.
use async_trait::async_trait;
use futures::{StreamExt, stream, stream::BoxStream};
use serde_json::json;
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_harness::{Harness, HarnessError, RunControls};
use zeron_proto::{
    AgentEvent, ChatConfig, DoneStatus, HarnessId, Model, ProviderPoint, ReasoningLevel,
    RunCheckpoint, RunRequest, SandboxLevel,
};
use zeron_rpc::{RpcClient, methods};

#[derive(Clone, Default)]
struct Observed {
    requests: Arc<Mutex<Vec<RunRequest>>>,
    fresh_contexts: Arc<Mutex<Vec<Option<String>>>>,
    forks: Arc<Mutex<Vec<ProviderPoint>>>,
    serial: Arc<AtomicUsize>,
}
struct Driver {
    id: HarnessId,
    observed: Observed,
}
#[async_trait]
impl Harness for Driver {
    fn id(&self) -> HarnessId {
        self.id
    }
    fn display_name(&self) -> &str {
        "History fixture"
    }
    fn supports_steering(&self) -> bool {
        false
    }
    fn steering_mode(&self) -> zeron_proto::SteeringMode {
        zeron_proto::SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[]
    }
    fn supports_native_fork(&self) -> bool {
        self.id == HarnessId::Codex
    }
    async fn fork_session(&self, point: &ProviderPoint) -> Result<String, HarnessError> {
        self.observed.forks.lock().unwrap().push(point.clone());
        Ok(format!(
            "native-fork-{}",
            self.observed.serial.fetch_add(1, Ordering::SeqCst)
        ))
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run_with_resume_context(
        &self,
        mut request: RunRequest,
        controls: RunControls,
        fresh_context: Option<String>,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.observed
            .fresh_contexts
            .lock()
            .unwrap()
            .push(fresh_context.clone());
        if request.prompt.contains("lost-rollout")
            && let Some(context) = fresh_context
        {
            request.prompt = context;
            request.resume = None;
        }
        self.run(request, controls).await
    }
    async fn run(
        &self,
        r: RunRequest,
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.observed.requests.lock().unwrap().push(r.clone());
        if r.prompt == "write a change" {
            std::fs::write(Path::new(&r.cwd).join("file.txt"), "generated change").unwrap();
        }
        let serial = self.observed.serial.fetch_add(1, Ordering::SeqCst);
        let session_id = r
            .resume
            .unwrap_or_else(|| format!("native-{:?}-{serial}", self.id));
        let held = r.prompt == "hold";
        let initial = vec![
            Ok(AgentEvent::SessionStarted {
                harness: self.id,
                model: "fixture".into(),
                tools: vec![],
                cwd: r.cwd,
                session_id: session_id.clone(),
                assistant_message_id: format!("assistant-{serial}"),
            }),
            Ok(AgentEvent::ProviderTurn {
                session_id: session_id.clone(),
                turn_id: format!("turn-{serial}"),
            }),
            Ok(AgentEvent::TextDelta {
                text: format!("reply {serial}"),
            }),
        ];
        let done = stream::once(async move {
            if held {
                controls.interrupt.cancelled().await;
            }
            Ok(AgentEvent::Done {
                status: if held {
                    DoneStatus::Interrupted
                } else {
                    DoneStatus::Completed
                },
                result: None,
                error: None,
                session_id: Some(session_id),
            })
        });
        Ok(stream::iter(initial).chain(done).boxed())
    }
}
fn engine(dir: &Path, o: &Observed) -> EngineCore {
    let registry = HarnessRegistry::new();
    for id in [HarnessId::Mock, HarnessId::Codex] {
        registry.register(Arc::new(Driver {
            id,
            observed: o.clone(),
        }));
    }
    EngineCore::assemble(dir, Arc::new(registry), HarnessId::Mock, None).unwrap()
}
fn config(harness: HarnessId) -> ChatConfig {
    ChatConfig {
        harness,
        model: None,
        reasoning: None,
        model_options: Default::default(),
        sandbox: SandboxLevel::WorkspaceWrite,
    }
}
fn chat(core: &EngineCore, id: &str, cwd: &Path, harness: HarnessId) {
    core.workspace
        .create_chat(
            id,
            None,
            Some(&core.device_id),
            Some(config(harness)),
            Some(cwd.display().to_string()),
        )
        .unwrap();
}
async fn git(root: &Path, args: &[&str]) -> Vec<u8> {
    let out = tokio::process::Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
async fn repo(root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    git(root, &["init", "-q"]).await;
    std::fs::write(root.join("file.txt"), "base\n").unwrap();
    std::fs::write(root.join(".gitignore"), "ignored\n").unwrap();
    git(root, &["add", "."]).await;
    git(
        root,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "base",
        ],
    )
    .await;
}
async fn run(core: &EngineCore, id: &str, prompt: &str, harness: HarnessId, cwd: &Path) {
    core.workspace
        .set_chat_config(id, &config(harness))
        .unwrap();
    core.sessions
        .dispatch(
            id,
            harness,
            RunRequest {
                prompt: prompt.into(),
                harness: Some(harness),
                model: None,
                reasoning: None,
                model_options: Default::default(),
                cwd: cwd.display().to_string(),
                sandbox: SandboxLevel::WorkspaceWrite,
                auto_approve: false,
                resume: None,
                attachments: vec![],
                worktree: None,
                mcp: None,
            },
            None,
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if core
                .sessions
                .session_status(id)
                .is_some_and(|s| s.status == zeron_proto::SessionStatus::Idle)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
async fn checkpoints(rpc: &RpcClient, chat: &str) -> Vec<RunCheckpoint> {
    rpc.call_as(methods::CHAT_CHECKPOINTS, json!({"chatId":chat}))
        .await
        .unwrap()
}
async fn restore(rpc: &RpcClient, chat: &str, point: &str, key: &str) -> String {
    rpc.call(
        methods::ROLLBACK_CHAT,
        json!({"chatId":chat,"checkpointId":point,"clientRequestId":key}),
    )
    .await
    .unwrap()["recoveryCheckpointId"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn checkpoint_restore_and_undo_survive_restart_preserve_index_and_ignored_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    repo(&root).await;
    let state = temp.path().join("state");
    let o = Observed::default();
    let core = engine(&state, &o);
    chat(&core, "chat", &root, HarnessId::Codex);
    std::fs::write(root.join("file.txt"), b"first\r\n").unwrap();
    std::fs::write(root.join("untracked.txt"), "first untracked").unwrap();
    run(&core, "chat", "first", HarnessId::Codex, &root).await;
    let rpc = zeron_rpc::memory_client(core.rpc_service());
    let first = checkpoints(&rpc, "chat").await.remove(0);
    assert!(first.files_available);
    assert_eq!(first.provider_strategy, "native");
    std::fs::write(root.join("file.txt"), "second").unwrap();
    git(&root, &["add", "file.txt"]).await;
    std::fs::write(root.join("file.txt"), "second unstaged").unwrap();
    std::fs::write(root.join("new.txt"), "new").unwrap();
    std::fs::write(root.join("ignored"), "private and newer").unwrap();
    run(&core, "chat", "second", HarnessId::Codex, &root).await;
    let index = std::fs::read(root.join(".git/index")).unwrap();
    let mut restricted = config(HarnessId::Codex);
    restricted.sandbox = SandboxLevel::ReadOnly;
    core.workspace.set_chat_config("chat", &restricted).unwrap();
    let head = git(&root, &["rev-parse", "HEAD"]).await;
    core.doc_host
        .open("chat")
        .unwrap()
        .doc()
        .update_context_usage(Some(9000), Some(10000))
        .unwrap();
    core.shutdown().await;
    drop(rpc);
    drop(core);
    let core = engine(&state, &o);
    let rpc = zeron_rpc::memory_client(core.rpc_service());
    let recovery = restore(&rpc, "chat", &first.id, "restore-first").await;
    assert!(
        core.doc_host
            .open("chat")
            .unwrap()
            .doc()
            .context_usage()
            .is_none()
    );
    assert_eq!(
        core.workspace
            .chat("chat")
            .unwrap()
            .unwrap()
            .config
            .unwrap()
            .sandbox,
        SandboxLevel::ReadOnly
    );
    assert_eq!(std::fs::read(root.join("file.txt")).unwrap(), b"first\r\n");
    assert!(!root.join("new.txt").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("ignored")).unwrap(),
        "private and newer"
    );
    assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(git(&root, &["rev-parse", "HEAD"]).await, head);
    assert_eq!(
        o.forks.lock().unwrap()[0],
        first.provider_point.clone().unwrap()
    );
    let count = checkpoints(&rpc, "chat").await.len();
    assert_eq!(
        restore(&rpc, "chat", &first.id, "restore-first").await,
        recovery
    );
    assert_eq!(checkpoints(&rpc, "chat").await.len(), count);
    assert_eq!(o.forks.lock().unwrap().len(), 1);
    let undo_recovery = restore(&rpc, "chat", &recovery, "undo-first").await;
    assert_ne!(undo_recovery, recovery);
    assert_eq!(
        std::fs::read_to_string(root.join("file.txt")).unwrap(),
        "second unstaged"
    );
    assert!(root.join("new.txt").exists());
    let entries = core
        .doc_host
        .open("chat")
        .unwrap()
        .doc()
        .read_entries()
        .unwrap();
    assert!(entries.iter().any(|e| {
        e.parts
            .iter()
            .any(|p| matches!(p,zeron_doc::MessagePart::Text { text,.. } if text == "second"))
    }));
    assert_eq!(
        o.requests.lock().unwrap().len(),
        2,
        "Restore must never execute a model turn"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn an_unfinished_native_turn_saves_portable_history_and_restores_without_that_provider() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    repo(&root).await;
    let state = temp.path().join("state");
    let o = Observed::default();
    let core = engine(&state, &o);
    chat(&core, "chat", &root, HarnessId::Codex);
    run(&core, "chat", "first", HarnessId::Codex, &root).await;
    let handle = core.doc_host.open("chat").unwrap();
    let assistant = handle
        .doc()
        .read_entries()
        .unwrap()
        .last()
        .unwrap()
        .id
        .clone();
    handle.doc().push_message(&serde_json::from_value(json!({
        "id":"continued-tail", "role":"assistant", "parts":[{"kind":"text","id":"tail-text","text":"continued output"}],
        "createdAt":9,"deviceId":core.device_id,"status":"complete","continuationOf":assistant
    })).unwrap()).unwrap();
    let mut point = handle.doc().provider_sessions().remove(0).point;
    handle
        .doc()
        .remember_provider(point.clone(), Some("continued-tail"))
        .unwrap();
    handle
        .doc()
        .record_root_run_outcome(&zeron_proto::RootRunOutcome {
            status: DoneStatus::Completed,
            message_id: "continued-tail".into(),
            finished_at: 9,
        })
        .unwrap();
    let rpc = zeron_rpc::memory_client(core.rpc_service());
    let continued: RunCheckpoint = rpc
        .call_as(methods::CREATE_CHAT_CHECKPOINT, json!({"chatId":"chat"}))
        .await
        .unwrap();
    assert_eq!(
        continued.provider_strategy, "native",
        "continuation entries still belong to a completed native turn"
    );
    assert_eq!(continued.message_id.as_deref(), Some(assistant.as_str()));
    point.turn_id = Some("unfinished-after-crash".into());
    handle.doc().remember_provider(point, None).unwrap();
    handle.doc().push_message(&serde_json::from_value(json!({
            "id":"unanswered", "role":"user", "parts":[{"kind":"text","id":"text","text":"unfinished request"}],
        "createdAt":10,"deviceId":core.device_id,"status":"complete"
    })).unwrap()).unwrap();
    let saved: RunCheckpoint = rpc
        .call_as(methods::CREATE_CHAT_CHECKPOINT, json!({"chatId":"chat"}))
        .await
        .unwrap();
    assert_eq!(
        saved.provider_strategy, "portable",
        "a started native turn is not a completed checkpoint boundary"
    );
    core.shutdown().await;
    drop(rpc);
    drop(handle);
    drop(core);
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(Driver {
        id: HarnessId::Mock,
        observed: o.clone(),
    }));
    let core = EngineCore::assemble(&state, Arc::new(registry), HarnessId::Mock, None).unwrap();
    let rpc = zeron_rpc::memory_client(core.rpc_service());
    restore(&rpc, "chat", &saved.id, "portable-without-codex").await;
    assert!(o.forks.lock().unwrap().is_empty());
    assert!(
        core.doc_host
            .open("chat")
            .unwrap()
            .doc()
            .read_entries()
            .unwrap()
            .iter()
            .any(|entry| entry.id == "unanswered")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn switching_back_resumes_the_original_provider_with_only_its_missing_delta() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let o = Observed::default();
    let core = engine(&state, &o);
    chat(&core, "chat", temp.path(), HarnessId::Mock);
    run(
        &core,
        "chat",
        "first-provider-message",
        HarnessId::Mock,
        temp.path(),
    )
    .await;
    run(
        &core,
        "chat",
        "second-provider-message",
        HarnessId::Codex,
        temp.path(),
    )
    .await;
    let original = core
        .doc_host
        .open("chat")
        .unwrap()
        .doc()
        .provider_sessions()
        .into_iter()
        .find(|s| s.point.harness == HarnessId::Mock)
        .unwrap();
    core.shutdown().await;
    drop(core);
    let core = engine(&state, &o);
    run(&core, "chat", "back-again", HarnessId::Mock, temp.path()).await;
    let requests = o.requests.lock().unwrap();
    let last = requests.last().unwrap();
    assert_eq!(
        last.resume.as_deref(),
        Some(original.point.session_id.as_str())
    );
    assert!(last.prompt.contains("second-provider-message"));
    assert!(!last.prompt.contains("first-provider-message"));
    drop(requests);
    let transfers = core
        .doc_host
        .open("chat")
        .unwrap()
        .doc()
        .context_transfers()
        .unwrap();
    assert!(
        transfers
            .iter()
            .any(|t| t.kind == "provider_reactivation" && t.status == "consumed")
    );
    run(
        &core,
        "chat",
        "back-to-codex",
        HarnessId::Codex,
        temp.path(),
    )
    .await;
    let requests = o.requests.lock().unwrap();
    let resumed = requests.last().unwrap();
    assert!(resumed.resume.is_some());
    assert!(resumed.prompt.contains("back-again"));
    assert!(!resumed.prompt.contains("first-provider-message"));
    drop(requests);
    let fallbacks = o.fresh_contexts.lock().unwrap();
    let fresh = fallbacks.last().unwrap().as_ref().unwrap();
    assert!(fresh.contains("first-provider-message"));
    assert!(fresh.contains("second-provider-message"));
    assert!(fresh.contains("back-again"));
    drop(fallbacks);
    run(&core, "chat", "lost-rollout", HarnessId::Codex, temp.path()).await;
    let doc = core.doc_host.open("chat").unwrap();
    assert!(
        doc.doc()
            .context_transfers()
            .unwrap()
            .iter()
            .any(|transfer| transfer.kind == "provider_handoff"
                && transfer.content.contains("lost-rollout") == false
                && transfer.content.contains("first-provider-message")
                && transfer.status == "consumed")
    );
    let current = doc
        .doc()
        .provider_session(HarnessId::Codex, &temp.path().display().to_string())
        .unwrap();
    assert_eq!(
        doc.doc().fork_history_session().as_deref(),
        Some(current.point.session_id.as_str())
    );
    core.shutdown().await;
}

#[tokio::test]
async fn native_fork_is_lazy_and_uses_the_selected_completed_turn() {
    let temp = tempfile::tempdir().unwrap();
    let o = Observed::default();
    let core = engine(&temp.path().join("state"), &o);
    chat(&core, "source", temp.path(), HarnessId::Codex);
    run(&core, "source", "first", HarnessId::Codex, temp.path()).await;
    let rpc = zeron_rpc::memory_client(core.rpc_service());
    let first = checkpoints(&rpc, "source").await.remove(0);
    run(&core, "source", "later", HarnessId::Codex, temp.path()).await;
    rpc.call(
        methods::FORK_SIDE_CHAT,
        json!({"chatId":"fork","sourceChatId":"source","sourceMessageId":first.message_id}),
    )
    .await
    .unwrap();
    assert!(o.forks.lock().unwrap().is_empty());
    run(&core, "fork", "follow-up", HarnessId::Codex, temp.path()).await;
    assert_eq!(
        o.forks.lock().unwrap().as_slice(),
        &[first.provider_point.unwrap()]
    );
    let requests = o.requests.lock().unwrap();
    let last = requests.last().unwrap();
    assert!(last.resume.as_ref().unwrap().starts_with("native-fork-"));
    assert_eq!(last.prompt, "follow-up");
    drop(requests);
    core.shutdown().await;
}

#[tokio::test]
async fn ignored_obstruction_refuses_restore_without_touching_current_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    repo(&root).await;
    let o = Observed::default();
    let core = engine(&temp.path().join("state"), &o);
    chat(&core, "chat", &root, HarnessId::Mock);
    std::fs::write(root.join("scratch.txt"), "old").unwrap();
    run(&core, "chat", "first", HarnessId::Mock, &root).await;
    let rpc = zeron_rpc::memory_client(core.rpc_service());
    let point = checkpoints(&rpc, "chat").await.remove(0);
    std::fs::write(root.join(".gitignore"), "ignored\nscratch.txt\n").unwrap();
    std::fs::write(root.join("scratch.txt"), "now private").unwrap();
    let error = rpc
        .call(
            methods::ROLLBACK_CHAT,
            json!({"chatId":"chat","checkpointId":point.id,"clientRequestId":"blocked"}),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("outside the checkpoint"));
    assert_eq!(
        std::fs::read_to_string(root.join("scratch.txt")).unwrap(),
        "now private"
    );
    assert!(
        std::fs::read_to_string(root.join(".gitignore"))
            .unwrap()
            .contains("scratch.txt")
    );
    core.shutdown().await;
}

#[tokio::test]
async fn restore_is_blocked_while_another_chat_owns_the_shared_checkout() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    repo(&root).await;
    let o = Observed::default();
    let core = engine(&temp.path().join("state"), &o);
    chat(&core, "chat", &root, HarnessId::Mock);
    chat(&core, "other", &root, HarnessId::Mock);
    run(&core, "chat", "first", HarnessId::Mock, &root).await;
    let rpc = zeron_rpc::memory_client(core.rpc_service());
    let point = checkpoints(&rpc, "chat").await.remove(0);
    core.sessions
        .dispatch(
            "other",
            HarnessId::Mock,
            RunRequest {
                prompt: "hold".into(),
                harness: Some(HarnessId::Mock),
                model: None,
                reasoning: None,
                model_options: Default::default(),
                cwd: root.display().to_string(),
                sandbox: SandboxLevel::WorkspaceWrite,
                auto_approve: false,
                resume: None,
                attachments: vec![],
                worktree: None,
                mcp: None,
            },
            None,
        )
        .await
        .unwrap();
    let view = checkpoints(&rpc, "chat").await;
    assert!(!view[0].restorable);
    assert!(
        view[0]
            .blocked_reason
            .as_ref()
            .unwrap()
            .contains("checkout")
    );
    let error = rpc
        .call(
            methods::ROLLBACK_CHAT,
            json!({"chatId":"chat","checkpointId":point.id,"clientRequestId":"busy"}),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("checkout"));
    assert_eq!(o.forks.lock().unwrap().len(), 0);
    core.sessions.terminate("other").await.unwrap();
    assert!(checkpoints(&rpc, "chat").await[0].restorable);
    core.shutdown().await;
}

#[tokio::test]
async fn interrupted_restore_receipt_can_resume_without_losing_intervening_edits() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    repo(&root).await;
    let state = temp.path().join("state");
    let o = Observed::default();
    let core = engine(&state, &o);
    chat(&core, "chat", &root, HarnessId::Mock);
    std::fs::write(root.join("file.txt"), "first").unwrap();
    run(&core, "chat", "first", HarnessId::Mock, &root).await;
    let rpc = zeron_rpc::memory_client(core.rpc_service());
    let first = checkpoints(&rpc, "chat").await.remove(0);
    std::fs::write(root.join("file.txt"), "second").unwrap();
    run(&core, "chat", "second", HarnessId::Mock, &root).await;
    let recovery = restore(&rpc, "chat", &first.id, "interrupted").await;
    let directory = std::fs::read_dir(state.join("history"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let receipt = std::fs::read_dir(&directory)
        .unwrap()
        .filter_map(Result::ok)
        .find(|p| p.file_name().to_string_lossy().starts_with("receipt-"))
        .unwrap()
        .path();
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
    value["completed"] = json!(false); // crash after files/doc save, before final receipt
    std::fs::write(&receipt, serde_json::to_vec(&value).unwrap()).unwrap();
    std::fs::write(root.join("file.txt"), "edited after interrupted restore").unwrap();
    assert_eq!(
        restore(&rpc, "chat", &first.id, "interrupted").await,
        recovery
    );
    assert_eq!(
        std::fs::read_to_string(root.join("file.txt")).unwrap(),
        "first"
    );
    let extra = checkpoints(&rpc, "chat")
        .await
        .into_iter()
        .find(|p| p.recovery && p.id != recovery)
        .unwrap();
    restore(&rpc, "chat", &extra.id, "get-intervening-edits").await;
    assert_eq!(
        std::fs::read_to_string(root.join("file.txt")).unwrap(),
        "edited after interrupted restore"
    );
    core.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn file_directory_transitions_symlinks_and_executable_modes_round_trip() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    repo(&root).await;
    std::fs::set_permissions(
        root.join("file.txt"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    symlink("file.txt", root.join("link")).unwrap();
    let outside = temp.path().join("outside");
    std::fs::write(&outside, "outside stays private").unwrap();
    let o = Observed::default();
    let core = engine(&temp.path().join("state"), &o);
    chat(&core, "chat", &root, HarnessId::Mock);
    run(&core, "chat", "first", HarnessId::Mock, &root).await;
    let rpc = zeron_rpc::memory_client(core.rpc_service());
    let first = checkpoints(&rpc, "chat").await.remove(0);
    std::fs::remove_file(root.join("file.txt")).unwrap();
    std::fs::create_dir(root.join("file.txt")).unwrap();
    std::fs::write(root.join("file.txt/child"), "child data").unwrap();
    std::fs::remove_file(root.join("link")).unwrap();
    symlink(&outside, root.join("link")).unwrap();
    let recovery = restore(&rpc, "chat", &first.id, "transition").await;
    assert_eq!(
        std::fs::metadata(root.join("file.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0o111
    );
    assert_eq!(
        std::fs::read_link(root.join("link")).unwrap(),
        Path::new("file.txt")
    );
    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "outside stays private"
    );
    restore(&rpc, "chat", &recovery, "undo-transition").await;
    assert_eq!(
        std::fs::read_to_string(root.join("file.txt/child")).unwrap(),
        "child data"
    );
    assert_eq!(std::fs::read_link(root.join("link")).unwrap(), outside);
    core.shutdown().await;
}

#[tokio::test]
async fn restoring_before_a_published_task_result_preserves_that_result() {
    let temp = tempfile::tempdir().unwrap();
    let o = Observed::default();
    let core = engine(&temp.path().join("state"), &o);
    chat(&core, "controller", temp.path(), HarnessId::Mock);
    chat(&core, "task", temp.path(), HarnessId::Mock);
    run(&core, "task", "first", HarnessId::Mock, temp.path()).await;
    let rpc = zeron_rpc::memory_client(core.rpc_service());
    let first = checkpoints(&rpc, "task").await.remove(0);
    run(&core, "task", "published", HarnessId::Mock, temp.path()).await;
    let entries = zeron_doc::join_continuation_entries(
        core.doc_host
            .open("task")
            .unwrap()
            .doc()
            .read_entries()
            .unwrap(),
    );
    let last = entries.last().unwrap();
    let result_id = last.id.clone();
    let expected = last
        .parts
        .iter()
        .filter_map(|p| match p {
            zeron_doc::MessagePart::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut row = core.workspace.chat("task").unwrap().unwrap();
    row.parent_chat_id = Some("controller".into());
    row.orchestration = Some(zeron_proto::ChatOrchestration {
        fork: None,
        task: Some(zeron_proto::DelegatedTask {
            id: "task".into(),
            parent_chat_id: "controller".into(),
            role: zeron_proto::TaskRole::General,
            isolation: zeron_proto::TaskIsolation::Shared,
            base_ref: None,
            status: zeron_proto::TaskStatus::Completed,
            prompt: String::new(),
            request_fingerprint: "fixture".into(),
            created_at: 1,
            updated_at: 2,
            summary: Some("compact preview".into()),
            summary_truncated: true,
            result_message_id: Some(result_id.clone()),
            delivered: true,
            cancel_requested: false,
        }),
    });
    core.workspace.import_chat_row(&row).unwrap();
    restore(&rpc, "task", &first.id, "rewind-task").await;
    let result: zeron_proto::TaskResult = rpc
        .call_as(
            methods::TASK_RESULT,
            json!({"parentChatId":"controller","taskId":"task"}),
        )
        .await
        .unwrap();
    assert_eq!(result.summary, expected);
    assert_eq!(result.message_id, Some(result_id.clone()));
    assert!(
        !core
            .doc_host
            .open("task")
            .unwrap()
            .doc()
            .read_entries()
            .unwrap()
            .iter()
            .any(|e| e.id == result_id)
    );
    core.shutdown().await;
}

#[tokio::test]
async fn merge_receipts_are_scoped_to_the_restored_conversation_revision() {
    let temp = tempfile::tempdir().unwrap();
    let o = Observed::default();
    let core = engine(&temp.path().join("state"), &o);
    chat(&core, "source", temp.path(), HarnessId::Mock);
    chat(&core, "target", temp.path(), HarnessId::Mock);
    run(&core, "source", "side result", HarnessId::Mock, temp.path()).await;
    run(
        &core,
        "target",
        "before merge",
        HarnessId::Mock,
        temp.path(),
    )
    .await;
    let rpc = zeron_rpc::memory_client(core.rpc_service());
    let point = checkpoints(&rpc, "target").await.remove(0);
    let params = json!({"sourceChatId":"source","targetChatId":"target"});
    let first = rpc.call(methods::MERGE_CHAT, params.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if core
                .doc_host
                .open("target")
                .unwrap()
                .doc()
                .read_commands()
                .unwrap()
                .iter()
                .all(|c| c.status != zeron_doc::SessionCommandStatus::Pending)
                && core
                    .sessions
                    .session_status("target")
                    .is_some_and(|s| s.status == zeron_proto::SessionStatus::Idle)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    restore(&rpc, "target", &point.id, "rewind-before-merge").await;
    let second = rpc.call(methods::MERGE_CHAT, params.clone()).await.unwrap();
    assert_ne!(first["transferId"], second["transferId"]);
    assert_eq!(second, rpc.call(methods::MERGE_CHAT, params).await.unwrap());
    core.shutdown().await;
}

#[tokio::test]
async fn the_first_turn_has_a_durable_baseline_and_portable_restore_starts_fresh() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    repo(&root).await;
    std::fs::write(root.join("file.txt"), "user's existing dirty file").unwrap();
    let state = temp.path().join("state");
    let o = Observed::default();
    let core = engine(&state, &o);
    chat(&core, "chat", &root, HarnessId::Mock);
    run(&core, "chat", "write a change", HarnessId::Mock, &root).await;
    let rpc = zeron_rpc::memory_client(core.rpc_service());
    let baseline = checkpoints(&rpc, "chat")
        .await
        .into_iter()
        .find(|p| p.message_id.is_none())
        .unwrap();
    assert_eq!(baseline.title, "Before first turn");
    assert!(baseline.files_available);
    restore(&rpc, "chat", &baseline.id, "undo-first-turn").await;
    assert_eq!(
        std::fs::read_to_string(root.join("file.txt")).unwrap(),
        "user's existing dirty file"
    );
    assert!(
        core.doc_host
            .open("chat")
            .unwrap()
            .doc()
            .read_entries()
            .unwrap()
            .is_empty()
    );
    core.shutdown().await;
    drop(rpc);
    drop(core);
    let core = engine(&state, &o);
    run(&core, "chat", "after rewind", HarnessId::Mock, &root).await;
    let requests = o.requests.lock().unwrap();
    assert!(requests.last().unwrap().resume.is_none());
    assert_eq!(requests.last().unwrap().prompt, "after rewind");
    drop(requests);
    core.shutdown().await;
}
