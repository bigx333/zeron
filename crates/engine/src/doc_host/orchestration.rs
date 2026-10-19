//! Managed delegation uses the existing registry and durable command ledger.
//! Only the child's host publishes its result; deterministic command ids make
//! launch and parent delivery safe to retry after a lost response or restart.
use super::*;
use std::time::Duration;
use zeron_doc::{MessageRole, MessageStatus};
use zeron_proto::{
    Chat, ChatOrchestration, DelegateTaskRequest, DelegatedTask, TaskIsolation, TaskStatus,
};

pub(crate) fn stable_id(scope: &str, key: &str) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("zeron:orchestration:{scope}:{key}").as_bytes(),
    )
    .to_string()
}

pub(crate) fn run_request(
    chat: &Chat,
    prompt: String,
) -> Result<zeron_proto::RunRequest, EngineError> {
    let config = chat
        .config
        .as_ref()
        .ok_or_else(|| EngineError::Other("Chat has no agent configuration".into()))?;
    Ok(zeron_proto::RunRequest {
        prompt,
        harness: Some(config.harness),
        model: config.model.clone(),
        reasoning: config.reasoning,
        model_options: config.model_options.clone(),
        cwd: chat.cwd.clone().unwrap_or_else(|| "~".into()),
        sandbox: config.sandbox,
        auto_approve: config.sandbox == zeron_proto::SandboxLevel::DangerFullAccess,
        resume: None,
        attachments: Vec::new(),
        worktree: None,
        mcp: None,
    })
}

fn bounded(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn registry_preview(text: &str) -> (String, bool) {
    let mut preview = String::new();
    let mut encoded = 2usize;
    for ch in text.chars() {
        let cost = match ch {
            '\"' | '\\' | '\n' | '\r' | '\t' => 2,
            ch if ch <= '\u{1f}' => 6,
            ch => ch.len_utf8(),
        };
        if encoded + cost > 4096 {
            break;
        }
        encoded += cost;
        preview.push(ch);
    }
    let truncated = preview.len() < text.len();
    (preview, truncated)
}

impl DocHost {
    pub(crate) fn task_result(
        &self,
        parent: &str,
        id: &str,
    ) -> Result<Option<zeron_proto::TaskResult>, EngineError> {
        let chat = self.task_chat(parent, id)?;
        if chat.device_id != self.device_id() {
            return Err(EngineError::Other(
                "Read the result on the task's host device".into(),
            ));
        }
        let task = chat
            .orchestration
            .as_ref()
            .and_then(|o| o.task.as_ref())
            .unwrap();
        if !task.status.is_terminal() {
            return Ok(None);
        }
        let handle = self.open(id)?;
        if let Some(result) = handle.doc.frozen_task_result() {
            return Ok(Some(result));
        }
        let cancellation = task
            .cancel_requested
            .then(|| handle.doc.task_cancellation_reason())
            .flatten();
        let text = cancellation
            .or_else(|| {
                task.result_message_id
                    .as_ref()
                    .and_then(|id| {
                        zeron_doc::join_continuation_entries(handle.doc.read_entries().ok()?)
                            .into_iter()
                            .find(|e| &e.id == id)
                    })
                    .map(|entry| {
                        entry
                            .parts
                            .iter()
                            .filter_map(|p| match p {
                                MessagePart::Text { text, .. } => Some(text.as_str()),
                                MessagePart::Error { message, .. } => Some(message.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
            })
            .unwrap_or_else(|| task.summary.clone().unwrap_or_default());
        let result = zeron_proto::TaskResult {
            message_id: task.result_message_id.clone(),
            summary: text.chars().take(24000).collect(),
            truncated: text.chars().count() > 24000,
        };
        handle.doc.freeze_task_result(&result)?;
        self.persist_orchestration(&handle)?;
        Ok(Some(handle.doc.frozen_task_result().unwrap_or(result)))
    }

    pub(super) fn persist_orchestration(&self, handle: &ChatDocHandle) -> Result<(), EngineError> {
        if let Some(persistence) = &handle.persistence {
            if persistence.flush_required() {
                Ok(())
            } else {
                Err(EngineError::Other(
                    "Could not persist orchestration state".into(),
                ))
            }
        } else {
            self.persist_fork(handle)
        }
    }

    /// Queue an app-owned command once. The same id with different contents is
    /// rejected. Caller holds the orchestration gate across the read and write.
    pub(crate) fn queue_orchestration_command(
        &self,
        chat_id: &str,
        id: &str,
        payload: SessionCommandPayload,
    ) -> Result<(), EngineError> {
        let handle = self.open(chat_id)?;
        if let Some(existing) = handle
            .doc
            .read_commands()?
            .into_iter()
            .find(|entry| entry.id == id)
        {
            if existing.payload != payload {
                return Err(EngineError::Other(
                    "Request id was already used with different content".into(),
                ));
            }
            self.persist_orchestration(&handle)?;
            // Recovery also reestablishes peer delivery for a pending command.
            if existing.status == SessionCommandStatus::Pending {
                self.nudge_remote_host(chat_id);
                self.spawn_command_delivery(chat_id, existing, Vec::new());
            }
            return Ok(());
        }
        let entry = SessionCommandEntry {
            id: id.to_owned(),
            payload,
            issued_by: self.device_id().to_owned(),
            issued_at: now_ms(),
            based_on: None,
            expires_at: Some(i64::MAX),
            status: SessionCommandStatus::Pending,
            resolution: None,
        };
        handle.doc.queue_command(&entry)?;
        self.persist_orchestration(&handle)?;
        self.unarchive_on_send(chat_id);
        self.nudge_remote_host(chat_id);
        self.spawn_command_delivery(chat_id, entry, Vec::new());
        Ok(())
    }

    pub(crate) async fn delegate_task(
        &self,
        request: DelegateTaskRequest,
        registry: &crate::HarnessRegistry,
    ) -> Result<Chat, EngineError> {
        if request.task.trim().is_empty() || request.task.len() > 120_000 {
            return Err(EngineError::Other(
                "Task must contain 1–120000 bytes".into(),
            ));
        }
        if request
            .title
            .as_ref()
            .is_some_and(|s| s.trim().is_empty() || s.len() > 512)
        {
            return Err(EngineError::Other("Title must contain 1–512 bytes".into()));
        }
        if request
            .client_request_id
            .as_ref()
            .is_some_and(|s| s.trim().is_empty() || s.len() > 256)
        {
            return Err(EngineError::Other(
                "clientRequestId must contain 1–256 bytes".into(),
            ));
        }
        let ws = self
            .workspace()
            .ok_or_else(|| EngineError::Other("Workspace unavailable".into()))?;
        let parent = ws
            .chat(&request.parent_chat_id)?
            .ok_or_else(|| EngineError::Other("Parent chat no longer exists".into()))?;
        if parent.device_id != self.device_id() {
            return Err(EngineError::Other(
                "Delegate on the parent chat's device".into(),
            ));
        }
        // Bound recursive agent trees, while allowing tasks to delegate.
        let mut ancestor = parent.parent_chat_id.clone();
        let mut depth = 1;
        while let Some(id) = ancestor {
            depth += 1;
            if depth > 4 {
                return Err(EngineError::Other(
                    "Delegation is limited to four levels".into(),
                ));
            }
            ancestor = ws.chat(&id)?.and_then(|chat| chat.parent_chat_id);
        }
        let id = request
            .client_request_id
            .as_ref()
            .map(|key| stable_id(&parent.id, key))
            .unwrap_or_else(new_id);
        let mut fingerprint_request = request.clone();
        fingerprint_request.client_request_id = None;
        use sha2::Digest;
        let fingerprint = format!(
            "{:x}",
            sha2::Sha256::digest(
                serde_json::to_vec(&fingerprint_request)
                    .map_err(|e| EngineError::Other(e.to_string()))?
            )
        );
        // Check receipts before consulting a catalog that may have changed since launch.
        if let Some(existing) = ws.chat(&id)? {
            let task = existing
                .orchestration
                .as_ref()
                .and_then(|o| o.task.as_ref());
            if !task.is_some_and(|task| {
                task.parent_chat_id == parent.id && task.request_fingerprint == fingerprint
            }) {
                return Err(EngineError::Other(
                    "clientRequestId was already used with different task content".into(),
                ));
            }
            return Ok(existing);
        }
        let inherited = parent
            .config
            .as_ref()
            .ok_or_else(|| EngineError::Other("Parent has no agent configuration".into()))?;
        let config = request
            .target
            .resolve(inherited)
            .map_err(|e| EngineError::Other(e.into()))?;
        let descriptor = registry
            .descriptors()
            .into_iter()
            .find(|h| h.id == config.harness)
            .ok_or_else(|| EngineError::Other("Unknown harness".into()))?;
        if !descriptor.installed
            || (!crate::registry::descriptor_enabled(&descriptor)
                // The explicitly selected developer rig may delegate to itself;
                // production providers must never silently switch to a mock.
                && !(config.harness == zeron_proto::HarnessId::Mock && inherited.harness == zeron_proto::HarnessId::Mock))
        {
            return Err(EngineError::Other(
                "Target harness is unavailable on this device".into(),
            ));
        }
        if let Some(model) = config.model.as_deref() {
            let models = registry.discover_models(config.harness).await?;
            if !models.iter().any(|m| m.id == model) {
                return Err(EngineError::Other(format!(
                    "Model {model:?} is not offered by the target harness"
                )));
            }
        }
        if request.isolation == TaskIsolation::Worktree && parent.space_id.is_none() {
            return Err(EngineError::Other(
                "Worktree isolation requires a project".into(),
            ));
        }
        let base_ref = if request.isolation == TaskIsolation::Worktree {
            let repos = self
                .inner
                .repos
                .get()
                .ok_or_else(|| EngineError::Other("Repositories unavailable".into()))?;
            Some(
                repos
                    .head_sha(std::path::Path::new(parent.cwd.as_deref().unwrap_or("")))
                    .await?
                    .ok_or_else(|| {
                        EngineError::Other(
                            "Worktree isolation requires a committed Git checkout".into(),
                        )
                    })?,
            )
        } else {
            None
        };
        let _gate = self.inner.orchestration.lock().await;
        // A concurrent retry may have won while model discovery was in flight.
        if let Some(existing) = ws.chat(&id)? {
            if existing
                .orchestration
                .as_ref()
                .and_then(|o| o.task.as_ref())
                .is_some_and(|t| t.request_fingerprint == fingerprint)
            {
                return Ok(existing);
            }
            return Err(EngineError::Other("Request id collision".into()));
        }
        if ws
            .chat(&parent.id)?
            .and_then(|chat| chat.orchestration)
            .and_then(|state| state.task)
            .is_some_and(|task| task.cancel_requested)
        {
            return Err(EngineError::Other(
                "A stopping or cancelled task cannot delegate new work".into(),
            ));
        }
        let now = now_ms();
        let mut chat = parent.clone();
        if request.isolation == TaskIsolation::Worktree {
            let space = parent
                .space_id
                .as_ref()
                .and_then(|id| ws.space(id).ok().flatten())
                .ok_or_else(|| EngineError::Other("Task project not found".into()))?;
            // The child is not already attached to the parent's worktree: the
            // command executor must materialize a fresh isolated checkout.
            chat.cwd = Some(space.path);
            chat.checkout_id = None;
        }
        chat.id = id.clone();
        chat.parent_chat_id = Some(parent.id.clone());
        chat.title = request.title.or_else(|| {
            let first_line = request
                .task
                .lines()
                .find(|line| !line.trim().is_empty())?
                .trim();
            Some(first_line.chars().take(80).collect())
        });
        chat.config = Some(config);
        chat.archived = false;
        chat.snoozed_until = None;
        chat.created_at = chrono::Utc::now();
        chat.last_message_at = None;
        chat.last_message_preview = None;
        chat.last_seen_at = None;
        chat.harness_session_id = None;
        chat.harness_session_cwd = None;
        chat.room_gen = Some(2);
        chat.orchestration = Some(ChatOrchestration {
            fork: None,
            task: Some(DelegatedTask {
                id: id.clone(),
                parent_chat_id: parent.id,
                role: request.role,
                isolation: request.isolation,
                base_ref,
                status: TaskStatus::Queued,
                prompt: String::new(),
                request_fingerprint: fingerprint,
                created_at: now,
                updated_at: now,
                summary: None,
                summary_truncated: false,
                result_message_id: None,
                delivered: false,
                cancel_requested: false,
            }),
        });
        if serde_json::to_vec(&chat)
            .map_err(|e| EngineError::Other(e.to_string()))?
            .len()
            > 8 * 1024
        {
            return Err(EngineError::Other(
                "Task configuration is too large to sync".into(),
            ));
        }
        let handle = self.open(&chat.id)?;
        handle.doc.stage_task_brief(
            &chat
                .orchestration
                .as_ref()
                .unwrap()
                .task
                .as_ref()
                .unwrap()
                .request_fingerprint,
            &request.task,
        )?;
        self.persist_orchestration(&handle)?;
        ws.import_chat_row(&chat)?;
        ws.persist_now()?;
        self.launch_task(&chat)?;
        Ok(chat)
    }

    fn launch_task(&self, chat: &Chat) -> Result<(), EngineError> {
        let Some(task) = chat.orchestration.as_ref().and_then(|o| o.task.as_ref()) else {
            return Ok(());
        };
        if task.cancel_requested || task.status.is_terminal() {
            return Ok(());
        }
        let handle = self.open(&chat.id)?;
        let brief = handle
            .doc
            .task_brief()
            .or_else(|| (!task.prompt.is_empty()).then(|| task.prompt.clone()))
            .ok_or_else(|| EngineError::Other("Task brief is not available yet".into()))?;
        let mut request = run_request(
            chat,
            format!(
                "You are working on a delegated {:?} task. Complete the brief and return a clear result to your parent.\n\n{}",
                task.role, brief
            ),
        )?;
        if task.isolation == TaskIsolation::Worktree {
            let ws = self
                .workspace()
                .ok_or_else(|| EngineError::Other("Workspace unavailable".into()))?;
            let space = chat
                .space_id
                .as_ref()
                .and_then(|id| ws.space(id).ok().flatten())
                .ok_or_else(|| EngineError::Other("Task project no longer exists".into()))?;
            request.worktree = Some(zeron_proto::WorktreeSpec {
                // Base HEAD of the actual parent checkout, including stacked branches.
                repo_path: space.path,
                base: task.base_ref.clone().unwrap_or_else(|| "HEAD".into()),
                space_id: Some(space.id),
            });
        }
        self.queue_orchestration_command(
            &chat.id,
            &format!("task:{}:launch", task.id),
            SessionCommandPayload::Run {
                request,
                message_id: format!("task:{}:brief", task.id),
            },
        )?;
        handle.doc.clear_task_brief()?;
        Ok(())
    }

    pub(crate) fn task_chat(&self, parent: &str, task: &str) -> Result<Chat, EngineError> {
        let chat = self
            .workspace()
            .ok_or_else(|| EngineError::Other("Workspace unavailable".into()))?
            .chat(task)?
            .ok_or_else(|| EngineError::Other("Task not found".into()))?;
        if !chat
            .orchestration
            .as_ref()
            .and_then(|o| o.task.as_ref())
            .is_some_and(|t| t.parent_chat_id == parent)
        {
            return Err(EngineError::Other(
                "Task belongs to a different parent chat".into(),
            ));
        }
        Ok(chat)
    }

    pub(crate) async fn cancel_task(
        &self,
        parent: &str,
        task_id: &str,
        reason: Option<&str>,
    ) -> Result<Chat, EngineError> {
        if reason.is_some_and(|s| s.len() > 2000) {
            return Err(EngineError::Other(
                "Cancellation reason exceeds 2000 bytes".into(),
            ));
        }
        let _gate = self.inner.orchestration.lock().await;
        let root = self.task_chat(parent, task_id)?;
        if root.device_id != self.device_id() {
            return Err(EngineError::Other("Cancel on the task's device".into()));
        }
        let ws = self.workspace().unwrap();
        let chats = ws.read_chats()?;
        let mut pending = vec![root.id.clone()];
        let mut visited = HashSet::new();
        let mut stops = Vec::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id.clone()) {
                continue;
            }
            for child in chats.iter().filter(|chat| {
                chat.orchestration
                    .as_ref()
                    .and_then(|o| o.task.as_ref())
                    .is_some_and(|task| task.parent_chat_id == id)
            }) {
                pending.push(child.id.clone());
            }
            let Some(chat) = ws.chat(&id)? else {
                continue;
            };
            let Some(mut state) = chat.orchestration.clone() else {
                continue;
            };
            let Some(task) = state.task.as_mut() else {
                continue;
            };
            task.cancel_requested = true;
            if !task.status.is_terminal() {
                let full = reason.unwrap_or("Cancelled by parent");
                let (summary, truncated) = registry_preview(full);
                task.summary = Some(summary);
                task.summary_truncated = truncated;
                task.updated_at = now_ms();
                let handle = self.open(&id)?;
                handle.doc.set_task_cancellation_reason(full)?;
                self.persist_orchestration(&handle)?;
            }
            // Persist suppression of completion delivery before stopping the runtime.
            ws.set_chat_orchestration(&id, &state)?;
            stops.push(id);
        }
        ws.persist_now()?;
        for id in stops {
            self.queue_orchestration_command(
                &id,
                &format!("task:{id}:cancel"),
                SessionCommandPayload::Interrupt {},
            )?;
        }
        self.task_chat(parent, task_id)
    }

    pub(crate) async fn merge_context(
        &self,
        target: &str,
        id: &str,
        text: String,
        transfer: zeron_proto::ContextTransfer,
    ) -> Result<(), EngineError> {
        let _gate = self.inner.orchestration.lock().await;
        let handle = self.open(target)?;
        if let Some(existing) = handle.doc.context_transfers()?.iter().find(|t| t.id == id) {
            if existing.source_chat_id != transfer.source_chat_id
                || existing.content != transfer.content
            {
                return Err(EngineError::Other(
                    "Request id was already used with a different merge".into(),
                ));
            }
            return Ok(());
        }
        self.queue_orchestration_command(
            target,
            id,
            SessionCommandPayload::Steer {
                prompt: text,
                message_id: Some(id.to_owned()),
            },
        )?;
        handle.doc.record_context_transfer(transfer)?;
        self.persist_orchestration(&handle)?;
        Ok(())
    }

    pub(crate) fn spawn_task_reconciler(&self) {
        let Some(ws) = self.workspace() else {
            return;
        };
        let Some(sessions) = self.sessions() else {
            return;
        };
        if self
            .inner
            .orchestration_started
            .swap(true, Ordering::AcqRel)
        {
            return;
        }
        let mut chats = ws.watch_chats();
        let mut statuses = ws.merged_sessions_watch(sessions.watch_sessions());
        let host = self.clone();
        self.spawn_worker(async move {
            loop {
                if let Err(error) = host.reconcile_tasks().await {
                    tracing::warn!(%error, "delegated task reconciliation failed");
                }
                let retry_needed = chats.borrow().iter().any(|chat| {
                    chat.device_id == host.device_id()
                        && chat
                            .orchestration
                            .as_ref()
                            .and_then(|o| o.task.as_ref())
                            .is_some_and(|task| {
                                !task.status.is_terminal()
                                    || (!task.delivered && !task.cancel_requested)
                            })
                });
                tokio::select! {
                    result = chats.changed() => if result.is_err() { break; },
                    result = statuses.changed() => if result.is_err() { break; },
                    _ = tokio::time::sleep(Duration::from_secs(5)), if retry_needed => {},
                }
            }
        });
    }

    async fn reconcile_tasks(&self) -> Result<(), EngineError> {
        let _gate = self.inner.orchestration.lock().await;
        let Some(ws) = self.workspace() else {
            return Ok(());
        };
        let chats = ws.read_chats()?;
        let mut sessions = ws.read_sessions()?;
        if let Some(engine) = self.sessions() {
            for live in engine.watch_sessions().borrow().iter() {
                sessions.retain(|s| s.chat_id != live.chat_id);
                sessions.push(live.clone());
            }
        }
        for chat in chats
            .iter()
            .filter(|chat| chat.device_id == self.device_id())
        {
            if let Err(error) = self.reconcile_task(chat, &chats, &sessions) {
                tracing::warn!(chat = %chat.id, %error, "delegated task could not be reconciled; retrying");
            }
        }
        Ok(())
    }

    fn reconcile_task(
        &self,
        chat: &Chat,
        chats: &[Chat],
        sessions: &[zeron_proto::Session],
    ) -> Result<(), EngineError> {
        let Some(ws) = self.workspace() else {
            return Ok(());
        };
        let Some(mut state) = chat.orchestration.clone() else {
            return Ok(());
        };
        let Some(task) = state.task.as_mut() else {
            return Ok(());
        };
        if task.cancel_requested {
            let handle = self.open(&chat.id)?;
            // Recover a crash between persisting cancellation intent and
            // appending its stop command.
            self.queue_orchestration_command(
                &chat.id,
                &format!("task:{}:cancel", task.id),
                SessionCommandPayload::Interrupt {},
            )?;
            if !task.status.is_terminal() {
                let stopped = handle.doc.read_commands()?.iter().any(|c| {
                    c.id == format!("task:{}:cancel", task.id)
                        && c.status == SessionCommandStatus::Applied
                });
                let active = sessions.iter().any(|s| {
                    s.chat_id == chat.id
                        && matches!(
                            s.status,
                            zeron_proto::SessionStatus::Working
                                | zeron_proto::SessionStatus::AwaitingInput
                        )
                });
                if stopped && !active {
                    task.status = TaskStatus::Cancelled;
                    task.updated_at = now_ms();
                    ws.set_chat_orchestration(&chat.id, &state)?;
                }
            }
            return Ok(());
        }
        if !task.status.is_terminal() {
            let handle = self.open(&chat.id)?;
            let commands = handle.doc.read_commands()?;
            let launch_id = format!("task:{}:launch", task.id);
            let launch = commands.iter().find(|c| c.id == launch_id);
            if launch.is_none() {
                self.launch_task(chat)?;
                return Ok(());
            }
            let session = sessions.iter().find(|s| s.chat_id == chat.id);
            let status = session.map(|s| s.status);
            let blocked = status == Some(zeron_proto::SessionStatus::AwaitingInput);
            let active = status == Some(zeron_proto::SessionStatus::Working);
            let has_children = chats.iter().any(|c| {
                c.parent_chat_id.as_deref() == Some(&chat.id)
                    && c.orchestration
                        .as_ref()
                        .and_then(|o| o.task.as_ref())
                        .is_some_and(|t| {
                            !t.status.is_terminal() || (!t.cancel_requested && !t.delivered)
                        })
            });
            let pending = commands
                .iter()
                .any(|c| c.status == SessionCommandStatus::Pending);
            let entries = zeron_doc::join_continuation_entries(handle.doc.read_entries()?);
            let terminal = entries.iter().rev().find(|e| {
                e.role == MessageRole::Assistant
                    && matches!(
                        e.status,
                        Some(MessageStatus::Complete | MessageStatus::Aborted)
                    )
            });
            let root_outcome = handle.doc.root_run_outcome();
            let rejected = launch.filter(|c| {
                matches!(
                    c.status,
                    SessionCommandStatus::Rejected
                        | SessionCommandStatus::Expired
                        | SessionCommandStatus::Cancelled
                )
            });
            let next = if blocked {
                TaskStatus::Waiting
            } else if active {
                TaskStatus::Running
            } else if rejected.is_some() {
                TaskStatus::Failed
            } else if has_children {
                TaskStatus::WaitingForChildren
            } else if pending {
                TaskStatus::Queued
            } else if status == Some(zeron_proto::SessionStatus::Errored) {
                TaskStatus::Failed
            }
            // Only a host-published root completion can complete a task.
            else if root_outcome
                .as_ref()
                .is_some_and(|outcome| outcome.status == zeron_proto::DoneStatus::Interrupted)
                || terminal.is_some_and(|e| e.status == Some(MessageStatus::Aborted))
            {
                TaskStatus::Interrupted
            } else if root_outcome
                .as_ref()
                .is_some_and(|outcome| outcome.status == zeron_proto::DoneStatus::Errored)
            {
                TaskStatus::Failed
            } else if root_outcome
                .as_ref()
                .is_some_and(|outcome| outcome.status == zeron_proto::DoneStatus::Completed)
                || session.is_some_and(|s| s.last_completed_turn.is_some())
            {
                TaskStatus::Completed
            } else if launch.is_some_and(|c| c.status == SessionCommandStatus::Applied)
                && session.is_none()
            {
                TaskStatus::Failed
            } else {
                task.status
            };
            if next != task.status {
                task.status = next;
                task.updated_at = now_ms();
                if next.is_terminal() {
                    task.result_message_id = terminal.map(|e| e.id.clone());
                    let text = terminal.map(|e| {
                        e.parts
                            .iter()
                            .filter_map(|p| match p {
                                MessagePart::Text { text, .. } => Some(text.as_str()),
                                MessagePart::Error { message, .. } => Some(message.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    });
                    let full = text.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| {
                        rejected
                            .and_then(|c| c.resolution.clone())
                            .unwrap_or_else(|| next.label().to_owned())
                    });
                    let (preview, truncated) = registry_preview(&full);
                    task.summary = Some(preview);
                    task.summary_truncated = truncated;
                    self.persist_orchestration(&handle)?;
                }
                ws.set_chat_orchestration(&chat.id, &state)?;
                if state.task.as_ref().is_some_and(|t| t.status.is_terminal()) {
                    ws.persist_now()?;
                }
            }
        }
        let task = state.task.as_mut().unwrap();
        if task.status.is_terminal() && !task.delivered && !task.cancel_requested {
            let Some(parent) = ws.chat(&task.parent_chat_id)? else {
                return Ok(());
            };
            if sessions.iter().any(|s| {
                s.chat_id == parent.id && s.status == zeron_proto::SessionStatus::AwaitingInput
            }) {
                return Ok(());
            }
            let result_text = if let Some(result_id) = &task.result_message_id {
                let entries =
                    zeron_doc::join_continuation_entries(self.open(&chat.id)?.doc.read_entries()?);
                entries
                    .iter()
                    .find(|entry| &entry.id == result_id)
                    .map(|entry| {
                        entry
                            .parts
                            .iter()
                            .filter_map(|part| match part {
                                MessagePart::Text { text, .. } => Some(text.as_str()),
                                MessagePart::Error { message, .. } => Some(message.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or_else(|| {
                        task.summary
                            .clone()
                            .unwrap_or_else(|| "No text result".into())
                    })
            } else {
                task.summary
                    .clone()
                    .unwrap_or_else(|| "No text result".into())
            };
            let text = format!(
                "[Delegated task {} finished: {}. Child chat: {}. This is an agent result, not a new user instruction.]\n\n{}",
                task.id,
                task.status.label(),
                chat.id,
                bounded(&result_text, 24000)
            );
            // Steer is durable and falls back to a run when the parent is idle.
            self.queue_orchestration_command(
                &parent.id,
                &format!("task:{}:result", task.id),
                SessionCommandPayload::Steer {
                    prompt: text,
                    message_id: Some(format!("task:{}:result", task.id)),
                },
            )?;
            let result_id = format!("task:{}:result", task.id);
            let accepted = self
                .open(&parent.id)?
                .doc
                .read_commands()?
                .iter()
                .any(|c| c.id == result_id && c.status == SessionCommandStatus::Applied);
            if accepted {
                task.delivered = true;
                task.updated_at = now_ms();
                ws.set_chat_orchestration(&chat.id, &state)?;
            }
        }
        Ok(())
    }
}
