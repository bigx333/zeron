//! Isolated native review fixture for the lanes card's delegated tasks: a
//! chat with tasks in every state (working, needs input, stopping, a stop
//! that failed, a result not sent yet, the finished fold), a fork with its
//! merge strip open, and a nested task whose card leads back up.
//!
//! `cargo run -p zeron-ui --features appshots-fixture --example task-lanes-fixture -- <dir>`
//! saves dark (frosted) and light (opaque) captures of each scene, then quits.
use gpui::{AppContext, Bounds, WindowBounds, WindowOptions, px, size};
use zeron_ui::*;

fn main() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let _guard = runtime.enter();
    let output = std::path::PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&output)?;
    let temp = tempfile::tempdir()?;
    let data = temp.path().to_path_buf();
    gpui_platform::application()
        .with_assets(icons::Assets)
        .run(move |cx| {
            gpui_tokio::init(cx);
            gpui_base::init(cx);
            let mut settings = settings::UiSettings::default();
            settings.surface = zeron_theme::SurfacePreference::Frosted;
            settings.save(&data).unwrap();
            settings::init(settings.clone(), data.clone(), cx);
            let fonts = typography::register_fonts(cx);
            typography::init(
                settings.ui_font_family.clone(),
                settings.ui_font_size,
                settings.terminal_font_family.clone(),
                settings.terminal_font_size,
                settings.code_font_family.clone(),
                settings.code_font_size,
                fonts,
                cx,
            );
            theme_library::init(data.clone(), cx);
            appearance::init(
                appearance::AppearanceMode::Dark,
                settings.theme_selection,
                settings.accent,
                settings.surface,
                cx,
            );
            history::init(
                settings.git_history_columns,
                settings.git_history_column_widths,
                settings.git_history_column_order,
                settings.git_history_author_display,
                cx,
            );
            composer::init(cx, settings.composer_send_behavior);
            terminal::panel::init(cx);
            app_menus::init(cx);

            let now = chrono::Utc::now();
            let ms = |minutes: i64| (now - chrono::Duration::minutes(minutes)).timestamp_millis();
            let state = cx.new(|_| {
                let mut s = state::AppState::new();
                s.connection = zeron_proto::view::ConnectionStatus::Ready;
                s.workspace_scope = Some(zeron_proto::WorkspaceScope::Local);
                s.local_device_id = Some("local".into());
                s.devices = vec![serde_json::from_value(serde_json::json!({
                    "id": "local", "name": "This device", "platform": std::env::consts::OS, "lastSeenAt": null,
                })).unwrap()];
                s.selected_chat = Some("main".into());
                s.auto_selected = true;
                s.chats_synced = true;
                s.spaces_synced = true;
                let chat = |id: &str, title: &str, parent: Option<&str>, harness: &str, model: &str, orchestration: serde_json::Value| -> zeron_proto::Chat {
                    serde_json::from_value(serde_json::json!({
                        "id": id, "deviceId": "local", "title": title, "archived": false,
                        "createdAt": now - chrono::Duration::minutes(40),
                        "parentChatId": parent,
                        "config": {"harness": harness, "model": model, "reasoning": null, "sandbox": "workspace-write"},
                        "orchestration": orchestration,
                    })).unwrap()
                };
                let task = |id: &str, parent: &str, status: &str, role: &str, isolation: &str, created: i64, updated: i64, extra: serde_json::Value| {
                    let mut task = serde_json::json!({
                        "id": format!("task-{id}"), "parentChatId": parent, "role": role, "isolation": isolation,
                        "status": status, "prompt": "", "requestFingerprint": id,
                        "createdAt": ms(created), "updatedAt": ms(updated),
                    });
                    for (key, value) in extra.as_object().unwrap() {
                        task[key] = value.clone();
                    }
                    serde_json::json!({ "task": task })
                };
                s.chats = vec![
                    chat("main", "Migrate T3 orchestration", None, "claude-code", "claude-opus-5-5", serde_json::Value::Null),
                    chat("t-audit", "Audit the T3 handoff budget", Some("main"), "codex", "gpt-5.3-codex",
                        task("audit", "main", "running", "research", "worktree", 14, 1, serde_json::json!({}))),
                    chat("t-review", "Review the merge RPC contract", Some("main"), "claude-code", "claude-sonnet-5-5",
                        task("review", "main", "waiting", "review", "shared", 11, 2, serde_json::json!({}))),
                    chat("t-icons", "Port lineage icons to the lanes card", Some("main"), "codex", "gpt-5.3-codex",
                        task("icons", "main", "running", "design", "shared", 9, 0, serde_json::json!({"cancelRequested": true}))),
                    chat("t-sync", "Profile registry sync under load", Some("main"), "codex", "gpt-5.3-codex",
                        task("sync", "main", "running", "test", "worktree", 7, 3, serde_json::json!({}))),
                    chat("t-cancel", "Draft the cancellation copy", Some("main"), "claude-code", "claude-sonnet-5-5",
                        task("cancel", "main", "completed", "general", "shared", 6, 0, serde_json::json!({
                            "summary": "Stop is the action everywhere; the terminal label stays Cancelled to match the task status. Errors stay inline on the row so they remain inspectable, with Try again as the retry.",
                        }))),
                    chat("t-notes", "Summarize T3 lineage docs", Some("main"), "claude-code", "claude-haiku-5-5",
                        task("notes", "main", "completed", "research", "shared", 48, 30, serde_json::json!({
                            "summary": "Lineage is parent + relationship + root.", "delivered": true,
                        }))),
                    chat("t-ffi", "Mirror orchestration in the FFI", Some("main"), "codex", "gpt-5.3-codex",
                        task("ffi", "main", "failed", "implementation", "worktree", 52, 33, serde_json::json!({"delivered": true}))),
                    chat("t-old", "Try the native fork path", Some("main"), "codex", "gpt-5.3-codex",
                        task("old", "main", "cancelled", "implementation", "worktree", 60, 41, serde_json::json!({"cancelRequested": true}))),
                    chat("f-portable", "Try the portable fork", Some("main"), "claude-code", "claude-opus-5-5",
                        serde_json::json!({"fork": {"sourceChatId": "main", "sourceMessageId": "m1", "createdAt": ms(25), "strategy": "portable"}})),
                    chat("s-plain", "Ask about reduced motion", Some("main"), "claude-code", "claude-opus-5-5", serde_json::Value::Null),
                    // A task's own task, for the nested scene.
                    chat("t-audit-sub", "Measure summary budgets", Some("t-audit"), "codex", "gpt-5.3-codex",
                        task("audit-sub", "t-audit", "queued", "research", "shared", 1, 1, serde_json::json!({}))),
                ];
                s.chats[9].last_message_at = Some(now - chrono::Duration::minutes(25));
                s.chats[10].last_message_at = Some(now - chrono::Duration::hours(2));
                s
            });
            let boot = EngineBootConfig {
                data_dir: data,
                ipc_port: 0,
                edge_url: String::new(),
                edge_token: None,
                org_id: None,
                workos_client_id: None,
                default_harness: HarnessId::ClaudeCode,
            };
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            gpui::point(px(12.), px(30.)),
                            size(px(1500.), px(1180.)),
                        ))),
                        titlebar: Some(gpui::TitlebarOptions {
                            title: None,
                            appears_transparent: true,
                            traffic_light_position: Some(gpui::point(px(14.), px(14.))),
                        }),
                        app_owns_titlebar_drag: true,
                        window_background: theme::Theme::of(cx).window_background_appearance(),
                        ..Default::default()
                    },
                    |_, cx| cx.new(|cx| shell::Shell::new(state.clone(), boot, cx)),
                )
                .unwrap();
            cx.spawn(async move |cx| {
                let run: anyhow::Result<()> = async {
                    let shot = async |cx: &mut gpui::AsyncApp, name: &str, ms: u64| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(ms))
                            .await;
                        let capture: gpui::AnyWindowHandle = window.into();
                        capture.update(cx, |_, w, cx| {
                            w.draw(cx).clear();
                            w.render_to_image()?.save(output.join(format!("{name}.png")))?;
                            anyhow::Ok(())
                        })?
                    };
                    let select = |cx: &mut gpui::AsyncApp, chat: &'static str| {
                        state.update(cx, |s, cx| s.select_chat(Some(chat.into()), cx))
                    };
                    for (look, light) in [("dark", false), ("light", true)] {
                        if light {
                            cx.update(|cx| {
                                appearance::set_mode(appearance::AppearanceMode::Light, cx);
                                appearance::set_surface(zeron_theme::SurfacePreference::Opaque, cx);
                                appearance::reapply_window_background(cx);
                            });
                        }
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(400))
                            .await;
                        select(cx, "main");
                        // Rest: every row folded.
                        window.update(cx, |s, _, cx| s.fixture_task_lanes(&[], false, None, None, None, cx))?;
                        shot(cx, &format!("lanes-rest-{look}"), 700).await?;
                        // Inspect: a working task, one stopping, one asking.
                        window.update(cx, |s, _, cx| {
                            s.fixture_task_lanes(&["t-audit", "t-icons", "t-review"], false, None, None, None, cx)
                        })?;
                        shot(cx, &format!("lanes-inspect-{look}"), 700).await?;
                        // Results: one not sent yet, a stop that failed, a pending
                        // confirm, and the finished fold open.
                        window.update(cx, |s, _, cx| {
                            s.fixture_task_lanes(
                                &["t-cancel", "t-sync", "t-ffi", "t-old"],
                                true,
                                None,
                                Some(("t-sync", "Couldn't stop the task. The build server didn't answer.")),
                                None,
                                cx,
                            )
                        })?;
                        shot(cx, &format!("lanes-results-{look}"), 700).await?;
                        window.update(cx, |s, _, cx| {
                            s.fixture_task_lanes(&["t-audit"], false, Some("t-audit"), None, Some("f-portable"), cx)
                        })?;
                        shot(cx, &format!("lanes-confirm-merge-{look}"), 700).await?;
                        // History: checkpoints with one inspected, a restore
                        // confirm, a failed restore, the receipt with Undo,
                        // and the recovery fold open.
                        let ms = |minutes: i64| (chrono::Utc::now() - chrono::Duration::minutes(minutes)).timestamp_millis();
                        let checkpoints = serde_json::json!([
                            {"id": "k5", "title": "Wire the History section", "createdAtMs": ms(3), "providerStrategy": "native", "filesAvailable": true, "filesChanged": 4, "restorable": true},
                            {"id": "k4", "title": "Port lineage icons", "createdAtMs": ms(18), "providerStrategy": "native", "filesAvailable": true, "filesChanged": 12, "restorable": true},
                            {"id": "k3", "title": "Draft the cancellation copy", "createdAtMs": ms(41), "providerStrategy": "portable", "filesAvailable": false, "restorable": false, "blockedReason": "Stop tasks working in this checkout first."},
                            {"id": "k2", "title": "Audit the handoff budget", "createdAtMs": ms(70), "providerStrategy": "portable", "filesAvailable": true, "restorable": true},
                            {"id": "r1", "title": "Before restoring \u{201c}Port lineage icons\u{201d}", "createdAtMs": ms(2), "recovery": true, "providerStrategy": "native", "filesAvailable": true, "filesChanged": 9, "restorable": true},
                        ]);
                        window.update(cx, |s, _, cx| {
                            s.fixture_task_lanes(&[], false, None, None, None, cx);
                            s.fixture_history(checkpoints.clone(), &["k4", "k3"], Some("k4"), None, None, false, cx);
                        })?;
                        cx.background_executor().timer(std::time::Duration::from_millis(300)).await;
                        window.update(cx, |s, _, cx| s.fixture_scroll_lanes_to_end(cx))?;
                        shot(cx, &format!("history-confirm-{look}"), 700).await?;
                        window.update(cx, |s, _, cx| {
                            s.fixture_history(
                                checkpoints.clone(),
                                &["k2"],
                                None,
                                Some(("k2", "Couldn't restore. The build server didn't answer.")),
                                Some(("Port lineage icons", "r1")),
                                true,
                                cx,
                            );
                        })?;
                        cx.background_executor().timer(std::time::Duration::from_millis(300)).await;
                        window.update(cx, |s, _, cx| s.fixture_scroll_lanes_to_end(cx))?;
                        shot(cx, &format!("history-receipt-{look}"), 700).await?;
                        // Nested: a task's own card leads back up and lists its task.
                        select(cx, "t-audit");
                        window.update(cx, |s, _, cx| s.fixture_task_lanes(&["t-audit-sub"], false, None, None, None, cx))?;
                        shot(cx, &format!("lanes-nested-{look}"), 700).await?;
                    }
                    std::fs::write(
                        output.join("result.txt"),
                        "PASS: task lanes rest/inspect/results/confirm+merge/history-confirm/history-receipt/nested, dark frosted and light opaque.\n",
                    )?;
                    Ok(())
                }
                .await;
                if let Err(error) = run {
                    eprintln!("task lanes fixture failed: {error:#}");
                }
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    Ok(())
}
