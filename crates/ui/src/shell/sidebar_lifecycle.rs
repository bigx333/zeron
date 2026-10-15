//! Durable sidebar parking and the timed snooze menu.
use super::*;

const SNOOZE_PRESETS: [(&str, u64); 4] = [
    ("For 15 minutes", 15 * 60),
    ("For 1 hour", 60 * 60),
    ("For 4 hours", 4 * 60 * 60),
    ("For 1 day", 24 * 60 * 60),
];

/// The inline "Custom…" field of an open snooze card: a duration typed
/// as "90m", "3h", "1h30m" or "2d". It lives only while that card is open.
pub(super) struct CustomSnoozeDialog {
    chat_id: String,
    input: Entity<ComposerInput>,
    focus_pending: bool,
    _events: Subscription,
}

/// A typed snooze duration in whole seconds: a sum of `<n><unit>` parts
/// (m/min, h/hr, d/day, w/wk, with plurals) where a bare number means
/// minutes. None when empty, zero, malformed or past `u32` seconds.
fn parse_snooze_duration(text: &str) -> Option<u32> {
    let text = text.trim().to_ascii_lowercase();
    let mut rest = text.as_str();
    let mut total: u32 = 0;
    while !rest.is_empty() {
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 {
            return None;
        }
        let amount: u32 = rest[..digits].parse().ok()?;
        rest = rest[digits..].trim_start();
        let letters = rest.len()
            - rest
                .trim_start_matches(|c: char| c.is_ascii_alphabetic())
                .len();
        let unit = match &rest[..letters] {
            "" | "m" | "min" | "mins" | "minute" | "minutes" => 60,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3_600,
            "d" | "day" | "days" => 86_400,
            "w" | "wk" | "wks" | "week" | "weeks" => 604_800,
            _ => return None,
        };
        total = total.checked_add(amount.checked_mul(unit)?)?;
        rest = rest[letters..].trim_start_matches([' ', ',']);
    }
    (total > 0).then_some(total)
}

/// Contextual wake times past the presets: this evening (while more than an
/// hour away), tomorrow morning, and next Monday morning unless that is
/// tomorrow.
fn snooze_until_suggestions<Tz: chrono::TimeZone>(
    now: &chrono::DateTime<Tz>,
) -> Vec<(&'static str, chrono::DateTime<Tz>)> {
    use chrono::{Datelike as _, Days};
    let at = |date: chrono::NaiveDate, hour: u32| {
        now.timezone()
            .from_local_datetime(&date.and_hms_opt(hour, 0, 0)?)
            .earliest()
    };
    let today = now.date_naive();
    let mut out = Vec::new();
    if let Some(evening) = at(today, 18)
        && evening.clone().signed_duration_since(now.clone()) > chrono::TimeDelta::hours(1)
    {
        out.push(("This evening", evening));
    }
    let Some(tomorrow) = today.checked_add_days(Days::new(1)) else {
        return out;
    };
    if let Some(morning) = at(tomorrow, 9) {
        out.push(("Tomorrow morning", morning));
    }
    let to_monday = (7 - tomorrow.weekday().num_days_from_monday()) % 7;
    if to_monday > 0
        && let Some(monday) = tomorrow.checked_add_days(Days::new(u64::from(to_monday)))
        && let Some(next_week) = at(monday, 9)
    {
        out.push(("Next week", next_week));
    }
    out
}

/// The core's duration for waking at `at`: whole minutes rounded up, at
/// least one; None once `at` has passed or past `u32` seconds.
fn snooze_seconds_until<Tz: chrono::TimeZone>(
    at: &chrono::DateTime<Tz>,
    now: &chrono::DateTime<Tz>,
) -> Option<u32> {
    let span = at
        .clone()
        .signed_duration_since(now.clone())
        .num_milliseconds();
    if span <= 0 {
        return None;
    }
    let minutes = span.checked_add(59_999)? / 60_000;
    u32::try_from(minutes.checked_mul(60)?).ok()
}

/// "6:00 PM" today, "Thu 9:00 AM" this week, "Oct 24, 9:00 AM" beyond,
/// with the year once it differs.
fn snooze_wake_label<Tz: chrono::TimeZone>(
    at: &chrono::DateTime<Tz>,
    now: &chrono::DateTime<Tz>,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    use chrono::Datelike as _;
    let days = (at.date_naive() - now.date_naive()).num_days();
    let format = match days {
        _ if at.year() != now.year() => "%b %-d, %Y, %-I:%M %p",
        0 => "%-I:%M %p",
        1..=6 => "%a %-I:%M %p",
        _ => "%b %-d, %-I:%M %p",
    };
    at.format(format).to_string()
}

/// A row action's hue, pulled toward the text color until it clears the 3:1
/// non-text floor on the sidebar under its own hover wash — the raw status
/// colors fall short on some light themes.
fn legible_row_action_tint(tint: gpui::Hsla, wash: gpui::Hsla, theme: &Theme) -> gpui::Hsla {
    let backdrop = crate::theme::flatten(wash, theme.surface);
    (0..=10)
        .map(|step| crate::theme::mix(tint, theme.text, step as f32 / 10.0))
        .find(|color| crate::theme::contrast_ratio(*color, backdrop) >= 3.0)
        .unwrap_or(theme.text)
}

/// The quick snooze menu a T3 row's clock button opens beside itself.
#[derive(Clone)]
pub(super) struct RowSnoozeMenu {
    pub(super) chat_id: String,
    pub(super) position: Point<Pixels>,
}

/// Hover action buttons: 18pt targets in the regular corner; compact rows
/// fit two 15pt buttons in their 30pt time slot so the title never shifts.
const ROW_ACTION_SIZE: f32 = 18.0;
const COMPACT_ROW_ACTION_SIZE: f32 = 15.0;

impl Shell {
    /// Swap the card's "Custom…" row for its inline duration field, keeping
    /// the card open.
    fn open_custom_snooze(&mut self, chat_id: String, cx: &mut Context<Self>) {
        let input = cx.new(|cx| {
            ComposerInput::new("e.g. 90m", cx).with_accessibility_role(gpui::Role::TextInput)
        });
        let events = cx.subscribe(&input, |this: &mut Shell, _, event, cx| {
            if matches!(event, ComposerInputEvent::Submitted) {
                this.submit_custom_snooze(cx);
            } else {
                cx.notify();
            }
        });
        self.custom_snooze = Some(CustomSnoozeDialog {
            chat_id,
            input,
            focus_pending: true,
            _events: events,
        });
        cx.notify();
    }

    /// What the inline custom field currently holds, while it is open.
    #[cfg(test)]
    pub(super) fn custom_snooze_text(&self, cx: &App) -> Option<String> {
        self.custom_snooze
            .as_ref()
            .map(|dialog| dialog.input.read(cx).text().to_owned())
    }

    fn submit_custom_snooze(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.custom_snooze.as_ref() else {
            return;
        };
        let Some(seconds) = parse_snooze_duration(dialog.input.read(cx).text()) else {
            return;
        };
        let chat_id = dialog.chat_id.clone();
        self.custom_snooze = None;
        self.chat_hover_resync = true;
        self.snooze_chat(
            chat_id,
            std::time::Duration::from_secs(u64::from(seconds)),
            cx,
        );
        cx.notify();
    }

    /// The inline field draws inside its snooze card, so this only focuses
    /// it once mounted and drops it when its card is gone.
    pub(super) fn render_custom_snooze(
        &mut self,
        _viewport: gpui::Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let chat_id = self.custom_snooze.as_ref()?.chat_id.clone();
        let card_open =
            self.row_snooze_menu
                .as_open()
                .is_some_and(|menu| menu.chat_id == chat_id)
                || self.chat_menu.as_open().is_some_and(|menu| {
                    menu.chat_id == chat_id && menu.page == ChatMenuPage::Snooze
                });
        if !card_open {
            self.custom_snooze = None;
            return None;
        }
        let dialog = self.custom_snooze.as_mut()?;
        if std::mem::take(&mut dialog.focus_pending) {
            window.focus(&dialog.input.focus_handle(cx), cx);
        }
        None
    }

    #[cfg(feature = "project-palette-fixture")]
    pub fn fixture_snooze_menu(&mut self, open: bool, cx: &mut Context<Self>) {
        if open {
            self.chat_menu.open(ChatMenuState {
                chat_id: "chat-3".into(),
                tab: None,
                position: gpui::point(px(260.0), px(240.0)),
                page: ChatMenuPage::Snooze,
            });
        } else {
            self.close_chat_menu(cx);
        }
        cx.notify();
    }

    /// Open a row's quick snooze card with its inline "Custom…" field
    /// showing `text`, for the native capture tour.
    #[cfg(feature = "project-palette-fixture")]
    pub fn fixture_custom_snooze(
        &mut self,
        chat_id: &str,
        position: Point<Pixels>,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        self.row_snooze_menu.open(RowSnoozeMenu {
            chat_id: chat_id.into(),
            position,
        });
        self.open_custom_snooze(chat_id.into(), cx);
        if let Some(input) = self
            .custom_snooze
            .as_ref()
            .map(|dialog| dialog.input.clone())
        {
            input.update(cx, |input, cx| input.set_text(text, cx));
        }
        cx.notify();
    }

    /// Pin a sidebar row's hover (wash + T3 actions) without a pointer;
    /// `None` releases it.
    #[cfg(feature = "project-palette-fixture")]
    pub fn fixture_sidebar_row_hover(&mut self, chat_id: Option<&str>, cx: &mut Context<Self>) {
        if let Some(previous) = self.chat_status_hover.take() {
            motion::set_hover(&format!("{previous}-hover"), false, self.reduced_motion);
        }
        if let Some(chat_id) = chat_id {
            let row_id = format!("chat-{chat_id}");
            motion::set_hover(&format!("{row_id}-hover"), true, self.reduced_motion);
            self.chat_status_hover = Some(row_id);
        }
        cx.notify();
    }

    /// Open a row's quick snooze menu at `position` (window coordinates).
    #[cfg(feature = "project-palette-fixture")]
    pub fn fixture_sidebar_row_snooze(
        &mut self,
        chat_id: &str,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.row_snooze_menu.open(RowSnoozeMenu {
            chat_id: chat_id.into(),
            position,
        });
        cx.notify();
    }

    pub(super) fn settle_chat(&mut self, chat_id: String, cx: &mut Context<Self>) {
        self.park_sidebar_chat(
            chat_id.clone(),
            serde_json::json!({"op": "setChatArchived", "chatId": chat_id, "archived": true}),
            cx,
        );
    }

    pub(super) fn snooze_chat(
        &mut self,
        chat_id: String,
        duration: std::time::Duration,
        cx: &mut Context<Self>,
    ) {
        let Ok(duration) = chrono::TimeDelta::from_std(duration) else {
            return;
        };
        let Some(until) = Utc::now().checked_add_signed(duration) else {
            return;
        };
        self.park_sidebar_chat(
            chat_id.clone(),
            serde_json::json!({"op": "setChatSnoozedUntil", "chatId": chat_id, "until": until}),
            cx,
        );
    }

    /// Move forward only after a successful write and only if the user is
    /// still reading the parked chat. A slow RPC must never steal navigation.
    fn park_sidebar_chat(
        &mut self,
        chat_id: String,
        params: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        self.close_chat_menu(cx);
        self.close_row_snooze_menu(cx);
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.show_notice(crate::toast::ToastKind::Error, "Engine not connected", cx);
            cx.notify();
            return;
        };
        let now = Utc::now();
        let order = self.sidebar_visible_order(cx);
        let (next, project) = {
            let state = self.state.read(cx);
            let eligible = |id: &&String| {
                id.as_str() != chat_id
                    && state.chats.iter().any(|chat| {
                        &chat.id == *id
                            && !chat.archived
                            && !chat.is_snoozed(now)
                            && state.display_status_for(chat, now)
                                != zeron_proto::ChatIndicator::Working
                    })
            };
            let start = order
                .iter()
                .position(|id| id == &chat_id)
                .map_or(0, |at| at + 1);
            let next = order
                .iter()
                .skip(start)
                .chain(order.iter().take(start))
                .find(eligible)
                .cloned();
            let project = state
                .chats
                .iter()
                .find(|chat| chat.id == chat_id)
                .and_then(|chat| chat.space_id.clone());
            (next, project)
        };
        cx.spawn(async move |this, cx| {
            let result = engine.client().call(methods::MUTATE, params).await;
            this.update(cx, |shell, cx| {
                if let Err(err) = result {
                    shell.show_notice(crate::toast::ToastKind::Error, format!("{err}"), cx);
                } else if shell.settings.sidebar_mode == SidebarMode::T3
                    && shell.state.read(cx).selected_chat.as_deref() == Some(&chat_id)
                {
                    let next = next.filter(|id| {
                        shell.state.read(cx).chats.iter().any(|chat| {
                            &chat.id == id
                                && !chat.archived
                                && !chat.is_snoozed(Utc::now())
                                && shell.state.read(cx).display_status_for(chat, Utc::now())
                                    != zeron_proto::ChatIndicator::Working
                        })
                    });
                    if let Some(next) = next {
                        shell.open_chat(next, cx);
                    } else {
                        shell.open_new_session(project, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn render_chat_snooze_submenu(
        &mut self,
        chat_id: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity().downgrade();
        self.snooze_preset_card("chat-snooze", chat_id, theme, cx)
            .child(
                gpui::canvas(
                    move |bounds, _, cx| {
                        let _ = entity.update(cx, |this, _| this.chat_copy_bounds = Some(bounds));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .into_any_element()
    }

    /// The snooze presets as a menu card, shared by the context menu's
    /// Snooze submenu and a row's quick snooze menu. `scope` namespaces
    /// element ids so the two never collide.
    fn snooze_preset_card(
        &mut self,
        scope: &'static str,
        chat_id: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        popover::popover_card(theme)
            .w(px(248.0))
            .relative()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .children(
                SNOOZE_PRESETS
                    .into_iter()
                    .enumerate()
                    .map(|(index, (label, seconds))| {
                        let id = chat_id.to_owned();
                        let key = format!("{scope}-{chat_id}-{index}");
                        popover::menu_row(theme, false, key.clone())
                            .id(SharedString::from(format!("{scope}-preset-{index}")))
                            .debug_selector(move || key.clone())
                            .aria_label(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.chat_hover_resync = true;
                                this.snooze_chat(
                                    id.clone(),
                                    std::time::Duration::from_secs(seconds),
                                    cx,
                                );
                            }))
                            .child(
                                icon(icons::CLOCK_CIRCLE)
                                    .size(px(16.0))
                                    .text_color(theme.text_muted),
                            )
                            .child(label)
                    }),
            )
            .child(popover::menu_separator())
            .children({
                // Past the presets the question is when it comes back.
                let now = chrono::Local::now();
                snooze_until_suggestions(&now)
                    .into_iter()
                    .enumerate()
                    .map(|(index, (label, at))| {
                        let id = chat_id.to_owned();
                        let key = format!("{scope}-{chat_id}-until-{index}");
                        let wake = snooze_wake_label(&at, &now);
                        popover::menu_row(theme, false, key.clone())
                            .id(SharedString::from(format!("{scope}-until-{index}")))
                            .debug_selector(move || key.clone())
                            .aria_label(SharedString::from(format!("{label}, {wake}")))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let Some(seconds) =
                                    snooze_seconds_until(&at, &chrono::Local::now())
                                else {
                                    return;
                                };
                                this.chat_hover_resync = true;
                                this.snooze_chat(
                                    id.clone(),
                                    std::time::Duration::from_secs(u64::from(seconds)),
                                    cx,
                                );
                            }))
                            .child(
                                icon(icons::MOON)
                                    .size(px(16.0))
                                    .text_color(theme.text_muted),
                            )
                            .child(div().flex_1().child(label))
                            .child(
                                div()
                                    .text_size(crate::typography::ui_rems(12.0))
                                    .text_color(theme.text_muted)
                                    .child(wake),
                            )
                    })
                    .collect::<Vec<_>>()
            })
            .child(self.snooze_custom_row(scope, chat_id, theme, cx))
    }

    /// "Custom…" until clicked; then, in its place, a small editor that
    /// reads as one: an outlined text field with a caret, a Snooze button
    /// that lights up once the text is a duration, and a caption that says
    /// what to type, what is wrong, or exactly when the session wakes.
    /// Enter or the button confirms; Escape leaves the field.
    fn snooze_custom_row(
        &self,
        scope: &'static str,
        chat_id: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = format!("{scope}-{chat_id}-custom");
        let editing = self
            .custom_snooze
            .as_ref()
            .filter(|dialog| dialog.chat_id == chat_id);
        let Some(dialog) = editing else {
            let id = chat_id.to_owned();
            return popover::menu_row(theme, false, key.clone())
                .id(SharedString::from(format!("{scope}-custom")))
                .debug_selector(move || key.clone())
                .aria_label("Custom snooze duration")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.open_custom_snooze(id.clone(), cx);
                }))
                .child(icon(icons::PEN).size(px(16.0)).text_color(theme.text_muted))
                .child(div().flex_1().child("Custom…"))
                // Says, before the click, that this row takes typing.
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_muted.opacity(0.7))
                        .child("90m, 3h…"),
                )
                .into_any_element();
        };
        let text = dialog.input.read(cx).text().trim().to_owned();
        // A press anywhere in the editor (padding, a disabled button) keeps
        // the caret in the field instead of dropping focus.
        let keep_focus = || {
            cx.listener(|this: &mut Shell, _: &gpui::MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if let Some(dialog) = &this.custom_snooze {
                    window.focus(&dialog.input.focus_handle(cx), cx);
                }
            })
        };
        let now = chrono::Local::now();
        let seconds = parse_snooze_duration(&text);
        let wake = seconds
            .and_then(|seconds| {
                now.checked_add_signed(chrono::TimeDelta::seconds(i64::from(seconds)))
            })
            .map(|at| snooze_wake_label(&at, &now));
        let valid = wake.is_some();
        let (caption, caption_color): (SharedString, _) = match &wake {
            Some(wake) => (format!("Wakes {wake}").into(), theme.text_muted),
            None if text.is_empty() => (
                "Minutes, hours or days: 45m, 3h, 1d".into(),
                theme.text_muted.opacity(0.8),
            ),
            None => ("Not a duration yet. Try 45m or 1h30m".into(), theme.danger),
        };
        let field = div()
            .id("custom-snooze-amount")
            .debug_selector(|| "custom-snooze-amount".to_string())
            .flex_1()
            .min_w_0()
            .h(px(28.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .rounded(px(6.0))
            .border_1()
            // Always focused while shown: the ring says "type here".
            .border_color(if !valid && !text.is_empty() {
                theme.danger.opacity(0.7)
            } else {
                theme.accent.opacity(0.7)
            })
            .bg(crate::theme::ink(0.04))
            .text_size(crate::typography::ui_rems(13.0))
            .cursor_text()
            .on_mouse_down(MouseButton::Left, keep_focus())
            .child(div().w_full().child(dialog.input.clone()));
        let confirm = div()
            .id("custom-snooze-confirm")
            .debug_selector(|| "custom-snooze-confirm".to_string())
            .flex_none()
            .h(px(28.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .rounded(px(6.0))
            .text_size(crate::typography::ui_rems(12.0))
            .font_weight(gpui::FontWeight::MEDIUM)
            .role(gpui::Role::Button)
            .on_mouse_down(MouseButton::Left, keep_focus())
            .aria_label(match &wake {
                Some(wake) => SharedString::from(format!("Snooze until {wake}")),
                None => "Snooze (enter a duration first)".into(),
            })
            .map(|button| {
                if valid {
                    button
                        .bg(theme.accent)
                        .text_color(theme.on_accent)
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.9))
                        .on_click(cx.listener(|this, _, _, cx| this.submit_custom_snooze(cx)))
                } else {
                    button
                        .bg(crate::theme::wash(0.06))
                        .text_color(theme.text_muted.opacity(0.6))
                }
            })
            .child("Snooze");
        let editor = div()
            .id(SharedString::from(format!("{scope}-custom-editor")))
            .flex()
            .flex_col()
            .gap(px(4.0))
            .px(px(6.0))
            .py(px(6.0))
            .rounded(px(popover::MENU_ITEM_RADIUS))
            .bg(crate::theme::wash(0.04))
            .on_mouse_down(MouseButton::Left, keep_focus())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(field)
                    .child(confirm),
            )
            .child(
                div()
                    .debug_selector(|| "custom-snooze-hint".to_string())
                    .px(px(2.0))
                    .text_size(crate::typography::ui_rems(11.5))
                    .text_color(caption_color)
                    .child(caption),
            );
        if self.reduced_motion {
            return editor.into_any_element();
        }
        // The row opens into the editor: a short fade and settle, once.
        editor
            .with_animation(
                SharedString::from(format!("{scope}-custom-editor-in")),
                motion::FADE_QUICK.animation(),
                |el, t| {
                    el.opacity(0.4 + 0.6 * t)
                        .relative()
                        .top(px(-3.0 * (1.0 - t)))
                },
            )
            .into_any_element()
    }

    pub(super) fn close_row_snooze_menu(&mut self, cx: &mut Context<Self>) {
        if self.row_snooze_menu.begin_close() {
            popover::reap_popup(cx, |shell: &mut Self| &mut shell.row_snooze_menu);
            cx.notify();
        }
    }

    /// The quick snooze menu, floated beside the clock button that opened it.
    pub(super) fn render_row_snooze_menu(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let menu = self.row_snooze_menu.get()?.clone();
        let closing = self.row_snooze_menu.closing_since();
        let card = self
            .snooze_preset_card("row-snooze", &menu.chat_id, theme, cx)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_row_snooze_menu(cx)))
            .into_any_element();
        Some(popover::menu_at(
            "row-snooze-menu",
            menu.position,
            card,
            closing,
        ))
    }

    /// T3 hover actions for a sidebar row, in place of the single archive
    /// control: Snooze (opens the quick menu) and Settle for active rows,
    /// including snoozed rows, and Unsettle for settled rows. Each button
    /// keeps its press and click to itself, so the row under it neither
    /// selects nor starts a drag.
    pub(super) fn render_t3_row_actions(
        &self,
        chat_id: &str,
        row_id: &str,
        archived: bool,
        compact: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        #[derive(Clone, Copy)]
        enum Action {
            Snooze,
            Settle,
            Unsettle,
        }
        let actions: &[Action] = if archived {
            &[Action::Unsettle]
        } else {
            &[Action::Snooze, Action::Settle]
        };
        let size = if compact {
            COMPACT_ROW_ACTION_SIZE
        } else {
            ROW_ACTION_SIZE
        };
        let glyph = if compact {
            SIDEBAR_ACTIVE_HARNESS_ICON_SIZE
        } else {
            12.0
        };
        let menu_open = self
            .row_snooze_menu
            .as_open()
            .is_some_and(|menu| menu.chat_id == chat_id);
        let buttons = actions.iter().map(|&action| {
            // Each action wears its own hue: amber to snooze, green to
            // settle, accent to bring a settled row back.
            let (suffix, glyph_path, tooltip, tint) = match action {
                Action::Snooze => ("snooze", icons::CLOCK_CIRCLE, "Snooze…", theme.warning),
                Action::Settle => ("settle", icons::CHECK, "Settle", theme.success),
                Action::Unsettle => (
                    "unsettle",
                    icons::ARCHIVE_UP_MINIMALISTIC,
                    "Unsettle",
                    theme.accent,
                ),
            };
            let wash = tint.opacity(0.18);
            let tint = legible_row_action_tint(tint, wash, theme);
            let element_id = format!("{row_id}-{suffix}");
            let id = chat_id.to_owned();
            let active = matches!(action, Action::Snooze) && menu_open;
            let mut button = div()
                .id(SharedString::from(element_id.clone()))
                .debug_selector(move || element_id.clone())
                .size(px(size))
                .relative()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .cursor_pointer()
                .role(gpui::Role::Button)
                .aria_label(tooltip)
                .when(active, |el| el.bg(wash))
                .hover(move |el| el.bg(wash))
                .child(icon(glyph_path).size(px(glyph)).text_color(tint))
                // Above the row: below it the chip would cover the next row.
                .tooltip(crate::settings::widgets::text_tooltip_above(tooltip));
            button = match action {
                Action::Snooze => {
                    let owner = id.clone();
                    // Measured at prepaint: the menu opens beside this button.
                    let anchor =
                        std::rc::Rc::new(std::cell::Cell::new(None::<gpui::Bounds<Pixels>>));
                    let paint_anchor = anchor.clone();
                    button
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.row_snooze_menu
                                    .note_trigger_press_matching(|menu| menu.chat_id == owner);
                            }),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            // This press dismissed the menu: leave it closed.
                            if this.row_snooze_menu.take_press_was_open() {
                                return;
                            }
                            this.close_chat_menu(cx);
                            let bounds = anchor.get();
                            this.row_snooze_menu.open(RowSnoozeMenu {
                                chat_id: id.clone(),
                                // Beside the button, top edges aligned with
                                // the menu's first row.
                                position: bounds.map_or(gpui::point(px(0.0), px(0.0)), |b| {
                                    gpui::point(b.right() + px(6.0), b.top() - px(6.0))
                                }),
                            });
                            cx.notify();
                        }))
                        .child(
                            gpui::canvas(
                                move |bounds, _, _| paint_anchor.set(Some(bounds)),
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .inset_0(),
                        )
                }
                _ => button
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        // The row leaves under a still pointer; let the next
                        // row adopt the hover instead of flashing.
                        this.chat_hover_resync = true;
                        match action {
                            Action::Settle => this.settle_chat(id.clone(), cx),
                            Action::Unsettle => this.set_chat_archived(id.clone(), false, cx),
                            Action::Snooze => {}
                        }
                    })),
            };
            button
        });
        div()
            .flex()
            .flex_row()
            .items_center()
            .when(!compact, |el| el.gap(px(2.0)).mr(px(-3.0)))
            .children(buttons)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_action_tints_clear_the_non_text_floor_on_every_builtin() {
        let registry = zeron_theme::ThemeRegistry::builtin();
        for variant in registry.families.iter().flat_map(|family| &family.variants) {
            let theme = Theme::from_variant(
                variant,
                zeron_theme::AccentSelection::ThemeDefault,
                zeron_theme::SurfacePreference::Opaque,
            );
            for tint in [theme.warning, theme.success, theme.accent] {
                let wash = tint.opacity(0.18);
                let backdrop = crate::theme::flatten(wash, theme.surface);
                let legible = legible_row_action_tint(tint, wash, &theme);
                assert!(
                    crate::theme::contrast_ratio(legible, backdrop) >= 3.0,
                    "{} row action tint",
                    variant.id
                );
            }
        }
    }

    /// 2026-10-14 (a Wednesday) at `hour:minute` UTC, plus `days`.
    fn at(hour: u32, minute: u32, days: u32) -> chrono::DateTime<Utc> {
        use chrono::TimeZone as _;
        Utc.with_ymd_and_hms(2026, 10, 14 + days, hour, minute, 0)
            .unwrap()
    }

    #[test]
    fn typed_durations_sum_unit_parts_within_u32_seconds() {
        assert_eq!(parse_snooze_duration("90"), Some(5_400));
        assert_eq!(parse_snooze_duration("90m"), Some(5_400));
        assert_eq!(parse_snooze_duration(" 3h "), Some(10_800));
        assert_eq!(parse_snooze_duration("1h30m"), Some(5_400));
        assert_eq!(parse_snooze_duration("1 day, 4 hours"), Some(100_800));
        assert_eq!(parse_snooze_duration("2W"), Some(1_209_600));
        for invalid in [
            "",
            " ",
            "0",
            "0m",
            "-1h",
            "1.5h",
            "h",
            "3x",
            "tomorrow",
            "4294967296",
        ] {
            assert_eq!(parse_snooze_duration(invalid), None, "{invalid}");
        }
        assert_eq!(parse_snooze_duration("4294967295m"), None);
        assert_eq!(parse_snooze_duration("49710d"), Some(49_710 * 86_400));
        assert_eq!(parse_snooze_duration("49711d"), None);
    }

    #[test]
    fn until_suggestions_follow_the_clock_and_calendar() {
        let titles = |now| {
            snooze_until_suggestions(&now)
                .into_iter()
                .map(|(title, at)| (title, at))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            titles(at(10, 0, 0)),
            [
                ("This evening", at(18, 0, 0)),
                ("Tomorrow morning", at(9, 0, 1)),
                ("Next week", at(9, 0, 5)),
            ]
        );
        // Under an hour before 6 PM, the evening is no longer offered.
        assert_eq!(titles(at(17, 30, 0)).len(), 2);
        // Sunday: tomorrow morning is already Monday.
        assert_eq!(
            titles(at(10, 0, 4))
                .iter()
                .map(|(title, _)| *title)
                .collect::<Vec<_>>(),
            ["This evening", "Tomorrow morning"]
        );
        // Monday: next week is the following Monday.
        assert_eq!(titles(at(8, 0, 5)).last().unwrap().1, at(9, 0, 12));
    }

    #[test]
    fn wake_times_round_up_to_minutes_and_name_the_day_and_year_when_needed() {
        let now = at(10, 0, 0);
        let later = |seconds| now + chrono::TimeDelta::seconds(seconds);
        assert_eq!(snooze_seconds_until(&later(30), &now), Some(60));
        assert_eq!(snooze_seconds_until(&later(61), &now), Some(120));
        assert_eq!(snooze_seconds_until(&now, &now), None);
        assert_eq!(
            snooze_seconds_until(&later(i64::from(u32::MAX) + 60), &now),
            None
        );
        assert_eq!(snooze_wake_label(&at(18, 0, 0), &now), "6:00 PM");
        assert_eq!(snooze_wake_label(&at(9, 0, 1), &now), "Thu 9:00 AM");
        assert_eq!(snooze_wake_label(&at(9, 0, 10), &now), "Oct 24, 9:00 AM");
        assert_eq!(
            snooze_wake_label(&later(90 * 86_400), &now),
            "Jan 12, 2027, 10:00 AM"
        );
    }

    #[gpui::test]
    fn parking_waits_for_success_and_does_not_steal_later_navigation(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let dir = tempfile::tempdir().unwrap();
        let (out, mut requests) = tokio::sync::mpsc::channel(16);
        let (replies, inbound) = tokio::sync::mpsc::channel(16);
        let engine =
            crate::state::EngineHandle::from_test_client(zeron_rpc::RpcClient::new(out, inbound));
        cx.update(|cx| {
            gpui_base::init(cx);
            cx.set_global(Theme::default());
            settings::init(UiSettings::default(), dir.path(), cx);
            crate::app_menus::init(cx);
            crate::history::init(
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                cx,
            );
        });
        let window = cx.add_window(|_, cx| {
            let state = cx.new(|_| AppState::new());
            state.update(cx, |state, _| {
                state.workspace_scope = Some(WorkspaceScope::Local);
                state.no_project = true;
                state.local_device_id = Some("local".into());
                state.selected_chat = Some("first".into());
                state.set_test_engine(engine);
                state.chats = ["first", "next", "elsewhere"]
                    .into_iter()
                    .enumerate()
                    .map(|(ix, id)| {
                        serde_json::from_value(serde_json::json!({
                            "id": id, "deviceId": "local", "title": id, "archived": false,
                            "createdAt": Utc::now() - chrono::TimeDelta::minutes(ix as i64),
                        }))
                        .unwrap()
                    })
                    .collect();
            });
            let mut shell = Shell::new(
                state,
                EngineBootConfig {
                    data_dir: dir.path().into(),
                    ipc_port: 0,
                    edge_url: String::new(),
                    edge_token: None,
                    org_id: None,
                    workos_client_id: None,
                    default_harness: zeron_proto::HarnessId::Mock,
                },
                cx,
            );
            shell.settings.sidebar_mode = SidebarMode::T3;
            shell
        });
        let deliver = |reply: serde_json::Value| {
            runtime.block_on(async {
                replies.send(reply.to_string()).await.unwrap();
                tokio::time::timeout(std::time::Duration::from_secs(1), async {
                    while replies.capacity() < replies.max_capacity() {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
            });
        };
        window
            .update(cx, |shell, _, cx| shell.settle_chat("first".into(), cx))
            .unwrap();
        cx.run_until_parked();
        let request: serde_json::Value =
            serde_json::from_str(&requests.try_recv().unwrap()).unwrap();
        assert_eq!(request["params"]["op"], "setChatArchived");
        assert_eq!(
            window
                .read_with(cx, |shell, cx| shell.state.read(cx).selected_chat.clone())
                .unwrap()
                .as_deref(),
            Some("first")
        );
        deliver(serde_json::json!({"id": request["id"], "err": "registry unavailable"}));
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                assert_eq!(shell.state.read(cx).selected_chat.as_deref(), Some("first"));
                assert!(
                    shell
                        .sidebar_notice
                        .as_deref()
                        .unwrap()
                        .contains("registry unavailable")
                );
                shell.snooze_chat("first".into(), std::time::Duration::from_secs(900), cx);
            })
            .unwrap();
        cx.run_until_parked();
        let request: serde_json::Value =
            serde_json::from_str(&requests.try_recv().unwrap()).unwrap();
        assert_eq!(request["params"]["op"], "setChatSnoozedUntil");
        window
            .update(cx, |shell, _, cx| {
                shell.state.update(cx, |state, _| {
                    state.selected_chat = Some("elsewhere".into())
                });
            })
            .unwrap();
        deliver(serde_json::json!({"id": request["id"], "ok": {"ok": true}}));
        cx.run_until_parked();
        assert_eq!(
            window
                .read_with(cx, |shell, cx| shell.state.read(cx).selected_chat.clone())
                .unwrap()
                .as_deref(),
            Some("elsewhere")
        );
        window
            .update(cx, |shell, _, cx| {
                shell
                    .state
                    .update(cx, |state, _| state.selected_chat = Some("first".into()));
                shell.settle_chat("first".into(), cx);
            })
            .unwrap();
        cx.run_until_parked();
        let request: serde_json::Value =
            serde_json::from_str(&requests.try_recv().unwrap()).unwrap();
        deliver(serde_json::json!({"id": request["id"], "ok": {"ok": true}}));
        cx.run_until_parked();
        assert_eq!(
            window
                .read_with(cx, |shell, cx| shell.state.read(cx).selected_chat.clone())
                .unwrap()
                .as_deref(),
            Some("next")
        );
    }
}
