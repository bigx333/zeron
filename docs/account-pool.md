# Native account pool

Claude Code and Codex can route requests through the saved accounts on the device
running the chat. Enable **Account pool** in **Settings → Accounts** or in the
provider's expanded Accounts section under **Settings → Agents**. Pooling is off
by default. Accounts use the existing sign-in and private credential-slot store;
Claude also supports adding an Anthropic API key without changing the CLI login.

**Soonest weekly reset** is the default routing mode. Each request uses the
available account whose applicable weekly quota resets first. Session limits,
weekly limits, model-family limits, temporary holds and rejected credentials can
move a request to another account. When a temporary limit resets, the preferred
account becomes eligible again. Unknown or expired weekly reset dates sort after
known dates. Account order breaks ties. A monthly Codex window is never treated
as a weekly window.

**Priority order** keeps a current account and conversation bindings, advancing
after an account becomes unavailable. Bindings and the current account survive
engine restarts. Bindings expire after 30 idle minutes; at most 4,096 are retained.
A model-family detour does not replace a healthy shared binding.

The default switch threshold is 98%. Account participation and order are
independent of the CLI's live login. Changes to routing or a chat's bypass take
effect on its next turn. The composer usage card shows pool availability and
allows a chat to use the live login instead. Codex voice uses the live login;
title generation and model/command discovery also use the CLI's own credentials.

The composer percentage combines the enabled, supported accounts for the chat's
provider. Each measured account contributes its most-used shared or applicable
model-family limit, and the ring shows the average of those fractions. For
example, accounts at 20% and 80% produce 50% combined use. Accounts that have hit
a limit remain in the total; excluded and unsupported accounts do not. Expired
limits contribute zero until new observations arrive. Providers do not expose
comparable absolute quota capacities, so this is an equally weighted account
average, not a token-weighted estimate across different plans.

The tooltip labels combined usage and reports how many enabled accounts have
measurements. Unknown usage, including API-key capacity, never counts as unused
capacity; if no account reports usage, the trigger says **Pool**. Routing to a
different next account does not change the total. The ring warns when no member
can currently take a request, retaining the measured percentage. A bypassed chat
continues to show the live account's usage. Pool observations refresh every 30
seconds without forcing provider probes while the composer is mounted.

## Engine behavior

The engine lazily binds an HTTP/1.1 hub on `127.0.0.1` at an ephemeral port.
Only supported Anthropic Messages and Codex endpoints are accepted. A random
engine-local capability authenticates requests. It is passed to the agent
process through its environment, never serialized into chat configuration,
documents or RPC replies. Nested engines discard inherited pool capabilities.
Upstream authorization is replaced with the selected account's credentials;
only allowlisted client headers are forwarded. Claude request account metadata
is rewritten to match the selected account.

Quota headers update the native account meters. Background probes are scoped
to the pool's provider, reuse the usage cache's throttling and backoff, and do
not serialize healthy requests behind account-list refreshes. Exhausted accounts
are rechecked before refusing a request. A refusal includes `Retry-After` when
an unblock time is known. Short temporary rate limits wait once on the same
account, for up to 20 seconds; longer limits hold that account while others can
serve requests.

Fallback happens before the hub commits a response. Once streaming begins,
the hub never replays that request on another account. An interrupted SSE stream
ends with an error event. Closing the client stream drops the upstream request.
Codex retains bounded client retries as separate requests.

Inactive OAuth slots refresh through a single-flight path. The pool never
refreshes or overwrites the CLI's live credential store. It rereads live tokens
after rejection; an expired live login is skipped until the CLI renews it.
Temporary token-endpoint failures back off and can continue using an unexpired
access token. Freshly rotated credentials are retained in memory if a slot write
temporarily fails, with persistence retried in the background and at shutdown.

Settings and non-secret observations live under the existing device-local
`agent-accounts` directory, in `pool.json` and `pool-state.json`. Credential slots
retain atomic private writes (`0600` on Unix). Hub capabilities are not persisted.
Remote settings changes are forwarded to the owning engine using the existing
`targetDeviceId` mechanism.

## RPC surface

- `ListAgentAccounts` includes an additive `pool` snapshot with settings,
  provider availability, the next account per model family and per-account status.
- `ConfigureAccountPool { harness, config }` accepts a complete provider config:
  `enabled`, `routingMode` (`weekly-reset` or `sequential`), `switchThreshold`,
  `accountOrder`, and `disabledAccountIds`. A nonempty order must name every
  saved account for that provider exactly once.
- `SetAccountPoolBypass { chatId, bypassed }` changes the device-local chat bypass.
- `AddAccountPoolApiKey { harness: "claude-code", apiKey, label? }` saves an
  Anthropic key without switching the CLI login. Replies contain no key material.

These features replace the BB plugin's routing and account controls. BB plugin
loading, `bb pool` commands and BB parent-server proxying are not used by Zeron.

## Validation

Validated on macOS on 2026-10-15:

- `cargo check --locked -p zeron` passed.
- The engine library suite passed (524 tests, 2 ignored), along with all 17
  account/upload/title integration tests. Follow-up pool tests passed after
  adding model-family picks, legacy metadata handling and zero-expiry handling:
  20 pool tests in total.
- `cargo test -p zeron-harness --lib --tests -- --test-threads=1` passed,
  including all 322 library tests and the native Claude/Codex protocol fixtures.
- All 1,785 UI library tests passed with `--test-threads=1`; the parallel UI run
  aborted without naming a failing test. The serial run covers the changed UI.
- The installed Codex 0.162.1 accepted the pool provider settings with strict
  config validation. A local rollout test exposed the missing-provider resume
  case; a protocol regression test verifies recovery without losing history.
- [Native captures](screenshots/account-pool/README.md) were reviewed in light
  and dark appearances, wide and narrow layouts, and embedded provider settings.

Provider request tests use isolated credentials and local mock upstreams. No
live-provider inference smoke test was run.
