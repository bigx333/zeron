//! Orchestration policy lives in the engine; MCP supplies origin and shapes replies.
use super::*;
use zeron_proto::{DelegateTaskRequest, TaskIsolation, TaskRole, TaskStatus, TaskTarget};
use zeron_rpc::methods;

fn target_schema() -> Value {
    json!({"type":"object","properties":{
        "harness":{"type":"string"},"model":{"type":"string"},"reasoning":{"type":"string"},
        "sandbox":{"type":"string","enum":["read-only","workspace-write","danger-full-access"]},
        "modelOptions":{"type":"object","additionalProperties":true}
    },"additionalProperties":false})
}

pub(super) fn catalog() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "chat_checkpoints",
            description: "List this chat's durable conversation and checkout checkpoints, newest first, with host-authoritative restore availability. Recovery points preserve the state before each restore.",
            input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
        },
        ToolDef {
            name: "create_chat_checkpoint",
            description: "Save an idle chat's conversation and complete tracked/nonignored checkout before an experiment. Pass chat to operate on an idle child while your own turn is active. Does not change Git HEAD or index. Completed root turns also create checkpoints automatically.",
            input_schema: json!({"type":"object","properties":{"chat":{"type":"string"}},"additionalProperties":false}),
        },
        ToolDef {
            name: "rollback_chat",
            description: "Restore an idle chat's checkpoint conversation and working files. First durably saves a recovery checkpoint, returned for Undo. Requires the chat, its delegated tasks and shared checkout to be idle. Native provider history is restored at an exact completed boundary where available; otherwise the next turn uses portable context. Git index, commits, current permissions and ignored files are preserved. Reuse client_request_id for safe retry.",
            input_schema: json!({"type":"object","properties":{"chat":{"type":"string"},"checkpoint_id":{"type":"string"},"client_request_id":{"type":"string","minLength":1,"maxLength":256}},"required":["checkpoint_id"],"additionalProperties":false}),
        },
        ToolDef {
            name: "orchestrator_capabilities",
            description: "Discover live provider/model catalogs, inherited settings, and explicit orchestration capabilities. Managed tasks inherit the current chat's checkout and settings unless overridden. Unavailable providers include constraints. Native fork and rollback are reported separately from portable context transfer.",
            input_schema: json!({"type":"object","properties":{}}),
        },
        ToolDef {
            name: "delegate_task",
            description: "Delegate a self-contained task to a managed child agent. Provider/model, permissions, project and checkout inherit. Parent history is not copied. Optional worktree isolation starts from the parent's local HEAD; uncommitted changes are not copied. Async completion automatically sends the result to this chat, waking or steering it: end your turn instead of polling. Use mode wait only when you need the result immediately; timeout never cancels work. A blocking question returns hasPendingQuestion and childChatId for respond_to_input. A child cannot gain broader permissions. Reuse client_request_id to safely retry the same task. Each delegated review round needs a new task and request id, with the original brief and prior findings. Messaging childChatId does not reopen a completed task. Completed results remain stable across later child messages.",
            input_schema: json!({"type":"object","properties":{
            "task":{"type":"string","minLength":1,"maxLength":120000},"title":{"type":"string","maxLength":512},
            "role":{"type":"string","enum":["implementation","research","review","design","test","general"]},
            "target":target_schema(),"isolation":{"type":"string","enum":["shared","worktree"],"default":"shared"},
            "mode":{"type":"string","enum":["async","wait"],"default":"async"},
            "timeout_secs":{"type":"integer","minimum":1,"maximum":3600,"default":600},
            "client_request_id":{"type":"string","minLength":1,"maxLength":256}
        },"required":["task"],"additionalProperties":false}),
        },
        ToolDef {
            name: "task_status",
            description: "Read one of your delegated tasks by task_id. Returns the durable result, work state, child chat and pending-question flag. Published results remain stable after completion. A task owned by another parent is rejected.",
            input_schema: json!({"type":"object","properties":{"task_id":{"type":"string"}},"required":["task_id"]}),
        },
        ToolDef {
            name: "task_cancel",
            description: "Stop one of your managed tasks and its descendants, hold queued work, and suppress automatic result delivery. Published task results are preserved. Idempotent; cancellation never deletes the child conversation or worktree.",
            input_schema: json!({"type":"object","properties":{"task_id":{"type":"string"},"reason":{"type":"string","maxLength":2000}},"required":["task_id"]}),
        },
        ToolDef {
            name: "list_tasks",
            description: "List your durable delegated tasks, including finished and archived children. Results include stable task ids, statuses and summaries.",
            input_schema: json!({"type":"object","properties":{}}),
        },
        ToolDef {
            name: "fork_chat",
            description: "Create a side conversation from a completed assistant response, optionally choosing another provider/model. Records source lineage and copies the visible transcript. The first run resolves an exact native Codex fork when available, otherwise a bounded portable context transfer; source sessions and live approvals are never shared. Defaults to the latest completed response; source_message_id selects an earlier response. Reuse client_request_id for retries.",
            input_schema: chat_key_schema(json!({
                "source_message_id":{"type":"string"},"target":target_schema(),"title":{"type":"string"},
                "client_request_id":{"type":"string","minLength":1,"maxLength":256}
            })),
        },
        ToolDef {
            name: "merge_chat",
            description: "Send a finished side conversation's result back to its parent, or an explicit target chat on the same device. Records an inspectable merge artifact and wakes/steers the target. This transfers context, not Git changes. Defaults to the latest completed assistant response; summary overrides the content. Reusing client_request_id is safe.",
            input_schema: chat_key_schema(json!({
                "target_chat":{"type":"string"},"summary":{"type":"string","maxLength":120000},"client_request_id":{"type":"string","minLength":1,"maxLength":256}
            })),
        },
        ToolDef {
            name: "chat_transfers",
            description: "Inspect a chat's recent portable context artifacts: provider handoffs, fork bootstraps and merge-back results. Includes source ids, preparation/consumption state, transferred visible text and truncation. Never contains reasoning or provider-native state.",
            input_schema: chat_key_schema(json!({})),
        },
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DelegateArgs {
    task: String,
    title: Option<String>,
    #[serde(default)]
    role: TaskRole,
    #[serde(default)]
    target: TaskTarget,
    #[serde(default)]
    isolation: TaskIsolation,
    mode: Option<String>,
    timeout_secs: Option<u64>,
    client_request_id: Option<String>,
}
#[derive(Deserialize)]
struct TaskArgs {
    task_id: String,
    reason: Option<String>,
}
#[derive(Deserialize)]
struct ForkArgs {
    chat: String,
    source_message_id: Option<String>,
    #[serde(default)]
    target: TaskTarget,
    title: Option<String>,
    client_request_id: Option<String>,
}
#[derive(Deserialize)]
struct MergeArgs {
    chat: String,
    target_chat: Option<String>,
    summary: Option<String>,
    client_request_id: Option<String>,
}

pub(super) fn task_result(chat: &Chat) -> Value {
    let Some(task) = chat.orchestration.as_ref().and_then(|o| o.task.as_ref()) else {
        return Value::Null;
    };
    json!({"taskId":task.id,"childChatId":chat.id,"parentChatId":task.parent_chat_id,"status":task.status,
        "role":task.role,"isolation":task.isolation,"harness":chat.config.as_ref().map(|c|c.harness),"model":chat.config.as_ref().and_then(|c|c.model.as_ref()),
        "workState":if task.status.is_terminal(){"result_available"}else if task.status==TaskStatus::WaitingForChildren{"waiting_for_children"}else{"working"},
        "hasPendingQuestion":task.status==TaskStatus::Waiting,"summary":task.summary,"summaryTruncated":task.summary_truncated,"resultMessageId":task.result_message_id,
        "resultDelivered":task.delivered,"cancelRequested":task.cancel_requested,"waitTimedOut":false})
}

impl Tools {
    async fn complete_task_result(&self, chat: &Chat) -> Value {
        let mut result = task_result(chat);
        let Some(task) = chat.orchestration.as_ref().and_then(|o| o.task.as_ref()) else {
            return result;
        };
        if task.summary_truncated {
            if let Ok(saved) = self.zeron.call(methods::TASK_RESULT, json!({"parentChatId":task.parent_chat_id,"taskId":task.id,"targetDeviceId":chat.device_id})).await
                && let Ok(saved) = serde_json::from_value::<zeron_proto::TaskResult>(saved) {
                result["summary"] = json!(saved.summary); result["summaryTruncated"] = json!(saved.truncated);
                return result;
            }
            let Some(id) = &task.result_message_id else {
                return result;
            };
            match self
                .zeron
                .transcript_on(&chat.id, Some(&chat.device_id))
                .await
            {
                Ok(entries) => {
                    let entries = zeron_doc::join_continuation_entries(entries);
                    if let Some(entry) = entries.iter().find(|entry| &entry.id == id) {
                        let text = entry
                            .parts
                            .iter()
                            .filter_map(|part| match part {
                                zeron_doc::MessagePart::Text { text, .. } => Some(text.as_str()),
                                zeron_doc::MessagePart::Error { message, .. } => {
                                    Some(message.as_str())
                                }
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        if !text.is_empty() {
                            result["summaryTruncated"] = json!(text.chars().count() > 24000);
                            result["summary"] = json!(text.chars().take(24000).collect::<String>());
                        }
                    }
                }
                Err(error) => result["resultReadError"] = json!(error.to_string()),
            }
        }
        result
    }

    async fn orchestration_parent(&self) -> anyhow::Result<Chat> {
        let origin = self
            .zeron
            .origin()
            .chat_id
            .as_deref()
            .context("Managed orchestration requires a Zeron chat origin")?;
        self.zeron.resolve_chat(origin).await
    }
    pub(super) async fn orchestration_call(
        &self,
        name: &str,
        args: Value,
    ) -> anyhow::Result<Value> {
        match name {
            "chat_checkpoints" | "create_chat_checkpoint" | "rollback_chat" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct RestoreArgs {
                    chat: Option<String>,
                    checkpoint_id: Option<String>,
                    client_request_id: Option<String>,
                }
                let a: RestoreArgs = parse(args).map_err(anyhow::Error::msg)?;
                let chat = if let Some(chat) = a.chat {
                    self.zeron.resolve_chat(&chat).await?
                } else {
                    self.orchestration_parent().await?
                };
                if name == "rollback_chat" {
                    anyhow::ensure!(a.checkpoint_id.is_some(), "checkpoint_id is required");
                }
                let method = match name {
                    "rollback_chat" => methods::ROLLBACK_CHAT,
                    "create_chat_checkpoint" => methods::CREATE_CHAT_CHECKPOINT,
                    _ => methods::CHAT_CHECKPOINTS,
                };
                self.zeron.call(method, json!({"chatId":chat.id,"checkpointId":a.checkpoint_id,"clientRequestId":a.client_request_id,"targetDeviceId":chat.device_id})).await
            }
            "orchestrator_capabilities" => {
                let parent = match self.zeron.origin().chat_id.as_deref() {
                    Some(id) => Some(self.zeron.resolve_chat(id).await?),
                    None => None,
                };
                let device_id = match parent.as_ref() {
                    Some(chat) => chat.device_id.clone(),
                    None => self.zeron.local_device_id().await?,
                };
                let device = Some(device_id.as_str());
                let devices = self.zeron.devices().await?;
                let supported = devices
                    .iter()
                    .find(|d| d.id == device_id)
                    .is_some_and(|d| d.supports(zeron_proto::capabilities::ORCHESTRATION_V2));
                let harnesses = self.zeron.harnesses_on(device).await?;
                let history = devices.iter().find(|d| d.id == device_id).is_some_and(|d| {
                    d.supports(zeron_proto::capabilities::ORCHESTRATION_HISTORY_V2)
                });
                let native = history
                    && harnesses
                        .iter()
                        .any(|h| h.id == HarnessId::Codex && h.available());
                let providers = futures::future::join_all(harnesses.into_iter().map(|h| async move {
                    let (models, error) = if h.available() {
                        match tokio::time::timeout(Duration::from_secs(20), self.zeron.models_on(h.id, device)).await {
                            Ok(Ok(models)) => (json!(models), None),
                            Ok(Err(error)) => (json!([]), Some(error.to_string())),
                            Err(_) => (json!([]), Some("Model discovery timed out".into())),
                        }
                    } else { (json!([]), Some("Provider is missing or disabled".into())) };
                    json!({"harness":h.id,"name":h.name,"available":h.available(),"models":models,"constraint":error,"supportsSteering":h.supports_steering,"steersMidTurn":h.steers_mid_turn(),"reasoningLevels":h.reasoning_levels,"nativeFork":history && h.id == HarnessId::Codex,"nativeCheckpointRestore":history && h.id == HarnessId::Codex})
                })).await;
                Ok(
                    json!({"inherited":parent.as_ref().and_then(|c|c.config.as_ref()),"deviceId":device,"providers":providers,
                    "features":{"managedTasks":supported && parent.is_some(),"asyncResultDelivery":supported,"taskCancellation":supported,"taskDepthLimit":4,"worktreeIsolation":supported,"batchCreation":true,"portableFork":supported,"providerHandoff":supported,"mergeBack":supported,"nativeFork":native,"rollback":history,"nativeCheckpointRestore":native,"providerSessionReactivation":history},
                    "contextBudgetChars":48000}),
                )
            }
            "delegate_task" => {
                let a: DelegateArgs = parse(args).map_err(anyhow::Error::msg)?;
                anyhow::ensure!(
                    a.mode
                        .as_deref()
                        .is_none_or(|m| m == "async" || m == "wait"),
                    "mode must be async or wait"
                );
                let parent = self.orchestration_parent().await?;
                let mut p = serde_json::to_value(DelegateTaskRequest {
                    parent_chat_id: parent.id.clone(),
                    task: a.task,
                    title: a.title,
                    role: a.role,
                    isolation: a.isolation,
                    target: a.target,
                    client_request_id: a.client_request_id,
                })?;
                p["targetDeviceId"] = json!(parent.device_id);
                let chat: Chat =
                    serde_json::from_value(self.zeron.call(methods::DELEGATE_TASK, p).await?)?;
                if a.mode.as_deref() == Some("wait") {
                    self.await_task(&parent, &chat.id, wait_duration(a.timeout_secs))
                        .await
                } else {
                    Ok(self.complete_task_result(&chat).await)
                }
            }
            "task_status" | "task_cancel" => {
                let a: TaskArgs = parse(args).map_err(anyhow::Error::msg)?;
                let parent = self.orchestration_parent().await?;
                let method = if name == "task_cancel" {
                    methods::TASK_CANCEL
                } else {
                    methods::TASK_STATUS
                };
                let chat:Chat=serde_json::from_value(self.zeron.call(method,json!({"parentChatId":parent.id,"taskId":a.task_id,"reason":a.reason,"targetDeviceId":parent.device_id})).await?)?;
                Ok(self.complete_task_result(&chat).await)
            }
            "list_tasks" => {
                let parent = self.orchestration_parent().await?;
                let chats = self.zeron.chats().await?;
                let mut tasks: Vec<_> = chats
                    .iter()
                    .filter(|c| {
                        c.orchestration
                            .as_ref()
                            .and_then(|o| o.task.as_ref())
                            .is_some_and(|t| t.parent_chat_id == parent.id)
                    })
                    .collect();
                tasks.sort_by_key(|c| std::cmp::Reverse(c.created_at));
                Ok(
                    json!({"parentChatId":parent.id,"tasks":tasks.into_iter().map(task_result).collect::<Vec<_>>()}),
                )
            }
            "fork_chat" => {
                let a: ForkArgs = parse(args).map_err(anyhow::Error::msg)?;
                let source = self.zeron.resolve_chat(&a.chat).await?;
                let inherited = source
                    .config
                    .as_ref()
                    .context("Source chat has no provider configuration")?;
                let config = a.target.resolve(inherited).map_err(anyhow::Error::msg)?;
                if let Some(key) = a.client_request_id.as_ref() {
                    anyhow::ensure!(
                        !key.trim().is_empty() && key.len() <= 256,
                        "Invalid client_request_id"
                    );
                }
                let key = a
                    .client_request_id
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                let id = uuid::Uuid::new_v5(
                    &uuid::Uuid::NAMESPACE_URL,
                    format!("zeron:fork:{}:{key}", source.id).as_bytes(),
                )
                .to_string();
                let result=self.zeron.call(methods::FORK_SIDE_CHAT,json!({"chatId":id,"sourceChatId":source.id,"sourceMessageId":a.source_message_id,"parentChatId":source.parent_chat_id.clone().unwrap_or(source.id.clone()),"config":config,"targetDeviceId":source.device_id})).await?;
                if let Some(title) = a.title {
                    self.zeron
                        .mutate(json!({"op":"renameChat","chatId":id,"title":title}))
                        .await?;
                }
                Ok(result)
            }
            "merge_chat" => {
                let a: MergeArgs = parse(args).map_err(anyhow::Error::msg)?;
                let source = self.zeron.resolve_chat(&a.chat).await?;
                let target = match a.target_chat.as_deref() {
                    Some(id) => Some(self.zeron.resolve_chat(id).await?.id),
                    None => None,
                };
                self.zeron.call(methods::MERGE_CHAT,json!({"sourceChatId":source.id,"targetChatId":target,"summary":a.summary,"clientRequestId":a.client_request_id,"targetDeviceId":source.device_id})).await
            }
            "chat_transfers" => {
                let a: ChatArgs = parse(args).map_err(anyhow::Error::msg)?;
                let chat = self.zeron.resolve_chat(&a.chat).await?;
                let transfers = self
                    .zeron
                    .call(
                        methods::CHAT_TRANSFERS,
                        json!({"chatId":chat.id,"targetDeviceId":chat.device_id}),
                    )
                    .await?;
                Ok(json!({"chatId":chat.id,"transfers":transfers}))
            }
            _ => anyhow::bail!("Unknown orchestration tool"),
        }
    }
    async fn await_task(
        &self,
        parent: &Chat,
        id: &str,
        timeout: Duration,
    ) -> anyhow::Result<Value> {
        let mut watch = self
            .zeron
            .subscribe(methods::WATCH_CHATS, json!({}))
            .await?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let chat:Chat=serde_json::from_value(self.zeron.call(methods::TASK_STATUS,json!({"parentChatId":parent.id,"taskId":id,"targetDeviceId":parent.device_id})).await?)?;
            let mut last = task_result(&chat);
            let task = chat
                .orchestration
                .as_ref()
                .and_then(|o| o.task.as_ref())
                .context("Task disappeared")?;
            if task.status.is_terminal() || task.status == TaskStatus::Waiting {
                return Ok(self.complete_task_result(&chat).await);
            }
            match tokio::time::timeout_at(deadline, watch.recv()).await {
                Ok(Some(_)) => {}
                Ok(None) => anyhow::bail!("Task watch ended; use task_status to resume waiting"),
                Err(_) => {
                    last["waitTimedOut"] = json!(true);
                    return Ok(last);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zeron::Origin;
    use async_trait::async_trait;
    use futures::StreamExt;
    use zeron_rpc::{RpcError, RpcReply, RpcService};
    struct World {
        writes: std::sync::Mutex<Vec<(String, Value)>>,
    }
    impl World {
        fn task() -> Value {
            json!({"id":"child","deviceId":"host","createdAt":"2026-10-19T00:00:00Z","archived":false,"parentChatId":"parent",
            "orchestration":{"task":{"id":"child","parentChatId":"parent","role":"review","isolation":"shared","status":"running","prompt":"brief","requestFingerprint":"fingerprint","createdAt":1,"updatedAt":1}}})
        }
    }
    #[async_trait]
    impl RpcService for World {
        async fn handle(&self, method: &str, params: Value) -> Result<RpcReply, RpcError> {
            match method {
                methods::WATCH_CHATS=>Ok(RpcReply::Stream(futures::stream::once(async{json!([{"id":"parent","deviceId":"host","createdAt":"2026-10-19T00:00:00Z","archived":false},Self::task()])}).chain(futures::stream::pending()).boxed())),
                methods::DELEGATE_TASK|methods::TASK_STATUS|methods::TASK_CANCEL=>{self.writes.lock().unwrap().push((method.into(),params));RpcReply::value(&Self::task())},
                _=>Err(RpcError::UnknownMethod(method.into()))
            }
        }
    }
    fn tools(world: Arc<World>, origin: Option<&str>) -> Tools {
        Tools::new(Arc::new(Zeron::with_client(
            zeron_rpc::memory_client(world),
            Origin {
                chat_id: origin.map(str::to_owned),
                device_id: Some("host".into()),
            },
        )))
    }
    #[tokio::test]
    async fn task_wait_timeout_preserves_work_and_origin_is_not_caller_supplied() {
        let world = Arc::new(World {
            writes: Default::default(),
        });
        let tools = tools(world.clone(), Some("parent"));
        assert!(
            tools
                .call(
                    "delegate_task",
                    json!({"task":"brief","parent_chat_id":"someone-else"})
                )
                .await
                .is_err()
        );
        assert!(world.writes.lock().unwrap().is_empty());
        let result = tools
            .call(
                "delegate_task",
                json!({"task":"brief","mode":"wait","timeout_secs":1}),
            )
            .await
            .unwrap();
        assert_eq!(result["waitTimedOut"], true);
        assert_eq!(result["status"], "running");
        let writes = world.writes.lock().unwrap();
        assert!(
            writes
                .iter()
                .all(|(method, _)| method != methods::TASK_CANCEL)
        );
        let launch = &writes
            .iter()
            .find(|(method, _)| method == methods::DELEGATE_TASK)
            .unwrap()
            .1;
        assert_eq!(launch["parentChatId"], "parent");
        assert_eq!(launch["targetDeviceId"], "host");
    }
    #[tokio::test]
    async fn terminal_mcp_cannot_claim_a_managed_task_parent() {
        let world = Arc::new(World {
            writes: Default::default(),
        });
        let tools = tools(world.clone(), None);
        assert!(
            tools
                .call("delegate_task", json!({"task":"brief"}))
                .await
                .unwrap_err()
                .contains("origin")
        );
        assert!(world.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn initialize_exposes_the_same_guidance_as_provider_runs() {
        let tools = tools(
            Arc::new(World {
                writes: Default::default(),
            }),
            Some("parent"),
        );
        let reply = crate::jsonrpc::handle_request(&tools, json!(1), "initialize", json!({})).await;
        let instructions = reply["result"]["instructions"].as_str().unwrap();
        assert!(instructions.contains(zeron_proto::ORCHESTRATION_INSTRUCTIONS));
        assert!(instructions.contains("For each delegated review round"));
    }
}
