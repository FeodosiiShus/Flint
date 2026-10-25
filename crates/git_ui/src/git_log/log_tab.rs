mod filters;
mod persistence;
mod splitter;

use git::repository::{GitUser, LogFilter};
use gpui::{
    ClickEvent, DragMoveEvent, Entity, EventEmitter, FocusHandle, Focusable, Global, Subscription,
    Task, WeakEntity, actions,
};
use project::Project;
use project::git_store::{GitStore, GitStoreEvent, Repository, RepositoryEvent};
use ui::prelude::*;
use util::ResultExt as _;
use workspace::Workspace;

use self::splitter::{DraggedLogSplitter, Splitter, SplitterKind};
use super::branches_pane::{self, BranchesPane, BranchesPaneEvent};
use super::changes_browser::{ChangesBrowser, ChangesBrowserEvent};
use super::commit_details::CommitDetailsPanel;
use super::commit_table::{CommitTable, CommitTableEvent};
use super::filter_toolbar::{GoToHashOrRef, LogFilterToolbar, LogFilterToolbarEvent, Refresh};
use super::properties::{LogAppProperties, LogFilterState, LogTabProperties, MAIN_LOG_TAB_ID};

actions!(git_log, [FocusTextFilter, ShowDetails]);

const KEY_CONTEXT: &str = "GitLog";
const DIVIDER_RESET_CLICK_COUNT: usize = 2;
const DIVIDER_RESET_PROPORTION: f32 = 0.5;
const DETAILS_MINIMUM_SIZE: Pixels = px(20.);
const TOOLBAR_MINIMUM_SIZE_FACTOR: f32 = 1.5;
const STATUS_TEXT_LINE_GAP: Pixels = px(2.);
const NO_REPOSITORY_TEXT: &str = "To track changes to code:";
const STATUS_TEXT_LINE_PREFIX: &str = "\u{2014} ";
const CREATE_REPOSITORY_TEXT: &str = "Create Git repository\u{2026}";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GitLogTabEvent {
    TitleChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PropertiesSource {
    Frame,
    Repository,
    BranchesPane,
    FilterToolbar,
    CommitTable,
    ChangesBrowser,
}

struct SharedLogAppProperties(LogAppProperties);

impl Global for SharedLogAppProperties {}

pub(crate) struct GitLogTab {
    tab_id: SharedString,
    repository: Option<Entity<Repository>>,
    focus_handle: FocusHandle,
    app_properties: LogAppProperties,
    tab_properties: LogTabProperties,
    tab_properties_await_repository: bool,
    branch_names: Vec<SharedString>,
    current_user: Option<GitUser>,
    current_user_loaded: bool,
    log_authors: Option<Vec<GitUser>>,
    log_authors_loading: bool,
    applied_filter: Option<LogFilter>,
    table_repository: Option<Entity<Repository>>,
    title: SharedString,
    toolbar_height: Option<Pixels>,
    branches_pane: Entity<BranchesPane>,
    filter_toolbar: Entity<LogFilterToolbar>,
    commit_table: Entity<CommitTable>,
    changes_browser: Entity<ChangesBrowser>,
    commit_details: Entity<CommitDetailsPanel>,
    _users_task: Task<()>,
    app_properties_save: Task<()>,
    tab_properties_save: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl GitLogTab {
    pub(crate) fn new(
        tab_id: SharedString,
        workspace: WeakEntity<Workspace>,
        project: Entity<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let git_store = project.read(cx).git_store().clone();
        let repository = git_store.read(cx).active_repository();
        let is_main_tab = tab_id == MAIN_LOG_TAB_ID;
        let app_properties = match cx.try_global::<SharedLogAppProperties>() {
            Some(shared) => shared.0.clone(),
            None => {
                let properties = persistence::load_app_properties(cx);
                cx.set_global(SharedLogAppProperties(properties.clone()));
                properties
            }
        };
        let tab_properties = match &repository {
            Some(repository) => persistence::load_tab_properties(
                &tab_id,
                &repository.read(cx).work_directory_abs_path,
                is_main_tab,
                cx,
            ),
            None => LogTabProperties::new(is_main_tab),
        };

        let branches_pane = cx.new(|cx| {
            BranchesPane::new(
                workspace.clone(),
                repository.clone(),
                tab_properties.show_branches,
                tab_properties.branches_single_click,
                window,
                cx,
            )
        });
        let filter_toolbar = cx.new(|cx| {
            LogFilterToolbar::new(
                workspace.clone(),
                app_properties.clone(),
                tab_properties.clone(),
                window,
                cx,
            )
        });
        let commit_table = cx.new(|cx| {
            CommitTable::new(
                workspace.clone(),
                app_properties.clone(),
                tab_properties.clone(),
                window,
                cx,
            )
        });
        let changes_browser = cx.new(|cx| ChangesBrowser::new(workspace.clone(), window, cx));
        let commit_details = cx.new(|cx| CommitDetailsPanel::new(workspace, window, cx));
        changes_browser.update(cx, |browser, cx| {
            browser.set_properties(app_properties.clone(), tab_properties.clone(), cx)
        });

        let subscriptions = vec![
            cx.subscribe_in(&git_store, window, Self::handle_git_store_event),
            cx.subscribe_in(&branches_pane, window, Self::handle_branches_pane_event),
            cx.subscribe_in(&filter_toolbar, window, Self::handle_filter_toolbar_event),
            cx.subscribe_in(&commit_table, window, Self::handle_commit_table_event),
            cx.subscribe_in(&changes_browser, window, Self::handle_changes_browser_event),
            cx.observe_global_in::<SharedLogAppProperties>(window, |this, window, cx| {
                let properties = cx.global::<SharedLogAppProperties>().0.clone();
                this.receive_shared_app_properties(properties, window, cx);
            }),
        ];

        let title = filters::title(&tab_properties, &[]);
        let mut this = Self {
            tab_id,
            repository,
            focus_handle: cx.focus_handle(),
            app_properties,
            tab_properties,
            tab_properties_await_repository: false,
            branch_names: Vec::new(),
            current_user: None,
            current_user_loaded: false,
            log_authors: None,
            log_authors_loading: false,
            applied_filter: None,
            table_repository: None,
            title,
            toolbar_height: None,
            branches_pane,
            filter_toolbar,
            commit_table,
            changes_browser,
            commit_details,
            _users_task: Task::ready(()),
            app_properties_save: Task::ready(()),
            tab_properties_save: Task::ready(()),
            _subscriptions: subscriptions,
        };
        let toolbar_height = this.filter_toolbar.read(cx).height(window, cx);
        this.sync_toolbar_height(toolbar_height, cx);
        this.reset_repository_state(cx);
        this.update_log_filter(cx);
        this.update_title(cx);
        this
    }

    pub(crate) fn tab_id(&self) -> &SharedString {
        &self.tab_id
    }

    pub(crate) fn title(&self) -> SharedString {
        self.title.clone()
    }

    pub(crate) fn filters(&self) -> LogFilterState {
        self.tab_properties.filters.clone()
    }

    pub(crate) fn set_filters(
        &mut self,
        filters: LogFilterState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let properties = LogTabProperties {
            filters,
            ..self.tab_properties.clone()
        };
        self.set_tab_properties(properties, PropertiesSource::Frame, window, cx);
    }

    pub(crate) fn set_branch_filter(
        &mut self,
        revisions: Vec<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_filters(
            filters::filter_by_branches(&self.tab_properties.filters, &revisions),
            window,
            cx,
        );
    }

    pub(crate) fn forget_state(&mut self, cx: &mut Context<Self>) {
        self.tab_properties_save = Task::ready(());
        forget_log_tab_state(self.tab_id(), cx);
    }

    fn is_main_tab(&self) -> bool {
        *self.tab_id() == MAIN_LOG_TAB_ID
    }

    fn set_repository(
        &mut self,
        repository: Option<Entity<Repository>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.repository == repository {
            return;
        }
        let carry_over_properties =
            self.repository.is_none() && self.tab_properties_await_repository;
        std::mem::replace(&mut self.tab_properties_save, Task::ready(())).detach();
        self.repository = repository.clone();
        self.tab_properties_await_repository = false;
        self.reset_repository_state(cx);
        let properties = match &self.repository {
            Some(_) if carry_over_properties => self.tab_properties.clone(),
            Some(repository) => persistence::load_tab_properties(
                self.tab_id(),
                &repository.read(cx).work_directory_abs_path,
                self.is_main_tab(),
                cx,
            ),
            None => LogTabProperties::new(self.is_main_tab()),
        };
        self.branches_pane
            .update(cx, |pane, cx| pane.set_repository(repository, window, cx));
        self.set_tab_properties(properties, PropertiesSource::Repository, window, cx);
        if carry_over_properties {
            self.save_tab_properties(cx);
        }
        self.update_log_filter(cx);
        self.update_title(cx);
        cx.notify();
    }

    fn reset_repository_state(&mut self, cx: &mut Context<Self>) {
        let repository = self.repository.clone();
        self.filter_toolbar.update(cx, |toolbar, cx| {
            toolbar.set_repository(repository.clone(), cx);
            toolbar.set_cherry_pick_enabled(false, cx);
        });
        self.changes_browser.update(cx, |browser, cx| {
            browser.set_commits(repository.clone(), Vec::new(), cx)
        });
        self.commit_details.update(cx, |details, cx| {
            details.set_commits(repository.clone(), Vec::new(), cx)
        });
        self.branch_names = repository
            .as_ref()
            .map(|repository| branch_names(repository.read(cx)))
            .unwrap_or_default();
        self.current_user_loaded = false;
        self.log_authors = None;
        self.applied_filter = None;
        self.reload_users(cx);
    }

    fn refresh_repository_state(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.as_ref() else {
            return;
        };
        let branch_names = branch_names(repository.read(cx));
        if branch_names != self.branch_names {
            self.branch_names = branch_names;
            self.update_log_filter(cx);
            self.update_title(cx);
        }
        if !self.tab_properties.filters.users.is_empty() {
            self.reload_users(cx);
        }
    }

    fn reload_users(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else {
            self._users_task = Task::ready(());
            self.log_authors_loading = false;
            return;
        };
        let load_authors = !self.tab_properties.filters.users.is_empty();
        self.log_authors_loading = load_authors;
        let current_user = repository.read(cx).current_user(cx);
        let log_authors = load_authors.then(|| repository.read(cx).log_authors(cx));
        self._users_task = cx.spawn(async move |this, cx| {
            let current_user = current_user.await.log_err().flatten();
            let log_authors = match log_authors {
                Some(log_authors) => Some(log_authors.await.log_err().unwrap_or_default()),
                None => None,
            };
            this.update(cx, |this, cx| {
                this.apply_users(current_user, log_authors, cx)
            })
            .ok();
        });
    }

    fn ensure_log_authors(&mut self, cx: &mut Context<Self>) {
        if self.tab_properties.filters.users.is_empty()
            || self.log_authors.is_some()
            || self.log_authors_loading
        {
            return;
        }
        self.reload_users(cx);
    }

    fn apply_users(
        &mut self,
        current_user: Option<GitUser>,
        log_authors: Option<Vec<GitUser>>,
        cx: &mut Context<Self>,
    ) {
        self.current_user_loaded = true;
        if self.current_user != current_user {
            self.current_user = current_user.clone();
            self.commit_table
                .update(cx, |table, cx| table.set_current_user(current_user, cx));
        }
        if let Some(log_authors) = log_authors {
            self.log_authors = Some(log_authors);
            self.log_authors_loading = false;
        }
        self.update_log_filter(cx);
    }

    fn resolved_log_filter(&self) -> Option<LogFilter> {
        let users = &self.tab_properties.filters.users;
        let authors = if users.is_empty() {
            Vec::new()
        } else {
            let all_users = self.log_authors.as_deref()?;
            if !self.current_user_loaded {
                return None;
            }
            filters::author_identities(users, self.current_user.as_ref(), all_users)
        };
        Some(filters::log_filter(
            &self.tab_properties,
            &self.branch_names,
            authors,
        ))
    }

    fn update_log_filter(&mut self, cx: &mut Context<Self>) {
        if self.table_repository != self.repository {
            self.set_table_repository(None, cx);
        }
        let Some(repository) = self.repository.clone() else {
            return;
        };
        let Some(filter) = self.resolved_log_filter() else {
            return;
        };
        if self.applied_filter.as_ref() != Some(&filter) {
            self.applied_filter = Some(filter.clone());
            self.commit_table
                .update(cx, |table, cx| table.set_filter(filter, cx));
        }
        self.set_table_repository(Some(repository), cx);
    }

    fn set_table_repository(
        &mut self,
        repository: Option<Entity<Repository>>,
        cx: &mut Context<Self>,
    ) {
        if self.table_repository == repository {
            return;
        }
        self.table_repository = repository.clone();
        self.commit_table
            .update(cx, |table, cx| table.set_repository(repository, cx));
    }

    fn update_title(&mut self, cx: &mut Context<Self>) {
        let title = filters::title(&self.tab_properties, &self.branch_names);
        if title != self.title {
            self.title = title;
            cx.emit(GitLogTabEvent::TitleChanged);
            cx.notify();
        }
    }

    fn set_tab_properties(
        &mut self,
        properties: LogTabProperties,
        source: PropertiesSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if properties == self.tab_properties {
            return;
        }
        let previous = std::mem::replace(&mut self.tab_properties, properties);
        if self.repository.is_some() {
            self.save_tab_properties(cx);
        } else if source != PropertiesSource::Repository {
            self.tab_properties_await_repository = true;
        }
        if shared_tab_fields_changed(&previous, &self.tab_properties) {
            self.push_properties(source, window, cx);
        }
        if source != PropertiesSource::BranchesPane {
            let show_branches = self.tab_properties.show_branches;
            let single_click = self.tab_properties.branches_single_click;
            if previous.show_branches != show_branches {
                self.branches_pane
                    .update(cx, |pane, cx| pane.set_expanded(show_branches, cx));
            }
            if previous.branches_single_click != single_click {
                self.branches_pane.update(cx, |pane, cx| {
                    pane.set_single_click_action(single_click, cx)
                });
            }
        }
        self.ensure_log_authors(cx);
        self.update_log_filter(cx);
        self.update_title(cx);
        cx.notify();
    }

    fn set_app_properties(
        &mut self,
        properties: LogAppProperties,
        source: PropertiesSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if properties == self.app_properties {
            return;
        }
        let previous = std::mem::replace(&mut self.app_properties, properties);
        self.app_properties_save = persistence::save_app_properties(&self.app_properties, cx);
        cx.set_global(SharedLogAppProperties(self.app_properties.clone()));
        if shared_app_fields_changed(&previous, &self.app_properties) {
            self.push_properties(source, window, cx);
        }
        cx.notify();
    }

    fn receive_shared_app_properties(
        &mut self,
        properties: LogAppProperties,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if properties == self.app_properties {
            return;
        }
        let previous = std::mem::replace(&mut self.app_properties, properties);
        if shared_app_fields_changed(&previous, &self.app_properties) {
            self.push_properties(PropertiesSource::Frame, window, cx);
        }
        cx.notify();
    }

    fn push_properties(
        &mut self,
        source: PropertiesSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let app_properties = self.app_properties.clone();
        let tab_properties = self.tab_properties.clone();
        if source != PropertiesSource::FilterToolbar {
            self.filter_toolbar.update(cx, |toolbar, cx| {
                toolbar.set_properties(app_properties.clone(), tab_properties.clone(), window, cx)
            });
        }
        if source != PropertiesSource::CommitTable {
            self.commit_table.update(cx, |table, cx| {
                table.set_properties(app_properties.clone(), tab_properties.clone(), cx)
            });
        }
        if source != PropertiesSource::ChangesBrowser {
            self.changes_browser.update(cx, |browser, cx| {
                browser.set_properties(app_properties, tab_properties, cx)
            });
        }
    }

    fn save_tab_properties(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.as_ref() else {
            return;
        };
        let repository_path = repository.read(cx).work_directory_abs_path.clone();
        self.tab_properties_save = persistence::save_tab_properties(
            self.tab_id(),
            &repository_path,
            &self.tab_properties,
            cx,
        );
    }

    fn sync_toolbar_height(&mut self, height: Pixels, cx: &mut Context<Self>) {
        if self.toolbar_height == Some(height) {
            return;
        }
        self.toolbar_height = Some(height);
        self.branches_pane
            .update(cx, |pane, cx| pane.set_toolbar_height(height, cx));
        self.changes_browser
            .update(cx, |browser, cx| browser.set_toolbar_height(height, cx));
    }

    fn set_splitter_proportion(
        &mut self,
        kind: SplitterKind,
        proportion: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let properties = kind.with_proportion(&self.app_properties, proportion);
        self.set_app_properties(properties, PropertiesSource::Frame, window, cx);
    }

    fn drag_splitter(
        &mut self,
        splitter: Splitter,
        event: &DragMoveEvent<DraggedLogSplitter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dragged = *event.drag(cx);
        if dragged.tab != cx.entity_id() || dragged.kind != splitter.kind {
            return;
        }
        let proportion = splitter.dragged_proportion(event.event.position, event.bounds);
        self.set_splitter_proportion(splitter.kind, proportion, window, cx);
    }

    fn handle_git_store_event(
        &mut self,
        git_store: &Entity<GitStore>,
        event: &GitStoreEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            GitStoreEvent::ActiveRepositoryChanged(_) => {
                let repository = git_store.read(cx).active_repository();
                self.set_repository(repository, window, cx);
            }
            GitStoreEvent::RepositoryUpdated(
                repository_id,
                RepositoryEvent::BranchListChanged | RepositoryEvent::HeadChanged,
                _,
            ) => {
                let is_current = self
                    .repository
                    .as_ref()
                    .is_some_and(|repository| repository.read(cx).id == *repository_id);
                if is_current {
                    self.refresh_repository_state(cx);
                }
            }
            GitStoreEvent::GlobalConfigurationUpdated => self.reload_users(cx),
            _ => {}
        }
    }

    fn handle_branches_pane_event(
        &mut self,
        _: &Entity<BranchesPane>,
        event: &BranchesPaneEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            BranchesPaneEvent::FilterByRevisions(revisions) => {
                self.set_branch_filter(revisions.clone(), window, cx);
            }
            BranchesPaneEvent::NavigateToRevision(revision) => {
                let revision = revision.clone();
                self.commit_table
                    .update(cx, |table, cx| table.jump_to_revision(revision, window, cx));
            }
            BranchesPaneEvent::ExpandedChanged(expanded) => {
                let properties = LogTabProperties {
                    show_branches: *expanded,
                    ..self.tab_properties.clone()
                };
                self.set_tab_properties(properties, PropertiesSource::BranchesPane, window, cx);
            }
            BranchesPaneEvent::SingleClickActionChanged(action) => {
                let properties = LogTabProperties {
                    branches_single_click: *action,
                    ..self.tab_properties.clone()
                };
                self.set_tab_properties(properties, PropertiesSource::BranchesPane, window, cx);
            }
        }
    }

    fn handle_filter_toolbar_event(
        &mut self,
        _: &Entity<LogFilterToolbar>,
        event: &LogFilterToolbarEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            LogFilterToolbarEvent::TabPropertiesChanged(properties) => {
                let properties = with_frame_tab_fields(&self.tab_properties, properties.clone());
                self.set_tab_properties(properties, PropertiesSource::FilterToolbar, window, cx);
            }
            LogFilterToolbarEvent::AppPropertiesChanged(properties) => {
                let properties = with_frame_app_fields(&self.app_properties, properties.clone());
                self.set_app_properties(properties, PropertiesSource::FilterToolbar, window, cx);
            }
            LogFilterToolbarEvent::Refresh => {
                self.commit_table.update(cx, |table, cx| table.refresh(cx));
            }
            LogFilterToolbarEvent::GoToRevision(revision) => {
                let revision = revision.clone();
                self.commit_table
                    .update(cx, |table, cx| table.jump_to_revision(revision, window, cx));
            }
            LogFilterToolbarEvent::CherryPick => {
                self.commit_table
                    .update(cx, |table, cx| table.cherry_pick_selected(window, cx));
            }
        }
    }

    fn handle_commit_table_event(
        &mut self,
        commit_table: &Entity<CommitTable>,
        event: &CommitTableEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            CommitTableEvent::SelectionChanged(commits) => {
                let can_cherry_pick = commit_table.read(cx).can_cherry_pick();
                self.filter_toolbar.update(cx, |toolbar, cx| {
                    toolbar.set_cherry_pick_enabled(can_cherry_pick, cx)
                });
                let repository = self.repository.clone();
                self.changes_browser.update(cx, |browser, cx| {
                    browser.set_commits(repository.clone(), commits.clone(), cx)
                });
                self.commit_details.update(cx, |details, cx| {
                    details.set_commits(repository, commits.clone(), cx)
                });
            }
            CommitTableEvent::ColumnWidthsChanged(column_widths) => {
                let properties = LogTabProperties {
                    column_widths: column_widths.clone(),
                    ..self.tab_properties.clone()
                };
                self.set_tab_properties(properties, PropertiesSource::CommitTable, window, cx);
            }
            CommitTableEvent::LoadingChanged(_) => {}
        }
    }

    fn handle_changes_browser_event(
        &mut self,
        _: &Entity<ChangesBrowser>,
        event: &ChangesBrowserEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ChangesBrowserEvent::TabPropertiesChanged(properties) => {
                let properties = with_frame_tab_fields(&self.tab_properties, properties.clone());
                self.set_tab_properties(properties, PropertiesSource::ChangesBrowser, window, cx);
            }
            ChangesBrowserEvent::AppPropertiesChanged(properties) => {
                let properties = with_frame_app_fields(&self.app_properties, properties.clone());
                self.set_app_properties(properties, PropertiesSource::ChangesBrowser, window, cx);
            }
        }
    }

    fn focus_text_filter(
        &mut self,
        _: &FocusTextFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .filter_toolbar
            .focus_handle(cx)
            .contains_focused(window, cx)
        {
            let table_focus = self.commit_table.focus_handle(cx);
            window.focus(&table_focus, cx);
        } else {
            self.filter_toolbar
                .update(cx, |toolbar, cx| toolbar.focus_text_filter(window, cx));
        }
    }

    fn toggle_details(&mut self, _: &ShowDetails, window: &mut Window, cx: &mut Context<Self>) {
        let properties = LogTabProperties {
            show_details: !self.tab_properties.show_details,
            ..self.tab_properties.clone()
        };
        self.set_tab_properties(properties, PropertiesSource::Frame, window, cx);
    }

    fn refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        self.commit_table.update(cx, |table, cx| table.refresh(cx));
    }

    fn go_to_hash_or_ref(
        &mut self,
        _: &GoToHashOrRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.filter_toolbar
            .update(cx, |toolbar, cx| toolbar.show_go_to_popup(window, cx));
    }

    fn render_splitter(
        &self,
        splitter: Splitter,
        first: AnyElement,
        second: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kind = splitter.kind;
        splitter.render(
            cx.entity_id(),
            first,
            second,
            cx.theme().colors().border,
            cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() == DIVIDER_RESET_CLICK_COUNT {
                    this.set_splitter_proportion(kind, DIVIDER_RESET_PROPORTION, window, cx);
                }
                cx.stop_propagation();
            }),
            cx.listener(
                move |this, event: &DragMoveEvent<DraggedLogSplitter>, window, cx| {
                    this.drag_splitter(splitter, event, window, cx)
                },
            ),
        )
    }

    fn render_main_frame(&self, toolbar_height: Pixels, cx: &mut Context<Self>) -> AnyElement {
        let changes_minimum = (toolbar_height * TOOLBAR_MINIMUM_SIZE_FACTOR).round();
        let table_side = v_flex()
            .size_full()
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .child(self.filter_toolbar.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(self.commit_table.clone()),
            )
            .into_any_element();
        let (changes_side, changes_side_minimum) = if self.tab_properties.show_details {
            let details_splitter = Splitter {
                kind: SplitterKind::Details,
                proportion: SplitterKind::Details.proportion(&self.app_properties),
                leading_size: Pixels::ZERO,
                first_minimum: changes_minimum,
                second_minimum: DETAILS_MINIMUM_SIZE,
            };
            (
                self.render_splitter(
                    details_splitter,
                    self.changes_browser.clone().into_any_element(),
                    self.commit_details.clone().into_any_element(),
                    cx,
                ),
                changes_minimum.max(DETAILS_MINIMUM_SIZE),
            )
        } else {
            (
                div()
                    .size_full()
                    .child(self.changes_browser.clone())
                    .into_any_element(),
                changes_minimum,
            )
        };
        let changes_splitter = Splitter {
            kind: SplitterKind::Changes,
            proportion: SplitterKind::Changes.proportion(&self.app_properties),
            leading_size: Pixels::ZERO,
            first_minimum: Pixels::ZERO,
            second_minimum: changes_side_minimum,
        };
        self.render_splitter(changes_splitter, table_side, changes_side, cx)
    }

    fn render_no_repository(&self) -> impl IntoElement {
        v_flex().size_full().items_center().justify_center().child(
            v_flex()
                .items_start()
                .gap(STATUS_TEXT_LINE_GAP)
                .child(Label::new(NO_REPOSITORY_TEXT).color(Color::Muted))
                .child(
                    h_flex()
                        .child(Label::new(STATUS_TEXT_LINE_PREFIX).color(Color::Muted))
                        .child(
                            div()
                                .id("git-log-create-repository")
                                .cursor_pointer()
                                .child(Label::new(CREATE_REPOSITORY_TEXT).color(Color::Accent))
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(git::Init), cx)
                                }),
                        ),
                ),
        )
    }
}

pub(crate) fn forget_log_tab_state(tab_id: &str, cx: &App) {
    persistence::delete_tab_properties(tab_id, cx);
}

fn branch_names(repository: &Repository) -> Vec<SharedString> {
    repository
        .branch_list
        .iter()
        .map(|branch| SharedString::from(branch.name()))
        .collect()
}

fn with_frame_tab_fields(
    current: &LogTabProperties,
    incoming: LogTabProperties,
) -> LogTabProperties {
    LogTabProperties {
        show_branches: current.show_branches,
        branches_single_click: current.branches_single_click,
        column_widths: current.column_widths.clone(),
        ..incoming
    }
}

fn with_frame_app_fields(
    current: &LogAppProperties,
    incoming: LogAppProperties,
) -> LogAppProperties {
    LogAppProperties {
        branches_splitter_proportion: current.branches_splitter_proportion,
        changes_splitter_proportion: current.changes_splitter_proportion,
        details_splitter_proportion: current.details_splitter_proportion,
        ..incoming
    }
}

fn shared_tab_fields_changed(previous: &LogTabProperties, current: &LogTabProperties) -> bool {
    with_frame_tab_fields(previous, current.clone()) != *previous
}

fn shared_app_fields_changed(previous: &LogAppProperties, current: &LogAppProperties) -> bool {
    with_frame_app_fields(previous, current.clone()) != *previous
}

impl EventEmitter<GitLogTabEvent> for GitLogTab {}

impl Focusable for GitLogTab {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.repository.is_some() {
            self.commit_table.focus_handle(cx)
        } else {
            self.focus_handle.clone()
        }
    }
}

impl Drop for GitLogTab {
    fn drop(&mut self) {
        std::mem::replace(&mut self.app_properties_save, Task::ready(())).detach();
        std::mem::replace(&mut self.tab_properties_save, Task::ready(())).detach();
    }
}

impl Render for GitLogTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div()
            .id(("git-log-tab", cx.entity_id().as_u64()))
            .key_context(KEY_CONTEXT)
            .size_full()
            .flex()
            .flex_row()
            .bg(ui::content_background(
                cx.theme().colors().panel_background,
                window,
            ));
        if self.repository.is_none() {
            return root
                .track_focus(&self.focus_handle)
                .child(self.render_no_repository())
                .into_any_element();
        }

        let toolbar_height = self.filter_toolbar.read(cx).height(window, cx);
        self.sync_toolbar_height(toolbar_height, cx);
        let main_frame = self.render_main_frame(toolbar_height, cx);
        let content = if self.branches_pane.read(cx).is_expanded() {
            let branches_splitter = Splitter {
                kind: SplitterKind::Branches,
                proportion: SplitterKind::Branches.proportion(&self.app_properties),
                leading_size: branches_pane::TOOLBAR_WIDTH,
                first_minimum: Pixels::ZERO,
                second_minimum: Pixels::ZERO,
            };
            self.render_splitter(
                branches_splitter,
                self.branches_pane.clone().into_any_element(),
                main_frame,
                cx,
            )
        } else {
            h_flex()
                .size_full()
                .child(self.branches_pane.clone())
                .child(div().flex_1().min_w_0().h_full().child(main_frame))
                .into_any_element()
        };
        root.on_action(cx.listener(Self::focus_text_filter))
            .on_action(cx.listener(Self::toggle_details))
            .on_action(cx.listener(Self::refresh))
            .on_action(cx.listener(Self::go_to_hash_or_ref))
            .child(content)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_log::properties::{BranchesSingleClickAction, LogColumn};

    #[test]
    fn child_properties_keep_the_fields_the_frame_owns() {
        let current = LogTabProperties {
            show_branches: true,
            branches_single_click: BranchesSingleClickAction::UpdateBranchFilter,
            column_widths: vec![(LogColumn::Author, 140.)],
            ..LogTabProperties::new(true)
        };
        let incoming = LogTabProperties {
            show_details: false,
            show_branches: false,
            column_widths: Vec::new(),
            ..LogTabProperties::new(false)
        };
        let merged = with_frame_tab_fields(&current, incoming);
        assert!(!merged.show_details);
        assert!(merged.show_branches);
        assert_eq!(
            merged.branches_single_click,
            BranchesSingleClickAction::UpdateBranchFilter
        );
        assert_eq!(merged.column_widths, current.column_widths);
        assert!(shared_tab_fields_changed(&current, &merged));
        assert!(!shared_tab_fields_changed(
            &current,
            &LogTabProperties {
                column_widths: Vec::new(),
                show_branches: false,
                ..current.clone()
            }
        ));

        let current_app = LogAppProperties {
            branches_splitter_proportion: 0.2,
            ..LogAppProperties::default()
        };
        let incoming_app = LogAppProperties {
            compact_references: false,
            ..LogAppProperties::default()
        };
        let merged_app = with_frame_app_fields(&current_app, incoming_app);
        assert!(!merged_app.compact_references);
        assert_eq!(merged_app.branches_splitter_proportion, 0.2);
        assert!(shared_app_fields_changed(&current_app, &merged_app));
        assert!(!shared_app_fields_changed(
            &current_app,
            &LogAppProperties {
                details_splitter_proportion: 0.9,
                ..current_app.clone()
            }
        ));
    }
}
