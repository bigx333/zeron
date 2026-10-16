//! Skills: the shared skill library every provider loads (`~/.zeron/skills`)
//! and the skills.sh registry it installs from. A board in the Pull requests
//! page's language — the same column, segmented tray, search field and
//! carded rows — with a detail view that reads a skill's `SKILL.md`.
//!
//! The engine owns the library: listing, reading, removing and installing
//! all go through it, and it refuses paths outside the shared root. The page
//! only reveals and opens files, which happens on this device.
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    time::{Duration, Instant},
};

use chrono::Utc;
use gpui::{
    AnyElement, Context, Entity, FocusHandle, IntoElement, Render, SharedString, Subscription,
    Task, Window, div, prelude::*, px,
};
use zeron_engine::skill_library::{
    Library, LibrarySkill, RegistryPreview, RegistrySearch, RegistrySkill, SkillContents,
};
use zeron_rpc::methods;

mod packs;

use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::icons::{self, icon};
use crate::popover::{self, Loadable};
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

const PAGE_MAX_WIDTH: f32 = 760.0;
const PAGE_HORIZONTAL_PADDING: f32 = 40.0;
const ROW_HEIGHT: f32 = 64.0;
const SCROLL_FADE_BAND: f32 = 24.0;
const HEADER_CONTROL_HEIGHT: f32 = 32.0;
/// Typing pauses this long before skills.sh is asked.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(280);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Installed,
    Registry,
}

impl Tab {
    fn key(self) -> &'static str {
        match self {
            Self::Installed => "skills-tab-installed",
            Self::Registry => "skills-tab-registry",
        }
    }
}

/// A skill's `SKILL.md`, parsed once for every frame that shows it.
struct Document {
    description: Option<String>,
    files: Vec<String>,
    truncated: bool,
    tree: Rc<crate::markdown::BlockTree>,
}

enum Detail {
    Installed {
        skill: LibrarySkill,
        document: Loadable<Rc<Document>>,
    },
    Registry {
        skill: RegistrySkill,
        document: Loadable<Rc<Document>>,
    },
}

impl Detail {
    fn key(&self) -> String {
        match self {
            Self::Installed { skill, .. } => format!("installed-{}", skill.directory),
            Self::Registry { skill, .. } => format!("registry-{}", skill.id),
        }
    }
}

pub struct SkillsPage {
    state: Entity<AppState>,
    tab: Tab,
    tab_fades: crate::motion::HoverFades,
    search: Entity<ComposerInput>,
    query: String,
    library: Loadable<Library>,
    library_task: Option<Task<()>>,
    registry: Loadable<RegistrySearch>,
    /// The query the registry results (or the request in flight) answer.
    registry_query: Option<String>,
    registry_task: Option<Task<()>>,
    /// Registry ids being installed, each with its request.
    installing: HashMap<String, Task<()>>,
    /// Registry ids installed while this page was open, so their control
    /// settles from the spinner into the Installed mark.
    just_installed: HashSet<String>,
    /// Asked to confirm Remove.
    removing: Option<LibrarySkill>,
    remove_task: Option<Task<()>>,
    detail: Option<Detail>,
    detail_task: Option<Task<()>>,
    /// Bumped when what the list shows changes, so it fades in afresh.
    generation: u64,
    scroll: widgets::PageScroll,
    detail_scroll: widgets::PageScroll,
    visible: bool,
    focus: FocusHandle,
    selection_prefix: String,
    /// Packs shown with their skills listed.
    expanded_packs: HashSet<String>,
    /// Packs (by `name@commit`) whose waiting update's notes are open.
    changes_open: HashSet<String>,
    changes_trees: HashMap<String, Rc<crate::markdown::BlockTree>>,
    pack_busy: HashMap<String, (packs::PackBusy, Task<()>)>,
    removing_pack: Option<zeron_engine::skill_packs::SkillPack>,
    add_pack: Option<packs::AddPack>,
    /// Stale packs were checked for updates this session.
    checked_packs: bool,
    /// A pack install in flight, held apart from the dialog that started it.
    pack_install: Option<Task<()>>,
    /// Each pending auto-update toggle's value from before it, restored if
    /// the engine refuses it.
    toggle_previous: HashMap<String, bool>,
    /// Library replies are numbered when requested; one older than the
    /// library on screen is dropped, so a slow refresh can't undo a change.
    library_sent: u64,
    library_shown: u64,
    _search_events: Subscription,
    _observe: Subscription,
}

impl SkillsPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            ComposerInput::with_context("Filter installed skills", "PaletteSearch", cx)
                .with_text_metrics(12.0, 16.0)
                .with_accessibility_role(gpui::Role::SearchInput)
                .with_single_line()
                .with_tab_stop()
        });
        let search_events = cx.subscribe(&search, |page: &mut Self, input, event, cx| {
            if matches!(event, ComposerInputEvent::Edited) {
                page.query = input.read(cx).text().to_string();
                page.scroll.reset();
                if page.tab == Tab::Registry {
                    page.search_registry(SEARCH_DEBOUNCE, cx);
                }
                cx.notify();
            }
        });
        // The engine can connect after the page is shown (a boot straight
        // into Skills); load once it does.
        let observe = cx.observe(&state, |page: &mut Self, state, cx| {
            if page.visible
                && state.read(cx).engine().is_some()
                && matches!(page.library, Loadable::Idle)
            {
                page.on_visible(cx);
            }
        });
        let mut tab_fades = crate::motion::HoverFades::default();
        tab_fades.set_at(Tab::Installed.key(), true, true, Instant::now());
        Self {
            state,
            tab: Tab::Installed,
            tab_fades,
            search,
            query: String::new(),
            library: Loadable::Idle,
            library_task: None,
            registry: Loadable::Idle,
            registry_query: None,
            registry_task: None,
            installing: HashMap::new(),
            just_installed: HashSet::new(),
            removing: None,
            remove_task: None,
            detail: None,
            detail_task: None,
            generation: 0,
            scroll: widgets::PageScroll::default(),
            detail_scroll: widgets::PageScroll::default(),
            visible: false,
            focus: cx.focus_handle(),
            selection_prefix: format!("skills-{}-", cx.entity_id().as_u64()),
            expanded_packs: HashSet::new(),
            changes_open: HashSet::new(),
            changes_trees: HashMap::new(),
            pack_busy: HashMap::new(),
            removing_pack: None,
            add_pack: None,
            checked_packs: false,
            pack_install: None,
            toggle_previous: HashMap::new(),
            library_sent: 0,
            library_shown: 0,
            _search_events: search_events,
            _observe: observe,
        }
    }

    /// Re-read the library each visit: skills are edited and added by hand.
    pub fn on_visible(&mut self, cx: &mut Context<Self>) {
        self.visible = true;
        self.load_library(cx);
        if self.tab == Tab::Registry && !matches!(self.registry, Loadable::Ready(_)) {
            self.search_registry(Duration::ZERO, cx);
        }
    }

    pub fn on_hidden(&mut self) {
        self.visible = false;
        self.removing = None;
        self.removing_pack = None;
    }

    /// Escape: an open dialog first, then the detail view.
    pub fn dismiss_on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        if self.removing.take().is_some()
            || self.removing_pack.take().is_some()
            || self.add_pack.take().is_some()
        {
            cx.notify();
            return true;
        }
        if self.detail.is_some() {
            self.close_detail(cx);
            return true;
        }
        false
    }

    fn call<T: serde::de::DeserializeOwned + 'static>(
        &self,
        method: &'static str,
        params: serde_json::Value,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, Result<T, String>, &mut Context<Self>) + 'static,
    ) -> Option<Task<()>> {
        let engine = self.state.read(cx).engine().cloned()?;
        Some(cx.spawn(async move |page, cx| {
            let result = engine
                .client()
                .call(method, params)
                .await
                .map_err(|error| rpc_error_copy(&error))
                .and_then(|value| serde_json::from_value::<T>(value).map_err(|e| e.to_string()));
            page.update(cx, |page, cx| {
                done(page, result, cx);
                cx.notify();
            })
            .ok();
        }))
    }

    fn load_library(&mut self, cx: &mut Context<Self>) {
        if self.state.read(cx).engine().is_none() {
            return;
        }
        if !matches!(self.library, Loadable::Ready(_)) {
            self.library = Loadable::Loading;
        }
        let seq = self.next_library_seq();
        self.library_task = self.call(
            methods::LIST_SKILL_LIBRARY,
            serde_json::json!({}),
            cx,
            move |page, result: Result<Library, String>, cx| {
                page.library_task = None;
                match result {
                    Ok(library) => {
                        page.accept_library(seq, library, cx);
                        page.check_stale_packs(cx);
                    }
                    // A failed refresh keeps the list on screen.
                    Err(error) if matches!(page.library, Loadable::Ready(_)) => {
                        crate::toast::error(cx, error)
                    }
                    Err(error) => page.library = Loadable::Error(error),
                }
            },
        );
        cx.notify();
    }

    fn next_library_seq(&mut self) -> u64 {
        self.library_sent += 1;
        self.library_sent
    }

    /// Show `library` unless a newer request's library is already shown.
    fn accept_library(&mut self, seq: u64, library: Library, cx: &mut Context<Self>) {
        if seq < self.library_shown {
            return;
        }
        self.library_shown = seq;
        self.set_library(library, cx);
    }

    fn set_library(&mut self, library: Library, cx: &mut Context<Self>) {
        let first = !matches!(self.library, Loadable::Ready(_));
        // An open detail follows its skill's fresh row, or closes once the
        // skill is gone (removed by hand, say).
        if let Some(Detail::Installed { skill, .. }) = &mut self.detail {
            match library
                .skills
                .iter()
                .find(|fresh| fresh.directory == skill.directory)
            {
                Some(fresh) => *skill = fresh.clone(),
                None => {
                    self.detail = None;
                    self.detail_task = None;
                }
            }
        }
        self.library = Loadable::Ready(library);
        if first && self.tab == Tab::Installed {
            self.generation += 1;
        }
        cx.notify();
    }

    /// Ask skills.sh for the current query after `delay`; a newer request
    /// replaces this one before it is sent.
    fn search_registry(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let query = self.query.trim().to_string();
        if self.registry_query.as_deref() == Some(query.as_str())
            && !matches!(self.registry, Loadable::Error(_))
        {
            return;
        }
        self.registry_query = Some(query.clone());
        if !matches!(self.registry, Loadable::Ready(_)) {
            self.registry = Loadable::Loading;
        }
        self.registry_task = Some(cx.spawn(async move |page, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let result = engine
                .client()
                .call(
                    methods::SEARCH_SKILL_REGISTRY,
                    serde_json::json!({ "query": query }),
                )
                .await
                .map_err(|error| rpc_error_copy(&error))
                .and_then(|value| {
                    serde_json::from_value::<RegistrySearch>(value).map_err(|e| e.to_string())
                });
            page.update(cx, |page, cx| {
                page.registry_task = None;
                match result {
                    Ok(results) => {
                        page.registry = Loadable::Ready(results);
                        page.generation += 1;
                        page.scroll.reset();
                    }
                    Err(error) => {
                        page.registry_query = None;
                        page.registry = Loadable::Error(error);
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn select_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.tab == tab {
            return;
        }
        for candidate in [Tab::Installed, Tab::Registry] {
            self.tab_fades.set_at(
                candidate.key(),
                candidate == tab,
                crate::motion::reduced_motion(cx),
                Instant::now(),
            );
        }
        self.tab = tab;
        self.generation += 1;
        self.scroll.reset();
        self.search.update(cx, |input, cx| {
            input.set_placeholder(
                match tab {
                    Tab::Installed => "Filter installed skills",
                    Tab::Registry => "Search skills.sh",
                },
                cx,
            )
        });
        if tab == Tab::Registry {
            self.search_registry(Duration::ZERO, cx);
        }
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        match self.tab {
            Tab::Installed => self.load_library(cx),
            Tab::Registry => {
                self.registry_query = None;
                self.search_registry(Duration::ZERO, cx);
            }
        }
    }

    fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.search.update(cx, |input, cx| input.set_text("", cx));
        self.query.clear();
        self.scroll.reset();
        if self.tab == Tab::Registry {
            self.search_registry(Duration::ZERO, cx);
        }
        cx.notify();
    }

    // ---- detail ----

    fn open_installed(&mut self, skill: LibrarySkill, window: &mut Window, cx: &mut Context<Self>) {
        let directory = skill.directory.clone();
        self.detail = Some(Detail::Installed {
            skill,
            document: Loadable::Loading,
        });
        self.detail_scroll.reset();
        self.detail_task = self.call(
            methods::READ_LIBRARY_SKILL,
            serde_json::json!({ "directory": directory }),
            cx,
            move |page, result: Result<SkillContents, String>, _| {
                page.detail_task = None;
                if let Some(Detail::Installed { skill, document }) = &mut page.detail
                    && skill.directory == directory
                {
                    *document = result.map_or_else(Loadable::Error, |contents| {
                        Loadable::Ready(Rc::new(Document {
                            description: None,
                            files: contents.files,
                            truncated: contents.truncated,
                            tree: Rc::new(crate::markdown::parse_full(&contents.markdown)),
                        }))
                    });
                }
            },
        );
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn open_registry(&mut self, skill: RegistrySkill, window: &mut Window, cx: &mut Context<Self>) {
        let id = skill.id.clone();
        self.detail = Some(Detail::Registry {
            skill,
            document: Loadable::Loading,
        });
        self.detail_scroll.reset();
        self.detail_task = self.call(
            methods::PREVIEW_REGISTRY_SKILL,
            serde_json::json!({ "id": id }),
            cx,
            move |page, result: Result<RegistryPreview, String>, _| {
                page.detail_task = None;
                if let Some(Detail::Registry { skill, document }) = &mut page.detail
                    && skill.id == id
                {
                    *document = result.map_or_else(Loadable::Error, |preview| {
                        Loadable::Ready(Rc::new(Document {
                            description: preview.description,
                            files: Vec::new(),
                            truncated: false,
                            tree: Rc::new(crate::markdown::parse_full(&preview.markdown)),
                        }))
                    });
                }
            },
        );
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn close_detail(&mut self, cx: &mut Context<Self>) {
        crate::markdown::render::clear_selection_surface(&self.selection_prefix);
        self.detail = None;
        self.detail_task = None;
        self.generation += 1;
        cx.notify();
    }

    // ---- install / remove ----

    /// The library skill a registry listing installed, if any: by its
    /// recorded registry id, else by a folder of the same name.
    fn installed_as(&self, skill: &RegistrySkill) -> Option<&LibrarySkill> {
        let library = self.library.ready()?;
        library
            .skills
            .iter()
            .find(|installed| installed.registry_id.as_deref() == Some(skill.id.as_str()))
            .or_else(|| {
                library.skills.iter().find(|installed| {
                    std::path::Path::new(&installed.directory)
                        .file_name()
                        .is_some_and(|name| name.eq_ignore_ascii_case(&skill.skill_id))
                })
            })
    }

    fn install(&mut self, skill: RegistrySkill, cx: &mut Context<Self>) {
        if self.installing.contains_key(&skill.id) || self.installed_as(&skill).is_some() {
            return;
        }
        let id = skill.id.clone();
        let task = self.call(
            methods::INSTALL_REGISTRY_SKILL,
            serde_json::json!({ "id": id }),
            cx,
            move |page, result: Result<LibrarySkill, String>, cx| {
                page.installing.remove(&skill.id);
                match result {
                    Ok(installed) => {
                        page.just_installed.insert(skill.id.clone());
                        crate::toast::success(
                            cx,
                            format!("Installed {}. New sessions can use it.", installed.name),
                        );
                        if let Loadable::Ready(library) = &mut page.library {
                            library.skills.push(installed);
                            library
                                .skills
                                .sort_by_key(|skill| skill.name.to_lowercase());
                        }
                        page.load_library(cx);
                    }
                    Err(error) => {
                        crate::toast::error(cx, format!("Couldn't install {}. {error}", skill.name))
                    }
                }
            },
        );
        if let Some(task) = task {
            self.installing.insert(id, task);
        }
        cx.notify();
    }

    fn confirm_remove(&mut self, cx: &mut Context<Self>) {
        let Some(skill) = self.removing.take() else {
            return;
        };
        let name = skill.name.clone();
        let directory = skill.directory.clone();
        let seq = self.next_library_seq();
        self.remove_task = self.call(
            methods::REMOVE_LIBRARY_SKILL,
            serde_json::json!({ "directory": directory }),
            cx,
            move |page, result: Result<Library, String>, cx| {
                page.remove_task = None;
                match result {
                    Ok(library) => {
                        if matches!(
                            &page.detail,
                            Some(Detail::Installed { skill, .. }) if skill.directory == directory
                        ) {
                            page.close_detail(cx);
                        }
                        page.accept_library(seq, library, cx);
                        page.load_library(cx);
                        crate::toast::success(cx, format!("Removed {name}."));
                    }
                    Err(error) => {
                        crate::toast::error(cx, format!("Couldn't remove {name}. {error}"))
                    }
                }
            },
        );
        cx.notify();
    }

    fn reveal_library(&self, cx: &mut Context<Self>) {
        let root = self
            .library
            .ready()
            .map(|library| std::path::PathBuf::from(&library.root))
            .unwrap_or_else(zeron_engine::skill_library::root);
        // A fresh install has no folder yet; make it so Finder has one to show.
        if std::fs::create_dir_all(&root).is_ok() {
            cx.reveal_path(&root);
        }
    }
}

/// Drives the page for the native review fixture (`examples/skills-fixture.rs`).
#[cfg(feature = "appshots-fixture")]
impl SkillsPage {
    pub fn fixture_tab(&mut self, registry: bool, cx: &mut Context<Self>) {
        self.close_detail(cx);
        self.select_tab(
            if registry {
                Tab::Registry
            } else {
                Tab::Installed
            },
            cx,
        );
    }

    pub fn fixture_search(&mut self, query: &str, cx: &mut Context<Self>) {
        self.search
            .update(cx, |input, cx| input.set_text(query, cx));
        self.query = query.into();
        if self.tab == Tab::Registry {
            self.search_registry(Duration::ZERO, cx);
        }
        cx.notify();
    }

    /// Open the row at `index` of the current tab's list.
    pub fn fixture_open(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        match self.tab {
            Tab::Installed => {
                if let Some(skill) = self
                    .library
                    .ready()
                    .and_then(|library| library.skills.get(index).cloned())
                {
                    self.open_installed(skill, window, cx);
                }
            }
            Tab::Registry => {
                if let Some(skill) = self
                    .registry
                    .ready()
                    .and_then(|results| results.skills.get(index).cloned())
                {
                    self.open_registry(skill, window, cx);
                }
            }
        }
    }

    pub fn fixture_ask_remove(&mut self, index: usize, cx: &mut Context<Self>) {
        self.removing = self
            .library
            .ready()
            .and_then(|library| library.skills.get(index).cloned());
        cx.notify();
    }

    pub fn fixture_dismiss(&mut self, cx: &mut Context<Self>) {
        self.removing = None;
        self.removing_pack = None;
        self.add_pack = None;
        self.close_detail(cx);
    }

    /// Expand `pack`, optionally opening its waiting update's notes.
    pub fn fixture_expand_pack(&mut self, pack: &str, notes: bool, cx: &mut Context<Self>) {
        self.expanded_packs.insert(pack.into());
        if notes
            && let Some(update) = self.library.ready().and_then(|library| {
                library
                    .packs
                    .iter()
                    .find(|candidate| candidate.name == pack)
                    .and_then(|pack| pack.update.clone())
            })
        {
            self.changes_open
                .insert(format!("{pack}@{}", update.commit));
        }
        cx.notify();
    }

    pub fn fixture_collapse_packs(&mut self, cx: &mut Context<Self>) {
        self.expanded_packs.clear();
        self.changes_open.clear();
        cx.notify();
    }

    /// Open Add pack with `source` entered and inspect it.
    pub fn fixture_add_pack(&mut self, source: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.open_add_pack(window, cx);
        self.fixture_add_pack_source(source, cx);
    }
}

impl gpui::Focusable for SkillsPage {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl popover::ScrollRailHost for SkillsPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        if self.detail.is_some() {
            self.detail_scroll.rail_bar()
        } else {
            self.scroll.rail_bar()
        }
    }
    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        if self.detail.is_some() {
            self.detail_scroll.rail_scroll()
        } else {
            self.scroll.rail_scroll()
        }
    }
}

impl Render for SkillsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        if self.tab_fades.tick_at(Instant::now()) {
            window.request_animation_frame();
        }
        let viewport = window.viewport_size();
        let dialog = self
            .render_remove_dialog(viewport, cx)
            .or_else(|| self.render_remove_pack_dialog(viewport, cx))
            .or_else(|| self.render_add_pack_dialog(viewport, cx));
        let body = if self.detail.is_some() {
            self.render_detail(&theme, window, cx)
        } else {
            self.render_board(&theme, window, cx)
        };
        div()
            .id("skills-page")
            .debug_selector(|| "skills-page".into())
            .track_focus(&self.focus)
            .key_context("SkillsPage")
            .on_key_down(cx.listener(|page, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && page.dismiss_on_escape(cx) {
                    cx.stop_propagation();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .pt(px(Theme::TITLEBAR_HEIGHT + Theme::SPACE_LG))
            .child(body)
            .children(dialog)
    }
}

// ---- the board ----

impl SkillsPage {
    fn render_board(
        &mut self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let header = self.render_header(theme, cx);
        let content = match self.tab {
            Tab::Installed => self.render_installed(theme, window, cx),
            Tab::Registry => self.render_registry(theme, cx),
        };
        let scroll = self.scroll.scroll.clone();
        let rail = popover::rail(self, "skills-scrollbar", theme, cx);
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .id("skills-scroll-host")
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .on_hover(cx.listener(|page, hovered: &bool, _, cx| {
                        if page.scroll.set_list_hovered(*hovered) {
                            cx.notify();
                        }
                    }))
                    .child(
                        crate::edge_fade::edge_faded(
                            SCROLL_FADE_BAND,
                            true,
                            true,
                            div()
                                .id("skills-scroll")
                                .debug_selector(|| "skills-scroll".into())
                                .size_full()
                                .overflow_y_scroll()
                                .track_scroll(&scroll)
                                .child(
                                    div()
                                        .w_full()
                                        .max_w(px(PAGE_MAX_WIDTH))
                                        .mx_auto()
                                        .px(px(PAGE_HORIZONTAL_PADDING))
                                        .pt(px(8.0))
                                        .pb(px(32.0))
                                        .child(content),
                                ),
                        )
                        .fade_overflow_y(&scroll),
                    )
                    .children(rail),
            )
            .into_any_element()
    }

    fn render_header(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let loading = match self.tab {
            Tab::Installed => self.library_task.is_some(),
            Tab::Registry => self.registry_task.is_some(),
        };
        let refresh_label = match self.tab {
            Tab::Installed => "Reload installed skills",
            Tab::Registry => "Refresh skills.sh results",
        };
        let subtitle: SharedString = match self.tab {
            Tab::Installed => {
                "Every provider loads these from ~/.zeron/skills, in every new session.".into()
            }
            Tab::Registry => match &self.registry {
                Loadable::Ready(results) if results.popular => {
                    "The most installed skills on skills.sh.".into()
                }
                Loadable::Ready(results) => match results.skills.len() {
                    0 => format!("No skills.sh results for “{}”.", results.query.trim()).into(),
                    1 => format!("1 result for “{}”.", results.query.trim()).into(),
                    n => format!("{n} results for “{}”.", results.query.trim()).into(),
                },
                _ => "Skills published on skills.sh, installed for every provider.".into(),
            },
        };
        widgets::page_column()
            .id("skills-column")
            .max_w(px(PAGE_MAX_WIDTH))
            .pt_0()
            .pb(px(Theme::SPACE_LG))
            .flex_none()
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap(px(12.0))
                    .child(widgets::page_header(theme, "Skills", None))
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(
                                widgets::action_button(theme, widgets::ActionTone::Quiet)
                                    .id("skills-add-pack")
                                    .debug_selector(|| "skills-add-pack".into())
                                    .role(gpui::Role::Button)
                                    .tab_index(0)
                                    .min_h(px(HEADER_CONTROL_HEIGHT))
                                    .child(
                                        icon(icons::PLUS)
                                            .size(px(14.0))
                                            .text_color(theme.text_muted),
                                    )
                                    .child("Add pack")
                                    .tooltip(widgets::text_tooltip(
                                        "Install a git repository of skills",
                                    ))
                                    .on_click(cx.listener(|page, _, window, cx| {
                                        page.open_add_pack(window, cx)
                                    })),
                            )
                            .child(
                                header_icon_button("skills-refresh", refresh_label, theme)
                                    .when(loading, |el| el.opacity(0.5))
                                    .on_click(cx.listener(|page, _, _, cx| page.refresh(cx)))
                                    .child(if loading {
                                        crate::loaders::mini_glyph_spinner(
                                            "skills-refresh-spinner",
                                            1.75,
                                            theme.glyph,
                                            cx.entity_id(),
                                            cx,
                                        )
                                        .into_any_element()
                                    } else {
                                        icon(icons::REFRESH)
                                            .size(px(16.0))
                                            .text_color(theme.text_muted)
                                            .into_any_element()
                                    }),
                            )
                            .child(
                                header_icon_button("skills-reveal-folder", reveal_label(), theme)
                                    .on_click(cx.listener(|page, _, _, cx| page.reveal_library(cx)))
                                    .child(
                                        icon(icons::FOLDER)
                                            .size(px(16.0))
                                            .text_color(theme.text_muted),
                                    ),
                            ),
                    ),
            )
            .child(widgets::page_subtitle(theme, subtitle))
            .child(
                div()
                    .mt(px(16.0))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(12.0))
                    .child(self.render_tabs(theme, cx))
                    .child(self.render_search(theme, cx)),
            )
            .into_any_element()
    }

    fn render_tabs(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let installed = self.library.ready().map(|library| library.skills.len());
        div()
            .id("skills-tab-tray")
            .role(gpui::Role::TabList)
            .aria_label("Skills to show")
            .h(px(HEADER_CONTROL_HEIGHT))
            .flex_none()
            .p(px(2.0))
            .rounded(px(8.0))
            .bg(crate::theme::ink(0.035))
            .flex()
            .items_center()
            .gap(px(2.0))
            .children(
                [(Tab::Installed, "Installed"), (Tab::Registry, "skills.sh")].map(
                    |(tab, label)| {
                        let id = tab.key();
                        let chosen = self.tab == tab;
                        let selected = self.tab_fades.value_at(id, Instant::now());
                        let hover_key = format!("{id}-{}", cx.entity_id());
                        let hover = crate::motion::hover_t(&hover_key);
                        let fill = move || {
                            crate::theme::wash(0.12 * selected + 0.06 * hover * (1.0 - selected))
                        };
                        crate::surface_chrome::tab_frame(id, chosen, theme)
                            .role(gpui::Role::Tab)
                            .debug_selector(move || id.into())
                            .h(px(HEADER_CONTROL_HEIGHT - 4.0))
                            .px(px(12.0))
                            .gap(px(6.0))
                            .bg(fill())
                            .when(chosen, |el| {
                                el.shadow(crate::theme::card_selected_shadows())
                            })
                            .text_color(crate::motion::mix(
                                theme.text_muted,
                                theme.text,
                                selected.max(hover),
                            ))
                            .text_size(crate::typography::ui_rems(12.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .on_hover(crate::motion::hover_listener(hover_key))
                            .hover(move |style| style.bg(fill()))
                            .child(label)
                            .when_some(installed.filter(|_| tab == Tab::Installed), |el, count| {
                                el.child(
                                    div()
                                        .text_size(crate::typography::ui_rems(11.0))
                                        .text_color(theme.text_muted)
                                        .child(SharedString::from(count.to_string())),
                                )
                            })
                            .on_click(cx.listener(move |page, _, _, cx| page.select_tab(tab, cx)))
                    },
                ),
            )
            .into_any_element()
    }

    fn render_search(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_1()
            .min_w(px(180.0))
            .child(
                crate::surface_chrome::input()
                    .h(px(HEADER_CONTROL_HEIGHT))
                    .rounded(px(8.0))
                    .min_w(px(160.0))
                    .child(
                        icon(icons::MAGNIFER)
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(16.0))
                            .overflow_hidden()
                            .child(self.search.clone()),
                    )
                    .when(!self.query.is_empty(), |el| {
                        el.child(
                            div()
                                .id("skills-clear-search")
                                .role(gpui::Role::Button)
                                .aria_label("Clear search")
                                .tab_index(0)
                                .size(px(24.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4.0))
                                .cursor_pointer()
                                .hover(|style| style.bg(crate::theme::ink(0.04)))
                                .on_click(cx.listener(|page, _, _, cx| page.clear_search(cx)))
                                .child(
                                    icon(icons::CLOSE)
                                        .size(px(12.0))
                                        .text_color(theme.text_muted),
                                ),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_installed(
        &mut self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let library = match &self.library {
            Loadable::Ready(library) => library.clone(),
            Loadable::Error(error) => {
                return empty_state(
                    icons::INFO_CIRCLE,
                    "Couldn't read your skills",
                    error.clone(),
                    theme,
                )
                .child(
                    empty_action("skills-retry-library", "Try again", theme)
                        .on_click(cx.listener(|page, _, _, cx| page.load_library(cx))),
                )
                .into_any_element();
            }
            Loadable::Idle | Loadable::Loading => {
                return skeleton(cx.entity_id(), theme, cx);
            }
        };
        if library.skills.is_empty() && library.packs.is_empty() {
            return empty_state(
                icons::MAGIC_STICK_3,
                "No shared skills yet",
                "Install one from skills.sh, add a pack from a git repository, or put a folder with a SKILL.md in ~/.zeron/skills. Every provider picks it up.",
                theme,
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        empty_action("skills-browse-registry", "Browse skills.sh", theme)
                            .on_click(cx.listener(|page, _, _, cx| page.select_tab(Tab::Registry, cx))),
                    )
                    .child(
                        empty_action("skills-empty-add-pack", "Add a pack", theme).on_click(
                            cx.listener(|page, _, window, cx| page.open_add_pack(window, cx)),
                        ),
                    ),
            )
            .into_any_element();
        }
        let packs = self.render_packs(&library, theme, window, cx);
        // Pack skills are listed under their pack.
        let matches: Vec<LibrarySkill> = library
            .skills
            .iter()
            .filter(|skill| skill.pack.is_none())
            .filter(|skill| matches_query(&[&skill.name, &skill.description], &self.query))
            .cloned()
            .collect();
        if matches.is_empty() && packs.is_none() {
            let query = self.query.trim().to_string();
            return empty_state(
                icons::MAGNIFER,
                "No installed skills match",
                format!("Nothing installed matches “{query}”. skills.sh may have it."),
                theme,
            )
            .child(
                empty_action("skills-search-registry", "Search skills.sh", theme)
                    .on_click(cx.listener(|page, _, _, cx| page.select_tab(Tab::Registry, cx))),
            )
            .into_any_element();
        }
        let skills = (!matches.is_empty()).then(|| {
            let last = matches.len() - 1;
            let rows: Vec<AnyElement> = matches
                .into_iter()
                .enumerate()
                .map(|(index, skill)| {
                    self.render_installed_row(skill, index == 0, index == last, theme, cx)
                })
                .collect();
            let list = card().children(rows);
            if packs.is_some() {
                widgets::section(theme, "Skills", list)
                    .mt(px(28.0))
                    .into_any_element()
            } else {
                list.into_any_element()
            }
        });
        crate::motion::fade_in(
            ("skills-installed-list", self.generation as usize),
            div()
                .w_full()
                .flex()
                .flex_col()
                .children(packs)
                .children(skills),
        )
        .into_any_element()
    }

    fn render_installed_row(
        &mut self,
        skill: LibrarySkill,
        first: bool,
        last: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = format!("skills-row-{}-{}", cx.entity_id(), skill.directory);
        let hover = crate::motion::hover_t(&key);
        // A pack's card already names where its skills come from.
        let origin: Option<SharedString> = match (&skill.pack, &skill.registry_id) {
            (Some(_), _) => None,
            (None, Some(id)) => Some(registry_source(id).into()),
            (None, None) if skill.linked => Some("Linked folder".into()),
            (None, None) => Some("Local".into()),
        };
        let edited = skill.modified_at.map(|at| {
            SharedString::from(format!(
                "Edited {}",
                crate::pull_requests::compact_relative_time(at, Utc::now())
            ))
        });
        let actions = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(2.0))
            .opacity(0.4 + 0.6 * hover)
            .child(
                row_icon_button(
                    format!("skills-row-reveal-{}", skill.directory),
                    reveal_label(),
                    icons::FOLDER,
                    theme,
                )
                .on_click({
                    let directory = skill.directory.clone();
                    move |_, _, cx| {
                        cx.stop_propagation();
                        cx.reveal_path(std::path::Path::new(&directory));
                    }
                }),
            )
            .child(
                row_icon_button(
                    format!("skills-row-open-{}", skill.directory),
                    "Open SKILL.md in your editor",
                    icons::PEN,
                    theme,
                )
                .on_click({
                    let file = skill.file.clone();
                    move |_, _, cx| {
                        cx.stop_propagation();
                        cx.open_with_system(std::path::Path::new(&file));
                    }
                }),
            )
            .child(match &skill.pack {
                Some(pack) => pack_owned_remove(
                    format!("skills-row-remove-{}", skill.directory),
                    pack,
                    theme,
                ),
                None => row_icon_button(
                    format!("skills-row-remove-{}", skill.directory),
                    "Remove skill",
                    icons::TRASH_BIN_MINIMALISTIC,
                    theme,
                )
                .on_click(cx.listener({
                    let skill = skill.clone();
                    move |page, _, _, cx| {
                        cx.stop_propagation();
                        page.removing = Some(skill.clone());
                        cx.notify();
                    }
                }))
                .into_any_element(),
            });
        row_shell(first, last, theme)
            .id(SharedString::from(key.clone()))
            .debug_selector(|| "skills-installed-row".into())
            .role(gpui::Role::Button)
            .aria_label(format!("{}. {}. Show skill", skill.name, skill.description))
            .tab_index(0)
            .bg(row_fill(&key, theme))
            .on_hover(crate::motion::hover_listener(key.clone()))
            .cursor_pointer()
            .child(skill_tile(theme))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(widgets::row_title(theme, skill.name.clone()).truncate())
                    .when(!skill.description.is_empty(), |el| {
                        el.child(
                            div()
                                .mt(px(Theme::TEXT_STACK_GAP))
                                .min_w_0()
                                .truncate()
                                .text_size(crate::typography::ui_rems(12.0))
                                .line_height(crate::typography::ui_rems(16.0))
                                .text_color(theme.text_muted)
                                .child(SharedString::from(single_line(&skill.description))),
                        )
                    })
                    .child(widgets::meta_line(
                        theme,
                        origin
                            .into_iter()
                            .chain(edited)
                            .map(|text| meta_text(text, theme))
                            .collect(),
                    )),
            )
            .child(actions)
            .on_click(cx.listener(move |page, _, window, cx| {
                page.open_installed(skill.clone(), window, cx)
            }))
            .into_any_element()
    }

    fn render_registry(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let results = match &self.registry {
            Loadable::Ready(results) => results,
            Loadable::Error(error) => {
                return empty_state(
                    icons::WIFI_OFF,
                    "skills.sh is out of reach",
                    error.clone(),
                    theme,
                )
                .child(
                    empty_action("skills-retry-registry", "Try again", theme)
                        .on_click(cx.listener(|page, _, _, cx| page.refresh(cx))),
                )
                .into_any_element();
            }
            Loadable::Idle | Loadable::Loading => return skeleton(cx.entity_id(), theme, cx),
        };
        if results.skills.is_empty() {
            return empty_state(
                icons::MAGNIFER,
                "Nothing on skills.sh matches",
                "Try a broader word, or the name of the repository that publishes it.",
                theme,
            )
            .child(
                empty_action("skills-show-popular", "Show popular skills", theme)
                    .on_click(cx.listener(|page, _, _, cx| page.clear_search(cx))),
            )
            .into_any_element();
        }
        let skills = results.skills.clone();
        let popular = results.popular;
        let last = skills.len() - 1;
        let rows: Vec<AnyElement> = skills
            .into_iter()
            .enumerate()
            .map(|(index, skill)| {
                self.render_registry_row(
                    skill,
                    popular.then_some(index + 1),
                    index == 0,
                    index == last,
                    theme,
                    cx,
                )
            })
            .collect();
        crate::motion::fade_in(
            ("skills-registry-list", self.generation as usize),
            card().children(rows),
        )
        .into_any_element()
    }

    fn render_registry_row(
        &mut self,
        skill: RegistrySkill,
        rank: Option<usize>,
        first: bool,
        last: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = format!("skills-registry-row-{}-{}", cx.entity_id(), skill.id);
        let control = self.render_install_control(&skill, false, theme, cx);
        row_shell(first, last, theme)
            .id(SharedString::from(key.clone()))
            .debug_selector(|| "skills-registry-row".into())
            .role(gpui::Role::Button)
            .aria_label(format!(
                "{} from {}, {} installs. Show skill",
                skill.name,
                skill.source,
                crate::pull_requests::format_compact_count(skill.installs)
            ))
            .tab_index(0)
            .bg(row_fill(&key, theme))
            .on_hover(crate::motion::hover_listener(key.clone()))
            .cursor_pointer()
            .child(match rank {
                // The leaderboard's place stands where the glyph would.
                Some(rank) => div()
                    .flex_none()
                    .w(px(24.0))
                    .h(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(crate::typography::ui_rems(12.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .child(SharedString::from(rank.to_string())),
                None => skill_tile(theme),
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(widgets::row_title(theme, skill.name.clone()).truncate())
                    .child(widgets::meta_line(
                        theme,
                        vec![
                            meta_text(skill.source.clone(), theme),
                            meta_text(installs_label(skill.installs), theme),
                        ],
                    )),
            )
            .child(control)
            .on_click(
                cx.listener(move |page, _, window, cx| {
                    page.open_registry(skill.clone(), window, cx)
                }),
            )
            .into_any_element()
    }

    /// Install, the spinner while it runs, then the Installed mark. `large`
    /// is the detail view's primary action; rows use the filled tone.
    fn render_install_control(
        &mut self,
        skill: &RegistrySkill,
        large: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id_suffix = format!("{}-{}", if large { "detail" } else { "row" }, skill.id);
        if self.installing.contains_key(&skill.id) {
            return widgets::action_button(theme, widgets::ActionTone::Filled)
                .id(SharedString::from(format!("skills-installing-{id_suffix}")))
                .aria_label(format!("Installing {}", skill.name))
                .flex_none()
                .cursor_default()
                .text_color(theme.text_muted)
                .child(crate::loaders::mini_glyph_spinner(
                    format!("skills-install-spinner-{id_suffix}"),
                    1.5,
                    theme.glyph,
                    cx.entity_id(),
                    cx,
                ))
                .child("Installing")
                .into_any_element();
        }
        if self.installed_as(skill).is_some() {
            let mark = div()
                .id(SharedString::from(format!("skills-installed-{id_suffix}")))
                .flex_none()
                .min_h(px(32.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_size(crate::typography::ui_rems(12.5))
                .text_color(theme.success_muted)
                .child(icon(icons::CHECK).size(px(14.0)).text_color(theme.success))
                .child("Installed");
            // Only a fresh install animates; a skill already installed is
            // simply marked.
            return if self.just_installed.contains(&skill.id) {
                crate::motion::fade_in(format!("skills-installed-in-{id_suffix}"), mark)
                    .into_any_element()
            } else {
                mark.into_any_element()
            };
        }
        let tone = if large {
            widgets::ActionTone::Solid
        } else {
            widgets::ActionTone::Filled
        };
        let skill = skill.clone();
        widgets::action_button(theme, tone)
            .id(SharedString::from(format!("skills-install-{id_suffix}")))
            .debug_selector(|| "skills-install".into())
            .role(gpui::Role::Button)
            .aria_label(format!("Install {}", skill.name))
            .tab_index(0)
            .flex_none()
            .child("Install")
            .on_click(cx.listener(move |page, _, _, cx| {
                cx.stop_propagation();
                page.install(skill.clone(), cx);
            }))
            .into_any_element()
    }

    // ---- detail ----

    fn render_detail(
        &mut self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(detail) = self.detail.as_ref() else {
            return div().into_any_element();
        };
        let key = detail.key();
        let (name, description, meta, document): (
            String,
            Option<String>,
            Vec<SharedString>,
            Loadable<Rc<Document>>,
        ) = match detail {
            Detail::Installed { skill, document } => (
                skill.name.clone(),
                Some(skill.description.clone()).filter(|text| !text.is_empty()),
                std::iter::once(SharedString::from(display_path(&skill.directory)))
                    .chain(
                        skill
                            .pack
                            .as_deref()
                            .map(|pack| SharedString::from(format!("From the {pack} pack"))),
                    )
                    .chain(skill.registry_id.as_deref().map(|id| {
                        SharedString::from(format!("From skills.sh · {}", registry_source(id)))
                    }))
                    .chain(skill.modified_at.map(|at| {
                        SharedString::from(format!(
                            "Edited {}",
                            crate::pull_requests::relative_time(at, Utc::now())
                        ))
                    }))
                    .collect(),
                clone_loadable(document),
            ),
            Detail::Registry { skill, document } => (
                skill.name.clone(),
                document
                    .ready()
                    .and_then(|document| document.description.clone()),
                vec![
                    SharedString::from(skill.source.clone()),
                    installs_label(skill.installs),
                ],
                clone_loadable(document),
            ),
        };
        let actions = self.render_detail_actions(theme, cx);
        let files = self.render_detail_files(&document, theme);
        let body = self.render_document(&document, &key, theme, window, cx);
        let scroll = self.detail_scroll.scroll.clone();
        let rail = popover::rail(self, "skills-detail-scrollbar", theme, cx);
        let column = div()
            .w_full()
            .max_w(px(PAGE_MAX_WIDTH))
            .mx_auto()
            .px(px(PAGE_HORIZONTAL_PADDING))
            .pb(px(48.0))
            .flex()
            .flex_col()
            .child(
                widgets::ghost_action(theme)
                    .id("skills-detail-back")
                    .debug_selector(|| "skills-detail-back".into())
                    .role(gpui::Role::Button)
                    .aria_label("Back to skills")
                    .tab_index(0)
                    .self_start()
                    .ml(px(-6.0))
                    .px(px(6.0))
                    .gap(px(4.0))
                    .child(
                        icon(icons::ALT_ARROW_LEFT)
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    )
                    .child("Skills")
                    .on_click(cx.listener(|page, _, _, cx| page.close_detail(cx))),
            )
            .child(
                div()
                    .mt(px(16.0))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex_none()
                            .size(px(40.0))
                            .rounded(px(10.0))
                            .bg(widgets::block_fill(theme))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                icon(icons::MAGIC_STICK_3)
                                    .size(px(20.0))
                                    .text_color(theme.text),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(160.0))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(crate::typography::ui_rems(20.0))
                                    .line_height(crate::typography::ui_rems(26.0))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(theme.text)
                                    .child(SharedString::from(name)),
                            )
                            .child(widgets::meta_line(
                                theme,
                                meta.into_iter()
                                    .map(|text| meta_text(text, theme))
                                    .collect(),
                            )),
                    )
                    .child(actions),
            )
            .when_some(description, |el, description| {
                el.child(
                    div()
                        .mt(px(16.0))
                        .text_size(crate::typography::ui_rems(13.0))
                        .line_height(crate::typography::ui_rems(19.0))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(description)),
                )
            })
            .child(widgets::section(theme, "SKILL.md", body))
            .children(files);
        let content = crate::motion::fade_in(
            SharedString::from(format!("skills-detail-{key}")),
            div().w_full().child(column),
        );
        div()
            .id("skills-detail-host")
            .flex_1()
            .min_h_0()
            .relative()
            .on_hover(cx.listener(|page, hovered: &bool, _, cx| {
                if page.detail_scroll.set_list_hovered(*hovered) {
                    cx.notify();
                }
            }))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|page, _, window, cx| window.focus(&page.focus, cx)),
            )
            .on_key_down(|event: &gpui::KeyDownEvent, _, cx| {
                let keystroke = &event.keystroke;
                if keystroke.key == "c"
                    && (keystroke.modifiers.platform || keystroke.modifiers.control)
                    && !keystroke.modifiers.shift
                    && !keystroke.modifiers.alt
                    && let Some(text) = crate::markdown::selection::selected_text()
                {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    cx.stop_propagation();
                }
            })
            // Paint before Markdown registers this frame's text geometry.
            .child(crate::markdown::render::selection_frame_reset_for(
                cx.entity_id().as_u64(),
            ))
            .child(
                crate::edge_fade::edge_faded(
                    SCROLL_FADE_BAND,
                    true,
                    true,
                    div()
                        .id("skills-detail-scroll")
                        .debug_selector(|| "skills-detail-scroll".into())
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&scroll)
                        .child(content),
                )
                .fade_overflow_y(&scroll),
            )
            .children(rail)
            .into_any_element()
    }

    fn render_detail_actions(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let row = div().flex_none().flex().items_center().gap(px(4.0));
        match self.detail.as_ref() {
            Some(Detail::Installed { skill, .. }) => {
                let skill = skill.clone();
                row.child(
                    widgets::action_button(theme, widgets::ActionTone::Filled)
                        .id("skills-detail-open")
                        .debug_selector(|| "skills-detail-open".into())
                        .role(gpui::Role::Button)
                        .tab_index(0)
                        .child(icon(icons::PEN).size(px(14.0)).text_color(theme.text))
                        .child("Open in editor")
                        .tooltip(widgets::text_tooltip(
                            "Open SKILL.md in your default editor",
                        ))
                        .on_click({
                            let file = skill.file.clone();
                            move |_, _, cx| cx.open_with_system(std::path::Path::new(&file))
                        }),
                )
                .child(
                    widgets::action_button(theme, widgets::ActionTone::Quiet)
                        .id("skills-detail-reveal")
                        .role(gpui::Role::Button)
                        .aria_label(reveal_label())
                        .tab_index(0)
                        .child(
                            icon(icons::FOLDER)
                                .size(px(14.0))
                                .text_color(theme.text_muted),
                        )
                        .tooltip(widgets::text_tooltip(reveal_label()))
                        .on_click({
                            let directory = skill.directory.clone();
                            move |_, _, cx| cx.reveal_path(std::path::Path::new(&directory))
                        }),
                )
                .when_some(skill.pack.clone(), |row, pack| {
                    row.child(pack_owned_remove(
                        "skills-detail-remove".into(),
                        &pack,
                        theme,
                    ))
                })
                .when(skill.pack.is_none(), |row| {
                    row.child(
                        // Quiet, but warming toward danger under the pointer.
                        div()
                            .id("skills-detail-remove")
                            .debug_selector(|| "skills-detail-remove".into())
                            .role(gpui::Role::Button)
                            .aria_label("Remove skill")
                            .tab_index(0)
                            .min_h(px(32.0))
                            .px(px(10.0))
                            .flex()
                            .items_center()
                            .rounded(px(8.0))
                            .cursor_pointer()
                            .bg(crate::motion::hover_blend(
                                "skills-detail-remove",
                                theme.danger.opacity(0.0),
                                theme.danger.opacity(0.12),
                            ))
                            .on_hover(crate::motion::hover_listener("skills-detail-remove"))
                            .child(
                                icon(icons::TRASH_BIN_MINIMALISTIC)
                                    .size(px(14.0))
                                    .text_color(crate::motion::hover_blend(
                                        "skills-detail-remove",
                                        theme.text_muted,
                                        theme.danger,
                                    )),
                            )
                            .tooltip(widgets::text_tooltip("Remove skill"))
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.removing = Some(skill.clone());
                                cx.notify();
                            })),
                    )
                })
                .into_any_element()
            }
            Some(Detail::Registry { skill, .. }) => {
                let skill = skill.clone();
                let installed = self.installed_as(&skill).cloned();
                let url = format!("https://skills.sh/{}", skill.id);
                row.when_some(installed, |row, installed| {
                    row.child(
                        widgets::action_button(theme, widgets::ActionTone::Quiet)
                            .id("skills-detail-show-installed")
                            .role(gpui::Role::Button)
                            .tab_index(0)
                            .child("Show installed")
                            .on_click(cx.listener(move |page, _, window, cx| {
                                page.tab = Tab::Installed;
                                page.select_tab_fades(Tab::Installed, cx);
                                page.open_installed(installed.clone(), window, cx);
                            })),
                    )
                })
                .child(
                    widgets::action_button(theme, widgets::ActionTone::Quiet)
                        .id("skills-detail-web")
                        .role(gpui::Role::Button)
                        .aria_label("View on skills.sh")
                        .tab_index(0)
                        .child(
                            icon(icons::ARROW_UP_RIGHT)
                                .size(px(14.0))
                                .text_color(theme.text_muted),
                        )
                        .tooltip(widgets::text_tooltip("View on skills.sh"))
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                )
                .child(self.render_install_control(&skill, true, theme, cx))
                .into_any_element()
            }
            None => row.into_any_element(),
        }
    }

    /// Point the tray at `tab` without the side effects of switching to it.
    fn select_tab_fades(&mut self, tab: Tab, cx: &mut Context<Self>) {
        for candidate in [Tab::Installed, Tab::Registry] {
            self.tab_fades.set_at(
                candidate.key(),
                candidate == tab,
                crate::motion::reduced_motion(cx),
                Instant::now(),
            );
        }
        self.search.update(cx, |input, cx| {
            input.set_placeholder("Filter installed skills", cx)
        });
    }

    fn render_detail_files(
        &self,
        document: &Loadable<Rc<Document>>,
        theme: &Theme,
    ) -> Option<AnyElement> {
        let Some(Detail::Installed { skill, .. }) = &self.detail else {
            return None;
        };
        let document = document.ready()?;
        // A lone SKILL.md is the section below; list files only when the
        // skill carries more.
        if document.files.len() < 2 {
            return None;
        }
        let last = document.files.len() - 1;
        let rows = document.files.iter().enumerate().map(|(index, file)| {
            let path = std::path::Path::new(&skill.directory).join(file);
            let key = format!("skills-file-{}-{file}", skill.directory);
            row_shell(index == 0, index == last, theme)
                .min_h(px(40.0))
                .py(px(8.0))
                .id(SharedString::from(key.clone()))
                .role(gpui::Role::Button)
                .aria_label(format!("Open {file}"))
                .tab_index(0)
                .cursor_pointer()
                .bg(row_fill(&key, theme))
                .on_hover(crate::motion::hover_listener(key.clone()))
                .child(
                    icon(file_icon(file))
                        .size(px(14.0))
                        .flex_none()
                        .text_color(theme.text_muted),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(crate::typography::ui_rems(12.5))
                        .text_color(theme.text)
                        .child(SharedString::from(file.clone())),
                )
                .child(
                    icon(icons::ARROW_UP_RIGHT)
                        .size(px(12.0))
                        .flex_none()
                        .text_color(
                            theme
                                .text_muted
                                .opacity(0.4 + 0.6 * crate::motion::hover_t(&key)),
                        ),
                )
                .tooltip(widgets::text_tooltip("Open in your default editor"))
                .on_click(move |_, _, cx| cx.open_with_system(&path))
                .into_any_element()
        });
        let label = if document.truncated {
            format!("Files · first {}", document.files.len())
        } else {
            format!("Files · {}", document.files.len())
        };
        Some(widgets::section(theme, label, card().children(rows)).into_any_element())
    }

    fn render_document(
        &self,
        document: &Loadable<Rc<Document>>,
        key: &str,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let frame = || {
            div()
                .w_full()
                .rounded(px(12.0))
                .bg(widgets::block_fill(theme))
                .px(px(20.0))
                .py(px(18.0))
        };
        match document {
            Loadable::Ready(document) if document.tree.blocks.is_empty() => frame()
                .text_size(crate::typography::ui_rems(13.0))
                .text_color(theme.text_muted)
                .child("This skill's SKILL.md has no instructions beyond its header.")
                .into_any_element(),
            Loadable::Ready(document) => {
                let options = self.markdown_options(key);
                crate::motion::fade_quick(
                    SharedString::from(format!("skills-document-{key}")),
                    frame().child(crate::markdown::render::render_tree(
                        &document.tree,
                        &options,
                        theme,
                        window,
                        &|_| None,
                    )),
                )
                .into_any_element()
            }
            Loadable::Error(error) => widgets::error_strip(theme, error.clone())
                .mt_0()
                .into_any_element(),
            Loadable::Idle | Loadable::Loading => frame()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .children((0..7).map(|index| {
                    crate::pull_request_skeleton::bar(
                        gpui::relative([0.42, 0.9, 0.84, 0.95, 0.6, 0.88, 0.7][index]),
                        if index == 0 { 14.0 } else { 10.0 },
                        index,
                        cx.entity_id(),
                        theme,
                        cx,
                    )
                    .when(index == 1, |el| el.mt(px(6.0)))
                }))
                .into_any_element(),
        }
    }

    /// Markdown on this page: selectable, with copyable code blocks.
    fn markdown_options(&self, key: &str) -> crate::markdown::render::RenderOptions {
        crate::markdown::render::RenderOptions {
            tasks: None,
            media: None,
            row_key: format!("{}{key}", self.selection_prefix).into(),
            veil: None,
            cache: None,
            now: Instant::now(),
            link: None,
            workspace_root: None,
            code: None,
            copy: Some(crate::markdown::render::CopyUi {
                handler: Rc::new(|_, code, _, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(code.to_string()))
                }),
                copied_ix: None,
            }),
        }
    }

    fn render_remove_dialog(
        &mut self,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let skill = self.removing.as_ref()?;
        let theme = Theme::of(cx).for_popup();
        let body = if skill.linked {
            format!(
                "This unlinks {} from ~/.zeron/skills. The folder it points to stays where it is.",
                skill.name
            )
        } else {
            format!(
                "This deletes {} and every file in it. New sessions stop loading the skill.",
                display_path(&skill.directory)
            )
        };
        let card = popover::dialog_card(&theme)
            .child(popover::dialog_title(
                &theme,
                &format!("Remove {}?", skill.name),
            ))
            .child(div().mt(px(6.0)).child(popover::dialog_body(&theme, body)))
            .child(
                div()
                    .mt(px(16.0))
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        popover::btn_ghost(&theme, "Cancel", "skills-remove-cancel")
                            .id("skills-remove-cancel")
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.removing = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        popover::btn_danger(&theme, "Remove")
                            .id("skills-remove-confirm")
                            .debug_selector(|| "skills-remove-confirm".into())
                            .on_click(cx.listener(|page, _, _, cx| page.confirm_remove(cx))),
                    ),
            )
            .into_any_element();
        Some(popover::modal("skills-remove-dialog", viewport, card))
    }
}

// ---- pieces ----

fn clone_loadable(loadable: &Loadable<Rc<Document>>) -> Loadable<Rc<Document>> {
    match loadable {
        Loadable::Idle => Loadable::Idle,
        Loadable::Loading => Loadable::Loading,
        Loadable::Ready(document) => Loadable::Ready(document.clone()),
        Loadable::Error(error) => Loadable::Error(error.clone()),
    }
}

fn card() -> gpui::Div {
    div().w_full().flex().flex_col()
}

/// One row of a board card: each row paints its slice of the card's fill.
fn row_shell(first: bool, last: bool, theme: &Theme) -> gpui::Div {
    div()
        .w_full()
        .bg(widgets::block_fill(theme))
        .when(first, |el| el.rounded_t(px(12.0)))
        .when(last, |el| el.rounded_b(px(12.0)))
        .px(px(16.0))
        .py(px(12.0))
        .min_h(px(ROW_HEIGHT))
        .when(!first, |el| {
            el.border_t_1().border_color(widgets::row_divider(theme))
        })
        .flex()
        .flex_row()
        .flex_nowrap()
        .items_center()
        .gap(px(Theme::SPACE_LG))
}

/// A card row's fill: the card's own wash, deepening under the pointer.
/// Rows paint the card, so hover can't simply replace the fill.
fn row_fill(key: &str, theme: &Theme) -> gpui::Hsla {
    crate::motion::hover_blend(key, widgets::block_fill(theme), theme.wash(0.085))
}

fn skill_tile(theme: &Theme) -> gpui::Div {
    widgets::row_tile(theme, icons::MAGIC_STICK_3)
}

/// Remove, shown but unavailable: a pack skill goes with its pack.
fn pack_owned_remove(id: String, pack: &str, theme: &Theme) -> AnyElement {
    div()
        .id(SharedString::from(id))
        .role(gpui::Role::Button)
        .aria_label(format!("Remove skill, unavailable: part of the {pack} pack"))
        .size(px(28.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .tooltip(widgets::text_tooltip(format!(
            "Part of the {pack} pack, which would bring it back on its next update. Remove the pack instead."
        )))
        .child(
            icon(icons::TRASH_BIN_MINIMALISTIC)
                .size(px(14.0))
                .text_color(theme.text_muted.opacity(0.35)),
        )
        .into_any_element()
}

fn header_icon_button(
    id: &'static str,
    label: &'static str,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(gpui::Role::Button)
        .aria_label(label)
        .tab_index(0)
        .size(px(HEADER_CONTROL_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .border_1()
        .border_color(gpui::transparent_black())
        .focus_visible(|style| style.border_color(theme.accent))
        .hover(|style| style.bg(crate::theme::ink(0.04)))
        .tooltip(widgets::text_tooltip(label))
}

/// An icon action in a row; `label` is its accessible name and its one tooltip.
fn row_icon_button(
    id: String,
    label: impl Into<SharedString>,
    glyph: &'static str,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let label: SharedString = label.into();
    div()
        .id(SharedString::from(id))
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .tab_index(0)
        .size(px(28.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .border_1()
        .border_color(gpui::transparent_black())
        .focus_visible(|style| style.border_color(theme.accent))
        .hover(|style| style.bg(theme.glass_hover()))
        .tooltip(widgets::text_tooltip(label))
        .child(icon(glyph).size(px(14.0)).text_color(theme.text_muted))
}

fn meta_text(text: impl Into<SharedString>, _theme: &Theme) -> AnyElement {
    div().truncate().child(text.into()).into_any_element()
}

fn empty_state(
    glyph: &'static str,
    title: &'static str,
    body: impl Into<SharedString>,
    theme: &Theme,
) -> gpui::Div {
    div()
        .mt(px(56.0))
        .flex_none()
        .flex()
        .flex_col()
        .items_center()
        .text_center()
        .child(
            div()
                .mb(px(Theme::SPACE_LG))
                .child(icon(glyph).size(px(24.0)).text_color(theme.text_muted)),
        )
        .child(
            div()
                .w(px(420.0))
                .max_w_full()
                .text_size(crate::typography::ui_rems(15.0))
                .line_height(crate::typography::ui_rems(22.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child(title),
        )
        .child(
            div()
                .mt(px(6.0))
                .w(px(420.0))
                .max_w_full()
                .text_size(crate::typography::ui_rems(13.0))
                .line_height(crate::typography::ui_rems(19.0))
                .text_color(theme.text_muted)
                .child(body.into()),
        )
}

fn empty_action(id: &'static str, label: &'static str, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    widgets::ghost_action(theme)
        .id(id)
        .debug_selector(move || id.into())
        .role(gpui::Role::Button)
        .tab_index(0)
        .mt(px(20.0))
        .flex_none()
        .min_h(px(32.0))
        .line_height(crate::typography::ui_rems(18.0))
        .px(px(14.0))
        .border_1()
        .border_color(theme.border)
        .child(label)
}

/// Rows shaped like the board's while the library or skills.sh answers.
fn skeleton(view: gpui::EntityId, theme: &Theme, cx: &mut gpui::App) -> AnyElement {
    const WIDTHS: [f32; 6] = [0.38, 0.52, 0.31, 0.46, 0.27, 0.42];
    card()
        .children((0..6).map(|index| {
            row_shell(index == 0, index == 5, theme)
                .child(crate::pull_request_skeleton::bar(
                    px(16.0),
                    16.0,
                    index,
                    view,
                    theme,
                    cx,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(crate::pull_request_skeleton::bar(
                            gpui::relative(WIDTHS[index]),
                            12.0,
                            index,
                            view,
                            theme,
                            cx,
                        ))
                        .child(crate::pull_request_skeleton::bar(
                            gpui::relative(WIDTHS[index] + 0.3),
                            10.0,
                            index,
                            view,
                            theme,
                            cx,
                        )),
                )
                .child(crate::pull_request_skeleton::bar(
                    px(64.0),
                    24.0,
                    index,
                    view,
                    theme,
                    cx,
                ))
        }))
        .into_any_element()
}

fn matches_query(fields: &[&str], query: &str) -> bool {
    let text = fields.join(" ").to_lowercase();
    query
        .split_whitespace()
        .all(|word| text.contains(&word.to_lowercase()))
}

fn single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `owner/repo` of a registry id `owner/repo/skill`.
fn registry_source(id: &str) -> String {
    id.rsplit_once('/')
        .map_or(id, |(source, _)| source)
        .to_owned()
}

fn installs_label(installs: u64) -> SharedString {
    match installs {
        1 => "1 install".into(),
        n => format!("{} installs", crate::pull_requests::format_compact_count(n)).into(),
    }
}

/// A path with the home directory written as `~`.
fn display_path(path: &str) -> String {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return path.to_owned();
    };
    match std::path::Path::new(path).strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.to_owned(),
    }
}

fn file_icon(file: &str) -> &'static str {
    match std::path::Path::new(file)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("md" | "mdx" | "markdown" | "txt") => icons::FILE_MARKDOWN,
        Some("png" | "jpg" | "jpeg" | "gif" | "svg" | "webp") => icons::FILE_IMAGE,
        Some("json" | "yaml" | "yml" | "toml" | "csv" | "xml") => icons::FILE_DATA,
        _ => icons::FILE_CODE,
    }
}

fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Show in Finder"
    } else if cfg!(windows) {
        "Show in File Explorer"
    } else {
        "Show in file manager"
    }
}

fn rpc_error_copy(error: &zeron_rpc::RpcError) -> String {
    match error {
        zeron_rpc::RpcError::UnknownMethod(_) => {
            "This engine is older than the app. Restart Zeron to finish updating.".into()
        }
        zeron_rpc::RpcError::Failed(message) => message.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_words_all_have_to_match() {
        assert!(matches_query(&["pdf", "Fill PDF forms"], "fill pdf"));
        assert!(matches_query(&["pdf", "Fill PDF forms"], ""));
        assert!(!matches_query(&["pdf", "Fill PDF forms"], "docx"));
    }

    #[test]
    fn registry_labels() {
        assert_eq!(
            registry_source("anthropics/skills/pdf"),
            "anthropics/skills"
        );
        assert_eq!(installs_label(1).as_ref(), "1 install");
        assert_eq!(installs_label(211_404).as_ref(), "211.4k installs");
    }

    #[test]
    fn files_get_a_glyph_by_kind() {
        assert_eq!(file_icon("SKILL.md"), icons::FILE_MARKDOWN);
        assert_eq!(file_icon("scripts/fill.py"), icons::FILE_CODE);
        assert_eq!(file_icon("schema.JSON"), icons::FILE_DATA);
    }
}
