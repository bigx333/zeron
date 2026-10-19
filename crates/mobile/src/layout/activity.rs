//! Orchestration reports in a parent's transcript, as the desktop draws them:
//! the source chat's pill and what it did ("finished", "needs an answer",
//! "wrote"), a one-line preview while folded, and the report itself with a
//! way into the source once unfolded. Rows come from typed activity entries
//! and from the text headers older hosts wrote into user messages.
//!
//! The pill opens the source chat (a `zeron-chat:` link); the chevron and
//! the preview fold the report like a tool group.

use zeron_doc::parts::{ActivityKind, ChatActivity};
use zeron_markdown::parser::parse_full;
use zeron_proto::TaskStatus;
use zeron_text::{OverflowWrap, PrepareOptions, Span, WhiteSpace};

use super::chats::ChatLabel;
use super::display::{ColorRole, Decoration, DisplayBuilder, WidgetKind};
use super::markdown::{Ctx, PBlock, PText, Px, SpanPaint, place_stack, place_text, prepare_block, prepare_plain};
use super::style::{Family, TYPE, Weight, baseline};

/// Source lines a report shows inline; the chat holds the rest.
const BODY_MAX_LINES: usize = 40;
const BODY_MAX_BYTES: usize = 6_000;
const PREVIEW_MAX_CHARS: usize = 140;

/// One delivered report, whatever wrote it.
pub(crate) struct Report<'a> {
    pub kind: ActivityKind,
    pub source: &'a str,
    pub title: &'a str,
    pub status: Option<TaskStatus>,
    pub body: &'a str,
    pub truncated: bool,
}

impl<'a> Report<'a> {
    pub fn from_activity(activity: &'a ChatActivity) -> Self {
        Self {
            kind: activity.activity,
            source: &activity.source_chat_id,
            title: &activity.source_title,
            status: activity.task_status,
            body: &activity.body,
            truncated: activity.truncated,
        }
    }

    /// A user message an older host wrote for orchestration, if it is one.
    pub fn from_legacy(text: &'a str) -> Option<Self> {
        if let Some(result) = zeron_proto::chat_refs::parse_legacy_task_result(text) {
            return Some(Self {
                kind: ActivityKind::TaskResult,
                source: result.child_chat_id,
                title: "",
                status: result.status,
                body: result.body,
                truncated: false,
            });
        }
        let message = zeron_proto::chat_refs::parse_agent_message(text)?;
        Some(Self {
            kind: ActivityKind::Message,
            source: message.sender_chat_id?,
            title: message.sender_label,
            status: None,
            body: message.body,
            truncated: false,
        })
    }

    /// The words after the pill, and their tone. A question row never
    /// claims to be the question waiting now — clients can't see which
    /// request a task is blocked on — so it says what happened; the pill's
    /// source opens the task, where the waiting question is.
    pub fn verb(&self) -> (&'static str, ColorRole) {
        match self.kind {
            ActivityKind::TaskResult => match self.status {
                Some(TaskStatus::Completed) => ("finished", ColorRole::TextSecondary),
                Some(TaskStatus::Failed) => ("failed", ColorRole::Danger),
                Some(TaskStatus::Cancelled) => ("was stopped", ColorRole::TextSecondary),
                Some(TaskStatus::Interrupted) => ("was interrupted", ColorRole::Warning),
                _ => ("reported", ColorRole::TextSecondary),
            },
            ActivityKind::TaskInput => ("asked a question", ColorRole::TextSecondary),
            ActivityKind::Message => ("wrote", ColorRole::TextSecondary),
        }
    }

    fn open_label(&self) -> &'static str {
        match self.kind {
            ActivityKind::Message => "Open chat",
            _ => "Open task",
        }
    }
}

/// A prepared report row.
pub(crate) struct ActivityRow {
    pub glyph: &'static str,
    pub glyph_color: ColorRole,
    /// The pill and the verb, one wrapping line.
    pub header: PText,
    pub preview: Option<PText>,
    pub body: Vec<PBlock>,
    /// "Open task" / "Answer" — a link to the source; `None` when unknown.
    pub footer: Option<PText>,
    pub note: Option<PText>,
    pub expanded: bool,
    /// There is a body to unfold (prepared only while unfolded).
    pub has_body: bool,
}

fn capped(body: &str) -> (&str, bool) {
    let body = body.trim();
    let mut end = body.len().min(BODY_MAX_BYTES);
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    let capped = &body[..end];
    let capped = match capped.match_indices('\n').nth(BODY_MAX_LINES - 1) {
        Some((at, _)) => &capped[..at],
        None => capped,
    };
    (capped, capped.len() < body.len())
}

/// The first line of Markdown as plain words.
pub(crate) fn preview_line(markdown: &str) -> String {
    use pulldown_cmark::{Event, Tag, TagEnd};
    let mut out = String::new();
    let mut code = false;
    let mut image = 0usize;
    for event in pulldown_cmark::Parser::new(markdown) {
        match event {
            Event::Start(Tag::CodeBlock(_)) => code = true,
            Event::End(TagEnd::CodeBlock) => {
                code = false;
                if !out.trim().is_empty() {
                    break;
                }
            }
            Event::Start(Tag::Image { .. }) => image += 1,
            Event::End(TagEnd::Image) => image = image.saturating_sub(1),
            Event::Text(text) | Event::Code(text) if image == 0 => {
                if code {
                    if out.is_empty() {
                        out.push_str(text.lines().next().unwrap_or_default());
                    }
                } else {
                    out.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak if !out.is_empty() => out.push(' '),
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item) if !out.trim().is_empty() => break,
            _ => {}
        }
        if out.len() > 400 {
            break;
        }
    }
    let line = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > PREVIEW_MAX_CHARS {
        let mut cut: String = line.chars().take(PREVIEW_MAX_CHARS).collect();
        cut = cut.trim_end().to_owned();
        cut.push('…');
        cut
    } else {
        line
    }
}

/// The header: the source's pill (harness mark in its well, the title; a
/// tap opens the chat), then the verb in its tone.
fn header_text(ctx: &mut Ctx, live: Option<&ChatLabel>, fallback: &str, verb: &str, tone: ColorRole) -> PText {
    let (size, lh) = TYPE.small;
    let lh = ctx.typo.px(lh) + ctx.typo.px(8.0);
    let pill_style = ctx.typo.style(Family::Sans, Weight::Medium, false, size);
    let verb_style = ctx.typo.style(Family::Sans, Weight::Regular, false, size);
    let chip_h = lh - ctx.typo.px(8.0);
    let pad = (1.0 + (chip_h - 2.0) + ctx.typo.px(5.0), ctx.typo.px(7.0));
    let title = live.map_or_else(
        || if fallback.trim().is_empty() { "Unknown chat".to_owned() } else { fallback.split_whitespace().collect::<Vec<_>>().join(" ") },
        |chat| chat.display_title().to_owned(),
    );
    let mut text = title.clone();
    let pill_end = text.len();
    text.push(' ');
    let verb_start = text.len();
    text.push_str(verb);
    let spans = [
        Span {
            pad_start: pad.0,
            pad_end: pad.1,
            atomic: true,
            ..Span::new(0..pill_end, pill_style.id)
        },
        Span::new(pill_end..verb_start, verb_style.id),
        Span::new(verb_start..text.len(), verb_style.id),
    ];
    let p = zeron_text::prepare(
        &ctx.typo.book,
        ctx.cache,
        &text,
        &spans,
        &PrepareOptions {
            white_space: WhiteSpace::Normal,
            overflow_wrap: OverflowWrap::Anywhere,
            ..PrepareOptions::default()
        },
    );
    let link = live.map(|chat| chat.link());
    PText {
        p,
        lh,
        base: baseline(lh, pill_style),
        paints: vec![
            SpanPaint {
                color: if live.is_some() { ColorRole::Text } else { ColorRole::TextSecondary },
                decoration: Decoration::None,
                link: link.as_ref().map(|_| 0),
                chip: false,
            },
            SpanPaint { color: tone, decoration: Decoration::None, link: None, chip: false },
            SpanPaint { color: tone, decoration: Decoration::None, link: None, chip: false },
        ],
        links: link.into_iter().collect(),
        chip: ((lh - chip_h) / 2.0, chip_h),
        badges: vec![(0, live.map_or_else(|| "tool-bot".to_owned(), ChatLabel::icon))],
        math: Vec::new(),
    }
}

pub(crate) fn prepare(ctx: &mut Ctx, report: &Report, expanded: bool) -> ActivityRow {
    let live = ctx.chats.resolve(report.source).cloned();
    let (verb, tone) = report.verb();
    let header = header_text(ctx, live.as_ref(), report.title, verb, tone);
    let (body_text, clipped) = capped(report.body);
    let (size, lh) = TYPE.small;
    let small = ctx.typo.style(Family::Sans, Weight::Regular, false, size);
    let small_lh = ctx.typo.px(lh);
    let preview = (!expanded && !body_text.is_empty()).then(|| {
        prepare_plain(ctx, &preview_line(body_text), small, small_lh, ColorRole::TextTertiary, WhiteSpace::Normal)
    });
    let body = if expanded && !body_text.is_empty() {
        parse_full(body_text)
            .blocks
            .iter()
            .map(|top| {
                ctx.math_area = 0.0;
                prepare_block(ctx, &top.block, 0, false)
            })
            .collect()
    } else {
        Vec::new()
    };
    let footer = live.as_ref().filter(|_| expanded).map(|chat| {
        let label = report.open_label();
        let style = ctx.typo.style(Family::Sans, Weight::Medium, false, size);
        let mut t = prepare_plain(ctx, label, style, small_lh, ColorRole::Link, WhiteSpace::Pre);
        t.links = vec![chat.link()];
        t.paints[0].link = Some(0);
        t
    });
    let note = (expanded && (report.truncated || clipped)).then(|| {
        prepare_plain(
            ctx,
            match report.kind {
                ActivityKind::Message => "Shortened. Open the chat for the rest.",
                _ => "Shortened. Open the task for its conversation.",
            },
            small,
            small_lh,
            ColorRole::TextTertiary,
            WhiteSpace::Normal,
        )
    });
    ActivityRow {
        glyph: match report.kind {
            ActivityKind::Message => "tool-chat-round-line",
            _ => "tool-bot",
        },
        glyph_color: match tone {
            ColorRole::Danger => tone,
            _ => ColorRole::TextTertiary,
        },
        header,
        preview,
        body,
        footer,
        note,
        expanded,
        has_body: !body_text.is_empty(),
    }
}

/// The report's plain words, for copy and assistive tech.
pub(crate) fn spoken(report: &Report, live: Option<&ChatLabel>) -> String {
    let title = live.map_or(report.title, ChatLabel::display_title);
    let (verb, _) = report.verb();
    if report.body.trim().is_empty() {
        format!("{title} {verb}.")
    } else {
        format!("{title} {verb}.\n\n{}", report.body.trim())
    }
}

/// Height of a report row at `width`, painting when `out` is given.
pub(crate) fn place(a: &ActivityRow, px: Px, x: f32, y: f32, width: f32, mut out: Option<&mut DisplayBuilder>) -> f32 {
    let indent = px.v(24.0);
    let tx = x + indent;
    let tw = (width - indent - if a.has_body { px.v(28.0) } else { 0.0 }).max(1.0);
    let header_h = place_text(&a.header, tx, y, tw, out.as_deref_mut()).max(px.v(28.0));
    if let Some(out) = out.as_deref_mut() {
        let s = px.v(16.0);
        out.widget(
            WidgetKind::Icon { name: a.glyph.into(), color: a.glyph_color },
            (x + px.v(2.0), y + (a.header.lh - s) / 2.0, s, s),
            None,
        );
        if a.has_body {
            // The chevron is the fold's tap target; the pill keeps its own
            // tap (it opens the source).
            let c = px.v(16.0);
            let tap = px.v(44.0);
            out.widget(WidgetKind::Chevron { expanded: a.expanded }, (x + width - c - px.v(4.0), y + (a.header.lh - c) / 2.0, c, c), None);
            out.widget(WidgetKind::Disclosure { expanded: a.expanded }, (x + width - tap, y, tap, header_h), None);
        }
    }
    let mut h = header_h;
    if let Some(preview) = &a.preview {
        let ph = place_text(preview, tx, y + h, (width - indent).max(1.0), out.as_deref_mut());
        if let Some(out) = out.as_deref_mut() {
            // Tapping the preview unfolds the report too.
            out.widget(WidgetKind::Disclosure { expanded: false }, (tx, y + h, (width - indent).max(1.0), ph), None);
        }
        h += ph;
    }
    if a.expanded && a.has_body {
        h += px.v(6.0);
        let bh = place_stack(&a.body, px, tx, y + h, (width - indent).max(1.0), 12.0, out.as_deref_mut());
        if let Some(out) = out.as_deref_mut() {
            // The rail the report hangs off.
            out.fill(x + px.v(9.5), y + header_h, px.v(1.0), h + bh - header_h, 0.5, ColorRole::ToolRail);
        }
        h += bh;
        if let Some(note) = &a.note {
            h += px.v(6.0);
            h += place_text(note, tx, y + h, (width - indent).max(1.0), out.as_deref_mut());
        }
        if let Some(footer) = &a.footer {
            h += px.v(8.0);
            h += place_text(footer, tx, y + h, (width - indent).max(1.0), out.as_deref_mut());
        }
    }
    h
}
