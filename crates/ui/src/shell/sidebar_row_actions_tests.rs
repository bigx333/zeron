//! T3 row hover actions: separate Snooze and Settle buttons, the quick
//! snooze menu, and their isolation from row selection.
use super::*;
use gpui::{AppContext, Modifiers, TestAppContext, VisualTestContext};

struct Host(Entity<Shell>);

impl Render for Host {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.0.update(cx, |shell, cx| {
            let theme = Theme::default();
            div()
                .size_full()
                .capture_key_down(cx.listener(Shell::on_key_down_capture))
                .child(
                    div()
                        .w(px(280.0))
                        .h(px(800.0))
                        .child(shell.render_chat_sidebar(&theme, cx)),
                )
                .children(shell.render_overlays(window.viewport_size(), window, cx))
        })
    }
}

fn setup(cx: &mut TestAppContext) -> (Entity<Shell>, &mut VisualTestContext) {
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        gpui_base::init(cx);
        cx.set_global(Theme::default());
        crate::app_menus::init(cx);
        settings::init(UiSettings::default(), dir.path(), cx);
        crate::history::init(
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            cx,
        );
        apply_keymap(
            cx,
            &KeymapConfig::default(),
            ComposerSendBehavior::default(),
        );
    });
    let (host, cx) = cx.add_window_view(|_, cx| {
        Host(cx.new(|cx| {
            let state = cx.new(|_| AppState::new());
            let mut shell = Shell::new(
                state,
                EngineBootConfig {
                    data_dir: dir.path().into(),
                    ipc_port: 0,
                    edge_url: "http://127.0.0.1:1".into(),
                    edge_token: None,
                    org_id: None,
                    workos_client_id: None,
                    default_harness: zeron_proto::HarnessId::Mock,
                },
                cx,
            );
            shell.settings.sidebar_organization = SidebarOrganization::InOneList;
            shell.settings.sidebar_mode = SidebarMode::T3;
            shell.reduced_motion = true;
            shell.state.update(cx, |state, _| {
                state.workspace_scope = Some(WorkspaceScope::Local);
                state.local_device_id = Some("local".into());
                state.no_project = true;
                state.chats = ["a", "b"]
                    .into_iter()
                    .enumerate()
                    .map(|(ix, id)| {
                        serde_json::from_value(serde_json::json!({
                            "id": id, "title": id, "deviceId": "local", "archived": false,
                            "createdAt": Utc::now() - chrono::Duration::minutes(ix as i64),
                        }))
                        .unwrap()
                    })
                    .collect();
            });
            shell
        }))
    });
    let shell = host.read_with(cx, |host, _| host.0.clone());
    redraw(cx);
    (shell, cx)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear();
    });
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx.debug_bounds(selector).unwrap();
    cx.simulate_click(bounds.center(), Modifiers::default());
}

fn hover(cx: &mut VisualTestContext, selector: &'static str) {
    let row = cx.debug_bounds(selector).unwrap();
    cx.simulate_mouse_move(row.center(), None, Modifiers::default());
    redraw(cx);
}

/// Let a closing menu finish its exit, so the next press opens fresh. The
/// reap compares wall-clock `Instant`s, which the test clock does not move,
/// so the finished exit is applied directly once it has begun.
fn finish_close(shell: &Entity<Shell>, cx: &mut VisualTestContext) {
    cx.run_until_parked();
    shell.update(cx, |shell, cx| {
        assert!(shell.row_snooze_menu.as_open().is_none());
        shell.row_snooze_menu = popover::Popup::default();
        cx.notify();
    });
    redraw(cx);
}

fn selected(shell: &Entity<Shell>, cx: &mut VisualTestContext) -> Option<String> {
    shell.read_with(cx, |shell, cx| shell.state.read(cx).selected_chat.clone())
}

fn snooze_menu(shell: &Entity<Shell>, cx: &mut VisualTestContext) -> Option<String> {
    shell.read_with(cx, |shell, _| {
        shell
            .row_snooze_menu
            .as_open()
            .map(|menu| menu.chat_id.clone())
    })
}

#[gpui::test]
fn custom_snooze_types_inline_in_the_card_without_selecting(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let before = selected(&shell, cx);
    hover(cx, "chat-a");
    click(cx, "chat-a-snooze");
    redraw(cx);
    // Four presets, then wake times (tomorrow morning is always offered).
    assert!(cx.debug_bounds("row-snooze-a-4").is_none());
    assert!(cx.debug_bounds("row-snooze-a-until-0").is_some());

    // "Custom…" turns into an empty, focused field in place: the card stays
    // open, with a Snooze button and a caption saying what to type.
    click(cx, "row-snooze-a-custom");
    redraw(cx);
    assert_eq!(
        shell.read_with(cx, |shell, cx| shell.custom_snooze_text(cx)),
        Some(String::new())
    );
    assert_eq!(snooze_menu(&shell, cx).as_deref(), Some("a"));
    for part in [
        "custom-snooze-amount",
        "custom-snooze-confirm",
        "custom-snooze-hint",
    ] {
        assert!(cx.debug_bounds(part).is_some(), "{part}");
    }
    assert_eq!(selected(&shell, cx), before);
    let no_snooze = |cx: &mut VisualTestContext| {
        assert!(shell.read_with(cx, |shell, _| shell.custom_snooze.is_some()));
        assert!(shell.read_with(cx, |shell, _| shell.sidebar_notice.is_none()));
    };

    // Empty: the Snooze button does nothing.
    click(cx, "custom-snooze-confirm");
    redraw(cx);
    no_snooze(cx);

    // Typing lands in the field (it took focus); unreadable text does
    // nothing on Enter or on the button.
    cx.simulate_input("abc");
    redraw(cx);
    assert_eq!(
        shell
            .read_with(cx, |shell, cx| shell.custom_snooze_text(cx))
            .as_deref(),
        Some("abc")
    );
    cx.simulate_keystrokes("enter");
    redraw(cx);
    no_snooze(cx);
    click(cx, "custom-snooze-confirm");
    redraw(cx);
    no_snooze(cx);

    // Clicking back into the field keeps editing; a duration arms the
    // button, and a click on it snoozes (no engine here) and closes the card.
    click(cx, "custom-snooze-amount");
    cx.simulate_keystrokes("backspace backspace backspace");
    cx.simulate_input("45m");
    redraw(cx);
    click(cx, "custom-snooze-confirm");
    redraw(cx);
    assert!(shell.read_with(cx, |shell, _| shell.custom_snooze.is_none()));
    assert_eq!(snooze_menu(&shell, cx), None);
    assert_eq!(
        shell
            .read_with(cx, |shell, _| shell.sidebar_notice.clone())
            .as_deref(),
        Some("Engine not connected")
    );
    assert_eq!(selected(&shell, cx), before);
    finish_close(&shell, cx);

    // Keyboard alone: type and press Enter.
    shell.update(cx, |shell, _| shell.sidebar_notice = None);
    hover(cx, "chat-a");
    click(cx, "chat-a-snooze");
    redraw(cx);
    click(cx, "row-snooze-a-custom");
    redraw(cx);
    cx.simulate_input("1h30m");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(shell.read_with(cx, |shell, _| shell.custom_snooze.is_none()));
    assert_eq!(
        shell
            .read_with(cx, |shell, _| shell.sidebar_notice.clone())
            .as_deref(),
        Some("Engine not connected")
    );
    finish_close(&shell, cx);

    // Escape leaves the field first; the field also goes with its card.
    hover(cx, "chat-a");
    click(cx, "chat-a-snooze");
    redraw(cx);
    click(cx, "row-snooze-a-custom");
    redraw(cx);
    assert!(shell.update(cx, |shell, cx| shell.capture_escape_surface(cx)));
    assert!(shell.read_with(cx, |shell, _| shell.custom_snooze.is_none()));
    assert_eq!(snooze_menu(&shell, cx).as_deref(), Some("a"));
    click(cx, "row-snooze-a-custom");
    redraw(cx);
    shell.update(cx, |shell, cx| shell.close_row_snooze_menu(cx));
    redraw(cx);
    assert!(shell.read_with(cx, |shell, _| shell.custom_snooze.is_none()));
    cx.run_until_parked();
}

#[gpui::test]
fn escape_dismisses_custom_editor_then_either_snooze_menu(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let before = selected(&shell, cx);
    hover(cx, "chat-a");
    click(cx, "chat-a-snooze");
    redraw(cx);
    click(cx, "row-snooze-a-custom");
    redraw(cx);
    cx.simulate_input("45m");
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert!(shell.read_with(cx, |shell, _| shell.custom_snooze.is_none()));
    assert_eq!(snooze_menu(&shell, cx).as_deref(), Some("a"));
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(snooze_menu(&shell, cx), None);
    finish_close(&shell, cx);

    // The same editor is also entered through the context menu's Snooze card.
    shell.update(cx, |shell, cx| {
        shell.chat_menu.open(ChatMenuState {
            chat_id: "a".into(),
            tab: None,
            position: gpui::point(px(280.0), px(100.0)),
            page: ChatMenuPage::Snooze,
        });
        cx.notify();
    });
    redraw(cx);
    click(cx, "chat-snooze-a-custom");
    redraw(cx);
    cx.simulate_input("1h30m");
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert!(shell.read_with(cx, |shell, _| shell.custom_snooze.is_none()));
    assert!(shell.read_with(cx, |shell, _| shell.chat_menu.is_open()));
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert!(!shell.read_with(cx, |shell, _| shell.chat_menu.is_open()));
    assert_eq!(selected(&shell, cx), before);
    cx.run_until_parked();
}

#[gpui::test]
fn hover_actions_snooze_and_settle_without_selecting_the_row(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);
    let before = selected(&shell, cx);
    let row = cx.debug_bounds("chat-a").unwrap();
    assert!(cx.debug_bounds("chat-a-snooze").is_none());

    // Hover reveals both buttons on that row only, at a stable row size.
    hover(cx, "chat-a");
    let snooze = cx.debug_bounds("chat-a-snooze").unwrap();
    assert!(cx.debug_bounds("chat-a-settle").is_some());
    assert!(cx.debug_bounds("chat-b-snooze").is_none());
    assert_eq!(cx.debug_bounds("chat-a").unwrap().size, row.size);

    // The clock opens the quick menu beside itself; the row stays unselected.
    cx.simulate_click(snooze.center(), Modifiers::default());
    redraw(cx);
    assert_eq!(snooze_menu(&shell, cx).as_deref(), Some("a"));
    assert_eq!(selected(&shell, cx), before);
    let position = shell.read_with(cx, |shell, _| shell.row_snooze_menu.get().unwrap().position);
    assert!(position.x >= snooze.right());

    // Moving onto the menu keeps the row's buttons up.
    let preset = cx.debug_bounds("row-snooze-a-0").unwrap();
    cx.simulate_mouse_move(preset.center(), None, Modifiers::default());
    redraw(cx);
    assert!(cx.debug_bounds("chat-a-snooze").is_some());

    // Escape dismisses it before anything can read the key as an interrupt.
    assert!(shell.update(cx, |shell, cx| shell.capture_escape_surface(cx)));
    assert_eq!(snooze_menu(&shell, cx), None);
    finish_close(&shell, cx);

    // A second press on the same clock closes rather than reopening.
    hover(cx, "chat-a");
    let snooze = cx.debug_bounds("chat-a-snooze").unwrap();
    cx.simulate_click(snooze.center(), Modifiers::default());
    redraw(cx);
    assert_eq!(snooze_menu(&shell, cx).as_deref(), Some("a"));
    cx.simulate_click(snooze.center(), Modifiers::default());
    redraw(cx);
    assert_eq!(snooze_menu(&shell, cx), None);
    finish_close(&shell, cx);

    // A preset reaches the snooze write (no engine here) and closes the menu.
    hover(cx, "chat-a");
    let snooze = cx.debug_bounds("chat-a-snooze").unwrap();
    cx.simulate_click(snooze.center(), Modifiers::default());
    redraw(cx);
    let preset = cx.debug_bounds("row-snooze-a-1").unwrap();
    cx.simulate_click(preset.center(), Modifiers::default());
    redraw(cx);
    assert_eq!(snooze_menu(&shell, cx), None);
    assert_eq!(
        shell
            .read_with(cx, |shell, _| shell.sidebar_notice.clone())
            .as_deref(),
        Some("Engine not connected")
    );
    finish_close(&shell, cx);

    // Settle acts directly, still without selecting the row.
    shell.update(cx, |shell, _| shell.sidebar_notice = None);
    hover(cx, "chat-b");
    let settle = cx.debug_bounds("chat-b-settle").unwrap();
    cx.simulate_click(settle.center(), Modifiers::default());
    assert_eq!(
        shell
            .read_with(cx, |shell, _| shell.sidebar_notice.clone())
            .as_deref(),
        Some("Engine not connected")
    );
    assert_eq!(selected(&shell, cx), before);
    cx.run_until_parked();
}

#[gpui::test]
fn hover_actions_follow_lane_layout_and_mode(cx: &mut TestAppContext) {
    let (shell, cx) = setup(cx);

    // Compact rows fit both buttons in the time slot without resizing it.
    shell.update(cx, |shell, cx| {
        shell.settings.sidebar_compact = true;
        cx.notify();
    });
    redraw(cx);
    let slot = cx.debug_bounds("chat-time-a").unwrap();
    hover(cx, "chat-a");
    assert_eq!(cx.debug_bounds("chat-time-a").unwrap().size, slot.size);
    assert!(cx.debug_bounds("chat-a-snooze").is_some());
    assert!(cx.debug_bounds("chat-a-settle").is_some());

    // Snoozed rows can be re-timed or settled, with no separate Wake control.
    shell.update(cx, |shell, cx| {
        shell.settings.sidebar_compact = false;
        shell.t3_snoozed_open = true;
        shell.state.update(cx, |state, _| {
            state.chats[1].snoozed_until = Some(Utc::now() + chrono::Duration::hours(1));
        });
        cx.notify();
    });
    redraw(cx);
    hover(cx, "chat-b");
    assert!(cx.debug_bounds("chat-b-wake").is_none());
    assert!(cx.debug_bounds("chat-b-settle").is_some());
    assert!(cx.debug_bounds("chat-b-snooze").is_some());

    // Default mode keeps its single archive control.
    shell.update(cx, |shell, cx| {
        shell.settings.sidebar_mode = SidebarMode::Default;
        cx.notify();
    });
    redraw(cx);
    hover(cx, "chat-a");
    assert!(cx.debug_bounds("chat-a-snooze").is_none());
    assert!(cx.debug_bounds("chat-a-settle").is_none());
    assert!(cx.debug_bounds("chat-a-corner").is_some());
    cx.run_until_parked();
}
