//! Opt-in, provider-neutral controller events. Delivery never blocks a run.
use std::time::Duration;

use anyhow::{Context, bail};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use tokio::sync::mpsc;
use zeron_proto::Chat;

/// A bounded, best-effort outbox. Clones hold no engine references.
#[derive(Clone, Default)]
pub struct Notifier {
    tx: Option<mpsc::Sender<Value>>,
}

impl Notifier {
    pub fn from_env() -> anyhow::Result<Self> {
        let read = |key| std::env::var(key).ok().filter(|s| !s.trim().is_empty());
        match (read("ZERON_WEBHOOK_URL"), read("ZERON_WEBHOOK_SECRET")) {
            (None, None) => Ok(Self::default()),
            (Some(url), Some(secret)) => Self::start(&url, secret),
            _ => bail!("ZERON_WEBHOOK_URL and ZERON_WEBHOOK_SECRET must both be set"),
        }
    }

    fn start(url: &str, secret: String) -> anyhow::Result<Self> {
        let url = reqwest::Url::parse(url).map_err(|_| anyhow::anyhow!("invalid webhook URL"))?;
        anyhow::ensure!(
            matches!(url.scheme(), "http" | "https") && url.host().is_some(),
            "webhook URL must be HTTP(S)"
        );
        anyhow::ensure!(
            url.username().is_empty() && url.password().is_none(),
            "webhook URL must not contain credentials"
        );
        anyhow::ensure!(
            secret.len() >= 32,
            "ZERON_WEBHOOK_SECRET must be at least 32 bytes"
        );
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("building webhook client")?;
        let (tx, mut rx) = mpsc::channel::<Value>(256);
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let body = event.to_string();
                let mut delivered = false;
                for attempt in 0..3 {
                    let timestamp = chrono::Utc::now().timestamp().to_string();
                    let signature = signature(&secret, &timestamp, &body);
                    let result = client
                        .post(url.clone())
                        .header("Content-Type", "application/json")
                        .header("X-Zeron-Event-Id", event["id"].as_str().unwrap_or_default())
                        .header("X-Zeron-Timestamp", &timestamp)
                        .header("X-Zeron-Signature", format!("sha256={signature}"))
                        .body(body.clone())
                        .send()
                        .await;
                    match result {
                        Ok(response) if response.status().is_success() => {
                            delivered = true;
                            break;
                        }
                        Ok(response)
                            if !response.status().is_server_error()
                                && response.status().as_u16() != 429 =>
                        {
                            break;
                        }
                        _ => {}
                    }
                    if attempt < 2 {
                        tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
                    }
                }
                if !delivered {
                    // Never log endpoint URLs, signatures, secrets or transcript bodies.
                    tracing::warn!(event_id = ?event["id"], "controller webhook delivery failed");
                }
            }
        });
        Ok(Self { tx: Some(tx) })
    }

    pub fn enabled(&self) -> bool {
        self.tx.is_some()
    }

    pub fn emit(
        &self,
        event_type: &str,
        chat: &Chat,
        status: &str,
        last_reply: Option<String>,
        data: Value,
    ) -> anyhow::Result<String> {
        let tx = self
            .tx
            .as_ref()
            .context("controller webhook is not configured")?;
        let id = uuid::Uuid::new_v4().to_string();
        tx.try_send(json!({
            "schemaVersion": 1,
            "id": id,
            "type": event_type,
            "timestamp": chrono::Utc::now(),
            "chatId": chat.id,
            "deviceId": chat.device_id,
            "title": chat.title,
            "status": status,
            "lastReply": last_reply,
            "data": data,
        }))
        .map_err(|_| anyhow::anyhow!("controller webhook outbox is full or closed"))?;
        Ok(id)
    }
}

fn signature(secret: &str, timestamp: &str, body: &str) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key size");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_matches_independent_hmac_vector() {
        // Python hmac.new(b'secret', b'123.{"ok":true}', hashlib.sha256).
        assert_eq!(
            signature("secret", "123", r#"{"ok":true}"#),
            "12f14ade5e7e737164d9ae20ea4e070056a3045b2c8f42f5f216008eae4684dd"
        );
        assert_ne!(
            signature("secret", "124", "body"),
            signature("secret", "123", "body")
        );
    }
    #[tokio::test]
    async fn retries_keep_body_and_event_id_and_sign_each_request() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let receiver = tokio::spawn(async move {
            let mut requests = Vec::new();
            for attempt in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let (head_end, length) = loop {
                    let mut chunk = [0; 4096];
                    let count = stream.read(&mut chunk).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                    if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..pos]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .map(|s| s.parse::<usize>().unwrap())
                            })
                            .unwrap();
                        break (pos + 4, length);
                    }
                };
                while bytes.len() < head_end + length {
                    let mut chunk = [0; 4096];
                    let count = stream.read(&mut chunk).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                }
                requests.push((
                    String::from_utf8(bytes[..head_end].to_vec()).unwrap(),
                    String::from_utf8(bytes[head_end..head_end + length].to_vec()).unwrap(),
                ));
                let response = if attempt == 0 {
                    "HTTP/1.1 500 Retry\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else {
                    "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                };
                stream.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        });
        let secret = "test-only-signing-secret-1234567890";
        let notifier = Notifier::start(&format!("http://{address}/events"), secret.into()).unwrap();
        // Exercise the worker without constructing a workspace.
        notifier
            .tx
            .as_ref()
            .unwrap()
            .try_send(json!({"id":"stable-event","type":"agent.notification"}))
            .unwrap();
        let requests = tokio::time::timeout(Duration::from_secs(5), receiver)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(requests[0].1, requests[1].1);
        for (headers, body) in requests {
            let header = |key: &str| {
                headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case(key)
                            .then(|| value.trim().to_owned())
                    })
                    .unwrap()
            };
            assert_eq!(header("x-zeron-event-id"), "stable-event");
            assert_eq!(
                header("x-zeron-signature"),
                format!(
                    "sha256={}",
                    signature(secret, &header("x-zeron-timestamp"), &body)
                )
            );
        }
    }

    #[tokio::test]
    async fn engine_events_follow_persisted_replies_and_creation_is_idempotent() {
        use crate::{EngineCore, HarnessRegistry};
        use std::sync::Arc;
        use zeron_proto::{AgentEvent, DoneStatus, HarnessId, RunRequest, SandboxLevel};
        for done_status in [DoneStatus::Completed, DoneStatus::Errored] {
            let dir = tempfile::tempdir().unwrap();
            let registry = Arc::new(HarnessRegistry::new());
            registry.register(Arc::new(zeron_harness::mock::MockHarness {
                script: vec![
                    AgentEvent::TextDelta {
                        text: "Final reply".into(),
                    },
                    AgentEvent::Done {
                        status: done_status,
                        result: None,
                        error: None,
                        session_id: None,
                    },
                ],
            }));
            let core = EngineCore::assemble(dir.path(), registry, HarnessId::Mock, None).unwrap();
            let (tx, mut events) = mpsc::channel(16);
            let notifier = Notifier { tx: Some(tx) };
            core.sessions.set_notifier(notifier.clone());
            core.workspace.set_notifier(notifier);
            core.workspace
                .create_chat(
                    "test-chat",
                    None,
                    Some(&core.device_id),
                    None,
                    Some(dir.path().to_string_lossy().into()),
                )
                .unwrap();
            core.workspace
                .create_chat("test-chat", None, Some(&core.device_id), None, None)
                .unwrap();
            let created = events.recv().await.unwrap();
            assert_eq!(created["type"], "chat.created");
            assert!(events.try_recv().is_err());
            core.workspace
                .rename_chat("test-chat", "Example task")
                .unwrap();
            core.sessions
                .dispatch(
                    "test-chat",
                    HarnessId::Mock,
                    RunRequest {
                        prompt: "test".into(),
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
                    None,
                )
                .await
                .unwrap();
            let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                event["type"],
                if done_status == DoneStatus::Completed {
                    "turn.completed"
                } else {
                    "turn.errored"
                }
            );
            assert_eq!(event["title"], "Example task");
            assert_eq!(event["lastReply"], "Final reply");
            assert_eq!(
                event["status"],
                if done_status == DoneStatus::Completed {
                    "idle"
                } else {
                    "errored"
                }
            );
            core.shutdown().await;
        }
    }

    #[tokio::test]
    async fn parallel_permissions_remain_discoverable_until_both_are_answered() {
        use futures::StreamExt;
        use std::sync::Arc;
        use zeron_harness::{Harness, HarnessError, RunControls};
        use zeron_proto::{
            AgentEvent, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
            SessionStatus, SteeringMode, UserInputQuestion,
        };
        struct Asking;
        #[async_trait::async_trait]
        impl Harness for Asking {
            fn id(&self) -> HarnessId {
                HarnessId::Mock
            }
            fn display_name(&self) -> &str {
                "Parallel approvals"
            }
            fn supports_steering(&self) -> bool {
                false
            }
            fn steering_mode(&self) -> SteeringMode {
                SteeringMode::TurnBoundary
            }
            fn reasoning_levels(&self) -> &[ReasoningLevel] {
                &[]
            }
            async fn models(&self) -> Result<Vec<Model>, HarnessError> {
                Ok(vec![])
            }
            async fn run(
                &self,
                _: RunRequest,
                controls: RunControls,
            ) -> Result<
                futures::stream::BoxStream<'static, Result<AgentEvent, HarnessError>>,
                HarnessError,
            > {
                let ask = |id: &str| {
                    (controls.request_input)(vec![UserInputQuestion {
                        id: id.into(),
                        header: "Approve tool".into(),
                        question: "Allow it?".into(),
                        options: vec!["Yes".into(), "No".into()],
                        multi_select: false,
                        prefill: None,
                        multiline: false,
                    }])
                };
                let first = ask("q1");
                let second = ask("q2");
                Ok(futures::stream::once(async move {
                    let _ = first.await;
                    let _ = second.await;
                    Ok(AgentEvent::Done {
                        status: DoneStatus::Completed,
                        result: None,
                        error: None,
                        session_id: None,
                    })
                })
                .boxed())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(crate::HarnessRegistry::new());
        registry.register(Arc::new(Asking));
        let core =
            crate::EngineCore::assemble(dir.path(), registry, HarnessId::Mock, None).unwrap();
        let (tx, mut events) = mpsc::channel(16);
        core.sessions.set_notifier(Notifier { tx: Some(tx) });
        core.workspace
            .create_chat(
                "asking",
                None,
                Some(&core.device_id),
                None,
                Some(dir.path().to_string_lossy().into()),
            )
            .unwrap();
        let tools = zeron_mcp::Tools::new(Arc::new(zeron_mcp::Zeron::with_client(
            zeron_rpc::memory_client(core.rpc_service()),
            zeron_mcp::Origin::default(),
        )));
        core.sessions
            .dispatch(
                "asking",
                HarnessId::Mock,
                RunRequest {
                    prompt: "ask".into(),
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
                None,
            )
            .await
            .unwrap();
        for _ in 0..2 {
            let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(event["type"], "input.requested");
            assert_eq!(event["data"]["questions"][0]["options"][0], "Yes");
        }
        async fn wait_inputs(tools: &zeron_mcp::Tools, count: usize) -> Value {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let value = tools.call("list_pending_inputs", json!({})).await.unwrap();
                    if value["inputs"][0]["pendingInputs"]
                        .as_array()
                        .is_some_and(|a| a.len() == count)
                    {
                        return value;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap()
        }
        let pending = wait_inputs(&tools, 2).await;
        let inputs = pending["inputs"][0]["pendingInputs"].as_array().unwrap();
        for (index, input) in inputs.iter().enumerate() {
            tools
                .call(
                    "respond_to_input",
                    json!({"chat":"asking","request_id":input["requestId"],
                        "answers":[{"question_id":input["questions"][0]["id"],"labels":["Yes"]}]
                    }),
                )
                .await
                .unwrap();
            if index == 0 {
                wait_inputs(&tools, 1).await;
                assert_eq!(
                    core.sessions.session_status("asking").unwrap().status,
                    SessionStatus::AwaitingInput
                );
            }
        }
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event["type"], "turn.completed");
        core.shutdown().await;
    }
}
