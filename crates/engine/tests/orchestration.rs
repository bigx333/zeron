//! Exercise the real RPC -> registry -> command queue -> runtime -> result path.
use async_trait::async_trait;
use futures::{StreamExt, stream::BoxStream};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_harness::{Harness, HarnessError, RunControls};
use zeron_proto::{
    AgentEvent, Chat, ChatConfig, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest,
    SandboxLevel, TaskStatus,
};
use zeron_rpc::{RpcClient, methods};

struct Capture {
    id: HarnessId,
    requests: Arc<Mutex<Vec<RunRequest>>>,
    release: Arc<Semaphore>,
}
#[async_trait]
impl Harness for Capture {
    fn id(&self) -> HarnessId {
        self.id
    }
    fn display_name(&self) -> &str {
        "Orchestration test"
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
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        r: RunRequest,
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.requests.lock().unwrap().push(r.clone());
        let id = self.id;
        let held = r.prompt.starts_with("You are working on") && r.prompt.contains("hold task");
        let release = self.release.clone();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            let _ = tx.send(Ok(AgentEvent::SessionStarted {
                harness: id,
                model: "test".into(),
                tools: vec![],
                cwd: r.cwd,
                session_id: format!("native-{id:?}"),
                assistant_message_id: uuid::Uuid::new_v4().to_string(),
            }));
            if !r.prompt.contains("empty task") {
                let text = if r.prompt.starts_with("You are working on")
                    && r.prompt.contains("large task output")
                {
                    "🛰️\"\\\u{0001} long result ".repeat(3000)
                } else {
                    "Task result: checked".into()
                };
                let _ = tx.send(Ok(AgentEvent::TextDelta { text }));
            }
            if held {
                tokio::select! {permit = release.acquire()=>{permit.unwrap().forget();}, _ = controls.interrupt.cancelled()=>{}}
            }
            if r.prompt.starts_with("You are working on") && r.prompt.contains("ask task") {
                let response = (controls.request_input)(vec![zeron_proto::UserInputQuestion {
                    id: "q".into(),
                    header: "Review".into(),
                    question: "Continue?".into(),
                    options: vec!["Continue".into()],
                    multi_select: false,
                    prefill: None,
                    multiline: false,
                }]);
                let _ = response.await;
            }
            let cancelled = controls.interrupt.is_cancelled();
            let _ = tx.send(Ok(AgentEvent::Done {
                status: if cancelled {
                    DoneStatus::Interrupted
                } else {
                    DoneStatus::Completed
                },
                result: None,
                error: None,
                session_id: Some(format!("native-{id:?}")),
            }));
        });
        Ok(futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|event| (event, rx))
        })
        .boxed())
    }
}
fn assemble(
    path: &std::path::Path,
    requests: Arc<Mutex<Vec<RunRequest>>>,
    release: Arc<Semaphore>,
) -> EngineCore {
    let registry = HarnessRegistry::new();
    for id in [HarnessId::Mock, HarnessId::Codex] {
        registry.register(Arc::new(Capture {
            id,
            requests: requests.clone(),
            release: release.clone(),
        }));
    }
    EngineCore::assemble(path, Arc::new(registry), HarnessId::Mock, None).unwrap()
}
fn parent(core: &EngineCore, id: &str) {
    core.workspace
        .create_chat(
            id,
            None,
            Some(&core.device_id),
            Some(ChatConfig {
                harness: HarnessId::Mock,
                model: None,
                reasoning: None,
                model_options: Default::default(),
                sandbox: SandboxLevel::WorkspaceWrite,
            }),
            Some("/tmp".into()),
        )
        .unwrap();
}
async fn delegate(client: &RpcClient, task: &str, key: &str) -> Chat {
    client.call_as(methods::DELEGATE_TASK,json!({"parentChatId":"parent","task":task,"role":"review","isolation":"shared","target":{},"clientRequestId":key})).await.unwrap()
}
async fn wait_task(
    client: &RpcClient,
    id: &str,
    predicate: impl Fn(&zeron_proto::DelegatedTask) -> bool,
) -> Chat {
    wait_owned_task(client, "parent", id, predicate).await
}
async fn wait_owned_task(
    client: &RpcClient,
    parent: &str,
    id: &str,
    predicate: impl Fn(&zeron_proto::DelegatedTask) -> bool,
) -> Chat {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let chat: Chat = client
                .call_as(
                    methods::TASK_STATUS,
                    json!({"parentChatId":parent,"taskId":id}),
                )
                .await
                .unwrap();
            if predicate(chat.orchestration.as_ref().unwrap().task.as_ref().unwrap()) {
                return chat;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("task did not reach expected state")
}
#[tokio::test]
async fn delegation_inherits_and_retries_launch_once_then_delivers_result_once() {
    let dir = tempfile::tempdir().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let release = Arc::new(Semaphore::new(0));
    let core = assemble(dir.path(), requests.clone(), release.clone());
    parent(&core, "parent");
    parent(&core, "other");
    let client = zeron_rpc::memory_client(core.rpc_service());
    let (first, retry) = tokio::join!(
        delegate(&client, "hold task: review this change", "round-1"),
        delegate(&client, "hold task: review this change", "round-1")
    );
    assert_eq!(first.id, retry.id);
    assert_eq!(first.parent_chat_id.as_deref(), Some("parent"));
    assert_eq!(
        first.config.as_ref().unwrap().sandbox,
        SandboxLevel::WorkspaceWrite
    );
    wait_task(&client, &first.id, |t| t.status == TaskStatus::Running).await;
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert!(
        !requests.lock().unwrap()[0]
            .prompt
            .contains("prior visible conversation")
    );
    assert!(
        client
            .call(
                methods::TASK_STATUS,
                json!({"parentChatId":"other","taskId":first.id})
            )
            .await
            .is_err()
    );
    assert!(client.call(methods::DELEGATE_TASK,json!({"parentChatId":"parent","task":"a changed brief","target":{},"clientRequestId":"round-1"})).await.is_err());
    assert!(client.call(methods::DELEGATE_TASK,json!({"parentChatId":"parent","task":"escalation","target":{"sandbox":"danger-full-access"}})).await.is_err());
    release.add_permits(1);
    let finished = wait_task(&client, &first.id, |t| {
        t.status == TaskStatus::Completed && t.delivered
    })
    .await;
    assert!(
        finished
            .orchestration
            .as_ref()
            .unwrap()
            .task
            .as_ref()
            .unwrap()
            .summary
            .as_ref()
            .unwrap()
            .contains("checked")
    );
    let doc = core.doc_host.open("parent").unwrap();
    assert_eq!(
        doc.doc()
            .read_commands()
            .unwrap()
            .iter()
            .filter(|c| c.id == format!("task:{}:result", first.id))
            .count(),
        1
    );
    let retry = delegate(&client, "hold task: review this change", "round-1").await;
    assert_eq!(retry.id, first.id);
    // A later child turn cannot replace the published task result.
    core.workspace
        .rename_chat(&first.id, "Renamed review")
        .unwrap();
    let unchanged: Chat = client
        .call_as(
            methods::TASK_STATUS,
            json!({"parentChatId":"parent","taskId":first.id}),
        )
        .await
        .unwrap();
    assert_eq!(
        unchanged.orchestration.unwrap().task.unwrap().summary,
        finished.orchestration.unwrap().task.unwrap().summary
    );
    core.shutdown().await;
}
#[tokio::test]
async fn cancellation_stops_work_and_suppresses_parent_delivery() {
    let dir = tempfile::tempdir().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let release = Arc::new(Semaphore::new(0));
    let core = assemble(dir.path(), requests, release);
    parent(&core, "parent");
    let client = zeron_rpc::memory_client(core.rpc_service());
    let task = delegate(&client, "hold task until cancelled", "cancel-1").await;
    wait_task(&client, &task.id, |t| t.status == TaskStatus::Running).await;
    client
        .call(
            methods::TASK_CANCEL,
            json!({"parentChatId":"parent","taskId":task.id,"reason":"No longer needed"}),
        )
        .await
        .unwrap();
    let stopped = wait_task(&client, &task.id, |t| t.status == TaskStatus::Cancelled).await;
    let task = stopped.orchestration.unwrap().task.unwrap();
    assert!(task.cancel_requested);
    assert!(!task.delivered);
    assert_eq!(task.summary.as_deref(), Some("No longer needed"));
    assert!(
        client
            .call(
                methods::DELEGATE_TASK,
                json!({"parentChatId":task.id,"task":"late descendant","target":{}})
            )
            .await
            .is_err()
    );
    assert!(
        core.doc_host
            .open("parent")
            .unwrap()
            .doc()
            .read_commands()
            .unwrap()
            .is_empty()
    );
    core.shutdown().await;
}
#[tokio::test]
async fn fork_point_and_merge_have_stable_lineage_and_receipts() {
    use zeron_doc::{MessagePart, MessageRole, MessageStatus, SessionMessageEntry};
    let dir = tempfile::tempdir().unwrap();
    let core = assemble(
        dir.path(),
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Semaphore::new(0)),
    );
    parent(&core, "parent");
    let doc = core.doc_host.open("parent").unwrap();
    for (id, text) in [("a1", "First result"), ("a2", "Later result")] {
        doc.doc()
            .push_message(&SessionMessageEntry {
                id: id.into(),
                role: MessageRole::Assistant,
                parts: vec![MessagePart::Text {
                    id: id.into(),
                    text: text.into(),
                }],
                created_at: 1,
                device_id: core.device_id.clone(),
                status: Some(MessageStatus::Complete),
                continuation_of: None,
                duration_ms: None,
            })
            .unwrap();
    }
    let client = zeron_rpc::memory_client(core.rpc_service());
    let fork: Chat = client
        .call_as(
            methods::FORK_SIDE_CHAT,
            json!({"chatId":"fork","sourceChatId":"parent","sourceMessageId":"a1"}),
        )
        .await
        .unwrap();
    assert_eq!(
        fork.orchestration.unwrap().fork.unwrap().source_message_id,
        "a1"
    );
    let entries = core
        .doc_host
        .open("fork")
        .unwrap()
        .doc()
        .read_entries()
        .unwrap();
    assert_eq!(entries.len(), 2);
    let merge = json!({"sourceChatId":"fork","clientRequestId":"merge-1"});
    let one = client
        .call(methods::MERGE_CHAT, merge.clone())
        .await
        .unwrap();
    let two = client.call(methods::MERGE_CHAT, merge).await.unwrap();
    assert_eq!(one, two);
    assert_eq!(doc.doc().context_transfers().unwrap().len(), 1);
    assert_eq!(
        doc.doc()
            .read_commands()
            .unwrap()
            .iter()
            .filter(|c| c.id == one["transferId"].as_str().unwrap())
            .count(),
        1
    );
    core.shutdown().await;
}
#[tokio::test]
async fn switching_providers_transfers_visible_context_without_resuming_the_other_provider() {
    let dir = tempfile::tempdir().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let core = assemble(dir.path(), requests.clone(), Arc::new(Semaphore::new(0)));
    parent(&core, "parent");
    let request = |harness, prompt: &str| RunRequest {
        prompt: prompt.into(),
        harness: Some(harness),
        model: None,
        reasoning: None,
        model_options: Default::default(),
        cwd: "/tmp".into(),
        sandbox: SandboxLevel::WorkspaceWrite,
        auto_approve: false,
        resume: None,
        attachments: vec![],
        worktree: None,
        mcp: None,
    };
    core.sessions
        .dispatch(
            "parent",
            HarnessId::Mock,
            request(HarnessId::Mock, "Remember the orbit is elliptical"),
            Some("u1".into()),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if core
                .sessions
                .session_status("parent")
                .is_some_and(|s| s.status == zeron_proto::SessionStatus::Idle)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    core.sessions
        .dispatch(
            "parent",
            HarnessId::Codex,
            request(HarnessId::Codex, "Continue the analysis"),
            Some("u2".into()),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if requests.lock().unwrap().len() == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let second = requests.lock().unwrap()[1].clone();
    assert!(second.resume.is_none());
    assert!(second.prompt.contains("orbit is elliptical"));
    let transfers = core
        .doc_host
        .open("parent")
        .unwrap()
        .doc()
        .context_transfers()
        .unwrap();
    assert_eq!(transfers.len(), 1);
    assert_eq!(transfers[0].kind, "provider_handoff");
    assert!(transfers[0].content.contains("orbit"));
    core.shutdown().await;
}

#[tokio::test]
async fn parent_waits_for_nested_work_and_completion_followup() {
    let dir = tempfile::tempdir().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let release = Arc::new(Semaphore::new(0));
    let core = assemble(dir.path(), requests, release.clone());
    parent(&core, "parent");
    let client = zeron_rpc::memory_client(core.rpc_service());
    let outer = delegate(&client, "hold task outer", "outer").await;
    wait_task(&client, &outer.id, |t| t.status == TaskStatus::Running).await;
    let inner:Chat=client.call_as(methods::DELEGATE_TASK,json!({"parentChatId":outer.id,"task":"hold task inner","target":{},"clientRequestId":"inner"})).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let chat: Chat = client
                .call_as(
                    methods::TASK_STATUS,
                    json!({"parentChatId":outer.id,"taskId":inner.id}),
                )
                .await
                .unwrap();
            if chat.orchestration.unwrap().task.unwrap().status == TaskStatus::Running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    release.add_permits(1);
    wait_task(&client, &outer.id, |t| {
        t.status == TaskStatus::WaitingForChildren
    })
    .await;
    assert!(
        core.doc_host
            .open("parent")
            .unwrap()
            .doc()
            .read_commands()
            .unwrap()
            .is_empty()
    );
    release.add_permits(1);
    wait_task(&client, &outer.id, |t| {
        t.status == TaskStatus::Completed && t.delivered
    })
    .await;
    let requests = core
        .doc_host
        .open(&outer.id)
        .unwrap()
        .doc()
        .read_commands()
        .unwrap();
    assert!(
        requests
            .iter()
            .any(|c| c.id == format!("task:{}:result", inner.id))
    );
    core.shutdown().await;
}

#[tokio::test]
async fn durable_results_survive_restart_without_another_launch_or_delivery() {
    let dir = tempfile::tempdir().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let core = assemble(dir.path(), requests.clone(), Arc::new(Semaphore::new(0)));
    parent(&core, "parent");
    let client = zeron_rpc::memory_client(core.rpc_service());
    let task = delegate(&client, "Review the result", "restart-key").await;
    let finished = wait_task(&client, &task.id, |t| {
        t.status == TaskStatus::Completed && t.delivered
    })
    .await;
    core.shutdown().await;
    drop(client);
    drop(core);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let core = assemble(dir.path(), requests.clone(), Arc::new(Semaphore::new(0)));
    let client = zeron_rpc::memory_client(core.rpc_service());
    let retry = delegate(&client, "Review the result", "restart-key").await;
    assert_eq!(retry.id, task.id);
    assert_eq!(
        retry.orchestration.unwrap().task.unwrap().summary,
        finished.orchestration.unwrap().task.unwrap().summary
    );
    assert!(requests.lock().unwrap().is_empty());
    assert_eq!(
        core.doc_host
            .open("parent")
            .unwrap()
            .doc()
            .read_commands()
            .unwrap()
            .iter()
            .filter(|c| c.id == format!("task:{}:result", task.id))
            .count(),
        1
    );
    core.shutdown().await;
}

#[tokio::test]
async fn isolated_task_uses_a_fresh_worktree_from_parent_commit_and_excludes_dirty_files() {
    fn git(path: &std::path::Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().into()
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    std::fs::write(repo.join("tracked"), "committed\n").unwrap();
    git(&repo, &["add", "tracked"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
            "commit",
            "-m",
            "test: seed",
        ],
    );
    std::fs::write(repo.join("tracked"), "dirty parent\n").unwrap();
    std::fs::write(repo.join("untracked"), "local only").unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let core = assemble(
        &dir.path().join("engine"),
        requests.clone(),
        Arc::new(Semaphore::new(0)),
    );
    let trees = dir.path().join("trees");
    std::fs::create_dir(&trees).unwrap();
    core.repos
        .set_worktree_settings(zeron_proto::WorktreeSettings {
            use_custom_directory: true,
            custom_directory: Some(trees.to_string_lossy().into()),
        })
        .await
        .unwrap();
    core.workspace
        .create_space(
            "project",
            &core.device_id,
            repo.to_str().unwrap(),
            None,
            true,
        )
        .unwrap();
    parent(&core, "parent");
    let mut chat = core.workspace.chat("parent").unwrap().unwrap();
    chat.space_id = Some("project".into());
    chat.cwd = Some(repo.to_string_lossy().into());
    core.workspace.import_chat_row(&chat).unwrap();
    let client = zeron_rpc::memory_client(core.rpc_service());
    let task:Chat=client.call_as(methods::DELEGATE_TASK,json!({"parentChatId":"parent","task":"Inspect committed checkout","isolation":"worktree","target":{},"clientRequestId":"tree-1"})).await.unwrap();
    let task = wait_task(&client, &task.id, |t| t.status.is_terminal()).await;
    assert_eq!(
        task.orchestration
            .as_ref()
            .unwrap()
            .task
            .as_ref()
            .unwrap()
            .status,
        TaskStatus::Completed
    );
    let child = std::path::Path::new(task.cwd.as_ref().unwrap());
    assert_ne!(child, repo);
    assert!(
        std::fs::canonicalize(child)
            .unwrap()
            .starts_with(std::fs::canonicalize(&trees).unwrap()),
        "child: {child:?}, root: {trees:?}"
    );
    assert_eq!(
        std::fs::read_to_string(child.join("tracked")).unwrap(),
        "committed\n"
    );
    assert!(!child.join("untracked").exists());
    assert_eq!(
        std::fs::read_to_string(repo.join("tracked")).unwrap(),
        "dirty parent\n"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn empty_root_turns_complete_or_interrupt_without_sticking_the_task() {
    let dir = tempfile::tempdir().unwrap();
    let core = assemble(
        dir.path(),
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Semaphore::new(0)),
    );
    parent(&core, "parent");
    let client = zeron_rpc::memory_client(core.rpc_service());
    let completed = delegate(&client, "empty task", "empty-complete").await;
    wait_task(&client, &completed.id, |t| {
        t.status == TaskStatus::Completed
    })
    .await;
    let interrupted = delegate(&client, "hold task empty task", "empty-interrupted").await;
    wait_task(&client, &interrupted.id, |t| {
        t.status == TaskStatus::Running
    })
    .await;
    core.sessions.interrupt(&interrupted.id).await.unwrap();
    wait_task(&client, &interrupted.id, |t| {
        t.status == TaskStatus::Interrupted
    })
    .await;
    core.shutdown().await;
}

#[tokio::test]
async fn task_question_is_inspectable_and_normal_input_response_resumes_work() {
    let dir = tempfile::tempdir().unwrap();
    let core = assemble(
        dir.path(),
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Semaphore::new(0)),
    );
    parent(&core, "parent");
    let client = zeron_rpc::memory_client(core.rpc_service());
    let task = delegate(&client, "ask task", "question").await;
    let waiting = wait_task(&client, &task.id, |t| t.status == TaskStatus::Waiting).await;
    assert!(!waiting.orchestration.unwrap().task.unwrap().delivered);
    let doc = core.doc_host.open(&task.id).unwrap();
    let request_id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(id) = doc
                .doc()
                .read_entries()
                .unwrap()
                .iter()
                .flat_map(|e| &e.parts)
                .find_map(|p| match p {
                    zeron_doc::MessagePart::Input {
                        request_id,
                        resolved: false,
                        ..
                    } => Some(request_id.clone()),
                    _ => None,
                })
            {
                break id;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    client.call(methods::QUEUE_COMMAND,json!({"chatId":task.id,"command":{"kind":"respondInput","requestId":request_id,"answers":[{"questionId":"q","labels":["Continue"]}]}})).await.unwrap();
    wait_task(&client, &task.id, |t| {
        t.status == TaskStatus::Completed && t.delivered
    })
    .await;
    core.shutdown().await;
}

#[tokio::test]
async fn cancellation_stops_managed_descendants_and_preserves_independent_side_chats() {
    let dir = tempfile::tempdir().unwrap();
    let release = Arc::new(Semaphore::new(0));
    let core = assemble(
        dir.path(),
        Arc::new(Mutex::new(Vec::new())),
        release.clone(),
    );
    parent(&core, "parent");
    let client = zeron_rpc::memory_client(core.rpc_service());
    let outer = delegate(&client, "hold task outer", "cancel-tree").await;
    wait_task(&client, &outer.id, |t| t.status == TaskStatus::Running).await;
    let nested: Chat = client
        .call_as(
            methods::DELEGATE_TASK,
            json!({"parentChatId":outer.id,"task":"hold task nested","target":{}}),
        )
        .await
        .unwrap();
    wait_owned_task(&client, &outer.id, &nested.id, |t| {
        t.status == TaskStatus::Running
    })
    .await;
    core.workspace
        .create_chat_with_parent(
            "independent",
            None,
            Some(&core.device_id),
            outer.config.clone(),
            Some("/tmp".into()),
            Some(outer.id.clone()),
        )
        .unwrap();
    let separate: Chat = client
        .call_as(
            methods::DELEGATE_TASK,
            json!({"parentChatId":"independent","task":"hold task independent","target":{}}),
        )
        .await
        .unwrap();
    wait_owned_task(&client, "independent", &separate.id, |t| {
        t.status == TaskStatus::Running
    })
    .await;
    client
        .call(
            methods::TASK_CANCEL,
            json!({"parentChatId":"parent","taskId":outer.id}),
        )
        .await
        .unwrap();
    wait_task(&client, &outer.id, |t| t.status == TaskStatus::Cancelled).await;
    wait_owned_task(&client, &outer.id, &nested.id, |t| {
        t.status == TaskStatus::Cancelled
    })
    .await;
    let unchanged: Chat = client
        .call_as(
            methods::TASK_STATUS,
            json!({"parentChatId":"independent","taskId":separate.id}),
        )
        .await
        .unwrap();
    assert_eq!(
        unchanged.orchestration.unwrap().task.unwrap().status,
        TaskStatus::Running
    );
    release.add_permits(1);
    wait_owned_task(&client, "independent", &separate.id, |t| {
        t.status == TaskStatus::Completed
    })
    .await;
    core.shutdown().await;
}

#[tokio::test]
async fn large_briefs_and_results_keep_registry_operations_within_the_wire_budget() {
    let dir = tempfile::tempdir().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let core = assemble(dir.path(), requests.clone(), Arc::new(Semaphore::new(0)));
    parent(&core, "parent");
    let client = zeron_rpc::memory_client(core.rpc_service());
    let brief = format!("large task output\n{}", "x".repeat(100_000));
    let task = delegate(&client, &brief, "large-payload").await;
    let finished = wait_task(&client, &task.id, |t| {
        t.status == TaskStatus::Completed && t.delivered
    })
    .await;
    let task = finished
        .orchestration
        .as_ref()
        .unwrap()
        .task
        .as_ref()
        .unwrap();
    assert!(task.prompt.is_empty());
    assert!(task.summary_truncated);
    assert!(serde_json::to_vec(&finished).unwrap().len() < zeron_doc::registry::MAX_OP_BYTES);
    assert!(
        serde_json::to_vec(task.summary.as_ref().unwrap())
            .unwrap()
            .len()
            <= 4096
    );
    let captured = requests.lock().unwrap().clone();
    assert!(captured[0].prompt.contains(&brief));
    let commands = core
        .doc_host
        .open("parent")
        .unwrap()
        .doc()
        .read_commands()
        .unwrap();
    let result = commands
        .iter()
        .find(|c| c.id == format!("task:{}:result", task.id))
        .unwrap();
    let zeron_doc::SessionCommandPayload::Steer { prompt, .. } = &result.payload else {
        panic!("expected attributed result")
    };
    assert!(
        prompt.len() > 20_000,
        "parent receives more than the compact preview"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn cancellation_reasons_are_bounded_in_the_registry_with_the_full_result_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let core = assemble(
        dir.path(),
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Semaphore::new(0)),
    );
    parent(&core, "parent");
    let client = zeron_rpc::memory_client(core.rpc_service());
    let task = delegate(&client, "hold task", "cancel-escaped").await;
    wait_task(&client, &task.id, |task| task.status == TaskStatus::Running).await;
    let reason = "\u{0001}".repeat(2000);
    client
        .call(
            methods::TASK_CANCEL,
            json!({"parentChatId":"parent","taskId":task.id,"reason":reason}),
        )
        .await
        .unwrap();
    let cancelled = wait_task(&client, &task.id, |task| {
        task.status == TaskStatus::Cancelled
    })
    .await;
    let state = cancelled.orchestration.unwrap().task.unwrap();
    assert!(state.summary_truncated);
    assert!(serde_json::to_vec(&state.summary.unwrap()).unwrap().len() <= 4096);
    let full: zeron_proto::TaskResult = client
        .call_as(
            methods::TASK_RESULT,
            json!({"parentChatId":"parent","taskId":task.id}),
        )
        .await
        .unwrap();
    assert_eq!(full.summary, reason);
    assert!(!full.truncated);
    core.shutdown().await;
}
