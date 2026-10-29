# Shared message queue

Ported from `jg-personal-cut` at `fd3f97b`. The queue spans the session document,
host executor, RPC, desktop composer and iOS client.

## Storage and ownership

`SessionDoc.queue` is a Loro movable list, separate from the immutable command
ledger and host-authored transcript. Each row holds a stable ID, editable text,
committed attachment paths, author/timestamps, an optional hold-until-turn-end
policy and an optional delivery gate. Real move operations preserve row identity
and converge across devices without delete-and-insert duplicates.

Clients can enqueue and reorder through their local document. Only the chat host
may deliver, remove or grant an edit lease. Desktop routes these actions with
`targetDeviceId`; iOS uses the host's relay. `WatchQueue` sends whole-list snapshots.
Failed or unacknowledged mutations trigger a fresh authoritative projection.

## Delivery

The host drains after document changes and session status changes. A per-chat
mutex serializes drains, explicit delivery, removal and protected edits.

| State | Automatic delivery |
| --- | --- |
| Idle | Start the head as the next turn. |
| Working or awaiting input | Keep the head queued until the turn ends. |
| Head has attachments | Wait for a new turn that can accept files. |
| Head has an editing or review gate | Block delivery; do not skip the head. |
| Queue paused after Cancel or snapshot recovery | Wait for a successful explicit prompt or queue delivery. |

Explicit steering never interrupts. Send-now interrupts the active turn before
dispatching the selected row. The desktop labels both delivery paths **Steer**
and explains their semantics in the tooltip; iOS distinguishes **Steer** and
**Send now**. Provider metadata must explicitly resolve the available action.

Dispatch retains the queue ID as the user-message ID. Failed dispatch restores
the row. An acknowledged removal cannot subsequently be consumed by this host.
Shutdown freezes open queues before stopping runs, and handles opened with
recovered queued work start paused.

## Protected editing

`BeginQueuedMessageEdit` acquires a host-issued generation under the drain mutex
and persists its delivery gate before acknowledging. Leases last 60 seconds and
clients renew every 20 seconds. Expiry changes the gate to `ReviewRequired`; it
never silently releases the row. Timers check both generation and deadline, and
snapshot recovery rearms all outstanding leases.

Finish supports commit, cancel, discard and release-unchanged. Commit checks the
lease generation and original text hash. Text, optional replacement attachments
and gate removal commit atomically. Stale/conflicting finishes preserve the row
for review. Desktop edits restore the previous composer draft after finishing;
the edited row retains its place in the queue.

## Composer and compatibility

Desktop defaults to **Queue** mode, holding messages until the turn ends.
Settings → General → **Steer mode** switches busy-chat submissions to explicit
Steer commands, delivered into the active turn without an additional action or
confirmation. The preference persists across restarts and applies to the send
button, Enter and Cmd/Ctrl+Enter. Agents that cannot steer hold the message for
the next turn without interrupting. Idle chats still start a normal turn and
queue edits still save to their existing row. iOS exposes the same toggle in
Settings → Messages and applies it to its composer's primary send action.
Android shows the same toggle in Settings → Messages; its delivery chip starts
on Steer when the host can steer.
The choice is an account-wide `preferences/composer.queueSendMode` registry
field: it persists offline and propagates between mobile and desktop on sync.
Desktop watches the engine's `WatchQueueSendMode` stream; mobile observes its
workspace projection. Older engines retain the desktop's local setting.
iOS also offers explicit steering through the send button menu.
Cmd/Ctrl+Enter on desktop submits content, saves an active edit, or activates
the most recently added queue row when the composer is empty. Command+Return on
iOS activates the queue head. A blocked shortcut target is never skipped and an
empty modified submit never stops the agent.

Queued submissions do not create optimistic transcript bubbles. Desktop retains
the active reply's scroll runway and anchors a locally queued message only when
its stable ID appears in the transcript. Attachment paths remain separate from
editable text and are expanded at dispatch; legacy expanded rows are recognized
to avoid duplicate trailers. Queue attachments use committed host uploads.

Support is negotiated through `EngineInfo` and device-registry capabilities:
`message-queue-v1`, `message-queue-actions-v1`, `message-queue-attachments-v1`,
`message-queue-clean-attachment-text-v1` and `message-queue-edit-lease-v1`.
Missing capabilities default to unsupported, regardless of matching version
numbers. Older hosts retain the existing command-send path.

## Validation

- Document tests cover round trips, concurrent moves/edits and delivery gates.
- Engine queue tests cover drain serialization, awaiting-input behavior,
  steering, attachments, Cancel/restart recovery, protected edits and removal.
- Device-routing tests race local and remote consumers over the same row.
- Desktop tests cover action selection, modifier submission, optimistic echo,
  settings compatibility and transcript anchoring.
- `apps/ios/ZeronTests/MessageQueueTests.swift` covers projection, policy,
  attachment text, action acknowledgements and keyboard submission. Run with
  the iOS Xcode test suite on macOS.

Failed automatic dispatch pauses the queue before restoring its head, so the restore cannot trigger another delivery attempt. Fix the host configuration or connection, then explicitly send a queued row or a new prompt to resume. New queued turns use the current chat configuration.
