# Task lanes review captures

Native captures of the lanes card's delegated tasks (see `docs/orchestration-ui.md`), cropped
to the card's column. They come in dark (frosted) and light (opaque) pairs:

| Scene | What it shows |
|---|---|
| `lanes-rest-*` | Every row folded. Tasks lead the card, side chats follow, forks carry the fork glyph. |
| `lanes-inspect-*` | Expanded rows: a working task with its facts, a task that needs input, a task that is stopping. |
| `lanes-results-*` | A failed stop with `Try again`, a completed result not sent yet, and the finished fold open. |
| `lanes-confirm-merge-*` | The in-place stop confirmation, and a fork's open merge strip. |
| `lanes-nested-*` | A task's own card: "Delegated by …" leads back up, and its queued task is listed. |
| `history-confirm-*` | History: an inspected checkpoint with its in-place restore confirm, and one blocked by the engine. |
| `history-receipt-*` | "Restored to … Undo", a failed restore with `Try again`, and the recovery fold open. |

| Scene | Dark | Light |
|---|---|---|
| Rest | [View](lanes-rest-dark.png) | [View](lanes-rest-light.png) |
| Inspect | [View](lanes-inspect-dark.png) | [View](lanes-inspect-light.png) |
| Results | [View](lanes-results-dark.png) | [View](lanes-results-light.png) |
| Confirm and merge | [View](lanes-confirm-merge-dark.png) | [View](lanes-confirm-merge-light.png) |
| Nested | [View](lanes-nested-dark.png) | [View](lanes-nested-light.png) |
| History: confirm | [View](history-confirm-dark.png) | [View](history-confirm-light.png) |
| History: receipt | [View](history-receipt-dark.png) | [View](history-receipt-light.png) |

Build and run:

```
cargo run -p zeron-ui --features appshots-fixture --example task-lanes-fixture -- <output>
```

The fixture writes `result.txt` with a PASS marker once every scene has been captured.
