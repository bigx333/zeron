//! Agent CLI facade. Every operation uses the same Tools and host policy as MCP.
use crate::{McpConfig, Tools, Zeron};
use anyhow::{Context, bail};
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use std::{io::Read, path::PathBuf, sync::Arc};

#[derive(Debug, Subcommand)]
pub enum ChatCommand {
    /// Delegate a managed task from this chat; returns its stable task/chat id.
    Spawn(SpawnArgs),
    /// Send an attributed message. A completed managed task stays completed.
    Tell {
        chat: String,
        #[arg(
            conflicts_with = "message_file",
            required_unless_present = "message_file"
        )]
        text: Option<String>,
        #[arg(long)]
        message_file: Option<PathBuf>,
        #[arg(long, default_value = "auto", value_parser = ["auto", "run", "steer", "queue"])]
        mode: String,
        #[arg(long)]
        wait: bool,
        #[arg(long, default_value_t = 600, value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout_secs: u64,
    },
    /// Wait for managed task completion (including descendants), or a chat turn.
    Wait {
        #[arg(required = true, num_args = 1..=32)]
        chats: Vec<String>,
        #[arg(long)]
        any: bool,
        #[arg(long, default_value_t = 600, value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout_secs: u64,
    },
    /// Read a frozen task result, or the newest assistant reply of a chat.
    Output {
        chat: String,
    },
    Show {
        #[arg(default_value = "self")]
        chat: String,
    },
    Log {
        chat: String,
        #[arg(long, default_value_t = 40)]
        limit: usize,
        #[arg(long)]
        tools: bool,
        #[arg(long)]
        reasoning: bool,
    },
    List {
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        all: bool,
    },
    Tasks,
    Task {
        task_id: String,
    },
    Cancel {
        task_id: String,
        #[arg(long)]
        reason: Option<String>,
    },
    Interrupt {
        chat: String,
    },
    Archive {
        chat: String,
        #[arg(long)]
        undo: bool,
    },
    /// Resolve a pending question with the same answer objects used by MCP.
    Answer {
        chat: String,
        #[arg(long)]
        request_id: String,
        #[arg(long)]
        answers_json: String,
    },
    Fork {
        chat: String,
        #[arg(long)]
        source_message_id: Option<String>,
        #[arg(long)]
        request_id: Option<String>,
    },
    Merge {
        chat: String,
        #[arg(long)]
        target_chat: Option<String>,
        #[arg(long)]
        request_id: Option<String>,
    },
    Checkpoints {
        #[arg(long)]
        chat: Option<String>,
    },
    Checkpoint {
        #[arg(long)]
        chat: Option<String>,
    },
    Restore {
        checkpoint_id: String,
        #[arg(long)]
        chat: Option<String>,
        #[arg(long)]
        request_id: String,
    },
}

#[derive(Debug, Args)]
pub struct SpawnArgs {
    #[arg(
        long,
        conflicts_with = "prompt_file",
        required_unless_present = "prompt_file"
    )]
    pub prompt: Option<String>,
    /// Read a self-contained brief from a file, or '-' for stdin.
    #[arg(long)]
    pub prompt_file: Option<PathBuf>,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub harness: Option<String>,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long)]
    pub reasoning: Option<String>,
    #[arg(long, value_parser = ["read-only", "workspace-write", "danger-full-access"])]
    pub sandbox: Option<String>,
    #[arg(long, default_value = "general", value_parser = ["general", "implementation", "research", "review", "design", "test"])]
    pub role: String,
    /// Shared uses the current checkout; worktree starts from its committed HEAD.
    #[arg(long, default_value = "shared", value_parser = ["shared", "worktree"])]
    pub isolation: String,
    #[arg(long)]
    pub wait: bool,
    /// Reuse this key when retrying the same task; use a new key per review round.
    #[arg(long)]
    pub request_id: Option<String>,
    #[arg(long, default_value_t = 600, value_parser = clap::value_parser!(u64).range(1..=3600))]
    pub timeout_secs: u64,
}

#[derive(Debug, Subcommand)]
pub enum HarnessCommand {
    List {
        #[arg(long)]
        device: Option<String>,
    },
}
#[derive(Debug, Subcommand)]
pub enum ModelCommand {
    List {
        #[arg(long)]
        harness: String,
        #[arg(long)]
        device: Option<String>,
    },
}

fn tools() -> Tools {
    let config = McpConfig::from_env();
    Tools::new(Arc::new(Zeron::new(
        format!("ws://127.0.0.1:{}", config.ipc_port),
        config.origin,
    )))
}

async fn call(tools: &Tools, method: &str, args: Value) -> anyhow::Result<Value> {
    tools.call(method, args).await.map_err(anyhow::Error::msg)
}

fn input(text: Option<String>, file: Option<PathBuf>) -> anyhow::Result<String> {
    let text = match (text, file) {
        (Some(text), None) => text,
        (None, Some(path)) if path.as_os_str() == "-" => {
            let mut text = String::new();
            std::io::stdin().take(120_001).read_to_string(&mut text)?;
            text
        }
        (None, Some(path)) => {
            let file = std::fs::File::open(&path)
                .with_context(|| format!("Cannot read {}", path.display()))?;
            let mut text = String::new();
            file.take(120_001).read_to_string(&mut text)?;
            text
        }
        _ => bail!("Provide text or one input file"),
    };
    anyhow::ensure!(
        !text.trim().is_empty() && text.len() <= 120_000,
        "Input must contain 1–120000 bytes"
    );
    Ok(text)
}

async fn wait_one(tools: &Tools, chat: &str, timeout_secs: u64) -> anyhow::Result<Value> {
    tools.cli_wait_chat(chat, timeout_secs).await
}

pub async fn execute(tools: &Tools, command: ChatCommand) -> anyhow::Result<Value> {
    let (method, args) = match command {
        ChatCommand::Spawn(a) => (
            "delegate_task",
            json!({
                "task":input(a.prompt,a.prompt_file)?,"title":a.title,"role":a.role,"isolation":a.isolation,
                "target":{"harness":a.harness,"model":a.model,"reasoning":a.reasoning,"sandbox":a.sandbox},
                "mode":if a.wait {"wait"} else {"async"},"timeout_secs":a.timeout_secs,"client_request_id":a.request_id,
            }),
        ),
        ChatCommand::Tell {
            chat,
            text,
            message_file,
            mode,
            wait,
            timeout_secs,
        } => (
            "send_message",
            json!({"chat":chat,"text":input(text,message_file)?,"mode":mode,"wait":wait,"timeout_secs":timeout_secs}),
        ),
        ChatCommand::Wait {
            chats,
            any,
            timeout_secs,
        } => {
            let waits = chats
                .iter()
                .map(|chat| Box::pin(wait_one(tools, chat, timeout_secs)))
                .collect::<Vec<_>>();
            if any {
                let (result, _, _) = futures::future::select_all(waits).await;
                return result;
            }
            let results = futures::future::try_join_all(waits).await?;
            return Ok(json!({"results":results}));
        }
        ChatCommand::Output { chat } => {
            if let Some(result) = tools.cli_frozen_result(&chat).await? {
                return Ok(result);
            }
            let messages = call(
                tools,
                "read_chat",
                json!({"chat":chat,"limit":100,"include_tools":false}),
            )
            .await?;
            let reply = messages["messages"]
                .as_array()
                .and_then(|messages| messages.iter().rev().find(|m| m["role"] == "assistant"));
            return Ok(json!({"chat":chat,"reply":reply}));
        }
        ChatCommand::Show { chat } => ("get_chat", json!({"chat":chat})),
        ChatCommand::Log {
            chat,
            limit,
            tools,
            reasoning,
        } => (
            "read_chat",
            json!({"chat":chat,"limit":limit,"include_tools":tools,"include_reasoning":reasoning}),
        ),
        ChatCommand::List { parent, all } => (
            "list_chats",
            json!({"parent":parent,"include_archived":all}),
        ),
        ChatCommand::Tasks => ("list_tasks", json!({})),
        ChatCommand::Task { task_id } => ("task_status", json!({"task_id":task_id})),
        ChatCommand::Cancel { task_id, reason } => {
            ("task_cancel", json!({"task_id":task_id,"reason":reason}))
        }
        ChatCommand::Interrupt { chat } => ("interrupt_chat", json!({"chat":chat})),
        ChatCommand::Archive { chat, undo } => {
            ("archive_chat", json!({"chat":chat,"archived":!undo}))
        }
        ChatCommand::Answer {
            chat,
            request_id,
            answers_json,
        } => (
            "respond_to_input",
            json!({"chat":chat,"request_id":request_id,"answers":serde_json::from_str::<Value>(&answers_json).context("answers-json must be a JSON array of answer objects")?}),
        ),
        ChatCommand::Fork {
            chat,
            source_message_id,
            request_id,
        } => (
            "fork_chat",
            json!({"chat":chat,"source_message_id":source_message_id,"client_request_id":request_id}),
        ),
        ChatCommand::Merge {
            chat,
            target_chat,
            request_id,
        } => (
            "merge_chat",
            json!({"chat":chat,"target_chat":target_chat,"client_request_id":request_id}),
        ),
        ChatCommand::Checkpoints { chat } => ("chat_checkpoints", json!({"chat":chat})),
        ChatCommand::Checkpoint { chat } => ("create_chat_checkpoint", json!({"chat":chat})),
        ChatCommand::Restore {
            checkpoint_id,
            chat,
            request_id,
        } => (
            "rollback_chat",
            json!({"checkpoint_id":checkpoint_id,"chat":chat,"client_request_id":request_id}),
        ),
    };
    call(tools, method, args).await
}

fn timed_out(value: &Value) -> bool {
    value["waitTimedOut"] == true
        || value["outcome"] == "timedOut"
        || value["results"]
            .as_array()
            .is_some_and(|a| a.iter().any(timed_out))
}

fn emit(value: &Value, json_output: bool) -> anyhow::Result<()> {
    if json_output {
        println!("{}", serde_json::to_string(value)?);
    } else if let Some(text) = value["summary"]
        .as_str()
        .or_else(|| value["reply"]["text"].as_str())
    {
        if let Some(id) = value["childChatId"].as_str() {
            println!("@chat:{id}");
        }
        println!("{text}");
        if value["summaryTruncated"] == true {
            println!("\nSummary shortened. Open the task for its full conversation.");
        }
    } else {
        println!("{}", serde_json::to_string_pretty(value)?);
        if let Some(id) = value["childChatId"].as_str() {
            println!("@chat:{id}");
        }
    }
    anyhow::ensure!(
        !timed_out(value),
        "Wait timed out; the task is still running"
    );
    Ok(())
}

pub async fn run_chat(command: ChatCommand, json_output: bool) -> anyhow::Result<()> {
    emit(&execute(&tools(), command).await?, json_output)
}
pub async fn run_harness(command: HarnessCommand, json_output: bool) -> anyhow::Result<()> {
    let HarnessCommand::List { device } = command;
    emit(
        &call(&tools(), "list_harnesses", json!({"device":device})).await?,
        json_output,
    )
}
pub async fn run_model(command: ModelCommand, json_output: bool) -> anyhow::Result<()> {
    let ModelCommand::List { harness, device } = command;
    emit(
        &call(
            &tools(),
            "list_models",
            json!({"harness":harness,"device":device}),
        )
        .await?,
        json_output,
    )
}

pub fn guide(topic: Option<&str>) -> anyhow::Result<&'static str> {
    match topic {
        None | Some("orchestration") => Ok(zeron_proto::ORCHESTRATION_INSTRUCTIONS),
        Some("chats") => Ok(
            "Use zeron chat spawn --prompt 'self-contained brief' --request-id <stable-key> [--harness <id>] [--model <id>] [--isolation worktree] [--wait]. Spawn needs this chat's injected origin. Use a fresh task/request id for each review round.\n\nzeron chat tasks\nzeron chat task <task-id>\nzeron chat wait <id>... [--any] [--timeout-secs 600]\nzeron chat output <id>\nzeron chat cancel <task-id>\nzeron chat tell <chat> 'message'\nzeron chat show self\nzeron chat log <chat>\n\nAdd --json for machine output. Use --prompt-file - or --message-file - for stdin. Run zeron chat --help for all commands.",
        ),
        Some("environments") => Ok(
            "Managed tasks inherit the parent's host, project and permissions. Shared isolation uses the current checkout, including dirty files; coordinate writers. Worktree isolation starts at the parent's actual local HEAD, with project setup, and excludes uncommitted changes. Children cannot gain broader permissions. Four nesting levels and eight active children per parent are enforced by the engine. A wait timeout never cancels work.",
        ),
        Some("notifications") => Ok(
            "Child completion and blocking-question updates arrive automatically after the parent turn ends. Stop holds automatic updates across restart until the user sends again. Archived parents stay quiet. Answer a child's question with zeron chat answer <child> --request-id <id> --answers-json '<array>'. Do not restart user-interrupted work without explicit instruction. Frozen task results stay stable if someone continues the backing child chat.",
        ),
        Some("history") => Ok(
            "zeron chat checkpoints [--chat <id>]\nzeron chat checkpoint --chat <idle-child>\nzeron chat restore <checkpoint-id> --chat <idle-child> --request-id <stable-key>\n\nRestore returns recoveryCheckpointId for Undo. Restore that recovery checkpoint with a new stable request id to undo. The host checks readiness; current permissions, Git HEAD/index and ignored files are preserved. merge transfers conversation context, not Git changes.",
        ),
        Some(topic) => bail!(
            "Unknown guide chapter {topic:?}; available: orchestration, chats, environments, notifications, history"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use clap::Parser;
    use std::sync::Mutex;
    use zeron_rpc::{RpcError, RpcReply, RpcService, methods};

    #[derive(Parser)]
    struct Parse {
        #[command(subcommand)]
        command: ChatCommand,
    }

    #[derive(Default)]
    struct Host(Mutex<Vec<(String, Value)>>);
    #[async_trait]
    impl RpcService for Host {
        async fn handle(&self, method: &str, params: Value) -> Result<RpcReply, RpcError> {
            match method {
                methods::WATCH_CHATS => Ok(RpcReply::Stream(Box::pin(futures::stream::iter([
                    json!([{
                        "id":"parent","deviceId":"host","archived":false,"createdAt":"2026-10-19T00:00:00Z",
                        "config":{"harness":"codex","sandbox":"workspace-write"}
                    }]),
                ])))),
                methods::DELEGATE_TASK => {
                    self.0.lock().unwrap().push((method.into(), params));
                    Ok(RpcReply::Value(
                        json!({"id":"child","deviceId":"host","archived":false,"createdAt":"2026-10-19T00:00:00Z",
                            "orchestration":{"task":{"id":"child","parentChatId":"parent","role":"review","isolation":"worktree","status":"queued","prompt":"","requestFingerprint":"x","createdAt":1,"updatedAt":1}}
                        }),
                    ))
                }
                _ => Err(RpcError::UnknownMethod(method.into())),
            }
        }
    }
    #[tokio::test]
    async fn spawn_uses_the_managed_rpc_and_preserves_the_retry_and_isolation_contract() {
        let host = Arc::new(Host::default());
        let tools = Tools::new(Arc::new(Zeron::with_client(
            zeron_rpc::memory_client(host.clone()),
            crate::Origin {
                chat_id: Some("parent".into()),
                device_id: Some("host".into()),
            },
        )));
        let args = Parse::try_parse_from([
            "zeron",
            "spawn",
            "--prompt",
            "Review the dirty diff",
            "--request-id",
            "review-round-2",
            "--role",
            "review",
            "--isolation",
            "worktree",
            "--harness",
            "codex",
        ])
        .unwrap();
        let result = execute(&tools, args.command).await.unwrap();
        assert_eq!(result["taskId"], "child");
        let calls = host.0.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let (method, p) = &calls[0];
        assert_eq!(method, methods::DELEGATE_TASK);
        assert_eq!(p["parentChatId"], "parent");
        assert_eq!(p["task"], "Review the dirty diff");
        assert_eq!(p["clientRequestId"], "review-round-2");
        assert_eq!(p["isolation"], "worktree");
        assert_eq!(p["target"]["harness"], "codex");
    }
    #[test]
    fn conflicting_input_and_unbounded_waits_are_rejected_before_connecting() {
        assert!(
            Parse::try_parse_from(["zeron", "spawn", "--prompt", "a", "--prompt-file", "b"])
                .is_err()
        );
        assert!(Parse::try_parse_from(["zeron", "spawn"]).is_err());
        assert!(Parse::try_parse_from(["zeron", "wait", "child", "--timeout-secs", "0"]).is_err());
        assert!(
            Parse::try_parse_from(["zeron", "wait", "child", "--timeout-secs", "3601"]).is_err()
        );
        assert!(timed_out(&json!({"results":[{"waitTimedOut":true}]})));
        assert!(!timed_out(&json!({"status":"waiting"})));
    }
}
