//! App-owned delegation and conversation lineage, independent of provider ids.

use crate::{ChatConfig, HarnessId, ReasoningLevel, SandboxLevel};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskResult {
    pub message_id: Option<String>,
    pub summary: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPoint {
    pub harness: HarnessId,
    pub session_id: String,
    pub cwd: String,
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSessionState {
    pub point: ProviderPoint,
    pub last_message_id: Option<String>,
    #[serde(default)]
    pub context_ready: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCheckpoint {
    pub id: String,
    pub message_id: Option<String>,
    pub title: String,
    pub created_at_ms: i64,
    pub recovery: bool,
    /// `native` restores an exact provider boundary; `portable` starts fresh.
    pub provider_strategy: String,
    pub files_available: bool,
    pub files_changed: Option<u32>,
    pub restorable: bool,
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub provider_point: Option<ProviderPoint>,
    #[serde(default)]
    pub checkout_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Running,
    Waiting,
    WaitingForChildren,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl TaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Running => "Working",
            Self::Waiting => "Needs input",
            Self::WaitingForChildren => "Waiting for tasks",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
            Self::Interrupted => "Interrupted",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskRole {
    Implementation,
    Research,
    Review,
    Design,
    Test,
    #[default]
    General,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskIsolation {
    #[default]
    Shared,
    Worktree,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegatedTask {
    pub id: String,
    pub parent_chat_id: String,
    pub role: TaskRole,
    pub isolation: TaskIsolation,
    #[serde(default)]
    pub base_ref: Option<String>,
    pub status: TaskStatus,
    /// Legacy brief preview. Full briefs are staged in the child document.
    pub prompt: String,
    pub request_fingerprint: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub summary_truncated: bool,
    #[serde(default)]
    pub result_message_id: Option<String>,
    #[serde(default)]
    pub delivered: bool,
    #[serde(default)]
    pub cancel_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkLineage {
    pub source_chat_id: String,
    pub source_message_id: String,
    pub created_at: i64,
    /// Portable visible history; provider sessions and unresolved approvals are never shared.
    pub strategy: String,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatOrchestration {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<DelegatedTask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork: Option<ForkLineage>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskTarget {
    pub harness: Option<HarnessId>,
    pub model: Option<String>,
    pub reasoning: Option<ReasoningLevel>,
    pub sandbox: Option<SandboxLevel>,
    #[serde(default)]
    pub model_options: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DelegateTaskRequest {
    pub parent_chat_id: String,
    pub task: String,
    pub title: Option<String>,
    #[serde(default)]
    pub role: TaskRole,
    #[serde(default)]
    pub isolation: TaskIsolation,
    #[serde(default)]
    pub target: TaskTarget,
    pub client_request_id: Option<String>,
}

impl TaskTarget {
    pub fn resolve(&self, inherited: &ChatConfig) -> Result<ChatConfig, &'static str> {
        let sandbox = self.sandbox.unwrap_or(inherited.sandbox);
        let rank = |s| match s {
            SandboxLevel::ReadOnly => 0,
            SandboxLevel::WorkspaceWrite => 1,
            SandboxLevel::DangerFullAccess => 2,
        };
        if rank(sandbox) > rank(inherited.sandbox) {
            return Err("A child cannot have broader permissions than its parent");
        }
        let harness = self.harness.unwrap_or(inherited.harness);
        let same = harness == inherited.harness;
        Ok(ChatConfig {
            harness,
            model: self
                .model
                .clone()
                .or_else(|| same.then(|| inherited.model.clone()).flatten()),
            reasoning: self
                .reasoning
                .or(if same { inherited.reasoning } else { None }),
            model_options: if self.model_options.is_empty() && same {
                inherited.model_options.clone()
            } else {
                self.model_options.clone()
            },
            sandbox,
        })
    }
}

/// A bounded, inspectable artifact passed between conversations or providers.
/// Contains visible text only, never reasoning, native session state or approvals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextTransfer {
    pub id: String,
    pub kind: String,
    pub source_chat_id: String,
    pub source_message_id: Option<String>,
    pub target_harness: Option<HarnessId>,
    pub created_at: i64,
    pub status: String,
    pub content: String,
    pub truncated: bool,
}

/// Last authoritative root-turn terminal event, including empty/interrupted turns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootRunOutcome {
    pub status: crate::DoneStatus,
    pub message_id: String,
    pub finished_at: i64,
}
/// Shared model-facing policy, delivered by MCP and the engine's run boundary.
pub const ORCHESTRATION_INSTRUCTIONS: &str = include_str!("orchestration_instructions.md");
