//! `@chat:` references in rendered Markdown, drawn as pills: the chat's
//! harness mark, its title and its live status, on a quiet rounded wash.
//!
//! The pill is text. Flattening substitutes the written token with reserved
//! slot spaces around the title (so the shaped line alone fixes the pill's
//! geometry and nothing reflows on hover), and [`ChatPills`] paints the mark,
//! the status and the click target over those slots. Copy and selection map
//! the pill back to the written token through the flat text's offset map.
//!
//! Which chat a token names — and its title — is fixed at flatten time, so
//! the owner drops the flattened rows naming a token when that changes
//! ([`crate::chat_pill::layout_key`]). Status is read per paint.

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, Element, ElementId, EntityId, GlobalElementId,
    InspectorElementId, LayoutId, Pixels, SharedString, TextLayout, Window, div, point, prelude::*,
    px, size,
};

use super::render::{INLINE_CODE_INSET_Y, range_rects};
use crate::chat_pill::ChatRef;
use crate::theme::Theme;

const NBSP: char = '\u{00A0}';
/// Leading slot: the harness mark plus its breathing room, in ems.
const ICON_SLOT_EMS: f32 = 1.25;
/// Trailing slot: the status mark, in ems.
const STATUS_SLOT_EMS: f32 = 1.05;
/// Titles longer than this are cut with an ellipsis; the tooltip keeps all.
const MAX_LABEL_CHARS: usize = 40;
const MARK_SIZE: f32 = 12.0;
pub(crate) const PILL_RADIUS: f32 = super::render::INLINE_CODE_RADIUS;

/// Transcript wiring for chat pills. `resolve` is a per-render snapshot;
/// `open` activates a known chat (the transcript opens it beside this one).
#[derive(Clone)]
pub struct ChatUi {
    pub resolve: Rc<dyn Fn(&str) -> ChatRef>,
    pub open: Rc<dyn Fn(&str, &mut Window, &mut App)>,
    /// The view leasing the status spinners' frames.
    pub owner: EntityId,
}

/// One pill, in displayed-text coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatPillSpan {
    pub range: Range<usize>,
    pub icon_slot: Range<usize>,
    pub status_slot: Range<usize>,
    /// The id as written; [`ChatUi::resolve`] names it live per paint.
    pub written: SharedString,
    pub known: bool,
}

fn slot(ems: f32) -> String {
    let count = (ems / super::math::nbsp_em()).ceil().max(1.0) as usize;
    std::iter::repeat_n(NBSP, count).collect()
}

/// The text a resolved reference shows: icon slot, title, status slot. The
/// title's spaces are non-breaking so a pill never splits across lines.
pub(super) fn pill_text(chat: &ChatRef) -> (String, Range<usize>, Range<usize>) {
    let title = crate::chat_pill::display_title(&chat.title, chat.harness);
    let mut label: String = title.chars().take(MAX_LABEL_CHARS).collect();
    if title.chars().count() > MAX_LABEL_CHARS {
        label = label.trim_end().to_owned();
        label.push('…');
    }
    let label = label.replace(' ', "\u{00A0}");
    let lead = slot(ICON_SLOT_EMS);
    let trail = if chat.known {
        slot(STATUS_SLOT_EMS)
    } else {
        slot(0.45)
    };
    let text = format!("{lead}{label}{trail}");
    let icon = 0..lead.len();
    let status = text.len() - trail.len()..text.len();
    (text, icon, status)
}

/// Paints each pill's mark and status over its slots and makes known pills
/// clickable. Wraps the text element; layout is the child's.
pub(crate) struct ChatPills {
    pub id: SharedString,
    pub child: AnyElement,
    pub layout: TextLayout,
    pub pills: Vec<ChatPillSpan>,
    pub ui: ChatUi,
}

impl IntoElement for ChatPills {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for ChatPills {
    type RequestLayoutState = ();
    type PrepaintState = Vec<AnyElement>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone().into())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<AnyElement> {
        self.child.prepaint(window, cx);
        let theme = Theme::of(cx).clone();
        let mut overlays = Vec::new();
        for (ix, pill) in self.pills.iter().enumerate() {
            let chat = (self.ui.resolve)(&pill.written);
            // Mark: in the leading slot of the pill's first line.
            if let Some(rect) = range_rects(&self.layout, &pill.icon_slot, 0.0, INLINE_CODE_INSET_Y)
                .first()
                .copied()
            {
                let mark = match chat.harness.filter(|_| chat.known) {
                    Some(harness) => crate::chat_pill::harness_mark(harness, MARK_SIZE, &theme)
                        .into_any_element(),
                    None => crate::icons::icon(crate::icons::BOT)
                        .size(px(MARK_SIZE))
                        .text_color(theme.text_faint)
                        .into_any_element(),
                };
                let mut el = div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .pl(px(2.0))
                    .child(mark)
                    .into_any_element();
                el.prepaint_as_root(
                    rect.origin,
                    rect.size.map(AvailableSpace::Definite),
                    window,
                    cx,
                );
                overlays.push(el);
            }
            // Status: in the trailing slot of the pill's last line.
            if chat.known
                && let Some(rect) =
                    range_rects(&self.layout, &pill.status_slot, 0.0, INLINE_CODE_INSET_Y)
                        .last()
                        .copied()
            {
                let mut el = div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .pr(px(2.0))
                    .child(crate::chat_pill::mark_element(
                        format!("{}-{ix}", self.id),
                        chat.mark,
                        self.ui.owner,
                        &theme,
                        cx,
                    ))
                    .into_any_element();
                el.prepaint_as_root(
                    rect.origin,
                    rect.size.map(AvailableSpace::Definite),
                    window,
                    cx,
                );
                overlays.push(el);
            }
            if !chat.known {
                continue;
            }
            // The click target covers every line the pill spans; the first
            // line is the pill's one keyboard stop and accessible link.
            for (line, rect) in range_rects(&self.layout, &pill.range, 0.0, INLINE_CODE_INSET_Y)
                .into_iter()
                .enumerate()
            {
                let open = self.ui.open.clone();
                let chat_id = chat.chat_id.clone();
                let tooltip = SharedString::from(chat.spoken());
                let hover = theme.ink(0.06);
                let target = div()
                    .id(SharedString::from(format!("{}-hit-{ix}-{line}", self.id)))
                    .size_full()
                    .rounded(px(PILL_RADIUS))
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .tooltip(crate::settings::widgets::text_tooltip(tooltip.clone()))
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        open(&chat_id, window, cx);
                    });
                let mut el = if line == 0 {
                    crate::chat_pill::tab_stop(
                        target
                            .role(gpui::Role::Link)
                            .aria_label(SharedString::from(format!("Open {tooltip}"))),
                        &theme,
                    )
                    .into_any_element()
                } else {
                    target.into_any_element()
                };
                el.prepaint_as_root(
                    point(rect.origin.x, rect.origin.y),
                    size(rect.size.width, rect.size.height).map(AvailableSpace::Definite),
                    window,
                    cx,
                );
                overlays.push(el);
            }
        }
        overlays
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        overlays: &mut Vec<AnyElement>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
        for overlay in overlays {
            overlay.paint(window, cx);
        }
    }
}

/// Every written chat id a block references, for the owner's layout
/// fingerprint.
pub(crate) fn block_chat_ids<'a>(block: &'a super::parser::Block, out: &mut Vec<&'a str>) {
    use super::parser::Block;
    let runs = |runs: &'a [super::parser::InlineRun], out: &mut Vec<&'a str>| {
        out.extend(runs.iter().filter_map(|run| run.style.chat.as_deref()));
    };
    match block {
        Block::Paragraph { runs: r } | Block::Heading { runs: r, .. } => runs(r, out),
        Block::BlockQuote { children } => children.iter().for_each(|b| block_chat_ids(b, out)),
        Block::List { items, .. } => items
            .iter()
            .flatten()
            .for_each(|b| block_chat_ids(b, out)),
        Block::Table { header, rows, .. } => header
            .iter()
            .chain(rows.iter().flatten())
            .for_each(|cell| runs(cell, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_pill::Mark;
    use crate::markdown::parser::parse_full;

    const ID: &str = "3f6b2a18-9c4d-4e5f-8a7b-1c2d3e4f5a6b";

    fn resolve(written: &str) -> ChatRef {
        if ID.starts_with(written) {
            ChatRef {
                chat_id: ID.into(),
                title: "Audit the budget (Codex)".into(),
                harness: Some(zeron_proto::HarnessId::Codex),
                model: None,
                mark: Mark::Working,
                known: true,
                archived: false,
            }
        } else {
            ChatRef::unknown(written)
        }
    }

    fn flat(source: &str) -> super::super::render::FlatText {
        let tree = parse_full(source);
        let super::super::parser::Block::Paragraph { runs } = &tree.blocks[0].block else {
            panic!("paragraph");
        };
        super::super::render::flatten_runs_with(
            runs,
            &crate::theme::Theme::dark(),
            gpui::FontWeight::NORMAL,
            Default::default(),
            Some(&resolve),
        )
    }

    #[test]
    fn references_flatten_to_pills_that_copy_back_to_their_token() {
        let source = format!("Started @chat:{ID} and @chat:{}… today.", &ID[..8]);
        let flat = flat(&source);
        assert_eq!(flat.chats.len(), 2);
        let pill = &flat.chats[0];
        let shown = &flat.text[pill.range.clone()];
        // The title, without the harness the mark already names, unbroken.
        assert!(shown.contains("Audit\u{a0}the\u{a0}budget"), "{shown:?}");
        assert!(!shown.contains("Codex"));
        assert!(pill.known);
        assert!(flat.text[pill.icon_slot.clone()].chars().all(|c| c == '\u{a0}'));
        assert!(flat.text[pill.status_slot.clone()].chars().all(|c| c == '\u{a0}'));
        // Copy and selection read the written tokens.
        let original = flat.original.as_ref().unwrap();
        let map = &original.offsets;
        let token = &original.text[map.original(pill.range.start)..map.original(pill.range.end)];
        assert_eq!(token, format!("@chat:{ID}"));
        let short = &flat.chats[1];
        let token = &original.text[map.original(short.range.start)..map.original(short.range.end)];
        assert_eq!(token, format!("@chat:{}…", &ID[..8]));
        assert_eq!(&original.text[..], source);
    }

    #[test]
    fn unknown_references_read_as_unknown_and_stay_inert() {
        let flat = flat("See @chat:deadbeef-0000 later.");
        assert_eq!(flat.chats.len(), 1);
        assert!(!flat.chats[0].known);
        assert!(flat.text[flat.chats[0].range.clone()].contains("Unknown\u{a0}chat"));
        assert!(flat.links.is_empty(), "pills are not links");
    }
}
