//! Skill packs on the Installed tab: a card per pack (source, version, its
//! skills, the update waiting and what it changes), and the Add pack dialog
//! that inspects a repository before anything is linked.
use std::{collections::HashMap, rc::Rc};

use chrono::Utc;
use gpui::{
    AnyElement, Context, Entity, Focusable, IntoElement, SharedString, Subscription, Task, Window,
    div, prelude::*, px,
};
use zeron_engine::skill_library::{Library, LibrarySkill};
use zeron_engine::skill_packs::{Channel, PackInspection, SkillPack};
use zeron_rpc::methods;

use super::{SkillsPage, card, row_fill, row_shell};
use crate::composer::{ComposerInput, ComposerInputEvent};
use crate::icons::{self, icon};
use crate::popover;
use crate::settings::widgets;
use crate::theme::Theme;

/// What a pack's controls are waiting on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PackBusy {
    Checking,
    Updating,
    Toggling,
    Removing,
}

pub(super) enum AddStage {
    Editing,
    Inspecting,
    Ready(PackInspection),
    Installing(PackInspection),
}

pub(super) struct AddPack {
    source: Entity<ComposerInput>,
    subpath: Entity<ComposerInput>,
    stage: AddStage,
    error: Option<String>,
    auto_update: bool,
    task: Option<Task<()>>,
    _events: Vec<Subscription>,
}

impl SkillsPage {
    pub(super) fn open_add_pack(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = |placeholder: &'static str, cx: &mut Context<Self>| {
            cx.new(|cx| {
                ComposerInput::with_context(placeholder, "PaletteSearch", cx)
                    .with_text_metrics(13.0, 18.0)
                    .with_single_line()
                    .with_tab_stop()
            })
        };
        let source = input("owner/repo or https://…", cx);
        let subpath = input("Optional", cx);
        let events = [&source, &subpath]
            .into_iter()
            .map(|field| {
                cx.subscribe(field, |page: &mut Self, _, event, cx| match event {
                    // A changed source invalidates what was inspected.
                    ComposerInputEvent::Edited => {
                        if let Some(add) = &mut page.add_pack
                            && matches!(add.stage, AddStage::Ready(_))
                        {
                            add.stage = AddStage::Editing;
                            add.error = None;
                            cx.notify();
                        }
                    }
                    ComposerInputEvent::Submitted => page.advance_add_pack(cx),
                    _ => {}
                })
            })
            .collect();
        window.focus(&source.focus_handle(cx), cx);
        self.add_pack = Some(AddPack {
            source,
            subpath,
            stage: AddStage::Editing,
            error: None,
            auto_update: true,
            task: None,
            _events: events,
        });
        cx.notify();
    }

    #[cfg(feature = "appshots-fixture")]
    pub(super) fn fixture_add_pack_source(&mut self, source: &str, cx: &mut Context<Self>) {
        if let Some(add) = &self.add_pack {
            add.source
                .update(cx, |input, cx| input.set_text(source, cx));
        }
        self.inspect_pack(cx);
    }

    /// Enter or the primary button: inspect, then install.
    fn advance_add_pack(&mut self, cx: &mut Context<Self>) {
        let Some(add) = &self.add_pack else {
            return;
        };
        match &add.stage {
            AddStage::Editing => self.inspect_pack(cx),
            AddStage::Ready(inspection) => {
                let inspection = inspection.clone();
                self.install_pack(inspection, cx)
            }
            AddStage::Inspecting | AddStage::Installing(_) => {}
        }
    }

    fn inspect_pack(&mut self, cx: &mut Context<Self>) {
        let Some(add) = &mut self.add_pack else {
            return;
        };
        let source = add.source.read(cx).text().trim().to_string();
        if source.is_empty() {
            add.error = Some("Enter a repository, like owner/repo.".into());
            cx.notify();
            return;
        }
        let subpath = add.subpath.read(cx).text().trim().to_string();
        add.stage = AddStage::Inspecting;
        add.error = None;
        let task = self.call(
            methods::INSPECT_SKILL_PACK,
            serde_json::json!({
                "source": source,
                "subpath": (!subpath.is_empty()).then_some(subpath),
            }),
            cx,
            |page, result: Result<PackInspection, String>, _| {
                if let Some(add) = &mut page.add_pack {
                    add.task = None;
                    match result {
                        Ok(inspection) => {
                            // Without releases there is only a moving branch;
                            // following it is opt-in.
                            add.auto_update = !unversioned(&inspection);
                            add.stage = AddStage::Ready(inspection)
                        }
                        Err(error) => {
                            add.stage = AddStage::Editing;
                            add.error = Some(error);
                        }
                    }
                }
            },
        );
        if let Some(add) = &mut self.add_pack {
            add.task = task;
        }
        cx.notify();
    }

    fn install_pack(&mut self, inspection: PackInspection, cx: &mut Context<Self>) {
        let Some(add) = &mut self.add_pack else {
            return;
        };
        let auto_update = add.auto_update;
        add.stage = AddStage::Installing(inspection.clone());
        add.error = None;
        let task = self.call(
            methods::INSTALL_SKILL_PACK,
            serde_json::json!({ "token": inspection.token, "autoUpdate": auto_update }),
            cx,
            move |page, result: Result<Library, String>, cx| match result {
                Ok(library) => {
                    page.add_pack = None;
                    let linked = inspection.skills.len() - inspection.clashes.len();
                    crate::toast::success(
                        cx,
                        format!(
                            "Installed {}: {linked} {}. New sessions can use them.",
                            inspection.name,
                            if linked == 1 { "skill" } else { "skills" }
                        ),
                    );
                    page.expanded_packs.insert(inspection.name.clone());
                    page.set_library(library, cx);
                }
                Err(error) => {
                    if let Some(add) = &mut page.add_pack {
                        add.task = None;
                        // An expired token needs a fresh inspection.
                        add.stage = AddStage::Editing;
                        add.error = Some(error);
                    }
                }
            },
        );
        if let Some(add) = &mut self.add_pack {
            add.task = task;
        }
        cx.notify();
    }

    /// A pack mutation whose reply is the fresh library.
    pub(super) fn pack_action(
        &mut self,
        name: String,
        busy: PackBusy,
        method: &'static str,
        params: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        if self.pack_busy.contains_key(&name) {
            return;
        }
        let key = name.clone();
        let task = self.call(
            method,
            params,
            cx,
            move |page, result: Result<Library, String>, cx| {
                let label = page.pack_busy.remove(&name).map(|(busy, _)| busy);
                match result {
                    Ok(library) => {
                        let pack = library.packs.iter().find(|pack| pack.name == name).cloned();
                        match (label, pack) {
                            (Some(PackBusy::Updating), Some(pack)) => crate::toast::success(
                                cx,
                                format!(
                                    "Updated {} to {}.",
                                    pack.name,
                                    pack.version.as_deref().unwrap_or(&pack.reference)
                                ),
                            ),
                            (Some(PackBusy::Checking), Some(pack)) if pack.update.is_none() => {
                                crate::toast::info(cx, format!("{} is up to date.", pack.name))
                            }
                            (Some(PackBusy::Removing), _) => {
                                crate::toast::success(cx, format!("Removed the {name} pack."))
                            }
                            _ => {}
                        }
                        page.set_library(library, cx);
                    }
                    Err(error) => crate::toast::error(cx, error),
                }
            },
        );
        if let Some(task) = task {
            self.pack_busy.insert(key, (busy, task));
        }
        cx.notify();
    }

    /// Packs not checked for a while are checked when the page opens, so a
    /// pack without auto-update still shows the update waiting for it.
    pub(super) fn check_stale_packs(&mut self, cx: &mut Context<Self>) {
        let Some(library) = self.library.ready() else {
            return;
        };
        let stale = library.packs.iter().any(|pack| {
            pack.checked_at
                .is_none_or(|at| Utc::now() - at > chrono::Duration::hours(6))
        });
        if stale && !self.checked_packs {
            self.checked_packs = true;
            let check = self.call(
                methods::CHECK_SKILL_PACK_UPDATES,
                serde_json::json!({}),
                cx,
                |page, result: Result<Library, String>, cx| {
                    // Quiet: a failed background check shows on its pack.
                    if let Ok(library) = result {
                        page.set_library(library, cx);
                    }
                },
            );
            if let Some(check) = check {
                check.detach();
            }
        }
    }

    // ---- the cards ----

    /// The Packs section, or `None` when no pack is installed or none matches.
    pub(super) fn render_packs(
        &mut self,
        library: &Library,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let query = self.query.trim().to_string();
        let mut members: HashMap<&str, Vec<LibrarySkill>> = HashMap::new();
        for skill in &library.skills {
            if let Some(pack) = &skill.pack {
                members
                    .entry(pack.as_str())
                    .or_default()
                    .push(skill.clone());
            }
        }
        let cards: Vec<AnyElement> = library
            .packs
            .iter()
            .filter_map(|pack| {
                let all = members.remove(pack.name.as_str()).unwrap_or_default();
                let named = super::matches_query(&[&pack.name, &pack.source], &query);
                let shown: Vec<LibrarySkill> = if named {
                    all
                } else {
                    all.into_iter()
                        .filter(|skill| {
                            super::matches_query(&[&skill.name, &skill.description], &query)
                        })
                        .collect()
                };
                (named || !shown.is_empty()).then(|| {
                    self.render_pack(pack, shown, !query.is_empty() && !named, theme, window, cx)
                })
            })
            .collect();
        if cards.is_empty() {
            return None;
        }
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(widgets::section_label(theme, "Packs"))
                .child(div().flex().flex_col().gap(px(12.0)).children(cards))
                .into_any_element(),
        )
    }

    fn render_pack(
        &mut self,
        pack: &SkillPack,
        skills: Vec<LibrarySkill>,
        searching: bool,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = pack.name.clone();
        let expanded = searching || self.expanded_packs.contains(&name);
        let reveal = crate::motion::state_t(
            &format!("skills-pack-{}-{name}", cx.entity_id()),
            expanded,
            crate::motion::GLYPH_STATE,
            crate::motion::reduced_motion(cx),
        );
        let busy = self.pack_busy.get(&name).map(|(busy, _)| *busy);
        let key = format!("skills-pack-row-{}-{name}", cx.entity_id());
        let version = pack
            .version
            .clone()
            .unwrap_or_else(|| pack.reference.clone());
        let meta: Vec<SharedString> = vec![
            pack.source.clone().into(),
            version_label(&version).into(),
            match pack.skills.len() {
                1 => "1 skill".into(),
                n => format!("{n} skills").into(),
            },
        ];
        let changes_key = pack
            .update
            .as_ref()
            .map(|update| format!("{name}@{}", update.commit));
        let changes_open = changes_key
            .as_ref()
            .is_some_and(|key| self.changes_open.contains(key));
        let has_notes = pack.update.as_ref().is_some_and(|u| u.changes.is_some());
        let extras = has_notes || !pack.skipped.is_empty() || pack.last_error.is_some();

        let header = row_shell(true, !expanded && !extras, theme)
            .id(SharedString::from(key.clone()))
            .debug_selector(|| "skills-pack-row".into())
            .role(gpui::Role::Button)
            .aria_label(format!(
                "{name} pack, {}. Show its skills",
                pack.skills.len()
            ))
            .aria_expanded(expanded)
            .tab_index(0)
            .bg(row_fill(&key, theme))
            .on_hover(crate::motion::hover_listener(key.clone()))
            .cursor_pointer()
            .on_click(cx.listener({
                let name = name.clone();
                move |page, _, _, cx| {
                    if !page.expanded_packs.remove(&name) {
                        page.expanded_packs.insert(name.clone());
                    }
                    cx.notify();
                }
            }))
            .child(
                div()
                    .flex_none()
                    .w(px(24.0))
                    .h(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        icon(icons::ALT_ARROW_RIGHT)
                            .size(px(14.0))
                            .text_color(theme.text_muted)
                            .with_transformation(gpui::Transformation::rotate(gpui::percentage(
                                reveal * 0.25,
                            ))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(widgets::row_title(theme, name.clone()).truncate())
                            .when_some(pack.update.as_ref(), |el, update| {
                                el.child(update_badge(theme, update_label(update)))
                            }),
                    )
                    .child(widgets::meta_line(
                        theme,
                        meta.into_iter()
                            .map(|text| div().truncate().child(text).into_any_element())
                            .collect(),
                    )),
            )
            .child(self.render_pack_controls(pack, busy, theme, cx));

        let mut rows: Vec<AnyElement> = vec![header.into_any_element()];
        let extra_count = usize::from(has_notes)
            + usize::from(!pack.skipped.is_empty())
            + usize::from(pack.last_error.is_some());
        let mut extra_ix = 0;
        let mut last_extra = || {
            extra_ix += 1;
            extra_ix == extra_count && !expanded
        };
        if let (Some(update), Some(changes_key)) = (&pack.update, &changes_key)
            && let Some(changes) = &update.changes
        {
            let last = last_extra();
            let toggle_key = format!("skills-pack-notes-{}-{name}", cx.entity_id());
            let notes_reveal = crate::motion::state_t(
                &toggle_key,
                changes_open,
                crate::motion::GLYPH_STATE,
                crate::motion::reduced_motion(cx),
            );
            let tree = self
                .changes_trees
                .entry(changes_key.clone())
                .or_insert_with(|| Rc::new(crate::markdown::parse_full(changes)))
                .clone();
            let mut notes = row_shell(false, last, theme)
                .min_h(px(44.0))
                .py(px(10.0))
                .flex_col()
                .items_stretch()
                .gap(px(10.0))
                .child(
                    div()
                        .id(SharedString::from(toggle_key.clone()))
                        .role(gpui::Role::Button)
                        .aria_expanded(changes_open)
                        .tab_index(0)
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .cursor_pointer()
                        .text_size(crate::typography::ui_rems(12.5))
                        .text_color(crate::motion::hover_blend(
                            &toggle_key,
                            theme.text_muted,
                            theme.text,
                        ))
                        .on_hover(crate::motion::hover_listener(toggle_key.clone()))
                        .child(
                            icon(icons::ALT_ARROW_RIGHT)
                                .size(px(12.0))
                                .text_color(theme.text_muted)
                                .with_transformation(gpui::Transformation::rotate(
                                    gpui::percentage(notes_reveal * 0.25),
                                )),
                        )
                        .child(SharedString::from(format!(
                            "What's new in {}",
                            update_label(update)
                        )))
                        .on_click(cx.listener({
                            let changes_key = changes_key.clone();
                            move |page, _, _, cx| {
                                if !page.changes_open.remove(&changes_key) {
                                    page.changes_open.insert(changes_key.clone());
                                }
                                cx.notify();
                            }
                        })),
                );
            if changes_open {
                let options = self.markdown_options(&format!("pack-notes-{changes_key}"));
                notes = notes.child(crate::motion::fade_quick(
                    SharedString::from(format!("skills-pack-notes-body-{changes_key}")),
                    div()
                        .pl(px(18.0))
                        .text_size(crate::typography::ui_rems(13.0))
                        .child(crate::markdown::render::render_tree(
                            &tree,
                            &options,
                            theme,
                            window,
                            &|_| None,
                        )),
                ));
            }
            rows.push(notes.into_any_element());
        }
        if !pack.skipped.is_empty() {
            let last = last_extra();
            rows.push(
                row_shell(false, last, theme)
                    .min_h(px(40.0))
                    .py(px(8.0))
                    .gap(px(8.0))
                    .items_start()
                    .child(
                        icon(icons::INFO_CIRCLE)
                            .size(px(14.0))
                            .mt(px(1.0))
                            .flex_none()
                            .text_color(theme.warning),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(crate::typography::ui_rems(12.0))
                            .line_height(crate::typography::ui_rems(17.0))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(skipped_copy(&pack.skipped))),
                    )
                    .into_any_element(),
            );
        }
        if let Some(error) = &pack.last_error {
            let last = last_extra();
            rows.push(
                row_shell(false, last, theme)
                    .min_h(px(40.0))
                    .py(px(8.0))
                    .gap(px(8.0))
                    .items_start()
                    .child(
                        icon(icons::DANGER_TRIANGLE)
                            .size(px(14.0))
                            .mt(px(1.0))
                            .flex_none()
                            .text_color(theme.danger),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(crate::typography::ui_rems(12.0))
                            .line_height(crate::typography::ui_rems(17.0))
                            .text_color(theme.danger_muted)
                            .child(SharedString::from(format!("Last update failed: {error}"))),
                    )
                    .into_any_element(),
            );
        }
        if expanded {
            let last = skills.len().saturating_sub(1);
            for (index, skill) in skills.into_iter().enumerate() {
                rows.push(
                    div()
                        .pl(px(24.0))
                        .bg(widgets::block_fill(theme))
                        .when(index == last, |el| el.rounded_b(px(12.0)))
                        .child(self.render_installed_row(skill, false, index == last, theme, cx))
                        .into_any_element(),
                );
            }
        }
        crate::motion::fade_in(
            SharedString::from(format!("skills-pack-{name}-{}", self.generation)),
            card().children(rows),
        )
        .into_any_element()
    }

    fn render_pack_controls(
        &mut self,
        pack: &SkillPack,
        busy: Option<PackBusy>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = pack.name.clone();
        let row = div().flex_none().flex().items_center().gap(px(4.0));
        if let Some(busy) = busy.filter(|busy| *busy != PackBusy::Toggling) {
            return row
                .min_h(px(32.0))
                .px(px(10.0))
                .gap(px(8.0))
                .text_size(crate::typography::ui_rems(12.5))
                .text_color(theme.text_muted)
                .child(crate::loaders::mini_glyph_spinner(
                    format!("skills-pack-spinner-{name}"),
                    1.5,
                    theme.glyph,
                    cx.entity_id(),
                    cx,
                ))
                .child(match busy {
                    PackBusy::Checking => "Checking",
                    PackBusy::Updating => "Updating",
                    _ => "Removing",
                })
                .into_any_element();
        }
        let auto = pack.auto_update;
        let accent = theme.accent;
        row.when_some(pack.update.as_ref(), |row, update| {
            let name = name.clone();
            row.child(
                widgets::action_button(theme, widgets::ActionTone::Solid)
                    .id(SharedString::from(format!("skills-pack-update-{name}")))
                    .debug_selector(|| "skills-pack-update".into())
                    .role(gpui::Role::Button)
                    .tab_index(0)
                    .child(SharedString::from(format!(
                        "Update to {}",
                        update_label(update)
                    )))
                    .on_click(cx.listener(move |page, _, _, cx| {
                        cx.stop_propagation();
                        page.pack_action(
                            name.clone(),
                            PackBusy::Updating,
                            methods::UPDATE_SKILL_PACK,
                            serde_json::json!({ "name": name }),
                            cx,
                        );
                    })),
            )
        })
        .when(pack.update.is_none(), |row| {
            let name = name.clone();
            let checked = match pack.checked_at {
                Some(at) => format!(
                    "Check for updates · last checked {}",
                    crate::pull_requests::relative_time(at, Utc::now())
                ),
                None => "Check for updates".into(),
            };
            row.child(
                super::row_icon_button(
                    format!("skills-pack-check-{name}"),
                    "Check for updates",
                    icons::REFRESH,
                    theme,
                )
                .tooltip(widgets::text_tooltip(checked))
                .on_click(cx.listener(move |page, _, _, cx| {
                    cx.stop_propagation();
                    page.pack_action(
                        name.clone(),
                        PackBusy::Checking,
                        methods::CHECK_SKILL_PACK_UPDATES,
                        serde_json::json!({ "name": name }),
                        cx,
                    );
                })),
            )
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .pl(px(6.0))
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .child("Auto-update"),
                )
                .child(
                    widgets::toggle_switch(theme, auto, format!("skills-pack-auto-{name}"))
                        .id(SharedString::from(format!("skills-pack-auto-{name}")))
                        .role(gpui::Role::Switch)
                        .aria_label(format!("Update {name} automatically"))
                        .aria_toggled(if auto {
                            gpui::Toggled::True
                        } else {
                            gpui::Toggled::False
                        })
                        .tab_index(0)
                        .focus_visible(move |style| style.border_2().border_color(accent))
                        .cursor_pointer()
                        .tooltip(widgets::text_tooltip(
                            "Check every six hours and install new releases",
                        ))
                        .on_click(cx.listener({
                            let name = name.clone();
                            move |page, _, _, cx| {
                                cx.stop_propagation();
                                // Flip it now; the engine's reply settles it.
                                if let crate::popover::Loadable::Ready(library) = &mut page.library
                                    && let Some(pack) =
                                        library.packs.iter_mut().find(|pack| pack.name == name)
                                {
                                    pack.auto_update = !auto;
                                }
                                page.pack_action(
                                    name.clone(),
                                    PackBusy::Toggling,
                                    methods::SET_SKILL_PACK_AUTO_UPDATE,
                                    serde_json::json!({ "name": name, "enabled": !auto }),
                                    cx,
                                );
                            }
                        })),
                ),
        )
        .child(
            super::row_icon_button(
                format!("skills-pack-remove-{name}"),
                "Remove pack",
                icons::TRASH_BIN_MINIMALISTIC,
                theme,
            )
            .on_click(cx.listener({
                let pack = pack.clone();
                move |page, _, _, cx| {
                    cx.stop_propagation();
                    page.removing_pack = Some(pack.clone());
                    cx.notify();
                }
            })),
        )
        .into_any_element()
    }

    // ---- dialogs ----

    pub(super) fn render_remove_pack_dialog(
        &mut self,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let pack = self.removing_pack.as_ref()?;
        let theme = Theme::of(cx).for_popup();
        let name = pack.name.clone();
        let body = format!(
            "This unlinks its {} and deletes {}. Skills of your own are left alone.",
            match pack.skills.len() {
                1 => "skill".to_string(),
                n => format!("{n} skills"),
            },
            super::display_path(&pack.directory)
        );
        let card = popover::dialog_card(&theme)
            .child(popover::dialog_title(
                &theme,
                &format!("Remove the {name} pack?"),
            ))
            .child(div().mt(px(6.0)).child(popover::dialog_body(&theme, body)))
            .child(
                div()
                    .mt(px(16.0))
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        popover::btn_ghost(&theme, "Cancel", "skills-remove-pack-cancel")
                            .id("skills-remove-pack-cancel")
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.removing_pack = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        popover::btn_danger(&theme, "Remove pack")
                            .id("skills-remove-pack-confirm")
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.removing_pack = None;
                                page.pack_action(
                                    name.clone(),
                                    PackBusy::Removing,
                                    methods::REMOVE_SKILL_PACK,
                                    serde_json::json!({ "name": name }),
                                    cx,
                                );
                            })),
                    ),
            )
            .into_any_element();
        Some(popover::modal("skills-remove-pack-dialog", viewport, card))
    }

    pub(super) fn render_add_pack_dialog(
        &mut self,
        viewport: gpui::Size<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let add = self.add_pack.as_ref()?;
        let theme = Theme::of(cx).for_popup();
        let working = matches!(add.stage, AddStage::Inspecting | AddStage::Installing(_));
        let field = |label: &'static str, input: Entity<ComposerInput>| {
            div()
                .mt(px(14.0))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(12.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.text_muted)
                        .child(label),
                )
                .child(
                    crate::surface_chrome::input()
                        .h(px(34.0))
                        .px(px(10.0))
                        .rounded(px(8.0))
                        .when(working, |el| el.opacity(0.6))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h(px(18.0))
                                .overflow_hidden()
                                .child(input),
                        ),
                )
        };
        let summary = match &add.stage {
            AddStage::Ready(inspection) | AddStage::Installing(inspection) => {
                Some(inspection_summary(inspection, add.auto_update, &theme, cx))
            }
            _ => None,
        };
        let status = match &add.stage {
            AddStage::Inspecting => Some("Cloning and reading the pack…"),
            AddStage::Installing(_) => Some("Linking its skills…"),
            _ => None,
        };
        let primary_label: SharedString = match &add.stage {
            AddStage::Editing | AddStage::Inspecting => "Inspect".into(),
            AddStage::Ready(inspection) | AddStage::Installing(inspection) => {
                match inspection.skills.len() - inspection.clashes.len() {
                    1 => "Install 1 skill".into(),
                    n => format!("Install {n} skills").into(),
                }
            }
        };
        let error = add.error.clone();
        let card = popover::dialog_card(&theme)
            .w(px(460.0))
            .child(popover::dialog_title(&theme, "Add a skill pack"))
            .child(div().mt(px(6.0)).child(popover::dialog_body(
                &theme,
                "A git repository of skills. Zeron clones it into ~/.zeron/packs, links its skills for every provider, and never runs anything from it.",
            )))
            .child(field("Repository", add.source.clone()))
            .child(field("Folder in the repository", add.subpath.clone()))
            .children(summary)
            .when_some(error, |el, error| {
                el.child(widgets::error_strip(&theme, error).mt(px(14.0)))
            })
            .child(
                div()
                    .mt(px(18.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .when_some(status, |el, status| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .text_size(crate::typography::ui_rems(12.0))
                                .text_color(theme.text_muted)
                                .child(crate::loaders::mini_glyph_spinner(
                                    "skills-add-pack-spinner",
                                    1.5,
                                    theme.glyph,
                                    cx.entity_id(),
                                    cx,
                                ))
                                .child(status),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        popover::btn_ghost(&theme, "Cancel", "skills-add-pack-cancel")
                            .id("skills-add-pack-cancel")
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.add_pack = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        widgets::action_button(&theme, widgets::ActionTone::Solid)
                            .id("skills-add-pack-primary")
                            .debug_selector(|| "skills-add-pack-primary".into())
                            .role(gpui::Role::Button)
                            .tab_index(0)
                            .when(working, |el| el.opacity(0.5).cursor_default())
                            .child(primary_label)
                            .on_click(cx.listener(|page, _, _, cx| page.advance_add_pack(cx))),
                    ),
            )
            .into_any_element();
        Some(popover::modal("skills-add-pack-dialog", viewport, card))
    }
}

fn inspection_summary(
    inspection: &PackInspection,
    auto_update: bool,
    theme: &Theme,
    cx: &mut Context<SkillsPage>,
) -> AnyElement {
    const PREVIEW: usize = 14;
    let version = inspection
        .version
        .clone()
        .unwrap_or_else(|| inspection.reference.clone());
    let follows = match inspection.channel {
        Channel::Tags => "Follows release tags".to_string(),
        Channel::Version if inspection.version.is_some() => {
            format!("Follows version bumps on {}", inspection.reference)
        }
        Channel::Version => format!(
            "No releases; follows new commits on {}",
            inspection.reference
        ),
    };
    let shown: Vec<&String> = inspection.skills.iter().take(PREVIEW).collect();
    let more = inspection.skills.len().saturating_sub(PREVIEW);
    let accent = theme.accent;
    crate::motion::fade_quick(
        SharedString::from(format!("skills-inspection-{}", inspection.token)),
        div()
            .mt(px(16.0))
            .p(px(14.0))
            .rounded(px(10.0))
            .bg(theme.wash(0.05))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        icon(icons::FOLDER_WITH_FILES)
                            .size(px(16.0))
                            .text_color(theme.text),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(widgets::row_title(theme, inspection.name.clone()))
                            .child(
                                div()
                                    .text_size(crate::typography::ui_rems(12.0))
                                    .text_color(theme.text_muted)
                                    .child(SharedString::from(format!(
                                        "{} · {} skills · {follows}",
                                        version_label(&version),
                                        inspection.skills.len()
                                    ))),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(4.0))
                    .children(shown.into_iter().map(|skill| {
                        let clash = inspection.clashes.contains(skill);
                        div()
                            .px(px(7.0))
                            .py(px(2.0))
                            .rounded(px(6.0))
                            .bg(if clash {
                                theme.warning.opacity(0.12)
                            } else {
                                theme.wash(0.07)
                            })
                            .text_size(crate::typography::ui_rems(11.0))
                            .text_color(if clash {
                                theme.warning_muted
                            } else {
                                theme.text_muted
                            })
                            .when(clash, |el| el.line_through())
                            .child(SharedString::from(skill.clone()))
                    }))
                    .when(more > 0, |el| {
                        el.child(
                            div()
                                .px(px(7.0))
                                .py(px(2.0))
                                .text_size(crate::typography::ui_rems(11.0))
                                .text_color(theme.text_muted)
                                .child(SharedString::from(format!("+{more} more"))),
                        )
                    }),
            )
            .when(!inspection.clashes.is_empty(), |el| {
                el.child(
                    widgets::warning_strip(theme, clash_copy(&inspection.clashes))
                        .mt_0()
                        .py(px(8.0)),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(crate::typography::ui_rems(12.5))
                                    .text_color(theme.text)
                                    .child("Keep it up to date"),
                            )
                            .child(
                                div()
                                    .text_size(crate::typography::ui_rems(11.5))
                                    .text_color(theme.text_muted)
                                    .child("Checks every six hours and installs new releases."),
                            ),
                    )
                    .child(
                        widgets::toggle_switch(theme, auto_update, "skills-add-pack-auto")
                            .id("skills-add-pack-auto")
                            .role(gpui::Role::Switch)
                            .aria_label("Keep the pack up to date")
                            .aria_toggled(if auto_update {
                                gpui::Toggled::True
                            } else {
                                gpui::Toggled::False
                            })
                            .tab_index(0)
                            .cursor_pointer()
                            .focus_visible(move |style| style.border_2().border_color(accent))
                            .on_click(cx.listener(|page, _, _, cx| {
                                if let Some(add) = &mut page.add_pack {
                                    add.auto_update = !add.auto_update;
                                }
                                cx.notify();
                            })),
                    ),
            ),
    )
    .into_any_element()
}

/// A pack with neither release tags nor a declared version.
fn unversioned(inspection: &PackInspection) -> bool {
    inspection.channel == Channel::Version && inspection.version.is_none()
}

fn update_label(update: &zeron_engine::skill_packs::PackUpdate) -> String {
    match &update.version {
        Some(version) => version_label(version),
        None => format!(
            "{} {}",
            "commit",
            &update.commit[..update.commit.len().min(7)]
        ),
    }
}

fn update_badge(theme: &Theme, version: String) -> AnyElement {
    div()
        .flex_none()
        .px(px(7.0))
        .py(px(1.0))
        .rounded_full()
        .bg(theme.accent.opacity(0.14))
        .text_size(crate::typography::ui_rems(10.5))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(theme.accent)
        .child(SharedString::from(format!("{version} available")))
        .into_any_element()
}

/// `0.9.86` → `v0.9.86`; tags and other labels are shown as they are.
fn version_label(version: &str) -> String {
    if version.starts_with(|c: char| c.is_ascii_digit()) {
        format!("v{version}")
    } else {
        version.to_owned()
    }
}

fn names(list: &[String]) -> String {
    match list {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

fn clash_copy(clashes: &[String]) -> String {
    match clashes.len() {
        1 => format!(
            "You already have a skill named {}, so the pack's won't be linked. Yours stays as it is.",
            clashes[0]
        ),
        _ => format!(
            "You already have skills named {}, so the pack's won't be linked. Yours stay as they are.",
            names(clashes)
        ),
    }
}

fn skipped_copy(skipped: &[String]) -> String {
    match skipped.len() {
        1 => format!(
            "{} isn't linked: you already have a skill by that name.",
            skipped[0]
        ),
        _ => format!(
            "{} aren't linked: you already have skills by those names.",
            names(skipped)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_names_lists_and_versions() {
        assert_eq!(names(&["how".into()]), "how");
        assert_eq!(
            names(&["how".into(), "tdd".into(), "why".into()]),
            "how, tdd and why"
        );
        assert_eq!(version_label("0.9.86"), "v0.9.86");
        assert_eq!(version_label("v0.9.86"), "v0.9.86");
        assert_eq!(version_label("main"), "main");
        assert!(skipped_copy(&["how".into()]).starts_with("how isn't linked"));
    }
}
