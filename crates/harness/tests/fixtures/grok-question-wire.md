# Grok question wire capture

Captured locally on 2026-10-16 with Grok **1.0.60**, model **grok-4.7**:

```sh
grok --no-auto-update agent --no-leader --model grok-4.7 stdio
```

`grok-question-request.json` is the actual server request. The initialize
capabilities were the normal Zeron capabilities: filesystem access and terminal
access disabled, with no `_meta` capability. The native question tool was
available without advertising anything extra or overriding Grok's feature flags.

`grok-question-response.json` is the successful result payload sent back on that
request's JSON-RPC id. Grok continued with `Color: Blue` and `Fruit: Apple, Pear`.
Answer keys are the original **question text**, not question ids. The wire has
no separate header, so Zeron leaves `header` empty and clients fall back to the question. Multi-select uses `multiSelect`.

Grok also sent this notification (twice on the successful acceptance):

```json
{"method":"_x.ai/session_notification","params":{"sessionId":"…","update":{"sessionUpdate":"interaction_resolved","tool_call_id":"…"}}}
```

The notification has no reason identifying acceptance versus timeout. It
withdraws the matching bridge receiver. A terminal `tool_call_update` also
withdraws it. Turn stop and interrupt return `{"outcome":"cancelled"}`.

[xAI's response types](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-tools/src/implementations/grok_build/ask_user_question/types.rs)
specify free text as `answers[question] = ["Other"]` plus
`annotations[question].notes`, and selected single-option previews as
`annotations[question].preview`. Zeron preserves both. Grok owns its configured
timeout, including `timeout_enabled = false`; Zeron adds no competing timer.

## Live Zeron check

The updated desktop build ran in an isolated local workspace on IPC port 49779.
Chat `aa482900-aa90-4c07-bd36-28e753519700` used Grok 1.0.60 / grok-4.7.
Its native color question appeared in the waiting-input panel with Blue and
Green options. Typing **Violet** into that panel and pressing Return reached
Grok, which replied **Confirmed: Violet**. The chat returned to `idle` with
`pendingInput: null`. This exercised free-text annotations through the real UI.
