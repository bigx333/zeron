# Orchestration UI: delegated tasks, lineage, forks and merge-back

Design spec and implementation notes for the orchestration v2 features ported from T3 Code:
- managed delegated tasks, with durable result delivery to the parent
- role and shared/worktree isolation
- stopping a task
- task status
- explicit fork lineage and merge-back

It covers desktop (GPUI), iOS (UIKit) and Android (Compose). Status: implemented; review
screenshots are in `docs/screenshots/task-lanes/`.

## Principles

1. **No new dashboard.** Orchestration lives where people already look:
   - Desktop: the lanes card under the session card.
   - Mobile: a "Tasks and side chats" sheet opened from the session's navigation bar.
2. **Status is true.** Every word comes from the task record the engine writes
   (`Chat.orchestration.task`; on mobile, `SessionRow.task`). The only local state is a
   request in flight. It lasts until the next chats/workspace frame confirms it.
3. **One unprompted moment.** A task reaching its end washes its row once (the *settle*).
   All other motion answers an action: expanding, confirming, stopping, merging.
4. **Zeron's visual language, unchanged.**
   - Desktop: picker rows, `menu_heading`, `status_dot_color` tones, glyph spinners, harness
     marks, frosted cards, ink/hairline washes, `ui_rems`.
   - iOS: `Palette`, `StatusTone`, `StatusGlyph`, `BrandMarks`, Geist via `Fonts`.
   - No new colors and no new radii.
5. **Keyboard and assistive tech are first-class.**
   - Desktop: every row and action is focusable, activates with Enter/Space, and shows the
     accent focus ring. Esc backs out of a confirm.
   - Mobile: every element is labelled, expanded state is exposed, Dynamic Type is
     supported, and reduced motion is honoured.

## Vocabulary

| Concept | Copy |
|---|---|
| Section | Tasks · Side chats (the desktop `menu_heading` uppercases it, as all card headings do) |
| Queued / Running / Waiting / WaitingForChildren | Queued · Working · Needs input · Waiting for tasks (`TaskStatus::label`) |
| `cancel_requested && !terminal` | **Stopping…** |
| Terminal statuses | Completed · Failed · Cancelled · Interrupted |
| `delivered` | **Sent to this chat** (the parent's result command was *applied*; it may still wait for the parent's turn boundary, so we never claim it was read) |
| Terminal, not delivered, not stopped | Result ready, not sent yet |
| Terminal with `cancel_requested` | Stopped from this chat (delivery is deliberately suppressed) |
| Role | Implementation · Research · Review · Design · Test (General shows nothing) |
| Isolation | Shared checkout · Own worktree |
| Stop | `Stop task` → confirm: desktop "Stop it? [Keep running] [Stop]"; mobile "Stop this task?" + "This stops the task and the tasks it started. Forks and side chats keep running. A stopped task's result isn't sent here." |
| Stop failure | Couldn't stop the task. {engine reason} → `Try again` |
| Offline host | Its device is offline (Stop disabled) |
| Lineage | Delegated by {chat} · Forked from {chat} · "a chat that's no longer here" |
| Fork | Fork chat / Forking… |
| Merge | Merge into this chat → "Sends the fork's latest response here as context." → `Merge` / `Not now`; then "Merged" |
| Merge blocked | Finish the fork's turn first |

Empty-state and outcome notes:
- Waiting to start.
- It's waiting on an answer. Open it to reply.
- Waiting for its own tasks to finish.
- Stopping its current turn.
- It stopped without a result.
- It was interrupted before finishing.
- Cancelled before it reported a result.
- Finished without a summary.

Mobile empty sheet: "Nothing delegated yet. Tasks an agent hands off from this chat show up
here. Fork it to try another direction."

## Model rules (shared by all platforms)

**Mark** (`mark_for` / `TaskMark`):

| Condition | Mark |
|---|---|
| Stop requested and not terminal | Stopping |
| Completed, and delivered or stopped | Sent (quiet check) |
| Completed otherwise | Ready (success check) |
| Any other status | Maps one-to-one |

**Lane ordering.**
- *Active*: not settled. Oldest `created_at` first, so the longest-running task leads and new
  ones join at the foot.
- *Finished*: settled, meaning `terminal && (delivered || cancel_requested)`. Newest
  `updated_at` first, folded under "N finished".
- A stopped task never lingers as "owed".

**Nesting.** Managed tasks can delegate up to 4 levels deep. Ordinary side chats stay one
level.
- Every chat's card or sheet lists only its *direct* children.
- A task row shows "N tasks" when it delegated in turn.
- The lineage row ("Delegated by …") leads up one level at every depth. A task's parent may
  be another task.

**Side chats** are children without a task. Tasks are removed from that lane so nothing is
listed twice. A child with `orchestration.fork` shows a fork glyph and can merge back.

## Desktop: the lanes card

Files:
- `crates/ui/src/shell/session_lanes.rs`
- `crates/ui/src/shell/session_lanes/tasks.rs` (new)
- Fixture: `crates/ui/examples/task-lanes-fixture.rs` (new), with an `[[example]]` entry in
  `crates/ui/Cargo.toml`.

```
┌ Lanes card (272px, frosted) ──────────────────────────────┐
│ ← Delegated by  Migrate T3 orchestration                  │  only for tasks/forks
│───────────────────────────────────────────────────────────│
│ TASKS                                                     │
│ ⠿ ◆ Audit the T3 handoff budget                     1m    │  hover: time ↔ open arrow
│      Working  for 14m                                     │  status in tone, time faint
│      [Research] [Own worktree] [gpt-5.3-codex] [1 task]   │  passive facts
│      summary, 4 lines, ellipsis                           │
│      ✓ Sent to this chat                                  │  delivery line (rolls)
│      [Open task] ⤢                          [Stop task]   │  ⤢ = open in main view
│ ● ✳ Review the merge RPC contract                   2m    │
│ › 3 finished                                              │
│───────────────────────────────────────────────────────────│
│ SIDE CHATS                                    [+] [fork]  │
│ ✓ ✳ ⑂ Try the portable fork                ↩      25m     │  ↩ merge tile on hover/focus
│      Sends the fork's latest response here as context.    │  merge strip (disclosure)
│      [Merge] [Not now]                                    │
│ SUBAGENTS (unchanged)                                     │
└───────────────────────────────────────────────────────────┘
```

**Marks** (16px slot):

| State | Mark |
|---|---|
| Queued | hollow ring |
| Working | glyph spinner |
| Waiting for tasks | muted mono spinner |
| Stopping | warning mono spinner |
| Needs input | accent dot |
| Ready | success check |
| Sent | muted check |
| Failed | danger dot |
| Cancelled | faint ✕ |
| Interrupted | warning dot |

**Stop flow.**
1. The confirm turns the action row into "Stop it? [Keep running] [Stop]". The Keep-running
   button reuses the Stop button's element id, so keyboard focus lands on the safe answer.
2. In flight: "Stopping…" with a spinner.
3. Success: the row keeps reading Stopping until a frame carries `cancel_requested`.
4. Failure: an inline danger line plus `Try again`.

`TaskCancel {parentChatId, taskId, targetDeviceId: child.device_id}`.

**Merge.** `MergeChat {sourceChatId, targetDeviceId: fork.device_id}`; the target defaults to
the parent.
- "Merged" comes from `ChatTransfers {chatId, targetDeviceId}` (kind `merge`, source = the
  fork). It's fetched once per visit and refreshed after a merge. Until that arrives, a
  local flag stands in.
- The tile is hidden across devices, since the engine refuses those merges.
- The tile is disabled while the fork is Working or AwaitingInput.

**Navigation.**
- Open task / the arrow: right-pane tab (`open_child_chat_tab`).
- ⤢ Open in main view: `open_chat`. This is how a nested task's own card is reached.
- The lineage row: `open_chat(parent)`.

## History: checkpoints and restore

The engine saves a checkpoint after each finished root turn. A checkpoint holds:
- the conversation point
- provider references: `native` resumes or forks the provider session at that turn;
  `portable` continues from a copy
- when captured, a reversible snapshot of the checkout

History lives in the same surfaces as everything else. On desktop it's the last section of
the lanes card; on mobile, the last section of the tasks sheet.

**Wire.**
- Desktop RPCs, each sent with `targetDeviceId` set to the chat's device:
  - `ChatCheckpoints {chatId}` returns `[RunCheckpoint]`, newest first.
  - `CreateChatCheckpoint {chatId}` returns a `RunCheckpoint`.
  - `RollbackChat {chatId, checkpointId, clientRequestId}` returns
    `{recoveryCheckpointId}`.
- Mobile calls: `chatCheckpoints`, `createChatCheckpoint`,
  `rollbackChat(chatId:checkpointId:clientRequestId:)`.
- Fields:
  - `id`, `messageId?`, `title`, `createdAtMs`
  - `recovery`
  - `providerStrategy` (`native` | `portable`)
  - `filesAvailable`, `filesChanged?`
  - `restorable`, `blockedReason?`
- An engine without these calls (`UnknownMethod`, or `Unsupported`/`NotImplemented` on
  mobile) hides the section.

```
│ HISTORY                                            [+]    │  + = Save now
│ ✓ Restored to "Port lineage icons".          [Undo]       │  receipt, eases in once
│ ◷ Wire the History section                          3m    │  hover/focus: time ↔ restore tile
│ ◷ Port lineage icons                                18m   │
│      Saved 18m ago                                        │
│      [12 files differ] [Native resume]                    │
│      Resumes the provider session at this point.          │
│      The conversation and the checkout go back to just    │  confirm, in place
│      after this turn. Where you are now is saved as a     │
│      recovery point.          [Keep current] [Restore]    │
│ ◷ Draft the cancellation copy                       41m   │
│      Stop tasks working in this checkout first.           │  engine's blockedReason
│ › 1 recovery point                                        │  folded, inspectable
```

**Copy.**
- Section and saving:
  - History · Save now · Saving…
  - Empty: "A checkpoint is saved after each finished turn."
- Facts: N files differ · 1 file differs · Checkout unchanged since · Checkout saved ·
  Conversation only · Native resume · Portable
- Strategy lines:
  - native: "Resumes the provider session at this point."
  - portable: "Continues from a copy of the conversation."
- Confirm blurb, one per combination of recovery point and files saved:
  - Normal point, files saved: "The conversation and the checkout go back to just after
    this turn. Where you are now is saved as a recovery point."
  - Normal point, no files: "The conversation goes back to just after this turn; no files
    were saved here, so the checkout stays as it is. Where you are now is saved as a
    recovery point."
  - Recovery point: "…return to where they were before the restore. …"
- Buttons: `Keep current` / `Restore`.
- Blocked reasons, in priority order:
  1. Its device is offline.
  2. Finish or stop the current turn first. (live status, Working or AwaitingInput)
  3. The engine's `blockedReason`, for example "Stop tasks working in this checkout first."
- In flight: Restoring…
- Failure: Couldn't restore. {reason} → `Try again`. The same `clientRequestId` is reused
  until the restore succeeds, so a retry dedupes.
- Receipt: "Restored to "{title}"." [Undo] → Undoing… → "Undone. The restore is kept as a
  recovery point."
- Undo restores the returned recovery checkpoint without asking again, because that
  restore itself keeps a recovery point.
- Recovery points: folded under "N recovery points". The row reads "Kept 2m ago, before a
  restore", and its button reads "Restore this point".

**Refresh.** The list is read once per visit, again after every finished turn (keyed by the
chat's last message time), and after a save or a restore. A checkpoint that arrives while
the card is up fades in once: `FADE_IN` 500ms with a 4px rise, skipped under reduced motion.

**Motion.**
- Rows and confirms use the task lane's measured `COLLAPSE` disclosure (the shared
  `lane_disclosure`).
- The recovery fold's chevron rotates via `state_t`.
- The receipt enters with `MENU_IN`.
- Mobile:
  - iOS: the same spring or crossfade as the task rows, a success haptic and a VoiceOver
    announcement on restore, and a `.alert` confirm with `preferredAction = Restore`.
  - Android: `animateContentSize` with an AnimatedVisibility receipt, both snapping under
    reduced motion, plus an `AlertDialog` confirm.

Files:
- Desktop: `crates/ui/src/shell/session_lanes/history.rs` (new), hooked from
  `session_lanes.rs`.
- iOS: `apps/ios/Zeron/Session/CheckpointHistory.swift` (new), stacked by
  `TaskLanesController`.
- Android: `apps/android/.../ui/CheckpointHistory.kt` (new), one `item {}` at the end of
  `OrchestrationSheet.kt`.

## Mobile

### iOS (UIKit)

Files:
- `apps/ios/Zeron/Session/TaskLanesController.swift` (new)
- `SessionViewController.swift`: a bar button and menu item, "Tasks and Side Chats", plus
  `openRelated`.
- `AppModel.swift`: `observeWorkspace`. Child rows never reach the front page, so their
  changes need a per-frame observer.

Behaviour:
- **Entry point.** A `square.stack.3d.up` bar button is always present, so the first fork is
  discoverable and the way up is always there. It's mirrored in the "…" menu.
- **Presentation.** A page sheet with medium and large detents and a grabber.
- **Row layout.**
  - 48pt rows: `StatusGlyph` mark, brand mark, fading title, time.
  - Tapping a task row expands its detail: status, facts, a 4-line summary, the note, the
    delivery line, the error, `Open task` and `Stop task`.
  - Side chats open on tap. Forks carry a 44pt merge button.
  - `Fork chat` is always last.
- **Stop.**
  - Action sheet: "Stop" (destructive) / "Keep running".
  - In flight, the button shows `showsActivityIndicator` and "Stopping…".
  - Errors are inline, announced to VoiceOver, and offer "Try again".
  - Offline: disabled, with the subtitle "Its device is offline".
- **Navigation.**
  - Opening a child pushes its `SessionViewController`. Back pops one level, so it means
    "up" at every depth.
  - The lineage row pops to the parent if it's already in the stack, otherwise pushes it.
  - Fork → `forkSession` → navigates into the new fork.
- **Motion.**
  - Disclosure: a 0.42s spring at damping 0.88. With reduced motion it becomes a 0.18s
    ease-out crossfade.
  - Mark swap: a 0.28s cross-dissolve.
  - Settle: a 1.1s wash, skipped under reduced motion. Completion also plays a success
    haptic, which is kept under reduced motion.
- **Accessibility.**
  - The header is a single button element: "{title}, {status}, {role}", value
    Expanded/Collapsed, hint "Double-tap to show details".
  - The side-chat row is a container with an open element plus a separate Merge button.
  - The fold reports Expanded/Collapsed.
  - Dynamic Type: `UIFontMetrics.scaledFont` with `adjustsFontForContentSizeCategory`, and
    attributed lines rebuild on content-size changes (`registerForTraitChanges`).

### Android (Compose)

`apps/android/app/src/main/java/sh/zeron/android/ui/OrchestrationSheet.kt`, integrated
through `SessionScreen.kt` and both navigation layouts in `ZeronRoot.kt`, implements:
- task/side-chat sheet
- expandable summaries
- finished folding, including cancellation suppression
- Stop confirmation, error, retry and offline states
- fork and merge
- nested parent navigation
- reduced-motion-aware settle and disclosure

Disclosure uses a 220ms content-size transition and 140/100ms fades. A terminal
transition washes the row once; an Android animation-scale observer disables
these effects when system animation is off. Touch actions are at least 48dp,
expandable rows expose their state, and errors use a polite accessibility live
region. The sheet updates from workspace revisions, including hidden children.

### Shared FFI

Shared by the native apps:
- `SessionRow.task: DelegatedTaskView?` with fields `id`, `parentChatId`, `status`,
  `statusLabel`, `terminal`, `stopping`, `cancelRequested`, `roleLabel`, `isolationLabel`,
  `summary`, `delivered`, `createdAtMs`, `updatedAtMs`.
- `SessionRow.fork: ForkLineageView?`
- `CoreClient.cancelTask(parentChatId:taskId:)`, `mergeChat(sourceChatId:) -> transferId`,
  `forkSession(sourceChatId:) -> chatId`.

## Motion (desktop catalog, reduced motion snaps)

| Moment | Spec | Mechanics |
|---|---|---|
| Expand/collapse a task or merge strip | `COLLAPSE` 180ms ease-out | `motion::state_t` per key. Height = measured × p; opacity 0.35→1; 3px lift. Height is measured by a paint-time canvas into `LaneState` (92px estimate on first frame). Natural height at rest. Reversible mid-flight; mount never replays. |
| Plate behind an open row | same p | `ink(0.035·p)` at `MENU_ITEM_RADIUS` |
| Status mark change | `GLYPH_STATE` 280ms | Old mark fades and lifts 3px, new mark rises 3px. Keyed by the change epoch. |
| Settle | 1100ms `EASE_OUT_SOFT` | success 0.12 / danger 0.10 / ink 0.06 wash → clear. Once per first arrival at an end. Ready→Sent doesn't count. Never on first sight. |
| Status / delivery words | `roll_text` | Keyed per task |
| Finished-fold chevron | `COLLAPSE` | rotates via `state_t` |
| Hover washes | `HOVER_FADE` | `picker_row`'s existing fade |

Frames come from the shell's existing render tail (`state_morphs_active`) and from
`with_animation`, which is mounted only while a transition is live. Idle cards schedule
nothing.

## Architectural constraints

1. **Status truth.** Optimistic state is limited to "Stopping… (in flight / asked for)" and
   "Merged (just now)". The next frame reconciles both.
2. **Routing.** `TaskCancel`, `MergeChat` and `ChatTransfers` carry `targetDeviceId`, which
   the engine strips. A task runs on its parent's host. Merge works on the same device only.
3. **No `shell.rs` edits.** All new view state lives in `LaneState.tasks` (`TaskLaneState`).
   `session_lanes/tasks.rs` is a child module declared by `session_lanes.rs`.
4. **Subagents and tasks stay separate lanes.** Provider-native spawn chips (`SubagentStatus`)
   are a different thing from app-owned tasks (child chats with `DelegatedTask`). This
   matches T3's `provider_native` / `app_owned` split.
5. **Deliberate differences from T3.**
   - T3 wakes the parent agent with "use task_status". Zeron shows the human the `delivered`
     state.
   - T3's "Lineage" panel lives in a details sidebar; here the lanes card plays that role.
   - The sidebar keeps children out of the top level, as T3 does.

## Follow-ups (outside this change)

- `transcript.rs::fork_marker` ignores its `source_chat_id`. Make "Forked from {title}" a link,
  and add a "Task from {parent}" seam. This was deferred because the file has unrelated
  uncommitted edits.
- The desktop lanes card only shows for the main chat. A task opened as a right-pane tab
  reaches its own tasks through ⤢ Open in main view.
- iOS has no unit-test target for `TaskMark`/`TaskLane`. The rules mirror the desktop's
  tested `tasks.rs`.

## Validation

**Desktop**
- `cargo check -p zeron-ui --tests` compiles.
- `cargo test -p zeron-ui --lib session_lanes`: 14 passed. These cover task
  ordering/folding, stop-suppression, delivery copy, settle-once, per-fork merge, the
  checkpoint wire shape (including older hosts), blocked-restore reasons, and the confirm
  copy.
- `cargo test -p zeron-ui --lib shell::`: 335 passed.
- The full `cargo test -p zeron-ui --lib` run aborts with SIGABRT inside the unmodified
  `attachments::` tests. That module also aborts when run on its own; every other module
  passes.

**Desktop visual QA**

`cargo run -p zeron-ui --features appshots-fixture --example task-lanes-fixture -- <dir>`
captures seven scenes × (dark frosted, light opaque):
- rest
- inspect (working / needs input / stopping)
- results (stop error, result not sent, finished fold)
- confirm + merge strip
- nested (a task's own card with "Delegated by")
- history-confirm (an inspected checkpoint with its in-place restore confirm, plus a
  blocked one)
- history-receipt (Restored … Undo, a failed restore with Try again, the recovery fold
  open)

**iOS**

Full Xcode isn't installed (CommandLineTools only, no iOS SDK). Instead:
- `swiftc -parse` passes on the three edited files.
- The whole app was type-checked against the Mac Catalyst UIKit SDK with freshly generated
  bindings:
  `swiftc -typecheck -target arm64-apple-ios26.0-macabi -sdk $(xcrun --show-sdk-path) -Fsystem $SDK/System/iOSSupport/System/Library/Frameworks -I <zeron_coreFFI modulemap> -swift-version 5`.
- That check has zero errors or warnings in the edited files. The only errors come from
  excluded voice files that need WebRTC or ActivityKit.
- The checked-in `zeron_core.swift` matches regenerated declarations and checksums
  (`diff -w` is clean); trailing whitespace was normalized on changed lines.
- History was re-checked the same way against the regenerated bindings
  (`RunCheckpointView`, `chatCheckpoints`, `createChatCheckpoint`, `rollbackChat`). There
  are zero diagnostics in `TaskLanesController.swift`, `CheckpointHistory.swift`,
  `SessionViewController.swift` and `AppModel.swift`.
- Not done: no device or simulator run, and no iOS screenshots.

**Android**: `:app:assembleDebug --offline --no-configuration-cache` passes with
freshly generated Kotlin bindings and newly compiled ARM64 and x86_64 Rust/JNI
libraries. The debug APK is `apps/android/app/build/outputs/apk/debug/app-debug.apk`.
The build used the rustup toolchain that owns the installed Android targets;
Homebrew Rust on this machine does not include those target libraries. Host
Rust for `zeron-mobile` also builds/checks. No device/emulator run was performed.

The History UI also compiles: `:app:compileDebugKotlin -PzeronSkipCore --offline
--no-configuration-cache` passes against the regenerated Kotlin bindings. No Android
screenshots were taken: no AVD is configured, and the demo backend has no tasks or
checkpoints, so the sheet would render without the new sections.
