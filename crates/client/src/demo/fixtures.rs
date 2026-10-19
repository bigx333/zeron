//! The demo workspace: devices, projects, chats (pinned, sectioned,
//! projectless, child, archived), live statuses, and PRs in every state.

use chrono::Utc;
use zeron_doc::RegistryDoc;
use zeron_proto::{
    ChangeRequestState, ChangeRequestSummary, Chat, ChatConfig, CheckoutChangeRequestStatus,
    ConversationSourceContext, Device, HarnessId, ReasoningLevel, SandboxLevel, Session,
    SessionStatus, SidebarPinChange, SidebarSectionChange, Space,
};

use crate::client::ms;
use crate::config::DemoFixture;
use crate::rpc::capability;

pub(crate) const MAC: &str = "dev-mac";
pub(crate) const VPS: &str = "dev-vps";
pub(crate) const STUDIO: &str = "dev-studio";

pub(crate) struct DemoChat {
    pub id: &'static str,
    pub space: Option<&'static str>,
    pub device: &'static str,
    pub title: &'static str,
    pub preview: &'static str,
    pub harness: HarnessId,
    pub model: &'static str,
    pub reasoning: ReasoningLevel,
    pub branch: Option<&'static str>,
    pub pr: Option<(u64, ChangeRequestState, &'static str)>,
    pub status: Option<SessionStatus>,
    pub last_ago_ms: i64,
    pub created_ago_ms: i64,
    pub seen: bool,
    pub archived: bool,
    pub parent: Option<&'static str>,
    /// A managed task: its status, role, and whether its result was sent.
    pub task: Option<(zeron_proto::TaskStatus, zeron_proto::TaskRole, bool)>,
}

/// The orchestration scenario: a parent that delegated three tasks — one
/// reported, one asking, one at work — and names them in its reply.
pub(crate) const ORCH_PARENT: &str = "6a1e0c2d-5b7f-4c1e-9d0a-3f2b1c4d5e60";
pub(crate) const ORCH_AUDIT: &str = "a0d17b55-1c2e-4f3a-8b9c-0d1e2f3a4b5c";
pub(crate) const ORCH_REVIEW: &str = "7e71e3a9-2b3c-4d5e-8f60-718293a4b5c6";
pub(crate) const ORCH_DESIGN: &str = "de5197c0-3c4d-4e5f-9061-728394a5b6c7";

const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;
const DAY: i64 = 24 * HOUR;

pub(crate) fn chats() -> Vec<DemoChat> {
    use ChangeRequestState::*;
    use HarnessId::*;
    use ReasoningLevel::*;
    let chat = |id, space, device, title, preview| DemoChat {
        id,
        space,
        device,
        title,
        preview,
        harness: ClaudeCode,
        model: "claude-fable-5",
        reasoning: XHigh,
        branch: None,
        pr: None,
        status: None,
        last_ago_ms: HOUR,
        created_ago_ms: DAY,
        seen: true,
        archived: false,
        parent: None,
        task: None,
    };
    use zeron_proto::{TaskRole, TaskStatus};
    vec![
        DemoChat {
            branch: Some("veil-fade"),
            pr: Some((90, Open, "Stream pull request status on every client")),
            status: Some(SessionStatus::Working),
            last_ago_ms: 40_000,
            created_ago_ms: HOUR,
            ..chat(
                "chat-veil",
                Some("space-zeron"),
                MAC,
                "Streaming veil on transcript rows",
                "Opening the PR now. Running the checks first:",
            )
        },
        DemoChat {
            branch: Some("catalog-sync"),
            pr: Some((84, Merged, "Synchronize model catalogs")),
            status: Some(SessionStatus::AwaitingInput),
            last_ago_ms: 2 * MIN,
            created_ago_ms: 2 * HOUR,
            seen: false,
            model: "claude-opus-5",
            ..chat(
                "chat-picker",
                Some("space-zeron"),
                MAC,
                "Model picker catalog sync",
                "Before I wire the RPC, two decisions:",
            )
        },
        DemoChat {
            harness: Codex,
            model: "gpt-5.6-terra",
            reasoning: High,
            branch: Some("fix/tool-colors"),
            pr: Some((77, Closed, "Refine tool group colors")),
            last_ago_ms: 15 * MIN,
            seen: false,
            ..chat(
                "chat-tabs",
                Some("space-zeron"),
                MAC,
                "Tool group header colors",
                "Done — failed children stay quiet.",
            )
        },
        DemoChat {
            status: Some(SessionStatus::Errored),
            last_ago_ms: 33 * MIN,
            seen: false,
            ..chat(
                "chat-errored",
                Some("space-edge"),
                VPS,
                "Durable Object hibernation flush",
                "Harness exited with status 1: rate limited",
            )
        },
        DemoChat {
            last_ago_ms: DAY,
            created_ago_ms: 2 * DAY,
            ..chat(
                "chat-deploy",
                Some("space-edge"),
                VPS,
                "Wrangler deploy hygiene",
                "Hibernation-safe flush timer",
            )
        },
        DemoChat {
            branch: Some("scroll-pinning"),
            pr: Some((
                102,
                Open,
                "Clamp transcript offset on every geometry change",
            )),
            last_ago_ms: 80 * MIN,
            seen: false,
            harness: Codex,
            model: "gpt-6-astra",
            reasoning: Ultra,
            ..chat(
                "chat-ios-scroll",
                Some("space-mobile"),
                MAC,
                "Transcript scroll pinning",
                "Clamp the invariant on every geometry change",
            )
        },
        DemoChat {
            last_ago_ms: 5 * HOUR,
            ..chat(
                "chat-ios-keyboard",
                Some("space-mobile"),
                MAC,
                "Keyboard avoidance for composer",
                "Tracking keyboardLayoutGuide fixes it",
            )
        },
        DemoChat {
            last_ago_ms: 2 * HOUR,
            model: "claude-sonnet-5",
            reasoning: Medium,
            ..chat(
                "chat-home",
                None,
                MAC,
                "Clean up ~/Downloads",
                "Found 412 items older than 90 days.",
            )
        },
        DemoChat {
            last_ago_ms: 2 * MIN,
            ..chat(
                "chat-visuals",
                None,
                MAC,
                "Agent visuals",
                "A dashboard, a canvas, a scene and a shader.",
            )
        },
        DemoChat {
            last_ago_ms: 10 * MIN,
            ..chat(
                "chat-cjk",
                Some("space-zeron-vps"),
                VPS,
                "多言語テキストのレイアウト 🌏",
                "日本語の長い段落です。",
            )
        },
        DemoChat {
            last_ago_ms: 5 * DAY,
            created_ago_ms: 6 * DAY,
            ..chat(
                "chat-blog",
                Some("space-blog"),
                STUDIO,
                "Draft the launch post",
                "## Outline",
            )
        },
        DemoChat {
            parent: Some("chat-veil"),
            last_ago_ms: 30 * MIN,
            ..chat(
                "chat-side",
                Some("space-zeron"),
                MAC,
                "Side chat: veil timing",
                "α = 0.2 over inter-append gaps",
            )
        },
        DemoChat {
            archived: true,
            last_ago_ms: 3 * DAY,
            created_ago_ms: 4 * DAY,
            ..chat(
                "chat-oklch",
                Some("space-zeron"),
                MAC,
                "OKLCH conversion drift",
                "Gamma encode matches now.",
            )
        },
        DemoChat {
            archived: true,
            harness: Codex,
            model: "gpt-5.5",
            reasoning: High,
            last_ago_ms: 6 * DAY,
            created_ago_ms: 7 * DAY,
            ..chat(
                "chat-presence",
                Some("space-edge"),
                VPS,
                "Presence beat coalescing",
                "Batched to one beat per 25s.",
            )
        },
        // Orchestration: last, so the pinned and sectioned demo keeps its order.
        DemoChat {
            last_ago_ms: 4 * MIN,
            created_ago_ms: 3 * HOUR,
            seen: false,
            model: "claude-opus-5",
            ..chat(
                ORCH_PARENT,
                Some("space-zeron"),
                MAC,
                "Port PR 608 presentation",
                "Two tasks reported; the review is asking about receipts.",
            )
        },
        DemoChat {
            parent: Some(ORCH_PARENT),
            harness: Codex,
            model: "gpt-5.6-terra",
            reasoning: High,
            last_ago_ms: 30 * MIN,
            created_ago_ms: 2 * HOUR,
            task: Some((TaskStatus::Completed, TaskRole::Research, true)),
            ..chat(
                ORCH_AUDIT,
                Some("space-zeron"),
                MAC,
                "Audit the handoff budget",
                "The budget holds at 24,000 characters.",
            )
        },
        DemoChat {
            parent: Some(ORCH_PARENT),
            status: Some(SessionStatus::AwaitingInput),
            last_ago_ms: 12 * MIN,
            created_ago_ms: 2 * HOUR,
            task: Some((TaskStatus::Waiting, TaskRole::Review, false)),
            ..chat(
                ORCH_REVIEW,
                Some("space-zeron"),
                MAC,
                "Review the merge receipts",
                "Should a restore write a receipt too?",
            )
        },
        DemoChat {
            parent: Some(ORCH_PARENT),
            status: Some(SessionStatus::Working),
            harness: Codex,
            model: "gpt-5.6-terra",
            reasoning: High,
            last_ago_ms: MIN,
            created_ago_ms: 2 * HOUR,
            task: Some((TaskStatus::Running, TaskRole::Design, false)),
            ..chat(
                ORCH_DESIGN,
                Some("space-zeron"),
                MAC,
                "Design the mobile report rows",
                "Folding reports like tool groups.",
            )
        },
    ]
}

pub(crate) fn spaces(now: i64) -> Vec<Space> {
    let space =
        |id: &str, device: &str, path: &str, name: Option<&str>, git: bool, ago: i64| Space {
            id: id.into(),
            device_id: device.into(),
            path: path.into(),
            name: name.map(str::to_owned),
            git_detected: git,
            git_checked_at: git.then(|| ms(now)),
            checkout_id: git.then(|| format!("co-{id}")),
            repository_id: None,
            created_at: ms(now - ago),
        };
    let zeron = |mut space: Space| {
        space.repository_id = Some("github.com/zeronsh/zeron".into());
        space
    };
    vec![
        space(
            "space-blog",
            STUDIO,
            "/Users/dev/Projects/blog",
            None,
            false,
            20 * DAY,
        ),
        zeron(space(
            "space-zeron",
            MAC,
            "/Users/dev/zeron",
            None,
            true,
            9 * DAY,
        )),
        // A second clone of the same repository on the VPS.
        zeron(space(
            "space-zeron-vps",
            VPS,
            "/srv/src/zeron",
            None,
            true,
            6 * DAY,
        )),
        space("space-edge", VPS, "/srv/deploys/edge", None, true, 4 * DAY),
        space(
            "space-mobile",
            MAC,
            "/Users/dev/zeron-ios",
            Some("Zeron iOS"),
            true,
            2 * DAY,
        ),
    ]
}

pub(crate) fn devices(self_id: &str, self_name: &str, now: i64) -> Vec<Device> {
    let device =
        |id: &str, name: &str, platform: &str, version: &str, caps: bool, seen: i64| Device {
            id: id.into(),
            name: name.into(),
            platform: platform.into(),
            last_seen_at: Some(ms(seen)),
            created_at: Some(ms(now - 30 * DAY)),
            version: Some(version.into()),
            cursor_sdk_version: None,
            capabilities: if caps {
                capability::ALL_QUEUE
                    .iter()
                    .map(|c| (*c).to_owned())
                    .collect()
            } else {
                Vec::new()
            },
        };
    vec![
        device(
            MAC,
            "MacBook Pro",
            "macos",
            env!("CARGO_PKG_VERSION"),
            true,
            now,
        ),
        device(
            VPS,
            "hetzner-01",
            "linux",
            env!("CARGO_PKG_VERSION"),
            true,
            now,
        ),
        device(
            STUDIO,
            "Mac Studio",
            "macos",
            "0.2.80",
            false,
            now - 3 * DAY,
        ),
        device(
            self_id,
            self_name,
            "ios",
            env!("CARGO_PKG_VERSION"),
            false,
            now,
        ),
    ]
}

/// Devices whose presence the demo keeps fresh.
pub(crate) const ONLINE: &[&str] = &[MAC, VPS];

pub(crate) struct Seeded {
    pub change_requests: Vec<CheckoutChangeRequestStatus>,
}

/// Write the whole dataset into the replica (as local writes, which the
/// local-only demo replica folds straight into its rows).
pub(crate) fn seed(
    doc: &mut RegistryDoc,
    fixture: DemoFixture,
    self_id: &str,
    self_name: &str,
) -> Result<Seeded, zeron_doc::DocError> {
    let now = crate::now_ms();
    let all_devices = devices(self_id, self_name, now);
    let mut change_requests = Vec::new();
    match fixture {
        DemoFixture::IosOnly => {
            if let Some(phone) = all_devices.iter().find(|d| d.id == self_id) {
                doc.upsert_device(phone)?;
            }
            doc.reconcile_sidebar_pins(true)?;
            return Ok(Seeded { change_requests });
        }
        DemoFixture::NoProjects => {
            for device in &all_devices {
                doc.upsert_device(device)?;
            }
            doc.reconcile_sidebar_pins(true)?;
            return Ok(Seeded { change_requests });
        }
        DemoFixture::Standard => {}
    }
    for device in &all_devices {
        doc.upsert_device(device)?;
    }
    let spaces = spaces(now);
    for space in &spaces {
        doc.upsert_space(space)?;
    }
    for demo in chats() {
        let space = demo.space.and_then(|id| spaces.iter().find(|s| s.id == id));
        let last = now - demo.last_ago_ms;
        let source_context = match (demo.branch, space) {
            (Some(branch), Some(space)) => Some(ConversationSourceContext {
                checkout_id: format!("co-{}", demo.id),
                repo_root: space.path.clone(),
                cwd: space.path.clone(),
                branch: branch.into(),
                head_sha: None,
                observed_at: ms(last),
            }),
            _ => None,
        };
        let chat = Chat {
            id: demo.id.into(),
            device_id: demo.device.into(),
            title: Some(demo.title.into()),
            archived: demo.archived,
            cwd: Some(space.map_or_else(|| "~".to_owned(), |s| s.path.clone())),
            branch: demo
                .branch
                .map(str::to_owned)
                .or_else(|| space.filter(|s| s.git_detected).map(|_| "main".to_owned())),
            checkout_id: source_context.as_ref().map(|s| s.checkout_id.clone()),
            source_context: source_context.clone(),
            snoozed_until: None,
            config: Some(ChatConfig {
                harness: demo.harness,
                model: Some(demo.model.into()),
                reasoning: Some(demo.reasoning),
                model_options: Default::default(),
                sandbox: SandboxLevel::WorkspaceWrite,
            }),
            last_message_preview: Some(demo.preview.into()),
            last_message_at: Some(ms(last)),
            created_at: ms(now - demo.created_ago_ms.max(demo.last_ago_ms + MIN)),
            harness_session_id: None,
            harness_session_cwd: None,
            space_id: demo.space.map(str::to_owned),
            last_seen_at: Some(ms(if demo.seen { last } else { last - MIN })),
            room_gen: Some(2),
            parent_chat_id: demo.parent.map(str::to_owned),
            orchestration: demo.task.map(|(status, role, delivered)| zeron_proto::ChatOrchestration {
                task: Some(zeron_proto::DelegatedTask {
                    id: demo.id.into(),
                    parent_chat_id: demo.parent.unwrap_or_default().into(),
                    role,
                    isolation: zeron_proto::TaskIsolation::Worktree,
                    base_ref: None,
                    status,
                    prompt: demo.title.into(),
                    request_fingerprint: demo.id.into(),
                    created_at: now - demo.created_ago_ms,
                    updated_at: now - demo.last_ago_ms,
                    summary: status.is_terminal().then(|| demo.preview.into()),
                    summary_truncated: false,
                    result_message_id: None,
                    delivered,
                    cancel_requested: false,
                }),
                fork: None,
            }),
        };
        doc.upsert_chat(&chat)?;
        if let Some(status) = demo.status {
            doc.upsert_session(&Session {
                last_completed_turn: None,
                running_subagents: 0,
                chat_id: demo.id.into(),
                device_id: demo.device.into(),
                status,
                started_at: Some(ms(now - 95_000)),
                updated_at: Utc::now(),
            })?;
        }
        if let (Some((number, state, title)), Some(source)) = (demo.pr, &source_context) {
            change_requests.push(CheckoutChangeRequestStatus {
                checkout_id: source.checkout_id.clone(),
                device_id: demo.device.into(),
                cwd: source.repo_root.clone(),
                branch: source.branch.clone(),
                change_request: Some(ChangeRequestSummary {
                    provider: "github".into(),
                    number,
                    title: title.into(),
                    url: format!("https://github.com/zeron-sh/zeron/pull/{number}"),
                    state,
                    base_ref: "main".into(),
                    head_ref: source.branch.clone(),
                }),
                updated_at: Utc::now(),
            });
        }
    }
    doc.reconcile_sidebar_pins(true)?;
    let mut after: Option<String> = None;
    for id in ["chat-veil", "chat-picker"] {
        doc.change_sidebar_pin(&SidebarPinChange::Pin {
            session_id: id.into(),
            after: after.clone(),
            before: None,
        })?;
        after = Some(id.into());
    }
    for (section, name, members) in [
        ("section-p0", "P0", &["chat-tabs", "chat-errored"][..]),
        (
            "section-mobile",
            "Mobile",
            &["chat-ios-scroll", "chat-ios-keyboard"][..],
        ),
    ] {
        doc.change_sidebar_pin(&SidebarPinChange::Section {
            change: SidebarSectionChange::Create {
                id: section.into(),
                name: name.into(),
            },
        })?;
        for member in members {
            doc.change_sidebar_pin(&SidebarPinChange::Section {
                change: SidebarSectionChange::Assign {
                    session_id: (*member).into(),
                    section_id: Some(section.into()),
                    after: None,
                    before: None,
                },
            })?;
        }
    }
    Ok(Seeded { change_requests })
}
