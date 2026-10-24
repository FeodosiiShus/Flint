mod columns;
mod context_menu;
mod new_tag_dialog;
mod operations;
mod presentation;
mod reset_dialog;
#[cfg(test)]
mod tests;

use std::{
    cell::{Cell, RefCell},
    ops::Range,
    rc::Rc,
    sync::Arc,
};

use collections::{HashMap, HashSet};
use git::{
    Oid,
    repository::{CommitData, GitUser, InitialGraphCommitData, LogFilter, LogOrder, LogSource},
};
use gpui::{
    Anchor, AnyElement, App, Bounds, ClickEvent, ClipboardItem, ContentMask, Context, Corners,
    DismissEvent, DragMoveEvent, Entity, EventEmitter, FocusHandle, Focusable, Font, FontWeight,
    Hsla, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, ScrollStrategy, SharedString,
    Subscription, Task, TextAlign, TextRun, UniformListScrollHandle, WeakEntity, Window, actions,
    anchored, canvas, deferred, fill, point, px, rgb, size, uniform_list,
};
use project::git_store::{CommitDataState, GitGraphEvent, Repository, RepositoryEvent};
use settings::Settings as _;
use theme::Appearance;
use theme_settings::ThemeSettings;
use ui::{Button, ContextMenu, Tooltip, WithScrollbar, prelude::*};
use util::ResultExt as _;
use workspace::Workspace;

use crate::branch_operations::{BranchContext, BranchNotice};
use crate::commit_view::CommitView;
use crate::git_log::{
    graph::{CommitGraph, GraphPalette, PrintElement, element_at, graph_width, paint_graph_row},
    log_colors, log_dates,
    properties::{HEAD_REVISION, LogAppProperties, LogColumn, LogTabProperties},
    ref_labels::{self, RefGroupingContext, RefLabel, RefLabelGroup, RefLabelsStyle},
};
use columns::{
    ColumnBounds, HASH_WIDTH_SAMPLE, MAX_ROWS_TO_CALCULATE_WIDTH, TEXT_HORIZONTAL_PADDING,
    TEXT_LEFT_PADDING, column_gaps, data_width, is_dynamic, layout_columns, merge_widths,
    persisted_width, reset_target, resizable_borders, resize_border, visible_columns,
    without_width,
};
use context_menu::{CommitMenuRequest, MenuCommit};
use presentation::{
    LabelsTooltip, MAX_SELECTED_COMMITS, SelectedRowType, arrow_tooltip_text, author_presentation,
    commit_presentation, go_to_action_text, is_filtered_by_me, is_hash_prefix, is_my_commit,
    is_single_user, labels_tooltip_areas, not_a_hash_message, not_found_message,
    reference_presentation, resolve_current_users, selected_row_type, short_hash,
    single_filtered_branch, subject_text, trim_hash_quotes,
};

actions!(
    git_log_table,
    [
        CopyRevisionNumber,
        Copy,
        SelectAll,
        SelectPageUp,
        SelectPageDown,
        ExtendSelectionUp,
        ExtendSelectionDown,
        ExtendSelectionPageUp,
        ExtendSelectionPageDown,
        ExtendSelectionToFirst,
        ExtendSelectionToLast,
        GoToParentCommit,
        GoToChildCommit,
        ShowDiff,
        CherryPick,
        NewBranchFromCommit,
    ]
);

const KEY_CONTEXT: &str = "GitLogTable";
const LOG_ORDER: LogOrder = LogOrder::DateOrder;
const UP_PRELOAD_COUNT: usize = 20;
const DOWN_PRELOAD_COUNT: usize = 40;
const MINIMUM_ROW_HEIGHT: Pixels = px(26.);
const VERTICAL_PADDING: Pixels = px(7.);
const SELECTION_INSET: Pixels = px(4.);
const SELECTION_RADIUS: Pixels = px(4.);
const SEARCH_MATCH_INSET: Pixels = px(2.);
const SEARCH_MATCH_RADIUS: Pixels = px(2.5);
const LABELS_RIGHT_PADDING: Pixels = px(4.);
const DISPLAYED_MESSAGE_PART: Pixels = px(80.);
const DEFAULT_COLUMN_WIDTH: Pixels = px(75.);
const PROGRESS_STRIPE_HEIGHT: Pixels = px(2.);
const ERROR_TEXT_MAX_LENGTH: usize = 150;
const SEARCH_MATCH_DARK: u32 = 0xBA9752;
const SEARCH_MATCH_LIGHT: u32 = 0xFEE6B1;
const SEARCH_MATCH_TEXT_DARK: u32 = 0x000000;
const SEARCH_MATCH_TEXT_LIGHT: u32 = 0x323232;
const LOADING_SUBJECT: &str = "Loading\u{2026}";
const DEFAULT_STATUS: &str = "Changes log";
const LOADING_STATUS: &str = "Loading commits\u{2026}";
const ERROR_LOADING_STATUS: &str = "Error loading commits";
const NO_COMMITS_STATUS: &str = "No changes committed.";
const COMMIT_STATUS_ACTION: &str = "Commit local changes";
const NO_MATCHING_COMMITS_STATUS: &str = "No commits matching filters";
const REFRESH_STATUS_ACTION: &str = "Refresh";
const SELECT_PARENT_TITLE: &str = "Select Parent to Navigate";
const SELECT_CHILD_TITLE: &str = "Select Child to Navigate";

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CommitTableEvent {
    SelectionChanged(Vec<Oid>),
    ColumnWidthsChanged(Vec<(LogColumn, f32)>),
    LoadingChanged(bool),
}

struct DisplayedLog {
    graph: CommitGraph,
    commits: Arc<Vec<Arc<InitialGraphCommitData>>>,
    /// Whether each commit, by its position in `commits`, is reachable from HEAD.
    contained_in_head: Option<Vec<bool>>,
}

#[derive(Clone, Copy)]
struct TableColors {
    background: Hsla,
    foreground: Hsla,
    selection: Hsla,
    inactive_selection: Hsla,
    unmatched: Hsla,
    current_branch: Hsla,
    reference: Hsla,
    search_match_background: Hsla,
    search_match_foreground: Hsla,
    palette: GraphPalette,
}

#[derive(Clone)]
struct FrameLayout {
    row_height: Pixels,
    font: Font,
    font_size: Pixels,
    columns: Vec<ColumnBounds>,
    colors: TableColors,
    label_style: RefLabelsStyle,
}

#[derive(Clone, Copy)]
struct RowStyle {
    background: Hsla,
    foreground: Hsla,
    bold_author: bool,
    bold_all: bool,
    overlay: Option<(Hsla, SelectedRowType)>,
}

impl RowStyle {
    fn visible_background(&self) -> Hsla {
        self.overlay.map_or(self.background, |(color, _)| color)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum JumpTarget {
    Commit(Oid),
    HashPrefix(String),
}

struct ColumnResize {
    border: usize,
    grab_offset: Pixels,
    start_columns: Vec<ColumnBounds>,
}

struct ScrollAnchor {
    sha: Oid,
    top_gap: Pixels,
    at_top: bool,
}

#[derive(Clone, Copy)]
enum StatusAction {
    Refresh,
    Commit,
}

#[derive(Clone)]
struct DraggedColumnBorder;

pub(crate) struct CommitTable {
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    app_properties: LogAppProperties,
    tab_properties: LogTabProperties,
    filter: LogFilter,
    current_user: Option<GitUser>,
    repository: Option<Entity<Repository>>,
    authors: Option<Vec<GitUser>>,
    my_identities: Option<HashSet<(String, String)>>,
    ref_context: RefGroupingContext,
    branch_names: Vec<SharedString>,
    commits: Vec<Arc<InitialGraphCommitData>>,
    /// The history and its authors load once the table is first drawn: on a huge repository they
    /// take hundreds of megabytes, which a Git tool window that stays closed shouldn't cost.
    shown: bool,
    displayed: Option<DisplayedLog>,
    is_loading: bool,
    fully_loaded: bool,
    load_error: Option<SharedString>,
    build_task: Option<Task<()>>,
    rebuild_requested: bool,
    /// How many commits the last graph build started with.
    built_commit_count: usize,
    metadata: HashMap<Oid, Arc<CommitData>>,
    selection: HashSet<Oid>,
    lead: Option<Oid>,
    anchor: Option<Oid>,
    last_emitted_selection: Vec<Oid>,
    hovered_row: Option<usize>,
    hovered_arrow: Option<PrintElement>,
    scroll_handle: UniformListScrollHandle,
    table_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    cell_bounds: Rc<RefCell<HashMap<usize, Bounds<Pixels>>>>,
    frame: Option<FrameLayout>,
    labels_height: Cell<Pixels>,
    data_widths: HashMap<LogColumn, Pixels>,
    initialized_columns: HashSet<LogColumn>,
    column_resize: Option<ColumnResize>,
    live_widths: Option<Vec<(LogColumn, f32)>>,
    context_menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
    pending_jump: Option<JumpTarget>,
    cherry_picks_in_progress: HashSet<Oid>,
    _repository_subscription: Option<Subscription>,
    _authors_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CommitTableEvent> for CommitTable {}

impl Focusable for CommitTable {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl CommitTable {
    pub(crate) fn new(
        workspace: WeakEntity<Workspace>,
        app_properties: LogAppProperties,
        tab_properties: LogTabProperties,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let subscriptions = vec![
            cx.on_focus_in(&focus_handle, window, |_, _, cx| cx.notify()),
            cx.on_focus_out(&focus_handle, window, |_, _, _, cx| cx.notify()),
        ];
        Self {
            workspace,
            focus_handle,
            app_properties,
            tab_properties,
            filter: LogFilter::default(),
            current_user: None,
            repository: None,
            authors: None,
            my_identities: None,
            ref_context: RefGroupingContext::default(),
            branch_names: Vec::new(),
            commits: Vec::new(),
            shown: false,
            displayed: None,
            is_loading: false,
            fully_loaded: false,
            load_error: None,
            build_task: None,
            rebuild_requested: false,
            built_commit_count: 0,
            metadata: HashMap::default(),
            selection: HashSet::default(),
            lead: None,
            anchor: None,
            last_emitted_selection: Vec::new(),
            hovered_row: None,
            hovered_arrow: None,
            scroll_handle: UniformListScrollHandle::new(),
            table_bounds: Rc::new(Cell::new(None)),
            cell_bounds: Rc::new(RefCell::new(HashMap::default())),
            frame: None,
            labels_height: Cell::new(Pixels::ZERO),
            data_widths: HashMap::default(),
            initialized_columns: HashSet::default(),
            column_resize: None,
            live_widths: None,
            context_menu: None,
            pending_jump: None,
            cherry_picks_in_progress: HashSet::default(),
            _repository_subscription: None,
            _authors_task: None,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn set_repository(
        &mut self,
        repository: Option<Entity<Repository>>,
        cx: &mut Context<Self>,
    ) {
        let unchanged = self
            .repository
            .as_ref()
            .map(|repository| repository.entity_id())
            == repository.as_ref().map(|repository| repository.entity_id());
        if unchanged {
            return;
        }
        self.repository = repository;
        self._repository_subscription = self.repository.as_ref().map(|repository| {
            cx.subscribe(
                repository,
                |this, repository, event: &RepositoryEvent, cx| {
                    this.handle_repository_event(repository, event, cx)
                },
            )
        });
        self.displayed = None;
        self.metadata.clear();
        self.selection.clear();
        self.lead = None;
        self.anchor = None;
        self.hovered_row = None;
        self.hovered_arrow = None;
        self.pending_jump = None;
        self.data_widths.clear();
        self.initialized_columns.clear();
        self.context_menu = None;
        self.update_ref_context(cx);
        self.load_authors(cx);
        self.reload(cx);
        self.selection_changed(cx);
    }

    pub(crate) fn set_filter(&mut self, filter: LogFilter, cx: &mut Context<Self>) {
        if self.filter == filter {
            return;
        }
        self.filter = filter;
        self.initialized_columns.clear();
        self.reload(cx);
    }

    pub(crate) fn set_properties(
        &mut self,
        app_properties: LogAppProperties,
        tab_properties: LogTabProperties,
        cx: &mut Context<Self>,
    ) {
        let graph_options_changed =
            self.tab_properties.graph_options != tab_properties.graph_options;
        let long_edges_changed =
            self.tab_properties.show_long_edges != tab_properties.show_long_edges;
        if self.app_properties.prefer_commit_date != app_properties.prefer_commit_date {
            self.initialized_columns.remove(&LogColumn::Date);
        }
        self.app_properties = app_properties;
        self.tab_properties = tab_properties;
        if long_edges_changed {
            let show_long_edges = self.tab_properties.show_long_edges;
            if let Some(displayed) = self.displayed.as_mut() {
                displayed.graph.set_show_long_edges(show_long_edges);
            }
            if self.build_task.is_some() {
                self.rebuild_requested = true;
            }
        }
        if graph_options_changed {
            self.schedule_graph_build(cx);
        }
        self.update_my_identities();
        cx.notify();
    }

    pub(crate) fn set_current_user(&mut self, user: Option<GitUser>, cx: &mut Context<Self>) {
        self.current_user = user;
        self.update_my_identities();
        cx.notify();
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if let Some(repository) = self.repository.clone() {
            repository.update(cx, |repository, cx| repository.rescan(cx));
        }
        self.load_authors(cx);
    }

    pub(crate) fn selected_commits(&self) -> Vec<Oid> {
        self.selected_rows()
            .into_iter()
            .filter_map(|row| self.sha_at_row(row))
            .collect()
    }

    pub(crate) fn can_cherry_pick(&self) -> bool {
        if self.repository.is_none() {
            return false;
        }
        let commits = self.selected_commits();
        !commits.is_empty()
            && commits
                .iter()
                .take(MAX_SELECTED_COMMITS)
                .all(|sha| !self.cherry_picks_in_progress.contains(sha))
    }

    pub(crate) fn cherry_pick_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_cherry_pick() {
            return;
        }
        let Some(repository) = self.repository.clone() else {
            return;
        };
        let commits = self.selected_menu_commits();
        let shas: Vec<Oid> = commits.iter().map(|commit| commit.sha).collect();
        self.cherry_picks_in_progress.extend(shas.iter().copied());
        let context = BranchContext::new(self.workspace.clone(), repository);
        let task = operations::cherry_pick_commits(context, commits, window, cx);
        cx.spawn(async move |this, cx| {
            task.await;
            this.update(cx, |this, cx| {
                for sha in &shas {
                    this.cherry_picks_in_progress.remove(sha);
                }
                cx.emit(CommitTableEvent::SelectionChanged(this.selected_commits()));
                cx.notify();
            })
            .log_err();
        })
        .detach();
        cx.emit(CommitTableEvent::SelectionChanged(self.selected_commits()));
        cx.notify();
    }

    pub(crate) fn jump_to_revision(
        &mut self,
        revision: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reference = revision.trim().to_string();
        if reference.is_empty() {
            return;
        }
        let target = match self.matching_reference(&reference, cx) {
            Some(sha) => JumpTarget::Commit(sha),
            None => {
                let trimmed = trim_hash_quotes(&reference);
                if !is_hash_prefix(trimmed) {
                    self.notify_warning(not_a_hash_message(&reference), cx);
                    return;
                }
                JumpTarget::HashPrefix(trimmed.to_ascii_lowercase())
            }
        };
        self.pending_jump = Some(target);
        self.resolve_pending_jump(Some(window), cx);
    }

    fn log_source(&self) -> LogSource {
        LogSource::Filtered(self.filter.clone())
    }

    fn handle_repository_event(
        &mut self,
        repository: Entity<Repository>,
        event: &RepositoryEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            RepositoryEvent::GraphEvent((source, order), graph_event) => {
                if !self.shown || *order != LOG_ORDER || *source != self.log_source() {
                    return;
                }
                match graph_event {
                    GitGraphEvent::CountUpdated(count) => {
                        self.append_commits(&repository, *count, cx)
                    }
                    GitGraphEvent::FullyLoaded => {
                        self.fully_loaded = true;
                        self.set_loading(false, cx);
                        self.schedule_graph_build(cx);
                    }
                    GitGraphEvent::LoadingError => {
                        self.load_error = repository
                            .read(cx)
                            .get_graph_data(source.clone(), *order)
                            .and_then(|data| data.error.clone());
                        self.fully_loaded = true;
                        self.set_loading(false, cx);
                        self.schedule_graph_build(cx);
                    }
                }
                cx.notify();
            }
            RepositoryEvent::HeadChanged | RepositoryEvent::BranchListChanged => {
                self.update_ref_context(cx);
                if repository.read(cx).scan_id > 1 {
                    self.reload(cx);
                }
            }
            RepositoryEvent::StashEntriesChanged => {
                if self.filter.revisions.is_empty() && repository.read(cx).scan_id > 2 {
                    self.reload(cx);
                }
            }
            _ => {}
        }
    }

    fn update_ref_context(&mut self, cx: &App) {
        match &self.repository {
            Some(repository) => {
                let repository = repository.read(cx);
                self.ref_context = presentation::ref_grouping_context(repository);
                self.branch_names = presentation::branch_names(repository);
            }
            None => {
                self.ref_context = RefGroupingContext::default();
                self.branch_names = Vec::new();
            }
        }
    }

    fn load_authors(&mut self, cx: &mut Context<Self>) {
        self.authors = None;
        self.update_my_identities();
        if !self.shown {
            return;
        }
        let Some(repository) = &self.repository else {
            self._authors_task = None;
            return;
        };
        let task = repository.read(cx).log_authors(cx);
        self._authors_task = Some(cx.spawn(async move |this, cx| {
            let authors = task.await.log_err();
            this.update(cx, |this, cx| {
                this.authors = authors;
                this.update_my_identities();
                cx.notify();
            })
            .log_err();
        }));
    }

    fn update_my_identities(&mut self) {
        self.my_identities = None;
        if !self.tab_properties.highlight_my_commits
            || is_filtered_by_me(&self.tab_properties.filters.users)
        {
            return;
        }
        let (Some(user), Some(authors)) = (&self.current_user, &self.authors) else {
            return;
        };
        if is_single_user(user, authors) {
            return;
        }
        self.my_identities = Some(resolve_current_users(user, authors));
    }

    fn set_loading(&mut self, is_loading: bool, cx: &mut Context<Self>) {
        if self.is_loading != is_loading {
            self.is_loading = is_loading;
            cx.emit(CommitTableEvent::LoadingChanged(is_loading));
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.commits.clear();
        self.build_task = None;
        self.rebuild_requested = false;
        self.built_commit_count = 0;
        self.fully_loaded = false;
        self.load_error = None;
        if !self.shown {
            return;
        }
        let Some(repository) = self.repository.clone() else {
            self.displayed = None;
            self.set_loading(false, cx);
            cx.notify();
            return;
        };
        let source = self.log_source();
        let (commits, is_loading, error) = repository.update(cx, |repository, cx| {
            let response = repository.graph_data(source, LOG_ORDER, 0..usize::MAX, cx);
            (
                response.commits.to_vec(),
                response.is_loading,
                response.error,
            )
        });
        self.commits = commits;
        self.load_error = error;
        self.fully_loaded = !is_loading;
        self.set_loading(is_loading, cx);
        if !self.commits.is_empty() || self.fully_loaded || self.load_error.is_some() {
            self.schedule_graph_build(cx);
        }
        cx.notify();
    }

    fn append_commits(
        &mut self,
        repository: &Entity<Repository>,
        count: usize,
        cx: &mut Context<Self>,
    ) {
        let start = self.commits.len();
        if count <= start {
            return;
        }
        let source = self.log_source();
        let (new_commits, is_loading) = repository.update(cx, |repository, cx| {
            let response = repository.graph_data(source, LOG_ORDER, start..count, cx);
            (response.commits.to_vec(), response.is_loading)
        });
        self.commits.extend(new_commits);
        self.set_loading(is_loading, cx);
        // Every build lays out the whole history, so while it streams in, the graph is rebuilt
        // only once the history has doubled: the work stays proportional to its length rather
        // than its square, and fewer graphs are alive at once. Loading ends with a full build.
        if self.commits.len() >= 2 * self.built_commit_count {
            self.schedule_graph_build(cx);
        }
    }

    fn schedule_graph_build(&mut self, cx: &mut Context<Self>) {
        if self.build_task.is_some() {
            self.rebuild_requested = true;
            return;
        }
        let commits = Arc::new(self.commits.clone());
        self.built_commit_count = commits.len();
        let options = self.tab_properties.graph_options;
        let show_long_edges = self.tab_properties.show_long_edges;
        let remote_names = self.ref_context.remote_names.clone();
        let compute_head_containment = self.filter == LogFilter::default();
        let build = cx.background_spawn({
            let commits = commits.clone();
            async move {
                let graph =
                    CommitGraph::new(commits.as_slice(), options, show_long_edges, &remote_names);
                let contained_in_head = compute_head_containment.then(|| {
                    presentation::commits_reachable_from_head(commits.as_slice(), |sha| {
                        graph.commit_index(sha)
                    })
                });
                (graph, contained_in_head)
            }
        });
        self.build_task = Some(cx.spawn(async move |this, cx| {
            let (graph, contained_in_head) = build.await;
            this.update(cx, |this, cx| {
                this.finish_graph_build(commits, graph, contained_in_head, cx)
            })
            .log_err();
        }));
    }

    fn finish_graph_build(
        &mut self,
        commits: Arc<Vec<Arc<InitialGraphCommitData>>>,
        graph: CommitGraph,
        contained_in_head: Option<Vec<bool>>,
        cx: &mut Context<Self>,
    ) {
        self.build_task = None;
        let anchor = self.scroll_anchor();
        self.displayed = Some(DisplayedLog {
            graph,
            commits,
            contained_in_head,
        });
        self.hovered_arrow = None;
        self.restore_scroll_anchor(anchor);
        if self.fully_loaded && !self.rebuild_requested {
            let present: HashSet<Oid> = self
                .selection
                .iter()
                .copied()
                .filter(|sha| self.row_of(sha).is_some())
                .collect();
            self.selection = present;
            if self
                .lead
                .is_some_and(|lead| !self.selection.contains(&lead))
            {
                self.lead = None;
            }
            if self
                .anchor
                .is_some_and(|anchor| !self.selection.contains(&anchor))
            {
                self.anchor = self.lead;
            }
        }
        self.selection_changed(cx);
        self.resolve_pending_jump(None, cx);
        if self.rebuild_requested {
            self.rebuild_requested = false;
            self.schedule_graph_build(cx);
        }
        cx.notify();
    }

    fn row_height(&self) -> Pixels {
        self.frame
            .as_ref()
            .map_or(MINIMUM_ROW_HEIGHT, |frame| frame.row_height)
    }

    fn scroll_anchor(&self) -> Option<ScrollAnchor> {
        let displayed = self.displayed.as_ref()?;
        let row_height = self.row_height();
        let row_count = displayed.graph.row_count();
        if row_count == 0 || row_height <= Pixels::ZERO {
            return None;
        }
        let scroll_top = -self.scroll_handle.0.borrow().base_handle.offset().y;
        let first_visible = ((scroll_top / row_height).floor().max(0.) as usize).min(row_count - 1);
        let viewport_height = self
            .table_bounds
            .get()
            .map_or(Pixels::ZERO, |bounds| bounds.size.height);
        let visible_count = ((viewport_height / row_height).ceil().max(1.) as usize).max(1);
        let visible_end = (first_visible + visible_count).min(row_count);
        let target_row = (first_visible..visible_end)
            .find(|row| {
                displayed
                    .graph
                    .sha_at_row(*row)
                    .is_some_and(|sha| self.selection.contains(&sha))
            })
            .unwrap_or(first_visible);
        let sha = displayed.graph.sha_at_row(target_row)?;
        Some(ScrollAnchor {
            sha,
            top_gap: row_height * target_row as f32 - scroll_top,
            at_top: scroll_top <= Pixels::ZERO,
        })
    }

    fn restore_scroll_anchor(&self, anchor: Option<ScrollAnchor>) {
        let Some(anchor) = anchor else {
            return;
        };
        if anchor.at_top {
            return;
        }
        let Some(row) = self.row_of(&anchor.sha) else {
            return;
        };
        let base_handle = self.scroll_handle.0.borrow().base_handle.clone();
        let offset = base_handle.offset();
        let top = (self.row_height() * row as f32 - anchor.top_gap).max(Pixels::ZERO);
        base_handle.set_offset(point(offset.x, -top));
    }

    fn row_count(&self) -> usize {
        self.displayed
            .as_ref()
            .map_or(0, |displayed| displayed.graph.row_count())
    }

    fn sha_at_row(&self, row: usize) -> Option<Oid> {
        self.displayed.as_ref()?.graph.sha_at_row(row)
    }

    fn row_of(&self, sha: &Oid) -> Option<usize> {
        self.displayed.as_ref()?.graph.row_for_sha(sha)
    }

    fn selected_rows(&self) -> Vec<usize> {
        let mut rows: Vec<usize> = self
            .selection
            .iter()
            .filter_map(|sha| self.row_of(sha))
            .collect();
        rows.sort_unstable();
        rows
    }

    fn lead_row(&self) -> Option<usize> {
        self.lead.and_then(|sha| self.row_of(&sha))
    }

    fn anchor_row(&self) -> Option<usize> {
        self.anchor
            .and_then(|sha| self.row_of(&sha))
            .or_else(|| self.lead_row())
    }

    fn select_only_row(&mut self, row: usize) {
        let Some(sha) = self.sha_at_row(row) else {
            return;
        };
        self.selection.clear();
        self.selection.insert(sha);
        self.lead = Some(sha);
        self.anchor = Some(sha);
    }

    fn select_range(&mut self, anchor_row: usize, lead_row: usize, add: bool) {
        if !add {
            self.selection.clear();
        }
        let range = anchor_row.min(lead_row)..=anchor_row.max(lead_row);
        for row in range {
            if let Some(sha) = self.sha_at_row(row) {
                self.selection.insert(sha);
            }
        }
        self.anchor = self.sha_at_row(anchor_row);
        self.lead = self.sha_at_row(lead_row);
    }

    fn selection_changed(&mut self, cx: &mut Context<Self>) {
        let commits = self.selected_commits();
        if commits != self.last_emitted_selection {
            self.last_emitted_selection = commits.clone();
            cx.emit(CommitTableEvent::SelectionChanged(commits));
        }
        cx.notify();
    }

    fn select_and_reveal(&mut self, row: usize, strategy: ScrollStrategy, cx: &mut Context<Self>) {
        self.select_only_row(row);
        self.scroll_handle.scroll_to_item(row, strategy);
        self.selection_changed(cx);
    }

    fn jump_to_row(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.select_only_row(row);
        self.scroll_handle
            .scroll_to_item_strict(row, ScrollStrategy::Center);
        window.focus(&self.focus_handle, cx);
        self.selection_changed(cx);
    }

    fn visible_row_count(&self) -> usize {
        let row_height = self.row_height();
        let viewport_height = self
            .table_bounds
            .get()
            .map_or(Pixels::ZERO, |bounds| bounds.size.height);
        if row_height <= Pixels::ZERO {
            return 0;
        }
        (viewport_height / row_height).floor().max(0.) as usize
    }

    fn first_visible_row(&self) -> usize {
        let row_height = self.row_height();
        if row_height <= Pixels::ZERO {
            return 0;
        }
        let scroll_top = -self.scroll_handle.0.borrow().base_handle.offset().y;
        (scroll_top / row_height).floor().max(0.) as usize
    }

    fn move_lead(&mut self, direction: isize, extend: bool, cx: &mut Context<Self>) {
        let row_count = self.row_count();
        if row_count == 0 {
            return;
        }
        let target = match self.lead_row() {
            Some(lead) => lead as isize + direction,
            None => direction - 1,
        };
        if target < 0 || target >= row_count as isize {
            return;
        }
        let target = target as usize;
        if extend {
            let anchor = self.anchor_row().unwrap_or(target);
            self.select_range(anchor, target, false);
        } else {
            self.select_only_row(target);
        }
        self.scroll_handle
            .scroll_to_item(target, ScrollStrategy::Nearest);
        self.selection_changed(cx);
    }

    fn move_page(&mut self, down: bool, extend: bool, cx: &mut Context<Self>) {
        let row_count = self.row_count();
        if row_count == 0 {
            return;
        }
        let visible = self.visible_row_count();
        let last_row = row_count - 1;
        if visible == 0 {
            let target = if down { last_row } else { 0 };
            self.select_and_reveal(target, ScrollStrategy::Nearest, cx);
            return;
        }
        let step = visible - 1;
        let first_visible = self.first_visible_row();
        let top = if down {
            (first_visible + step).min(last_row)
        } else {
            first_visible.saturating_sub(step)
        };
        let base = if extend {
            self.lead_row()
        } else {
            self.selected_rows().first().copied()
        }
        .unwrap_or(0);
        let target = if down {
            (base + step).min(last_row)
        } else {
            base.saturating_sub(step)
        };
        self.scroll_handle
            .scroll_to_item_strict(top, ScrollStrategy::Top);
        if extend {
            let anchor = self.anchor_row().unwrap_or(target);
            self.select_range(anchor, target, false);
            self.selection_changed(cx);
        } else {
            self.select_only_row(target);
            self.selection_changed(cx);
        }
    }

    fn extend_to_edge(&mut self, last: bool, cx: &mut Context<Self>) {
        let row_count = self.row_count();
        if row_count == 0 {
            return;
        }
        let target = if last { row_count - 1 } else { 0 };
        let anchor = self.anchor_row().unwrap_or(target);
        self.select_range(anchor, target, false);
        self.scroll_handle
            .scroll_to_item(target, ScrollStrategy::Nearest);
        self.selection_changed(cx);
    }

    fn select_previous(
        &mut self,
        _: &menu::SelectPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_lead(-1, false, cx);
    }

    fn select_next(&mut self, _: &menu::SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_lead(1, false, cx);
    }

    fn select_first(&mut self, _: &menu::SelectFirst, _: &mut Window, cx: &mut Context<Self>) {
        if self.row_count() > 0 {
            self.select_and_reveal(0, ScrollStrategy::Nearest, cx);
        }
    }

    fn select_last(&mut self, _: &menu::SelectLast, _: &mut Window, cx: &mut Context<Self>) {
        let row_count = self.row_count();
        if row_count > 0 {
            self.select_and_reveal(row_count - 1, ScrollStrategy::Nearest, cx);
        }
    }

    fn select_page_up(&mut self, _: &SelectPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_page(false, false, cx);
    }

    fn select_page_down(&mut self, _: &SelectPageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_page(true, false, cx);
    }

    fn extend_selection_up(
        &mut self,
        _: &ExtendSelectionUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_lead(-1, true, cx);
    }

    fn extend_selection_down(
        &mut self,
        _: &ExtendSelectionDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_lead(1, true, cx);
    }

    fn extend_selection_page_up(
        &mut self,
        _: &ExtendSelectionPageUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_page(false, true, cx);
    }

    fn extend_selection_page_down(
        &mut self,
        _: &ExtendSelectionPageDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_page(true, true, cx);
    }

    fn extend_selection_to_first(
        &mut self,
        _: &ExtendSelectionToFirst,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extend_to_edge(false, cx);
    }

    fn extend_selection_to_last(
        &mut self,
        _: &ExtendSelectionToLast,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extend_to_edge(true, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let row_count = self.row_count();
        if row_count == 0 {
            return;
        }
        self.select_range(0, row_count - 1, false);
        self.selection_changed(cx);
    }

    fn copy_revision_number(
        &mut self,
        _: &CopyRevisionNumber,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let hashes: Vec<String> = self
            .selected_commits()
            .into_iter()
            .rev()
            .map(|sha| sha.to_string())
            .collect();
        if hashes.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(hashes.join(" ")));
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let columns = visible_columns(&self.app_properties.column_order);
        let lines: Vec<String> = self
            .selected_rows()
            .into_iter()
            .take(MAX_SELECTED_COMMITS)
            .filter_map(|row| {
                let sha = self.sha_at_row(row)?;
                let data = self.metadata.get(&sha);
                Some(
                    columns
                        .iter()
                        .map(|column| self.column_text(*column, &sha, data).to_string())
                        .collect::<Vec<_>>()
                        .join(" "),
                )
            })
            .collect();
        if lines.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(lines.join("\n")));
    }

    fn column_text(
        &self,
        column: LogColumn,
        sha: &Oid,
        data: Option<&Arc<CommitData>>,
    ) -> SharedString {
        match (column, data) {
            (LogColumn::Root, _) => SharedString::default(),
            (LogColumn::Hash, _) => short_hash(sha),
            (LogColumn::Commit, Some(data)) => subject_text(&data.subject),
            (LogColumn::Commit, None) => SharedString::new_static(LOADING_SUBJECT),
            (LogColumn::Author, Some(data)) => author_presentation(data),
            (LogColumn::Date, Some(data)) => {
                let timestamp = if self.app_properties.prefer_commit_date {
                    data.committer_timestamp
                } else {
                    data.author_timestamp
                };
                if timestamp < 0 {
                    SharedString::default()
                } else {
                    log_dates::format_pretty_date_time(timestamp)
                }
            }
            (LogColumn::Author | LogColumn::Date, None) => SharedString::default(),
        }
    }

    fn rows_to_jump(&self, parents: bool) -> Vec<usize> {
        let rows = self.selected_rows();
        let ([row], Some(displayed)) = (rows.as_slice(), self.displayed.as_ref()) else {
            return Vec::new();
        };
        let mut adjacent = displayed.graph.adjacent_rows(*row, parents);
        adjacent.sort_unstable();
        adjacent
    }

    fn go_to_adjacent(&mut self, parents: bool, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows_to_jump(parents);
        match rows.as_slice() {
            [] => {}
            [row] => self.jump_to_row(*row, window, cx),
            _ => self.deploy_adjacent_menu(parents, rows, window, cx),
        }
    }

    fn deploy_adjacent_menu(
        &mut self,
        parents: bool,
        rows: Vec<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entries: Vec<(usize, String)> = rows
            .into_iter()
            .filter_map(|row| {
                let sha = self.sha_at_row(row)?;
                let data = self.metadata.get(&sha).map(|data| data.as_ref());
                Some((row, go_to_action_text(&sha, data)))
            })
            .collect();
        let title = if parents {
            SELECT_PARENT_TITLE
        } else {
            SELECT_CHILD_TITLE
        };
        let position = self
            .lead_row()
            .and_then(|row| self.cell_bounds.borrow().get(&row).copied())
            .map_or_else(|| window.mouse_position(), |bounds| bounds.bottom_left());
        let table = cx.weak_entity();
        let focus_handle = self.focus_handle.clone();
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            menu = menu.context(focus_handle).header(title);
            for (row, text) in entries {
                let table = table.clone();
                menu = menu.entry(text, None, move |window, cx| {
                    table
                        .update(cx, |this, cx| this.jump_to_row(row, window, cx))
                        .log_err();
                });
            }
            menu
        });
        self.show_context_menu(menu, position, window, cx);
    }

    fn go_to_parent_commit(
        &mut self,
        _: &GoToParentCommit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go_to_adjacent(true, window, cx);
    }

    fn go_to_child_commit(
        &mut self,
        _: &GoToChildCommit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go_to_adjacent(false, window, cx);
    }

    fn show_diff(&mut self, _: &ShowDiff, window: &mut Window, cx: &mut Context<Self>) {
        let commits = self.selected_commits();
        let ([sha], Some(repository)) = (commits.as_slice(), self.repository.as_ref()) else {
            return;
        };
        CommitView::open(
            sha.to_string(),
            repository.downgrade(),
            self.workspace.clone(),
            None,
            None,
            window,
            cx,
        );
    }

    fn cherry_pick(&mut self, _: &CherryPick, window: &mut Window, cx: &mut Context<Self>) {
        self.cherry_pick_selected(window, cx);
    }

    fn new_branch_from_commit(
        &mut self,
        _: &NewBranchFromCommit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let commits = self.selected_menu_commits();
        let ([commit], Some(repository)) = (commits.as_slice(), self.repository.clone()) else {
            return;
        };
        let context = BranchContext::new(self.workspace.clone(), repository);
        operations::new_branch_from_commit(context, commit.clone(), window, cx);
    }

    fn selected_menu_commits(&self) -> Vec<MenuCommit> {
        let Some(displayed) = self.displayed.as_ref() else {
            return Vec::new();
        };
        self.selected_rows()
            .into_iter()
            .take(MAX_SELECTED_COMMITS)
            .filter_map(|row| {
                let index = displayed.graph.commit_index_at_row(row)?;
                let commit = displayed.commits.get(index)?;
                Some(MenuCommit {
                    sha: commit.sha,
                    parents: commit.parents.to_vec(),
                    labels: ref_labels::parse_ref_labels(
                        &commit.ref_names,
                        &self.ref_context.remote_names,
                    ),
                    data: self.metadata.get(&commit.sha).cloned(),
                })
            })
            .collect()
    }

    fn matching_reference(&self, reference: &str, cx: &App) -> Option<Oid> {
        let mut candidates: Vec<(RefLabel, Oid)> = Vec::new();
        for commit in self
            .commits
            .iter()
            .filter(|commit| !commit.ref_names.is_empty())
        {
            for label in
                ref_labels::parse_ref_labels(&commit.ref_names, &self.ref_context.remote_names)
            {
                candidates.push((label, commit.sha));
            }
        }
        if let Some(repository) = &self.repository {
            let repository = repository.read(cx);
            for branch in repository.branch_list.iter() {
                let Some(sha) = branch
                    .most_recent_commit
                    .as_ref()
                    .and_then(|commit| commit.sha.parse::<Oid>().ok())
                else {
                    continue;
                };
                let label = RefLabel {
                    name: SharedString::from(branch.name().to_string()),
                    kind: if branch.is_remote() {
                        ref_labels::RefLabelKind::RemoteBranch
                    } else {
                        ref_labels::RefLabelKind::LocalBranch
                    },
                };
                candidates.push((label, sha));
            }
            if let Some(sha) = repository
                .head_commit
                .as_ref()
                .and_then(|commit| commit.sha.parse::<Oid>().ok())
            {
                candidates.push((
                    RefLabel {
                        name: SharedString::new_static(HEAD_REVISION),
                        kind: ref_labels::RefLabelKind::Head,
                    },
                    sha,
                ));
            }
        }
        let current_branch = self.ref_context.current_branch.as_deref();
        candidates
            .into_iter()
            .filter(|(label, _)| label.name.starts_with(reference))
            .min_by(|(left, _), (right, _)| {
                ref_labels::compare_ref_labels(left, right, current_branch)
            })
            .map(|(_, sha)| sha)
    }

    fn resolve_pending_jump(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let Some(target) = self.pending_jump.clone() else {
            return;
        };
        let Some(displayed) = self.displayed.as_ref() else {
            return;
        };
        let row = match &target {
            JumpTarget::Commit(sha) => displayed.graph.row_for_sha(sha),
            JumpTarget::HashPrefix(prefix) => displayed
                .commits
                .iter()
                .filter(|commit| commit.sha.to_string().starts_with(prefix.as_str()))
                .filter_map(|commit| displayed.graph.row_for_sha(&commit.sha))
                .min(),
        };
        let fully_built = displayed.commits.len() >= self.commits.len();
        if let Some(row) = row {
            self.pending_jump = None;
            match window {
                Some(window) => self.jump_to_row(row, window, cx),
                None => {
                    self.select_only_row(row);
                    self.scroll_handle
                        .scroll_to_item_strict(row, ScrollStrategy::Center);
                    self.selection_changed(cx);
                }
            }
            return;
        }
        if !self.fully_loaded || self.build_task.is_some() || !fully_built {
            return;
        }
        self.pending_jump = None;
        let message = match &target {
            JumpTarget::Commit(sha) => not_found_message(
                &commit_presentation(sha),
                self.filter != LogFilter::default(),
            ),
            JumpTarget::HashPrefix(prefix) => {
                not_found_message(&reference_presentation(prefix), false)
            }
        };
        self.notify_warning(message, cx);
    }

    fn notify_warning(&self, message: String, cx: &mut App) {
        let Some(repository) = self.repository.clone() else {
            return;
        };
        BranchContext::new(self.workspace.clone(), repository)
            .notify(BranchNotice::warning(message), cx);
    }

    fn graph_element_at(
        &self,
        row: usize,
        position: Point<Pixels>,
        window: &Window,
    ) -> Option<PrintElement> {
        let displayed = self.displayed.as_ref()?;
        let bounds = self.cell_bounds.borrow().get(&row).copied()?;
        let elements = displayed
            .graph
            .print_elements(row..row + 1, None)
            .into_iter()
            .next()?;
        let position_in_cell = position - bounds.origin;
        element_at(
            &elements,
            position_in_cell,
            self.row_height(),
            window.scale_factor(),
        )
        .copied()
    }

    fn handle_row_mouse_move(
        &mut self,
        row: usize,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.column_resize.is_some() {
            return;
        }
        let arrow = self
            .graph_element_at(row, event.position, window)
            .filter(PrintElement::has_arrow);
        if self.hovered_row != Some(row) || self.hovered_arrow != arrow {
            self.hovered_row = Some(row);
            self.hovered_arrow = arrow;
            cx.notify();
        }
    }

    fn handle_row_hover(&mut self, row: usize, hovered: bool, cx: &mut Context<Self>) {
        if !hovered && self.hovered_row == Some(row) {
            self.hovered_row = None;
            self.hovered_arrow = None;
            cx.notify();
        }
    }

    fn handle_row_mouse_down(
        &mut self,
        row: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        if let Some(element) = self.graph_element_at(row, event.position, window) {
            if element.has_arrow()
                && let Some(target) = self
                    .displayed
                    .as_ref()
                    .and_then(|displayed| displayed.graph.arrow_target(&element))
                && let Some(target_row) = self.row_of(&target)
            {
                self.jump_to_row(target_row, window, cx);
            }
            return;
        }
        let Some(sha) = self.sha_at_row(row) else {
            return;
        };
        if event.modifiers.shift {
            let anchor = self.anchor_row().unwrap_or(row);
            self.select_range(anchor, row, event.modifiers.secondary());
        } else if event.modifiers.secondary() {
            if !self.selection.remove(&sha) {
                self.selection.insert(sha);
            }
            self.lead = Some(sha);
            self.anchor = Some(sha);
        } else {
            self.select_only_row(row);
        }
        self.selection_changed(cx);
    }

    fn handle_row_secondary_mouse_down(
        &mut self,
        row: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        if self.selected_rows().len() < 2 {
            self.select_only_row(row);
            self.selection_changed(cx);
        }
        self.deploy_context_menu(event.position, window, cx);
        cx.stop_propagation();
    }

    fn deploy_context_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repository) = self.repository.clone() else {
            return;
        };
        let request = CommitMenuRequest {
            workspace: self.workspace.clone(),
            repository,
            focus_handle: self.focus_handle.clone(),
            commits: self.selected_menu_commits(),
            cherry_pick_enabled: self.can_cherry_pick(),
            can_go_to_parent: !self.rows_to_jump(true).is_empty(),
            can_go_to_child: !self.rows_to_jump(false).is_empty(),
        };
        let menu = context_menu::commit_context_menu(request, window, cx);
        self.show_context_menu(menu, position, window, cx);
    }

    fn show_context_menu(
        &mut self,
        menu: Entity<ContextMenu>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&menu.focus_handle(cx), cx);
        let subscription =
            cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
                if this
                    .context_menu
                    .as_ref()
                    .is_some_and(|(menu, _, _)| menu.focus_handle(cx).contains_focused(window, cx))
                {
                    window.focus(&this.focus_handle, cx);
                }
                this.context_menu = None;
                cx.notify();
            });
        self.context_menu = Some((menu, position, subscription));
        cx.notify();
    }

    fn start_column_resize(&mut self, border: usize, mouse_x: Pixels) {
        let Some(frame) = self.frame.as_ref() else {
            return;
        };
        let Some(column) = frame.columns.get(border) else {
            return;
        };
        let table_left = self
            .table_bounds
            .get()
            .map_or(Pixels::ZERO, |bounds| bounds.origin.x);
        self.column_resize = Some(ColumnResize {
            border,
            grab_offset: mouse_x - table_left - column.right(),
            start_columns: frame.columns.clone(),
        });
    }

    fn drag_column_border(
        &mut self,
        event: &DragMoveEvent<DraggedColumnBorder>,
        cx: &mut Context<Self>,
    ) {
        let Some(resize) = self.column_resize.as_ref() else {
            return;
        };
        let Some(column) = resize.start_columns.get(resize.border) else {
            return;
        };
        let table_left = self
            .table_bounds
            .get()
            .map_or(Pixels::ZERO, |bounds| bounds.origin.x);
        let border_x = event.event.position.x - table_left - resize.grab_offset;
        let updates = resize_border(
            &resize.start_columns,
            resize.border,
            border_x - column.right(),
        );
        self.live_widths = Some(merge_widths(&self.tab_properties.column_widths, &updates));
        cx.notify();
    }

    fn finish_column_resize(&mut self, cx: &mut Context<Self>) {
        if self.column_resize.take().is_none() {
            return;
        }
        if let Some(widths) = self.live_widths.take() {
            self.tab_properties.column_widths = widths.clone();
            cx.emit(CommitTableEvent::ColumnWidthsChanged(widths));
        }
        cx.notify();
    }

    fn reset_column_width(&mut self, border: usize, cx: &mut Context<Self>) {
        let Some(column) = self
            .frame
            .as_ref()
            .and_then(|frame| reset_target(&frame.columns, border))
        else {
            return;
        };
        if self
            .tab_properties
            .column_widths
            .iter()
            .any(|(persisted, _)| *persisted == column)
        {
            let widths = without_width(&self.tab_properties.column_widths, column);
            self.tab_properties.column_widths = widths.clone();
            cx.emit(CommitTableEvent::ColumnWidthsChanged(widths));
        }
        self.initialized_columns.remove(&column);
        self.data_widths.remove(&column);
        cx.notify();
    }

    fn compute_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) -> FrameLayout {
        let theme_settings = ThemeSettings::get_global(cx);
        let font = theme_settings.ui_font.clone();
        // IntelliJ's log table uses the 13 px label font and its reference labels one point less.
        let font_size = TextSize::Default.pixels(cx);
        let label_font_size = font_size - px(1.);
        let font_id = window.text_system().resolve_font(&font);
        let text_height = font_height(window, font_id, font_size);
        let label_style = RefLabelsStyle {
            font: font.clone(),
            font_size: label_font_size,
            compact: self.app_properties.compact_references,
        };
        if self
            .frame
            .as_ref()
            .is_none_or(|frame| frame.label_style != label_style)
        {
            self.labels_height.set(Pixels::ZERO);
        }
        let raw_row_height = self
            .labels_height
            .get()
            .max(text_height + VERTICAL_PADDING)
            .max(MINIMUM_ROW_HEIGHT);
        let scale_factor = window.scale_factor();
        let row_height = (raw_row_height * scale_factor).round() / scale_factor;
        let colors = table_colors(cx);
        let table_width = self
            .table_bounds
            .get()
            .map_or_else(|| window.viewport_size().width, |bounds| bounds.size.width);
        let visible = visible_columns(&self.app_properties.column_order);
        self.update_data_widths(&visible, &font, font_size, window);
        let widths = self
            .live_widths
            .clone()
            .unwrap_or_else(|| self.tab_properties.column_widths.clone());
        let columns = layout_columns(&visible, table_width, |_, column| {
            persisted_width(&widths, column, table_width).unwrap_or_else(|| {
                self.data_widths
                    .get(&column)
                    .copied()
                    .unwrap_or(DEFAULT_COLUMN_WIDTH)
            })
        });
        FrameLayout {
            row_height,
            font,
            font_size,
            columns,
            colors,
            label_style,
        }
    }

    fn update_data_widths(
        &mut self,
        visible: &[LogColumn],
        font: &Font,
        font_size: Pixels,
        window: &mut Window,
    ) {
        let mut results: Vec<(LogColumn, Pixels, bool)> = Vec::new();
        for (index, column) in visible.iter().enumerate() {
            if !is_dynamic(*column) || self.initialized_columns.contains(column) {
                continue;
            }
            let (leading, trailing) = column_gaps(index, visible.len());
            let gaps = leading + trailing;
            if *column == LogColumn::Hash {
                let bold_font = Font {
                    weight: FontWeight::BOLD,
                    ..font.clone()
                };
                let width = text_width(HASH_WIDTH_SAMPLE, &bold_font, font_size, window)
                    + TEXT_HORIZONTAL_PADDING
                    + gaps;
                results.push((*column, width, true));
                continue;
            }
            let Some(displayed) = self.displayed.as_ref() else {
                continue;
            };
            let checked = displayed.graph.row_count().min(MAX_ROWS_TO_CALCULATE_WIDTH);
            if checked == 0 {
                continue;
            }
            let mut max_width = Pixels::ZERO;
            let mut unloaded = 0;
            for row in 0..checked {
                let Some(sha) = displayed.graph.sha_at_row(row) else {
                    unloaded += 1;
                    continue;
                };
                let Some(data) = self.metadata.get(&sha) else {
                    unloaded += 1;
                    continue;
                };
                let text = self.column_text(*column, &sha, Some(data));
                if text.is_empty() {
                    unloaded += 1;
                    continue;
                }
                max_width = max_width.max(text_width(&format!("{text}*"), font, font_size, window));
            }
            results.push((
                *column,
                data_width(max_width, gaps),
                unloaded * 2 <= checked,
            ));
        }
        for (column, width, initialized) in results {
            self.data_widths.insert(column, width);
            if initialized {
                self.initialized_columns.insert(column);
            }
        }
    }

    fn prefetch_metadata(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else {
            return;
        };
        let Some(displayed) = self.displayed.as_ref() else {
            return;
        };
        let row_count = displayed.graph.row_count();
        let start = range.start.saturating_sub(UP_PRELOAD_COUNT);
        let end = range.end.saturating_add(DOWN_PRELOAD_COUNT).min(row_count);
        let mut shas: Vec<Oid> = (start..end)
            .filter_map(|row| displayed.graph.sha_at_row(row))
            .filter(|sha| !self.metadata.contains_key(sha))
            .collect();
        if let Some(target) = self
            .hovered_arrow
            .as_ref()
            .and_then(|arrow| displayed.graph.arrow_target(arrow))
            .filter(|target| !self.metadata.contains_key(target))
        {
            shas.push(target);
        }
        if shas.is_empty() {
            return;
        }
        let loaded: Vec<(Oid, Arc<CommitData>)> = repository.update(cx, |repository, cx| {
            shas.into_iter()
                .filter_map(|sha| match repository.fetch_commit_data(sha, false, cx) {
                    CommitDataState::Loaded(data) => Some((sha, data.clone())),
                    CommitDataState::Loading(_) => None,
                })
                .collect()
        });
        self.metadata.extend(loaded);
    }

    fn current_branch_highlight_enabled(&self) -> bool {
        if !self.tab_properties.highlight_current_branch {
            return false;
        }
        let Some(current_branch) = self.ref_context.current_branch.as_ref() else {
            return false;
        };
        match single_filtered_branch(&self.filter.revisions, &self.branch_names) {
            Some(branch) => branch.as_ref() != HEAD_REVISION && &branch != current_branch,
            None => true,
        }
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        self.prefetch_metadata(range.clone(), cx);
        let Some(frame) = self.frame.clone() else {
            return Vec::new();
        };
        let Some(displayed) = self.displayed.as_ref() else {
            return Vec::new();
        };
        let row_count = displayed.graph.row_count();
        let range = range.start.min(row_count)..range.end.min(row_count);
        let focused = self.focus_handle.contains_focused(window, cx);
        let selection_color = if focused {
            frame.colors.selection
        } else {
            frame.colors.inactive_selection
        };
        let print_rows = displayed
            .graph
            .print_elements(range.clone(), self.hovered_arrow.as_ref());
        let recommended_width = displayed.graph.recommended_width();
        let contained_in_head = self
            .current_branch_highlight_enabled()
            .then_some(displayed.contained_in_head.as_ref())
            .flatten();
        let author_visible = frame
            .columns
            .iter()
            .any(|column| column.column == LogColumn::Author);
        let single_selected_row = self.selection.len() == 1;
        let is_selected = |row: usize| {
            displayed
                .graph
                .sha_at_row(row)
                .is_some_and(|sha| self.selection.contains(&sha))
        };
        let mut elements = Vec::with_capacity(range.len());
        for (offset, row) in range.enumerate() {
            let Some((index, commit)) = displayed
                .graph
                .commit_index_at_row(row)
                .and_then(|index| Some((index, displayed.commits.get(index)?)))
            else {
                continue;
            };
            let data = self.metadata.get(&commit.sha).cloned();
            let selected = is_selected(row);
            let hovered = self.hovered_row == Some(row);
            let mut background = frame.colors.background;
            let mut foreground = frame.colors.foreground;
            let mut bold = false;
            if let Some(data) = &data {
                if !selected
                    && contained_in_head
                        .is_some_and(|contained| contained.get(index) == Some(&true))
                {
                    background = frame.colors.current_branch;
                }
                if !selected
                    && self.tab_properties.highlight_merge_commits
                    && data.parents.len() >= 2
                {
                    foreground = frame.colors.unmatched;
                }
                if let Some(identities) = &self.my_identities
                    && is_my_commit(data, identities)
                {
                    bold = true;
                }
            }
            let overlay = if selected {
                let row_type = selected_row_type(
                    single_selected_row,
                    row > 0 && is_selected(row - 1),
                    is_selected(row + 1),
                );
                Some((selection_color, row_type))
            } else if hovered {
                Some((
                    log_colors::hovered_background_over(background, cx),
                    SelectedRowType::Single,
                ))
            } else {
                None
            };
            let style = RowStyle {
                background,
                foreground,
                bold_author: bold && author_visible,
                bold_all: bold && !author_visible,
                overlay,
            };
            let arrow_tooltip = hovered
                .then_some(self.hovered_arrow)
                .flatten()
                .and_then(|arrow| displayed.graph.arrow_target(&arrow))
                .map(|target| {
                    arrow_tooltip_text(
                        &target,
                        self.metadata.get(&target).map(|data| data.as_ref()),
                    )
                });
            let print_elements = print_rows.get(offset).cloned().unwrap_or_default();
            let element = self.render_row(
                row,
                commit,
                data,
                print_elements,
                recommended_width,
                style,
                arrow_tooltip,
                &frame,
                window,
                cx,
            );
            elements.push(element);
        }
        elements
    }

    #[allow(clippy::too_many_arguments)]
    fn render_row(
        &self,
        row: usize,
        commit: &Arc<InitialGraphCommitData>,
        data: Option<Arc<CommitData>>,
        print_elements: Vec<PrintElement>,
        recommended_width: usize,
        style: RowStyle,
        arrow_tooltip: Option<String>,
        frame: &FrameLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row_height = frame.row_height;
        let background = style.background;
        let overlay = style.overlay;
        let background_layer = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                window.paint_quad(fill(bounds, background));
                if let Some((color, row_type)) = overlay {
                    let overlay_bounds = Bounds::from_corners(
                        point(bounds.left() + SELECTION_INSET, bounds.top()),
                        point(bounds.right() - SELECTION_INSET, bounds.bottom()),
                    );
                    let top = if row_type.rounds_top() {
                        SELECTION_RADIUS
                    } else {
                        Pixels::ZERO
                    };
                    let bottom = if row_type.rounds_bottom() {
                        SELECTION_RADIUS
                    } else {
                        Pixels::ZERO
                    };
                    let mut quad = fill(overlay_bounds, color);
                    quad.corner_radii = Corners {
                        top_left: top,
                        top_right: top,
                        bottom_right: bottom,
                        bottom_left: bottom,
                    };
                    window.paint_quad(quad);
                }
            },
        )
        .absolute()
        .size_full();
        let mut cells = Vec::with_capacity(frame.columns.len());
        for column in &frame.columns {
            match column.column {
                LogColumn::Root => {}
                LogColumn::Commit => cells.push(self.render_commit_cell(
                    row,
                    commit,
                    data.as_deref(),
                    print_elements.clone(),
                    recommended_width,
                    style,
                    column,
                    frame,
                    window,
                    cx,
                )),
                other => {
                    let text = self.column_text(other, &commit.sha, data.as_ref());
                    let bold = style.bold_all || (other == LogColumn::Author && style.bold_author);
                    cells.push(
                        div()
                            .flex_none()
                            .w(column.width)
                            .h_full()
                            .pl(column.leading_gap + TEXT_LEFT_PADDING)
                            .pr(column.trailing_gap)
                            .flex()
                            .items_center()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_color(style.foreground)
                            .when(bold, |this| this.font_weight(FontWeight::BOLD))
                            .child(text)
                            .into_any_element(),
                    );
                }
            }
        }
        div()
            .id(("git-log-commit-row", row))
            .relative()
            .h(row_height)
            .w_full()
            .child(background_layer)
            .child(h_flex().absolute().size_full().children(cells))
            .when_some(arrow_tooltip, |this, text| {
                this.cursor_pointer().tooltip(Tooltip::text(text))
            })
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                this.handle_row_hover(row, *hovered, cx)
            }))
            .on_mouse_move(
                cx.listener(move |this, event: &MouseMoveEvent, window, cx| {
                    this.handle_row_mouse_move(row, event, window, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.handle_row_mouse_down(row, event, window, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.handle_row_secondary_mouse_down(row, event, window, cx)
                }),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_commit_cell(
        &self,
        row: usize,
        commit: &Arc<InitialGraphCommitData>,
        data: Option<&CommitData>,
        print_elements: Vec<PrintElement>,
        recommended_width: usize,
        style: RowStyle,
        column: &ColumnBounds,
        frame: &FrameLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let graph_width = graph_width(&print_elements, recommended_width, frame.row_height);
        let subject = data
            .map(|data| subject_text(&data.subject))
            .unwrap_or_else(|| SharedString::new_static(LOADING_SUBJECT));
        let match_ranges = data
            .and_then(|_| self.filter.text_filter())
            .map(|text_filter| text_filter.matching_ranges(&subject))
            .unwrap_or_default();
        let font = if style.bold_all {
            Font {
                weight: FontWeight::BOLD,
                ..frame.font.clone()
            }
        } else {
            frame.font.clone()
        };
        let runs = subject_runs(
            &subject,
            &font,
            style.foreground,
            &match_ranges,
            frame.colors.search_match_foreground,
        );
        let shaped = window
            .text_system()
            .shape_line(subject, frame.font_size, &runs, None);
        let labels =
            ref_labels::parse_ref_labels(&commit.ref_names, &self.ref_context.remote_names);
        let groups: Vec<RefLabelGroup> = ref_labels::group_ref_labels(
            &labels,
            &self.ref_context,
            self.app_properties.compact_references,
            self.app_properties.show_tag_names,
        );
        let column_width = column.width;
        let content_width =
            (column.width - column.leading_gap - column.trailing_gap).max(Pixels::ZERO);
        let labels_layout = (!groups.is_empty())
            .then(|| {
                let text_and_labels_width = column_width - graph_width;
                let free_space = text_and_labels_width
                    - (graph_width + shaped.width() + TEXT_HORIZONTAL_PADDING);
                let available_width = if self.app_properties.compact_references {
                    free_space.min(text_and_labels_width / 3.)
                } else {
                    free_space.max(
                        (text_and_labels_width / 2.)
                            .max(text_and_labels_width - DISPLAYED_MESSAGE_PART),
                    )
                }
                .max(Pixels::ZERO);
                ref_labels::layout_ref_labels(
                    &groups,
                    available_width,
                    &frame.label_style,
                    frame.colors.reference,
                    window,
                    cx,
                )
            })
            .filter(|layout| !layout.is_empty());
        if let Some(layout) = &labels_layout {
            self.labels_height
                .set(self.labels_height.get().max(layout.height()));
        }
        let labels_width = labels_layout
            .as_ref()
            .map_or(Pixels::ZERO, |layout| layout.width());
        let left_aligned = self.app_properties.labels_left_aligned;
        let text_x = if left_aligned && labels_width > Pixels::ZERO {
            graph_width + labels_width + LABELS_RIGHT_PADDING
        } else {
            graph_width
        };
        let labels_x = if left_aligned {
            graph_width
        } else {
            graph_width.max(content_width - labels_width)
        };
        let tooltip_areas = labels_layout
            .as_ref()
            .map(|layout| {
                labels_tooltip_areas(
                    left_aligned,
                    graph_width,
                    column_width,
                    content_width,
                    labels_width,
                    layout.warning_range(),
                )
            })
            .unwrap_or_default();
        let current_branch = self.ref_context.current_branch.as_deref();
        let mut tooltip_labels = labels;
        tooltip_labels
            .sort_by(|left, right| ref_labels::compare_ref_labels(left, right, current_branch));
        let tooltips: Vec<AnyElement> = tooltip_areas
            .into_iter()
            .enumerate()
            .map(|(index, (area, tooltip))| {
                let area_element = div()
                    .id(("git-log-labels-tooltip", index))
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(column.leading_gap + area.start)
                    .w(area.end - area.start);
                match tooltip {
                    LabelsTooltip::References => {
                        let labels = tooltip_labels.clone();
                        area_element
                            .tooltip(move |window, cx| {
                                ref_labels::references_tooltip(labels.clone(), window, cx)
                            })
                            .into_any_element()
                    }
                    LabelsTooltip::DetachedHead => area_element
                        .tooltip(Tooltip::text(ref_labels::DETACHED_HEAD_TOOLTIP))
                        .into_any_element(),
                }
            })
            .collect();
        let visible_background = style.visible_background();
        let palette = frame.colors.palette;
        let search_match_background = frame.colors.search_match_background;
        let cell_bounds = self.cell_bounds.clone();
        div()
            .relative()
            .flex_none()
            .w(column.width)
            .h_full()
            .pl(column.leading_gap)
            .pr(column.trailing_gap)
            .child(
                canvas(
                    move |bounds, _, _| {
                        cell_bounds.borrow_mut().insert(row, bounds);
                    },
                    move |bounds, _, window, cx| {
                        window.with_content_mask(
                            Some(ContentMask {
                                bounds,
                                ..Default::default()
                            }),
                            |window| {
                                let text_origin = point(bounds.left() + text_x, bounds.top());
                                for range in &match_ranges {
                                    let start = text_origin.x + shaped.x_for_index(range.start);
                                    let end = text_origin.x + shaped.x_for_index(range.end);
                                    let mut quad = fill(
                                        Bounds::from_corners(
                                            point(start, bounds.top() + SEARCH_MATCH_INSET),
                                            point(end, bounds.bottom() - SEARCH_MATCH_INSET),
                                        ),
                                        search_match_background,
                                    );
                                    quad.corner_radii = Corners::all(SEARCH_MATCH_RADIUS);
                                    window.paint_quad(quad);
                                }
                                shaped
                                    .paint(
                                        text_origin,
                                        bounds.size.height,
                                        TextAlign::Left,
                                        None,
                                        window,
                                        cx,
                                    )
                                    .log_err();
                                if let Some(layout) = labels_layout.as_ref() {
                                    ref_labels::paint_ref_labels(
                                        layout,
                                        Bounds {
                                            origin: point(bounds.left() + labels_x, bounds.top()),
                                            size: size(labels_width, bounds.size.height),
                                        },
                                        visible_background,
                                        window,
                                        cx,
                                    );
                                }
                                paint_graph_row(
                                    &print_elements,
                                    bounds,
                                    visible_background,
                                    &palette,
                                    window,
                                );
                            },
                        );
                    },
                )
                .size_full(),
            )
            .children(tooltips)
            .into_any_element()
    }

    fn status(&self) -> (SharedString, Option<StatusAction>) {
        if self.repository.is_none() {
            return (SharedString::new_static(DEFAULT_STATUS), None);
        }
        if let Some(error) = &self.load_error {
            let message = if error.trim().is_empty() {
                ERROR_LOADING_STATUS.to_string()
            } else {
                presentation::shorten_with_ellipsis(error, ERROR_TEXT_MAX_LENGTH).replace('\n', " ")
            };
            return (message.into(), Some(StatusAction::Refresh));
        }
        if self.is_loading || self.displayed.is_none() {
            return (SharedString::new_static(LOADING_STATUS), None);
        }
        if self.filter == LogFilter::default() {
            (
                SharedString::new_static(NO_COMMITS_STATUS),
                Some(StatusAction::Commit),
            )
        } else {
            (SharedString::new_static(NO_MATCHING_COMMITS_STATUS), None)
        }
    }

    fn render_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let (text, action) = self.status();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_1()
            .child(Label::new(text).color(Color::Muted))
            .when_some(action, |this, action| {
                let label = match action {
                    StatusAction::Refresh => REFRESH_STATUS_ACTION,
                    StatusAction::Commit => COMMIT_STATUS_ACTION,
                };
                this.child(
                    Button::new("git-log-status-action", label)
                        .color(Color::Accent)
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, window, cx| match action {
                                StatusAction::Refresh => this.refresh(cx),
                                StatusAction::Commit => {
                                    window.dispatch_action(Box::new(git::Commit), cx)
                                }
                            },
                        )),
                )
            })
            .into_any_element()
    }

    fn render_column_borders(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(frame) = self.frame.as_ref() else {
            return Vec::new();
        };
        if self.row_count() == 0 {
            return Vec::new();
        }
        resizable_borders(&frame.columns)
            .into_iter()
            .filter_map(|border| {
                let column = frame.columns.get(border)?;
                Some(
                    div()
                        .id(("git-log-column-border", border))
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(column.right() - columns::BORDER_THICKNESS)
                        .w(columns::BORDER_THICKNESS * 2.)
                        .cursor_col_resize()
                        .block_mouse_except_scroll()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                this.start_column_resize(border, event.position.x);
                                cx.stop_propagation();
                            }),
                        )
                        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                            if event.click_count() >= 2 {
                                this.reset_column_width(border, cx);
                            }
                            cx.stop_propagation();
                        }))
                        .on_drag(DraggedColumnBorder, |_, _, _, cx| cx.new(|_| gpui::Empty))
                        .into_any_element(),
                )
            })
            .collect()
    }
}

fn font_height(window: &Window, font_id: gpui::FontId, font_size: Pixels) -> Pixels {
    let ascent = window.text_system().ascent(font_id, font_size);
    let descent = window.text_system().descent(font_id, font_size);
    let descent = if descent < Pixels::ZERO {
        -descent
    } else {
        descent
    };
    ascent + descent
}

fn text_width(text: &str, font: &Font, font_size: Pixels, window: &Window) -> Pixels {
    let run = TextRun {
        len: text.len(),
        font: font.clone(),
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(
            SharedString::from(text.to_string()),
            font_size,
            &[run],
            None,
        )
        .width()
}

fn subject_runs(
    subject: &str,
    font: &Font,
    color: Hsla,
    match_ranges: &[Range<usize>],
    match_color: Hsla,
) -> Vec<TextRun> {
    let run = |len: usize, color: Hsla| TextRun {
        len,
        font: font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let mut runs = Vec::new();
    let mut position = 0;
    for range in match_ranges {
        if range.start < position || range.end > subject.len() || range.start >= range.end {
            continue;
        }
        if range.start > position {
            runs.push(run(range.start - position, color));
        }
        runs.push(run(range.end - range.start, match_color));
        position = range.end;
    }
    if position < subject.len() {
        runs.push(run(subject.len() - position, color));
    }
    runs
}

fn table_colors(cx: &App) -> TableColors {
    let theme = cx.theme();
    let colors = theme.colors();
    let is_light = theme.appearance() == Appearance::Light;
    TableColors {
        background: colors.panel_background,
        foreground: colors.text,
        selection: colors.element_selected,
        inactive_selection: colors.element_background,
        unmatched: log_colors::unmatched_foreground(cx),
        current_branch: log_colors::current_branch_background(cx),
        reference: log_colors::reference_foreground(cx),
        search_match_background: Hsla::from(rgb(if is_light {
            SEARCH_MATCH_LIGHT
        } else {
            SEARCH_MATCH_DARK
        })),
        search_match_foreground: Hsla::from(rgb(if is_light {
            SEARCH_MATCH_TEXT_LIGHT
        } else {
            SEARCH_MATCH_TEXT_DARK
        })),
        palette: GraphPalette::new(theme.appearance(), colors.text),
    }
}

impl Render for CommitTable {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.shown {
            self.shown = true;
            cx.defer_in(window, |this, _, cx| {
                this.load_authors(cx);
                this.reload(cx);
            });
        }
        let frame = self.compute_frame(window, cx);
        let background = frame.colors.background;
        let font = frame.font.clone();
        let font_size = frame.font_size;
        self.frame = Some(frame);
        let row_count = self.row_count();
        let table_bounds = self.table_bounds.clone();
        let entity_id = cx.entity_id();
        let bounds_tracker = canvas(
            move |bounds, window, _| {
                if table_bounds.get() != Some(bounds) {
                    table_bounds.set(Some(bounds));
                    window.on_next_frame(move |_, cx| cx.notify(entity_id));
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let body = if row_count == 0 {
            self.render_status(cx)
        } else {
            div()
                .relative()
                .size_full()
                .child(
                    uniform_list(
                        "git-log-commits",
                        row_count,
                        cx.processor(|this, range: Range<usize>, window, cx| {
                            this.render_rows(range, window, cx)
                        }),
                    )
                    .size_full()
                    .track_scroll(&self.scroll_handle),
                )
                .children(self.render_column_borders(cx))
                .vertical_scrollbar_for(&self.scroll_handle, window, cx)
                .into_any_element()
        };
        v_flex()
            .id("git-log-commit-table")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(background)
            .font(font)
            .text_size(font_size)
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_first))
            .on_action(cx.listener(Self::select_last))
            .on_action(cx.listener(Self::select_page_up))
            .on_action(cx.listener(Self::select_page_down))
            .on_action(cx.listener(Self::extend_selection_up))
            .on_action(cx.listener(Self::extend_selection_down))
            .on_action(cx.listener(Self::extend_selection_page_up))
            .on_action(cx.listener(Self::extend_selection_page_down))
            .on_action(cx.listener(Self::extend_selection_to_first))
            .on_action(cx.listener(Self::extend_selection_to_last))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::copy_revision_number))
            .on_action(cx.listener(Self::go_to_parent_commit))
            .on_action(cx.listener(Self::go_to_child_commit))
            .on_action(cx.listener(Self::show_diff))
            .on_action(cx.listener(Self::cherry_pick))
            .on_action(cx.listener(Self::new_branch_from_commit))
            .on_drag_move::<DraggedColumnBorder>(cx.listener(
                |this, event: &DragMoveEvent<DraggedColumnBorder>, _, cx| {
                    this.drag_column_border(event, cx)
                },
            ))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_column_resize(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_column_resize(cx)),
            )
            .child(bounds_tracker)
            .when(self.is_loading, |this| {
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .w_full()
                        .h(PROGRESS_STRIPE_HEIGHT)
                        .bg(cx.theme().colors().text_accent),
                )
            })
            .child(body)
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
