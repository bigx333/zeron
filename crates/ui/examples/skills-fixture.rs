//! Native review fixture for the Skills page. Attaches to an engine already
//! serving on `ZERON_SKILLS_IPC_PORT` (start one with a throwaway `HOME` so
//! its `~/.zeron/skills` is disposable), opens the Skills route, and with
//! ZERON_SKILLS_CAPTURE=<dir> saves the installed list, a skill's detail, the
//! Remove dialog, skills.sh's popular list, a search and a registry preview,
//! then quits. ZERON_PALETTE_LIGHT renders the light theme.
use gpui::{AppContext, Bounds, WindowBounds, WindowOptions, px, size};
use zeron_ui::*;

fn main() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let _guard = runtime.enter();
    tracing_subscriber::fmt().with_env_filter("warn").init();
    let temp = tempfile::tempdir()?;
    let data = temp.path().to_path_buf();
    let ipc_port: u16 = std::env::var("ZERON_SKILLS_IPC_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(27933);
    // SAFETY: set before any thread reads the environment.
    unsafe { std::env::set_var("ZERON_OPEN_ROUTE", "skills") };
    let boot = state::EngineBootConfig {
        data_dir: data.clone(),
        ipc_port,
        edge_url: String::new(),
        edge_token: None,
        org_id: None,
        workos_client_id: None,
        default_harness: zeron_proto::HarnessId::ClaudeCode,
    };
    // Attaches to the engine serving on the port (or embeds one, whose
    // library is this process's own `HOME`).
    let handle = runtime.block_on(state::EngineHandle::bootstrap(boot.clone()))?;
    gpui_platform::application()
        .with_assets(icons::Assets)
        .run(move |cx| {
            gpui_tokio::init(cx);
            gpui_base::init(cx);
            let mut settings = settings::UiSettings::default();
            settings.sidebar_width = 280.0;
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
                if std::env::var_os("ZERON_PALETTE_LIGHT").is_some() {
                    appearance::AppearanceMode::Light
                } else {
                    appearance::AppearanceMode::Dark
                },
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
            let state = cx.new(|_| {
                let mut s = state::AppState::new();
                s.fixture_attachment_engine(handle);
                s.connection = zeron_proto::view::ConnectionStatus::Ready;
                s.workspace_scope = Some(zeron_proto::WorkspaceScope::Local);
                s.local_device_id = Some("local".into());
                s.chats_synced = true;
                s.spaces_synced = true;
                s.no_project = true;
                s
            });
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            gpui::point(px(12.), px(30.)),
                            size(px(1180.), px(820.)),
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
            let Some(directory) =
                std::env::var_os("ZERON_SKILLS_CAPTURE").map(std::path::PathBuf::from)
            else {
                return;
            };
            cx.spawn(async move |cx| {
                let run: anyhow::Result<()> = async {
                    std::fs::create_dir_all(&directory)?;
                    let prefix = if std::env::var_os("ZERON_PALETTE_LIGHT").is_some() {
                        "light"
                    } else {
                        "dark"
                    };
                    let wait = async |cx: &mut gpui::AsyncApp, ms: u64| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(ms))
                            .await
                    };
                    let shot = async |cx: &mut gpui::AsyncApp, name: &str, ms: u64| {
                        wait(cx, ms).await;
                        // Untyped: a typed update holds the Shell lease draw() needs.
                        let capture: gpui::AnyWindowHandle = window.into();
                        capture.update(cx, |_, w, cx| {
                            w.draw(cx).clear();
                            w.render_to_image()?
                                .save(directory.join(format!("{prefix}-{name}.png")))?;
                            anyhow::Ok(())
                        })?
                    };
                    let page = window.update(cx, |shell, _, cx| shell.fixture_skills_page(cx))?;
                    let drive = |cx: &mut gpui::AsyncApp,
                                 f: &dyn Fn(
                        &mut skills::SkillsPage,
                        &mut gpui::Window,
                        &mut gpui::Context<skills::SkillsPage>,
                    )| {
                        let page = page.clone();
                        window.update(cx, |_, w, cx| page.update(cx, |page, cx| f(page, w, cx)))
                    };
                    shot(cx, "01-installed", 2500).await?;
                    if std::env::var_os("ZERON_SKILLS_PACKS").is_some() {
                        drive(cx, &|page, _, cx| {
                            page.fixture_expand_pack("pstack-claude", true, cx)
                        })?;
                        shot(cx, "p1-pack-notes", 900).await?;
                        drive(cx, &|page, _, cx| {
                            page.fixture_collapse_packs(cx);
                            page.fixture_expand_pack("pstack-claude", false, cx)
                        })?;
                        shot(cx, "p2-pack-expanded", 900).await?;
                        drive(cx, &|page, _, cx| {
                            page.fixture_collapse_packs(cx);
                            page.fixture_search("thermo", cx)
                        })?;
                        shot(cx, "p3-pack-search", 900).await?;
                        drive(cx, &|page, w, cx| {
                            page.fixture_search("", cx);
                            page.fixture_add_pack("anthropics/skills", w, cx)
                        })?;
                        shot(cx, "p4-add-inspecting", 400).await?;
                        shot(cx, "p5-add-ready", 12000).await?;
                        drive(cx, &|page, _, cx| page.fixture_dismiss(cx))?;
                    }
                    drive(cx, &|page, w, cx| page.fixture_open(3, w, cx))?;
                    shot(cx, "02-installed-detail", 1200).await?;
                    drive(cx, &|page, _, cx| {
                        page.fixture_dismiss(cx);
                        page.fixture_ask_remove(4, cx)
                    })?;
                    shot(cx, "03-remove-dialog", 900).await?;
                    drive(cx, &|page, _, cx| {
                        page.fixture_dismiss(cx);
                        page.fixture_tab(true, cx)
                    })?;
                    shot(cx, "04-registry-popular", 4000).await?;
                    drive(cx, &|page, _, cx| page.fixture_search("react", cx))?;
                    shot(cx, "05-registry-search", 4000).await?;
                    drive(cx, &|page, w, cx| page.fixture_open(0, w, cx))?;
                    shot(cx, "06-registry-detail", 4000).await?;
                    drive(cx, &|page, _, cx| {
                        page.fixture_tab(false, cx);
                        page.fixture_search("zzz-nothing", cx)
                    })?;
                    shot(cx, "07-installed-no-match", 900).await?;
                    Ok(())
                }
                .await;
                if let Err(error) = run {
                    eprintln!("skills capture failed: {error:#}");
                }
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    Ok(())
}
