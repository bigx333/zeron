//! Orchestration activity in a parent's transcript: a delegated task
//! reporting its result or asking a question, or another chat's agent
//! writing here. These are reports, not the user speaking, so they read as
//! attributed lines in the activity rail — the source chat's live pill, what
//! it did, and the report itself folded under a one-line preview — instead
//! of user bubbles the person never typed.
//!
//! Rows come from typed `MessagePart::Activity` system entries and, for
//! transcripts written before those existed, from the text headers older
//! hosts put in user messages (`[Delegated task … finished: …]`,
//! `[Message from Zeron chat …]`). Consecutive reports merge into one row
//! with a summary header.
//!
//! Everything a report says about the source NOW (its title, its status,
//! whether a question is still open) resolves at render time from the
//! registry; the row itself only carries what was delivered.

use super::*;

use zeron_doc::{ActivityKind, ChatActivity};
use zeron_proto::TaskStatus;

use crate::chat_pill::{self, ChatRef};

/// Source lines a report body shows inline; the source chat holds the rest.
const BODY_MAX_LINES: usize = 40;
/// A report's body never parses more than this; long results continue in
/// their chat.
const BODY_MAX_BYTES: usize = 6_000;
/// The line under a collapsed report.
const PREVIEW_HEIGHT: f32 = 18.0;
/// Footer under an open report (its actions).
const FOOTER_HEIGHT: f32 = 30.0;
/// Body measurements before a first paint.
const BODY_ESTIMATE: f32 = 88.0;

/// One delivered report.
#[derive(Clone, Debug)]
pub struct ActivityItem {
    /// Stable within the transcript: entry id plus part id.
    pub key: SharedString,
    pub kind: ActivityKind,
    pub source_chat_id: SharedString,
    /// Title at delivery; the live title wins when the chat is known.
    pub source_title: SharedString,
    pub status: Option<TaskStatus>,
    /// A question's pending request.
    pub request_id: Option<SharedString>,
    pub body: Option<Arc<ActivityBody>>,
    /// The delivered body was cut short by its host.
    pub truncated: bool,
}

#[derive(Debug)]
pub struct ActivityBody {
    pub tree: Arc<BlockTree>,
    /// The (capped) Markdown source, for copy and the version key.
    pub text: SharedString,
    /// One plain line for the collapsed row.
    pub preview: SharedString,
    /// The display cap dropped part of the body.
    pub clipped: bool,
}

impl ActivityItem {
    pub(super) fn from_activity(
        entry_id: &str,
        activity: &ChatActivity,
        parse: &mut dyn FnMut(&str, &str) -> Arc<BlockTree>,
    ) -> Self {
        let key = format!("{entry_id}#{}", activity.id);
        Self {
            body: ActivityBody::new(&key, &activity.body, parse),
            key: key.into(),
            kind: activity.activity,
            source_chat_id: activity.source_chat_id.clone().into(),
            source_title: single_line(&activity.source_title).into(),
            status: activity.task_status,
            request_id: activity.request_id.clone().map(SharedString::from),
            truncated: activity.truncated,
        }
    }

    /// A user message older hosts wrote for orchestration, if it is one.
    /// External-controller messages aren't a chat and keep their bubble.
    pub(super) fn from_legacy_text(
        entry_id: &str,
        text: &str,
        parse: &mut dyn FnMut(&str, &str) -> Arc<BlockTree>,
    ) -> Option<Self> {
        let key = format!("{entry_id}#legacy");
        if let Some(result) = zeron_proto::chat_refs::parse_legacy_task_result(text) {
            return Some(Self {
                body: ActivityBody::new(&key, result.body, parse),
                key: key.into(),
                kind: ActivityKind::TaskResult,
                source_chat_id: result.child_chat_id.to_owned().into(),
                source_title: SharedString::default(),
                status: result.status,
                request_id: None,
                truncated: false,
            });
        }
        let message = zeron_proto::chat_refs::parse_agent_message(text)?;
        let sender = message.sender_chat_id?;
        Some(Self {
            body: ActivityBody::new(&key, message.body, parse),
            key: key.into(),
            kind: ActivityKind::Message,
            source_chat_id: sender.to_owned().into(),
            source_title: single_line(message.sender_label).into(),
            status: None,
            request_id: None,
            truncated: false,
        })
    }

    fn version_bytes(&self, acc: &mut Vec<u8>) {
        for field in [&self.key, &self.source_chat_id, &self.source_title] {
            acc.extend_from_slice(field.as_bytes());
            acc.push(0);
        }
        acc.push(self.kind as u8);
        acc.push(self.status.map_or(0xff, |s| s as u8));
        if let Some(request) = &self.request_id {
            acc.extend_from_slice(request.as_bytes());
        }
        acc.push(self.truncated as u8);
        if let Some(body) = &self.body {
            acc.extend_from_slice(body.text.as_bytes());
        }
        acc.push(0);
    }

    /// The source as the registry knows it now, falling back to what was
    /// delivered.
    pub(super) fn source(&self, state: &AppState) -> ChatRef {
        let mut chat = ChatRef::resolve(state, &self.source_chat_id);
        if !chat.known && !self.source_title.is_empty() {
            chat.title = self.source_title.clone();
        }
        chat
    }

    /// What the source did, as the words after its pill.
    ///
    /// A question row never claims to be the question waiting now: clients
    /// can't see which request a task is blocked on (a later question may
    /// not have been reported here yet), so the row says what happened and
    /// the pill's live mark says whether the task waits for input now.
    pub(super) fn verb(&self) -> (&'static str, Tone) {
        match self.kind {
            ActivityKind::TaskResult => match self.status {
                Some(TaskStatus::Completed) => ("finished", Tone::Quiet),
                Some(TaskStatus::Failed) => ("failed", Tone::Danger),
                Some(TaskStatus::Cancelled) => ("was stopped", Tone::Quiet),
                Some(TaskStatus::Interrupted) => ("was interrupted", Tone::Warning),
                _ => ("reported", Tone::Quiet),
            },
            ActivityKind::TaskInput => ("asked a question", Tone::Quiet),
            ActivityKind::Message => ("wrote", Tone::Quiet),
        }
    }

    fn open_label(&self) -> &'static str {
        match self.kind {
            ActivityKind::Message => "Open chat",
            _ => "Open task",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tone {
    Quiet,
    Warning,
    Danger,
}

impl Tone {
    fn color(self, theme: &Theme) -> gpui::Hsla {
        match self {
            Self::Quiet => theme.text_muted,
            Self::Warning => theme.warning,
            Self::Danger => theme.danger,
        }
    }
}

impl ActivityBody {
    fn new(
        key: &str,
        text: &str,
        parse: &mut dyn FnMut(&str, &str) -> Arc<BlockTree>,
    ) -> Option<Arc<Self>> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let mut end = text.len().min(BODY_MAX_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let capped = &text[..end];
        let capped = match capped.match_indices('\n').nth(BODY_MAX_LINES - 1) {
            Some((at, _)) => &capped[..at],
            None => capped,
        };
        let clipped = capped.len() < text.len();
        Some(Arc::new(Self {
            tree: parse(&format!("{key}#body"), capped),
            preview: preview_line(capped).into(),
            text: capped.to_owned().into(),
            clipped,
        }))
    }
}

/// The first line of a Markdown body as plain words: no markers, links as
/// their labels, code as its text.
fn preview_line(markdown: &str) -> String {
    let mut out = String::new();
    let mut in_code_block = false;
    let mut depth_skip = 0usize;
    for (event, _) in pulldown_cmark::Parser::new(markdown).into_offset_iter() {
        use pulldown_cmark::{Event, Tag, TagEnd};
        match event {
            Event::Start(Tag::CodeBlock(_)) => in_code_block = true,
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                if !out.trim().is_empty() {
                    break;
                }
            }
            Event::Start(Tag::Image { .. }) => depth_skip += 1,
            Event::End(TagEnd::Image) => depth_skip = depth_skip.saturating_sub(1),
            Event::Text(text) | Event::Code(text) if depth_skip == 0 => {
                if in_code_block {
                    if out.is_empty() {
                        out.push_str(text.lines().next().unwrap_or_default());
                    }
                } else {
                    out.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak if !out.is_empty() => out.push(' '),
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item)
                if !out.trim().is_empty() =>
            {
                break;
            }
            _ => {}
        }
        if out.len() > 400 {
            break;
        }
    }
    single_line(&out)
}

pub(super) fn activity_row(
    entry: &SessionMessageEntry,
    item: ActivityItem,
    timestamp: Option<i64>,
) -> Row {
    let mut acc = Vec::new();
    item.version_bytes(&mut acc);
    let copy_text = item
        .body
        .as_ref()
        .map(|body| body.text.clone())
        .filter(|text| !text.trim().is_empty());
    Row {
        id: item.key.clone(),
        version: fnv1a(&acc),
        turn_start: true,
        kind: RowKind::Activity {
            items: Arc::new(vec![item]),
        },
        entry_id: entry.id.clone().into(),
        timestamp,
        copy_text,
        compact_fold: None,
    }
}

/// Merge runs of adjacent activity rows into one row: bursts of reports
/// (several tasks finishing together) read as one summary. The merged row
/// keeps its first row's id, so a late arrival grows the row in place.
pub(super) fn merge_activity_rows(rows: &mut Vec<Row>) {
    let mut ix = 0;
    while ix + 1 < rows.len() {
        let mergeable = matches!(rows[ix].kind, RowKind::Activity { .. })
            && matches!(rows[ix + 1].kind, RowKind::Activity { .. });
        if !mergeable {
            ix += 1;
            continue;
        }
        let next = rows.remove(ix + 1);
        let row = &mut rows[ix];
        let (RowKind::Activity { items }, RowKind::Activity { items: more }) =
            (&mut row.kind, next.kind)
        else {
            continue;
        };
        let merged: Vec<ActivityItem> = items.iter().chain(more.iter()).cloned().collect();
        *items = Arc::new(merged);
        let mut acc = Vec::new();
        for item in items.iter() {
            item.version_bytes(&mut acc);
        }
        row.version = fnv1a(&acc);
        row.timestamp = next.timestamp.or(row.timestamp);
        row.copy_text = None;
    }
}

/// The header over a run of reports: "3 tasks finished", "2 tasks finished
/// · 1 question asked", "2 messages".
pub(super) fn summary(items: &[ActivityItem]) -> String {
    let mut finished = 0;
    let mut failed = 0;
    let mut stopped = 0;
    let mut asked = 0;
    let mut messages = 0;
    for item in items {
        match (item.kind, item.status) {
            (ActivityKind::TaskResult, Some(TaskStatus::Failed)) => failed += 1,
            (
                ActivityKind::TaskResult,
                Some(TaskStatus::Cancelled | TaskStatus::Interrupted),
            ) => stopped += 1,
            (ActivityKind::TaskResult, _) => finished += 1,
            (ActivityKind::TaskInput, _) => asked += 1,
            (ActivityKind::Message, _) => messages += 1,
        }
    }
    let plural = |n: usize, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    let mut parts = Vec::new();
    if finished > 0 {
        parts.push(plural(finished, "task finished", "tasks finished"));
    }
    if failed > 0 {
        parts.push(plural(failed, "task failed", "tasks failed"));
    }
    if stopped > 0 {
        parts.push(plural(stopped, "task stopped", "tasks stopped"));
    }
    if asked > 0 {
        parts.push(plural(asked, "question asked", "questions asked"));
    }
    if messages > 0 {
        parts.push(plural(messages, "message", "messages"));
    }
    parts.join(" · ")
}

/// A measured disclosure's current height: tweening from where the last
/// toggle left it toward the content's measured height (or zero), natural
/// height at rest.
fn fold_height(fold: FoldState, open: bool, natural: f32, reduce: bool, now: Instant) -> (f32, bool) {
    let target = if open { natural } else { 0.0 };
    let Some(at) = fold.toggled_at.filter(|_| !reduce) else {
        return (target, false);
    };
    let raw = now.saturating_duration_since(at).as_secs_f32() / TOOL_FOLD.total().as_secs_f32();
    if raw >= 1.0 {
        return (target, false);
    }
    (motion::lerp(fold.from, target, TOOL_FOLD.curve.eval(raw)), true)
}

/// A row that asks for frames while it moves, and only then.
fn with_frames(el: gpui::Div, animating: bool, view: gpui::EntityId) -> gpui::Div {
    el.when(animating, |el| {
        el.relative().child(
            canvas(
                |_, _, _| (),
                move |_, _, window, _| window.on_next_frame(move |_, cx| cx.notify(view)),
            )
            .absolute()
            .inset_0(),
        )
    })
}

impl Transcript {
    /// Every chat id the current rows name — pills in prose and activity
    /// sources — for the layout fingerprint.
    pub(super) fn collect_chat_refs(rows: &[Row]) -> Vec<String> {
        let mut ids: Vec<&str> = Vec::new();
        for row in rows {
            match &row.kind {
                RowKind::Markdown { tree, block_ix } | RowKind::LiveMarkdown { tree, block_ix } => {
                    if let Some(top) = tree.blocks.get(*block_ix) {
                        crate::markdown::chat_pills::block_chat_ids(&top.block, &mut ids);
                    }
                }
                RowKind::Activity { items } => {
                    for item in items.iter() {
                        ids.push(&item.source_chat_id);
                        if let Some(body) = &item.body {
                            for top in &body.tree.blocks {
                                crate::markdown::chat_pills::block_chat_ids(&top.block, &mut ids);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let mut ids: Vec<String> = ids.into_iter().map(str::to_owned).collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// Pill wiring for this frame: chats resolved now, clicks opening them.
    pub(super) fn chat_ui(&self, cx: &mut Context<Self>) -> Option<crate::markdown::chat_pills::ChatUi> {
        if self.chat_refs.is_empty() {
            return None;
        }
        let resolved: Rc<HashMap<String, ChatRef>> = {
            let state = self.state.read(cx);
            Rc::new(
                self.chat_refs
                    .iter()
                    .map(|id| (id.clone(), ChatRef::resolve(state, id)))
                    .collect(),
            )
        };
        let view = cx.entity().downgrade();
        Some(crate::markdown::chat_pills::ChatUi {
            resolve: Rc::new(move |written| {
                resolved
                    .get(written)
                    .cloned()
                    .unwrap_or_else(|| ChatRef::unknown(written))
            }),
            open: Rc::new(move |chat_id, window, cx| {
                let main_view = window.modifiers().platform;
                let chat_id = chat_id.to_owned();
                view.update(cx, |_, cx| {
                    cx.emit(TranscriptEvent::OpenChat { chat_id, main_view })
                })
                .ok();
            }),
            owner: cx.entity_id(),
        })
    }

    /// Re-flatten pills when what they name changed: a title, a harness, a
    /// chat arriving or leaving. Status alone never remeasures.
    pub(super) fn refresh_chat_ref_layout(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let revision = state.link_roots_revision;
        if revision == self.chat_refs_revision && !self.chat_refs_dirty {
            return;
        }
        self.chat_refs_revision = revision;
        self.chat_refs_dirty = false;
        let fingerprint =
            chat_pill::layout_fingerprint(state, self.chat_refs.iter().map(String::as_str));
        if fingerprint != self.chat_refs_fingerprint {
            let first = self.chat_refs_fingerprint == 0;
            self.chat_refs_fingerprint = fingerprint;
            if !first {
                self.render_cache.borrow_mut().clear();
                self.list.remeasure();
            }
        }
    }

    /// Note reports that arrive while the transcript is open, so they ease
    /// in once; a replayed history stays still.
    pub(super) fn track_activity_arrivals(&mut self, rows: &[Row], replay: bool) {
        let now = Instant::now();
        let mut present = HashSet::new();
        for row in rows {
            let RowKind::Activity { items } = &row.kind else {
                continue;
            };
            for item in items.iter() {
                present.insert(item.key.clone());
                if !self.activity_seen.contains(&item.key) {
                    self.activity_seen.insert(item.key.clone());
                    if !replay {
                        self.activity_arrivals.insert(item.key.clone(), now);
                    }
                }
            }
        }
        self.activity_seen.retain(|key| present.contains(key));
        self.activity_arrivals
            .retain(|key, at| present.contains(key) && at.elapsed() < motion::FADE_IN.total());
    }

    fn open_activity_source(&mut self, chat_id: &SharedString, main_view: bool, cx: &mut Context<Self>) {
        cx.emit(TranscriptEvent::OpenChat {
            chat_id: chat_id.to_string(),
            main_view,
        });
    }

    fn toggle_measured(&mut self, key: &SharedString, natural: f32, cx: &mut Context<Self>) {
        let reduce = motion::reduced_motion(cx);
        let now = Instant::now();
        let entry = self.folds.entry(key.clone()).or_default();
        let open = entry.open.unwrap_or(false);
        let (current, _) = fold_height(*entry, open, natural, reduce, now);
        entry.from = current;
        entry.open = Some(!open);
        entry.epoch += 1;
        entry.toggled_at = Some(now);
        entry.disclosure_at = entry.toggled_at;
        cx.notify();
    }

    fn measured_cell(&mut self, key: &SharedString) -> Rc<Cell<f32>> {
        self.activity_heights
            .entry(key.clone())
            .or_insert_with(|| Rc::new(Cell::new(0.0)))
            .clone()
    }

    pub(super) fn render_activity(
        &mut self,
        row_id: &SharedString,
        items: &Arc<Vec<ActivityItem>>,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity_id();
        let reduce = motion::reduced_motion(cx);
        let now = Instant::now();
        if let [item] = items.as_slice() {
            let (line, animating) = self.render_report(item, true, theme, window, cx);
            return with_frames(div().w_full().flex().flex_col(), animating, view)
                .font_family(theme.font_sans_fixed.clone())
                .child(line)
                .into_any_element();
        }
        // A run of reports: a quiet header folding over the lines.
        let group_key = SharedString::from(format!("{row_id}#group"));
        let fold = self.folds.get(&group_key).copied().unwrap_or_default();
        let open = fold.open.unwrap_or(true);
        let probe = self.measured_cell(&group_key);
        let natural = probe.get().max(if probe.get() > 0.0 {
            0.0
        } else {
            items.len() as f32 * TOOL_TREE_ROW_HEIGHT
        });
        let (height, mut animating) = fold_height(fold, open, natural, reduce, now);
        let progress = if reduce {
            if open { 1.0 } else { 0.0 }
        } else {
            tool_disclosure_progress(open, fold, now)
        };
        let toggle_key = group_key.clone();
        let header = div()
            .id(SharedString::from(format!("{row_id}-activity-hdr")))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .h(px(TOOL_GROUP_HEADER_HEIGHT))
            .cursor_pointer()
            .text_size(px(TOOL_LABEL_SIZE))
            .line_height(px(TOOL_LABEL_LINE_HEIGHT))
            .text_color(theme.text_muted)
            .hover(|s| s.text_color(theme.text))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                let natural = this.measured_cell(&toggle_key).get();
                // Groups open by default: pin the default before toggling.
                let entry = this.folds.entry(toggle_key.clone()).or_default();
                entry.open.get_or_insert(true);
                this.toggle_measured(&toggle_key, natural, cx);
            }))
            .child(
                div().w(px(22.0)).h(px(18.0)).flex_none().relative().child(
                    crate::icons::icon(crate::icons::ALT_ARROW_DOWN)
                        .absolute()
                        .left(px(ACTIVITY_TRUNK_X - 7.0))
                        .top(px(2.0))
                        .size(px(14.0))
                        .with_transformation(gpui::Transformation::rotate(gpui::radians(
                            -std::f32::consts::FRAC_PI_2 * (1.0 - progress),
                        )))
                        .text_color(theme.text_muted),
                ),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(SharedString::from(summary(items))),
            );
        let body_visible = open || animating;
        let mut column = div().flex().flex_col().pt(px(CHIPS_TOP_PAD));
        if body_visible {
            for item in items.iter() {
                let (line, line_animating) = self.render_report(item, false, theme, window, cx);
                animating |= line_animating;
                column = column.child(line);
            }
        }
        let content = div()
            .flex_none()
            .w_full()
            .relative()
            .child(column)
            .child(height_probe(probe));
        let settled_open = open && !animating;
        let body = if settled_open {
            content.into_any_element()
        } else if body_visible {
            div()
                .w_full()
                .h(px(height))
                .overflow_hidden()
                .child(content)
                .into_any_element()
        } else {
            gpui::Empty.into_any_element()
        };
        with_frames(div().w_full().flex().flex_col(), animating, view)
            .font_family(theme.font_sans_fixed.clone())
            .child(header)
            .child(body)
            .into_any_element()
    }

    /// One report: rail glyph, the source's pill, what it did, a chevron
    /// when there's a body; under it the preview (folded) or the body and
    /// its actions (open). `lone`: the row's only report, which shows its
    /// preview line while folded.
    fn render_report(
        &mut self,
        item: &ActivityItem,
        lone: bool,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (AnyElement, bool) {
        let view = cx.entity_id();
        let reduce = motion::reduced_motion(cx);
        let now = Instant::now();
        let live = item.source(self.state.read(cx));
        let (verb, tone) = item.verb();
        let key = item.key.clone();
        let fold = self.folds.get(&key).copied().unwrap_or_default();
        let open = fold.open.unwrap_or(false);
        let probe = self.measured_cell(&key);
        let natural = if probe.get() > 0.0 {
            probe.get()
        } else {
            BODY_ESTIMATE
        };
        let has_body = item.body.is_some();
        let (body_height, mut animating) = fold_height(fold, open, natural, reduce, now);
        let progress = if reduce {
            if open { 1.0 } else { 0.0 }
        } else {
            tool_disclosure_progress(open, fold, now)
        };
        // Arrival: a report landing while the chat is open eases in once.
        let arrival = self
            .activity_arrivals
            .get(&key)
            .filter(|_| !reduce)
            .map(|at| {
                motion::FADE_IN.progress(
                    at.elapsed().as_secs_f32() / motion::FADE_IN.total().as_secs_f32(),
                )
            })
            .filter(|t| *t < 1.0);
        animating |= arrival.is_some();

        let glyph_color = match tone {
            Tone::Danger => theme.danger,
            _ => theme.text_muted,
        };
        // The glyph heads the report's rail; an open body hangs off it, so
        // the reader sees whose words these are all the way down.
        let glyph = div()
            .w(px(ACTIVITY_TRUNK_X * 2.0))
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .child(
                div()
                    .h(px(TOOL_TREE_ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .child(
                        crate::icons::icon(match item.kind {
                            ActivityKind::Message => crate::icons::CHAT_ROUND_LINE,
                            _ => crate::icons::BOT,
                        })
                        .size(px(14.0))
                        .text_color(glyph_color),
                    ),
            )
            .when(has_body && (open || animating), |el| {
                el.child(
                    div()
                        .flex_1()
                        .w(px(1.0))
                        .mb(px(FOOTER_HEIGHT / 2.0))
                        .bg(theme.hairline(0.12))
                        .opacity(progress),
                )
            });
        let pill_id = SharedString::from(format!("{key}-pill"));
        let pill_chat = live.chat_id.clone();
        let pill = chat_pill::pill(&key, &live, true, view, theme, cx)
            .id(pill_id)
            .when(live.known, |pill| {
                let hover = theme.ink(0.09);
                pill.cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .tooltip(crate::settings::widgets::text_tooltip(live.spoken()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        let main_view = window.modifiers().platform;
                        this.open_activity_source(&pill_chat, main_view, cx);
                    }))
            });
        let toggle_key = key.clone();
        let header = div()
            .id(SharedString::from(format!("{key}-line")))
            .h(px(TOOL_TREE_ROW_HEIGHT))
            .flex_1()
            .min_w_0()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .text_size(px(TOOL_LABEL_SIZE))
            .line_height(px(TOOL_LABEL_LINE_HEIGHT))
            .text_color(theme.text_muted)
            .when(has_body, |el| {
                el.cursor_pointer()
                    .group("activity-line")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let natural = this.measured_cell(&toggle_key).get();
                        let natural = if natural > 0.0 { natural } else { BODY_ESTIMATE };
                        this.toggle_measured(&toggle_key, natural, cx);
                    }))
            })
            .child(pill)
            .child(
                div()
                    .flex_none()
                    .text_color(tone.color(theme))
                    .child(crate::roll_text::rolling(
                        format!("activity-verb-{key}"),
                        SharedString::from(verb),
                        reduce,
                    )
                    .unwrap_or_else(|| SharedString::from(verb).into_any_element())),
            )
            .child(div().flex_1())
            .when(has_body, |el| {
                el.child(
                    crate::icons::icon(crate::icons::ALT_ARROW_DOWN)
                        .size(px(14.0))
                        .flex_none()
                        .with_transformation(gpui::Transformation::rotate(gpui::radians(
                            std::f32::consts::PI * progress,
                        )))
                        .text_color(theme.text_faint),
                )
            });
        let mut column = div().flex_1().min_w_0().flex().flex_col().child(header);
        // Folded: the preview line, for a lone report.
        let preview = item
            .body
            .as_ref()
            .map(|body| body.preview.clone())
            .filter(|preview| lone && !preview.is_empty());
        let show_preview = preview.is_some() && (!open || animating);
        if let Some(preview) = preview.filter(|_| show_preview) {
            let fade = 1.0 - progress;
            column = column.child(
                div()
                    .h(px(PREVIEW_HEIGHT * fade.clamp(0.0, 1.0)))
                    .overflow_hidden()
                    .opacity(fade)
                    .child(
                        div()
                            .h(px(PREVIEW_HEIGHT))
                            .mt(px(-4.0))
                            .truncate()
                            .text_size(px(TOOL_LABEL_SIZE))
                            .text_color(theme.text_faint)
                            .child(preview),
                    ),
            );
        }
        if has_body && (open || animating) {
            let content = div()
                .flex_none()
                .w_full()
                .relative()
                .flex()
                .flex_col()
                .child(self.render_report_body(item, &live, theme, window, cx))
                .child(height_probe(probe));
            column = column.child(if open && !animating {
                content.into_any_element()
            } else {
                div()
                    .w_full()
                    .h(px(body_height))
                    .overflow_hidden()
                    .child(content)
                    .into_any_element()
            });
        }
        let row = div()
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .child(glyph)
            .child(column.ml(px(ACTIVITY_TEXT_GAP - 4.0)));
        let row = match arrival {
            Some(t) => row.relative().top(px(4.0 * (1.0 - t))).opacity(t),
            None => row,
        };
        (row.into_any_element(), animating)
    }

    /// An open report: the body as Markdown (pills, links and code as in
    /// any reply), then the actions.
    fn render_report_body(
        &mut self,
        item: &ActivityItem,
        live: &ChatRef,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(body) = item.body.clone() else {
            return gpui::Empty.into_any_element();
        };
        let link = self.link_ui(cx);
        let chats = self.frame_chat_ui.clone();
        let row_key = SharedString::from(format!("{}#body", item.key));
        let mut blocks = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(render::MD_BLOCK_GAP))
            .pt(px(2.0))
            .pb(px(6.0))
            .text_size(crate::typography::ui_rems(13.5))
            .font_family(theme.font_sans.clone())
            .text_color(theme.text);
        for (ix, top) in body.tree.blocks.iter().enumerate() {
            let code = self.code_uis_for(&row_key, &top.block, ix, cx);
            let opts = RenderOptions {
                tasks: None,
                media: None,
                row_key: row_key.clone(),
                veil: None,
                cache: (!render_cache_disabled()).then(|| self.render_cache.clone()),
                now: Instant::now(),
                copy: Some(self.copy_ui_for(&row_key, cx)),
                link: link.clone(),
                workspace_root: None,
                code,
                math: crate::settings::math_style(cx),
                chats: chats.clone(),
            };
            blocks = blocks.child(render::render_block(&top.block, ix, ix, &opts, theme, window, None));
        }
        let note = if item.truncated || body.clipped {
            Some(match item.kind {
                ActivityKind::Message => "Shortened. Open the chat for the rest.",
                _ => "Shortened. Open the task for its conversation.",
            })
        } else {
            None
        };
        let chat_id = live.chat_id.clone();
        let label = item.open_label();
        let footer = div()
            .h(px(FOOTER_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .when(live.known, |el| {
                el.child(
                    div()
                        .id(SharedString::from(format!("{}-open", item.key)))
                        .h(px(22.0))
                        .px(px(8.0))
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .rounded(px(Theme::CONTROL_RADIUS))
                        .cursor_pointer()
                        .text_size(px(TOOL_LABEL_SIZE))
                        .text_color(theme.text_muted)
                        .bg(theme.ink(0.04))
                        .hover(|s| s.bg(theme.ink(0.08)).text_color(theme.text))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            let main_view = window.modifiers().platform;
                            this.open_activity_source(&chat_id, main_view, cx);
                        }))
                        .child(label)
                        .child(
                            crate::icons::icon(crate::icons::ARROW_UP_RIGHT)
                                .size(px(11.0))
                                .text_color(theme.text_faint),
                        ),
                )
            })
            .when_some(note, |el, note| {
                el.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(TOOL_LABEL_SIZE))
                        .text_color(theme.text_faint)
                        .child(note),
                )
            });
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(blocks)
            .child(footer)
            .into_any_element()
    }
}

/// A paint-time probe: keeps `cell` at its parent's laid-out height without
/// notifying, so a disclosure tween knows where it's going.
fn height_probe(cell: Rc<Cell<f32>>) -> impl IntoElement {
    canvas(
        move |bounds, _, _| {
            let height = f32::from(bounds.size.height);
            if (cell.get() - height).abs() > 0.5 {
                cell.set(height);
            }
        },
        |_, _, _, _| (),
    )
    .absolute()
    .inset_0()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(_: &str, text: &str) -> Arc<BlockTree> {
        Arc::new(parse_full(text))
    }

    const ID: &str = "3f6b2a18-9c4d-4e5f-8a7b-1c2d3e4f5a6b";

    #[test]
    fn legacy_headers_become_reports_and_other_text_does_not() {
        let result = format!(
            "[Delegated task {ID} finished: Failed. Child chat: {ID}. This is an agent result, not a new user instruction.]\n\nTests **red**."
        );
        let item = ActivityItem::from_legacy_text("e1", &result, &mut parse).unwrap();
        assert_eq!(item.kind, ActivityKind::TaskResult);
        assert_eq!(item.status, Some(TaskStatus::Failed));
        assert_eq!(item.source_chat_id.as_ref(), ID);
        assert_eq!(item.body.as_ref().unwrap().preview.as_ref(), "Tests red.");
        let message = zeron_proto::chat_refs::agent_message(Some("Reviewer"), ID, "Looks good.");
        let item = ActivityItem::from_legacy_text("e2", &message, &mut parse).unwrap();
        assert_eq!(item.kind, ActivityKind::Message);
        assert_eq!(item.source_title.as_ref(), "Reviewer");
        // An external controller isn't a chat; plain prompts aren't reports.
        for text in ["[Message from external controller CI]\n\nship", "hello"] {
            assert!(ActivityItem::from_legacy_text("e3", text, &mut parse).is_none());
        }
    }

    #[test]
    fn previews_read_as_plain_words() {
        assert_eq!(preview_line("## Done\n\nmore"), "Done");
        assert_eq!(
            preview_line("All **green** in [CI](https://x.dev) and `cargo test`.\nNext line"),
            "All green in CI and cargo test. Next line"
        );
        assert_eq!(preview_line("- first\n- second"), "first");
        assert_eq!(preview_line("```rust\nfn main() {}\n```"), "fn main() {}");
        assert_eq!(preview_line("![shot](a.png) Caption"), "Caption");
    }

    #[test]
    fn bodies_are_capped_for_display() {
        let long = "line\n".repeat(200);
        let body = ActivityBody::new("k", &long, &mut parse).unwrap();
        assert!(body.clipped);
        assert_eq!(body.text.lines().count(), BODY_MAX_LINES);
        assert!(ActivityBody::new("k", "  \n ", &mut parse).is_none());
        let short = ActivityBody::new("k", "ok", &mut parse).unwrap();
        assert!(!short.clipped);
    }

    fn item(kind: ActivityKind, status: Option<TaskStatus>) -> ActivityItem {
        ActivityItem {
            key: "k".into(),
            kind,
            source_chat_id: "c".into(),
            source_title: "t".into(),
            status,
            request_id: None,
            body: None,
            truncated: false,
        }
    }

    #[test]
    fn summaries_count_by_outcome() {
        use ActivityKind::*;
        let items = [
            item(TaskResult, Some(TaskStatus::Completed)),
            item(TaskResult, Some(TaskStatus::Completed)),
            item(TaskResult, Some(TaskStatus::Failed)),
            item(TaskInput, Some(TaskStatus::Waiting)),
            item(Message, None),
        ];
        assert_eq!(
            summary(&items),
            "2 tasks finished · 1 task failed · 1 question asked · 1 message"
        );
    }

    #[test]
    fn adjacent_reports_merge_into_one_row_keeping_the_first_id() {
        let entry = |id: &str| SessionMessageEntry {
            id: id.into(),
            role: MessageRole::System,
            parts: Vec::new(),
            created_at: 1,
            device_id: "d".into(),
            status: Some(MessageStatus::Complete),
            continuation_of: None,
            duration_ms: None,
        };
        let report = |entry_id: &str| {
            let mut item = item(ActivityKind::TaskResult, Some(TaskStatus::Completed));
            item.key = format!("{entry_id}#activity").into();
            activity_row(&entry(entry_id), item, Some(2))
        };
        let mut rows = vec![report("a"), report("b"), report("c")];
        let first_id = rows[0].id.clone();
        let single_version = rows[0].version;
        merge_activity_rows(&mut rows);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, first_id);
        assert_ne!(rows[0].version, single_version);
        let RowKind::Activity { items } = &rows[0].kind else {
            panic!("activity row");
        };
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn questions_read_as_history_whatever_the_live_status() {
        // Which request a task waits on isn't visible to clients: an earlier
        // question must not read as the open one while a later, unreported
        // one waits. The pill's live mark carries "waiting now".
        let ask = item(ActivityKind::TaskInput, Some(TaskStatus::Waiting));
        assert_eq!(ask.verb(), ("asked a question", Tone::Quiet));
        assert_eq!(ask.open_label(), "Open task");
        let message = item(ActivityKind::Message, None);
        assert_eq!(message.open_label(), "Open chat");
    }
}

#[cfg(feature = "appshots-fixture")]
impl Transcript {
    /// Unfold the named reports (activity item keys, or `{row}#group`) as a
    /// settled click would; everything else folds.
    pub fn fixture_activity(&mut self, open: &[&str], cx: &mut Context<Self>) {
        let keys: Vec<SharedString> = self
            .rows
            .iter()
            .filter_map(|row| match &row.kind {
                RowKind::Activity { items } => Some(
                    items
                        .iter()
                        .map(|item| item.key.clone())
                        .chain(std::iter::once(SharedString::from(format!("{}#group", row.id))))
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .collect();
        for key in keys {
            let fold = self.folds.entry(key.clone()).or_default();
            fold.open = Some(key.ends_with("#group") || open.contains(&key.as_ref()));
            fold.toggled_at = None;
            fold.disclosure_at = None;
        }
        self.activity_arrivals.clear();
        cx.notify();
    }

    /// Scroll the transcript so row `ix` (from the end when negative) leads.
    pub fn fixture_scroll_to_row(&mut self, from_end: usize, cx: &mut Context<Self>) {
        let ix = self.rows.len().saturating_sub(from_end);
        self.pinned = false;
        self.list.scroll_to(gpui::ListOffset {
            item_ix: ix,
            offset_in_item: px(0.0),
        });
        cx.notify();
    }
}
