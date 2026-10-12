mod matcher;
mod model;

use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use collections::HashSet;
use editor::{Editor, EditorEvent};
use git::repository::{Branch, FetchOptions, Tag, Upstream};
use gpui::{
    Anchor, AnyElement, AnyView, App, Bounds, Context, DismissEvent, Entity, EventEmitter,
    FocusHandle, Focusable, Hsla, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point,
    ScrollStrategy, ShapedLine, SharedString, Subscription, Task, TransformationMatrix,
    UniformListScrollHandle, WeakEntity, Window, actions, anchored, canvas, deferred, point, px,
    radians, rgb, uniform_list,
};
use project::git_store::{Repository, RepositoryEvent};
use ui::{
    CommonAnimationExt as _, ContextMenu, ContextMenuEntry, HighlightedLabel, IconButton,
    IconButtonShape, PopoverMenu, TextSize, Tooltip, prelude::*,
};
use util::ResultExt as _;
use workspace::Workspace;

use self::matcher::{BranchesTreeMatcher, Matching};
use self::model::{ModelInput, Node, NodeKind, RefInfo, TreeRow};
use super::properties::BranchesSingleClickAction;
use crate::branch_operations::compare_view::{CompareRefs, CompareView};
use crate::branch_operations::{BranchContext, BranchNotice, checkout, compare, integrate, manage};
use crate::branch_refs::{RefKind, RefTarget};
use crate::branches_popup::ref_menu::{self, MenuContext, MenuRunner};
use crate::branches_popup::state::{self, PopupState, SavedPopupStates};
use crate::branches_popup::tree;
use crate::remote_output::{RemoteAction, format_output};

actions!(
    git_log_branches,
    [
        ToggleSearchFocus,
        ExpandAll,
        CollapseAll,
        CompareWithCurrent,
        DeleteSelected,
        NavigateToSelectedBranch,
        ToggleFavorite
    ]
);

const KEY_CONTEXT: &str = "GitLogBranches";
const TREE_KEY_CONTEXT: &str = "GitLogBranchesTree menu";
const SEARCH_KEY_CONTEXT: &str = "GitLogBranchesSearch";
const SEARCH_PLACEHOLDER: &str = "Branch or tag";
const LOADING_TEXT: &str = "Loading Branches…";
const EMPTY_TEXT: &str = "Nothing to show";
const STRIPE_LABEL: &str = "Branches";
const NO_TRACKING_BRANCH: &str = "No tracking branch";
const PUNCTUATION_MARKS: &str = "*_-+\"'/.#$>: ,;?!@%^&";
const FILTER_DEBOUNCE: Duration = Duration::from_millis(100);
const ROW_HEIGHT: Pixels = px(24.);
const TREE_LEFT_INSET: Pixels = px(12.);
const TREE_INDENT: Pixels = px(18.);
const RENDERER_INSET: Pixels = px(3.);
const ICON_TEXT_GAP: Pixels = px(2.);
const COUNTER_GAP: Pixels = px(3.);
const COUNTER_ICON_GAP: Pixels = px(1.);
pub(crate) const TOOLBAR_WIDTH: Pixels = px(36.);
const TOOLBAR_BUTTON_SIZE: Pixels = px(22.);
const TOOLBAR_VERTICAL_INSET: Pixels = px(5.);
const TOOLBAR_SEPARATOR_HEIGHT: Pixels = px(7.);
const TOOLBAR_SEPARATOR_INSET: Pixels = px(2.);
const STRIPE_ICON_TOP: Pixels = px(8.);
const STRIPE_ICON_SIZE: Pixels = px(16.);
const STRIPE_LABEL_TOP: Pixels = px(34.);
const SELECTION_INSET: Pixels = px(4.);
const SELECTION_RADIUS: Pixels = px(6.);
const PROGRESS_STRIPE_HEIGHT: Pixels = px(2.);
const DEFAULT_TOOLBAR_HEIGHT: Pixels = px(40.);
const INACTIVE_SELECTION_OPACITY: f32 = 0.5;
const MAX_COMMIT_COUNTER: u32 = 99;
const SHORT_SHA_LENGTH: usize = 8;
const REMOTE_HEAD_NAME: &str = "HEAD";
const INCOMING_LIGHT: u32 = 0x3574F0;
const INCOMING_DARK: u32 = 0x548AF7;
const OUTGOING_LIGHT: u32 = 0x369650;
const OUTGOING_DARK: u32 = 0x57965C;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum BranchesPaneEvent {
    FilterByRevisions(Vec<SharedString>),
    NavigateToRevision(SharedString),
    ExpandedChanged(bool),
    SingleClickActionChanged(BranchesSingleClickAction),
}

#[derive(Default)]
struct PaneData {
    local: Vec<Branch>,
    remote: Vec<Branch>,
    tags: Vec<Tag>,
    remote_names: Vec<SharedString>,
    current_branch: Option<SharedString>,
    head_sha: Option<SharedString>,
    has_commits: bool,
}

struct RowStyle {
    selection: Hsla,
    hover: Hsla,
    incoming: Hsla,
    outgoing: Hsla,
}

pub(crate) struct BranchesPane {
    workspace: WeakEntity<Workspace>,
    repository: Option<Entity<Repository>>,
    focus_handle: FocusHandle,
    search_editor: Entity<Editor>,
    scroll_handle: UniformListScrollHandle,
    expanded: bool,
    single_click: BranchesSingleClickAction,
    toolbar_height: Pixels,
    data: PaneData,
    shared_state: PopupState,
    shared_state_key: Option<String>,
    shared_state_loaded: bool,
    tree: Vec<Node>,
    visible: Vec<Node>,
    rows: Vec<TreeRow>,
    expanded_nodes: HashSet<SharedString>,
    tree_state_touched: bool,
    selection: Vec<SharedString>,
    selection_anchor: Option<SharedString>,
    query: String,
    applied_query: String,
    no_hits: bool,
    loading: bool,
    fetching: bool,
    context_menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
    _filter_task: Task<()>,
    _load_task: Task<()>,
    _state_task: Task<()>,
    save_task: Task<()>,
    _repository_subscription: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl BranchesPane {
    pub(crate) fn new(
        workspace: WeakEntity<Workspace>,
        repository: Option<Entity<Repository>>,
        expanded: bool,
        single_click: BranchesSingleClickAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text(SEARCH_PLACEHOLDER, window, cx);
            editor
        });
        let editor_subscription = cx.subscribe_in(
            &search_editor,
            window,
            |this, _, event: &EditorEvent, window, cx| {
                if let EditorEvent::BufferEdited = event {
                    this.schedule_filter(window, cx);
                }
            },
        );
        let state_subscription =
            cx.observe_global::<SavedPopupStates>(|this, cx| this.apply_saved_shared_state(cx));
        let mut pane = Self {
            workspace,
            repository: None,
            focus_handle: cx.focus_handle(),
            search_editor,
            scroll_handle: UniformListScrollHandle::new(),
            expanded,
            single_click,
            toolbar_height: DEFAULT_TOOLBAR_HEIGHT,
            data: PaneData::default(),
            shared_state: PopupState::default(),
            shared_state_key: None,
            shared_state_loaded: false,
            tree: Vec::new(),
            visible: Vec::new(),
            rows: Vec::new(),
            expanded_nodes: HashSet::default(),
            tree_state_touched: false,
            selection: Vec::new(),
            selection_anchor: None,
            query: String::new(),
            applied_query: String::new(),
            no_hits: false,
            loading: false,
            fetching: false,
            context_menu: None,
            _filter_task: Task::ready(()),
            _load_task: Task::ready(()),
            _state_task: Task::ready(()),
            save_task: Task::ready(()),
            _repository_subscription: None,
            _subscriptions: vec![editor_subscription, state_subscription],
        };
        pane.set_repository(repository, window, cx);
        pane
    }

    pub(crate) fn set_repository(
        &mut self,
        repository: Option<Entity<Repository>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        std::mem::replace(&mut self.save_task, Task::ready(())).detach();
        self.repository = repository;
        self.data = PaneData::default();
        self.selection.clear();
        self.selection_anchor = None;
        self.expanded_nodes.clear();
        self.tree_state_touched = false;
        self.shared_state = PopupState::default();
        self.shared_state_key = None;
        self.shared_state_loaded = false;
        self._repository_subscription = None;
        self._load_task = Task::ready(());
        self._state_task = Task::ready(());
        self.loading = false;
        let Some(repository) = self.repository.clone() else {
            self.rebuild(cx);
            cx.notify();
            return;
        };
        self._repository_subscription = Some(cx.subscribe_in(
            &repository,
            window,
            |this, _, event: &RepositoryEvent, window, cx| {
                if matches!(
                    event,
                    RepositoryEvent::BranchListChanged | RepositoryEvent::HeadChanged
                ) {
                    this.reload(window, cx);
                }
            },
        ));
        let key = state::serialization_key(&repository.read(cx).common_dir_abs_path);
        self.shared_state_key = Some(key.clone());
        let saved = cx
            .try_global::<SavedPopupStates>()
            .and_then(|states| states.get(&key))
            .cloned();
        match saved {
            Some(saved) => {
                self.shared_state = PopupState::from_persisted(saved);
                self.shared_state_loaded = true;
            }
            None => {
                let load = state::load(key, cx);
                self._state_task = cx.spawn(async move |this, cx| {
                    let persisted = load.await;
                    this.update(cx, |this, cx| {
                        if this.shared_state_loaded {
                            return;
                        }
                        if let Some(persisted) = persisted {
                            this.shared_state = PopupState::from_persisted(persisted);
                        }
                        this.shared_state_loaded = true;
                        this.rebuild(cx);
                        cx.notify();
                    })
                    .ok();
                });
            }
        }
        self.reload(window, cx);
    }

    pub(crate) fn set_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        self.expanded = expanded;
        cx.notify();
    }

    pub(crate) fn is_expanded(&self) -> bool {
        self.expanded
    }

    pub(crate) fn set_single_click_action(
        &mut self,
        action: BranchesSingleClickAction,
        cx: &mut Context<Self>,
    ) {
        self.single_click = action;
        cx.notify();
    }

    pub(crate) fn set_toolbar_height(&mut self, height: Pixels, cx: &mut Context<Self>) {
        self.toolbar_height = height;
        cx.notify();
    }

    pub(crate) fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.search_editor.focus_handle(cx);
        window.focus(&handle, cx);
    }

    fn hide_branches(&mut self, cx: &mut Context<Self>) {
        self.expanded = false;
        cx.emit(BranchesPaneEvent::ExpandedChanged(false));
        cx.notify();
    }

    fn expand_from_stripe(&mut self, cx: &mut Context<Self>) {
        self.expanded = true;
        cx.emit(BranchesPaneEvent::ExpandedChanged(true));
        cx.notify();
    }

    fn toggle_single_click(&mut self, action: BranchesSingleClickAction, cx: &mut Context<Self>) {
        self.single_click = if self.single_click == action {
            BranchesSingleClickAction::None
        } else {
            action
        };
        cx.emit(BranchesPaneEvent::SingleClickActionChanged(
            self.single_click,
        ));
        cx.notify();
    }

    fn branch_context(&self) -> Option<BranchContext> {
        self.repository
            .clone()
            .map(|repository| BranchContext::new(self.workspace.clone(), repository))
    }

    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else {
            return;
        };
        self.refresh_snapshot(cx);
        self.loading = true;
        let (tags, remotes) = repository.update(cx, |repository, _| {
            (repository.tags(), repository.get_remotes(None, false))
        });
        self._load_task = cx.spawn_in(window, async move |this, cx| {
            let tags = tags.await.ok().and_then(|result| result.log_err());
            let remotes = remotes.await.ok().and_then(|result| result.log_err());
            this.update_in(cx, |this, _, cx| {
                if let Some(tags) = tags {
                    this.data.tags = tags;
                }
                if let Some(remotes) = remotes {
                    this.data.remote_names =
                        remotes.into_iter().map(|remote| remote.name).collect();
                }
                this.loading = false;
                this.rebuild(cx);
                cx.notify();
            })
            .ok();
        });
        self.rebuild(cx);
        cx.notify();
    }

    fn refresh_snapshot(&mut self, cx: &App) {
        let Some(repository) = &self.repository else {
            return;
        };
        let snapshot = repository.read(cx);
        self.data.local = snapshot
            .branch_list
            .iter()
            .filter(|branch| !branch.is_remote())
            .cloned()
            .collect();
        self.data.remote = snapshot
            .branch_list
            .iter()
            .filter(|branch| branch.is_remote())
            .cloned()
            .collect();
        self.data.current_branch = snapshot
            .branch
            .as_ref()
            .map(|branch| SharedString::from(branch.name().to_string()));
        self.data.head_sha = snapshot
            .head_commit
            .as_ref()
            .map(|commit| commit.sha.clone());
        self.data.has_commits = snapshot.head_commit.is_some();
    }

    fn favorite_defaults(&self) -> collections::HashSet<RefTarget> {
        tree::default_favorites(&self.data.local, &self.data.remote)
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        if self.repository.is_none() {
            self.tree.clear();
        } else {
            let defaults = self.favorite_defaults();
            let current = self.data.current_branch.clone();
            let local: Vec<RefInfo> = self
                .data
                .local
                .iter()
                .map(|branch| {
                    let target = RefTarget::local(branch.name().to_string());
                    let status = branch.tracking_status();
                    RefInfo {
                        is_current: current.as_deref() == Some(branch.name()),
                        is_favorite: self.shared_state.is_favorite(&target, &defaults),
                        incoming: status.map_or(0, |status| status.behind),
                        outgoing: status.map_or(0, |status| status.ahead),
                        tracked: branch.upstream.as_ref().map(tracked_name),
                        ..RefInfo::new(target)
                    }
                })
                .collect();
            let remote: Vec<RefInfo> = self
                .data
                .remote
                .iter()
                .filter(|branch| !is_remote_head(branch.name()))
                .map(|branch| {
                    let target = RefTarget::remote(branch.name().to_string());
                    RefInfo {
                        remote: Some(remote_name_of(branch.name(), &self.data.remote_names)),
                        is_favorite: self.shared_state.is_favorite(&target, &defaults),
                        ..RefInfo::new(target)
                    }
                })
                .collect();
            let tags: Vec<RefInfo> = if self.shared_state.toggles.show_tags {
                self.data
                    .tags
                    .iter()
                    .map(|tag| {
                        let target = RefTarget::tag(tag.name.clone());
                        RefInfo {
                            is_current: current.is_none()
                                && self.data.head_sha.as_ref() == Some(&tag.commit_sha),
                            is_favorite: self.shared_state.is_favorite(&target, &defaults),
                            ..RefInfo::new(target)
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };
            self.tree = model::build_nodes(&ModelInput {
                local: &local,
                remote: &remote,
                tags: &tags,
                group_by_directory: self.shared_state.toggles.group_by_directory,
            });
        }
        self.apply_tree();
        cx.notify();
    }

    fn apply_tree(&mut self) -> Option<(i32, SharedString)> {
        let filtering = !self.applied_query.is_empty();
        let best_match = if filtering {
            let outcome =
                model::filter_nodes(&self.tree, &BranchesTreeMatcher::new(&self.applied_query));
            self.visible = outcome.nodes;
            self.no_hits = outcome.no_hits;
            outcome.best_match
        } else {
            self.visible = self.tree.clone();
            self.no_hits = false;
            None
        };
        if filtering || !self.tree_state_touched {
            let expand_everything =
                filtering || model::count_nodes(&self.visible) <= model::MANY_NODES_THRESHOLD;
            if expand_everything {
                let mut ids = Vec::new();
                model::container_ids(&self.visible, &mut ids);
                self.expanded_nodes.extend(ids);
            } else {
                self.expanded_nodes.clear();
            }
        }
        self.rows = model::flatten(&self.visible, &self.expanded_nodes);
        let rows = &self.rows;
        self.selection
            .retain(|id| rows.iter().any(|row| &row.id == id));
        best_match
    }

    fn schedule_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query = self.search_editor.read(cx).text(cx);
        self._filter_task = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(FILTER_DEBOUNCE).await;
            this.update_in(cx, |this, _, cx| this.refilter(cx)).ok();
        });
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let pattern_changed = self.applied_query != self.query;
        self.applied_query = self.query.clone();
        let previous_selection = self.selection.clone();
        let best_match = self.apply_tree();
        let matcher = BranchesTreeMatcher::new(&self.applied_query);
        match best_match {
            Some((degree, id)) => {
                if pattern_changed {
                    let selection_degree = self
                        .selection
                        .first()
                        .and_then(|id| self.row(id))
                        .and_then(|row| row.match_text.clone())
                        .map_or(i32::MIN, |text| matcher.degree_of(&text));
                    if selection_degree < degree {
                        self.select_only(id);
                    }
                }
            }
            None => self.select_default_match(&matcher),
        }
        if !self.applied_query.trim().is_empty() {
            self.scroll_to_lead();
        }
        if self.selection != previous_selection {
            self.selection_changed(cx);
        }
        cx.notify();
    }

    fn select_default_match(&mut self, matcher: &BranchesTreeMatcher) {
        let current = self.selection.first().cloned();
        let current_matching = current
            .as_ref()
            .map_or(Matching::None, |id| self.row_matching(id, matcher));
        if current_matching == Matching::Full {
            return;
        }
        if let Some(id) = self.next_matching_node(current.as_ref(), matcher, true) {
            self.select_only(id);
        } else if current_matching == Matching::None
            && let Some(id) = self.next_matching_node(current.as_ref(), matcher, false)
        {
            self.select_only(id);
        }
    }

    fn row(&self, id: &SharedString) -> Option<&TreeRow> {
        self.rows.iter().find(|row| &row.id == id)
    }

    fn row_matching(&self, id: &SharedString, matcher: &BranchesTreeMatcher) -> Matching {
        model::find_node(&self.visible, id).map_or(Matching::None, |node| {
            model::node_matching(&node.kind, node.match_text().as_deref(), matcher)
        })
    }

    fn next_matching_node(
        &self,
        start: Option<&SharedString>,
        matcher: &BranchesTreeMatcher,
        full: bool,
    ) -> Option<SharedString> {
        let mut ordered = Vec::new();
        model::preorder_nodes(&self.visible, &mut ordered);
        let start_index = start.and_then(|id| ordered.iter().position(|node| &node.id == id));
        let count = ordered.len();
        let offset = start_index.map_or(0, |index| index + 1);
        (0..count)
            .map(|step| (offset + step) % count)
            .filter(|index| Some(*index) != start_index)
            .filter_map(|index| ordered.get(index))
            .find(|node| {
                let matching =
                    model::node_matching(&node.kind, node.match_text().as_deref(), matcher);
                if full {
                    matching == Matching::Full
                } else {
                    matching != Matching::None
                }
            })
            .map(|node| node.id.clone())
    }

    fn select_only(&mut self, id: SharedString) {
        self.selection = vec![id.clone()];
        self.selection_anchor = Some(id);
    }

    fn lead_index(&self) -> Option<usize> {
        let lead = self.selection.last()?;
        self.rows.iter().position(|row| &row.id == lead)
    }

    fn scroll_to_lead(&self) {
        if let Some(index) = self.lead_index() {
            self.scroll_handle
                .scroll_to_item(index, ScrollStrategy::Nearest);
        }
    }

    fn selected_rows(&self) -> Vec<&TreeRow> {
        self.rows
            .iter()
            .filter(|row| self.selection.contains(&row.id))
            .collect()
    }

    fn selected_kinds(&self) -> Vec<NodeKind> {
        self.selected_rows()
            .into_iter()
            .map(|row| row.kind.clone())
            .collect()
    }

    fn selected_refs(&self) -> Vec<RefInfo> {
        self.selected_rows()
            .into_iter()
            .filter_map(|row| match &row.kind {
                NodeKind::Ref(info) => Some(info.clone()),
                _ => None,
            })
            .collect()
    }

    fn selected_branches(&self) -> Vec<RefInfo> {
        self.selected_refs()
            .into_iter()
            .filter(RefInfo::is_branch)
            .collect()
    }

    fn head_selected(&self) -> bool {
        self.selected_rows()
            .iter()
            .any(|row| row.kind == NodeKind::Head)
    }

    fn selection_changed(&mut self, cx: &mut Context<Self>) {
        if !self.expanded {
            return;
        }
        match self.single_click {
            BranchesSingleClickAction::UpdateBranchFilter => {
                let filters = model::branch_filters(&self.selected_kinds());
                cx.emit(BranchesPaneEvent::FilterByRevisions(filters));
            }
            BranchesSingleClickAction::NavigateLog => {
                if let Some(revision) = model::navigatable_revision(&self.selected_kinds()) {
                    cx.emit(BranchesPaneEvent::NavigateToRevision(revision));
                }
            }
            BranchesSingleClickAction::None => {}
        }
    }

    fn select_row(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        let id = row.id.clone();
        self.tree_state_touched = true;
        let changed = self.selection != [id.clone()];
        self.select_only(id);
        self.scroll_handle
            .scroll_to_item(index, ScrollStrategy::Nearest);
        if changed {
            self.selection_changed(cx);
        }
        cx.notify();
    }

    fn click_row(&mut self, id: &SharedString, event: &MouseDownEvent, cx: &mut Context<Self>) {
        self.tree_state_touched = true;
        let previous = self.selection.clone();
        if event.modifiers.secondary() {
            if let Some(position) = self.selection.iter().position(|selected| selected == id) {
                self.selection.remove(position);
            } else {
                self.selection.push(id.clone());
            }
            self.selection_anchor = Some(id.clone());
        } else if event.modifiers.shift
            && let Some(anchor) = self.selection_anchor.clone()
            && let Some(anchor_index) = self.rows.iter().position(|row| row.id == anchor)
            && let Some(index) = self.rows.iter().position(|row| &row.id == id)
        {
            let range = anchor_index.min(index)..=anchor_index.max(index);
            let mut selection: Vec<SharedString> = self.rows[range]
                .iter()
                .map(|row| row.id.clone())
                .filter(|row_id| row_id != id)
                .collect();
            selection.push(id.clone());
            self.selection = selection;
        } else {
            self.select_only(id.clone());
        }
        if self.selection != previous {
            self.selection_changed(cx);
        }
        cx.notify();
    }

    fn row_mouse_down(
        &mut self,
        id: &SharedString,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        let modified = event.modifiers.shift || event.modifiers.secondary();
        if event.click_count >= 2 && !modified {
            if self.row(id).is_some_and(|row| row.has_children) {
                let expand = !self.expanded_nodes.contains(id);
                self.set_node_expanded(id, expand, cx);
            } else {
                self.update_branch_filter(cx);
            }
            return;
        }
        self.click_row(id, event, cx);
    }

    fn row_secondary_mouse_down(
        &mut self,
        id: &SharedString,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        if !self.selection.contains(id) {
            self.tree_state_touched = true;
            self.select_only(id.clone());
            self.selection_changed(cx);
        }
        self.deploy_context_menu(event.position, window, cx);
    }

    fn set_node_expanded(&mut self, id: &SharedString, expand: bool, cx: &mut Context<Self>) {
        self.tree_state_touched = true;
        let Some(node) = model::find_node(&self.visible, id) else {
            return;
        };
        if expand {
            self.expanded_nodes.insert(node.id.clone());
            let mut current = node;
            while let [only_child] = current.children.as_slice() {
                if only_child.children.is_empty() {
                    break;
                }
                self.expanded_nodes.insert(only_child.id.clone());
                current = only_child;
            }
        } else {
            let mut descendants = Vec::new();
            model::container_ids(&node.children, &mut descendants);
            self.expanded_nodes.remove(id);
            for descendant in descendants {
                self.expanded_nodes.remove(&descendant);
            }
            let prefix = format!("{id}/");
            let had_hidden_selection = self
                .selection
                .iter()
                .any(|selected| selected.starts_with(&prefix));
            if had_hidden_selection {
                self.selection
                    .retain(|selected| !selected.starts_with(&prefix));
                if !self.selection.contains(id) {
                    self.selection.push(id.clone());
                }
            }
        }
        self.rows = model::flatten(&self.visible, &self.expanded_nodes);
        cx.notify();
    }

    fn expand_all(&mut self, cx: &mut Context<Self>) {
        self.tree_state_touched = true;
        let mut ids = Vec::new();
        model::container_ids(&self.visible, &mut ids);
        self.expanded_nodes.extend(ids);
        self.rows = model::flatten(&self.visible, &self.expanded_nodes);
        self.scroll_to_lead();
        cx.notify();
    }

    fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.tree_state_touched = true;
        self.expanded_nodes.clear();
        let lead_top_level = self.selection.last().and_then(|lead| {
            self.visible
                .iter()
                .find(|node| node.id == *lead || lead.starts_with(&format!("{}/", node.id)))
                .map(|node| node.id.clone())
        });
        self.rows = model::flatten(&self.visible, &self.expanded_nodes);
        if let Some(top_level) = lead_top_level {
            self.select_only(top_level);
        } else {
            self.selection.clear();
        }
        self.scroll_to_lead();
        cx.notify();
    }

    fn move_selection(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let current = self.lead_index();
        let target = if self.applied_query.is_empty() {
            Some(match current {
                Some(index) if forward => (index + 1).min(self.rows.len() - 1),
                Some(index) => index.saturating_sub(1),
                None => 0,
            })
        } else {
            let matcher = BranchesTreeMatcher::new(&self.applied_query);
            let count = self.rows.len();
            (1..=count)
                .map(|step| match current {
                    Some(index) if forward => (index + step) % count,
                    Some(index) => (index + count - step % count) % count,
                    None if forward => (step - 1) % count,
                    None => count - step,
                })
                .find(|index| {
                    self.rows.get(*index).is_some_and(|row| {
                        model::node_matching(&row.kind, row.match_text.as_deref(), &matcher)
                            != Matching::None
                    })
                })
        };
        if let Some(target) = target {
            self.select_row(target, cx);
        }
    }

    fn select_parent(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.lead_index() else {
            return;
        };
        let Some(row) = self.rows.get(index) else {
            return;
        };
        if row.has_children && row.expanded {
            let id = row.id.clone();
            self.set_node_expanded(&id, false, cx);
            return;
        }
        let depth = row.depth;
        if depth == 0 {
            return;
        }
        if let Some(parent) = self.rows[..index]
            .iter()
            .rposition(|candidate| candidate.depth + 1 == depth)
        {
            self.select_row(parent, cx);
        }
    }

    fn select_child(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.lead_index() else {
            return;
        };
        let Some(row) = self.rows.get(index) else {
            return;
        };
        if !row.has_children {
            return;
        }
        if row.expanded {
            self.select_row(index + 1, cx);
        } else {
            let id = row.id.clone();
            self.set_node_expanded(&id, true, cx);
        }
    }

    fn update_branch_filter(&mut self, cx: &mut Context<Self>) {
        let filters = model::branch_filters(&self.selected_kinds());
        if !filters.is_empty() {
            cx.emit(BranchesPaneEvent::FilterByRevisions(filters));
        }
    }

    fn navigate_to_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(revision) = model::navigatable_revision(&self.selected_kinds()) {
            cx.emit(BranchesPaneEvent::NavigateToRevision(revision));
        }
    }

    fn set_query_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.search_editor.update(cx, |editor, cx| {
            editor.set_text(text, window, cx);
        });
    }

    fn tree_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
            return;
        }
        if keystroke.key == "backspace" {
            if self.query.is_empty() {
                return;
            }
            let mut text = self.query.clone();
            text.pop();
            self.set_query_text(text, window, cx);
            cx.stop_propagation();
            return;
        }
        let Some(typed) = keystroke.key_char.as_deref() else {
            return;
        };
        let mut characters = typed.chars();
        let (Some(character), None) = (characters.next(), characters.next()) else {
            return;
        };
        let starts_with_whitespace = self.query.is_empty() && character.is_whitespace();
        let accepted = character.is_alphanumeric()
            || (!starts_with_whitespace && PUNCTUATION_MARKS.contains(character));
        if accepted {
            let mut text = self.query.clone();
            text.push(character);
            self.set_query_text(text, window, cx);
            cx.stop_propagation();
        }
    }

    fn tree_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.query.is_empty() {
            cx.propagate();
            return;
        }
        self.set_query_text(String::new(), window, cx);
    }

    fn toggle_search_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus_handle.contains_focused(window, cx) {
            self.focus_search(window, cx);
        } else {
            window.focus(&self.focus_handle, cx);
        }
    }

    fn search_moves_into_tree(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        self.move_selection(forward, cx);
    }

    fn search_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.query.is_empty() {
            window.focus(&self.focus_handle, cx);
        } else {
            cx.propagate();
        }
    }

    fn apply_saved_shared_state(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.shared_state_key.as_ref() else {
            return;
        };
        let Some(saved) = cx
            .try_global::<SavedPopupStates>()
            .and_then(|states| states.get(key))
            .cloned()
        else {
            return;
        };
        if self.shared_state_loaded && saved == self.shared_state.to_persisted() {
            return;
        }
        self.shared_state = PopupState::from_persisted(saved);
        self.shared_state_loaded = true;
        self.rebuild(cx);
    }

    fn persist_shared_state(&mut self, cx: &mut Context<Self>) {
        if let Some(key) = &self.shared_state_key {
            self.save_task = state::save(key.clone(), self.shared_state.to_persisted(), cx);
        }
        self.rebuild(cx);
    }

    fn toggle_favorite_of_selected(&mut self, cx: &mut Context<Self>) {
        let refs = self.selected_refs();
        if !self.shared_state_loaded || refs.is_empty() {
            return;
        }
        let defaults = self.favorite_defaults();
        for info in refs {
            self.shared_state.toggle_favorite(&info.target, &defaults);
        }
        self.persist_shared_state(cx);
    }

    fn toggle_group_by_directory(&mut self, cx: &mut Context<Self>) {
        if !self.shared_state_loaded {
            return;
        }
        self.shared_state.toggles.group_by_directory =
            !self.shared_state.toggles.group_by_directory;
        self.persist_shared_state(cx);
    }

    fn toggle_show_tags(&mut self, cx: &mut Context<Self>) {
        if !self.shared_state_loaded {
            return;
        }
        self.shared_state.toggles.show_tags = !self.shared_state.toggles.show_tags;
        self.persist_shared_state(cx);
    }

    fn new_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(context) = self.branch_context() else {
            return;
        };
        if self.head_selected() {
            checkout::new_branch(context, window, cx);
            return;
        }
        if let [branch] = self.selected_branches().as_slice() {
            checkout::new_branch_from(context, branch.target.clone(), window, cx);
        }
    }

    fn update_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(context) = self.branch_context() else {
            return;
        };
        for branch in self.selected_branches() {
            integrate::update_branch(context.clone(), branch.target, window, cx);
        }
    }

    fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(context) = self.branch_context() else {
            return;
        };
        if !delete_enabled(&self.selected_refs()) {
            return;
        }
        for info in self.selected_refs() {
            manage::delete_ref(context.clone(), info.target, window, cx);
        }
    }

    fn compare_with_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(context) = self.branch_context() else {
            return;
        };
        for branch in self.selected_branches() {
            if !branch.is_current {
                compare::compare_with_current(context.clone(), branch.target, window, cx);
            }
        }
    }

    fn branch_pair(&self) -> Option<(RefTarget, RefTarget)> {
        match self.selected_branches().as_slice() {
            [only] if self.head_selected() => Some((
                only.target.clone(),
                RefTarget::local(self.data.current_branch.clone()?),
            )),
            [first, second] => Some((first.target.clone(), second.target.clone())),
            _ => None,
        }
    }

    fn compare_branches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some((first, second)), Some(repository)) =
            (self.branch_pair(), self.repository.clone())
        else {
            return;
        };
        let refs = CompareRefs {
            selected_label: first.name.clone(),
            selected_revision: first.full_ref_name(),
            current_label: second.name.clone(),
            current_revision: second.full_ref_name(),
        };
        self.workspace
            .update(cx, |workspace, cx| {
                let existing = workspace.items_of_type::<CompareView>(cx).find(|item| {
                    let view = item.read(cx);
                    view.refs() == &refs && view.repository().read(cx).id == repository.read(cx).id
                });
                if let Some(existing) = existing {
                    existing.update(cx, |view, cx| view.reload(cx));
                    workspace.activate_item(&existing, true, true, window, cx);
                    return;
                }
                let workspace_handle = workspace.weak_handle();
                let view = cx.new(|cx| CompareView::new(refs, repository, workspace_handle, cx));
                workspace.add_item_to_active_pane(Box::new(view), None, true, window, cx);
            })
            .log_err();
    }

    fn fetch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fetching {
            return;
        }
        let (Some(context), Some(repository)) = (self.branch_context(), self.repository.clone())
        else {
            return;
        };
        self.fetching = true;
        cx.notify();
        let askpass = context.askpass_delegate("git fetch", window, cx);
        let receiver = repository.update(cx, |repository, cx| {
            repository.fetch(FetchOptions::All, askpass, cx)
        });
        cx.spawn(async move |this, cx| {
            let result = receiver.await;
            this.update(cx, |this, cx| {
                this.fetching = false;
                cx.notify();
            })
            .ok();
            match result {
                Ok(Ok(output)) => {
                    let message = format_output(&RemoteAction::Fetch(None), output).message;
                    cx.update(|cx| context.notify(BranchNotice::info(message), cx));
                }
                Ok(Err(error)) => cx.update(|cx| context.notify_error("fetch", error, cx)),
                Err(_) => {}
            }
        })
        .detach();
    }

    fn menu_plan_for(&self, target: &RefTarget) -> Vec<ref_menu::MenuItemPlan> {
        let has_tracked_branch = target.kind == RefKind::Local
            && self
                .data
                .local
                .iter()
                .any(|branch| branch.name() == &*target.name && branch.upstream.is_some());
        let tag_commit_sha = match target.kind {
            RefKind::Tag => self
                .data
                .tags
                .iter()
                .find(|tag| tag.name == target.name)
                .map(|tag| &*tag.commit_sha),
            RefKind::Local | RefKind::Remote => None,
        };
        let head_short_sha: Option<String> = self
            .data
            .head_sha
            .as_ref()
            .map(|sha| sha.chars().take(SHORT_SHA_LENGTH).collect());
        ref_menu::menu_plan(&MenuContext {
            target,
            current_branch: self.data.current_branch.as_deref(),
            head_short_sha: head_short_sha.as_deref(),
            head_sha: self.data.head_sha.as_deref(),
            tag_commit_sha,
            has_commits: self.data.has_commits,
            remotes: &self.data.remote_names,
            has_tracked_branch,
            fetching: self.fetching,
        })
    }

    fn build_context_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ContextMenu>> {
        let context = self.branch_context()?;
        let kinds = self.selected_kinds();
        let refs = self.selected_refs();
        let head = kinds.contains(&NodeKind::Head);
        if kinds.len() == 1 && (refs.len() == 1 || head) {
            let target = match refs.first() {
                Some(info) => info.target.clone(),
                None => RefTarget::local(self.data.current_branch.clone()?),
            };
            let plan = self.menu_plan_for(&target);
            let dismiss: Rc<dyn Fn(&mut Window, &mut App)> =
                Rc::new(|_: &mut Window, _: &mut App| {});
            return Some(ref_menu::build_ref_menu(
                plan,
                MenuRunner {
                    context,
                    target,
                    dismiss,
                },
                window,
                cx,
            ));
        }
        let head_and_branch = kinds.len() == 2 && refs.len() == 1 && head;
        let several_refs = kinds.len() == refs.len() && refs.len() > 1;
        if !head_and_branch && !several_refs {
            return None;
        }
        let pair_enabled = self
            .branch_pair()
            .map(|(first, second)| first.name != second.name);
        let show_update = several_refs && !self.data.remote_names.is_empty();
        let update_enabled = update_enabled(&self.selected_branches(), self.fetching);
        let delete = several_refs.then(|| delete_enabled(&refs));
        let delete_label = delete_label(&refs);
        let pane = cx.weak_entity();
        Some(ContextMenu::build(window, cx, move |menu, _, _| {
            let mut menu = menu;
            if let Some(enabled) = pair_enabled {
                menu = if enabled {
                    let pane = pane.clone();
                    menu.entry("Compare Branches", None, move |window, cx| {
                        pane.update(cx, |pane, cx| pane.compare_branches(window, cx))
                            .ok();
                    })
                } else {
                    menu.item(ContextMenuEntry::new("Compare Branches").disabled(true))
                };
            }
            if show_update {
                menu = if update_enabled {
                    let pane = pane.clone();
                    menu.entry("Update Selected", None, move |window, cx| {
                        pane.update(cx, |pane, cx| pane.update_selected(window, cx))
                            .ok();
                    })
                } else {
                    menu.item(ContextMenuEntry::new("Update Selected").disabled(true))
                };
            }
            if let Some(enabled) = delete {
                menu = if enabled {
                    menu.entry(delete_label, None, move |window, cx| {
                        pane.update(cx, |pane, cx| pane.delete_selected(window, cx))
                            .ok();
                    })
                } else {
                    menu.item(ContextMenuEntry::new(delete_label).disabled(true))
                };
            }
            menu
        }))
    }

    fn deploy_context_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.build_context_menu(window, cx) else {
            return;
        };
        window.focus(&menu.focus_handle(cx), cx);
        let subscription =
            cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
                let menu_focused = this
                    .context_menu
                    .as_ref()
                    .is_some_and(|(menu, _, _)| menu.focus_handle(cx).contains_focused(window, cx));
                if menu_focused {
                    window.focus(&this.focus_handle, cx);
                }
                this.context_menu = None;
                cx.notify();
            });
        self.context_menu = Some((menu, position, subscription));
        cx.notify();
    }

    fn render_stripe(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let text_color = colors.text;
        let mut text_style = window.text_style();
        text_style.color = text_color;
        let run = text_style.to_run(STRIPE_LABEL.len());
        let font_size = TextSize::Small.rems(cx).to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(STRIPE_LABEL.into(), font_size, &[run], None);
        let label_height = line.width + px(1.);
        v_flex()
            .id("git-log-branches-stripe")
            .flex_none()
            .w(TOOLBAR_WIDTH)
            .h_full()
            .border_r_1()
            .border_color(colors.border)
            .cursor_pointer()
            .hover(|style| style.bg(colors.element_hover))
            .tooltip(Tooltip::text(STRIPE_LABEL))
            .on_click(cx.listener(|this, _, _, cx| this.expand_from_stripe(cx)))
            .child(
                h_flex()
                    .w_full()
                    .pt(STRIPE_ICON_TOP)
                    .justify_center()
                    .child(Icon::new(IconName::ArrowExpand).size(IconSize::Medium)),
            )
            .child(
                div()
                    .w_full()
                    .mt(STRIPE_LABEL_TOP - STRIPE_ICON_TOP - STRIPE_ICON_SIZE)
                    .h(label_height)
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, _| {
                                paint_rotated_line(&line, bounds, text_color, window)
                            },
                        )
                        .size_full(),
                    ),
            )
            .into_any_element()
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let refs = self.selected_refs();
        let branches = self.selected_branches();
        let head = self.head_selected();
        let has_remotes = !self.data.remote_names.is_empty();
        let new_branch_enabled = self.data.has_commits && (head || branches.len() == 1);
        let new_branch_meta = (!head && branches.len() > 1).then_some(SharedString::new_static(
            "Select only one branch to proceed create a new branch",
        ));
        let update_meta = update_description(&branches, self.fetching);
        let compare_enabled = branches.iter().any(|branch| !branch.is_current);
        let has_rows = !self.rows.is_empty();
        let navigate_enabled = model::navigatable_revision(&self.selected_kinds()).is_some();
        v_flex()
            .flex_none()
            .w(TOOLBAR_WIDTH)
            .h_full()
            .py(TOOLBAR_VERTICAL_INSET)
            .items_center()
            .child(toolbar_button(
                "git-log-branches-hide",
                IconName::ArrowCollapse,
                tooltip("Hide Git Branches", None),
                false,
                |this, _, cx| this.hide_branches(cx),
                cx,
            ))
            .child(toolbar_separator(cx))
            .child(toolbar_button(
                "git-log-branches-new",
                IconName::GeneralAdd,
                tooltip("New Branch…", new_branch_meta),
                !new_branch_enabled,
                |this, window, cx| this.new_branch(window, cx),
                cx,
            ))
            .when(has_remotes, |this| {
                this.child(toolbar_button(
                    "git-log-branches-update",
                    IconName::VcsUpdate,
                    tooltip("Update Selected", update_meta),
                    !update_enabled(&branches, self.fetching),
                    |this, window, cx| this.update_selected(window, cx),
                    cx,
                ))
            })
            .child(toolbar_button(
                "git-log-branches-delete",
                IconName::GeneralDelete,
                tooltip(delete_label(&refs), None),
                !delete_enabled(&refs),
                |this, window, cx| this.delete_selected(window, cx),
                cx,
            ))
            .child(toolbar_button(
                "git-log-branches-compare",
                IconName::VcsDiff,
                tooltip(
                    "Compare with Current",
                    (!compare_enabled && !branches.is_empty())
                        .then_some(SharedString::new_static("Select non-current branches only")),
                ),
                !compare_enabled,
                |this, window, cx| this.compare_with_current(window, cx),
                cx,
            ))
            .child(
                h_flex()
                    .flex_none()
                    .size(TOOLBAR_BUTTON_SIZE)
                    .justify_center()
                    .child(
                        IconButton::new("git-log-branches-my", IconName::GeneralSearch)
                            .shape(IconButtonShape::Square)
                            .icon_size(IconSize::Medium)
                            .toggle_state(false)
                            .disabled(true)
                            .tooltip(tooltip(
                                "Show My Branches",
                                Some(SharedString::new_static(
                                    "Some of the repositories doesn't support indexing.",
                                )),
                            )),
                    ),
            )
            .child(self.render_fetch_button(has_remotes, cx))
            .child(toolbar_button(
                "git-log-branches-favorite",
                IconName::NodesStar,
                tooltip("Mark/Unmark As Favorite", None),
                refs.is_empty() || !self.shared_state_loaded,
                |this, _, cx| this.toggle_favorite_of_selected(cx),
                cx,
            ))
            .child(toolbar_button(
                "git-log-branches-navigate",
                IconName::Locate,
                tooltip("Navigate Log to Selected Branch Head", None),
                !navigate_enabled,
                |this, _, cx| this.navigate_to_selected(cx),
                cx,
            ))
            .child(toolbar_separator(cx))
            .child(self.render_settings_button(cx))
            .child(toolbar_button(
                "git-log-branches-expand-all",
                IconName::ExpandAll,
                tooltip("Expand All", None),
                !has_rows,
                |this, _, cx| this.expand_all(cx),
                cx,
            ))
            .child(toolbar_button(
                "git-log-branches-collapse-all",
                IconName::CollapseAll,
                tooltip("Collapse All", None),
                !has_rows,
                |this, _, cx| this.collapse_all(cx),
                cx,
            ))
    }

    fn render_fetch_button(&self, has_remotes: bool, cx: &mut Context<Self>) -> AnyElement {
        if self.fetching {
            return h_flex()
                .id("git-log-branches-fetching")
                .flex_none()
                .size(TOOLBAR_BUTTON_SIZE)
                .justify_center()
                .child(
                    Icon::new(IconName::ArrowCircle)
                        .size(IconSize::Medium)
                        .color(Color::Muted)
                        .with_rotate_animation(2),
                )
                .tooltip(tooltip(
                    "Fetch",
                    Some(SharedString::new_static("Fetch in progress…")),
                ))
                .into_any_element();
        }
        toolbar_button(
            "git-log-branches-fetch",
            IconName::Fetch,
            tooltip("Fetch", None),
            !has_remotes,
            |this, window, cx| this.fetch(window, cx),
            cx,
        )
    }

    fn render_settings_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pane = cx.weak_entity();
        let single_click = self.single_click;
        let group_by_directory = self.shared_state.toggles.group_by_directory;
        let show_tags = self.shared_state.toggles.show_tags;
        h_flex()
            .flex_none()
            .size(TOOLBAR_BUTTON_SIZE)
            .justify_center()
            .child(
                PopoverMenu::new("git-log-branches-settings")
                    .trigger_with_tooltip(
                        IconButton::new(
                            "git-log-branches-settings-button",
                            IconName::GeneralSettings,
                        )
                        .shape(IconButtonShape::Square)
                        .icon_size(IconSize::Medium),
                        Tooltip::text("Branches Pane Settings"),
                    )
                    .menu(move |window, cx| {
                        let pane = pane.clone();
                        Some(ContextMenu::build(window, cx, move |menu, _, _| {
                            let on_select = |action: BranchesSingleClickAction| {
                                let pane = pane.clone();
                                move |_: &mut Window, cx: &mut App| {
                                    pane.update(cx, |pane, cx| {
                                        pane.toggle_single_click(action, cx)
                                    })
                                    .ok();
                                }
                            };
                            let group_pane = pane.clone();
                            let tags_pane = pane.clone();
                            menu.header("On Single Click")
                                .toggleable_entry(
                                    "Update Branch Filter",
                                    single_click == BranchesSingleClickAction::UpdateBranchFilter,
                                    IconPosition::Start,
                                    None,
                                    on_select(BranchesSingleClickAction::UpdateBranchFilter),
                                )
                                .toggleable_entry(
                                    "Navigate Log to Branch Head",
                                    single_click == BranchesSingleClickAction::NavigateLog,
                                    IconPosition::Start,
                                    None,
                                    on_select(BranchesSingleClickAction::NavigateLog),
                                )
                                .separator()
                                .toggleable_entry(
                                    "Group by Directory",
                                    group_by_directory,
                                    IconPosition::Start,
                                    None,
                                    move |_, cx| {
                                        group_pane
                                            .update(cx, |pane, cx| {
                                                pane.toggle_group_by_directory(cx)
                                            })
                                            .ok();
                                    },
                                )
                                .toggleable_entry(
                                    "Show Tags",
                                    show_tags,
                                    IconPosition::Start,
                                    None,
                                    move |_, cx| {
                                        tags_pane
                                            .update(cx, |pane, cx| pane.toggle_show_tags(cx))
                                            .ok();
                                    },
                                )
                        }))
                    })
                    .anchor(Anchor::TopLeft)
                    .attach(Anchor::TopRight),
            )
    }

    fn render_search(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        h_flex()
            .key_context(SEARCH_KEY_CONTEXT)
            .flex_none()
            .w_full()
            .h(self.toolbar_height)
            .px_2()
            .gap_1p5()
            .border_b_1()
            .border_color(colors.border_variant)
            .when(self.no_hits, |this| {
                this.bg(cx.theme().status().error_background)
            })
            .on_action(cx.listener(|this, _: &menu::SelectNext, window, cx| {
                this.search_moves_into_tree(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &menu::SelectPrevious, window, cx| {
                this.search_moves_into_tree(false, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &menu::Cancel, window, cx| this.search_cancel(window, cx)),
            )
            .child(
                Icon::new(IconName::MagnifyingGlass)
                    .size(IconSize::Small)
                    .color(Color::Muted),
            )
            .child(div().flex_1().min_w_0().child(self.search_editor.clone()))
    }

    fn render_tree(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let showing_progress = self.loading || self.fetching;
        div()
            .id("git-log-branches-tree")
            .key_context(TREE_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
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
            .on_action(cx.listener(|this, _: &menu::Confirm, _, cx| this.update_branch_filter(cx)))
            .on_action(
                cx.listener(|this, _: &menu::Cancel, window, cx| this.tree_cancel(window, cx)),
            )
            .on_key_down(cx.listener(Self::tree_key_down))
            .when(showing_progress, |this| {
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(PROGRESS_STRIPE_HEIGHT)
                        .bg(colors.text_accent),
                )
            })
            .map(|this| {
                if self.rows.is_empty() {
                    let text = if self.loading {
                        LOADING_TEXT
                    } else {
                        EMPTY_TEXT
                    };
                    this.child(
                        h_flex()
                            .size_full()
                            .justify_center()
                            .child(Label::new(text).color(Color::Muted)),
                    )
                } else {
                    this.child(
                        uniform_list(
                            "git-log-branches-rows",
                            self.rows.len(),
                            cx.processor(Self::render_rows),
                        )
                        .size_full()
                        .track_scroll(&self.scroll_handle),
                    )
                }
            })
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let focused = self.focus_handle.contains_focused(window, cx)
            || self
                .search_editor
                .focus_handle(cx)
                .contains_focused(window, cx);
        let colors = cx.theme().colors();
        let light = cx.theme().appearance().is_light();
        let style = RowStyle {
            selection: if focused {
                colors.element_selected
            } else {
                colors
                    .element_selected
                    .grayscale()
                    .opacity(INACTIVE_SELECTION_OPACITY)
            },
            hover: colors.element_hover,
            incoming: Hsla::from(rgb(if light { INCOMING_LIGHT } else { INCOMING_DARK })),
            outgoing: Hsla::from(rgb(if light { OUTGOING_LIGHT } else { OUTGOING_DARK })),
        };
        let matcher = (!self.applied_query.trim().is_empty())
            .then(|| BranchesTreeMatcher::new(&self.applied_query));
        range
            .filter_map(|index| {
                self.rows
                    .get(index)
                    .map(|row| self.render_row(index, row, &style, matcher.as_ref(), cx))
            })
            .collect()
    }

    fn render_row(
        &self,
        index: usize,
        row: &TreeRow,
        style: &RowStyle,
        matcher: Option<&BranchesTreeMatcher>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let selected = self.selection.contains(&row.id);
        let indent = TREE_LEFT_INSET + TREE_INDENT * row.depth as f32;
        let chevron = row.has_children.then(|| {
            let id = row.id.clone();
            let expand = !row.expanded;
            div()
                .id(("git-log-branches-chevron", index))
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
        let icon = row_icon(&row.kind)
            .map(|(icon, color)| Icon::new(icon).size(IconSize::Medium).color(color));
        let highlights = matcher
            .zip(row.match_text.as_ref())
            .and_then(|(matcher, text)| {
                matcher
                    .match_fragments(text)
                    .map(|fragments| (fragments, text))
            })
            .map(|(fragments, text)| highlight_positions(&row.display_name, text, &fragments))
            .unwrap_or_default();
        let counters = match &row.kind {
            NodeKind::Ref(info) if info.incoming > 0 || info.outgoing > 0 => {
                Some(render_counters(index, info, style))
            }
            _ => None,
        };
        let row_tooltip = match &row.kind {
            NodeKind::Ref(info) if info.target.kind == RefKind::Local => {
                Some(match &info.tracked {
                    Some(tracked) => {
                        SharedString::from(format!("{} → {}", info.target.name, tracked))
                    }
                    None => SharedString::new_static(NO_TRACKING_BRANCH),
                })
            }
            _ => None,
        };
        let left_id = row.id.clone();
        let right_id = row.id.clone();
        div()
            .id(("git-log-branches-row", index))
            .relative()
            .h(ROW_HEIGHT)
            .w_full()
            .cursor_pointer()
            .when(!selected, |this| {
                this.hover(|row_style| row_style.bg(style.hover))
            })
            .when(selected, |this| {
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(SELECTION_INSET)
                        .right(SELECTION_INSET)
                        .rounded(SELECTION_RADIUS)
                        .bg(style.selection),
                )
            })
            .when_some(row_tooltip, |this, text| this.tooltip(Tooltip::text(text)))
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
                            .child(HighlightedLabel::new(row.display_name.clone(), highlights))
                            .children(counters),
                    ),
            )
            .into_any_element()
    }
}

fn toolbar_button(
    id: &'static str,
    icon: IconName,
    tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    disabled: bool,
    on_click: impl Fn(&mut BranchesPane, &mut Window, &mut Context<BranchesPane>) + 'static,
    cx: &Context<BranchesPane>,
) -> AnyElement {
    h_flex()
        .flex_none()
        .size(TOOLBAR_BUTTON_SIZE)
        .justify_center()
        .child(
            IconButton::new(id, icon)
                .shape(IconButtonShape::Square)
                .icon_size(IconSize::Medium)
                .disabled(disabled)
                .tooltip(tooltip)
                .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx))),
        )
        .into_any_element()
}

fn toolbar_separator(cx: &Context<BranchesPane>) -> impl IntoElement {
    h_flex()
        .flex_none()
        .w(TOOLBAR_BUTTON_SIZE)
        .h(TOOLBAR_SEPARATOR_HEIGHT)
        .px(TOOLBAR_SEPARATOR_INSET)
        .items_center()
        .child(
            div()
                .w_full()
                .h(px(1.))
                .bg(cx.theme().colors().border_variant),
        )
}

fn tooltip(
    title: impl Into<SharedString>,
    meta: Option<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let title = title.into();
    move |_, cx| match &meta {
        Some(meta) => Tooltip::with_meta(title.clone(), None, meta.clone(), cx),
        None => Tooltip::simple(title.clone(), cx),
    }
}

fn delete_enabled(refs: &[RefInfo]) -> bool {
    !refs.is_empty()
        && refs.iter().all(|info| {
            !info.is_current
                && !(info.target.kind == RefKind::Remote
                    && integrate::is_protected_remote_reference(&info.target.name))
        })
}

fn delete_label(refs: &[RefInfo]) -> SharedString {
    if refs.iter().all(RefInfo::is_branch) {
        if refs.len() > 1 {
            SharedString::new_static("Delete Branches")
        } else {
            SharedString::new_static("Delete Branch")
        }
    } else {
        SharedString::new_static("Delete")
    }
}

fn update_enabled(branches: &[RefInfo], fetching: bool) -> bool {
    !fetching
        && !branches.is_empty()
        && branches
            .iter()
            .all(|branch| branch.target.kind == RefKind::Local && branch.tracked.is_some())
}

fn update_description(branches: &[RefInfo], fetching: bool) -> Option<SharedString> {
    if fetching {
        return Some(SharedString::new_static("Update is already running"));
    }
    if branches.is_empty() {
        return None;
    }
    let noun = if branches.len() == 1 {
        "branch"
    } else {
        "branches"
    };
    if update_enabled(branches, fetching) {
        Some(
            format!(
                "Fetch from tracked remote and update selected {noun} using merge or fast-forward like `git fetch branch:branch` if possible"
            )
            .into(),
        )
    } else {
        Some(format!("Tracked branch is not configured for selected {noun}").into())
    }
}

fn row_icon(kind: &NodeKind) -> Option<(IconName, Color)> {
    match kind {
        NodeKind::Ref(info) => Some(match (info.is_current, info.is_favorite) {
            (true, true) => (IconName::CurrentBranchFavoriteLabel, Color::Warning),
            (true, false) => (IconName::CurrentBranchLabel, Color::Warning),
            (false, true) => (IconName::NodesStar, Color::Warning),
            (false, false) if info.target.kind == RefKind::Tag => {
                (IconName::TagLabel, Color::Muted)
            }
            (false, false) => (IconName::BranchNode, Color::Muted),
        }),
        NodeKind::Group { .. } | NodeKind::RemoteGroup => {
            Some((IconName::NodesFolder, Color::Muted))
        }
        NodeKind::Head | NodeKind::TopLevel(_) => None,
    }
}

fn counter_label(count: u32) -> SharedString {
    if count > MAX_COMMIT_COUNTER {
        SharedString::new_static("99+")
    } else {
        SharedString::from(count.to_string())
    }
}

fn counters_tooltip(info: &RefInfo) -> Option<SharedString> {
    let plural = |count: u32| if count == 1 { "commit" } else { "commits" };
    match (info.incoming, info.outgoing) {
        (0, 0) => None,
        (incoming, 0) => Some(format!("{incoming} incoming {}", plural(incoming)).into()),
        (0, outgoing) => Some(format!("{outgoing} outgoing {}", plural(outgoing)).into()),
        (incoming, outgoing) => {
            Some(format!("{incoming} incoming and {outgoing} outgoing commits").into())
        }
    }
}

fn render_counters(index: usize, info: &RefInfo, style: &RowStyle) -> AnyElement {
    let counter = |icon: IconName, count: u32, color: Hsla| {
        h_flex()
            .gap(COUNTER_ICON_GAP)
            .child(
                Icon::new(icon)
                    .size(IconSize::Medium)
                    .color(Color::Custom(color)),
            )
            .child(
                Label::new(counter_label(count))
                    .size(LabelSize::Small)
                    .color(Color::Custom(color)),
            )
    };
    h_flex()
        .id(("git-log-branches-counters", index))
        .flex_none()
        .gap(COUNTER_GAP)
        .when(info.incoming > 0, |this| {
            this.child(counter(
                IconName::IncomingCommits,
                info.incoming,
                style.incoming,
            ))
        })
        .when(info.outgoing > 0, |this| {
            this.child(counter(
                IconName::OutgoingCommits,
                info.outgoing,
                style.outgoing,
            ))
        })
        .when_some(counters_tooltip(info), |this, text| {
            this.tooltip(Tooltip::text(text))
        })
        .into_any_element()
}

fn highlight_positions(label: &str, text: &str, fragments: &[Range<usize>]) -> Vec<usize> {
    let label_length = label.chars().count();
    let offset = text.chars().count().saturating_sub(label_length);
    let byte_offsets: Vec<usize> = label.char_indices().map(|(offset, _)| offset).collect();
    fragments
        .iter()
        .flat_map(|fragment| fragment.clone())
        .filter_map(|position| position.checked_sub(offset))
        .filter_map(|position| byte_offsets.get(position).copied())
        .collect()
}

fn tracked_name(upstream: &Upstream) -> SharedString {
    let name = upstream
        .stripped_ref_name()
        .or_else(|| upstream.ref_name.strip_prefix("refs/heads/"))
        .unwrap_or(&upstream.ref_name);
    SharedString::from(name.to_string())
}

fn is_remote_head(name: &str) -> bool {
    name == REMOTE_HEAD_NAME
        || name
            .strip_suffix(REMOTE_HEAD_NAME)
            .is_some_and(|prefix| prefix.ends_with('/'))
}

fn remote_name_of(branch_name: &str, remote_names: &[SharedString]) -> SharedString {
    remote_names
        .iter()
        .filter(|remote| {
            branch_name
                .strip_prefix(remote.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
        })
        .max_by_key(|remote| remote.len())
        .cloned()
        .unwrap_or_else(|| {
            SharedString::from(
                branch_name
                    .split('/')
                    .next()
                    .unwrap_or(branch_name)
                    .to_string(),
            )
        })
}

fn paint_rotated_line(line: &ShapedLine, bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    let baseline = (bounds.size.width - line.ascent - line.descent) / 2. + line.ascent;
    let scale_factor = window.scale_factor();
    for run in &line.runs {
        for glyph in &run.glyphs {
            if glyph.is_emoji {
                continue;
            }
            let origin = point(
                bounds.left() + baseline + glyph.position.y,
                bounds.bottom() - glyph.position.x,
            );
            let transformation = TransformationMatrix::unit()
                .translate(origin.scale(scale_factor))
                .rotate(radians(-std::f32::consts::FRAC_PI_2))
                .translate(origin.scale(-scale_factor));
            window
                .paint_glyph_with_transformation(
                    origin,
                    run.font_id,
                    glyph.id,
                    line.font_size,
                    color,
                    transformation,
                )
                .log_err();
        }
    }
}

impl EventEmitter<BranchesPaneEvent> for BranchesPane {}

impl Focusable for BranchesPane {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Drop for BranchesPane {
    fn drop(&mut self) {
        std::mem::replace(&mut self.save_task, Task::ready(())).detach();
    }
}

impl Render for BranchesPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.expanded {
            return self.render_stripe(window, cx);
        }
        let colors = cx.theme().colors();
        let panel_background = colors.panel_background;
        let border = colors.border;
        h_flex()
            .key_context(KEY_CONTEXT)
            .size_full()
            .bg(panel_background)
            .on_action(cx.listener(|this, _: &ToggleSearchFocus, window, cx| {
                this.toggle_search_focus(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ExpandAll, _, cx| this.expand_all(cx)))
            .on_action(cx.listener(|this, _: &CollapseAll, _, cx| this.collapse_all(cx)))
            .on_action(cx.listener(|this, _: &CompareWithCurrent, window, cx| {
                this.compare_with_current(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &DeleteSelected, window, cx| {
                    this.delete_selected(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &NavigateToSelectedBranch, _, cx| {
                this.navigate_to_selected(cx)
            }))
            .on_action(
                cx.listener(|this, _: &ToggleFavorite, _, cx| this.toggle_favorite_of_selected(cx)),
            )
            .child(self.render_toolbar(cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .border_l_1()
                    .border_color(border)
                    .child(self.render_search(cx))
                    .child(self.render_tree(cx)),
            )
            .children(self.context_menu.as_ref().map(|(menu, position, _)| {
                deferred(
                    anchored()
                        .position(*position)
                        .anchor(Anchor::TopLeft)
                        .child(menu.clone()),
                )
                .with_priority(1)
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui::{TestAppContext, VisualTestContext};
    use settings::SettingsStore;

    use super::*;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            cx.set_global(db::AppDatabase::test_new());
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            editor::init(cx);
        });
    }

    #[gpui::test]
    async fn hiding_expanding_and_single_click_changes_are_reported(cx: &mut TestAppContext) {
        init_test(cx);
        let window = cx.add_window(|window, cx| {
            BranchesPane::new(
                WeakEntity::new_invalid(),
                None,
                true,
                BranchesSingleClickAction::None,
                window,
                cx,
            )
        });
        let pane = window.root(cx).expect("the window has a branches pane");
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        let events: Rc<RefCell<Vec<BranchesPaneEvent>>> = Rc::default();
        cx.update(|_, cx| {
            let events = events.clone();
            cx.subscribe(&pane, move |_, event: &BranchesPaneEvent, _| {
                events.borrow_mut().push(event.clone());
            })
            .detach();
        });

        pane.update(&mut cx, |pane, cx| pane.hide_branches(cx));
        assert!(!pane.read_with(&cx, |pane, _| pane.is_expanded()));
        pane.update(&mut cx, |pane, cx| pane.expand_from_stripe(cx));
        assert!(pane.read_with(&cx, |pane, _| pane.is_expanded()));
        pane.update(&mut cx, |pane, cx| pane.set_expanded(false, cx));
        assert!(!pane.read_with(&cx, |pane, _| pane.is_expanded()));

        pane.update(&mut cx, |pane, cx| {
            pane.toggle_single_click(BranchesSingleClickAction::UpdateBranchFilter, cx);
            pane.toggle_single_click(BranchesSingleClickAction::NavigateLog, cx);
            pane.toggle_single_click(BranchesSingleClickAction::NavigateLog, cx);
        });

        assert_eq!(
            *events.borrow(),
            vec![
                BranchesPaneEvent::ExpandedChanged(false),
                BranchesPaneEvent::ExpandedChanged(true),
                BranchesPaneEvent::SingleClickActionChanged(
                    BranchesSingleClickAction::UpdateBranchFilter
                ),
                BranchesPaneEvent::SingleClickActionChanged(BranchesSingleClickAction::NavigateLog),
                BranchesPaneEvent::SingleClickActionChanged(BranchesSingleClickAction::None),
            ]
        );
    }

    #[test]
    fn highlights_map_full_name_fragments_onto_the_displayed_segment() {
        assert_eq!(
            highlight_positions("login", "feature/login", &[8..10]),
            vec![0, 1]
        );
        assert_eq!(
            highlight_positions("feature", "feature", &[0..2]),
            vec![0, 1]
        );
        assert!(highlight_positions("login", "feature/login", &[0..3]).is_empty());
    }

    #[test]
    fn remote_names_prefer_the_longest_known_remote() {
        let remotes = [
            SharedString::from("origin"),
            SharedString::from("origin/mirror"),
        ];
        assert_eq!(
            remote_name_of("origin/mirror/main", &remotes),
            "origin/mirror"
        );
        assert_eq!(remote_name_of("origin/main", &remotes), "origin");
        assert_eq!(remote_name_of("upstream/main", &remotes), "upstream");
    }

    #[test]
    fn counters_cap_at_ninety_nine_and_describe_both_directions() {
        assert_eq!(counter_label(99), "99");
        assert_eq!(counter_label(100), "99+");
        let info = RefInfo {
            incoming: 2,
            outgoing: 1,
            ..RefInfo::new(RefTarget::local("main"))
        };
        assert_eq!(
            counters_tooltip(&info).as_deref(),
            Some("2 incoming and 1 outgoing commits")
        );
        let info = RefInfo {
            incoming: 1,
            ..RefInfo::new(RefTarget::local("main"))
        };
        assert_eq!(
            counters_tooltip(&info).as_deref(),
            Some("1 incoming commit")
        );
    }
}
