//! T3 Mode sidebar lanes (after t3code's inbox): sessions needing the user
//! lead the list, running sessions fold into a quiet Working shelf, snoozed
//! sessions wait in a Snoozed shelf until their deadline, and archived
//! sessions read as Settled. Lanes are derived at render time — pins and
//! custom sections keep their membership while a session sits on a shelf,
//! so it returns to its place when it comes back. Default mode never calls
//! into this module.

use super::spaces::{
    SIDEBAR_DISCLOSURE_BODY_INSET, SIDEBAR_DISCLOSURE_HEADER_HEIGHT,
    SIDEBAR_DISCLOSURE_SECTION_HEIGHT, SIDEBAR_SECTION_GAP, sidebar_count_badge,
    sidebar_header_glyph, status_dot_color,
};
use super::*;
use zeron_proto::ChatIndicator;

/// Where an active session sits in T3 Mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum T3Lane {
    Inbox,
    Working,
    Snoozed,
}

/// A snooze holds until its deadline, even over new activity; otherwise a
/// running session is shelved and everything else stays in the inbox.
pub(super) fn t3_lane(
    status: ChatIndicator,
    chat: &zeron_proto::Chat,
    now: DateTime<Utc>,
) -> T3Lane {
    if chat.is_snoozed(now) {
        T3Lane::Snoozed
    } else if status == ChatIndicator::Working {
        T3Lane::Working
    } else {
        T3Lane::Inbox
    }
}

/// Inbox rows that want the user: blocked on input, failed, or finished
/// unseen.
pub(super) fn t3_needs_attention(status: ChatIndicator) -> bool {
    matches!(
        status,
        ChatIndicator::AwaitingInput | ChatIndicator::Errored | ChatIndicator::Completed
    )
}

/// Lift attention rows above quiet ones, keeping the sort order within each
/// band so the resort glide carries only real promotions.
pub(super) fn t3_attention_first<T>(rows: &mut [T], status: impl Fn(&T) -> ChatIndicator) {
    rows.sort_by_key(|row| !t3_needs_attention(status(row)));
}

/// A snoozed row's time column: when it comes back ("in 5m", "in 3h",
/// "in 2d"). Rounds up so a hidden session never reads "in 0m".
pub(super) fn t3_wake_label(until: DateTime<Utc>, now: DateTime<Utc>) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    let remaining = until.signed_duration_since(now).num_seconds();
    if remaining <= 0 {
        return "now".into();
    }
    let ceil = |unit: i64| (remaining + unit - 1) / unit;
    if ceil(MINUTE) < 60 {
        format!("in {}m", ceil(MINUTE).max(1))
    } else if ceil(HOUR) < 24 {
        format!("in {}h", ceil(HOUR))
    } else {
        format!("in {}d", ceil(DAY))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum T3Shelf {
    Working,
    Snoozed,
}

impl T3Shelf {
    pub(super) fn of(lane: T3Lane) -> Option<Self> {
        match lane {
            T3Lane::Inbox => None,
            T3Lane::Working => Some(Self::Working),
            T3Lane::Snoozed => Some(Self::Snoozed),
        }
    }

    /// Disclosure motion and keyed-row identity.
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Working => "t3-working",
            Self::Snoozed => "t3-snoozed",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Working => "Working",
            Self::Snoozed => "Snoozed",
        }
    }
}

impl Shell {
    pub(super) fn t3_mode(&self) -> bool {
        self.settings.sidebar_mode == SidebarMode::T3
    }

    pub(super) fn t3_shelf_open(&self, shelf: T3Shelf) -> bool {
        match shelf {
            T3Shelf::Working => self.t3_working_open,
            T3Shelf::Snoozed => self.t3_snoozed_open,
        }
    }

    /// The Archived shelf in Default, the Settled shelf in T3: each mode
    /// keeps its own disclosure state, and Settled starts folded.
    pub(super) fn archived_shelf_open(&self) -> bool {
        if self.t3_mode() {
            self.t3_settled_open
        } else {
            self.archived_open
        }
    }

    pub(super) fn set_archived_shelf_open(&mut self, open: bool) {
        if self.t3_mode() {
            self.t3_settled_open = open;
        } else {
            self.archived_open = open;
        }
    }

    fn set_t3_shelf_open(&mut self, shelf: T3Shelf, open: bool) {
        match shelf {
            T3Shelf::Working => self.t3_working_open = open,
            T3Shelf::Snoozed => self.t3_snoozed_open = open,
        }
    }

    /// Open the shelf holding a session so a reveal (rename, jump) has a row
    /// to land on. The opening motion starts on the next render.
    pub(super) fn reveal_t3_shelf(&mut self, shelf: T3Shelf) {
        if !self.t3_shelf_open(shelf) {
            self.set_t3_shelf_open(shelf, true);
            self.queue_t3_shelf_reveal(shelf);
        }
    }

    fn queue_t3_shelf_reveal(&mut self, shelf: T3Shelf) {
        self.sidebar_reveal_motions.insert(shelf.key().to_owned());
        self.sidebar_prev_order.clear();
        self.sidebar_resort.clear();
        self.sidebar_new_keys.clear();
    }

    /// One shelf as a keyed sidebar row: a quiet header (lane glyph, label,
    /// count while collapsed) over the shared disclosure body. Rows arrive
    /// already rendered with their slot heights.
    pub(super) fn render_t3_shelf(
        &mut self,
        shelf: T3Shelf,
        rows: Vec<SidebarKeyedRow>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> SidebarKeyedRow {
        let key = shelf.key();
        let open = self.t3_shelf_open(shelf);
        let count = rows.len();
        let body_height = SIDEBAR_DISCLOSURE_BODY_INSET
            + rows.iter().map(|(_, height, _)| *height).sum::<f32>()
            + SIDEBAR_LIST_GAP * count.saturating_sub(1) as f32;
        self.begin_queued_sidebar_reveal(key, open, body_height);

        let glyph = match shelf {
            // The lane's own status colour, held still and softened: the rows
            // inside carry the live motion, so the folded shelf stays calm.
            T3Shelf::Working => div()
                .flex_none()
                .size(px(12.0))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div().size(px(6.0)).rounded_full().bg(status_dot_color(
                        ChatIndicator::Working,
                        theme,
                    )
                    .opacity(0.7)),
                )
                .into_any_element(),
            T3Shelf::Snoozed => icon(icons::MOON)
                .size(px(12.0))
                .flex_none()
                .text_color(sidebar_header_glyph(theme))
                .into_any_element(),
        };
        let badge = (!open).then(|| {
            let badge = sidebar_count_badge(theme, count);
            if self.reduced_motion {
                badge.into_any_element()
            } else {
                // Keyed by the count: a session landing on (or leaving) a
                // folded shelf settles the badge in, marking where it went.
                badge
                    .with_animation(
                        SharedString::from(format!("{key}-count-{count}")),
                        motion::GLYPH_STATE.animation(),
                        |el, t| {
                            el.opacity(0.35 + 0.65 * t)
                                .relative()
                                .top(px(-2.0 * (1.0 - t)))
                        },
                    )
                    .into_any_element()
            }
        });
        let header = div()
            .id(SharedString::from(format!("{key}-toggle")))
            .debug_selector(move || format!("{key}-toggle"))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .h(px(SIDEBAR_DISCLOSURE_HEADER_HEIGHT))
            .px(px(Theme::SPACE_SM))
            .cursor_pointer()
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(format!(
                "{} sessions, {count}",
                shelf.label()
            )))
            .child(glyph)
            .child(
                div()
                    .text_size(crate::typography::ui_rems(12.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text_muted.opacity(0.5))
                    .child(shelf.label()),
            )
            .children(badge)
            .child(div().flex_1())
            .child(self.sidebar_disclosure_chevron(key, open, theme))
            .on_click(cx.listener(move |this, _, _, cx| {
                let was_open = this.t3_shelf_open(shelf);
                this.begin_sidebar_disclosure_motion(
                    key,
                    if was_open { body_height } else { 0.0 },
                    if was_open { 0.0 } else { body_height },
                );
                this.set_t3_shelf_open(shelf, !was_open);
                cx.notify();
            }));
        let body = div()
            .w_full()
            .flex()
            .flex_col()
            .pt(px(SIDEBAR_DISCLOSURE_BODY_INSET))
            .gap(px(SIDEBAR_LIST_GAP))
            .children(rows.into_iter().map(|(_, _, row)| row));
        let body =
            self.render_sidebar_disclosure_body(key, open, body_height, body.into_any_element());
        let height = SIDEBAR_DISCLOSURE_SECTION_HEIGHT + if open { body_height } else { 0.0 };
        let element = div()
            .id(SharedString::from(format!("sidebar-{key}-section")))
            .debug_selector(move || format!("sidebar-{key}-section"))
            .w_full()
            .flex()
            .flex_col()
            .pt(px(SIDEBAR_SECTION_GAP))
            .child(header)
            .child(body)
            .into_any_element();
        (format!("s:{key}"), height, element)
    }
}

/// Native sidebar fixture only: fold or unfold every T3 shelf at once.
#[cfg(feature = "project-palette-fixture")]
impl Shell {
    pub fn fixture_t3_shelves(&mut self, open: bool, cx: &mut Context<Self>) {
        self.t3_working_open = open;
        self.t3_snoozed_open = open;
        self.t3_settled_open = open;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).unwrap()
    }

    fn chat(snoozed_until: Option<DateTime<Utc>>) -> zeron_proto::Chat {
        zeron_proto::Chat {
            id: "chat".into(),
            device_id: "device".into(),
            title: None,
            archived: false,
            cwd: None,
            branch: None,
            checkout_id: None,
            source_context: None,
            snoozed_until,
            config: None,
            last_message_preview: None,
            last_message_at: Some(at(10)),
            created_at: at(5),
            harness_session_id: None,
            harness_session_cwd: None,
            parent_chat_id: None,
            orchestration: None,
            space_id: None,
            last_seen_at: None,
            room_gen: None,
        }
    }

    #[test]
    fn snooze_holds_over_activity_until_its_deadline() {
        let now = at(1_000);
        let snoozed = chat(Some(at(2_000)));
        for status in [
            ChatIndicator::Working,
            ChatIndicator::AwaitingInput,
            ChatIndicator::Completed,
            ChatIndicator::Idle,
        ] {
            assert_eq!(t3_lane(status, &snoozed, now), T3Lane::Snoozed);
        }
        let woke = chat(Some(at(1_000)));
        assert_eq!(t3_lane(ChatIndicator::Working, &woke, now), T3Lane::Working);
        assert_eq!(t3_lane(ChatIndicator::Completed, &woke, now), T3Lane::Inbox);
        assert_eq!(
            t3_lane(ChatIndicator::Idle, &chat(None), now),
            T3Lane::Inbox
        );
    }

    #[test]
    fn attention_rows_lead_without_reordering_either_band() {
        use ChatIndicator::*;
        let mut rows = [
            (1, Idle),
            (2, Completed),
            (3, Idle),
            (4, AwaitingInput),
            (5, Errored),
        ];
        t3_attention_first(&mut rows, |row| row.1);
        let order: Vec<_> = rows.iter().map(|row| row.0).collect();
        assert_eq!(order, [2, 4, 5, 1, 3]);
    }

    #[test]
    fn wake_label_rounds_up_to_the_next_unit() {
        let now = at(0);
        assert_eq!(t3_wake_label(at(0), now), "now");
        assert_eq!(t3_wake_label(at(1), now), "in 1m");
        assert_eq!(t3_wake_label(at(15 * 60), now), "in 15m");
        assert_eq!(t3_wake_label(at(59 * 60 + 1), now), "in 1h");
        assert_eq!(t3_wake_label(at(60 * 60), now), "in 1h");
        assert_eq!(t3_wake_label(at(60 * 60 + 1), now), "in 2h");
        assert_eq!(t3_wake_label(at(23 * 60 * 60 + 1), now), "in 1d");
        assert_eq!(t3_wake_label(at(24 * 60 * 60), now), "in 1d");
        assert_eq!(t3_wake_label(at(7 * 24 * 60 * 60), now), "in 7d");
    }
}
