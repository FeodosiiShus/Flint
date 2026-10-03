use anyhow::Result;
use collections::{HashMap, HashSet};
use file_icons::FileIcons;
use git::{
    repository::RepoPath,
    status::{FileStatus, UnmergedStatus, UnmergedStatusCode},
};
use gpui::{
    Action as _, AnyElement, App, AsyncApp, ClickEvent, Context, DismissEvent, Entity,
    EventEmitter, FocusHandle, Focusable, FontWeight, MouseButton, MouseDownEvent, Pixels, Point,
    Render, Subscription, TaskExt as _, WeakEntity, Window, anchored, deferred,
};
use project::{
    Project,
    git_store::{Repository, RepositoryEvent, RepositoryId},
};
use three_way_merge::{
    Counters, IgnorePolicy, MergeDocument, MergeModel, NonConflictingScope, Side,
};
use ui::{ContextMenu, Modal, ModalFooter, ModalHeader, Section, prelude::*};
use util::{ResultExt as _, paths::PathStyle};
use workspace::{ModalView, Workspace, notifications::DetachAndPromptErr as _};

use super::{
    AcceptTheirs, AcceptYours, MergeBranches, MergeSessions, OpenMergeTool,
    RevertConflictResolution, accept_side, clear_merge_session, document_matches_merge,
    merge_for_stages, merge_session, merge_session_counters, open_merge_tool, resolved_text,
    store_merge_session, write_merge_result,
};
use crate::git_panel::GitPanel;

const SIDE_COLUMN_WIDTH: f32 = 10.;
const DIALOG_WIDTH: f32 = 48.;
const LIST_MAX_HEIGHT: f32 = 24.;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ConflictRow {
    pub(super) repo_path: RepoPath,
    pub(super) status: Option<UnmergedStatus>,
}

struct RowContextMenu {
    menu: Entity<ContextMenu>,
    position: Point<Pixels>,
    _subscription: Subscription,
}

pub struct ConflictsDialog {
    workspace: WeakEntity<Workspace>,
    project: Entity<Project>,
    repository: Entity<Repository>,
    repository_id: RepositoryId,
    focus_handle: FocusHandle,
    pub(super) branches: MergeBranches,
    pub(super) unresolved: Vec<ConflictRow>,
    pub(super) resolved: Vec<ConflictRow>,
    pub(super) selected: Option<RepoPath>,
    known_statuses: HashMap<RepoPath, UnmergedStatus>,
    fresh_counters: HashMap<RepoPath, Counters>,
    counting: HashSet<RepoPath>,
    resolving_simple_conflicts: bool,
    context_menu: Option<RowContextMenu>,
    _subscriptions: Vec<Subscription>,
}

impl ConflictsDialog {
    pub(crate) fn new(
        workspace: WeakEntity<Workspace>,
        project: Entity<Project>,
        repository: Entity<Repository>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let repository_id = repository.read(cx).id;
        let branches = MergeBranches::for_snapshot(repository.read(cx));
        let subscriptions = vec![
            cx.subscribe(&repository, |this, _, event: &RepositoryEvent, cx| {
                if matches!(
                    event,
                    RepositoryEvent::StatusesChanged | RepositoryEvent::HeadChanged
                ) {
                    this.refresh(cx);
                }
            }),
            cx.observe_global::<MergeSessions>(|_, cx| cx.notify()),
        ];
        let mut dialog = Self {
            workspace,
            project,
            repository,
            repository_id,
            focus_handle: cx.focus_handle(),
            branches,
            unresolved: Vec::new(),
            resolved: Vec::new(),
            selected: None,
            known_statuses: HashMap::default(),
            fresh_counters: HashMap::default(),
            counting: HashSet::default(),
            resolving_simple_conflicts: false,
            context_menu: None,
            _subscriptions: subscriptions,
        };
        dialog.refresh(cx);
        dialog
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let repository = self.repository.read(cx);
        self.branches = MergeBranches::for_snapshot(repository);
        let mut paths = repository
            .merge
            .merge_heads_by_conflicted_path
            .iter()
            .map(|(repo_path, _)| repo_path.clone())
            .collect::<Vec<_>>();
        for entry in repository.status() {
            if entry.status.is_conflicted() && !paths.contains(&entry.repo_path) {
                paths.push(entry.repo_path);
            }
        }
        paths.sort();

        let mut unresolved = Vec::new();
        let mut resolved = Vec::new();
        for repo_path in paths {
            match repository
                .status_for_path(&repo_path)
                .map(|entry| entry.status)
            {
                Some(FileStatus::Unmerged(status)) => {
                    self.known_statuses.insert(repo_path.clone(), status);
                    unresolved.push(ConflictRow {
                        repo_path,
                        status: Some(status),
                    });
                }
                _ => {
                    let status = self.known_statuses.get(&repo_path).copied();
                    resolved.push(ConflictRow { repo_path, status });
                }
            }
        }
        self.unresolved = unresolved;
        self.resolved = resolved;

        let selection_exists = self
            .selected
            .as_ref()
            .is_some_and(|selected| self.rows().any(|row| &row.repo_path == selected));
        if !selection_exists {
            let first_row = self.rows().next().map(|row| row.repo_path.clone());
            self.selected = first_row;
        }
        self.count_changes(cx);
        cx.notify();
    }

    fn rows(&self) -> impl Iterator<Item = &ConflictRow> {
        self.unresolved.iter().chain(self.resolved.iter())
    }

    fn count_changes(&mut self, cx: &mut Context<Self>) {
        for row in &self.unresolved {
            let repo_path = row.repo_path.clone();
            if self.fresh_counters.contains_key(&repo_path)
                || self.counting.contains(&repo_path)
                || merge_session_counters(self.repository_id, &repo_path, cx).is_some()
            {
                continue;
            }
            self.counting.insert(repo_path.clone());
            let stages = self.repository.update(cx, |repository, cx| {
                repository.load_conflict_stages(repo_path.clone(), cx)
            });
            cx.spawn(async move |this, cx| {
                let stages = stages.await;
                this.update(cx, |this, cx| {
                    this.counting.remove(&repo_path);
                    if let Some(stages) = stages.log_err() {
                        let counters =
                            MergeModel::new(merge_for_stages(&stages, IgnorePolicy::default()))
                                .counters();
                        this.fresh_counters.insert(repo_path, counters);
                    }
                    cx.notify();
                })
            })
            .detach_and_log_err(cx);
        }
    }

    fn is_unresolved(&self, repo_path: &RepoPath) -> bool {
        self.unresolved
            .iter()
            .any(|row| &row.repo_path == repo_path)
    }

    fn selected_unresolved(&self) -> Option<RepoPath> {
        self.selected
            .clone()
            .filter(|selected| self.is_unresolved(selected))
    }

    fn selected_resolved(&self) -> Option<RepoPath> {
        self.selected
            .clone()
            .filter(|selected| self.resolved.iter().any(|row| &row.repo_path == selected))
    }

    pub(super) fn badge(&self, row: &ConflictRow, cx: &App) -> Option<String> {
        let counters = merge_session_counters(self.repository_id, &row.repo_path, cx)
            .or_else(|| self.fresh_counters.get(&row.repo_path).copied())?;
        if row.status.is_some() && self.is_unresolved(&row.repo_path) {
            Some(format!("{}/{}", counters.resolved(), counters.total))
        } else {
            Some(format!("{}/{}", counters.total, counters.total))
        }
    }

    pub(super) fn select(&mut self, repo_path: RepoPath, cx: &mut Context<Self>) {
        self.selected = Some(repo_path);
        cx.notify();
    }

    fn select_relative(&mut self, forward: bool, cx: &mut Context<Self>) {
        let paths = self
            .rows()
            .map(|row| row.repo_path.clone())
            .collect::<Vec<_>>();
        if paths.is_empty() {
            return;
        }
        let current = self
            .selected
            .as_ref()
            .and_then(|selected| paths.iter().position(|path| path == selected));
        let next = match (current, forward) {
            (None, true) => 0,
            (None, false) => paths.len() - 1,
            (Some(index), true) => (index + 1).min(paths.len() - 1),
            (Some(index), false) => index.saturating_sub(1),
        };
        if let Some(path) = paths.get(next) {
            self.select(path.clone(), cx);
        }
    }

    fn select_next(&mut self, _: &menu::SelectNext, _window: &mut Window, cx: &mut Context<Self>) {
        self.select_relative(true, cx);
    }

    fn select_previous(
        &mut self,
        _: &menu::SelectPrevious,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_relative(false, cx);
    }

    fn cancel(&mut self, _: &menu::Cancel, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn confirm(&mut self, _: &menu::Confirm, window: &mut Window, cx: &mut Context<Self>) {
        self.resolve_manually(window, cx);
    }

    fn open_merge_tool_action(
        &mut self,
        _: &OpenMergeTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.resolve_manually(window, cx);
    }

    fn accept_yours(&mut self, _: &AcceptYours, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(repo_path) = self.selected_unresolved() {
            self.accept(repo_path, Side::Left, window, cx);
        }
    }

    fn accept_theirs(&mut self, _: &AcceptTheirs, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(repo_path) = self.selected_unresolved() {
            self.accept(repo_path, Side::Right, window, cx);
        }
    }

    fn revert_conflict_resolution(
        &mut self,
        _: &RevertConflictResolution,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(repo_path) = self.selected_resolved() {
            self.revert_resolution(repo_path, window, cx);
        }
    }

    fn accept(
        &mut self,
        repo_path: RepoPath,
        side: Side,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        accept_side(
            self.project.clone(),
            self.repository.clone(),
            repo_path,
            side,
            cx,
        )
        .detach_and_prompt_err(
            "Failed to accept the conflict side",
            window,
            cx,
            |_, _, _| None,
        );
    }

    fn revert_resolution(
        &mut self,
        repo_path: RepoPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        clear_merge_session(self.repository_id, &repo_path, cx);
        self.fresh_counters.remove(&repo_path);
        self.repository
            .update(cx, |repository, cx| {
                repository.unresolve_paths(vec![repo_path], cx)
            })
            .detach_and_prompt_err(
                "Failed to revert the conflict resolution",
                window,
                cx,
                |_, _, _| None,
            );
    }

    fn resolve_manually(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo_path) = self.selected_unresolved() else {
            return;
        };
        let repository = self.repository.clone();
        cx.emit(DismissEvent);
        self.workspace
            .update(cx, |workspace, cx| {
                open_merge_tool(workspace, repository, repo_path, window, cx)
                    .detach_and_prompt_err(
                        "Failed to open the merge tool",
                        window,
                        cx,
                        |_, _, _| None,
                    );
            })
            .log_err();
    }

    pub(super) fn accept_and_finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.unresolved.is_empty() {
            return;
        }
        self.workspace
            .update(cx, |workspace, cx| {
                if let Some(panel) = workspace.focus_panel::<GitPanel>(window, cx) {
                    let commit_editor = panel.read(cx).commit_editor.focus_handle(cx);
                    window.focus(&commit_editor, cx);
                }
            })
            .log_err();
        cx.emit(DismissEvent);
    }

    pub(super) fn resolve_all_simple_conflicts(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.resolving_simple_conflicts {
            return;
        }
        let paths = self
            .unresolved
            .iter()
            .map(|row| row.repo_path.clone())
            .collect::<Vec<_>>();
        if paths.is_empty() {
            return;
        }
        self.resolving_simple_conflicts = true;
        cx.notify();
        let project = self.project.clone();
        let repository = self.repository.clone();
        cx.spawn(async move |this, cx| {
            let mut first_error = None;
            for repo_path in paths {
                let result = resolve_simple_conflicts_in_file(
                    project.clone(),
                    repository.clone(),
                    repo_path,
                    cx,
                )
                .await;
                if let Err(error) = result {
                    first_error.get_or_insert(error);
                }
            }
            this.update(cx, |this, cx| {
                this.resolving_simple_conflicts = false;
                this.refresh(cx);
            })?;
            match first_error {
                Some(error) => Err(error),
                None => Ok(()),
            }
        })
        .detach_and_prompt_err(
            "Failed to resolve simple conflicts",
            window,
            cx,
            |_, _, _| None,
        );
    }

    fn deploy_context_menu(
        &mut self,
        repo_path: RepoPath,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select(repo_path.clone(), cx);
        let dialog = cx.weak_entity();
        let menu = ContextMenu::build(window, cx, move |menu, _, _| {
            menu.entry(
                "Revert conflict resolution",
                Some(RevertConflictResolution.boxed_clone()),
                move |window, cx| {
                    dialog
                        .update(cx, |dialog, cx| {
                            dialog.revert_resolution(repo_path.clone(), window, cx);
                        })
                        .log_err();
                },
            )
        });
        let menu_focus_handle = menu.focus_handle(cx);
        window.defer(cx, move |window, cx| {
            window.focus(&menu_focus_handle, cx);
        });
        let subscription =
            cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
                if this.context_menu.as_ref().is_some_and(|context_menu| {
                    context_menu
                        .menu
                        .focus_handle(cx)
                        .contains_focused(window, cx)
                }) {
                    window.focus(&this.focus_handle, cx);
                }
                this.context_menu = None;
                cx.notify();
            });
        self.context_menu = Some(RowContextMenu {
            menu,
            position,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn render_header(&self) -> impl IntoElement {
        h_flex()
            .gap_1()
            .child(
                Label::new("Merging branch")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(
                Label::new(self.branches.theirs.clone())
                    .size(LabelSize::Small)
                    .weight(FontWeight::BOLD),
            )
            .child(
                Label::new("into branch")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(
                Label::new(self.branches.ours.clone())
                    .size(LabelSize::Small)
                    .weight(FontWeight::BOLD),
            )
    }

    fn render_column_headers(&self, cx: &App) -> impl IntoElement {
        h_flex()
            .w_full()
            .px_2()
            .py_0p5()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().colors().border_variant)
            .child(
                div().flex_1().min_w_0().child(
                    Label::new("Name")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                ),
            )
            .child(
                div().w(rems(SIDE_COLUMN_WIDTH)).child(
                    Label::new(format!("Yours ({})", self.branches.ours))
                        .size(LabelSize::Small)
                        .color(Color::Muted)
                        .truncate(),
                ),
            )
            .child(
                div().w(rems(SIDE_COLUMN_WIDTH)).child(
                    Label::new(format!("Theirs ({})", self.branches.theirs))
                        .size(LabelSize::Small)
                        .color(Color::Muted)
                        .truncate(),
                ),
            )
    }

    fn render_group_header(&self, unresolved: bool, count: usize) -> impl IntoElement {
        let (icon, color, title) = if unresolved {
            (IconName::XCircle, Color::Error, "Unresolved")
        } else {
            (IconName::Check, Color::Success, "Resolved")
        };
        h_flex()
            .w_full()
            .px_2()
            .pt_1()
            .gap_1()
            .child(Icon::new(icon).size(IconSize::Small).color(color))
            .child(Label::new(title).size(LabelSize::Small))
            .child(
                Label::new(format!(
                    "{count} {}",
                    if count == 1 { "file" } else { "files" }
                ))
                .size(LabelSize::Small)
                .color(Color::Muted),
            )
    }

    fn render_side_cell(
        &self,
        row: &ConflictRow,
        side: Side,
        show_accept: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if show_accept {
            let repo_path = row.repo_path.clone();
            let id = match side {
                Side::Left => "conflicts-accept-yours",
                Side::Right => "conflicts-accept-theirs",
            };
            return Button::new(id, "Accept")
                .label_size(LabelSize::Small)
                .style(ButtonStyle::Filled)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.accept(repo_path.clone(), side, window, cx);
                }))
                .into_any_element();
        }
        let code = row.status.map(|status| match side {
            Side::Left => status.first_head,
            Side::Right => status.second_head,
        });
        Label::new(code.map(change_label).unwrap_or_default())
            .size(LabelSize::Small)
            .color(Color::Muted)
            .into_any_element()
    }

    fn render_row(
        &self,
        row: &ConflictRow,
        index: usize,
        unresolved: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.selected.as_ref() == Some(&row.repo_path);
        let colors = cx.theme().colors();
        let hover_background = colors.element_hover;
        let selected_background = colors.element_selected;
        let file_name = row.repo_path.file_name().unwrap_or_default().to_string();
        let directory = row
            .repo_path
            .parent()
            .map(|parent| parent.display(PathStyle::local()).into_owned())
            .filter(|directory| !directory.is_empty());
        let icon = FileIcons::get_icon(row.repo_path.as_std_path(), cx)
            .map(Icon::from_path)
            .unwrap_or_else(|| Icon::new(IconName::File))
            .size(IconSize::Small)
            .color(Color::Muted);
        let badge = self.badge(row, cx);
        let show_accept = selected && unresolved;
        let click_path = row.repo_path.clone();
        let menu_path = row.repo_path.clone();
        let id = if unresolved {
            ("conflicts-unresolved-row", index)
        } else {
            ("conflicts-resolved-row", index)
        };
        h_flex()
            .id(id)
            .w_full()
            .px_2()
            .py_0p5()
            .gap_2()
            .rounded_sm()
            .when(selected, |this| this.bg(selected_background))
            .hover(move |style| style.bg(hover_background))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.select(click_path.clone(), cx);
                if event.click_count() > 1 {
                    this.resolve_manually(window, cx);
                }
            }))
            .when(!unresolved, |this| {
                this.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        this.deploy_context_menu(menu_path.clone(), event.position, window, cx);
                    }),
                )
            })
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(icon)
                    .child(Label::new(file_name).size(LabelSize::Small))
                    .when_some(directory, |this, directory| {
                        this.child(
                            Label::new(directory)
                                .size(LabelSize::Small)
                                .color(Color::Muted)
                                .truncate(),
                        )
                    })
                    .when_some(badge, |this, badge| {
                        this.child(
                            Label::new(badge)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                    }),
            )
            .child(
                div()
                    .w(rems(SIDE_COLUMN_WIDTH))
                    .child(self.render_side_cell(row, Side::Left, show_accept, cx)),
            )
            .child(
                div()
                    .w(rems(SIDE_COLUMN_WIDTH))
                    .child(self.render_side_cell(row, Side::Right, show_accept, cx)),
            )
            .into_any_element()
    }
}

impl EventEmitter<DismissEvent> for ConflictsDialog {}

impl ModalView for ConflictsDialog {}

impl Focusable for ConflictsDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ConflictsDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let unresolved_rows = self
            .unresolved
            .iter()
            .enumerate()
            .map(|(index, row)| self.render_row(row, index, true, cx))
            .collect::<Vec<_>>();
        let resolved_rows = self
            .resolved
            .iter()
            .enumerate()
            .map(|(index, row)| self.render_row(row, index, false, cx))
            .collect::<Vec<_>>();
        let has_unresolved = !self.unresolved.is_empty();
        let can_resolve_manually = self.selected_unresolved().is_some();

        v_flex()
            .key_context("ConflictsDialog")
            .w(rems(DIALOG_WIDTH))
            .elevation_3(cx)
            .overflow_hidden()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::open_merge_tool_action))
            .on_action(cx.listener(Self::accept_yours))
            .on_action(cx.listener(Self::accept_theirs))
            .on_action(cx.listener(Self::revert_conflict_resolution))
            .child(
                Modal::new("conflicts-dialog", None)
                    .header(
                        ModalHeader::new()
                            .icon(
                                Icon::new(IconName::GitMergeConflict)
                                    .size(IconSize::Small)
                                    .color(Color::Muted),
                            )
                            .headline("Conflicts")
                            .show_dismiss_button(true),
                    )
                    .section(
                        Section::new().child(
                            v_flex()
                                .gap_1()
                                .child(self.render_header())
                                .child(
                                    h_flex().child(
                                        Button::new(
                                            "resolve-all-simple-conflicts",
                                            "Resolve All Simple Conflicts",
                                        )
                                        .label_size(LabelSize::Small)
                                        .start_icon(Icon::new(IconName::Wand).size(IconSize::Small))
                                        .loading(self.resolving_simple_conflicts)
                                        .disabled(
                                            !has_unresolved || self.resolving_simple_conflicts,
                                        )
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.resolve_all_simple_conflicts(window, cx);
                                            }),
                                        ),
                                    ),
                                )
                                .child(self.render_column_headers(cx))
                                .child(
                                    v_flex()
                                        .id("conflicts-dialog-rows")
                                        .max_h(rems(LIST_MAX_HEIGHT))
                                        .overflow_y_scroll()
                                        .child(
                                            self.render_group_header(true, self.unresolved.len()),
                                        )
                                        .children(unresolved_rows)
                                        .child(self.render_group_header(false, self.resolved.len()))
                                        .children(resolved_rows),
                                ),
                        ),
                    )
                    .footer(
                        ModalFooter::new().end_slot(
                            h_flex()
                                .gap_1()
                                .child(Button::new("conflicts-close", "Close").on_click(
                                    cx.listener(|_, _, _, cx| {
                                        cx.emit(DismissEvent);
                                    }),
                                ))
                                .child(
                                    Button::new("conflicts-accept-and-finish", "Accept and Finish")
                                        .disabled(has_unresolved)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.accept_and_finish(window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("conflicts-resolve-manually", "Resolve Manually")
                                        .style(ButtonStyle::Filled)
                                        .disabled(!can_resolve_manually)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.resolve_manually(window, cx);
                                        })),
                                ),
                        ),
                    ),
            )
            .children(self.context_menu.as_ref().map(|context_menu| {
                deferred(
                    anchored()
                        .position(context_menu.position)
                        .anchor(gpui::Anchor::TopLeft)
                        .child(context_menu.menu.clone()),
                )
                .with_priority(1)
            }))
    }
}

fn change_label(code: UnmergedStatusCode) -> &'static str {
    match code {
        UnmergedStatusCode::Added => "Added",
        UnmergedStatusCode::Deleted => "Deleted",
        UnmergedStatusCode::Updated => "Modified",
    }
}

async fn resolve_simple_conflicts_in_file(
    project: Entity<Project>,
    repository: Entity<Repository>,
    repo_path: RepoPath,
    cx: &mut AsyncApp,
) -> Result<()> {
    let stages = repository
        .update(cx, |repository, cx| {
            repository.load_conflict_stages(repo_path.clone(), cx)
        })
        .await?;
    let write = cx.update(|cx| {
        let repository_id = repository.read(cx).id;
        let merge = merge_for_stages(&stages, IgnorePolicy::default());
        let mut document = merge_session(repository_id, &repo_path, cx)
            .filter(|document| document_matches_merge(document, &merge))
            .unwrap_or_else(|| MergeDocument::new(merge));
        document.apply_non_conflicting(NonConflictingScope::All);
        document.resolve_simple_conflicts();
        if document.model().counters().is_complete() {
            let text = resolved_text(&stages, document.result().text().to_string());
            Some(write_merge_result(project, repository, repo_path, text, cx))
        } else {
            store_merge_session(repository_id, repo_path, document, cx);
            None
        }
    });
    if let Some(write) = write {
        write.await?;
    }
    Ok(())
}
