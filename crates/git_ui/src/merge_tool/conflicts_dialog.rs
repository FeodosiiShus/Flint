pub(crate) mod actions;
mod commit_details;
mod flows;
mod interaction;
mod menus;
pub(crate) mod messages;
pub(crate) mod model_loading;
mod ordering;
mod overlay;
pub(crate) mod pane_titles;
mod persistence;
pub(crate) mod request;
mod rich_label;
mod state;
#[cfg(test)]
pub(crate) mod test_driver;
#[cfg(test)]
mod tests;
mod tree_table;

use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use git::repository::RepoPath;
use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Context, Entity, FocusHandle, Focusable,
    Render, Task, UniformListScrollHandle, Window, px, size,
};
use project::{Project, git_store::Repository};
use ui::{CommonAnimationExt as _, Tooltip, prelude::*};
use util::ResultExt as _;

use self::menus::RowContextMenu;
use self::overlay::Overlay;
use self::rich_label::rich_label;
use self::state::{DefaultButton, DialogState, RowId};
use crate::merge_tool::conflict_resolution::dialog_texts::DialogTexts;
use crate::merge_tool::conflict_resolution::rich_text::RichText;
use crate::merge_tool::dialog_window::{
    CloseDecision, CloseRequest, DialogOwner, DialogWindow, DialogWindowContent, DialogWindowSpec,
    open_dialog_window,
};
use crate::merge_tool::merge_model::iterative_data_holder::MergeConflictIterativeDataHolder;
use crate::merge_tool::merge_window::{dialog_button, dialog_button_with_content};

pub(crate) use self::request::{ConflictsDialogRequest, ConflictsDialogResult};

const DIALOG_WIDTH: f32 = 874.;
const DIALOG_HEIGHT_FEW_FILES: f32 = 505.;
const DIALOG_HEIGHT_MANY_FILES: f32 = 605.;
const DIALOG_MIN_WIDTH: f32 = 574.;
const DIALOG_MIN_HEIGHT: f32 = 312.;
const FEW_FILES_LIMIT: usize = 6;
const BOUNDS_KEY: &str = "MultipleFileMergeDialog";
const SOUTH_BUTTON_GAP: f32 = 12.;
const SOUTH_TOP_GAP: f32 = 32.;
const CONTROLS_GAP: f32 = 16.;
const RESOLVE_ICON_TEXT_GAP: f32 = 4.;

enum Description {
    Loading,
    Loaded(RichText),
}

type SharedOnClosed = Rc<RefCell<Option<Box<dyn FnOnce(ConflictsDialogResult, &mut App)>>>>;

pub(crate) struct ConflictsDialog {
    window_handle: AnyWindowHandle,
    project: Entity<Project>,
    repository: Entity<Repository>,
    holder: Entity<MergeConflictIterativeDataHolder>,
    state: DialogState,
    texts: DialogTexts,
    description: Description,
    focus_handle: FocusHandle,
    table_scroll: UniformListScrollHandle,
    hovered_row: Option<RowId>,
    speed_search: Option<String>,
    auto_resolve_status: Option<String>,
    overlay: Option<Overlay>,
    context_menu: Option<RowContextMenu>,
    grouping_key: String,
    should_finish_merge: bool,
    closing: bool,
    on_closed: SharedOnClosed,
    _description_task: Task<()>,
}

fn outer_height(file_count: usize) -> f32 {
    if file_count <= FEW_FILES_LIMIT {
        DIALOG_HEIGHT_FEW_FILES
    } else {
        DIALOG_HEIGHT_MANY_FILES
    }
}

pub(crate) fn open_conflicts_dialog(request: ConflictsDialogRequest, cx: &mut App) {
    let ConflictsDialogRequest {
        workspace,
        owner_window,
        repository,
        entries,
        texts,
        description,
        reversed,
        on_closed,
    } = request;
    let on_closed: SharedOnClosed = Rc::new(RefCell::new(Some(on_closed)));
    let Some(project) = workspace
        .upgrade()
        .map(|workspace| workspace.read(cx).project().clone())
    else {
        log::warn!("the workspace of the conflicts dialog is gone");
        finish_without_dialog(&on_closed, cx);
        return;
    };

    let mut files: Vec<RepoPath> = entries.iter().map(|entry| entry.path.clone()).collect();
    files.sort_by(|left, right| left.as_unix_str().cmp(right.as_unix_str()));
    let presence: BTreeMap<_, _> = entries
        .iter()
        .map(|entry| (entry.path.clone(), entry.stages))
        .collect();
    let height = outer_height(files.len());
    let spec = DialogWindowSpec {
        title: texts.dialog_title.clone(),
        initial_size: size(px(DIALOG_WIDTH), px(height)),
        min_size: size(px(DIALOG_MIN_WIDTH), px(DIALOG_MIN_HEIGHT)),
        bounds_key: BOUNDS_KEY,
    };
    let grouping_key = persistence::grouping_key(
        &repository
            .read(cx)
            .work_directory_abs_path
            .to_string_lossy(),
    );
    let group_by_directory = persistence::read_group_by_directory(&grouping_key, cx);

    let on_closed_for_dialog = on_closed.clone();
    let on_closed_for_failure = on_closed;
    open_dialog_window(
        spec,
        DialogOwner::workspace(workspace, owner_window),
        cx,
        move |window, cx| {
            let window_handle = window.window_handle();
            cx.new(|cx| {
                ConflictsDialog::new(
                    NewDialog {
                        window_handle,
                        project,
                        repository,
                        files,
                        presence,
                        reversed,
                        group_by_directory,
                        grouping_key,
                        texts,
                        description,
                        on_closed: on_closed_for_dialog,
                    },
                    window,
                    cx,
                )
            })
        },
        move |opened, cx| match opened {
            Ok(opened_dialog) => register_opened_dialog(opened_dialog, cx),
            Err(error) => {
                log::error!("could not open the conflicts dialog: {error:#}");
                finish_without_dialog(&on_closed_for_failure, cx);
            }
        },
    );
}

#[cfg(test)]
fn register_opened_dialog(opened: DialogWindow<ConflictsDialog>, cx: &mut App) {
    cx.set_global(test_driver::OpenedDialog {
        window: opened.window,
        content: opened.content,
    });
}

#[cfg(not(test))]
fn register_opened_dialog(_opened: DialogWindow<ConflictsDialog>, _cx: &mut App) {}

fn finish_without_dialog(on_closed: &SharedOnClosed, cx: &mut App) {
    let callback = on_closed.borrow_mut().take();
    if let Some(callback) = callback {
        callback(
            ConflictsDialogResult {
                processed_files: Vec::new(),
                should_finish_merge: false,
            },
            cx,
        );
    }
}

struct NewDialog {
    window_handle: AnyWindowHandle,
    project: Entity<Project>,
    repository: Entity<Repository>,
    files: Vec<RepoPath>,
    presence: BTreeMap<RepoPath, git::repository::UnmergedStagePresence>,
    reversed: bool,
    group_by_directory: bool,
    grouping_key: String,
    texts: DialogTexts,
    description: Task<RichText>,
    on_closed: SharedOnClosed,
}

impl ConflictsDialog {
    fn new(parts: NewDialog, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let NewDialog {
            window_handle,
            project,
            repository,
            files,
            presence,
            reversed,
            group_by_directory,
            grouping_key,
            texts,
            description,
            on_closed,
        } = parts;
        let holder = cx.new(|_| MergeConflictIterativeDataHolder::new());
        let state = DialogState::new(files, presence, reversed, group_by_directory);
        let description_task = cx.spawn(async move |this, cx| {
            let loaded = description.await;
            this.update(cx, |this, cx| {
                this.description = Description::Loaded(loaded);
                cx.notify();
            })
            .log_err();
        });
        cx.on_release(|dialog, cx| dialog.report_closed(cx))
            .detach();
        Self {
            window_handle,
            project,
            repository,
            holder,
            state,
            texts,
            description: Description::Loading,
            focus_handle: cx.focus_handle(),
            table_scroll: UniformListScrollHandle::new(),
            hovered_row: None,
            speed_search: None,
            auto_resolve_status: None,
            overlay: None,
            context_menu: None,
            grouping_key,
            should_finish_merge: false,
            closing: false,
            on_closed,
            _description_task: description_task,
        }
    }

    fn report_closed(&mut self, cx: &mut App) {
        self.holder.update(cx, |holder, _| holder.dispose());
        let callback = self.on_closed.borrow_mut().take();
        if let Some(callback) = callback {
            callback(
                ConflictsDialogResult {
                    processed_files: self.state.processed_files().to_vec(),
                    should_finish_merge: self.should_finish_merge,
                },
                cx,
            );
        }
    }

    fn table_enabled(&self) -> bool {
        !self.state.is_resolving()
    }

    fn render_description(&self, all_reviewed: bool, cx: &App) -> AnyElement {
        let content = if all_reviewed {
            h_flex()
                .gap_2()
                .child(
                    div()
                        .relative()
                        .size(px(16.))
                        .child(
                            Icon::new(IconName::StatusSuccessDisc)
                                .color(Color::Success)
                                .size(IconSize::Medium),
                        )
                        .child(
                            div().absolute().top_0().left_0().child(
                                Icon::new(IconName::StatusSuccessCheck)
                                    .color(Color::Custom(gpui::white()))
                                    .size(IconSize::Medium),
                            ),
                        ),
                )
                .child(Label::new(messages::ALL_RESOLVED_DESCRIPTION))
                .into_any_element()
        } else {
            match &self.description {
                Description::Loading => {
                    Label::new(messages::LOADING_DESCRIPTION).into_any_element()
                }
                Description::Loaded(text) => div()
                    .w_full()
                    .text_ui(cx)
                    .child(rich_label(text, cx))
                    .into_any_element(),
            }
        };
        div()
            .w_full()
            .pt(px(14.))
            .pb(px(10.))
            .child(content)
            .into_any_element()
    }

    fn render_resolve_all_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let resolving = self.state.is_resolving();
        let enabled = self.state.resolve_all_enabled();
        let label = if resolving {
            messages::RESOLVING_CONFLICTS
        } else {
            messages::RESOLVE_ALL_SIMPLE_CONFLICTS
        };
        let tooltip = if resolving {
            None
        } else if enabled {
            Some(messages::RESOLVE_ALL_TOOLTIP)
        } else {
            Some(messages::RESOLVE_ALL_DISABLED_TOOLTIP)
        };
        let icon = if resolving {
            Icon::new(IconName::LoadCircle)
                .size(IconSize::Small)
                .color(Color::Muted)
                .with_keyed_rotate_animation("conflicts-resolve-all-spinner", 2)
                .into_any_element()
        } else {
            Icon::new(IconName::DiffMagicResolve)
                .size(IconSize::Small)
                .into_any_element()
        };
        let content = h_flex()
            .gap(px(RESOLVE_ICON_TEXT_GAP))
            .items_center()
            .child(icon)
            .child(Label::new(label));
        dialog_button_with_content("conflicts-resolve-all-simple", content, false, !enabled, cx)
            .when_some(tooltip, |button, text| button.tooltip(Tooltip::text(text)))
            .when(enabled, |button| {
                button.on_click(cx.listener(|this, _, window, cx| {
                    this.resolve_all_simple_conflicts(window, cx);
                }))
            })
            .into_any_element()
    }

    fn render_controls_row(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .w_full()
            .py(px(6.))
            .gap(px(CONTROLS_GAP))
            .items_center()
            .child(self.render_resolve_all_button(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .when_some(self.auto_resolve_status.clone(), |this, status| {
                        this.child(Label::new(status))
                    }),
            )
            .child(self.render_view_options(cx))
            .into_any_element()
    }

    fn render_south_buttons(&self, cx: &mut Context<Self>) -> AnyElement {
        let enabled = self.table_enabled();
        let default_button = self.state.default_button();
        let accept_enabled = self.state.accept_and_finish_enabled();
        let review_visible = self.state.review_button_visible();
        let review_label = self.state.review_button_label();

        h_flex()
            .w_full()
            .pt(px(SOUTH_TOP_GAP))
            .gap(px(SOUTH_BUTTON_GAP))
            .justify_end()
            .child(
                dialog_button("conflicts-close", messages::CLOSE, false, false, cx).on_click(
                    cx.listener(|this, _, window, cx| {
                        this.close_button_pressed(window, cx);
                    }),
                ),
            )
            .child(
                dialog_button(
                    "conflicts-accept-and-finish",
                    messages::ACCEPT_AND_FINISH,
                    default_button == DefaultButton::AcceptAndFinish,
                    !accept_enabled,
                    cx,
                )
                .when(accept_enabled, |button| {
                    button.on_click(cx.listener(|this, _, window, cx| {
                        this.accept_and_finish(window, cx);
                    }))
                }),
            )
            .when(review_visible, |this| {
                this.child(
                    dialog_button(
                        "conflicts-review-or-resolve",
                        review_label,
                        default_button == DefaultButton::ReviewOrResolve,
                        !enabled,
                        cx,
                    )
                    .when(enabled, |button| {
                        button.on_click(cx.listener(|this, _, window, cx| {
                            this.review_button_pressed(window, cx);
                        }))
                    }),
                )
            })
            .into_any_element()
    }
}

impl Focusable for ConflictsDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl DialogWindowContent for ConflictsDialog {
    fn close_requested(
        &mut self,
        _request: CloseRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> CloseDecision {
        self.silent_close_requested(window, cx)
    }
}

impl Render for ConflictsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let all_reviewed = self.state.derived().all_files_resolved_and_reviewed;
        v_flex()
            .id("conflicts-dialog")
            .key_context("ConflictsDialog")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_first))
            .on_action(cx.listener(Self::select_last))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::secondary_confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::select_all_rows))
            .on_action(cx.listener(Self::extend_selection_up))
            .on_action(cx.listener(Self::extend_selection_down))
            .on_action(cx.listener(Self::collapse_row))
            .on_action(cx.listener(Self::expand_row))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::extend_page_up))
            .on_action(cx.listener(Self::extend_page_down))
            .on_action(cx.listener(Self::extend_to_first_row))
            .on_action(cx.listener(Self::extend_to_last_row))
            .on_key_down(cx.listener(Self::handle_key_down))
            .relative()
            .size_full()
            .px(px(12.))
            .py(px(8.))
            .child(self.render_description(all_reviewed, cx))
            .child(self.render_controls_row(cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(60.))
                    .w_full()
                    .py(px(6.))
                    .child(self.render_tree_table(window, cx))
                    .children(self.render_speed_search(cx)),
            )
            .child(self.render_south_buttons(cx))
            .children(self.render_context_menu())
            .children(self.render_overlay(cx))
    }
}
