# Native account pool

Native GPUI captures with synthetic accounts; no engine, credentials or provider
requests are involved. Opus 5.5 designed and implemented the settings layout,
account controls and motion using Zeron's existing primitives.

```sh
cargo run -p zeron-ui --example account-pool-fixture --features account-pool-fixture -- /tmp/zeron-account-pool-captures
```

- [Dark, full account list](pool-on-dark.png)
- [Light, narrow account list](pool-on-light-narrow.png)
- [Embedded provider accounts](pool-embedded-dark.png)
- [Composer footer](pool-composer-dark.png)
- [Composer account card, dark](pool-composer-card-dark.png)
- [Composer account card, light](pool-composer-card-light.png)

Visual QA checked wide and narrow settings, light and dark appearances, provider
embedding, account actions, and notices. The narrow layout keeps account identity
and status to one truncated line, reduces meter width, and retains uniform row
heights. The fixture additionally captures pool-off and open-menu states, and the
production composer with its pool indicator and expanded card. Composer captures
use an isolated synthetic chat and account snapshot with no engine connection.
The composer percentage is combined normalized usage across the enabled
accounts with measurements, not the usage of the next selected account.

Rendered tests cover reduced-motion behavior, fixed account-row height,
participation, reorder, optimistic rollback and pool-off switching. Footer tests
cover combined pool usage, model-family limits, provider scoping, excluded and
unsupported members, expired limits, unknown measurements and per-chat bypass.
