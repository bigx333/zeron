# Orchestration activity review captures

Native captures of orchestration in the transcript and the sidebar (see
`docs/orchestration-ui.md`, "Transcript activity, chat pills and the sidebar
tree").

Desktop scenes come in dark (frosted) and light (opaque) pairs:

| Scene | What it shows |
|---|---|
| `activity-rest-*` | A spawn group mixing three managed delegations with a provider's own subagent, live `@chat:` pills in the reply (a full id, an abbreviated id and a top-level chat), a task's result as a single report, a burst of three reports under one summary (a question a task asked, a failure, a side chat's message), and the parent's tasks folded in the sidebar with their running and waiting counts. |
| `activity-open-*` | The audit's result and the review's question unfolded (Markdown body, its rail, `Open task`), and the parent's tree unfolded in the sidebar with nested tasks. |

| Scene | Dark | Light |
|---|---|---|
| Rest | [View](activity-rest-dark.png) | [View](activity-rest-light.png) |
| Open | [View](activity-open-dark.png) | [View](activity-open-light.png) |

Build and run:

```
cargo run -p zeron-ui --features appshots-fixture --example orchestration-activity-fixture -- <output>
```

Android captures come from the debug APK on the `zeron-github-projects-review`
emulator (Pixel 7, API 36), demo workspace, chat "Port PR 608 presentation":

| Capture | What it shows |
|---|---|
| [android-parent](android-parent.png) | Delegation cards with harness marks and a live spinner, `@chat:` pills, a finished report with its preview, and a question report. |
| [android-report-open](android-report-open.png) | A report unfolded like a tool group: body, rail, `Open task`. |
| [android-child-opened](android-child-opened.png) | Tapping a pill or a delegation card opens the child; "Task from …" leads back up. |
| [android-task-tree](android-task-tree.png) | The parent's tasks sheet: working, needs input, one finished. |
