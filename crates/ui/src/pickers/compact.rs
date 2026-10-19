//! Alternate presentation of the same model and option mutations.
use super::*;
use crate::effort_slider;

pub(super) const COMPACT_WIDTH: f32 = 256.0;
/// One-line model rows (plus the 2px gap) so about seven fit at once.
pub(super) const COMPACT_ROW_HEIGHT: f32 = 32.0;
pub(super) const COMPACT_LIST_ROWS: f32 = 7.0;
/// The list fits its rows, up to [`COMPACT_LIST_ROWS`]; empty and loading
/// states keep room for a few skeleton rows.
pub(super) fn compact_list_height(rows: usize) -> f32 {
    let rows = if rows == 0 {
        4.0
    } else {
        (rows as f32).min(COMPACT_LIST_ROWS)
    };
    rows * (COMPACT_ROW_HEIGHT + popover::MENU_GAP) + 2.0 * popover::CARD_INSET
}
// The effort track (see `effort_slider`): a 28px pill with the thumb inset
// in it. Stops and pointer input share the thumb's centre inset.
const SLIDER_HEIGHT: f32 = crate::effort_slider::TRACK_HEIGHT;
/// The beat between the two knocks of crossing into the top effort tiers.
pub(super) const PEAK_KNOCK_GAP: std::time::Duration = std::time::Duration::from_millis(70);
/// Both header buttons fill the header's height, so they sit one card inset
/// from the card's top and sides and their radius is concentric with its
/// corners. The model's name titles it; the effort has its own row below.
const HEADER_HEIGHT: f32 = 36.0;
/// Flush with the card inset, a 15pt glyph centred in 32pt ends 12pt from
/// the card edge: the slider rail's end. The provider button mirrors it on
/// the leading side.
const FAST_BUTTON_WIDTH: f32 = 32.0;
/// The effort's own header: its icon and level, and reset at the far end.
const EFFORT_HEADER_HEIGHT: f32 = 24.0;
const RESET_BUTTON_SIZE: f32 = 24.0;
/// The header inside the card inset, then the slider block and the space
/// above the option rows.
fn header_block() -> f32 {
    2.0 * popover::CARD_INSET + HEADER_HEIGHT
}
/// The slider's 8pt below its box leaves the track 12pt from the card's
/// bottom when no options follow.
const SLIDER_TOP: f32 = 4.0;
const SLIDER_BOTTOM: f32 = 8.0;
const EFFORT_BLOCK: f32 = EFFORT_HEADER_HEIGHT + SLIDER_TOP + SLIDER_HEIGHT + SLIDER_BOTTOM;
const OPTIONS_GAP: f32 = 4.0;
/// The card's 1pt border, top and bottom, sits inside its height.
const CARD_BORDERS: f32 = 2.0;
/// The model list's chrome above the rows: the back button with the
/// provider tab strip (one row plus the card inset above and below, 40),
/// then the search row (40).
pub(super) const LIST_HEADER: f32 = 80.0;
/// Option rows (26pt plus a 2pt gap) all show up to this many, then scroll.
const OPTIONS_MAX_ROWS: usize = 6;
/// The rows' 4pt top inset, then the rows; the panel's inset closes below.
fn options_height(count: usize) -> f32 {
    2.0 + count.min(OPTIONS_MAX_ROWS) as f32 * 28.0
}

/// Option ids Cursor uses for an effort ladder (its models carry none).
const EFFORT_OPTION_IDS: [&str; 3] = ["effort", "reasoning", "reasoning_effort"];

pub(super) struct CompactEffort {
    pub labels: Vec<SharedString>,
    pub selected: usize,
    /// The model's own default step, the reset target.
    pub default: Option<usize>,
    /// Which steps are a top tier (Ultra, Ultracode, Ultrathink).
    pub peaks: Vec<bool>,
    stops: EffortStops,
}

enum EffortStops {
    Reasoning(Vec<ReasoningLevel>),
    Option {
        id: String,
        choices: Vec<String>,
        default: String,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CompactPage {
    Panel,
    Models,
}

#[derive(Default)]
pub(super) struct CompactMotion {
    /// The effort thumb's spring toward its step.
    effort: Option<effort_slider::Spring>,
    press: Option<ScalarTransition>,
    /// The pointer's place on the track while dragging; the thumb follows it.
    drag_fraction: Option<f32>,
    /// The step under the pointer while the track is held, applied on release.
    effort_pending: Option<usize>,
    /// The fade into the top tiers' halo.
    top: Option<effort_slider::Fade>,
    reset: Option<ScalarTransition>,
    /// The track's sparks, sheen and breath, paced by the effort.
    heat: effort_slider::HeatClock,
    energy: Option<ScalarTransition>,
    /// Shimmer clock for fast mode and the top effort levels, integrated
    /// per frame so changing intensity never jumps the band.
    shimmer_phase: f32,
    shimmer_last_frame: Option<std::time::Instant>,
    fast: Option<ScalarTransition>,
    height: Option<ScalarTransition>,
    page: Option<CompactPage>,
    reveal: Option<ScalarTransition>,
    pub frame_height: f32,
}

pub(super) struct ScalarTransition {
    from: f32,
    to: f32,
    started: std::time::Instant,
}
impl ScalarTransition {
    pub(super) fn value(&self, now: std::time::Instant) -> f32 {
        let progress = motion::RESIZE.progress(
            now.duration_since(self.started).as_secs_f32()
                / motion::RESIZE
                    .total()
                    .mul_f32(motion::speed_scale())
                    .as_secs_f32(),
        );
        motion::lerp(self.from, self.to, progress)
    }
    pub(super) fn sample(
        state: &mut Option<Self>,
        target: f32,
        now: std::time::Instant,
        reduced: bool,
    ) -> (f32, bool) {
        let Some(transition) = state else {
            *state = Some(Self {
                from: target,
                to: target,
                started: now,
            });
            return (target, false);
        };
        if reduced {
            transition.from = target;
            transition.to = target;
            return (target, false);
        }
        if transition.to != target {
            transition.from = transition.value(now);
            transition.to = target;
            transition.started = now;
        }
        let value = transition.value(now);
        (value, (value - target).abs() > 0.001)
    }
}

impl Pickers {
    pub(super) fn render_compact_model_row(
        &mut self,
        ix: usize,
        row: &ModelRowData,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).for_popup();
        let selected = self.effective_harness(cx) == Some(row.harness)
            && self
                .selected_model(cx)
                .is_some_and(|model| model.id == row.model.id);
        let favorite = self.defaults.is_favorite(row.harness, &row.model.id);
        let (icon, tint) = model_brand_icon(row.harness, &row.model.id);
        let harness = row.harness;
        let model = row.model.id.clone();
        let label: SharedString = row.model.label.clone().into();
        // The brand mark names the provider; a description appears only to
        // tell identically named rows apart.
        let attribution: Option<SharedString> = row
            .model
            .description
            .as_deref()
            .map(str::trim)
            .filter(|d| row.ambiguous && !d.is_empty())
            .map(|d| SharedString::from(d.to_owned()));
        let hovered = self.active == ix;
        let details: SharedString = match row.model.description.as_deref() {
            Some(description) => format!(
                "{} · {}\n{}",
                row.model.label, row.harness_name, description
            )
            .into(),
            None => format!("{} · {}", row.model.label, row.harness_name).into(),
        };
        let accessible_name = details.clone();
        let favorite_hint: SharedString = format!(
            "{} {} {}",
            if favorite { "Remove" } else { "Add" },
            row.model.label,
            if favorite {
                "from favorites"
            } else {
                "to favorites"
            }
        )
        .into();
        let favorite_tooltip: SharedString =
            format!("{}\n⌘⇧F on the highlighted model", favorite_hint).into();
        let item = div()
            .id(("model-row", ix))
            .role(gpui::Role::ListBoxOption)
            .aria_label(accessible_name)
            .aria_selected(selected)
            .tooltip(move |_, cx| cx.new(|_| PickerHint(details.clone())).into())
            .tooltip_show_delay(std::time::Duration::from_millis(500))
            .h(px(COMPACT_ROW_HEIGHT))
            .pl(px(8.0))
            .pr(px(4.0))
            .rounded(px(popover::MENU_ITEM_RADIUS))
            .flex()
            .items_center()
            .gap(px(8.0))
            .cursor_pointer()
            .text_color(theme.text)
            .when(selected, |el| {
                el.bg(crate::theme::card_selected_bg())
                    .shadow(crate::theme::card_selected_shadows())
            })
            .when(!selected && self.active == ix, |el| {
                el.bg(crate::theme::ink(0.05))
            })
            .when(self.compact_keyboard && self.active == ix, |el| {
                el.aria_active_descendant()
            })
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && (this.active != ix || this.compact_keyboard) {
                    this.compact_keyboard = false;
                    this.active = ix;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.activate_model_index(ix, cx)))
            .child(
                crate::icons::icon(icon)
                    .size(px(14.0))
                    .flex_none()
                    .text_color(tint.unwrap_or(theme.text_muted)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_baseline()
                    .gap(px(6.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .child(
                        div()
                            .flex_none()
                            .max_w_full()
                            .truncate()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(label),
                    )
                    .when_some(attribution, |el, attribution| {
                        el.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(crate::typography::ui_rems(11.0))
                                .text_color(theme.text_muted)
                                .child(attribution),
                        )
                    }),
            )
            .child(
                div()
                    .id(("model-star", ix))
                    .role(gpui::Role::Button)
                    .aria_label(favorite_hint)
                    .aria_toggled(if favorite {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .aria_keyshortcuts("Meta+Shift+F")
                    .tooltip(move |_, cx| cx.new(|_| PickerHint(favorite_tooltip.clone())).into())
                    .size(px(24.0))
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    // Quiet until the row is hovered or the model is starred.
                    .when(!favorite && !hovered, |el| el.opacity(0.0))
                    .hover(|s| s.bg(crate::theme::ink(0.08)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_model_favorite(harness, &model, cx);
                    }))
                    .child(
                        crate::icons::icon(if favorite {
                            crate::icons::STAR_BOLD
                        } else {
                            crate::icons::STAR
                        })
                        .size(px(13.0))
                        .text_color(if favorite {
                            theme.accent
                        } else {
                            theme.text_muted
                        }),
                    ),
            );
        div()
            .pb(px(popover::MENU_GAP))
            .child(item)
            .into_any_element()
    }

    pub(super) fn compact_model_picker(&self, cx: &App) -> bool {
        self.title.is_none() && crate::settings::compact_model_picker(cx)
    }

    pub(super) fn show_compact_models(&mut self, cx: &mut Context<Self>) {
        self.setting_menu = None;
        self.compact_model_list = true;
        self.model_rail = ModelRail::Harness;
        self.reset_compact_search("Search models…", cx);
        let selected = self.selected_model_index(cx);
        self.active = selected;
        self.model_scroll_base().set_offset(gpui::Point::default());
        self.model_scroll
            .scroll_to_item(selected, gpui::ScrollStrategy::Nearest);
        self.focus_on_mount = true;
        cx.notify();
    }

    /// Back from the model list to the panel.
    pub(super) fn show_compact_panel(&mut self, cx: &mut Context<Self>) {
        self.compact_model_list = false;
        self.focus_on_mount = true;
        cx.notify();
    }

    /// Clear the shared filter for a new page without letting the clear's
    /// Edited event reset the highlight anchored right after.
    fn reset_compact_search(&mut self, placeholder: &'static str, cx: &mut Context<Self>) {
        self.search_reset_muted = !self.search.read(cx).text().is_empty();
        self.search.update(cx, |input, cx| {
            input.set_placeholder(placeholder, cx);
            if !input.text().is_empty() {
                input.set_text("", cx);
            }
        });
    }

    /// Picking a provider runs its last-used model at that model's last
    /// settings (the remembered defaults take over once the harness moves).
    pub(super) fn pick_compact_provider(&mut self, harness: HarnessId, cx: &mut Context<Self>) {
        self.model_rail = ModelRail::Harness;
        self.pick_harness(harness, cx);
        self.show_compact_panel(cx);
    }

    /// Tabs stay available through searches and catalog loading. A committed
    /// chat's rail already limits the providers and favorites to its harness.
    pub(super) fn compact_tabs(&self, cx: &App) -> Vec<Option<HarnessId>> {
        std::iter::once(None)
            .chain(self.rail_descriptors(cx).into_iter().map(|d| Some(d.id)))
            .collect()
    }

    pub(super) fn select_compact_tab(&mut self, harness: Option<HarnessId>, cx: &mut Context<Self>) {
        self.compact_keyboard = false;
        self.setting_menu = None;
        if let Some(harness) = harness {
            self.model_rail = ModelRail::Harness;
            self.pick_harness(harness, cx);
        } else {
            self.model_rail = ModelRail::Favorites;
        }
        self.active = self.selected_model_index(cx);
        self.model_scroll_base().set_offset(gpui::Point::default());
        self.model_scroll
            .scroll_to_item(self.active, gpui::ScrollStrategy::Nearest);
        cx.notify();
    }

    /// Back to the panel, then provider tabs that scope the list and search.
    /// Selection is independent of scroll position, including short catalogs.
    pub(super) fn compact_model_back_header(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let theme = Theme::of(cx).for_popup();
        let groups = self.compact_tabs(cx);
        let current = if self.model_rail == ModelRail::Favorites {
            None
        } else {
            self.effective_harness(cx)
        };
        // Many providers overflow the strip: keep the viewed tab in sight
        // when switching providers, without fighting a manual sideways scroll.
        let viewed_ix = groups.iter().position(|group| current == *group);
        if viewed_ix != self.compact_strip_viewed {
            self.compact_strip_viewed = viewed_ix;
            if let Some(ix) = viewed_ix {
                self.compact_strip_scroll.scroll_to_item(ix);
            }
        }
        let strip = (!groups.is_empty()).then(|| {
            let tabs = div()
                .id("compact-group-strip")
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .items_center()
                .gap(px(popover::MENU_GAP))
                .overflow_x_scroll()
                .track_scroll(&self.compact_strip_scroll)
                .children(groups.into_iter().enumerate().map(|(ix, group)| {
                    let viewed = current == group;
                    let (icon, tint, name): (&'static str, Option<gpui::Hsla>, SharedString) =
                        match group {
                            None => (crate::icons::STAR_BOLD, None, "Starred".into()),
                            Some(harness) => {
                                let (icon, tint) = harness_brand_icon(harness);
                                let name = self
                                    .rail_descriptors(cx)
                                    .into_iter()
                                    .find(|d| d.id == harness)
                                    .map(|d| d.name)
                                    .unwrap_or_default();
                                (icon, tint, name.into())
                            }
                        };
                    let hint = name.clone();
                    div()
                        .id(SharedString::from(format!("compact-group-{ix}")))
                        .role(gpui::Role::Button)
                        .aria_label(SharedString::from(format!("Show {name} models")))
                        .aria_selected(viewed)
                        .tooltip(move |_, cx| cx.new(|_| PickerHint(hint.clone())).into())
                        // A row-height square with the rows' corners, so the
                        // strip sits on the card's grid like the list below.
                        .flex_none()
                        .size(px(COMPACT_ROW_HEIGHT))
                        .rounded(px(popover::MENU_ITEM_RADIUS))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .when(viewed, |el| el.bg(crate::theme::ink(0.08)))
                        .when(!viewed, |el| {
                            el.opacity(0.7)
                                .hover(|s| s.bg(crate::theme::ink(0.05)).opacity(1.0))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_compact_tab(group, cx);
                        }))
                        .child(crate::icons::icon(icon).size(px(14.0)).text_color(
                            tint.unwrap_or(if viewed { theme.text } else { theme.text_muted }),
                        ))
                }));
            // Each side fades over the tabs hidden past it, up to one tab
            // pitch, reaching zero exactly at the clip edge.
            crate::edge_fade::edge_faded(
                COMPACT_ROW_HEIGHT + popover::MENU_GAP,
                false,
                false,
                tabs,
            )
                .fade_left(true)
                .fade_right(true)
                .fade_scroll_x(&self.compact_strip_scroll)
        });
        // The card's inset on every side of one row-height strip: the same
        // 4pt edge and 32pt pitch as the list rows and the panel's buttons.
        div()
            .h(px(COMPACT_ROW_HEIGHT + 2.0 * popover::CARD_INSET))
            .flex_none()
            .px(px(popover::CARD_INSET))
            .flex()
            .items_center()
            .gap(px(popover::MENU_GAP))
            .child(
                div()
                    .id("compact-model-back")
                    .role(gpui::Role::Button)
                    .aria_label("Back")
                    .flex_none()
                    .size(px(COMPACT_ROW_HEIGHT))
                    .rounded(px(popover::MENU_ITEM_RADIUS))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(crate::theme::ink(0.05)))
                    .on_click(cx.listener(|this, _, _, cx| this.show_compact_panel(cx)))
                    .child(
                        crate::icons::icon(crate::icons::ALT_ARROW_LEFT)
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    ),
            )
            .children(strip)
    }

    pub(super) fn compact_fast_choice(&self, cx: &App) -> Option<(String, String, bool, bool)> {
        let (option, on, off) = self
            .selected_model(cx)?
            .options
            .iter()
            .find_map(|o| fast_mode_values(o).map(|(on, off)| (o, on, off)))?;
        let options = self.explicit_options(cx);
        let fast = options
            .get(&option.id)
            .and_then(|v| v.as_str())
            .unwrap_or(&option.default_choice)
            == on;
        let next = if fast { off } else { on };
        Some((
            option.id.clone(),
            next.to_owned(),
            next == option.default_choice,
            fast,
        ))
    }

    /// The selected model's fast-mode option id, whichever form it takes.
    pub(super) fn fast_option_id(&self, cx: &App) -> Option<String> {
        self.selected_model(cx)?
            .options
            .iter()
            .find(|o| fast_mode_values(o).is_some())
            .map(|o| o.id.clone())
    }

    /// The panel's title for a [`ModelName`]. Loading also draws a ghost
    /// bar in the name's place, as the composer chip does.
    fn compact_title_text(name: &ModelName) -> SharedString {
        match name {
            ModelName::Named(label) => label.clone(),
            ModelName::Loading => "Loading models…".into(),
            ModelName::None { no_agents: true } => "No agents available".into(),
            ModelName::None { .. } => "Select model".into(),
        }
    }

    /// The slider's stops: the model's reasoning ladder, else an option
    /// shaped like one (Cursor's `effort`/`reasoning` choices), so every
    /// model with an effort gets the same slider.
    pub(super) fn compact_effort(&self, cx: &App) -> Option<CompactEffort> {
        let levels = self.trait_ladder(cx);
        if !levels.is_empty() {
            let current = self.effective_reasoning(cx);
            let default = default_reasoning(&levels);
            return Some(CompactEffort {
                labels: levels.iter().map(|l| reasoning_label(*l).into()).collect(),
                selected: levels.iter().position(|l| Some(*l) == current).unwrap_or(0),
                default: levels.iter().position(|l| Some(*l) == default),
                peaks: levels.iter().map(|l| effort_slider::is_peak_level(*l)).collect(),
                stops: EffortStops::Reasoning(levels),
            });
        }
        let option = self
            .selected_model(cx)?
            .options
            .iter()
            .find(|o| EFFORT_OPTION_IDS.contains(&o.id.as_str()) && o.choices.len() > 1)?;
        let options = self.explicit_options(cx);
        let current = options
            .get(&option.id)
            .and_then(|v| v.as_str())
            .unwrap_or(&option.default_choice);
        Some(CompactEffort {
            labels: option
                .choices
                .iter()
                .map(|c| SharedString::from(c.label.clone()))
                .collect(),
            selected: option
                .choices
                .iter()
                .position(|c| c.id == current)
                .unwrap_or(0),
            default: option
                .choices
                .iter()
                .position(|c| c.id == option.default_choice),
            peaks: option
                .choices
                .iter()
                .map(|c| effort_slider::is_peak_id(&c.id))
                .collect(),
            stops: EffortStops::Option {
                id: option.id.clone(),
                choices: option.choices.iter().map(|c| c.id.clone()).collect(),
                default: option.default_choice.clone(),
            },
        })
    }

    /// Option ids the compact card draws as its own controls, not rows.
    pub(super) fn compact_hidden_options(&self, cx: &App) -> Vec<String> {
        let effort = match self.compact_effort(cx).map(|e| e.stops) {
            Some(EffortStops::Option { id, .. }) => Some(id),
            _ => None,
        };
        self.fast_option_id(cx).into_iter().chain(effort).collect()
    }

    /// The step the slider shows: the one under a held pointer, else the
    /// applied one.
    pub(super) fn shown_effort_step(&self, effort: &CompactEffort) -> usize {
        self.effort_dragging
            .then_some(self.compact_motion.effort_pending)
            .flatten()
            .unwrap_or(effort.selected)
            .min(effort.labels.len().saturating_sub(1))
    }

    /// Pressing the track picks the step under the pointer and the thumb
    /// springs to it; nothing applies until release.
    pub(super) fn effort_press(&mut self, x: gpui::Pixels, cx: &mut Context<Self>) {
        self.effort_dragging = true;
        self.effort_against_end = false;
        self.compact_motion.drag_fraction = None;
        self.compact_motion.effort_pending = None;
        self.effort_point(x, false, cx);
    }

    /// While held, the thumb follows the pointer, ticking as it crosses
    /// each step.
    pub(super) fn effort_drag(&mut self, x: gpui::Pixels, cx: &mut Context<Self>) {
        if self.effort_dragging {
            self.effort_point(x, true, cx);
        }
    }

    fn effort_point(&mut self, x: gpui::Pixels, follow: bool, cx: &mut Context<Self>) {
        let (Some(bounds), Some(effort)) = (self.effort_bounds, self.compact_effort(cx)) else {
            return;
        };
        let count = effort.labels.len();
        let (x, width, height) = (
            f32::from(x - bounds.left()),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
        );
        let Some(step) = effort_slider::step_at(x, width, height, count) else {
            return;
        };
        if follow {
            let fraction = effort_slider::fraction_at(x, width, height);
            self.compact_motion.drag_fraction = Some(if count > 1 { fraction } else { 0.0 });
            // Pushing past either end bumps once, until the pointer comes back.
            let against = effort_slider::past_end(x, width, height);
            if against && !self.effort_against_end {
                crate::haptics::perform(crate::haptics::Feedback::Edge);
            }
            self.effort_against_end = against;
        }
        let feedback = effort_slider::detent_feedback(self.shown_effort_step(&effort), step, &effort.peaks);
        self.effort_feedback(feedback, cx);
        self.compact_motion.effort_pending = Some(step);
        cx.notify();
    }

    /// Plays a detent's trackpad feedback; crossing into the top tiers is a
    /// firmer double knock.
    fn effort_feedback(&mut self, feedback: Option<crate::haptics::Feedback>, cx: &mut Context<Self>) {
        let Some(feedback) = feedback else { return };
        crate::haptics::perform(feedback);
        if feedback == crate::haptics::Feedback::Peak {
            cx.spawn(async move |_, cx| {
                cx.background_executor().timer(PEAK_KNOCK_GAP).await;
                crate::haptics::selection_step();
            })
            .detach();
        }
    }

    /// Release applies the step the thumb springs to (the haptics already
    /// ticked on the way there).
    pub(super) fn effort_release(&mut self, cx: &mut Context<Self>) {
        if !self.effort_dragging {
            return;
        }
        self.effort_dragging = false;
        self.compact_motion.drag_fraction = None;
        if let Some(step) = self.compact_motion.effort_pending.take()
            && let Some(effort) = self.compact_effort(cx)
        {
            self.apply_compact_effort(effort, step, cx);
        }
        cx.notify();
    }

    /// Effort and fast mode have their own controls in the compact card.
    pub(super) fn compact_option_visible(id: &ModelSetting, hidden: &[String]) -> bool {
        !matches!(id, ModelSetting::Reasoning)
            && !matches!(id, ModelSetting::Option(id) if hidden.contains(id))
    }

    /// Apply a step from the keyboard, the reset button or a test, with its
    /// trackpad feedback.
    pub(super) fn pick_compact_effort(
        &mut self,
        effort: CompactEffort,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let feedback = effort_slider::detent_feedback(effort.selected, index, &effort.peaks);
        if self.apply_compact_effort(effort, index, cx) {
            self.effort_feedback(feedback, cx);
        }
    }

    /// Apply a step through the same mutation the menus use; false when it
    /// is already the selection.
    fn apply_compact_effort(
        &mut self,
        effort: CompactEffort,
        index: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        if index == effort.selected {
            return false;
        }
        match effort.stops {
            EffortStops::Reasoning(levels) => {
                let Some(level) = levels.get(index).copied() else {
                    return false;
                };
                self.pick_reasoning(level, cx);
            }
            EffortStops::Option {
                id,
                choices,
                default,
            } => {
                let Some(choice) = choices.get(index).cloned() else {
                    return false;
                };
                let is_default = choice == default;
                self.pick_option(id, choice, is_default, cx);
            }
        }
        true
    }

    pub(super) fn render_compact_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hidden = self.compact_hidden_options(cx);
        let options = self
            .setting_groups(cx)
            .iter()
            .filter(|g| Self::compact_option_visible(&g.id, &hidden))
            .count();
        let effort = self.compact_effort(cx).is_some();
        let panel_height = CARD_BORDERS
            + header_block()
            + if effort { EFFORT_BLOCK } else { 0.0 }
            + if options == 0 {
                0.0
            } else {
                OPTIONS_GAP + options_height(options)
            };
        let page = if self.compact_model_list {
            CompactPage::Models
        } else {
            CompactPage::Panel
        };
        let target = self.menu_geometry().height.min(match page {
            CompactPage::Models => {
                LIST_HEADER + compact_list_height(self.model_rows_len(cx)) + CARD_BORDERS
            }
            CompactPage::Panel => panel_height,
        });
        let (height, moving) = ScalarTransition::sample(
            &mut self.compact_motion.height,
            target,
            std::time::Instant::now(),
            cx.reduce_motion(),
        );
        self.compact_motion.frame_height = height;
        if moving {
            window.request_animation_frame();
        }
        let content = match page {
            CompactPage::Models => self.render_harness_model_popover(cx),
            CompactPage::Panel => self.render_compact_model_panel(window, cx),
        };
        let now = std::time::Instant::now();
        if self.compact_motion.page.is_some_and(|last| last != page) {
            self.compact_motion.reveal = Some(ScalarTransition {
                from: 0.0,
                to: 1.0,
                started: now,
            });
        }
        self.compact_motion.page = Some(page);
        let (reveal, revealing) = ScalarTransition::sample(
            &mut self.compact_motion.reveal,
            1.0,
            now,
            cx.reduce_motion(),
        );
        if revealing {
            window.request_animation_frame();
        }
        // The new page fades in while the card resizes to it: the swap
        // never shows at full strength and nothing slides sideways against
        // the vertical resize. Initial opening uses the popover entrance.
        let content = div().opacity(reveal).child(content).into_any_element();
        self.popover_frame_flush(COMPACT_WIDTH, content, cx)
    }

    /// The panel's shortcuts, live only while it is the picker's page: Up
    /// and Down open the model list on the model beside the selected one,
    /// Tab cycles providers, Left/Right (Home/End) set the effort, F toggles
    /// fast mode.
    pub(super) fn compact_panel_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        self.compact_keyboard = true;
        match event.keystroke.key.as_str() {
            "escape" => self.animate_close(cx),
            "f" if !event.keystroke.modifiers.modified() => {
                let Some((option, choice, default, _)) = self.compact_fast_choice(cx) else {
                    return;
                };
                self.pick_option(option, choice, default, cx);
            }
            "up" | "down" => {
                self.show_compact_models(cx);
                let delta = if event.keystroke.key == "up" { -1 } else { 1 };
                if let Some(next) =
                    popover::menu_step(Some(self.active), self.model_rows_len(cx), delta)
                {
                    self.active = next;
                    self.model_scroll
                        .scroll_to_item(next, gpui::ScrollStrategy::Nearest);
                }
            }
            "enter" | "space" => self.show_compact_models(cx),
            "tab" => {
                let delta = if event.keystroke.modifiers.shift {
                    -1
                } else {
                    1
                };
                self.cycle_compact_provider(delta, cx);
            }
            "left" | "right" | "home" | "end" => {
                if let Some(effort) = self.compact_effort(cx) {
                    let last = effort.labels.len().saturating_sub(1);
                    let next = match event.keystroke.key.as_str() {
                        "left" => effort.selected.saturating_sub(1),
                        "right" => (effort.selected + 1).min(last),
                        "home" => 0,
                        _ => last,
                    };
                    self.pick_compact_effort(effort, next, cx);
                }
            }
            _ => return,
        }
        cx.notify();
        cx.stop_propagation();
    }

    /// Tab on the panel: the next provider on offer, wrapping, at its
    /// last-used model. A chat's fixed provider stays put.
    fn cycle_compact_provider(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.harness_locked(cx) {
            return;
        }
        let providers = self.rail_descriptors(cx);
        let current = self
            .effective_harness(cx)
            .and_then(|harness| providers.iter().position(|d| d.id == harness));
        if let Some(next) = popover::menu_step(current, providers.len(), delta) {
            self.pick_compact_provider(providers[next].id, cx);
        }
    }

    pub(super) fn render_compact_model_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).for_popup();
        let compact_effort = self.compact_effort(cx);
        let shown = compact_effort
            .as_ref()
            .map_or(0, |e| self.shown_effort_step(e));
        let (levels, selected, default, peaks) = compact_effort.map_or(
            (Vec::new(), 0, None, Vec::new()),
            |e| (e.labels, e.selected, e.default, e.peaks),
        );
        let name = self.model_name(cx);
        let label = Self::compact_title_text(&name);
        let name_element: AnyElement = if name == ModelName::Loading {
            popover::skeleton_bar(72.0, cx.entity_id(), cx)
        } else {
            div()
                .min_w_0()
                .truncate()
                .child(label.clone())
                .into_any_element()
        };
        let effort: SharedString = levels
            .get(selected)
            .cloned()
            .unwrap_or_else(|| "Default".into());
        let model_accessible: SharedString =
            format!("{} · {} · Change model", effort, label).into();
        let now = std::time::Instant::now();
        let reduced = cx.reduce_motion();
        // Fast mode breathes an accent halo under the effort thumb (the
        // track's own heat says how hard the model thinks).
        let fast = self
            .compact_fast_choice(cx)
            .is_some_and(|(_, _, _, fast)| fast);
        let (intensity, powering) = ScalarTransition::sample(
            &mut self.compact_motion.energy,
            if fast { 1.0 } else { 0.0 },
            now,
            reduced,
        );
        if powering {
            window.request_animation_frame();
        }
        let dt = self
            .compact_motion
            .shimmer_last_frame
            .replace(now)
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32().min(0.05));
        let alive =
            intensity > 0.001 && !reduced && window.is_window_active() && self.open.is_open();
        if alive {
            self.compact_motion.shimmer_phase =
                (self.compact_motion.shimmer_phase + dt * (0.32 + 0.22 * intensity)).fract();
            window.request_animation_frame();
        }
        let phase = self.compact_motion.shimmer_phase;
        let breath = if alive {
            0.5 + 0.5 * (std::f32::consts::TAU * phase).sin()
        } else {
            0.5
        };
        // The model's name titles the panel as one button filling the row up
        // to fast mode, its text on the track's and option rows' edge; fast
        // mode is a small icon button whose glyph ends where the track does.
        // The chevron leans toward the list on hover.
        let model_key: SharedString = format!("compact-model-link-{}", cx.entity_id()).into();
        let hover = motion::hover_t(&model_key);
        let lean = motion::EASE_OUT_QUINT.eval(hover);
        let title = div()
            .id("compact-select-model")
            .role(gpui::Role::Button)
            .aria_label(model_accessible)
            .flex_1()
            .min_w_0()
            .h_full()
            .px(px(8.0))
            .rounded(px(popover::MENU_ITEM_RADIUS))
            .bg(crate::theme::ink(0.05 * hover))
            .flex()
            .flex_col()
            .items_start()
            .justify_center()
            .cursor_pointer()
            .on_hover(motion::hover_listener(model_key))
            .on_click(cx.listener(|this, _, _, cx| {
                this.compact_keyboard = false;
                this.show_compact_models(cx);
            }))
            .child(
                div()
                    .max_w_full()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(3.0))
                    .text_size(crate::typography::ui_rems(14.0))
                    .line_height(px(17.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(name_element)
                    .child(
                        // Leans 3pt toward the list and firms up on hover.
                        div()
                            .flex_none()
                            .relative()
                            .left(px(3.0 * lean))
                            .opacity(0.55 + 0.45 * lean)
                            .child(
                                crate::icons::icon(crate::icons::ALT_ARROW_RIGHT)
                                    .size(px(10.0))
                                    .text_color(motion::mix(theme.text_muted, theme.text, lean)),
                            ),
                    ),
            );
        let mut fast_button = None;
        if let Some((option, choice, default, fast)) = self.compact_fast_choice(cx) {
            let (fast_t, fading) = ScalarTransition::sample(
                &mut self.compact_motion.fast,
                if fast { 1.0 } else { 0.0 },
                now,
                reduced,
            );
            if fading {
                window.request_animation_frame();
            }
            let fast_key: SharedString = format!("compact-fast-{}", cx.entity_id()).into();
            let fast_hover = motion::hover_t(&fast_key);
            fast_button = Some(
                div()
                    .id("compact-fast")
                    .role(gpui::Role::Button)
                    .aria_label("Fast mode")
                    .aria_toggled(if fast {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .tooltip(move |_, cx| {
                        cx.new(|_| {
                            PickerHint(
                                if fast {
                                    "Fast mode on · Turn off"
                                } else {
                                    "Fast mode off · Turn on"
                                }
                                .into(),
                            )
                        })
                        .into()
                    })
                    // Bare like the model button beside it: a plate only on
                    // hover. The glyph's colour is the state, and the
                    // slider's fill carries the accent.
                    .w(px(FAST_BUTTON_WIDTH))
                    .h_full()
                    .flex_none()
                    .rounded(px(popover::MENU_ITEM_RADIUS))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .bg(crate::theme::ink(0.05 * fast_hover))
                    .on_hover(motion::hover_listener(fast_key))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.compact_keyboard = false;
                        this.pick_option(option.clone(), choice.clone(), default, cx);
                    }))
                    .child({
                        let color = motion::mix(
                            motion::mix(theme.text_muted, theme.text, fast_hover),
                            theme.accent,
                            fast_t,
                        );
                        // On fills the bolt: the solid twin fades in over the
                        // outline, which shares its silhouette.
                        div()
                            .relative()
                            .size(px(15.0))
                            .child(
                                crate::icons::icon(crate::icons::FAST_TIER)
                                    .size(px(15.0))
                                    .text_color(color),
                            )
                            .when(fast_t > 0.0, |el| {
                                el.child(
                                    crate::icons::icon(crate::icons::FAST_TIER_BOLD)
                                        .absolute()
                                        .top_0()
                                        .left_0()
                                        .size(px(15.0))
                                        .text_color(color.opacity(fast_t)),
                                )
                            })
                    }),
            );
        }
        let header = div()
            .h(px(HEADER_HEIGHT))
            .flex_none()
            .flex()
            .gap(px(popover::CARD_INSET))
            .child(title)
            .children(fast_button);
        let mut panel = div()
            .p(px(popover::CARD_INSET))
            .flex()
            .flex_col()
            .child(header);
        if !levels.is_empty() {
            panel = panel.child(self.render_compact_effort(
                &theme,
                EffortView {
                    levels: &levels,
                    selected,
                    shown,
                    default,
                    peaks: &peaks,
                    fast_halo: intensity * (0.55 + 0.45 * breath),
                    dt,
                },
                window,
                cx,
            ));
        }
        let hidden = self.compact_hidden_options(cx);
        let option_count = self
            .setting_groups(cx)
            .iter()
            .filter(|g| Self::compact_option_visible(&g.id, &hidden))
            .count();
        if option_count > 0 {
            let options = self.render_traits_sections(cx);
            let scrollbar = popover::rail(self, "compact-options-scrollbar", &theme, cx);
            let chrome = CARD_BORDERS
                + header_block()
                + if levels.is_empty() { 0.0 } else { EFFORT_BLOCK }
                + OPTIONS_GAP;
            // Space, not a rule, separates the options from the effort group.
            panel = panel.child(
                div().mt(px(OPTIONS_GAP)).child(
                    popover::menu_scroll_host("compact-options-host")
                        .on_hover(cx.listener(Self::on_menu_list_hover))
                        .child(popover::faded_menu_list(
                            &self.menu_scroll,
                            popover::menu_scroll_list("compact-options", &self.menu_scroll)
                                .max_h(px(options_height(option_count)
                                    .min((self.menu_geometry().height - chrome).max(0.0))))
                                .child(options),
                        ))
                        .children(scrollbar),
                ),
            );
        }
        panel.into_any_element()
    }
}

/// What the effort block shows this frame.
struct EffortView<'a> {
    levels: &'a [SharedString],
    /// The applied step.
    selected: usize,
    /// The step under a held pointer, else the applied one.
    shown: usize,
    default: Option<usize>,
    peaks: &'a [bool],
    /// Fast mode's breathing halo under the thumb.
    fast_halo: f32,
    /// Seconds since the last frame, for the track's clock.
    dt: f32,
}

impl Pickers {
    /// The effort's header (its icon and level in the tier's colour, reset
    /// at the far end) over the track. The sparks animate only while the
    /// panel is open in an active window, and hold still under reduced
    /// motion.
    fn render_compact_effort(
        &mut self,
        theme: &Theme,
        view: EffortView<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let EffortView {
            levels,
            selected,
            shown,
            default,
            peaks,
            fast_halo,
            dt,
        } = view;
        let now = std::time::Instant::now();
        let reduced = cx.reduce_motion();
        let count = levels.len();
        let target = effort_slider::step_fraction(shown, count);
        let (mut fraction, moving) =
            effort_slider::Spring::sample(&mut self.compact_motion.effort, target, now, reduced);
        if self.effort_dragging
            && let Some(dragged) = self.compact_motion.drag_fraction
        {
            // The thumb follows the pointer; release springs on from here.
            fraction = dragged;
            effort_slider::Spring::hold(&mut self.compact_motion.effort, dragged, now);
        }
        let peak = if peaks.get(shown).copied().unwrap_or(false) {
            1.0
        } else {
            0.0
        };
        let (top, topping) = effort_slider::Fade::sample(
            &mut self.compact_motion.top,
            peak,
            effort_slider::TOP_FADE,
            now,
            reduced,
        );
        let (press, pressing) = ScalarTransition::sample(
            &mut self.compact_motion.press,
            if self.effort_dragging { 1.0 } else { 0.0 },
            now,
            reduced,
        );
        let resettable = effort_slider::reset_visible(shown, default);
        let (reset_t, resetting) = ScalarTransition::sample(
            &mut self.compact_motion.reset,
            if resettable { 1.0 } else { 0.0 },
            now,
            reduced,
        );
        // The heat moves only while it can be seen, at a pace set by the
        // effort; reduced motion leaves one still frame.
        let alive = !reduced && window.is_window_active() && self.open.is_open();
        if alive {
            self.compact_motion.heat.advance(dt, fraction);
        }
        if moving || topping || pressing || resetting || alive {
            window.request_animation_frame();
        }

        let tier = effort_slider::label_heat(fraction);
        let level = levels.get(shown).cloned().unwrap_or_default();
        let applied = levels.get(selected).cloned().unwrap_or_default();
        let reset = default.filter(|_| reset_t > 0.001).map(|default| {
            let name = levels.get(default).cloned().unwrap_or_default();
            let hint: SharedString = format!("Reset to the model's default ({name})").into();
            let tooltip = hint.clone();
            let key: SharedString = format!("compact-effort-reset-{}", cx.entity_id()).into();
            let hover = motion::hover_t(&key);
            div()
                .id("compact-effort-reset")
                .role(gpui::Role::Button)
                .aria_label(hint)
                .tooltip(move |_, cx| cx.new(|_| PickerHint(tooltip.clone())).into())
                .flex_none()
                .size(px(RESET_BUTTON_SIZE))
                .rounded(px(popover::MENU_ITEM_RADIUS))
                .flex()
                .items_center()
                .justify_center()
                .opacity(reset_t)
                .bg(crate::theme::ink(0.05 * hover))
                .on_hover(motion::hover_listener(key))
                .when(resettable, |el| {
                    el.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| {
                        this.compact_keyboard = false;
                        if let Some(effort) = this.compact_effort(cx) {
                            this.pick_compact_effort(effort, default, cx);
                        }
                        cx.notify();
                    }))
                })
                .child(
                    crate::icons::icon(crate::icons::RESET)
                        .size(px(14.0))
                        .text_color(motion::mix(theme.text_muted, theme.text, hover)),
                )
        });
        // The level names the track in its tier's colour, rolling between
        // values like the composer chips as the thumb crosses steps.
        let header = div()
            .h(px(EFFORT_HEADER_HEIGHT))
            .flex_none()
            .pl(px(8.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(
                crate::icons::icon(crate::icons::TUNING)
                    .flex_none()
                    .size(px(13.0))
                    .text_color(tier),
            )
            .child(
                // Secondary to the model's name above it.
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(crate::typography::ui_rems(12.5))
                    .line_height(px(15.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(tier)
                    .child(crate::roll_text::roll_text(
                        format!("compact-effort-title-{}", cx.entity_id()),
                        level.clone(),
                        reduced,
                    )),
            )
            .children(reset);

        let track = effort_slider::Track {
            fraction,
            count,
            top,
            clock: self.compact_motion.heat,
            fast_halo,
            press,
        };
        let paint_theme = theme.clone();
        let entity = cx.entity().downgrade();
        let drag_entity = entity.clone();
        let slider = div()
            .id("compact-effort-slider")
            .role(gpui::Role::Slider)
            .aria_label("Reasoning effort")
            .aria_value(applied)
            .aria_numeric_value(selected as f64)
            .aria_min_numeric_value(0.0)
            .aria_max_numeric_value(count.saturating_sub(1) as f64)
            .aria_numeric_value_step(1.0)
            .aria_orientation(gpui::Orientation::Horizontal)
            .aria_description("Use Left and Right to adjust reasoning; Home and End select the first and last levels")
            .relative()
            .h(px(SLIDER_HEIGHT))
            .cursor_pointer()
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    this.compact_keyboard = false;
                    this.active = NO_ACTIVE_ROW;
                    this.effort_press(event.position.x, cx);
                    cx.stop_propagation();
                }),
            )
            .child(
                gpui::canvas(
                    move |bounds, _, cx| {
                        let _ = entity.update(cx, |this, _| this.effort_bounds = Some(bounds));
                    },
                    move |bounds, _, window, _| {
                        effort_slider::paint(window, bounds, &paint_theme, track);
                        let release = drag_entity.clone();
                        window.on_mouse_event(move |_: &gpui::MouseUpEvent, phase, _, cx| {
                            if phase == gpui::DispatchPhase::Bubble {
                                let _ = release.update(cx, |this, cx| this.effort_release(cx));
                            }
                        });
                        let entity = drag_entity.clone();
                        window.on_mouse_event(
                            move |event: &gpui::MouseMoveEvent, phase, _, cx| {
                                if phase == gpui::DispatchPhase::Bubble
                                    && event.pressed_button == Some(gpui::MouseButton::Left)
                                {
                                    let _ = entity.update(cx, |this, cx| {
                                        this.effort_drag(event.position.x, cx)
                                    });
                                }
                            },
                        );
                    },
                )
                .absolute()
                .inset_0(),
            );
        // The track starts on the same edge as the title and row labels.
        div()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .px(px(8.0))
                    .pt(px(SLIDER_TOP))
                    .pb(px(SLIDER_BOTTOM))
                    .child(slider),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_transitions_start_settled_and_retarget_without_jumping() {
        let start = std::time::Instant::now();
        let mut state = None;
        assert_eq!(
            ScalarTransition::sample(&mut state, 0.5, start, false),
            (0.5, false)
        );
        assert_eq!(
            ScalarTransition::sample(&mut state, 1.0, start, false).0,
            0.5
        );
        let middle = start + motion::RESIZE.total().mul_f32(motion::speed_scale() * 0.5);
        let before = state.as_ref().unwrap().value(middle);
        let retargeted = ScalarTransition::sample(&mut state, 0.0, middle, false).0;
        assert!((before - retargeted).abs() < 0.0001);
        assert_eq!(
            ScalarTransition::sample(&mut state, 0.0, middle, true),
            (0.0, false)
        );
    }
}

struct PickerHint(SharedString);
impl Render for PickerHint {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_popup();
        let card = popover::popover_card(&theme)
            .max_w(px(320.0))
            .p(px(8.0))
            .text_size(crate::typography::ui_rems(12.0))
            .line_height(px(17.0))
            .text_color(theme.text)
            .child(self.0.clone());
        crate::frost::frosted(popover::CARD_RADIUS, crate::frost::MENU_BLUR, card)
    }
}
