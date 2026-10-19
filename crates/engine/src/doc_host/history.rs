//! Host-owned, restart-safe checkpoints. Entry objects are content-addressed;
//! checkpoints share them instead of copying whole conversations every turn.
use super::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use zeron_proto::{ChatConfig, ProviderPoint, ProviderSessionState, RunCheckpoint};

#[derive(Serialize, Deserialize)]
struct StoredCheckpoint {
    record: RunCheckpoint,
    entries: Vec<String>,
    tree: Option<String>,
    root: Option<PathBuf>,
    config: Option<ChatConfig>,
    providers: Vec<ProviderSessionState>,
}

#[derive(Serialize, Deserialize)]
struct RestoreReceipt {
    checkpoint: String,
    recovery: String,
    completed: bool,
    provider: Option<ProviderPoint>,
    provider_resolved: bool,
    #[serde(default)]
    files_started: bool,
}

async fn durable(path: PathBuf, bytes: Vec<u8>) -> Result<(), EngineError> {
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent)?;
        let temp = parent.join(format!(".{}.tmp", new_id()));
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(temp, &path)?;
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok::<_, std::io::Error>(())
    })
    .await
    .map_err(|e| EngineError::Other(e.to_string()))??;
    Ok(())
}

impl DocHost {
    fn history_dir(&self, chat: &str) -> Result<PathBuf, EngineError> {
        let repos = self
            .inner
            .repos
            .get()
            .ok_or_else(|| EngineError::Other("Checkpoint storage is unavailable".into()))?;
        Ok(repos
            .data_dir()
            .join("history")
            .join(super::orchestration::stable_id("chat", chat)))
    }
    fn history_chat(&self, chat: &str) -> Result<zeron_proto::Chat, EngineError> {
        let row = self
            .workspace()
            .ok_or_else(|| EngineError::Other("Workspace unavailable".into()))?
            .chat(chat)?
            .ok_or_else(|| EngineError::Other("Chat no longer exists".into()))?;
        if row.device_id != self.device_id() {
            return Err(EngineError::Other(
                "History belongs to the chat's host device".into(),
            ));
        }
        Ok(row)
    }
    async fn read_checkpoint(&self, chat: &str, id: &str) -> Result<StoredCheckpoint, EngineError> {
        if !self
            .open(chat)?
            .doc
            .run_checkpoints()?
            .iter()
            .any(|p| p.id == id)
        {
            return Err(EngineError::Other("Checkpoint no longer exists".into()));
        }
        // Only app-owned UUIDs name local files, never an arbitrary RPC path.
        uuid::Uuid::parse_str(id)
            .map_err(|_| EngineError::Other("Invalid checkpoint id".into()))?;
        let data = tokio::fs::read(self.history_dir(chat)?.join(format!("{id}.json"))).await?;
        serde_json::from_slice(&data)
            .map_err(|e| EngineError::Other(format!("Checkpoint could not be read: {e}")))
    }

    pub(crate) async fn capture_run_checkpoint(
        &self,
        chat_id: &str,
        recovery: bool,
        automatic: bool,
    ) -> Result<RunCheckpoint, EngineError> {
        let _capture = self.inner.history_capture.lock().await;
        let chat = self.history_chat(chat_id)?;
        let handle = self.open(chat_id)?;
        let mut entries = handle.doc.read_entries()?;
        if automatic {
            let boundary = handle
                .doc
                .provider_harness()
                .and_then(|h| {
                    handle
                        .doc
                        .provider_sessions()
                        .into_iter()
                        .rev()
                        .find(|s| s.point.harness == h)
                })
                .and_then(|s| s.last_message_id);
            if let Some(index) = boundary.and_then(|id| entries.iter().position(|e| e.id == id)) {
                entries.truncate(index + 1);
            }
        }
        let joined = join_continuation_entries(entries.clone());
        let last = joined.last().map(|e| e.id.clone());
        let id = if automatic {
            super::orchestration::stable_id(
                chat_id,
                &format!("checkpoint:{}", last.as_deref().unwrap_or("empty")),
            )
        } else {
            new_id()
        };
        if automatic
            && let Some(existing) = handle
                .doc
                .run_checkpoints()?
                .into_iter()
                .find(|p| p.id == id)
        {
            return Ok(existing);
        }
        let base = self.history_dir(chat_id)?;
        let mut keys = Vec::new();
        for entry in &entries {
            let bytes = serde_json::to_vec(entry).map_err(|e| EngineError::Other(e.to_string()))?;
            let key = format!("{:x}", sha2::Sha256::digest(&bytes));
            let path = base.join("entries").join(&key);
            if !path.exists() {
                durable(path, bytes).await?;
            }
            keys.push(key);
        }
        let providers = handle.doc.provider_sessions();
        let provider_state = handle
            .doc
            .provider_harness()
            .and_then(|h| providers.iter().rev().find(|s| s.point.harness == h))
            .cloned();
        let outcome = handle.doc.root_run_outcome();
        let native = provider_state.as_ref().is_some_and(|s| {
            s.context_ready
                && s.point.harness == HarnessId::Codex
                && s.point.turn_id.is_some()
                && s.last_message_id.is_some()
                && s.last_message_id.as_ref() == entries.last().map(|entry| &entry.id)
                && outcome.as_ref().map_or_else(
                    || handle.doc.history_was_restored(),
                    |outcome| {
                        outcome.status == zeron_proto::DoneStatus::Completed
                            && Some(&outcome.message_id) == s.last_message_id.as_ref()
                    },
                )
        });
        let provider_point = provider_state.map(|s| s.point);
        let identity = match (&chat.cwd, self.inner.repos.get()) {
            (Some(cwd), Some(repos)) => repos.checkout_identity(Path::new(cwd)).await.ok(),
            _ => None,
        };
        let (tree, root, checkout_id) = if let Some(identity) = identity {
            let store = self
                .inner
                .repos
                .get()
                .unwrap()
                .data_dir()
                .join("history-files")
                .join(&identity.id);
            match super::history_files::capture(&identity.root, &store, &id).await {
                Ok(tree) => (Some(tree), Some(identity.root), Some(identity.id)),
                Err(error) if automatic => {
                    tracing::warn!(chat = chat_id, %error, "turn checkpoint has conversation only");
                    (None, None, None)
                }
                Err(error) => return Err(error),
            }
        } else {
            (None, None, None)
        };
        let title = if automatic && handle.doc.root_run_outcome().is_none() {
            "Before first turn".to_owned()
        } else if recovery {
            "Before restoring checkpoint".to_owned()
        } else {
            joined
                .iter()
                .rev()
                .find(|e| e.role == MessageRole::User)
                .and_then(|e| {
                    e.parts.iter().find_map(|p| match p {
                        MessagePart::Text { text, .. } => Some(
                            text.lines()
                                .next()
                                .unwrap_or("")
                                .chars()
                                .take(100)
                                .collect(),
                        ),
                        _ => None,
                    })
                })
                .unwrap_or_else(|| "Saved checkpoint".into())
        };
        let record = RunCheckpoint {
            id: id.clone(),
            message_id: last,
            title,
            created_at_ms: now_ms().max(
                handle
                    .doc
                    .run_checkpoints()?
                    .first()
                    .map(|p| p.created_at_ms.saturating_add(1))
                    .unwrap_or(0),
            ),
            recovery,
            provider_strategy: if native { "native" } else { "portable" }.into(),
            files_available: tree.is_some(),
            files_changed: None,
            restorable: true,
            blocked_reason: None,
            provider_point,
            checkout_id,
        };
        let stored = StoredCheckpoint {
            record: record.clone(),
            entries: keys,
            tree,
            root,
            config: chat.config,
            providers,
        };
        durable(
            base.join(format!("{id}.json")),
            serde_json::to_vec(&stored).map_err(|e| EngineError::Other(e.to_string()))?,
        )
        .await?;
        handle.doc.record_run_checkpoint(&record)?;
        self.persist_orchestration(&handle)?;
        Ok(record)
    }

    async fn history_blocked(
        &self,
        chat: &zeron_proto::Chat,
        point: Option<&RunCheckpoint>,
    ) -> Result<Option<String>, EngineError> {
        let sessions = self
            .sessions()
            .ok_or_else(|| EngineError::Other("Sessions unavailable".into()))?;
        if sessions.session_status(&chat.id).is_some_and(|s| {
            matches!(
                s.status,
                zeron_proto::SessionStatus::Working | zeron_proto::SessionStatus::AwaitingInput
            )
        }) {
            return Ok(Some("Finish or stop the current turn first".into()));
        }
        if sessions
            .session_status(&chat.id)
            .is_some_and(|s| s.running_subagents > 0)
        {
            return Ok(Some("Stop running subagents before restoring".into()));
        }
        let chats = self.workspace().unwrap().read_chats()?;
        let mut owners = vec![chat.id.clone()];
        let mut seen = HashSet::new();
        while let Some(owner) = owners.pop() {
            if !seen.insert(owner.clone()) {
                continue;
            }
            for child in &chats {
                if let Some(task) = child.orchestration.as_ref().and_then(|o| o.task.as_ref())
                    && task.parent_chat_id == owner
                {
                    if !task.status.is_terminal() {
                        return Ok(Some("Stop delegated tasks before restoring".into()));
                    }
                    if !task.delivered && !task.cancel_requested {
                        return Ok(Some(
                            "Wait for task results to be sent before restoring".into(),
                        ));
                    }
                    owners.push(child.id.clone());
                }
            }
        }
        if self
            .open(&chat.id)?
            .doc
            .read_commands()?
            .iter()
            .any(|c| c.status == SessionCommandStatus::Pending)
        {
            return Ok(Some("Finish or cancel pending messages first".into()));
        }
        if !self.open(&chat.id)?.doc.read_queue()?.is_empty() {
            return Ok(Some(
                "Send or remove queued messages before restoring".into(),
            ));
        }
        if chat
            .orchestration
            .as_ref()
            .and_then(|o| o.task.as_ref())
            .is_some_and(|t| t.status.is_terminal() && !t.delivered && !t.cancel_requested)
        {
            return Ok(Some("Wait for this task's result to be sent first".into()));
        }
        let repos = self.inner.repos.get().unwrap();
        let current = match chat.cwd.as_ref() {
            Some(cwd) => repos.checkout_identity(Path::new(cwd)).await.ok(),
            None => None,
        };
        let saved_id = point.and_then(|p| p.checkout_id.as_ref());
        if saved_id.is_some_and(|id| current.as_ref().is_none_or(|i| &i.id != id)) {
            return Ok(Some("This chat is using a different checkout".into()));
        }
        if let Some(id) = saved_id.or_else(|| current.as_ref().map(|i| &i.id)) {
            for other in self.workspace().unwrap().read_chats()? {
                if other.id == chat.id || other.device_id != chat.device_id {
                    continue;
                }
                let active = sessions.session_status(&other.id).is_some_and(|s| {
                    matches!(
                        s.status,
                        zeron_proto::SessionStatus::Working
                            | zeron_proto::SessionStatus::AwaitingInput
                    )
                }) || other
                    .orchestration
                    .as_ref()
                    .and_then(|o| o.task.as_ref())
                    .is_some_and(|t| !t.status.is_terminal() && !t.cancel_requested);
                if active
                    && let Some(cwd) = other.cwd
                    && repos
                        .checkout_identity(Path::new(&cwd))
                        .await
                        .is_ok_and(|i| &i.id == id)
                {
                    return Ok(Some("Stop tasks working in this checkout first".into()));
                }
            }
        }
        Ok(None)
    }

    pub(crate) async fn chat_checkpoints(
        &self,
        id: &str,
    ) -> Result<Vec<RunCheckpoint>, EngineError> {
        let chat = self.history_chat(id)?;
        let mut points = self.open(id)?.doc.run_checkpoints()?;
        if points.is_empty() {
            return Ok(points);
        }
        let blocked = self.history_blocked(&chat, None).await?;
        let current_id = match chat.cwd.as_ref() {
            Some(cwd) => self
                .inner
                .repos
                .get()
                .unwrap()
                .checkout_identity(Path::new(cwd))
                .await
                .ok()
                .map(|i| i.id),
            None => None,
        };
        for point in &mut points {
            point.blocked_reason = blocked.clone().or_else(|| {
                point
                    .checkout_id
                    .as_ref()
                    .filter(|id| current_id.as_ref() != Some(id))
                    .map(|_| "This chat is using a different checkout".into())
            });
            if point.blocked_reason.is_none() && self.read_checkpoint(id, &point.id).await.is_err()
            {
                point.blocked_reason = Some("Checkpoint files are unavailable on this host".into());
            }
            point.restorable = point.blocked_reason.is_none();
            point.files_changed = None;
        }
        Ok(points)
    }

    pub(crate) async fn create_chat_checkpoint(
        &self,
        id: &str,
    ) -> Result<RunCheckpoint, EngineError> {
        let sessions = self
            .sessions()
            .ok_or_else(|| EngineError::Other("Sessions unavailable".into()))?;
        let _guard = sessions.history_mutation().await;
        let chat = self.history_chat(id)?;
        if let Some(reason) = self.history_blocked(&chat, None).await? {
            return Err(EngineError::Other(reason));
        }
        self.capture_run_checkpoint(id, false, false).await
    }

    pub(crate) async fn rollback_chat(
        &self,
        chat_id: &str,
        checkpoint: &str,
        request: Option<&str>,
        registry: &crate::HarnessRegistry,
    ) -> Result<String, EngineError> {
        if request.is_some_and(|s| s.is_empty() || s.len() > 256) {
            return Err(EngineError::Other(
                "clientRequestId must contain 1–256 bytes".into(),
            ));
        }
        let sessions = self
            .sessions()
            .ok_or_else(|| EngineError::Other("Sessions unavailable".into()))?;
        let _guard = sessions.history_mutation().await;
        let _gate = self.inner.orchestration.lock().await;
        let chat = self.history_chat(chat_id)?;
        let base = self.history_dir(chat_id)?;
        let key = super::orchestration::stable_id(
            chat_id,
            &format!(
                "restore:{}",
                request.map(str::to_owned).unwrap_or_else(new_id)
            ),
        );
        let receipt_path = base.join(format!("receipt-{key}.json"));
        let prior = match tokio::fs::read(&receipt_path).await {
            Ok(data) => Some(
                serde_json::from_slice::<RestoreReceipt>(&data)
                    .map_err(|e| EngineError::Other(e.to_string()))?,
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        if let Some(prior) = &prior {
            if prior.checkpoint != checkpoint {
                return Err(EngineError::Other(
                    "Request id was already used for another checkpoint".into(),
                ));
            }
            if prior.completed {
                return Ok(prior.recovery.clone());
            }
        }
        let target = self.read_checkpoint(chat_id, checkpoint).await?;
        if let Some(task) = chat.orchestration.as_ref().and_then(|o| o.task.as_ref()) {
            // A task's published result is immutable even when its backing
            // conversation moves to a checkpoint before that result.
            self.task_result(&task.parent_chat_id, chat_id)?;
        }
        let mut entries = Vec::new();
        for key in &target.entries {
            if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(EngineError::Other("Invalid checkpoint entry key".into()));
            }
            entries.push(
                serde_json::from_slice::<SessionMessageEntry>(
                    &tokio::fs::read(base.join("entries").join(key)).await?,
                )
                .map_err(|e| EngineError::Other(e.to_string()))?,
            );
        }
        if let Some(reason) = self.history_blocked(&chat, Some(&target.record)).await? {
            return Err(EngineError::Other(reason));
        }
        // Retire even an idle native runtime: it must not accept a new prompt
        // against provider state ahead of the restored conversation.
        sessions.terminate(chat_id).await?;
        let mut receipt = if let Some(prior) = prior {
            prior
        } else {
            let recovery = self.capture_run_checkpoint(chat_id, true, false).await?;
            let receipt = RestoreReceipt {
                checkpoint: checkpoint.into(),
                recovery: recovery.id,
                completed: false,
                provider: None,
                provider_resolved: false,
                files_started: false,
            };
            durable(receipt_path.clone(), serde_json::to_vec(&receipt).unwrap()).await?;
            receipt
        };
        let recovery = self.read_checkpoint(chat_id, &receipt.recovery).await?;
        if !receipt.provider_resolved {
            if target.record.provider_strategy == "native"
                && let Some(point) = &target.record.provider_point
            {
                let harness = registry.resolve(point.harness)?;
                if harness.supports_native_fork() && point.turn_id.is_some() {
                    let _lease = registry.execution_lease(point.harness).await;
                    let native = harness.fork_session(point).await?;
                    receipt.provider = Some(ProviderPoint {
                        session_id: native,
                        ..point.clone()
                    });
                }
            }
            receipt.provider_resolved = true;
            durable(receipt_path.clone(), serde_json::to_vec(&receipt).unwrap()).await?;
        }
        if let (Some(tree), Some(before), Some(root), Some(id)) = (
            &target.tree,
            &recovery.tree,
            &target.root,
            &target.record.checkout_id,
        ) {
            let store = self
                .inner
                .repos
                .get()
                .unwrap()
                .data_dir()
                .join("history-files")
                .join(id);
            if receipt.files_started {
                // Preserve edits made after an interrupted attempt before
                // resuming its pending receipt.
                self.capture_run_checkpoint(chat_id, true, false).await?;
                super::history_files::recover_interrupted(root, &store, before, tree).await?;
            }
            super::history_files::validate_restore(root, &store, before, tree).await?;
            receipt.files_started = true;
            durable(receipt_path.clone(), serde_json::to_vec(&receipt).unwrap()).await?;
            super::history_files::restore(root, &store, before, tree)
                .await
                .map_err(|e| {
                    EngineError::Other(format!("{e}. Recovery checkpoint: {}", receipt.recovery))
                })?;
        }
        let handle = self.open(chat_id)?;
        handle.doc.restore_messages(&entries)?;
        handle.doc.clear_context_usage()?;
        let states = receipt
            .provider
            .as_ref()
            .map(|p| {
                vec![ProviderSessionState {
                    context_ready: true,
                    point: p.clone(),
                    last_message_id: target
                        .providers
                        .iter()
                        .find(|s| Some(&s.point) == target.record.provider_point.as_ref())
                        .and_then(|s| s.last_message_id.clone()),
                }]
            })
            .unwrap_or_default();
        handle.doc.set_provider_sessions(&states)?;
        handle.doc.mark_history_restore(
            checkpoint,
            &key,
            receipt
                .provider
                .as_ref()
                .map(|p| p.harness)
                .or_else(|| target.config.as_ref().map(|c| c.harness)),
        )?;
        handle.doc.set_fork_history_session(
            receipt
                .provider
                .as_ref()
                .map(|p| p.session_id.as_str())
                .unwrap_or(""),
        )?;
        // Clear stale authoritative outcomes; a subsequent run publishes its
        // own root completion rather than inheriting the removed future.
        handle
            .doc
            .doc()
            .get_map("meta")
            .delete("rootRunOutcome")
            .map_err(DocError::from)?;
        handle.doc.doc().commit();
        if let Some(config) = &target.config {
            let mut config = config.clone();
            if let Some(current) = &chat.config {
                config.sandbox = current.sandbox;
            }
            self.workspace()
                .unwrap()
                .set_chat_config(chat_id, &config)?;
        }
        sessions.restore_provider(chat_id, receipt.provider.as_ref());
        let preview = entries
            .iter()
            .rev()
            .flat_map(|e| e.parts.iter())
            .find_map(|p| match p {
                MessagePart::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .unwrap_or("");
        self.workspace().unwrap().note_message(chat_id, preview);
        self.persist_orchestration(&handle)?;
        self.workspace().unwrap().persist_now()?;
        receipt.completed = true;
        durable(receipt_path, serde_json::to_vec(&receipt).unwrap()).await?;
        Ok(receipt.recovery)
    }
}
