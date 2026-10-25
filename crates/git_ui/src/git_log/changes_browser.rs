mod model;

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use collections::{HashMap, HashSet};
use file_icons::FileIcons;
use git::Oid;
use git::repository::CommitData;
use git::status::{DiffTreeType, TreeDiff, TreeDiffStatus};
use gpui::{
    Anchor, AnyElement, App, AsyncApp, Context, DismissEvent, Entity, EventEmitter, FocusHandle,
    Focusable, MouseButton, MouseDownEvent, Pixels, Point, ScrollStrategy, ScrollWheelEvent,
    SharedString, Subscription, Task, UniformListScrollHandle, WeakEntity, Window, actions,
    anchored, deferred, px, uniform_list,
};
use project::git_store::{CommitDataState, Repository};
use ui::{
    CommonAnimationExt as _, ContextMenu, IconButton, IconButtonShape, PopoverMenu, Tooltip,
    prelude::*,
};
use workspace::Workspace;

use self::model::{
    Change, ChangeKind, ChangesData, ChangesState, CommitChanges, NodeKind, StatusAction, TreeNode,
    TreeRow, affected_paths, all_container_ids, ancestors_of, build_tree, changes_state,
    default_expanded, find_node, flatten, ids_of_changes, merge_parent_changes, parent_label,
    status_text,
};
use super::properties::{LogAppProperties, LogTabProperties};
use crate::commit_view::CommitView;

actions!(
    git_log_changes,
    [
        ShowDiff,
        ShowHistory,
        ExpandAll,
        CollapseAll,
        ToggleGroupByDirectory
    ]
);

const KEY_CONTEXT: &str = "GitLogChanges";
const TREE_KEY_CONTEXT: &str = "GitLogChangesTree menu";
const EMPTY_TREE_SHA: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const LOADING_TEXT: &str = "Loading…";
const LOADING_DELAY: Duration = Duration::from_millis(300);
const ROW_HEIGHT: Pixels = px(24.);
const TREE_LEFT_INSET: Pixels = px(12.);
const TREE_INDENT: Pixels = px(18.);
const RENDERER_INSET: Pixels = px(3.);
const ICON_TEXT_GAP: Pixels = px(2.);
const TOOLBAR_BUTTON_SIZE: Pixels = px(22.);
const TOOLBAR_HORIZONTAL_INSET: Pixels = px(7.);
const DEFAULT_TOOLBAR_HEIGHT: Pixels = px(40.);
const STATUS_LINE_GAP: Pixels = px(2.);
const SELECTION_INSET: Pixels = px(4.);
const SELECTION_RADIUS: Pixels = px(6.);
const INACTIVE_SELECTION_OPACITY: f32 = 0.5;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ChangesBrowserEvent {
    TabPropertiesChanged(LogTabProperties),
    AppPropertiesChanged(LogAppProperties),
}

pub(crate) struct ChangesBrowser {
    workspace: WeakEntity<Workspace>,
    repository: Option<Entity<Repository>>,
    commits: Vec<Oid>,
    loaded: Vec<CommitChanges>,
    parent_labels: HashMap<Oid, SharedString>,
    app_properties: LogAppProperties,
    tab_properties: LogTabProperties,
    show_anyway: bool,
    toolbar_height: Pixels,
    focus_handle: FocusHandle,
    scroll_handle: UniformListScrollHandle,
    state: ChangesState,
    tree: Vec<TreeNode>,
    rows: Vec<TreeRow>,
    expanded: HashSet<SharedString>,
    selection: Vec<SharedString>,
    selection_anchor: Option<SharedString>,
    loading: bool,
    loading_visible: bool,
    _load_task: Task<()>,
    _loading_indicator_task: Task<()>,
    context_menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
}

impl ChangesBrowser {
    pub(crate) fn new(
        workspace: WeakEntity<Workspace>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            workspace,
            repository: None,
            commits: Vec::new(),
            loaded: Vec::new(),
            parent_labels: HashMap::default(),
            app_properties: LogAppProperties::default(),
            tab_properties: LogTabProperties::default(),
            show_anyway: false,
            toolbar_height: DEFAULT_TOOLBAR_HEIGHT,
            focus_handle: cx.focus_handle(),
            scroll_handle: UniformListScrollHandle::new(),
            state: ChangesState::Changes(ChangesData::default()),
            tree: Vec::new(),
            rows: Vec::new(),
            expanded: HashSet::default(),
            selection: Vec::new(),
            selection_anchor: None,
            loading: false,
            loading_visible: false,
            _load_task: Task::ready(()),
            _loading_indicator_task: Task::ready(()),
            context_menu: None,
        }
    }

    pub(crate) fn set_commits(
        &mut self,
        repository: Option<Entity<Repository>>,
        commits: Vec<Oid>,
        cx: &mut Context<Self>,
    ) {
        let same_repository = match (&self.repository, &repository) {
            (Some(current), Some(new)) => current == new,
            (None, None) => true,
            _ => false,
        };
        if same_repository && self.commits == commits && !matches!(self.state, ChangesState::Error)
        {
            return;
        }
        self.repository = repository;
        self.commits = commits;
        self.show_anyway = false;
        self.loaded.clear();
        self.parent_labels.clear();
        self.load(cx);
    }

    pub(crate) fn set_properties(
        &mut self,
        app_properties: LogAppProperties,
        tab_properties: LogTabProperties,
        cx: &mut Context<Self>,
    ) {
        let grouping_changed = app_properties.changes_group_by_directory
            != self.app_properties.changes_group_by_directory;
        let tree_changed = app_properties.show_changes_from_parents
            != self.app_properties.show_changes_from_parents
            || tab_properties.show_only_affected_changes
                != self.tab_properties.show_only_affected_changes
            || tab_properties.filters.paths != self.tab_properties.filters.paths;
        let selected = self.selected_changes();
        self.app_properties = app_properties;
        self.tab_properties = tab_properties;
        if tree_changed {
            self.rebuild(true, cx);
        } else if grouping_changed {
            self.regroup(selected, cx);
        } else {
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(crate) fn commits(&self) -> &[Oid] {
        &self.commits
    }

    pub(crate) fn set_toolbar_height(&mut self, height: Pixels, cx: &mut Context<Self>) {
        self.toolbar_height = height;
        cx.notify();
    }

    pub(crate) fn show_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.as_ref() else {
            return;
        };
        let changes = self.selected_or_all_changes();
        let Some(change) = changes.first() else {
            return;
        };
        CommitView::open(
            change.commit.to_string(),
            repository.downgrade(),
            self.workspace.clone(),
            None,
            Some(change.path.clone()),
            window,
            cx,
        );
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.context_menu = None;
        let repository = self
            .repository
            .as_ref()
            .filter(|_| !self.commits.is_empty())
            .map(Entity::downgrade);
        let Some(repository) = repository else {
            self._load_task = Task::ready(());
            self._loading_indicator_task = Task::ready(());
            self.loading = false;
            self.loading_visible = false;
            self.state = ChangesState::Changes(ChangesData::default());
            self.rebuild(true, cx);
            return;
        };
        self.state = ChangesState::Empty;
        self.loading = true;
        self.loading_visible = false;
        self.rebuild(true, cx);
        self._loading_indicator_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LOADING_DELAY).await;
            this.update(cx, |this, cx| {
                if this.loading {
                    this.loading_visible = true;
                    cx.notify();
                }
            })
            .ok();
        });
        let commits = self.commits.clone();
        self._load_task = cx.spawn(async move |this, cx| {
            let result = load_commit_changes(repository, commits, cx).await;
            this.update(cx, |this, cx| {
                this.loading = false;
                this.loading_visible = false;
                match result {
                    Ok((loaded, parent_labels)) => {
                        this.loaded = loaded;
                        this.parent_labels = parent_labels;
                        this.state =
                            changes_state(&this.loaded, &this.parent_labels, this.show_anyway);
                    }
                    Err(error) => {
                        log::error!("error loading changes of the selected commits: {error:#}");
                        this.state = ChangesState::Error;
                    }
                }
                this.rebuild(true, cx);
            })
            .ok();
        });
    }

    fn repository_name(&self, cx: &App) -> String {
        self.repository
            .as_ref()
            .and_then(|repository| {
                repository
                    .read(cx)
                    .work_directory_abs_path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
            })
            .unwrap_or_default()
    }

    fn rebuild(&mut self, reset_tree_state: bool, cx: &mut Context<Self>) {
        let affected = affected_paths(&self.tab_properties.filters.paths);
        self.tree = match &self.state {
            ChangesState::Changes(data) => build_tree(
                data,
                affected.as_deref(),
                self.tab_properties.show_only_affected_changes,
                self.app_properties.show_changes_from_parents,
                self.app_properties.changes_group_by_directory,
                &self.repository_name(cx),
            ),
            _ => Vec::new(),
        };
        if reset_tree_state {
            self.expanded = default_expanded(&self.tree);
            self.selection.clear();
            self.selection_anchor = None;
        }
        self.rows = flatten(&self.tree, &self.expanded);
        cx.notify();
    }

    fn refresh_rows(&mut self, cx: &mut Context<Self>) {
        self.rows = flatten(&self.tree, &self.expanded);
        self.selection
            .retain(|id| self.rows.iter().any(|row| &row.id == id));
        cx.notify();
    }

    fn selected_changes(&self) -> Vec<Change> {
        let mut changes = Vec::new();
        for id in &self.selection {
            if let Some(node) = find_node(&self.tree, id) {
                node.changes(&mut changes);
            }
        }
        changes.dedup();
        changes
    }

    fn selected_or_all_changes(&self) -> Vec<Change> {
        let selected = self.selected_changes();
        if !selected.is_empty() {
            return selected;
        }
        let mut changes = Vec::new();
        for node in &self.tree {
            node.changes(&mut changes);
        }
        changes
    }

    fn show_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let changes = self.selected_changes();
        let [change] = changes.as_slice() else {
            return;
        };
        let Some(repository) = self.repository.as_ref() else {
            return;
        };
        let Some(project_path) = repository
            .read(cx)
            .repo_path_to_project_path(&change.path, cx)
        else {
            return;
        };
        self.workspace
            .update(cx, |workspace, cx| {
                git_ui_core::open_file_history(workspace, &project_path, window, cx)
            })
            .ok();
    }

    fn update_tab_properties(
        &mut self,
        update: impl FnOnce(&mut LogTabProperties),
        cx: &mut Context<Self>,
    ) {
        let mut properties = self.tab_properties.clone();
        update(&mut properties);
        cx.emit(ChangesBrowserEvent::TabPropertiesChanged(
            properties.clone(),
        ));
        self.set_properties(self.app_properties.clone(), properties, cx);
    }

    fn set_show_changes_from_parents(&mut self, show: bool, cx: &mut Context<Self>) {
        let mut properties = self.app_properties.clone();
        properties.show_changes_from_parents = show;
        cx.emit(ChangesBrowserEvent::AppPropertiesChanged(
            properties.clone(),
        ));
        self.set_properties(properties, self.tab_properties.clone(), cx);
    }

    fn run_status_action(&mut self, action: StatusAction, cx: &mut Context<Self>) {
        match action {
            StatusAction::ShowChangesToParents => self.set_show_changes_from_parents(true, cx),
            StatusAction::ShowAllPaths => self.update_tab_properties(
                |properties| properties.show_only_affected_changes = false,
                cx,
            ),
            StatusAction::ShowAnyway => {
                self.show_anyway = true;
                self.state = changes_state(&self.loaded, &self.parent_labels, true);
                self.rebuild(true, cx);
            }
        }
    }

    fn toggle_group_by_directory(&mut self, cx: &mut Context<Self>) {
        let mut properties = self.app_properties.clone();
        properties.changes_group_by_directory = !properties.changes_group_by_directory;
        cx.emit(ChangesBrowserEvent::AppPropertiesChanged(
            properties.clone(),
        ));
        self.set_properties(properties, self.tab_properties.clone(), cx);
    }

    fn regroup(&mut self, selected: Vec<Change>, cx: &mut Context<Self>) {
        self.expanded.clear();
        self.rebuild(false, cx);
        let mut ids = Vec::new();
        ids_of_changes(&self.tree, &selected, &mut ids);
        for id in &ids {
            if let Some(ancestors) = ancestors_of(&self.tree, id) {
                self.expanded.extend(ancestors);
            }
        }
        self.selection_anchor = ids.last().cloned();
        self.selection = ids;
        self.rows = flatten(&self.tree, &self.expanded);
        cx.notify();
    }

    fn expand_all(&mut self, cx: &mut Context<Self>) {
        all_container_ids(&self.tree, &mut self.expanded);
        self.refresh_rows(cx);
    }

    fn collapse_all(&mut self, cx: &mut Context<Self>) {
        let lead_top_level = self
            .selection
            .last()
            .and_then(|lead| {
                ancestors_of(&self.tree, lead).map(|ancestors| (lead.clone(), ancestors))
            })
            .map(|(lead, ancestors)| ancestors.first().cloned().unwrap_or(lead));
        self.expanded.clear();
        if let Some(top_level) = lead_top_level {
            self.selection = vec![top_level.clone()];
            self.selection_anchor = Some(top_level);
        }
        self.refresh_rows(cx);
    }

    fn set_node_expanded(&mut self, id: &SharedString, expand: bool, cx: &mut Context<Self>) {
        if expand {
            self.expanded.insert(id.clone());
        } else {
            self.expanded.remove(id);
        }
        self.refresh_rows(cx);
    }

    fn lead_index(&self) -> Option<usize> {
        let lead = self.selection.last()?;
        self.rows.iter().position(|row| &row.id == lead)
    }

    fn select_row(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        self.selection = vec![row.id.clone()];
        self.selection_anchor = Some(row.id.clone());
        self.scroll_handle
            .scroll_to_item(index, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn move_selection(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let index = match self.lead_index() {
            Some(index) if forward => (index + 1).min(self.rows.len() - 1),
            Some(index) => index.saturating_sub(1),
            None => 0,
        };
        self.select_row(index, cx);
    }

    fn select_parent(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.lead_index() else {
            return;
        };
        let Some(row) = self.rows.get(index).cloned() else {
            return;
        };
        if row.has_children && row.expanded {
            self.set_node_expanded(&row.id, false, cx);
            return;
        }
        if let Some(parent) = self.rows[..index]
            .iter()
            .rposition(|candidate| candidate.depth + 1 == row.depth)
        {
            self.select_row(parent, cx);
        }
    }

    fn select_child(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.lead_index() else {
            return;
        };
        let Some(row) = self.rows.get(index).cloned() else {
            return;
        };
        if row.has_children && !row.expanded {
            self.set_node_expanded(&row.id, true, cx);
        } else if row.has_children {
            self.select_row(index + 1, cx);
        }
    }

    fn row_mouse_down(
        &mut self,
        id: &SharedString,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        if event.click_count == 2 {
            let Some(row) = self.rows.iter().find(|row| &row.id == id).cloned() else {
                return;
            };
            if row.has_children {
                self.set_node_expanded(&row.id, !row.expanded, cx);
            } else {
                self.show_diff(window, cx);
            }
            return;
        }
        if event.modifiers.secondary() {
            if let Some(position) = self.selection.iter().position(|selected| selected == id) {
                self.selection.remove(position);
            } else {
                self.selection.push(id.clone());
            }
            self.selection_anchor = Some(id.clone());
        } else if event.modifiers.shift
            && let Some(anchor) = self.selection_anchor.clone()
        {
            let anchor_index = self.rows.iter().position(|row| row.id == anchor);
            let target_index = self.rows.iter().position(|row| &row.id == id);
            if let (Some(anchor_index), Some(target_index)) = (anchor_index, target_index) {
                let range = anchor_index.min(target_index)..=anchor_index.max(target_index);
                self.selection = self.rows[range].iter().map(|row| row.id.clone()).collect();
                if target_index < anchor_index {
                    self.selection.reverse();
                }
            }
        } else {
            self.selection = vec![id.clone()];
            self.selection_anchor = Some(id.clone());
        }
        cx.notify();
    }

    fn row_secondary_mouse_down(
        &mut self,
        id: &SharedString,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selection.contains(id) {
            self.selection = vec![id.clone()];
            self.selection_anchor = Some(id.clone());
        }
        self.deploy_context_menu(event.position, window, cx);
    }

    fn deploy_context_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let browser = cx.weak_entity();
        let show_changes_from_parents = self.app_properties.show_changes_from_parents;
        let can_show_history = self.selected_changes().len() == 1;
        let menu = ContextMenu::build(window, cx, move |menu, _, _| {
            let diff_browser = browser.clone();
            let history_browser = browser.clone();
            let parents_browser = browser;
            let mut menu = menu.entry("Show Diff", Some(Box::new(ShowDiff)), move |window, cx| {
                diff_browser
                    .update(cx, |browser, cx| browser.show_diff(window, cx))
                    .ok();
            });
            if can_show_history {
                menu = menu.entry(
                    "Show History for Revision",
                    Some(Box::new(ShowHistory)),
                    move |window, cx| {
                        history_browser
                            .update(cx, |browser, cx| browser.show_history(window, cx))
                            .ok();
                    },
                );
            }
            menu.separator().toggleable_entry(
                "Show Changes to Parents",
                show_changes_from_parents,
                IconPosition::Start,
                None,
                move |_, cx| {
                    parents_browser
                        .update(cx, |browser, cx| {
                            browser.set_show_changes_from_parents(!show_changes_from_parents, cx)
                        })
                        .ok();
                },
            )
        });
        window.focus(&menu.focus_handle(cx), cx);
        let subscription =
            cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
                this.context_menu.take();
                window.focus(&this.focus_handle, cx);
                cx.notify();
            });
        self.context_menu = Some((menu, position, subscription));
        cx.notify();
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_changes = !self.selected_or_all_changes().is_empty();
        let history_enabled = self.selected_changes().len() == 1;
        let has_affected_paths = !self.tab_properties.filters.paths.is_empty();
        let tree_actions_visible = self.app_properties.changes_group_by_directory;
        let has_rows = !self.rows.is_empty();
        h_flex()
            .flex_none()
            .w_full()
            .h(self.toolbar_height)
            .px(TOOLBAR_HORIZONTAL_INSET)
            .items_center()
            .justify_between()
            .child(
                h_flex()
                    .items_center()
                    .child(toolbar_button(
                        "git-log-changes-show-diff",
                        IconName::VcsDiff,
                        "Show Diff",
                        !has_changes,
                        false,
                        |this, window, cx| this.show_diff(window, cx),
                        cx,
                    ))
                    .child(toolbar_button(
                        "git-log-changes-revert",
                        IconName::VcsRevert,
                        "Revert Selected Changes",
                        true,
                        false,
                        |_, _, _| {},
                        cx,
                    ))
                    .child(toolbar_button(
                        "git-log-changes-history",
                        IconName::GeneralHistory,
                        "Show History for Revision",
                        !history_enabled,
                        false,
                        |this, window, cx| this.show_history(window, cx),
                        cx,
                    ))
                    .when(has_affected_paths, |this| {
                        this.child(toolbar_button(
                            "git-log-changes-only-affected",
                            IconName::NodesFolder,
                            "Show Only Affected Changes",
                            false,
                            self.tab_properties.show_only_affected_changes,
                            |this, _, cx| {
                                this.update_tab_properties(
                                    |properties| {
                                        properties.show_only_affected_changes =
                                            !properties.show_only_affected_changes
                                    },
                                    cx,
                                )
                            },
                            cx,
                        ))
                    })
                    .child(self.render_view_options(cx)),
            )
            .when(tree_actions_visible, |this| {
                this.child(
                    h_flex()
                        .items_center()
                        .child(toolbar_button(
                            "git-log-changes-expand-all",
                            IconName::ExpandAll,
                            "Expand All",
                            !has_rows,
                            false,
                            |this, _, cx| this.expand_all(cx),
                            cx,
                        ))
                        .child(toolbar_button(
                            "git-log-changes-collapse-all",
                            IconName::CollapseAll,
                            "Collapse All",
                            !has_rows,
                            false,
                            |this, _, cx| this.collapse_all(cx),
                            cx,
                        )),
                )
            })
    }

    fn render_view_options(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let browser = cx.weak_entity();
        let group_by_directory = self.app_properties.changes_group_by_directory;
        let show_details = self.tab_properties.show_details;
        h_flex()
            .flex_none()
            .size(TOOLBAR_BUTTON_SIZE)
            .justify_center()
            .child(
                PopoverMenu::new("git-log-changes-view-options")
                    .trigger_with_tooltip(
                        IconButton::new(
                            "git-log-changes-view-options-button",
                            IconName::GeneralShow,
                        )
                        .shape(IconButtonShape::Square)
                        .icon_size(IconSize::Medium),
                        Tooltip::text("View Options"),
                    )
                    .menu(move |window, cx| {
                        let browser = browser.clone();
                        Some(ContextMenu::build(window, cx, move |menu, _, _| {
                            let group_browser = browser.clone();
                            let details_browser = browser;
                            menu.header("Group By")
                                .toggleable_entry(
                                    "Directory",
                                    group_by_directory,
                                    IconPosition::Start,
                                    Some(Box::new(ToggleGroupByDirectory)),
                                    move |_, cx| {
                                        group_browser
                                            .update(cx, |browser, cx| {
                                                browser.toggle_group_by_directory(cx)
                                            })
                                            .ok();
                                    },
                                )
                                .separator()
                                .header("Layout")
                                .toggleable_entry(
                                    "Show Details",
                                    show_details,
                                    IconPosition::Start,
                                    None,
                                    move |_, cx| {
                                        details_browser
                                            .update(cx, |browser, cx| {
                                                browser.update_tab_properties(
                                                    |properties| {
                                                        properties.show_details =
                                                            !properties.show_details
                                                    },
                                                    cx,
                                                )
                                            })
                                            .ok();
                                    },
                                )
                        }))
                    })
                    .anchor(Anchor::TopLeft)
                    .attach(Anchor::BottomLeft),
            )
    }

    fn render_status(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.rows.is_empty() || self.loading_visible {
            return None;
        }
        let status = status_text(
            &self.state,
            self.tab_properties.show_only_affected_changes,
            !self.tab_properties.filters.paths.is_empty(),
        )?;
        let link = status.link.map(|(text, action)| {
            div()
                .id("git-log-changes-status-link")
                .cursor_pointer()
                .child(Label::new(text).color(Color::Accent))
                .on_click(cx.listener(move |this, _, _, cx| this.run_status_action(action, cx)))
        });
        Some(
            v_flex()
                .absolute()
                .size_full()
                .items_center()
                .child(div().flex_1())
                .child(
                    v_flex()
                        .items_center()
                        .gap(STATUS_LINE_GAP)
                        .child(Label::new(status.text).color(Color::Muted))
                        .children(link),
                )
                .child(div().flex_1())
                .child(div().flex_1())
                .into_any_element(),
        )
    }

    fn render_loading(&self) -> Option<AnyElement> {
        self.loading_visible.then(|| {
            h_flex()
                .absolute()
                .size_full()
                .justify_center()
                .items_center()
                .gap_1()
                .child(
                    Icon::new(IconName::ArrowCircle)
                        .size(IconSize::Medium)
                        .color(Color::Muted)
                        .with_rotate_animation(2),
                )
                .child(Label::new(LOADING_TEXT).color(Color::Muted))
                .into_any_element()
        })
    }

    fn render_tree(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let scrolled = self.scroll_handle.0.borrow().base_handle.offset().y < px(0.);
        div()
            .id("git-log-changes-tree")
            .key_context(TREE_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .border_t_1()
            .border_color(if scrolled {
                colors.border
            } else {
                gpui::transparent_black()
            })
            .on_scroll_wheel(cx.listener(|_, _: &ScrollWheelEvent, _, cx| cx.notify()))
            .on_action(
                cx.listener(|this, _: &menu::SelectNext, _, cx| this.move_selection(true, cx)),
            )
            .on_action(
                cx.listener(|this, _: &menu::SelectPrevious, _, cx| this.move_selection(false, cx)),
            )
            .on_action(cx.listener(|this, _: &menu::SelectFirst, _, cx| this.select_row(0, cx)))
            .on_action(cx.listener(|this, _: &menu::SelectLast, _, cx| {
                let last = this.rows.len().saturating_sub(1);
                this.select_row(last, cx)
            }))
            .on_action(cx.listener(|this, _: &menu::SelectParent, _, cx| this.select_parent(cx)))
            .on_action(cx.listener(|this, _: &menu::SelectChild, _, cx| this.select_child(cx)))
            .on_action(
                cx.listener(|this, _: &menu::Confirm, window, cx| this.show_diff(window, cx)),
            )
            .when(!self.rows.is_empty(), |this| {
                this.child(
                    uniform_list(
                        "git-log-changes-rows",
                        self.rows.len(),
                        cx.processor(Self::render_rows),
                    )
                    .size_full()
                    .track_scroll(&self.scroll_handle),
                )
            })
            .children(self.render_status(cx))
            .children(self.render_loading())
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let focused = self.focus_handle.contains_focused(window, cx);
        let colors = cx.theme().colors();
        let selection = if focused {
            colors.element_selected
        } else {
            colors
                .element_selected
                .grayscale()
                .opacity(INACTIVE_SELECTION_OPACITY)
        };
        let hover = colors.element_hover;
        range
            .filter_map(|index| {
                self.rows
                    .get(index)
                    .map(|row| self.render_row(index, row, selection, hover, cx))
            })
            .collect()
    }

    fn render_row(
        &self,
        index: usize,
        row: &TreeRow,
        selection: gpui::Hsla,
        hover: gpui::Hsla,
        cx: &Context<Self>,
    ) -> AnyElement {
        let selected = self.selection.contains(&row.id);
        let indent = TREE_LEFT_INSET + TREE_INDENT * row.depth as f32;
        let chevron = row.has_children.then(|| {
            let id = row.id.clone();
            let expand = !row.expanded;
            div()
                .id(("git-log-changes-chevron", index))
                .flex_none()
                .child(
                    Icon::new(if row.expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(IconSize::Medium)
                    .color(Color::Muted),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.set_node_expanded(&id, expand, cx);
                    }),
                )
        });
        let (icon, label, extra) = match &row.kind {
            NodeKind::Directory { label, .. } => (
                Some(Icon::new(IconName::NodesFolder).size(IconSize::Medium)),
                Label::new(label.clone()).single_line(),
                row.count_text.clone(),
            ),
            NodeKind::Change {
                change,
                name,
                parent_path,
            } => (
                Some(
                    FileIcons::get_icon(Path::new(change.path.as_unix_str()), cx)
                        .map(Icon::from_path)
                        .unwrap_or_else(|| Icon::new(IconName::File))
                        .size(IconSize::Medium),
                ),
                Label::new(name.clone())
                    .single_line()
                    .color(change_color(change.kind)),
                parent_path
                    .as_ref()
                    .map(|path| SharedString::from(format!(" \u{2009}{path}"))),
            ),
            NodeKind::Tag { label, .. } => (
                None,
                Label::new(label.clone()).single_line(),
                row.count_text.clone(),
            ),
        };
        let left_id = row.id.clone();
        let right_id = row.id.clone();
        div()
            .id(("git-log-changes-row", index))
            .relative()
            .h(ROW_HEIGHT)
            .w_full()
            .cursor_pointer()
            .when(!selected, |this| this.hover(|style| style.bg(hover)))
            .when(selected, |this| {
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(SELECTION_INSET)
                        .right(SELECTION_INSET)
                        .rounded(SELECTION_RADIUS)
                        .bg(selection),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.row_mouse_down(&left_id, event, window, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.row_secondary_mouse_down(&right_id, event, window, cx)
                }),
            )
            .child(
                h_flex()
                    .relative()
                    .h_full()
                    .pl(indent)
                    .child(div().flex_none().w(TREE_INDENT).children(chevron))
                    .child(
                        h_flex()
                            .min_w_0()
                            .pl(RENDERER_INSET)
                            .gap(ICON_TEXT_GAP)
                            .children(icon)
                            .child(label)
                            .children(
                                extra
                                    .map(|text| Label::new(text).single_line().color(Color::Muted)),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn change_color(kind: ChangeKind) -> Color {
    match kind {
        ChangeKind::Added => Color::VersionControlAdded,
        ChangeKind::Modified => Color::VersionControlModified,
        ChangeKind::Deleted => Color::VersionControlDeleted,
    }
}

fn toolbar_button(
    id: &'static str,
    icon: IconName,
    tooltip: &'static str,
    disabled: bool,
    toggled: bool,
    on_click: impl Fn(&mut ChangesBrowser, &mut Window, &mut Context<ChangesBrowser>) + 'static,
    cx: &Context<ChangesBrowser>,
) -> AnyElement {
    h_flex()
        .flex_none()
        .size(TOOLBAR_BUTTON_SIZE)
        .justify_center()
        .child(
            IconButton::new(id, icon)
                .shape(IconButtonShape::Square)
                .icon_size(IconSize::Medium)
                .toggle_state(toggled)
                .disabled(disabled)
                .tooltip(Tooltip::text(tooltip))
                .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx))),
        )
        .into_any_element()
}

async fn commit_data(
    repository: &WeakEntity<Repository>,
    sha: Oid,
    cx: &mut AsyncApp,
) -> Result<Arc<CommitData>> {
    let state = repository.update(cx, |repository, cx| {
        repository.fetch_commit_data(sha, true, cx).clone()
    })?;
    match state {
        CommitDataState::Loaded(data) => Ok(data),
        CommitDataState::Loading(Some(receiver)) => receiver
            .await
            .map_err(|_| anyhow!("commit data for {sha} was not loaded")),
        CommitDataState::Loading(None) => Err(anyhow!("commit data for {sha} was not requested")),
    }
}

fn tree_diff_changes(diff: TreeDiff, commit: Oid) -> Vec<Change> {
    let mut changes: Vec<Change> = diff
        .entries
        .into_iter()
        .map(|(path, status)| Change {
            path,
            kind: match status {
                TreeDiffStatus::Added => ChangeKind::Added,
                TreeDiffStatus::Modified { .. } => ChangeKind::Modified,
                TreeDiffStatus::Deleted { .. } => ChangeKind::Deleted,
            },
            commit,
        })
        .collect();
    changes.sort_by(|left, right| left.path.as_unix_str().cmp(right.path.as_unix_str()));
    changes
}

async fn tree_diff(
    repository: &WeakEntity<Repository>,
    base: String,
    commit: Oid,
    cx: &mut AsyncApp,
) -> Result<Vec<Change>> {
    let receiver = repository.update(cx, |repository, cx| {
        repository.diff_tree(
            DiffTreeType::Since {
                base: base.into(),
                head: commit.to_string().into(),
            },
            cx,
        )
    })?;
    Ok(tree_diff_changes(receiver.await??, commit))
}

async fn load_commit_changes(
    repository: WeakEntity<Repository>,
    commits: Vec<Oid>,
    cx: &mut AsyncApp,
) -> Result<(Vec<CommitChanges>, HashMap<Oid, SharedString>)> {
    let mut loaded = Vec::with_capacity(commits.len());
    let mut parent_labels = HashMap::default();
    for commit in commits {
        let parents = commit_data(&repository, commit, cx).await?.parents.to_vec();
        if parents.len() > 1 {
            let mut changes_to_parents = Vec::with_capacity(parents.len());
            for parent in parents {
                let changes = tree_diff(&repository, parent.to_string(), commit, cx).await?;
                let subject = commit_data(&repository, parent, cx)
                    .await
                    .ok()
                    .map(|data| data.subject.clone());
                parent_labels.insert(parent, parent_label(parent, subject.as_deref()));
                changes_to_parents.push((parent, changes));
            }
            let lists: Vec<Vec<Change>> = changes_to_parents
                .iter()
                .map(|(_, changes)| changes.clone())
                .collect();
            loaded.push(CommitChanges {
                changes: merge_parent_changes(&lists),
                changes_to_parents,
            });
            continue;
        }
        let diff_task = repository.update(cx, |repository, cx| {
            repository.load_commit_diff(commit.to_string(), false, cx)
        })?;
        let diff = diff_task.await?;
        let changes = if diff.is_shallow_boundary {
            tree_diff(&repository, EMPTY_TREE_SHA.to_string(), commit, cx).await?
        } else {
            diff.files
                .into_iter()
                .map(|file| Change {
                    kind: match (&file.old_text, &file.new_text) {
                        (None, Some(_)) => ChangeKind::Added,
                        (Some(_), None) => ChangeKind::Deleted,
                        _ => ChangeKind::Modified,
                    },
                    path: file.path,
                    commit,
                })
                .collect()
        };
        loaded.push(CommitChanges {
            changes,
            changes_to_parents: Vec::new(),
        });
    }
    Ok((loaded, parent_labels))
}

impl EventEmitter<ChangesBrowserEvent> for ChangesBrowser {}

impl Focusable for ChangesBrowser {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ChangesBrowser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        v_flex()
            .key_context(KEY_CONTEXT)
            .size_full()
            .bg(ui::content_background(colors.panel_background, window))
            .on_action(cx.listener(|this, _: &ShowDiff, window, cx| this.show_diff(window, cx)))
            .on_action(
                cx.listener(|this, _: &ShowHistory, window, cx| this.show_history(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ExpandAll, _, cx| this.expand_all(cx)))
            .on_action(cx.listener(|this, _: &CollapseAll, _, cx| this.collapse_all(cx)))
            .on_action(cx.listener(|this, _: &ToggleGroupByDirectory, _, cx| {
                this.toggle_group_by_directory(cx)
            }))
            .child(self.render_toolbar(cx))
            .child(self.render_tree(cx))
            .children(self.context_menu.as_ref().map(|(menu, position, _)| {
                deferred(
                    anchored()
                        .position(*position)
                        .anchor(Anchor::TopLeft)
                        .child(menu.clone()),
                )
                .with_priority(1)
            }))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{TestAppContext, VisualTestContext};
    use settings::SettingsStore;

    use super::*;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
    }

    #[gpui::test]
    async fn empty_selection_and_property_toggles_follow_intellij(cx: &mut TestAppContext) {
        init_test(cx);
        let window =
            cx.add_window(|window, cx| ChangesBrowser::new(WeakEntity::new_invalid(), window, cx));
        let browser = window.root(cx).expect("the window has a changes browser");
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        let events: Rc<RefCell<Vec<ChangesBrowserEvent>>> = Rc::default();
        cx.update(|_, cx| {
            let events = events.clone();
            cx.subscribe(&browser, move |_, event: &ChangesBrowserEvent, _| {
                events.borrow_mut().push(event.clone());
            })
            .detach();
        });

        browser.update(&mut cx, |browser, cx| {
            browser.set_commits(None, Vec::new(), cx)
        });
        let status = browser.read_with(&cx, |browser, _| {
            status_text(&browser.state, false, false).map(|status| status.text)
        });
        assert_eq!(status, Some(model::SELECT_COMMIT_TEXT.into()));

        browser.update(&mut cx, |browser, cx| {
            browser.run_status_action(StatusAction::ShowChangesToParents, cx);
            browser.run_status_action(StatusAction::ShowAllPaths, cx);
        });
        let recorded = events.borrow().clone();
        assert!(matches!(
            recorded.as_slice(),
            [
                ChangesBrowserEvent::AppPropertiesChanged(app),
                ChangesBrowserEvent::TabPropertiesChanged(tab),
            ] if app.show_changes_from_parents && !tab.show_only_affected_changes
        ));
    }
}
