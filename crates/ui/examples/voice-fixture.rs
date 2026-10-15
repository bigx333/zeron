//! Production voice-stage presentation with no engine, transport or microphone.
//! Pass `transmitting` to preview the held-button state; otherwise it is muted.
use gpui::{AppContext, Bounds, WindowBounds, WindowOptions, px, size};
use zeron_ui::*;

fn main() -> anyhow::Result<()> {
    let held = std::env::args().nth(1).as_deref() == Some("transmitting");
    let runtime = tokio::runtime::Runtime::new()?;
    let _guard = runtime.enter();
    let temporary = tempfile::tempdir()?;
    let data = temporary.path().to_path_buf();
    gpui_platform::application()
        .with_assets(icons::Assets)
        .run(move |cx| {
            gpui_tokio::init(cx);
            gpui_base::init(cx);
            let mut saved = settings::UiSettings::default();
            saved.voice_push_to_talk = true;
            saved.sidebar_collapsed = true;
            saved.surface = zeron_theme::SurfacePreference::Opaque;
            settings::init(saved.clone(), data.clone(), cx);
            let fonts = typography::register_fonts(cx);
            typography::init(
                saved.ui_font_family.clone(),
                saved.ui_font_size,
                saved.terminal_font_family.clone(),
                saved.terminal_font_size,
                saved.code_font_family.clone(),
                saved.code_font_size,
                fonts,
                cx,
            );
            theme_library::init(data.clone(), cx);
            appearance::init(
                appearance::AppearanceMode::Dark,
                saved.theme_selection,
                saved.accent,
                saved.surface,
                cx,
            );
            history::init(
                saved.git_history_columns,
                saved.git_history_column_widths,
                saved.git_history_column_order,
                saved.git_history_author_display,
                cx,
            );
            composer::init(cx, saved.composer_send_behavior);
            terminal::panel::init(cx);
            app_menus::init(cx);
            let state = cx.new(|_| {
                let mut state = state::AppState::new();
                state.connection = zeron_proto::view::ConnectionStatus::Ready;
                state.workspace_scope = Some(zeron_proto::WorkspaceScope::Local);
                state.local_device_id = Some("preview".into());
                state.auto_selected = true;
                state.chats_synced = true;
                state.spaces_synced = true;
                state
            });
            let boot = EngineBootConfig {
                data_dir: data,
                ipc_port: 0,
                edge_url: String::new(),
                edge_token: None,
                org_id: None,
                workos_client_id: None,
                default_harness: HarnessId::Mock,
            };
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        gpui::point(px(100.0), px(80.0)),
                        size(px(960.0), px(640.0)),
                    ))),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: None,
                        appears_transparent: true,
                        traffic_light_position: Some(gpui::point(px(14.0), px(14.0))),
                    }),
                    app_owns_titlebar_drag: true,
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|cx| {
                        let mut shell = shell::Shell::new(state.clone(), boot, cx);
                        shell.fixture_voice_stage(held, cx);
                        shell
                    })
                },
            )
            .unwrap();
            cx.activate(true);
        });
    Ok(())
}
