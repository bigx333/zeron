//! Isolated visual evidence for the native account pool UI: Settings →
//! Accounts, an expanded agent's embedded list, and the composer footer, painted from a synthetic
//! snapshot. No engine, no provider calls, no credentials.
//!
//! cargo run -p zeron-ui --example account-pool-fixture \
//!     --features account-pool-fixture -- <output-dir>
use chrono::{TimeDelta, Utc};
use gpui::{
    AppContext, AsyncApp, Bounds, Context, Entity, Render, Window, WindowBounds, WindowOptions,
    div, prelude::*, px, size,
};
use std::{path::Path, path::PathBuf, time::Duration};
use zeron_proto::{
    AccountPoolAccountStatus, AccountPoolProviderStatus, AccountPoolRoutingMode, AccountPoolStatus,
    AgentAccount, AgentAccountsSnapshot, AgentAuthKind, AgentUsageWindow, HarnessId,
};
use zeron_ui::settings::accounts::AccountsPage;
use zeron_ui::*;

struct Fixture {
    page: Entity<AccountsPage>,
    composer: Option<Entity<composer::Composer>>,
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::Theme::of(cx).for_settings_surface();
        let content = match &self.composer {
            Some(composer) => div()
                .size_full()
                .flex()
                .flex_col()
                .justify_end()
                .child(composer.clone())
                .into_any_element(),
            None => self.page.clone().into_any_element(),
        };
        div()
            .id("account-pool-fixture")
            .size_full()
            .bg(theme.surface)
            .text_color(theme.text)
            .font_family(theme.font_sans.clone())
            .p(px(24.0))
            .child(content)
    }
}

fn account(
    id: &str,
    harness: HarnessId,
    email: &str,
    active: bool,
    windows: &[(&str, f32, i64)],
) -> AgentAccount {
    let now = Utc::now();
    AgentAccount {
        id: id.into(),
        harness,
        email: Some(email.into()),
        plan_label: Some(
            if harness == HarnessId::Codex {
                "Pro"
            } else {
                "Max 20x"
            }
            .into(),
        ),
        active,
        usage_windows: windows
            .iter()
            .map(|(label, used, hours)| AgentUsageWindow {
                label: (*label).into(),
                used_fraction: *used,
                resets_at: Some(now + TimeDelta::hours(*hours)),
            })
            .collect(),
        usage_fetched_at: None,
        usage_error: None,
        display_name: None,
        organization: None,
        auth_kind: None,
        switchable: true,
        saved_at: None,
        provider: None,
    }
}

fn status(id: &str, status: AccountPoolStatus) -> AccountPoolAccountStatus {
    AccountPoolAccountStatus {
        account_id: id.into(),
        status,
        until: None,
        family: None,
        weekly_reset_at: None,
        poolable: true,
        unpoolable_reason: None,
        held_until: None,
        error: None,
        last_used_at: None,
        in_flight: 0,
    }
}

/// Claude pooled (soonest weekly reset) across every status; Codex off.
fn snapshot(claude_on: bool, notice: bool) -> AgentAccountsSnapshot {
    let now = Utc::now();
    let mut key = account("key", HarnessId::ClaudeCode, "Team API key", false, &[]);
    key.auth_kind = Some(AgentAuthKind::ApiKey);
    key.plan_label = Some("API key".into());
    let mut frank = account(
        "frank",
        HarnessId::ClaudeCode,
        "frank@example.com",
        false,
        &[],
    );
    frank.usage_error = Some("Signed out — sign in again".into());
    let mut snapshot = AgentAccountsSnapshot {
        accounts: vec![
            account(
                "alice",
                HarnessId::ClaudeCode,
                "alice@example.com",
                false,
                &[
                    ("Session", 0.22, 3),
                    ("Week", 0.41, 60),
                    ("Week (opus)", 0.12, 60),
                ],
            ),
            account(
                "bob",
                HarnessId::ClaudeCode,
                "bob@example.com",
                true,
                &[("Session", 0.05, 4), ("Week", 0.63, 120)],
            ),
            account(
                "carol",
                HarnessId::ClaudeCode,
                "carol@example.com",
                false,
                &[("Session", 0.99, 2), ("Week", 0.8, 30)],
            ),
            account(
                "dave",
                HarnessId::ClaudeCode,
                "dave@example.com",
                false,
                &[
                    ("Session", 0.3, 1),
                    ("Week", 0.5, 90),
                    ("Week (opus)", 0.98, 90),
                ],
            ),
            account(
                "erin",
                HarnessId::ClaudeCode,
                "erin@example.com",
                false,
                &[("Session", 0.1, 4), ("Week", 0.2, 140)],
            ),
            frank,
            key,
            account(
                "x1",
                HarnessId::Codex,
                "dev@example.com",
                true,
                &[("5h", 0.34, 2), ("Week", 0.27, 100)],
            ),
            account(
                "x2",
                HarnessId::Codex,
                "ops@example.com",
                false,
                &[("5h", 0.0, 5), ("Week", 0.71, 50)],
            ),
        ],
        warnings: vec![],
        pool: Default::default(),
    };
    let claude = &mut snapshot.pool.config.claude;
    claude.enabled = claude_on;
    claude.routing_mode = AccountPoolRoutingMode::WeeklyReset;
    claude.account_order = ["alice", "bob", "carol", "dave", "erin", "frank", "key"]
        .map(String::from)
        .to_vec();
    claude.disabled_account_ids = vec!["erin".into()];
    snapshot.pool.providers = vec![
        AccountPoolProviderStatus {
            harness: HarnessId::ClaudeCode,
            serving: true,
            next_account_id: Some("alice".into()),
            next_account_ids: Default::default(),
            notice: notice.then(|| {
                "Every pooled account is limited or unavailable — requests resume when an \
                 account is ready."
                    .into()
            }),
        },
        AccountPoolProviderStatus {
            harness: HarnessId::Codex,
            serving: true,
            next_account_id: Some("x1".into()),
            next_account_ids: Default::default(),
            notice: None,
        },
    ];
    let mut alice = status("alice", AccountPoolStatus::Next);
    alice.weekly_reset_at = Some(now + TimeDelta::hours(60));
    let mut bob = status("bob", AccountPoolStatus::Ready);
    bob.weekly_reset_at = Some(now + TimeDelta::hours(120));
    let mut carol = status("carol", AccountPoolStatus::Limited);
    carol.until = Some(now + TimeDelta::hours(2));
    let mut dave = status("dave", AccountPoolStatus::FamilyLimited);
    dave.family = Some("opus".into());
    dave.until = Some(now + TimeDelta::hours(90));
    let mut frank = status("frank", AccountPoolStatus::NeedsSignIn);
    frank.error = Some("The refresh token was revoked — sign in again.".into());
    let mut key = status("key", AccountPoolStatus::Ready);
    key.weekly_reset_at = None;
    snapshot.pool.accounts = vec![
        alice,
        bob,
        carol,
        dave,
        status("erin", AccountPoolStatus::Excluded),
        frank,
        key,
        status("x1", AccountPoolStatus::Next),
        status("x2", AccountPoolStatus::Ready),
    ];
    snapshot
}

async fn pause(cx: &mut AsyncApp) {
    cx.background_executor()
        .timer(Duration::from_millis(450))
        .await;
}

fn capture(
    window: gpui::AnyWindowHandle,
    cx: &mut AsyncApp,
    output: &Path,
    name: &str,
) -> anyhow::Result<()> {
    window.update(cx, |_, window, cx| {
        window.draw(cx).clear();
        window.render_to_image()?.save(output.join(name))?;
        Ok(())
    })?
}

async fn scene(
    window: gpui::WindowHandle<Fixture>,
    cx: &mut AsyncApp,
    output: &Path,
    name: &str,
    setup: impl FnOnce(&mut Fixture, &mut Window, &mut Context<Fixture>),
) -> anyhow::Result<()> {
    window.update(cx, |view, window, cx| {
        setup(view, window, cx);
        cx.notify();
    })?;
    pause(cx).await;
    capture(window.into(), cx, output, name)
}

fn main() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let _guard = runtime.enter();
    let output = PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&output)?;
    let temp = tempfile::tempdir()?;
    let data = temp.path().to_path_buf();
    gpui_platform::application()
        .with_assets(icons::Assets)
        .run(move |cx| {
            gpui_tokio::init(cx);
            gpui_base::init(cx);
            let prefs = settings::UiSettings::default();
            settings::init(prefs.clone(), data.clone(), cx);
            let fonts = typography::register_fonts(cx);
            typography::init(
                prefs.ui_font_family.clone(),
                prefs.ui_font_size,
                prefs.terminal_font_family.clone(),
                prefs.terminal_font_size,
                prefs.code_font_family.clone(),
                prefs.code_font_size,
                fonts,
                cx,
            );
            theme_library::init(data, cx);
            appearance::init(
                appearance::AppearanceMode::Dark,
                prefs.theme_selection,
                prefs.accent,
                prefs.surface,
                cx,
            );
            composer::init(cx, prefs.composer_send_behavior);

            let state = cx.new(|_| state::AppState::new());
            let standalone = cx.new(|cx| AccountsPage::new(state.clone(), cx));
            let embedded =
                cx.new(|cx| AccountsPage::new_embedded(state, None, HarnessId::ClaudeCode, cx));
            let composer_state = cx.new(|_| {
                let mut state = state::AppState::new();
                state.local_device_id = Some("preview-device".into());
                state.selected_chat = Some("pool-preview".into());
                state.chats = vec![serde_json::from_value(serde_json::json!({
                    "id": "pool-preview",
                    "deviceId": "preview-device",
                    "title": "Account pool preview",
                    "archived": false,
                    "createdAt": Utc::now(),
                    "config": {
                        "harness": "claude-code",
                        "model": "claude-opus-4-6",
                        "sandbox": "workspace-write"
                    }
                })).unwrap()];
                state.context_usage = Some(zeron_proto::ContextUsage {
                    tokens: Some(48_000),
                    window: Some(200_000),
                });
                state
            });
            let composer = cx.new(|cx| composer::Composer::new(composer_state, cx));
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            gpui::point(px(40.0), px(40.0)),
                            size(px(900.0), px(1180.0)),
                        ))),
                        ..Default::default()
                    },
                    |_, cx| {
                        cx.new(|_| Fixture {
                            page: standalone.clone(),
                            composer: None,
                        })
                    },
                )
                .unwrap();
            cx.activate(true);

            cx.spawn(async move |cx| {
                let result: anyhow::Result<()> = async {
                    pause(cx).await;
                    let set = |page: &Entity<AccountsPage>,
                               snapshot: AgentAccountsSnapshot,
                               menu: Option<&'static str>| {
                        let page = page.clone();
                        move |view: &mut Fixture, _: &mut Window, cx: &mut Context<Fixture>| {
                            page.update(cx, |page, cx| page.fixture_snapshot(snapshot, menu, cx));
                            view.page = page;
                        }
                    };
                    scene(window, cx, &output, "pool-off-dark.png", set(&standalone, snapshot(false, false), None)).await?;
                    scene(window, cx, &output, "pool-on-dark.png", set(&standalone, snapshot(true, false), None)).await?;
                    scene(window, cx, &output, "pool-on-notice-dark.png", set(&standalone, snapshot(true, true), None)).await?;
                    scene(window, cx, &output, "pool-row-menu-dark.png", set(&standalone, snapshot(true, false), Some("carol"))).await?;
                    scene(window, cx, &output, "pool-embedded-dark.png", set(&embedded, snapshot(true, false), None)).await?;
                    window.update(cx, |_, window, cx| {
                        appearance::set_mode(appearance::AppearanceMode::Light, cx);
                        window.resize(size(px(900.0), px(1180.0)));
                    })?;
                    scene(window, cx, &output, "pool-on-light.png", set(&standalone, snapshot(true, false), None)).await?;
                    window.update(cx, |_, window, _| window.resize(size(px(620.0), px(1180.0))))?;
                    scene(window, cx, &output, "pool-on-light-narrow.png", set(&standalone, snapshot(true, false), None)).await?;
                    window.update(cx, |view, window, cx| {
                        appearance::set_mode(appearance::AppearanceMode::Dark, cx);
                        window.resize(size(px(900.0), px(220.0)));
                        view.composer = Some(composer.clone());
                        composer.update(cx, |composer, cx| {
                            composer.fixture_account_pool(snapshot(true, false), false, cx);
                            composer.set_available_width(852.0, cx);
                        });
                        cx.notify();
                    })?;
                    pause(cx).await;
                    capture(window.into(), cx, &output, "pool-composer-dark.png")?;
                    window.update(cx, |_, window, _| window.resize(size(px(900.0), px(660.0))))?;
                    composer.update(cx, |composer, cx| composer.fixture_account_pool(snapshot(true, false), true, cx));
                    pause(cx).await;
                    capture(window.into(), cx, &output, "pool-composer-card-dark.png")?;
                    window.update(cx, |_, _, cx| appearance::set_mode(appearance::AppearanceMode::Light, cx))?;
                    pause(cx).await;
                    capture(window.into(), cx, &output, "pool-composer-card-light.png")?;
                    std::fs::write(
                        output.join("result.txt"),
                        "PASS: account pool settings and production composer captured (off, on, notice, row menu, embedded; dark/light; wide/narrow; footer and expanded card).\n",
                    )?;
                    Ok(())
                }
                .await;
                if let Err(error) = result {
                    eprintln!("account pool fixture failed: {error:#}");
                }
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    Ok(())
}
