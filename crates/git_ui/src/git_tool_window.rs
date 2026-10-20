mod console;
mod content_tabs;
mod header;
mod persistence;
#[cfg(test)]
mod tests;

use editor::Editor;
use gpui::{
    AnyView, App, Context, Entity, FocusHandle, Focusable, Pixels, SharedString, Subscription,
    Task, WeakEntity, Window, actions,
};
use project::{
    Project, ProjectPath,
    git_store::{GitStore, GitStoreEvent},
};
use settings::Settings as _;
use ui::prelude::*;
use util::ResultExt as _;
use workspace::{Workspace, WorkspaceSettings};

use self::console::GitConsoleView;
use self::content_tabs::{
    CONSOLE_TAB_NAME, CONSOLE_TAB_ORDER_WEIGHT, ContentTabKey, ContentTabs,
    order_weight_for_tab_name,
};
use self::header::{GitToolWindowHeader, TabListEntry, TabMenuCommand};
use self::persistence::{PersistedLogTab, PersistedTabs};
use crate::git_log::log_tab::{GitLogTab, GitLogTabEvent, forget_log_tab_state};
use crate::git_log::properties::{LogFilterState, MAIN_LOG_TAB_ID};
use crate::git_panel::GitPanel;

actions!(
    git_tool_window,
    [
        OpenAnotherLogTab,
        ShowConsoleTab,
        CloseActiveTab,
        CloseOtherTabs,
        CloseAllTabs,
        SelectNextTab,
        SelectPreviousTab,
        ShowListOfTabs,
        ToggleConsoleSoftWraps,
        ScrollConsoleToEnd,
        ClearConsole
    ]
);

const KEY_CONTEXT: &str = "GitToolWindow";

pub(crate) struct HeaderTab {
    pub key: ContentTabKey,
    pub title: SharedString,
    pub closeable: bool,
    pub selected: bool,
}

pub(crate) struct HeaderSnapshot {
    pub tabs: Vec<HeaderTab>,
    pub active: bool,
    pub focus_handle: FocusHandle,
    pub header_height: Pixels,
    pub can_open_log_tab: bool,
    pub dropdown_visible: bool,
    pub islands: bool,
}

pub(crate) struct TabMenuState {
    pub closeable: bool,
    pub can_close_all: bool,
    pub can_close_others: bool,
    pub can_navigate: bool,
}

pub(crate) struct DropdownState {
    pub show_history: Option<(SharedString, bool)>,
    pub show_console_tab: bool,
    pub show_worktrees: bool,
}

struct LogTabEntry {
    id: SharedString,
    tab: Entity<GitLogTab>,
    _subscription: Subscription,
}

pub(crate) struct GitToolWindow {
    workspace: WeakEntity<Workspace>,
    project: Entity<Project>,
    git_store: Entity<GitStore>,
    focus_handle: FocusHandle,
    tabs: ContentTabs,
    log_tabs: Vec<LogTabEntry>,
    console: Option<Entity<GitConsoleView>>,
    header: Entity<GitToolWindowHeader>,
    persistence_key: Option<String>,
    restored: bool,
    restore_task: Task<()>,
    pending_save: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl GitToolWindow {
    pub(crate) fn new(
        workspace: WeakEntity<Workspace>,
        project: Entity<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let git_store = project.read(cx).git_store().clone();
        let header_owner = cx.weak_entity();
        let header = cx.new(|_| GitToolWindowHeader::new(header_owner));

        let subscriptions = vec![
            cx.on_focus(&focus_handle, window, |this, window, cx| {
                this.focus_selected_content(window, cx)
            }),
            cx.on_focus_in(&focus_handle, window, |this, _, cx| this.notify_header(cx)),
            cx.on_focus_out(&focus_handle, window, |this, _, _, cx| {
                this.notify_header(cx)
            }),
            cx.subscribe_in(
                &git_store,
                window,
                |this, _, event: &GitStoreEvent, window, cx| match event {
                    GitStoreEvent::ConsoleUpdated => this.console_updated(window, cx),
                    GitStoreEvent::RepositoryAdded
                    | GitStoreEvent::RepositoryRemoved(_)
                    | GitStoreEvent::ActiveRepositoryChanged(_) => this.notify_header(cx),
                    _ => {}
                },
            ),
        ];

        let mut this = Self {
            workspace,
            project,
            git_store,
            focus_handle,
            tabs: ContentTabs::default(),
            log_tabs: Vec::new(),
            console: None,
            header,
            persistence_key: None,
            restored: false,
            restore_task: Task::ready(()),
            pending_save: Task::ready(()),
            _subscriptions: subscriptions,
        };
        let main_id = SharedString::new_static(MAIN_LOG_TAB_ID);
        this.create_log_tab(main_id.clone(), window, cx);
        let main_key = ContentTabKey::Log(main_id);
        this.tabs.append(
            main_key.clone(),
            order_weight_for_tab_name(content_tabs::LOG_TAB_NAME),
        );
        this.tabs.select(&main_key);
        if !this.git_store.read(cx).console_lines().is_empty() {
            this.ensure_console(window, cx);
        }
        cx.defer_in(window, |this, window, cx| this.restore(window, cx));
        this
    }

    pub(crate) fn header_tabs(&self) -> AnyView {
        self.header.clone().into()
    }

    fn notify_header(&mut self, cx: &mut Context<Self>) {
        App::notify(cx, self.header.entity_id());
        cx.notify();
    }

    fn log_tab(&self, id: &str) -> Option<&Entity<GitLogTab>> {
        self.log_tabs
            .iter()
            .find(|entry| entry.id.as_ref() == id)
            .map(|entry| &entry.tab)
    }

    fn create_log_tab(
        &mut self,
        id: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<GitLogTab> {
        let workspace = self.workspace.clone();
        let project = self.project.clone();
        let tab_id = id.clone();
        let tab = cx.new(|cx| GitLogTab::new(id, workspace, project, window, cx));
        let subscription = cx.subscribe(&tab, |this, _, event: &GitLogTabEvent, cx| match event {
            GitLogTabEvent::TitleChanged => {
                this.save(cx);
                this.notify_header(cx);
            }
        });
        self.log_tabs.push(LogTabEntry {
            id: tab_id,
            tab: tab.clone(),
            _subscription: subscription,
        });
        tab
    }

    fn is_closeable(key: &ContentTabKey) -> bool {
        match key {
            ContentTabKey::Log(id) => id.as_ref() != MAIN_LOG_TAB_ID,
            ContentTabKey::Console => true,
        }
    }

    fn tab_title(&self, key: &ContentTabKey, cx: &App) -> SharedString {
        match key {
            ContentTabKey::Log(id) => self
                .log_tab(id)
                .map(|tab| tab.read(cx).title())
                .unwrap_or_else(|| SharedString::new_static(content_tabs::LOG_TAB_NAME)),
            ContentTabKey::Console => SharedString::new_static(CONSOLE_TAB_NAME),
        }
    }

    fn content_focus_handle(&self, key: &ContentTabKey, cx: &App) -> Option<FocusHandle> {
        match key {
            ContentTabKey::Log(id) => self.log_tab(id).map(|tab| tab.focus_handle(cx)),
            ContentTabKey::Console => self
                .console
                .as_ref()
                .map(|console| console.focus_handle(cx)),
        }
    }

    fn focus_selected_content(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self
            .tabs
            .selected()
            .and_then(|key| self.content_focus_handle(key, cx));
        if let Some(handle) = handle {
            window.focus(&handle, cx);
        }
    }

    pub(crate) fn select_tab(
        &mut self,
        key: &ContentTabKey,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.select(key) {
            self.save(cx);
        }
        if focus {
            self.focus_selected_content(window, cx);
        }
        self.notify_header(cx);
    }

    pub(crate) fn close_tab(
        &mut self,
        key: &ContentTabKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !Self::is_closeable(key) {
            return;
        }
        let was_focused = self
            .content_focus_handle(key, cx)
            .is_some_and(|handle| handle.contains_focused(window, cx));
        self.remove_tab(key, cx);
        if was_focused {
            self.focus_selected_content(window, cx);
        }
    }

    fn remove_tab(&mut self, key: &ContentTabKey, cx: &mut Context<Self>) {
        if !self.tabs.remove(key) {
            return;
        }
        match key {
            ContentTabKey::Log(id) => {
                if let Some(index) = self.log_tabs.iter().position(|entry| entry.id == *id) {
                    let entry = self.log_tabs.remove(index);
                    entry.tab.update(cx, |tab, cx| tab.forget_state(cx));
                }
            }
            ContentTabKey::Console => self.console = None,
        }
        self.save(cx);
        self.notify_header(cx);
    }

    pub(crate) fn close_other_tabs(
        &mut self,
        key: &ContentTabKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let others: Vec<ContentTabKey> = self
            .tabs
            .keys()
            .filter(|other| *other != key && Self::is_closeable(other))
            .cloned()
            .collect();
        for other in others {
            self.close_tab(&other, window, cx);
        }
    }

    pub(crate) fn close_all_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let closeable: Vec<ContentTabKey> = self
            .tabs
            .keys()
            .filter(|key| Self::is_closeable(key))
            .cloned()
            .collect();
        for key in closeable {
            self.close_tab(&key, window, cx);
        }
    }

    fn hide_tool_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.close_panel::<GitPanel>(window, cx)
            })
            .log_err();
    }

    pub(crate) fn process_close_click(
        &mut self,
        key: &ContentTabKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if Self::is_closeable(key) {
            self.close_tab(key, window, cx);
        } else if self.tabs.len() == 1 {
            self.hide_tool_window(window, cx);
        }
    }

    fn close_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.tabs.selected().cloned() {
            Some(key) if Self::is_closeable(&key) => self.close_tab(&key, window, cx),
            _ => self.hide_tool_window(window, cx),
        }
    }

    pub(crate) fn select_next_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.tabs.next_key() {
            self.select_tab(&key, true, window, cx);
        }
    }

    pub(crate) fn select_previous_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.tabs.previous_key() {
            self.select_tab(&key, true, window, cx);
        }
    }

    pub(crate) fn show_list_of_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() < 2 {
            return;
        }
        let selected = self.tabs.selected();
        let tabs: Vec<TabListEntry> = self
            .tabs
            .keys()
            .map(|key| TabListEntry {
                key: key.clone(),
                title: self.tab_title(key, cx),
                selected: selected == Some(key),
            })
            .collect();
        self.header
            .update(cx, |header, cx| header.deploy_tab_list(tabs, window, cx));
    }

    pub(crate) fn run_tab_menu_command(
        &mut self,
        command: TabMenuCommand,
        key: &ContentTabKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            TabMenuCommand::CloseTab => self.close_tab(key, window, cx),
            TabMenuCommand::CloseAllTabs => self.close_all_tabs(window, cx),
            TabMenuCommand::CloseOtherTabs => self.close_other_tabs(key, window, cx),
            TabMenuCommand::SelectNextTab => self.select_next_tab(window, cx),
            TabMenuCommand::SelectPreviousTab => self.select_previous_tab(window, cx),
            TabMenuCommand::ShowListOfTabs => self.show_list_of_tabs(window, cx),
        }
    }

    pub(crate) fn shortcut_focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    fn has_repositories(&self, cx: &App) -> bool {
        !self.git_store.read(cx).repositories().is_empty()
    }

    fn selected_log_filters(&self, cx: &App) -> LogFilterState {
        match self.tabs.selected() {
            Some(ContentTabKey::Log(id)) => self
                .log_tab(id)
                .map(|tab| tab.read(cx).filters())
                .unwrap_or_default(),
            _ => LogFilterState::default(),
        }
    }

    pub(crate) fn open_another_log_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_repositories(cx) {
            return;
        }
        let filters = self.selected_log_filters(cx);
        let id = persistence::new_log_tab_id(|candidate| {
            candidate == MAIN_LOG_TAB_ID || self.log_tab(candidate).is_some()
        });
        let id = SharedString::from(id);
        let tab = self.create_log_tab(id.clone(), window, cx);
        if !filters.is_empty() {
            tab.update(cx, |tab, cx| tab.set_filters(filters, window, cx));
        }
        let weight = order_weight_for_tab_name(&tab.read(cx).title());
        let key = ContentTabKey::Log(id);
        self.tabs.append(key.clone(), weight);
        self.select_tab(&key, true, window, cx);
        self.save(cx);
    }

    pub(crate) fn open_log_tab_with_revisions(
        &mut self,
        revisions: Vec<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.has_repositories(cx) {
            return;
        }
        let id = persistence::new_log_tab_id(|candidate| {
            candidate == MAIN_LOG_TAB_ID || self.log_tab(candidate).is_some()
        });
        let id = SharedString::from(id);
        let tab = self.create_log_tab(id.clone(), window, cx);
        tab.update(cx, |tab, cx| tab.set_branch_filter(revisions, window, cx));
        let weight = order_weight_for_tab_name(&tab.read(cx).title());
        let key = ContentTabKey::Log(id);
        self.tabs.append(key.clone(), weight);
        self.select_tab(&key, true, window, cx);
        self.save(cx);
    }

    fn ensure_console(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<GitConsoleView> {
        if let Some(console) = &self.console {
            return console.clone();
        }
        let git_store = self.git_store.clone();
        let console = cx.new(|cx| GitConsoleView::new(git_store, window, cx));
        self.tabs
            .insert_by_weight(ContentTabKey::Console, CONSOLE_TAB_ORDER_WEIGHT);
        self.console = Some(console.clone());
        console
    }

    fn console_updated(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(console) = self.console.clone() {
            console.update(cx, |console, cx| console.sync(window, cx));
        } else if !self.git_store.read(cx).console_lines().is_empty() {
            self.ensure_console(window, cx);
        }
        self.notify_header(cx);
    }

    pub(crate) fn show_console_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let console = self.ensure_console(window, cx);
        self.select_tab(&ContentTabKey::Console, true, window, cx);
        console.update(cx, |console, cx| console.scroll_to_end(window, cx));
    }

    fn console_visible(&self) -> bool {
        self.console.is_some() && self.tabs.selected() == Some(&ContentTabKey::Console)
    }

    pub(crate) fn header_snapshot(&self, window: &Window, cx: &App) -> HeaderSnapshot {
        let selected = self.tabs.selected();
        let tabs = self
            .tabs
            .keys()
            .map(|key| HeaderTab {
                key: key.clone(),
                title: self.tab_title(key, cx),
                closeable: Self::is_closeable(key),
                selected: selected == Some(key),
            })
            .collect();
        let dropdown = self.dropdown_state(cx);
        HeaderSnapshot {
            tabs,
            active: self.focus_handle.contains_focused(window, cx),
            focus_handle: self.focus_handle.clone(),
            header_height: ui::panel_header_height(cx),
            islands: WorkspaceSettings::get_global(cx).islands.enabled,
            can_open_log_tab: self.has_repositories(cx),
            dropdown_visible: dropdown.show_history.is_some()
                || dropdown.show_console_tab
                || dropdown.show_worktrees,
        }
    }

    pub(crate) fn tab_menu_state(&self, key: &ContentTabKey) -> TabMenuState {
        TabMenuState {
            closeable: Self::is_closeable(key),
            can_close_all: self.tabs.keys().any(Self::is_closeable),
            can_close_others: self
                .tabs
                .keys()
                .any(|other| other != key && Self::is_closeable(other)),
            can_navigate: self.tabs.len() > 1,
        }
    }

    fn show_history_state(&self, cx: &App) -> Option<(SharedString, bool)> {
        if !self.has_repositories(cx) {
            return None;
        }
        let file_name = self.workspace.upgrade().and_then(|workspace| {
            let workspace = workspace.read(cx);
            let editor = workspace.active_item_as::<Editor>(cx)?;
            let editor = editor.read(cx);
            let file = editor.file_at(editor.selections.newest_anchor().head(), cx)?;
            let project_path = ProjectPath {
                worktree_id: file.worktree_id(cx),
                path: file.path().clone(),
            };
            crate::git_graph::resolve_file_history_target_from_project_path(
                workspace,
                &project_path,
                cx,
            )?;
            Some(SharedString::from(file.file_name(cx).to_string()))
        });
        Some(match file_name {
            Some(file_name) => (
                SharedString::from(format!("Show History for {file_name}")),
                true,
            ),
            None => (SharedString::new_static("Show History"), false),
        })
    }

    pub(crate) fn dropdown_state(&self, cx: &App) -> DropdownState {
        DropdownState {
            show_history: self.show_history_state(cx),
            show_console_tab: !self.git_store.read(cx).console_lines().is_empty()
                && !self.console_visible(),
            show_worktrees: self.has_repositories(cx),
        }
    }

    fn persisted_state(&self, cx: &App) -> PersistedTabs {
        let log_tabs = self
            .tabs
            .keys()
            .filter_map(|key| match key {
                ContentTabKey::Log(id) if id.as_ref() != MAIN_LOG_TAB_ID => {
                    let tab = self.log_tab(id)?;
                    Some(PersistedLogTab {
                        id: id.to_string(),
                        has_filters: !tab.read(cx).filters().is_empty(),
                    })
                }
                _ => None,
            })
            .collect();
        let selected_tab = self
            .tabs
            .selection_history()
            .iter()
            .find_map(|key| match key {
                ContentTabKey::Log(id) => Some(id.to_string()),
                ContentTabKey::Console => None,
            });
        PersistedTabs {
            log_tabs,
            selected_tab,
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if !self.restored {
            return;
        }
        let Some(key) = self.persistence_key.clone() else {
            return;
        };
        let state = self.persisted_state(cx);
        self.pending_save = persistence::save_tabs(key, state, cx);
    }

    fn restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.persistence_key = self
            .workspace
            .upgrade()
            .and_then(|workspace| persistence::workspace_key(workspace.read(cx)));
        let Some(key) = self.persistence_key.clone() else {
            self.restored = true;
            return;
        };
        let load = persistence::load_tabs(key, cx);
        self.restore_task = cx.spawn_in(window, async move |this, cx| {
            let state = load.await.unwrap_or_default();
            this.update_in(cx, |this, window, cx| {
                this.apply_restored_state(state, window, cx)
            })
            .log_err();
        });
    }

    fn apply_restored_state(
        &mut self,
        state: PersistedTabs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let plan = persistence::restore_plan(&state.log_tabs);
        for id in &plan.dropped {
            forget_log_tab_state(id, cx);
        }
        for id in plan.reopened {
            if id == MAIN_LOG_TAB_ID || self.log_tab(&id).is_some() {
                continue;
            }
            let id = SharedString::from(id);
            let tab = self.create_log_tab(id.clone(), window, cx);
            let weight = order_weight_for_tab_name(&tab.read(cx).title());
            self.tabs.append(ContentTabKey::Log(id), weight);
        }
        if let Some(selected) = state.selected_tab {
            let key = ContentTabKey::Log(SharedString::from(selected));
            if self.tabs.contains(&key) {
                self.tabs.select(&key);
            }
        }
        self.restored = true;
        self.save(cx);
        self.notify_header(cx);
    }
}

impl Focusable for GitToolWindow {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.tabs
            .selected()
            .and_then(|key| self.content_focus_handle(key, cx))
            .unwrap_or_else(|| self.focus_handle.clone())
    }
}

impl Render for GitToolWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.tabs.selected() {
            Some(ContentTabKey::Log(id)) => {
                self.log_tab(id).map(|tab| tab.clone().into_any_element())
            }
            Some(ContentTabKey::Console) => self
                .console
                .as_ref()
                .map(|console| console.clone().into_any_element()),
            None => None,
        };
        v_flex()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .on_action(cx.listener(|this, _: &OpenAnotherLogTab, window, cx| {
                this.open_another_log_tab(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &ShowConsoleTab, window, cx| {
                    this.show_console_tab(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &CloseActiveTab, window, cx| {
                    this.close_active_tab(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &CloseOtherTabs, window, cx| {
                if let Some(key) = this.tabs.selected().cloned() {
                    this.close_other_tabs(&key, window, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &CloseAllTabs, window, cx| this.close_all_tabs(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SelectNextTab, window, cx| this.select_next_tab(window, cx)),
            )
            .on_action(cx.listener(|this, _: &SelectPreviousTab, window, cx| {
                this.select_previous_tab(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowListOfTabs, window, cx| {
                this.show_list_of_tabs(window, cx)
            }))
            .child(div().flex_1().min_h_0().w_full().children(content))
    }
}
