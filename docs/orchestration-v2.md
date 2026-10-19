# Orchestration v2 parity

Source audit: [T3 `9077e818`](https://github.com/pingdotgg/t3code/tree/9077e818d836ec6e9cc373d7469034cccc593a5a),
2026-10-19, especially `docs/orchestration-v2`, `orchestratorMcp.ts`, and the
orchestrator toolkit. This ports product behavior into Zeron's existing
registry, document command queue, harness events and native clients.

| Behavior | Zeron implementation |
| --- | --- |
| App-owned chat, turn and tool identities | Existing session documents and normalized harness events. Provider session handles remain separate. |
| Standalone and batch sessions, cross-provider messaging, waits, interrupt | Existing `create_chat(s)`, `send_message(s)`, `wait_for_turn`, `wait_for_any`, `interrupt_chat`. |
| Questions and approvals | Existing `list_pending_inputs`, `get_chat`, `respond_to_input`; task waits return when a child needs input. |
| Managed delegated work | `delegate_task`, `task_status`, `task_cancel`, `list_tasks`; role, inherited provider/model/options/permissions, shared or isolated checkout. |
| Async completion | Child host publishes an immutable result and queues an attributed parent notification with a deterministic command/message id. Parent can be active or idle. |
| Completion barriers | Child tool/subagent completion never completes a root turn. Managed tasks wait for root completion and their managed descendants and result-delivery turns. |
| Idempotent task creation | Parent-scoped `client_request_id`, deterministic task id, fingerprint conflict detection, deterministic launch/result commands. |
| Cancellation | Stop child and managed descendants, pause queued work, preserve transcripts and worktrees; suppress completion delivery. |
| Context forks | Existing UI fork plus MCP `fork_chat`, selected completed message boundary, explicit source lineage, isolated provider session. |
| Provider switching | A different provider never resumes another provider's native session. Returning to a provider reactivates its saved cwd-scoped session and transfers only its missing visible conversation. Fresh providers receive bounded portable history. |
| Native forks | First dispatch resolves an exact Codex completed-turn boundary into an independent native thread; the adapter verifies the returned thread's final turn. Other providers use portable context. |
| Checkpoint restore | Completed root turns automatically save checkpoints; manual saves, restore, retry receipts and Undo are available through RPC, MCP and all native UIs. Restore preserves a recovery point before changing conversation or working files. |
| Merge-back | `merge_chat` delivers an explicit result to the parent or another chat on the same host, with a retry-safe receipt and inspectable artifact. This does not merge Git branches. |
| Capability discovery | `orchestrator_capabilities` exposes live models, settings, constraints and feature flags. Unsupported native operations are explicit. |
| Context inspection | `chat_transfers` reads the latest 32 context artifacts, including source boundary, strategy, transferred text, truncation and consumption. |

## Model guidance

T3 combines MCP tool schemas with shared provider instructions in
[`orchestrationInstructions.ts`](https://github.com/pingdotgg/t3code/blob/9077e818d836ec6e9cc373d7469034cccc593a5a/packages/provider-core/src/server/orchestrationInstructions.ts).
Zeron similarly shares [one instruction source](../crates/proto/src/orchestration_instructions.md)
between MCP initialization and the engine. The engine includes it on the first
normal prompt of each runtime, including a resumed runtime, for every harness.
It is app context rather than stored user text. Idle and native-command starts
leave guidance pending for the first normal warm send; subsequent warm sends
do not repeat it. Hosts without an injected Zeron MCP server do not advertise
unavailable tools. User delegation restrictions remain authoritative.

Guidance distinguishes managed tasks from ordinary conversations, live model
discovery, shared/worktree isolation, async completion, bounded waits, pending
questions, retry IDs, and checkpoint recovery. Each delegated review round uses
a new task with the original brief and previous findings, rather than messaging
the backing child chat and treating its frozen result as a new review result.

## Task lifecycle and ownership

Managed tasks are child chats with `Chat.orchestration.task`. The task id is the
child chat id. Only the child host advances its state and publishes its result.
Task status/cancellation require the originating parent id; MCP supplies that
id from its injected origin. Task ids are not permission-approval credentials.
Children may delegate up to four levels; ordinary side-chat creation retains its
existing single-level policy.

The full brief is staged and persisted in the child document before its registry
row is published. A restart between row creation and queue insertion can recover
from that staging record. After launch is durably queued the staging copy clears,
while its fingerprint remains as a receipt. Large briefs never enter registry
operations, which have a 16 KiB wire limit. Launch and parent notification use
the existing processed-command ledger. Repeated retries cannot
start another child or append another result message. A changed brief under the
same request key is rejected. A wait timeout never stops the child. A completed
task result remains stable across later sends to its backing chat.

Registry task summaries are previews capped at 4,096 JSON-encoded bytes, including
escaping, with an explicit truncation flag. Single-task MCP reads retrieve the
frozen result from the child document when that preview is truncated. Parent
notifications and full task reads include up to 24,000 characters. `list_tasks`
keeps compact previews; the full transcript remains available in the child chat.
Document writes use the existing serialized persistence path, preserving the
incremental-sync cursor. Registry and document durability barriers precede launch
and result delivery.
`TaskResult` freezes full published results independently of the active transcript,
so restoring the backing chat to an earlier checkpoint does not change its result.

Permissions inherit and explicit escalation is rejected. Cross-provider switches
drop incompatible model and reasoning defaults. Worktree isolation uses the
host's existing worktree/setup pipeline, based on HEAD of the parent's actual
checkout; dirty files remain in the parent checkout.

## Portable context policy

Only visible user/assistant text is transferred. Reasoning, raw tool inputs and
outputs, approval state and provider-native session state are not
copied into portable artifacts. Local attachments remain owned by their host.
The recent conversation is bounded to 48,000 characters, on Unicode boundaries,
and records truncation. Native slash commands retain their leading command and
are never wrapped with history. The artifact becomes consumed when a provider
confirms its session. Fork transcripts still retain their existing frozen
history and source seam.

## Provider continuity and checkpoints

Provider sessions retain their native id, cwd, exact turn reference when available,
and the last app message they consumed. Switching back resumes that provider's
own session after an engine restart and supplies a bounded visible-text delta.
Native slash commands do not consume a pending history bootstrap. Warm sends
never replay an in-flight turn's history; the existing fork-delivery receipt
still governs copied history that a side chat has not received yet.
If a saved Codex rollout has disappeared, its fresh-session fallback receives
the full bounded visible conversation rather than the delta intended for resume.
Manual checkpoints after an interrupted or unfinished native turn use portable
history; an in-progress native turn is not presented as a completed boundary.

Codex forks use `thread/fork` with the inclusive `lastTurnId`, then verify the new
thread's latest turn with `thread/turns/list`. An old CLI that ignores the boundary
cannot silently fork later history. Native resolution happens on first dispatch,
uses no model turn, and never mutates the source. Native checkpoint restoration
uses the same exact-boundary fork, preserving the original provider future for
Undo, including paginated histories. Providers without this control operation
restore through a fresh session and the saved visible conversation.

`ChatCheckpoints`, `CreateChatCheckpoint` and `RollbackChat` route to the owning
host. The separate `orchestration-history-v2` capability gates discovery for
older hosts. `RollbackChat` accepts `clientRequestId` and returns
`recoveryCheckpointId`; retrying a completed restore has no additional effects.
Restoring a recovery point saves another recovery point, so Undo is reversible.
The host refuses active turns, pending or queued sends, shared-checkout writers,
and a checkpoint belonging to a different checkout.
Managed descendants must have settled and delivered their results before a
parent can restore. Each restore also advances the conversation revision used
by default merge receipts, so a fork's result can be merged into the restored
conversation again without weakening retry deduplication.

Completed root turns create app-owned checkpoints. A durable baseline is saved
before the first dispatch, so even the first turn can be undone while preserving
pre-existing dirty files. Transcript objects are
content-addressed on the host; checkpoints reuse them. Working files are saved
in Zeron's private Git object database with durable refs, outside the user's
object database. Files, executable modes and symlinks are preserved; capture
disables content filters and line-ending conversion. Restore does not move Git
HEAD, alter the index or overwrite ignored files. An ignored obstruction is an
error. Recovery is durably saved before any working file is replaced, and a
failed restore attempts to put the previous working tree back.
Restoring older agent settings retains the chat's current permissions. Private
Git objects and refs are fsynced before a restore can replace working files.

Complete filesystem snapshots are bounded to 256 MiB and 50,000 files. Manual
saves refuse unsupported or oversized trees; an automatic checkpoint can retain
conversation only and exposes `filesAvailable: false`. Submodules and special
files require an explicit future snapshot strategy. Checkpoint data remains on
the owning host, so a viewer requests it through host RPC.

Zeron preserves its document architecture instead of importing T3's Effect/SQL
execution graph. Provider support is reported individually rather than claiming
native controls for every CLI. Scheduled automation and PR watches remain
separate domains.

## Desktop and mobile

The [UI design and implementation notes](orchestration-ui.md) cover GPUI,
Android Compose and iOS UIKit. All three expose task status, summaries, nested
navigation, stop confirmation with retry/offline states, fork lineage and
merge-back. Motion responds to disclosure and state changes, with reduced-motion
support. Task metadata participates in the shared workspace revision, so hidden
child-chat updates reach mobile sheets without adding them to the front page.
Desktop review captures are in [the screenshot gallery](screenshots/task-lanes/README.md).

## Validation

`crates/engine/tests/orchestration.rs` drives the real RPC, registry, command
executor and harness boundary. It covers concurrent retry dedupe, inheritance,
permission escalation rejection, task ownership, root completion delivery,
cancellation, selected fork points, merge receipts, cross-provider context,
nested completion barriers, durable restart deduplication, real worktree
isolation, empty/interrupted turns and blocking question responses. Root terminal
outcomes are persisted independently of whether the provider emitted visible text.
Existing side-chat and MCP tests cover continuation, blocking input, wait races
and transport behavior.

The initial migration's verification run passed 589 engine unit tests (5 ignored), 11
orchestration integration tests and 41 MCP tests. The integration suite also
checks 100 KB briefs and escaped Unicode results against the registry wire limit.
Client projection tests verify that child updates change workspace revisions
without polluting top-level sessions. Desktop lane tests and real GPUI fixture
captures cover ordering, cancellation, delivery wording and motion. The complete
desktop suite encounters a SIGABRT in the unchanged attachments tests;
the lane and shell suites pass independently.

Android's debug APK builds with regenerated bindings and fresh ARM64/x86_64
Rust libraries. iOS source parsing
passes; a Mac Catalyst UIKit type-check reports no diagnostics in the new UI,
but unrelated voice dependencies prevent a clean whole-app check. This machine
has command-line tools rather than full Xcode, so no iOS simulator/device run was
performed. Android device/runtime validation is also still outstanding.

`crates/engine/tests/history.rs` additionally covers lazy native forks at an older
selected turn, provider reactivation with only missing context after restart,
checkpoint restore and Undo across restart, idempotent retry, unchanged Git index
and HEAD, ignored-file preservation, ignored obstructions and active writers in
a shared checkout. `crates/harness/tests/history.rs` exercises the real Codex
JSON-RPC adapter against a control-only fixture, including a paginated native
fork and rejection of a missing source boundary without starting a process.

The reviewed migration passes 11 history integration tests, all 12
orchestration and 7 side-chat regressions, 15 session unit tests, 39 Codex adapter
tests (4 ignored), and 3 native-history adapter tests. Document (153), protocol
(85), client (26) and MCP (42) unit suites also pass. History checks include first-turn
baseline recovery, interrupted retry recovery with intervening edits, executable
modes, symlinks, file/directory transitions, frozen task results and merge receipt
revisions. Desktop lane coverage is now 15 tests; the four new History captures
were visually reviewed in dark and light. `cargo check --workspace` passes.

The latest broader engine run from a clean review checkout reports 590 passed,
1 failed and 5 ignored: an unchanged workspace-search test fails in the full
parallel run but passes alone. An earlier run also encountered a loopback
download failure in the unchanged installer fixture; it passes in the clean
review checkout. The full desktop
suite's attachment-test abort also remains. These runs are not reported as clean.
Android's final debug APK builds with both native ABIs. iOS parsing and the
limited UIKit check pass for the edited sources; full Xcode and device/emulator
runtime verification are still unavailable.

The review loop additionally checks native resume fallback context, exclusion of
queued future messages from bootstraps, completed native boundaries across
continuation entries, bounded cancellation previews with full stored reasons,
async UI callbacks after navigation, and nested/archived task availability.
Mobile merge history now reads durable host receipts; recorded merges remain
history rather than preventing a newer result from merging. The checked-in Swift
binding matches the generator byte for byte, as required by CI.

History eligibility refreshes when this chat or its managed descendants change.
Other chats sharing the checkout are always checked by the engine at restore;
their status alone does not refresh an open History list. Closing a mobile sheet
during a restore can lose its immediate Undo receipt, while the durable recovery
point remains available in History.
