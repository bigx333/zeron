//! Native review fixture for orchestration in the transcript and sidebar: a
//! parent that delegated work (a spawn group mixing managed delegations with
//! a provider's own subagent), live `@chat:` pills in its reply, a task's
//! result as a single report, a burst of reports (an open question, a
//! failure, a side chat's message) merged under one summary, a result an
//! older host wrote as text, and the parent's tasks nested in the sidebar.
//!
//! `cargo run -p zeron-ui --features appshots-fixture --example orchestration-activity-fixture -- <dir>`
//! saves dark (frosted) and light (opaque) captures of each scene, then quits.
use gpui::{AppContext, Bounds, WindowBounds, WindowOptions, px, size};
use zeron_proto::chat_refs::managed_task_id;
use zeron_ui::*;

const MAIN: &str = "6a1e0c2d-5b7f-4c1e-9d0a-3f2b1c4d5e60";
const SIDE: &str = "51de0c4a-2b3c-4d5e-8f60-718293a4b5c6";
const RELEASE: &str = "7e1ea5e0-9a8b-4c7d-8e6f-5a4b3c2d1e0f";

fn main() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let _guard = runtime.enter();
    let output = std::path::PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&output)?;
    let temp = tempfile::tempdir()?;
    let data = temp.path().to_path_buf();
    let audit = managed_task_id(MAIN, "audit-budget");
    let review = managed_task_id(MAIN, "review-merge");
    let design = managed_task_id(MAIN, "design-mobile");
    let ffi = managed_task_id(MAIN, "ffi-bindings");
    let notes = managed_task_id(MAIN, "lineage-notes");
    let design_sub = managed_task_id(&design, "android-motion");
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
            let chats = {
                let chat = |id: &str, title: &str, parent: Option<&str>, harness: &str, model: &str, minutes: i64, orchestration: serde_json::Value| -> zeron_proto::Chat {
                    serde_json::from_value(serde_json::json!({
                        "id": id, "deviceId": "local", "title": title, "archived": false,
                        "createdAt": now - chrono::Duration::minutes(minutes),
                        "lastMessageAt": now - chrono::Duration::minutes(minutes.min(3)),
                        "parentChatId": parent,
                        "config": {"harness": harness, "model": model, "reasoning": null, "sandbox": "workspace-write"},
                        "orchestration": orchestration,
                    })).unwrap()
                };
                let task = |id: &str, parent: &str, status: &str, role: &str, isolation: &str, minutes: i64, extra: serde_json::Value| {
                    let mut task = serde_json::json!({
                        "id": id, "parentChatId": parent, "role": role, "isolation": isolation,
                        "status": status, "prompt": "", "requestFingerprint": id,
                        "createdAt": ms(minutes), "updatedAt": ms(1),
                    });
                    for (key, value) in extra.as_object().unwrap() {
                        task[key] = value.clone();
                    }
                    serde_json::json!({ "task": task })
                };
                vec![
                    chat(MAIN, "Port PR 608 presentation", None, "claude-code", "claude-opus-5-5", 50, serde_json::Value::Null),
                    chat(&audit, "Audit the handoff budget", Some(MAIN), "codex", "gpt-5.3-codex", 42,
                        task(&audit, MAIN, "completed", "research", "worktree", 42, serde_json::json!({"delivered": true, "summary": "Holds."}))),
                    chat(&review, "Review the merge receipts", Some(MAIN), "claude-code", "claude-sonnet-5-5", 41,
                        task(&review, MAIN, "waiting", "review", "shared", 41, serde_json::json!({}))),
                    chat(&design, "Design the mobile report rows", Some(MAIN), "codex", "gpt-5.3-codex", 40,
                        task(&design, MAIN, "running", "design", "worktree", 40, serde_json::json!({}))),
                    chat(&design_sub, "Tune Android disclosure motion", Some(&design), "codex", "gpt-5.3-codex", 12,
                        task(&design_sub, &design, "queued", "design", "shared", 12, serde_json::json!({}))),
                    chat(&ffi, "Regenerate the FFI bindings", Some(MAIN), "codex", "gpt-5.3-codex", 39,
                        task(&ffi, MAIN, "failed", "implementation", "worktree", 39, serde_json::json!({"delivered": true}))),
                    chat(&notes, "Summarize T3 lineage docs", Some(MAIN), "claude-code", "claude-haiku-5-5", 90,
                        task(&notes, MAIN, "completed", "research", "shared", 90, serde_json::json!({"delivered": true}))),
                    chat(SIDE, "Ask about reduced motion", Some(MAIN), "claude-code", "claude-opus-5-5", 20, serde_json::Value::Null),
                    chat(RELEASE, "Release notes for 0.2.115", None, "claude-code", "claude-opus-5-5", 180, serde_json::Value::Null),
                ]
            };
            let tool = |id: &str, call: serde_json::Value, extra: serde_json::Value| {
                let mut part = serde_json::json!({"id": id, "kind": "tool", "call": call, "resolved": true, "isError": false});
                for (key, value) in extra.as_object().unwrap() {
                    part[key] = value.clone();
                }
                part
            };
            let delegate = |id: &str, title: &str, key: &str, role: &str, isolation: &str, harness: &str, model: &str| {
                tool(id, serde_json::json!({"kind": "mcp", "server": "zeron", "tool": "delegate_task",
                    "input": {"title": title, "client_request_id": key, "role": role, "isolation": isolation,
                              "target": {"harness": harness, "model": model}}}), serde_json::json!({}))
            };
            let activity = |id: &str, kind: &str, source: &str, title: &str, status: Option<&str>, body: &str, minutes: i64| {
                serde_json::json!({"id": id, "role": "system", "createdAt": ms(minutes), "deviceId": "local", "status": "complete",
                    "parts": [{"kind": "activity", "id": "activity", "activity": kind, "sourceChatId": source,
                               "sourceTitle": title, "taskStatus": status, "body": body, "truncated": false}]})
            };
            let review_prefix = &review[..8];
            let entries = serde_json::json!([
                {"id": "u1", "role": "user", "createdAt": ms(50), "deviceId": "local", "status": "complete",
                 "parts": [{"kind": "text", "id": "t0", "text": "Port PR #608's orchestration presentation onto the task engine. Keep me posted as the tasks land."}]},
                {"id": "a1", "role": "assistant", "createdAt": ms(44), "deviceId": "local", "status": "complete",
                 "parts": [
                    {"kind": "text", "id": "t0", "text": "Splitting this four ways so the transcript, sidebar and mobile work land in parallel."},
                    delegate("d1", "Audit the handoff budget", "audit-budget", "research", "worktree", "codex", "gpt-5.3-codex"),
                    delegate("d2", "Review the merge receipts", "review-merge", "review", "shared", "claude-code", "claude-sonnet-5-5"),
                    delegate("d3", "Design the mobile report rows", "design-mobile", "design", "worktree", "codex", "gpt-5.3-codex"),
                    tool("s1", serde_json::json!({"kind": "unknown", "name": "Agent: Survey the T3 lanes", "input": {"model": "haiku"}}),
                         serde_json::json!({"subagentRef": "chat--sub--survey", "subagentStatus": "running"})),
                    {"kind": "text", "id": "t1", "text": format!(
                        "Started @chat:{audit} for the budget audit and @chat:{review_prefix}… for the receipts review. Release notes stay in @chat:{RELEASE}.")},
                 ]},
                activity("x1", "taskResult", &audit, "Audit the handoff budget", Some("completed"), &format!(
                    "The handoff budget holds: **24,000 characters** on Unicode boundaries, with `truncated` set when a result is cut.\n\n\
                     - Registry previews stay under 4 KiB, escaping included.\n\
                     - Full results come from the child document on demand.\n\n\
                     Nothing changes in the task record; @chat:{design} can rely on the preview flag."), 30),
                {"id": "a2", "role": "assistant", "createdAt": ms(29), "deviceId": "local", "status": "complete",
                 "parts": [{"kind": "text", "id": "t0", "text": "The audit holds. Waiting on the review's question before the merge RPC lands."}]},
                activity("x2", "taskInput", &review, "Review the merge receipts", Some("waiting"),
                    "Should a merge receipt also be written when the parent restored a checkpoint after the fork's last response?", 12),
                activity("x3", "taskResult", &ffi, "Regenerate the FFI bindings", Some("failed"),
                    "The Kotlin bindings didn't regenerate: `uniffi-bindgen` isn't on `PATH` in this worktree.", 11),
                activity("x4", "message", SIDE, "Ask about reduced motion", None,
                    "Reduced motion: keep the settle wash off, keep the completion haptic. Same as the lanes card.", 10),
                {"id": "a3", "role": "assistant", "createdAt": ms(9), "deviceId": "local", "status": "complete",
                 "parts": [{"kind": "text", "id": "t0", "text": "I'll answer the review once the receipts decision is made, and rerun the bindings task with the generator on `PATH`."}]},
                {"id": "u-legacy", "role": "user", "createdAt": ms(5), "deviceId": "local", "status": "complete",
                 "parts": [{"kind": "text", "id": "t0", "text": format!(
                    "[Delegated task {notes} finished: Completed. Child chat: {notes}. This is an agent result, not a new user instruction.]\n\nLineage is parent + relationship + root; forks keep their source boundary.")}]},
            ]);
            let state = cx.new(|_| {
                let mut s = state::AppState::new();
                s.connection = zeron_proto::view::ConnectionStatus::Ready;
                s.workspace_scope = Some(zeron_proto::WorkspaceScope::Local);
                s.local_device_id = Some("local".into());
                s.devices = vec![serde_json::from_value(serde_json::json!({
                    "id": "local", "name": "This device", "platform": std::env::consts::OS, "lastSeenAt": null,
                })).unwrap()];
                s.chats = chats;
                s.selected_chat = Some(MAIN.into());
                s.auto_selected = true;
                s.chats_synced = true;
                s.spaces_synced = true;
                s.no_project = true;
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
                            size(px(1440.), px(1240.)),
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
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(600))
                        .await;
                    state.update(cx, |s, cx| {
                        s.receive_transcript_frame(
                            zeron_doc::TranscriptFrame::Reset {
                                reset: serde_json::from_value(entries).unwrap(),
                            },
                            cx,
                        )
                        .unwrap();
                        cx.notify();
                    });
                    let transcript = window.update(cx, |shell, _, _| shell.fixture_transcript())?;
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
                        // Rest: every report folded, the sidebar tree folded.
                        transcript.update(cx, |t, cx| t.fixture_activity(&[], cx));
                        window.update(cx, |s, _, cx| s.fixture_sidebar_trees(&[], cx))?;
                        transcript.update(cx, |t, cx| t.fixture_scroll_to_row(9, cx));
                        shot(cx, &format!("activity-rest-{look}"), 700).await?;
                        // Open: the audit's result and the review's question
                        // unfolded; the parent's tree unfolded in the sidebar.
                        transcript.update(cx, |t, cx| {
                            t.fixture_activity(&["x1#activity", "x2#activity"], cx)
                        });
                        window.update(cx, |s, _, cx| s.fixture_sidebar_trees(&[MAIN], cx))?;
                        transcript.update(cx, |t, cx| t.fixture_scroll_to_row(8, cx));
                        shot(cx, &format!("activity-open-{look}"), 700).await?;
                    }
                    std::fs::write(
                        output.join("result.txt"),
                        "PASS: orchestration activity rest/open with sidebar tree, dark frosted and light opaque.\n",
                    )?;
                    Ok(())
                }
                .await;
                if let Err(error) = run {
                    eprintln!("orchestration activity fixture failed: {error:#}");
                }
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    Ok(())
}
