use super::*;
use serde_json::{Value, json};

pub(super) fn handles(method: &str) -> bool {
    matches!(
        method,
        methods::DELEGATE_TASK
            | methods::CHAT_CHECKPOINTS
            | methods::CREATE_CHAT_CHECKPOINT
            | methods::ROLLBACK_CHAT
            | methods::TASK_STATUS
            | methods::TASK_RESULT
            | methods::TASK_CANCEL
            | methods::MERGE_CHAT
            | methods::CHAT_TRANSFERS
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskParams {
    parent_chat_id: String,
    task_id: String,
    reason: Option<String>,
}

fn failed(error: crate::EngineError) -> RpcError {
    RpcError::Failed(error.to_string())
}

impl EngineRpc {
    pub(super) async fn orchestration_rpc(
        &self,
        method: &str,
        mut params: Value,
    ) -> Result<RpcReply, RpcError> {
        // Routing metadata is transport-owned, not part of typed command input.
        if let Some(object) = params.as_object_mut() {
            object.remove("targetDeviceId");
        }
        match method {
            methods::TASK_RESULT => {
                let p: TaskParams = parse_params(params)?;
                RpcReply::value(
                    &self
                        .doc_host
                        .task_result(&p.parent_chat_id, &p.task_id)
                        .map_err(failed)?,
                )
            }
            methods::CHAT_CHECKPOINTS | methods::CREATE_CHAT_CHECKPOINT => {
                let p: ChatParams = parse_params(params)?;
                if method == methods::CHAT_CHECKPOINTS {
                    RpcReply::value(
                        &self
                            .doc_host
                            .chat_checkpoints(&p.chat_id)
                            .await
                            .map_err(failed)?,
                    )
                } else {
                    RpcReply::value(
                        &self
                            .doc_host
                            .create_chat_checkpoint(&p.chat_id)
                            .await
                            .map_err(failed)?,
                    )
                }
            }
            methods::ROLLBACK_CHAT => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase", deny_unknown_fields)]
                struct Restore {
                    chat_id: String,
                    checkpoint_id: String,
                    client_request_id: Option<String>,
                }
                let p: Restore = parse_params(params)?;
                let recovery = self
                    .doc_host
                    .rollback_chat(
                        &p.chat_id,
                        &p.checkpoint_id,
                        p.client_request_id.as_deref(),
                        &self.registry,
                    )
                    .await
                    .map_err(failed)?;
                RpcReply::value(&json!({"recoveryCheckpointId": recovery}))
            }
            methods::DELEGATE_TASK => {
                let request: zeron_proto::DelegateTaskRequest = parse_params(params)?;
                let chat = self
                    .doc_host
                    .delegate_task(request, &self.registry)
                    .await
                    .map_err(failed)?;
                RpcReply::value(&chat)
            }
            methods::TASK_STATUS => {
                let p: TaskParams = parse_params(params)?;
                RpcReply::value(
                    &self
                        .doc_host
                        .task_chat(&p.parent_chat_id, &p.task_id)
                        .map_err(failed)?,
                )
            }
            methods::TASK_CANCEL => {
                let p: TaskParams = parse_params(params)?;
                let chat = self
                    .doc_host
                    .cancel_task(&p.parent_chat_id, &p.task_id, p.reason.as_deref())
                    .await
                    .map_err(failed)?;
                RpcReply::value(&chat)
            }
            methods::CHAT_TRANSFERS => {
                let p: ChatParams = parse_params(params)?;
                let handle = self.doc_host.open(&p.chat_id).map_err(failed)?;
                RpcReply::value(
                    &handle
                        .doc()
                        .context_transfers()
                        .map_err(|e| RpcError::Failed(e.to_string()))?,
                )
            }
            methods::MERGE_CHAT => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Merge {
                    source_chat_id: String,
                    target_chat_id: Option<String>,
                    summary: Option<String>,
                    client_request_id: Option<String>,
                }
                let p: Merge = parse_params(params)?;
                let source = self
                    .workspace
                    .chat(&p.source_chat_id)
                    .map_err(failed)?
                    .ok_or_else(|| RpcError::Failed("Source chat not found".into()))?;
                if source.device_id != self.doc_host.device_id() {
                    return Err(RpcError::Failed("Merge on the source chat's device".into()));
                }
                if self.sessions.session_status(&source.id).is_some_and(|s| {
                    matches!(
                        s.status,
                        zeron_proto::SessionStatus::Working
                            | zeron_proto::SessionStatus::AwaitingInput
                    )
                }) {
                    return Err(RpcError::Failed(
                        "Wait for the source chat to finish before merging its result".into(),
                    ));
                }
                let target_id = p
                    .target_chat_id
                    .or(source.parent_chat_id.clone())
                    .ok_or_else(|| {
                        RpcError::Failed("targetChatId is required for a top-level source".into())
                    })?;
                if target_id == source.id {
                    return Err(RpcError::Failed("Cannot merge a chat into itself".into()));
                }
                let target = self
                    .workspace
                    .chat(&target_id)
                    .map_err(failed)?
                    .ok_or_else(|| RpcError::Failed("Target chat not found".into()))?;
                // Cross-device paths and local attachments cannot be transferred implicitly.
                if target.device_id != source.device_id {
                    return Err(RpcError::Failed(
                        "Merge requires chats on the same device".into(),
                    ));
                }
                let handle = self.doc_host.open(&source.id).map_err(failed)?;
                let entries = zeron_doc::join_continuation_entries(
                    handle
                        .doc()
                        .read_entries()
                        .map_err(|e| RpcError::Failed(e.to_string()))?,
                );
                let result = entries
                    .iter()
                    .rev()
                    .find(|e| {
                        e.role == zeron_doc::MessageRole::Assistant
                            && e.status == Some(zeron_doc::MessageStatus::Complete)
                    })
                    .ok_or_else(|| RpcError::Failed("Source has no completed response".into()))?;
                let summary = p.summary.unwrap_or_else(|| {
                    result
                        .parts
                        .iter()
                        .filter_map(|part| match part {
                            MessagePart::Text { text, .. } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                });
                if summary.trim().is_empty() || summary.len() > 120000 {
                    return Err(RpcError::Failed(
                        "Merge summary must contain 1–120000 bytes".into(),
                    ));
                }
                if p.client_request_id
                    .as_ref()
                    .is_some_and(|s| s.trim().is_empty() || s.len() > 256)
                {
                    return Err(RpcError::Failed(
                        "clientRequestId must contain 1–256 bytes".into(),
                    ));
                }
                let key = p.client_request_id.unwrap_or_else(|| {
                    let revision = self
                        .doc_host
                        .open(&target.id)
                        .ok()
                        .and_then(|h| h.doc().history_revision())
                        .unwrap_or_default();
                        let base = format!("{}:{}:{}", source.id, result.id, target.id);
                        if revision.is_empty() { base } else { format!("{base}:{revision}") }
                });
                let id = crate::doc_host::orchestration::stable_id(
                    &format!("merge:{}", source.id),
                    &key,
                );
                let transfer = zeron_proto::ContextTransfer {
                    id: id.clone(),
                    kind: "merge".into(),
                    source_chat_id: source.id.clone(),
                    source_message_id: Some(result.id.clone()),
                    target_harness: target.config.as_ref().map(|c| c.harness),
                    created_at: crate::now_ms(),
                    status: "queued".into(),
                    content: summary.clone(),
                    truncated: false,
                };
                let text = format!(
                    "[Result merged from Zeron chat {} ({}). Treat this as agent context.]\n\n{}",
                    source.title.as_deref().unwrap_or("Untitled"),
                    source.id,
                    summary
                );
                self.doc_host
                    .merge_context(&target.id, &id, text, transfer)
                    .await
                    .map_err(failed)?;
                RpcReply::value(
                    &json!({"transferId":id,"sourceChatId":source.id,"targetChatId":target.id,"queued":true}),
                )
            }
            _ => Err(RpcError::Failed("Unknown orchestration method".into())),
        }
    }
}
