mod dialogs;
mod model;
mod popup;
pub(crate) mod users;

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use collections::{HashMap, HashSet};
use db::kvp::KeyValueStore;
use editor::{Editor, EditorEvent};
use git::repository::{Branch, GitUser};
use gpui::{
    Action, Anchor, AnyElement, AnyView, App, Bounds, ClickEvent, Context, DismissEvent, Div,
    Entity, EntityId, EventEmitter, FocusHandle, Focusable, Hsla, ManagedView, Modifiers,
    MouseButton, MouseDownEvent, MouseUpEvent, PathBuilder, PathPromptOptions, Pixels, Point,
    SharedString, Subscription, Task, WeakEntity, Window, actions, anchored, canvas, deferred,
    point, px, rgb,
};
use project::git_store::Repository;
use time::OffsetDateTime;
use ui::{
    ContextMenu, IconButton, IconButtonShape, PopoverMenu, TextSize, Tooltip, prelude::*,
    text_for_keystroke,
};
use util::ResultExt as _;
use workspace::Workspace;

use self::dialogs::{DatePeriodDialog, GoToPopup, ValuesCompletion, ValuesPopup, ValuesPopupEvent};
use self::model::{
    BUTTON_DIRECTIONAL_GAP, BUTTON_ORTHOGONAL_GAP, BUTTON_SIZE, BranchRef, BranchRefKind,
    CHIP_INSET, TOOLBAR_HORIZONTAL_INSET, TOOLBAR_VERTICAL_INSET,
};
use self::popup::{FavoriteHandler, FilterPopup, PopupAction, PopupHandler, PopupIcon, PopupItem};
use super::log_dates;
use super::log_tab::FocusTextFilter;
use super::properties::{
    GraphOptions, GraphSortType, HEAD_REVISION, LogAppProperties, LogTabProperties, USER_FILTER_ME,
    USER_FILTER_ME_LABEL,
};
use crate::branch_refs::RefTarget;
use crate::branches_popup::state::{self, PopupState, SavedPopupStates};
use crate::branches_popup::tree;

actions!(
    git_log_filters,
    [
        Refresh,
        GoToHashOrRef,
        ShowSearchHistory,
        PreviousHistoryItem,
        ResetFilter
    ]
);

const KEY_CONTEXT: &str = "GitLogFilterToolbar";
const TEXT_FIELD_KEY_CONTEXT: &str = "GitLogTextFilter";
const CHIP_KEY_CONTEXT: &str = "GitLogFilterChip";
const TEXT_FILTER_HISTORY_KEY: &str = "Vcs.Log.Text.Filter.History";
const RECENT_FILTERS_KEY_PREFIX: &str = "git_log_recent_filters";

const TEXT_FILTER_PLACEHOLDER: &str = "Text or hash";
const TEXT_FILTER_TOOLTIP: &str = "Filter by Commit Message or Hashes";
const REGEX_TEXT: &str = "Regex";
const MATCH_CASE_TEXT: &str = "Match Case";
const SELECT_TEXT: &str = "Select\u{2026}";
const SELECT_IN_TREE_TEXT: &str = "Select in Tree\u{2026}";
const RECENT_TEXT: &str = "Recent";
const FAVORITES_TEXT: &str = "Favorites";
const LAST_DAY_TEXT: &str = "Last 24 hours";
const LAST_WEEK_TEXT: &str = "Last 7 days";
const CLEAR_TEXT: &str = "Clear";
const CLEAR_POPUP_ADVERTISEMENT: &str = "Press {0} to clear the filter";
const VALUES_ADVERTISEMENT: &str =
    "Select one or more values separated with {0}, use {1} to finish";
const GRAPH_OPTIONS_TEXT: &str = "Graph Options";
const GRAPH_OPTIONS_DESCRIPTION: &str = "VCS log graph options and actions";
const SORT_SEPARATOR: &str = "Sort";
const OPTIONS_SEPARATOR: &str = "Options";
const SORT_BY_COMMIT_DATE_TEXT: &str = "By Commit Date";
const SORT_TOPOLOGICALLY_TEXT: &str = "Topologically";
const FIRST_PARENT_TEXT: &str = "First Parent";
const REFRESH_TEXT: &str = "Refresh";
const REFRESH_DESCRIPTION: &str = "Check for new commits and refresh Log if necessary";
const CHERRY_PICK_TEXT: &str = "Cherry-Pick";
const VIEW_OPTIONS_TEXT: &str = "View Options";
const VIEW_OPTIONS_DESCRIPTION: &str = "Configure presentation of the Log";
const GO_TO_TEXT: &str = "Go To Hash/Branch/Tag";
const GO_TO_DESCRIPTION: &str =
    "Specify hash or name of a branch or a tag to navigate to the commit it points";
const SHOW_SEPARATOR: &str = "Show";
const COMPACT_REFERENCES_TEXT: &str = "Compact References View";
const TAG_NAMES_TEXT: &str = "Tag Names";
const LONG_EDGES_TEXT: &str = "Long Edges";
const COMMIT_TIMESTAMP_TEXT: &str = "Commit Timestamp";
const REFERENCES_ON_THE_LEFT_TEXT: &str = "References on the Left";
const COLUMNS_TEXT: &str = "Columns";
const HIGHLIGHT_SEPARATOR: &str = "Highlight";
const CURRENT_BRANCH_TEXT: &str = "Current Branch";
const MERGE_COMMITS_TEXT: &str = "Merge Commits";
const MY_COMMITS_TEXT: &str = "My Commits";

const TEXT_FIELD_MINIMUM_WIDTH: f32 = 150.;
const TEXT_FIELD_COLUMNS: f32 = 15.;
const TEXT_FIELD_BORDER_INSET: f32 = 3.;
const SEARCH_ICON_AREA_WIDTH: f32 = 24.;
const COMPONENT_ARC: f32 = 8.;
const FOCUSED_CHIP_ARC: f32 = 10.;
const FOCUSED_CHIP_THICKNESS: f32 = 2.;
const CHIP_GAP_BEFORE_ARROW: f32 = 3.;
const CHIP_ICON_SIZE: f32 = 16.;
const ICON_SIZE: f32 = 16.;
const BRANCH_POPUP_MINIMUM_WIDTH: f32 = 250.;
const POPUP_MINIMUM_WIDTH: f32 = 200.;
const BADGE_CENTER_X: f32 = 16.5 / 20.;
const BADGE_CENTER_Y: f32 = 3.5 / 20.;
const BADGE_RADIUS: f32 = 3.5 / 20.;
const BADGE_BORDER: f32 = 1.5 / 20.;
const SEARCH_OPTION_SELECTED_DARK: u32 = 0x2E4D89;
const SEARCH_OPTION_SELECTED_HOVERED_DARK: u32 = 0x2F5EB9;
const SEARCH_OPTION_SELECTED_LIGHT: u32 = 0xD0DFFE;
const SEARCH_OPTION_SELECTED_HOVERED_LIGHT: u32 = 0xBDD3FF;
const BADGE_SUCCESS_DARK: u32 = 0x5FAD65;
const BADGE_SUCCESS_LIGHT: u32 = 0x55A76A;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LogFilterToolbarEvent {
    TabPropertiesChanged(LogTabProperties),
    AppPropertiesChanged(LogAppProperties),
    Refresh,
    GoToRevision(SharedString),
    CherryPick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum FilterKind {
    Branch,
    User,
    Date,
    Paths,
}

impl FilterKind {
    const ALL: [FilterKind; 4] = [
        FilterKind::Branch,
        FilterKind::User,
        FilterKind::Date,
        FilterKind::Paths,
    ];

    fn index(self) -> usize {
        match self {
            FilterKind::Branch => 0,
            FilterKind::User => 1,
            FilterKind::Date => 2,
            FilterKind::Paths => 3,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            FilterKind::Branch => "Branch",
            FilterKind::User => "User",
            FilterKind::Date => "Date",
            FilterKind::Paths => "Paths",
        }
    }

    fn element_id(self) -> &'static str {
        match self {
            FilterKind::Branch => "git-log-filter-branch",
            FilterKind::User => "git-log-filter-user",
            FilterKind::Date => "git-log-filter-date",
            FilterKind::Paths => "git-log-filter-paths",
        }
    }

    fn button_id(self) -> &'static str {
        match self {
            FilterKind::Branch => "git-log-filter-branch-button",
            FilterKind::User => "git-log-filter-user-button",
            FilterKind::Date => "git-log-filter-date-button",
            FilterKind::Paths => "git-log-filter-paths-button",
        }
    }

    fn recent_name(self) -> Option<&'static str> {
        match self {
            FilterKind::Branch => Some(model::BRANCH_FILTER_NAME),
            FilterKind::User => Some(model::USER_FILTER_NAME),
            FilterKind::Paths => Some(model::PATHS_FILTER_NAME),
            FilterKind::Date => None,
        }
    }

    fn minimum_popup_width(self) -> f32 {
        match self {
            FilterKind::Branch => BRANCH_POPUP_MINIMUM_WIDTH,
            FilterKind::User | FilterKind::Date | FilterKind::Paths => POPUP_MINIMUM_WIDTH,
        }
    }
}

#[derive(Default)]
struct BranchData {
    has_head: bool,
    locals: Vec<Branch>,
    remotes: Vec<Branch>,
}

struct OpenPopup {
    entity_id: EntityId,
    view: AnyView,
    anchor: Point<Pixels>,
    popup_focus: FocusHandle,
    restore_focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

pub(crate) struct LogFilterToolbar {
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    app_properties: LogAppProperties,
    tab_properties: LogTabProperties,
    repository: Option<Entity<Repository>>,
    text_editor: Entity<Editor>,
    chip_focus_handles: [FocusHandle; 4],
    chip_bounds: [Rc<Cell<Option<Bounds<Pixels>>>>; 4],
    text_field_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    hovered_chip: Option<FilterKind>,
    hovered_chip_button: Option<FilterKind>,
    clear_button_hovered: bool,
    text_history: Vec<String>,
    history_index: usize,
    recent_filters: HashMap<String, Vec<Vec<String>>>,
    recent_filters_key: Option<String>,
    authors: Vec<GitUser>,
    current_user: Option<GitUser>,
    favorites: PopupState,
    favorites_key: Option<String>,
    cherry_pick_enabled: bool,
    open_popup: Option<OpenPopup>,
    _users_task: Task<()>,
    _favorites_task: Task<()>,
    _recent_filters_task: Task<()>,
    _history_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl LogFilterToolbar {
    pub(crate) fn new(
        workspace: WeakEntity<Workspace>,
        app_properties: LogAppProperties,
        tab_properties: LogTabProperties,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = tab_properties.filters.text.clone();
        let text_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text(TEXT_FILTER_PLACEHOLDER, window, cx);
            editor.set_text(text, window, cx);
            editor
        });
        let editor_subscription = cx.subscribe_in(
            &text_editor,
            window,
            |this, _, event: &EditorEvent, _, cx| match event {
                EditorEvent::Blurred => this.text_focus_lost(cx),
                EditorEvent::BufferEdited => cx.notify(),
                _ => {}
            },
        );
        let favorites_subscription =
            cx.observe_global::<SavedPopupStates>(|this, cx| this.apply_saved_favorites(cx));
        let store = KeyValueStore::global(cx);
        let history_task = cx.spawn(async move |this, cx| {
            let raw = cx
                .background_spawn(async move {
                    store.read_kvp(TEXT_FILTER_HISTORY_KEY).log_err().flatten()
                })
                .await;
            this.update(cx, |this, cx| {
                if let Some(raw) = raw {
                    this.text_history = raw
                        .split('\n')
                        .filter(|item| !item.is_empty())
                        .map(str::to_string)
                        .collect();
                }
                cx.notify();
            })
            .ok();
        });
        Self {
            workspace,
            focus_handle: cx.focus_handle(),
            app_properties,
            tab_properties,
            repository: None,
            text_editor,
            chip_focus_handles: [
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
                cx.focus_handle(),
            ],
            chip_bounds: std::array::from_fn(|_| Rc::new(Cell::new(None))),
            text_field_bounds: Rc::new(Cell::new(None)),
            hovered_chip: None,
            hovered_chip_button: None,
            clear_button_hovered: false,
            text_history: Vec::new(),
            history_index: 0,
            recent_filters: HashMap::default(),
            recent_filters_key: None,
            authors: Vec::new(),
            current_user: None,
            favorites: PopupState::default(),
            favorites_key: None,
            cherry_pick_enabled: false,
            open_popup: None,
            _users_task: Task::ready(()),
            _favorites_task: Task::ready(()),
            _recent_filters_task: Task::ready(()),
            _history_task: history_task,
            _subscriptions: vec![editor_subscription, favorites_subscription],
        }
    }

    pub(crate) fn set_repository(
        &mut self,
        repository: Option<Entity<Repository>>,
        cx: &mut Context<Self>,
    ) {
        if self.repository.as_ref().map(Entity::entity_id)
            == repository.as_ref().map(Entity::entity_id)
        {
            return;
        }
        self.repository = repository;
        self.authors.clear();
        self.current_user = None;
        self.recent_filters.clear();
        self.recent_filters_key = None;
        self.favorites = PopupState::default();
        self.favorites_key = None;
        self._users_task = Task::ready(());
        self._favorites_task = Task::ready(());
        self._recent_filters_task = Task::ready(());
        let Some(repository) = self.repository.clone() else {
            cx.notify();
            return;
        };
        let common_dir = repository.read(cx).common_dir_abs_path.clone();
        self.load_favorites(state::serialization_key(&common_dir), cx);
        self.load_recent_filters(
            format!(
                "{RECENT_FILTERS_KEY_PREFIX}-{}",
                common_dir.to_string_lossy()
            ),
            cx,
        );
        self.load_users(cx);
        cx.notify();
    }

    pub(crate) fn set_properties(
        &mut self,
        app_properties: LogAppProperties,
        tab_properties: LogTabProperties,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let model_text = tab_properties.filters.text.clone();
        self.app_properties = app_properties;
        self.tab_properties = tab_properties;
        if !model::is_same_filter_as(&self.field_text(cx), &model_text) {
            self.set_field_text(model_text, window, cx);
        }
        cx.notify();
    }

    pub(crate) fn height(&self, window: &Window, cx: &App) -> Pixels {
        px(model::toolbar_height(label_font_height(window, cx)))
    }

    pub(crate) fn focus_text_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.text_editor.focus_handle(cx);
        window.focus(&handle, cx);
    }

    pub(crate) fn set_cherry_pick_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.cherry_pick_enabled != enabled {
            self.cherry_pick_enabled = enabled;
            cx.notify();
        }
    }

    pub(crate) fn show_go_to_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else {
            return;
        };
        let branches = self.branch_names(cx);
        let toolbar = cx.weak_entity();
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.toggle_modal(window, cx, move |window, cx| {
                    GoToPopup::new(
                        repository,
                        branches,
                        move |revision, _, cx| {
                            toolbar
                                .update(cx, |_, cx| {
                                    cx.emit(LogFilterToolbarEvent::GoToRevision(revision.into()))
                                })
                                .ok();
                        },
                        window,
                        cx,
                    )
                });
            })
            .ok();
    }

    fn load_users(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else {
            return;
        };
        let (authors, current_user) = {
            let repository = repository.read(cx);
            (repository.log_authors(cx), repository.current_user(cx))
        };
        self._users_task = cx.spawn(async move |this, cx| {
            let authors = authors.await.log_err();
            let current_user = current_user.await.log_err().flatten();
            this.update(cx, |this, cx| {
                if let Some(authors) = authors {
                    this.authors = authors;
                }
                this.current_user = current_user;
                cx.notify();
            })
            .ok();
        });
    }

    fn load_favorites(&mut self, key: String, cx: &mut Context<Self>) {
        self.favorites_key = Some(key.clone());
        let saved = cx
            .try_global::<SavedPopupStates>()
            .and_then(|states| states.get(&key))
            .cloned();
        if let Some(saved) = saved {
            self.favorites = PopupState::from_persisted(saved);
            return;
        }
        let load = state::load(key, cx);
        self._favorites_task = cx.spawn(async move |this, cx| {
            let persisted = load.await;
            this.update(cx, |this, cx| {
                if let Some(persisted) = persisted {
                    this.favorites = PopupState::from_persisted(persisted);
                }
                cx.notify();
            })
            .ok();
        });
    }

    fn apply_saved_favorites(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.favorites_key.as_ref() else {
            return;
        };
        let saved = cx
            .try_global::<SavedPopupStates>()
            .and_then(|states| states.get(key))
            .cloned();
        if let Some(saved) = saved {
            self.favorites = PopupState::from_persisted(saved);
            cx.notify();
        }
    }

    fn load_recent_filters(&mut self, key: String, cx: &mut Context<Self>) {
        self.recent_filters_key = Some(key.clone());
        let store = KeyValueStore::global(cx);
        self._recent_filters_task = cx.spawn(async move |this, cx| {
            let raw = cx
                .background_spawn(async move { store.read_kvp(&key).log_err().flatten() })
                .await;
            let filters = raw.and_then(|raw| {
                serde_json::from_str::<HashMap<String, Vec<Vec<String>>>>(&raw).log_err()
            });
            this.update(cx, |this, cx| {
                if let Some(filters) = filters {
                    this.recent_filters = filters;
                }
                cx.notify();
            })
            .ok();
        });
    }

    fn save_recent_filters(&self, cx: &mut Context<Self>) {
        let Some(key) = self.recent_filters_key.clone() else {
            return;
        };
        let Some(value) = serde_json::to_string(&self.recent_filters).log_err() else {
            return;
        };
        let store = KeyValueStore::global(cx);
        cx.background_spawn(async move {
            store.write_kvp(key, value).await.log_err();
        })
        .detach();
    }

    fn save_text_history(&self, cx: &mut Context<Self>) {
        let value = self.text_history.join("\n");
        let store = KeyValueStore::global(cx);
        cx.background_spawn(async move {
            store
                .write_kvp(TEXT_FILTER_HISTORY_KEY.to_string(), value)
                .await
                .log_err();
        })
        .detach();
    }

    fn recent_group(&self, name: &str) -> Vec<Vec<String>> {
        self.recent_filters.get(name).cloned().unwrap_or_default()
    }

    fn update_tab(&mut self, cx: &mut Context<Self>, update: impl FnOnce(&mut LogTabProperties)) {
        if let Some(next) = model::updated(&self.tab_properties, update) {
            self.tab_properties = next;
            cx.emit(LogFilterToolbarEvent::TabPropertiesChanged(
                self.tab_properties.clone(),
            ));
        }
        cx.notify();
    }

    fn update_app(&mut self, cx: &mut Context<Self>, update: impl FnOnce(&mut LogAppProperties)) {
        if let Some(next) = model::updated(&self.app_properties, update) {
            self.app_properties = next;
            cx.emit(LogFilterToolbarEvent::AppPropertiesChanged(
                self.app_properties.clone(),
            ));
        }
        cx.notify();
    }

    fn set_values(
        &mut self,
        kind: FilterKind,
        values: Vec<String>,
        remember: bool,
        cx: &mut Context<Self>,
    ) {
        if values.is_empty() {
            self.reset_filter(kind, cx);
            return;
        }
        match kind {
            FilterKind::Branch => {
                let sorted = model::sorted_values(values.clone());
                self.update_tab(cx, |tab| tab.filters.branches = sorted);
            }
            FilterKind::User => self.update_tab(cx, |tab| tab.filters.users = values.clone()),
            FilterKind::Paths => self.update_tab(cx, |tab| tab.filters.paths = values.clone()),
            FilterKind::Date => {}
        }
        if remember && let Some(name) = kind.recent_name() {
            model::add_recent_group(
                self.recent_filters.entry(name.to_string()).or_default(),
                values,
            );
            self.save_recent_filters(cx);
        }
    }

    fn set_dates(&mut self, after: Option<i64>, before: Option<i64>, cx: &mut Context<Self>) {
        self.update_tab(cx, |tab| {
            tab.filters.date_after = after;
            tab.filters.date_before = before;
        });
    }

    fn reset_filter(&mut self, kind: FilterKind, cx: &mut Context<Self>) {
        self.update_tab(cx, |tab| match kind {
            FilterKind::Branch => tab.filters.branches.clear(),
            FilterKind::User => tab.filters.users.clear(),
            FilterKind::Date => {
                tab.filters.date_after = None;
                tab.filters.date_before = None;
            }
            FilterKind::Paths => tab.filters.paths.clear(),
        });
    }

    fn field_text(&self, cx: &App) -> String {
        self.text_editor.read(cx).text(cx)
    }

    fn set_field_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.text_editor.update(cx, |editor, cx| {
            editor.set_text(text, window, cx);
            editor.change_selections(Default::default(), window, cx, |selections| {
                selections.select_ranges([editor::Anchor::Max..editor::Anchor::Max]);
            });
        });
    }

    fn apply_text_filter(&mut self, add_to_history: bool, cx: &mut Context<Self>) {
        let text = model::text_filter_value(&self.field_text(cx));
        self.update_tab(cx, |tab| tab.filters.text = text);
        if add_to_history {
            self.add_current_text_to_history(cx);
        }
    }

    fn add_current_text_to_history(&mut self, cx: &mut Context<Self>) {
        let text = self.field_text(cx);
        if model::add_to_search_history(&mut self.text_history, &text) {
            self.save_text_history(cx);
        }
    }

    fn text_focus_lost(&mut self, cx: &mut Context<Self>) {
        if !model::is_same_filter_as(&self.field_text(cx), &self.tab_properties.filters.text) {
            self.apply_text_filter(false, cx);
        }
    }

    fn confirm_text(&mut self, _: &menu::Confirm, _: &mut Window, cx: &mut Context<Self>) {
        self.apply_text_filter(true, cx);
    }

    fn cancel_text(&mut self, _: &menu::Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.field_text(cx).is_empty() {
            cx.propagate();
            return;
        }
        self.set_field_text(String::new(), window, cx);
    }

    fn clear_text_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_field_text(String::new(), window, cx);
        self.update_tab(cx, |tab| tab.filters.text = String::new());
    }

    fn previous_history_item(
        &mut self,
        _: &PreviousHistoryItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.field_text(cx);
        if !self.text_history.contains(&text) {
            self.add_current_text_to_history(cx);
        }
        let size = self.text_history.len().min(model::SEARCH_HISTORY_SIZE);
        if size < 2 {
            return;
        }
        self.history_index = if self.history_index == 0 || self.history_index > size {
            size - 1
        } else {
            self.history_index - 1
        };
        if let Some(value) = self.text_history.get(self.history_index).cloned() {
            self.set_field_text(value, window, cx);
        }
    }

    fn history_item_chosen(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) {
        self.set_field_text(value, window, cx);
        self.add_current_text_to_history(cx);
    }

    fn show_history_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_current_text_to_history(cx);
        let history: Vec<String> = self
            .text_history
            .iter()
            .take(model::SEARCH_HISTORY_SIZE)
            .cloned()
            .collect();
        if history.is_empty() {
            return;
        }
        let toolbar = cx.weak_entity();
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            for item in &history {
                let toolbar = toolbar.clone();
                let value = item.clone();
                menu = menu.entry(item.clone(), None, move |window, cx| {
                    toolbar
                        .update(cx, |toolbar, cx| {
                            toolbar.history_item_chosen(value.clone(), window, cx)
                        })
                        .ok();
                });
            }
            menu
        });
        let anchor = self
            .text_field_bounds
            .get()
            .map(|bounds| bounds.bottom_left())
            .unwrap_or_default();
        let restore_focus = self.text_editor.focus_handle(cx);
        self.open_popup(menu, anchor, restore_focus, Vec::new(), window, cx);
    }

    fn toggle_regex(&mut self, cx: &mut Context<Self>) {
        self.update_tab(cx, |tab| tab.text_filter_regex = !tab.text_filter_regex);
    }

    fn toggle_match_case(&mut self, cx: &mut Context<Self>) {
        self.update_tab(cx, |tab| {
            tab.text_filter_match_case = !tab.text_filter_match_case
        });
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        cx.emit(LogFilterToolbarEvent::Refresh);
    }

    fn branch_data(&self, cx: &App) -> BranchData {
        let Some(repository) = &self.repository else {
            return BranchData::default();
        };
        let snapshot = repository.read(cx);
        BranchData {
            has_head: snapshot.head_commit.is_some(),
            locals: snapshot
                .branch_list
                .iter()
                .filter(|branch| !branch.is_remote())
                .cloned()
                .collect(),
            remotes: snapshot
                .branch_list
                .iter()
                .filter(|branch| branch.is_remote() && !is_remote_head(branch.name()))
                .cloned()
                .collect(),
        }
    }

    fn branch_names(&self, cx: &App) -> Vec<String> {
        let data = self.branch_data(cx);
        let mut names = Vec::new();
        if data.has_head {
            names.push(HEAD_REVISION.to_string());
        }
        names.extend(data.locals.iter().map(|branch| branch.name().to_string()));
        names.extend(data.remotes.iter().map(|branch| branch.name().to_string()));
        names
    }

    fn chip_value(&self, kind: FilterKind) -> Option<String> {
        let filters = &self.tab_properties.filters;
        match kind {
            FilterKind::Branch => (!filters.branches.is_empty()).then(|| {
                model::displayable_text(&filters.branches, model::MAX_FILTER_VALUE_LENGTH)
            }),
            FilterKind::User => (!filters.users.is_empty()).then(|| {
                model::displayable_text(
                    &model::localized_users(&filters.users),
                    model::MAX_FILTER_VALUE_LENGTH,
                )
            }),
            FilterKind::Date => (filters.date_after.is_some() || filters.date_before.is_some())
                .then(|| {
                    model::date_chip_text(filters.date_after, filters.date_before, |seconds| {
                        log_dates::format_date(seconds).to_string()
                    })
                }),
            FilterKind::Paths => {
                (!filters.paths.is_empty()).then(|| model::structure_text(&filters.paths))
            }
        }
    }

    fn chip_tooltip(&self, kind: FilterKind) -> Option<String> {
        let filters = &self.tab_properties.filters;
        match kind {
            FilterKind::Branch => {
                (!filters.branches.is_empty()).then(|| model::values_tooltip(&filters.branches))
            }
            FilterKind::User => (!filters.users.is_empty())
                .then(|| model::values_tooltip(&model::localized_users(&filters.users))),
            FilterKind::Date => None,
            FilterKind::Paths => model::structure_tooltip(&filters.paths),
        }
    }

    fn chip_anchor(&self, kind: FilterKind) -> Point<Pixels> {
        self.chip_bounds[kind.index()]
            .get()
            .map(|bounds| bounds.bottom_left())
            .unwrap_or_default()
    }

    fn open_popup<V: ManagedView>(
        &mut self,
        view: Entity<V>,
        anchor: Point<Pixels>,
        restore_focus: FocusHandle,
        mut subscriptions: Vec<Subscription>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entity_id = view.entity_id();
        let popup_focus = view.focus_handle(cx);
        subscriptions.push(cx.subscribe_in(
            &view,
            window,
            move |this, _, _: &DismissEvent, window, cx| {
                this.popup_dismissed(entity_id, window, cx)
            },
        ));
        self.open_popup = Some(OpenPopup {
            entity_id,
            view: view.into(),
            anchor,
            popup_focus: popup_focus.clone(),
            restore_focus,
            _subscriptions: subscriptions,
        });
        window.focus(&popup_focus, cx);
        cx.notify();
    }

    fn popup_dismissed(
        &mut self,
        entity_id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(popup) = self
            .open_popup
            .take_if(|popup| popup.entity_id == entity_id)
        else {
            return;
        };
        if popup.popup_focus.contains_focused(window, cx) || window.focused(cx).is_none() {
            window.focus(&popup.restore_focus, cx);
        }
        cx.notify();
    }

    fn predefined_action(
        &self,
        kind: FilterKind,
        values: Vec<String>,
        text: String,
        remember: bool,
        cx: &mut Context<Self>,
    ) -> PopupAction {
        let toolbar = cx.weak_entity();
        let tooltip = match kind {
            FilterKind::User => model::values_tooltip(&model::localized_users(&values)),
            FilterKind::Branch | FilterKind::Date | FilterKind::Paths => {
                model::values_tooltip(&values)
            }
        };
        let handler: PopupHandler = Rc::new(move |_: &mut Window, cx: &mut App| {
            toolbar
                .update(cx, |toolbar, cx| {
                    toolbar.set_values(kind, values.clone(), remember, cx)
                })
                .ok();
        });
        PopupAction::new(text, handler).tooltip(tooltip)
    }

    fn select_values_action(&self, kind: FilterKind, cx: &mut Context<Self>) -> PopupAction {
        let toolbar = cx.weak_entity();
        let handler: PopupHandler = Rc::new(move |window: &mut Window, cx: &mut App| {
            toolbar
                .update(cx, |toolbar, cx| {
                    toolbar.open_values_popup(kind, window, cx)
                })
                .ok();
        });
        PopupAction::new(SELECT_TEXT, handler)
    }

    fn branch_action(
        &self,
        reference: &BranchRef,
        defaults: &HashSet<RefTarget>,
        cx: &mut Context<Self>,
    ) -> PopupAction {
        let action = self
            .predefined_action(
                FilterKind::Branch,
                vec![reference.name.clone()],
                reference.name.clone(),
                true,
                cx,
            )
            .icon(PopupIcon::Favorite(reference.favorite));
        let target = match &reference.kind {
            BranchRefKind::Head => None,
            BranchRefKind::Local => Some(RefTarget::local(reference.name.clone())),
            BranchRefKind::Remote(_) => Some(RefTarget::remote(reference.name.clone())),
        };
        let Some(target) = target else {
            return action;
        };
        let toolbar = cx.weak_entity();
        let defaults = defaults.clone();
        let handler: FavoriteHandler = Rc::new(move |_: &mut Window, cx: &mut App| {
            toolbar
                .update(cx, |toolbar, cx| {
                    toolbar.toggle_favorite(&target, &defaults, cx)
                })
                .unwrap_or(false)
        });
        action.on_icon_click(handler)
    }

    fn toggle_favorite(
        &mut self,
        target: &RefTarget,
        defaults: &HashSet<RefTarget>,
        cx: &mut Context<Self>,
    ) -> bool {
        let favorite = self.favorites.toggle_favorite(target, defaults);
        if let Some(key) = self.favorites_key.clone() {
            state::save(key, self.favorites.to_persisted(), cx).detach();
        }
        cx.notify();
        favorite
    }

    fn branch_popup_items(&self, cx: &mut Context<Self>) -> Vec<PopupItem> {
        let data = self.branch_data(cx);
        let defaults = tree::default_favorites(&data.locals, &data.remotes);
        let mut refs = Vec::new();
        if data.has_head {
            refs.push(BranchRef {
                name: HEAD_REVISION.to_string(),
                kind: BranchRefKind::Head,
                favorite: true,
            });
        }
        for branch in &data.locals {
            let name = branch.name().to_string();
            let favorite = self
                .favorites
                .is_favorite(&RefTarget::local(name.clone()), &defaults);
            refs.push(BranchRef {
                name,
                kind: BranchRefKind::Local,
                favorite,
            });
        }
        for branch in &data.remotes {
            let name = branch.name().to_string();
            let remote = branch.remote_name().unwrap_or_default().to_string();
            let favorite = self
                .favorites
                .is_favorite(&RefTarget::remote(name.clone()), &defaults);
            refs.push(BranchRef {
                name,
                kind: BranchRefKind::Remote(remote),
                favorite,
            });
        }
        let by_name: HashMap<String, BranchRef> = refs
            .iter()
            .map(|reference| (reference.name.clone(), reference.clone()))
            .collect();
        let available: HashSet<String> = by_name.keys().cloned().collect();
        let recent = model::process_recent_branch_filters(
            &available,
            &self.recent_group(model::BRANCH_FILTER_NAME),
        );
        let groups = model::branch_popup_groups(&refs, recent);

        let mut items = vec![PopupItem::Action(
            self.select_values_action(FilterKind::Branch, cx),
        )];
        let mut speed_search = Vec::new();
        if !groups.recent.is_empty() {
            let recent_items: Vec<PopupItem> = groups
                .recent
                .iter()
                .map(|values| {
                    PopupItem::Action(self.predefined_action(
                        FilterKind::Branch,
                        values.clone(),
                        model::action_name(values),
                        true,
                        cx,
                    ))
                })
                .collect();
            speed_search.push(PopupItem::Separator(Some(RECENT_TEXT.into())));
            speed_search.extend(
                recent_items
                    .iter()
                    .cloned()
                    .map(PopupItem::into_speed_search_only),
            );
            items.push(PopupItem::Submenu {
                text: RECENT_TEXT.into(),
                items: Rc::new(recent_items),
            });
        }
        if groups.favorites.len() > 1 {
            items.push(PopupItem::Action(self.predefined_action(
                FilterKind::Branch,
                groups.favorites.clone(),
                FAVORITES_TEXT.to_string(),
                false,
                cx,
            )));
        }
        for name in &groups.favorites {
            if let Some(reference) = by_name.get(name) {
                items.push(PopupItem::Action(
                    self.branch_action(reference, &defaults, cx),
                ));
            }
        }
        items.push(PopupItem::Separator(None));
        for (group, names) in &groups.others {
            let group_items: Vec<PopupItem> = names
                .iter()
                .filter_map(|name| by_name.get(name))
                .map(|reference| PopupItem::Action(self.branch_action(reference, &defaults, cx)))
                .collect();
            speed_search.push(PopupItem::Separator(Some(group.clone().into())));
            speed_search.extend(
                group_items
                    .iter()
                    .cloned()
                    .map(PopupItem::into_speed_search_only),
            );
            items.push(PopupItem::Submenu {
                text: group.clone().into(),
                items: Rc::new(group_items),
            });
        }
        items.extend(speed_search);
        items
    }

    fn user_popup_items(&self, cx: &mut Context<Self>) -> Vec<PopupItem> {
        let mut items = vec![PopupItem::Action(
            self.select_values_action(FilterKind::User, cx),
        )];
        if self.current_user.is_some() {
            items.push(PopupItem::Action(
                self.predefined_action(
                    FilterKind::User,
                    vec![USER_FILTER_ME.to_string()],
                    USER_FILTER_ME_LABEL.to_string(),
                    true,
                    cx,
                )
                .hidden_while_filtering(true),
            ));
        }
        let recent = self.recent_group(model::USER_FILTER_NAME);
        if !recent.is_empty() {
            items.push(PopupItem::Separator(Some(RECENT_TEXT.into())));
            for values in recent {
                let text = model::action_name(&model::localized_users(&values));
                let single = values.len() == 1;
                items.push(PopupItem::Action(
                    self.predefined_action(FilterKind::User, values, text, true, cx)
                        .hidden_while_filtering(single),
                ));
            }
            items.push(PopupItem::Separator(None));
        }
        items.push(PopupItem::Action(
            self.predefined_action(
                FilterKind::User,
                vec![USER_FILTER_ME.to_string()],
                USER_FILTER_ME_LABEL.to_string(),
                true,
                cx,
            )
            .speed_search_only(),
        ));
        items.push(PopupItem::Separator(None));
        for user in users::display_names(&self.authors) {
            items.push(PopupItem::Action(
                self.predefined_action(FilterKind::User, vec![user.clone()], user, true, cx)
                    .speed_search_only(),
            ));
        }
        items
    }

    fn date_popup_items(&self, cx: &mut Context<Self>) -> Vec<PopupItem> {
        let toolbar = cx.weak_entity();
        let [last_day, last_week] = model::date_presets(OffsetDateTime::now_utc().unix_timestamp());
        let select: PopupHandler = {
            let toolbar = toolbar.clone();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                toolbar
                    .update(cx, |toolbar, cx| toolbar.open_date_dialog(window, cx))
                    .ok();
            })
        };
        let preset = |text: &'static str, since: i64| {
            let toolbar = toolbar.clone();
            let handler: PopupHandler = Rc::new(move |_: &mut Window, cx: &mut App| {
                toolbar
                    .update(cx, |toolbar, cx| toolbar.set_dates(Some(since), None, cx))
                    .ok();
            });
            PopupItem::Action(PopupAction::new(text, handler))
        };
        vec![
            PopupItem::Action(PopupAction::new(SELECT_TEXT, select)),
            preset(LAST_DAY_TEXT, last_day),
            preset(LAST_WEEK_TEXT, last_week),
        ]
    }

    fn paths_popup_items(&self, cx: &mut Context<Self>) -> Vec<PopupItem> {
        let toolbar = cx.weak_entity();
        let select_in_tree: PopupHandler = Rc::new(move |_: &mut Window, cx: &mut App| {
            toolbar
                .update(cx, |toolbar, cx| toolbar.select_paths_in_tree(cx))
                .ok();
        });
        let mut items = vec![
            PopupItem::Action(self.select_values_action(FilterKind::Paths, cx)),
            PopupItem::Action(PopupAction::new(SELECT_IN_TREE_TEXT, select_in_tree)),
            PopupItem::Separator(Some(RECENT_TEXT.into())),
        ];
        let current = model::naturally_sorted(self.tab_properties.filters.paths.clone());
        for values in self.recent_group(model::PATHS_FILTER_NAME) {
            let selected = model::naturally_sorted(values.clone()) == current;
            let text = model::structure_text(&values);
            let tooltip = values.join(" ");
            let toolbar = cx.weak_entity();
            let handler: PopupHandler = Rc::new(move |_: &mut Window, cx: &mut App| {
                toolbar
                    .update(cx, |toolbar, cx| {
                        toolbar.set_values(FilterKind::Paths, values.clone(), false, cx)
                    })
                    .ok();
            });
            items.push(PopupItem::Action(
                PopupAction::new(text, handler)
                    .tooltip(tooltip)
                    .icon(PopupIcon::Checked(selected)),
            ));
        }
        items
    }

    fn show_filter_popup(&mut self, kind: FilterKind, window: &mut Window, cx: &mut Context<Self>) {
        let items = match kind {
            FilterKind::Branch => self.branch_popup_items(cx),
            FilterKind::User => self.user_popup_items(cx),
            FilterKind::Date => self.date_popup_items(cx),
            FilterKind::Paths => self.paths_popup_items(cx),
        };
        let advertisement: SharedString = CLEAR_POPUP_ADVERTISEMENT
            .replace(
                "{0}",
                &text_for_keystroke(&Modifiers::default(), "delete", cx),
            )
            .into();
        let toolbar = cx.weak_entity();
        let on_reset: PopupHandler = Rc::new(move |_: &mut Window, cx: &mut App| {
            toolbar
                .update(cx, |toolbar, cx| toolbar.reset_filter(kind, cx))
                .ok();
        });
        let minimum_width = px(kind.minimum_popup_width());
        let popup = cx.new(|cx| {
            FilterPopup::new(
                items,
                Some(advertisement),
                Some(on_reset),
                minimum_width,
                cx,
            )
        });
        let anchor = self.chip_anchor(kind);
        let restore_focus = self.chip_focus_handles[kind.index()].clone();
        self.open_popup(popup, anchor, restore_focus, Vec::new(), window, cx);
    }

    fn open_values_popup(&mut self, kind: FilterKind, window: &mut Window, cx: &mut Context<Self>) {
        let filters = self.tab_properties.filters.clone();
        let (initial_text, candidates, completion, separators): (
            String,
            Vec<String>,
            ValuesCompletion,
            &'static [char],
        ) = match kind {
            FilterKind::Branch => (
                filters.branches.join("\n"),
                self.branch_names(cx),
                ValuesCompletion::Branches,
                &model::VALUE_SEPARATORS,
            ),
            FilterKind::User => {
                let mut candidates = vec![USER_FILTER_ME_LABEL.to_string()];
                candidates.extend(users::display_names(&self.authors));
                (
                    model::localized_users(&filters.users).join("\n"),
                    candidates,
                    ValuesCompletion::Values,
                    &model::VALUE_SEPARATORS,
                )
            }
            FilterKind::Paths => (
                model::naturally_sorted(filters.paths).join("\n"),
                Vec::new(),
                ValuesCompletion::None,
                &model::PATH_SEPARATORS,
            ),
            FilterKind::Date => return,
        };
        let advertisement: SharedString = VALUES_ADVERTISEMENT
            .replace("{0}", &model::separators_text(separators))
            .replace(
                "{1}",
                &text_for_keystroke(&Modifiers::command(), "enter", cx),
            )
            .into();
        let popup = cx.new(|cx| {
            ValuesPopup::new(
                initial_text,
                candidates,
                completion,
                separators,
                advertisement,
                window,
                cx,
            )
        });
        let subscription = cx.subscribe_in(
            &popup,
            window,
            move |this, _, event: &ValuesPopupEvent, _, cx| {
                let ValuesPopupEvent::Confirmed(values) = event;
                this.values_confirmed(kind, values.clone(), cx);
            },
        );
        let anchor = self.chip_anchor(kind);
        let restore_focus = self.chip_focus_handles[kind.index()].clone();
        self.open_popup(popup, anchor, restore_focus, vec![subscription], window, cx);
    }

    fn values_confirmed(&mut self, kind: FilterKind, values: Vec<String>, cx: &mut Context<Self>) {
        match kind {
            FilterKind::User => {
                self.set_values(kind, model::parse_localized_users(&values), true, cx)
            }
            FilterKind::Paths => {
                let paths = values
                    .iter()
                    .map(|text| self.text_to_path(text, cx))
                    .collect();
                self.set_values(kind, paths, true, cx);
            }
            FilterKind::Branch | FilterKind::Date => self.set_values(kind, values, true, cx),
        }
    }

    fn text_to_path(&self, text: &str, cx: &App) -> String {
        let text = text.replace('\\', "/");
        if text.ends_with('/') {
            return text;
        }
        let Some(repository) = &self.repository else {
            return text;
        };
        let work_directory = repository.read(cx).work_directory_abs_path.clone();
        let absolute = if Path::new(&text).is_absolute() {
            PathBuf::from(&text)
        } else {
            work_directory.join(&text)
        };
        let is_directory = self.is_directory(&absolute, cx);
        let value = absolute
            .strip_prefix(&*work_directory)
            .ok()
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .filter(|relative| !relative.is_empty())
            .unwrap_or(text);
        if is_directory && !value.ends_with('/') {
            format!("{value}/")
        } else {
            value
        }
    }

    fn is_directory(&self, path: &Path, cx: &App) -> bool {
        let Some(workspace) = self.workspace.upgrade() else {
            return false;
        };
        let project = workspace.read(cx).project().read(cx);
        project
            .find_project_path(path, cx)
            .and_then(|project_path| project.entry_for_path(&project_path, cx))
            .is_some_and(|entry| entry.is_dir())
    }

    fn select_paths_in_tree(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let Some(Some(paths)) = paths.await.ok().and_then(|result| result.log_err()) else {
                return;
            };
            this.update(cx, |this, cx| {
                let values: Vec<String> = paths
                    .iter()
                    .map(|path| this.text_to_path(&path.to_string_lossy(), cx))
                    .collect();
                this.set_values(FilterKind::Paths, values, true, cx);
            })
            .ok();
        })
        .detach();
    }

    fn open_date_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let after = self.tab_properties.filters.date_after;
        let before = self.tab_properties.filters.date_before;
        let toolbar = cx.weak_entity();
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.toggle_modal(window, cx, move |window, cx| {
                    DatePeriodDialog::new(
                        after,
                        before,
                        move |after, before, _, cx| {
                            toolbar
                                .update(cx, |toolbar, cx| toolbar.set_dates(after, before, cx))
                                .ok();
                        },
                        window,
                        cx,
                    )
                });
            })
            .ok();
    }

    fn render_text_option(
        &self,
        id: &'static str,
        icon: IconName,
        title: &'static str,
        selected: bool,
        toggle: fn(&mut Self, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let (selected_background, selected_hovered_background) = search_option_colors(cx);
        let hover_background = cx.theme().colors().ghost_element_hover;
        div()
            .id(id)
            .flex_none()
            .size(px(BUTTON_SIZE))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .when(selected, |this| this.bg(selected_background))
            .hover(move |style| {
                style.bg(if selected {
                    selected_hovered_background
                } else {
                    hover_background
                })
            })
            .tooltip(Tooltip::text(title))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| toggle(this, cx)))
            .child(Icon::new(icon).size(IconSize::Medium))
    }

    fn render_text_field(
        &self,
        height: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let focused = self.text_editor.focus_handle(cx).is_focused(window);
        let has_text = !self.field_text(cx).is_empty();
        let has_history = !self.text_history.is_empty();
        let width = text_field_width(window, cx);
        let bounds_cell = self.text_field_bounds.clone();
        let regex = self.tab_properties.text_filter_regex;
        let match_case = self.tab_properties.text_filter_match_case;
        let clear_icon = if self.clear_button_hovered {
            IconName::CloseSmallHovered
        } else {
            IconName::CloseSmall
        };
        let regex_option = self.render_text_option(
            "git-log-text-filter-regex",
            IconName::InlineRegex,
            REGEX_TEXT,
            regex,
            Self::toggle_regex,
            cx,
        );
        let match_case_option = self.render_text_option(
            "git-log-text-filter-match-case",
            IconName::InlineMatchCase,
            MATCH_CASE_TEXT,
            match_case,
            Self::toggle_match_case,
            cx,
        );
        let colors = cx.theme().colors();
        let icon_color = colors.icon;
        let inset = if focused {
            TEXT_FIELD_BORDER_INSET - 1.
        } else {
            TEXT_FIELD_BORDER_INSET
        };
        div()
            .id("git-log-text-filter")
            .key_context(TEXT_FIELD_KEY_CONTEXT)
            .relative()
            .flex_none()
            .min_w(px(TEXT_FIELD_MINIMUM_WIDTH))
            .w(width)
            .h(height)
            .p(px(inset))
            .on_action(cx.listener(Self::confirm_text))
            .on_action(cx.listener(Self::cancel_text))
            .on_action(cx.listener(|this, _: &ShowSearchHistory, window, cx| {
                this.show_history_popup(window, cx)
            }))
            .on_action(cx.listener(Self::previous_history_item))
            .child(
                h_flex()
                    .size_full()
                    .rounded(px(COMPONENT_ARC / 2.))
                    .when(focused, |this| {
                        this.border_2().border_color(colors.border_focused)
                    })
                    .when(!focused, |this| this.border_1().border_color(colors.border))
                    .bg(colors.editor_background)
                    .pr(px(2.))
                    .child(
                        div()
                            .id("git-log-text-filter-history")
                            .flex_none()
                            .w(px(SEARCH_ICON_AREA_WIDTH))
                            .h_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(has_history, |this| {
                                this.cursor_pointer().on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.show_history_popup(window, cx);
                                    }),
                                )
                            })
                            .child(
                                canvas(
                                    |_, _, _| {},
                                    move |bounds, _, window, _| {
                                        paint_search_history_icon(bounds, icon_color, window)
                                    },
                                )
                                .size(px(ICON_SIZE)),
                            ),
                    )
                    .child(
                        div()
                            .id("git-log-text-filter-editor")
                            .flex_1()
                            .min_w_0()
                            .tooltip(Tooltip::for_action_title(
                                TEXT_FILTER_TOOLTIP,
                                &FocusTextFilter,
                            ))
                            .child(self.text_editor.clone()),
                    )
                    .when(has_text, |this| {
                        this.child(
                            div()
                                .id("git-log-text-filter-clear")
                                .flex_none()
                                .size(px(BUTTON_SIZE))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                    this.clear_button_hovered = *hovered;
                                    cx.notify();
                                }))
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.clear_text_field(window, cx)
                                }))
                                .child(Icon::new(clear_icon).size(IconSize::Medium)),
                        )
                    })
                    .child(regex_option)
                    .child(match_case_option),
            )
            .child(
                canvas(
                    move |bounds, _, _| bounds_cell.set(Some(bounds)),
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }

    fn render_chip(
        &self,
        kind: FilterKind,
        chip_height: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors();
        let value = self.chip_value(kind);
        let selected = value.is_some();
        let hovered = self.hovered_chip == Some(kind);
        let focus_handle = self.chip_focus_handles[kind.index()].clone();
        let focused = focus_handle.is_focused(window);
        let name_color = if hovered {
            colors.text
        } else {
            colors.text_muted
        };
        let value_color = colors.text;
        let button_hovered = self.hovered_chip_button == Some(kind);
        let icon = match (selected, button_hovered) {
            (true, true) => IconName::CloseSmallHovered,
            (true, false) => IconName::CloseSmall,
            (false, _) => IconName::GeneralChevronDown,
        };
        let tooltip = self.chip_tooltip(kind);
        let bounds_cell = self.chip_bounds[kind.index()].clone();
        let padding = if focused {
            CHIP_INSET - FOCUSED_CHIP_THICKNESS
        } else {
            CHIP_INSET
        };
        let focus_border = colors.border_focused;
        let button = div()
            .id(kind.button_id())
            .flex_none()
            .size(px(CHIP_ICON_SIZE))
            .flex()
            .items_center()
            .justify_center()
            .child(Icon::new(icon).size(IconSize::Medium))
            .when(selected, |this| {
                this.tooltip(Tooltip::for_action_title_in(
                    CLEAR_TEXT,
                    &ResetFilter,
                    &focus_handle,
                ))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered {
                        this.hovered_chip_button = Some(kind);
                    } else if this.hovered_chip_button == Some(kind) {
                        this.hovered_chip_button = None;
                    }
                    cx.notify();
                }))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.reset_filter(kind, cx);
                }))
            });
        h_flex()
            .id(kind.element_id())
            .track_focus(&focus_handle)
            .key_context(CHIP_KEY_CONTEXT)
            .relative()
            .flex_none()
            .h(chip_height)
            .p(px(padding))
            .rounded(px(FOCUSED_CHIP_ARC / 2.))
            .when(focused, |this| this.border_2().border_color(focus_border))
            .cursor_pointer()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hovered_chip = Some(kind);
                } else if this.hovered_chip == Some(kind) {
                    this.hovered_chip = None;
                }
                cx.notify();
            }))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.show_filter_popup(kind, window, cx)
            }))
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(move |this, _: &MouseUpEvent, _, cx| this.reset_filter(kind, cx)),
            )
            .on_action(cx.listener(move |this, _: &menu::Confirm, window, cx| {
                this.show_filter_popup(kind, window, cx)
            }))
            .on_action(cx.listener(move |this, _: &menu::SelectNext, window, cx| {
                this.show_filter_popup(kind, window, cx)
            }))
            .on_action(cx.listener(move |this, _: &ResetFilter, _, cx| this.reset_filter(kind, cx)))
            .when_some(tooltip, |this, tooltip| {
                this.tooltip(Tooltip::text(tooltip))
            })
            .child(
                Label::new(model::chip_name(kind.display_name(), selected))
                    .color(Color::Custom(name_color)),
            )
            .when_some(value, |this, value| {
                this.child(Label::new(value).color(Color::Custom(value_color)))
            })
            .child(div().flex_none().w(px(CHIP_GAP_BEFORE_ARROW)))
            .child(button)
            .child(
                canvas(
                    move |bounds, _, _| bounds_cell.set(Some(bounds)),
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .into_any_element()
    }

    fn render_graph_options(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let toolbar = cx.weak_entity();
        let non_default = self.tab_properties.graph_options != GraphOptions::default();
        let colors = cx.theme().colors();
        let icon_color = colors.icon;
        let hole_color = colors.panel_background;
        let badge = badge_color(cx);
        toolbar_cell()
            .child(
                PopoverMenu::new("git-log-graph-options")
                    .trigger_with_tooltip(
                        IconButton::new("git-log-graph-options-button", IconName::IntelliSort)
                            .shape(IconButtonShape::Square)
                            .icon_size(IconSize::Medium),
                        |_, cx| {
                            Tooltip::with_meta(
                                GRAPH_OPTIONS_TEXT,
                                None,
                                GRAPH_OPTIONS_DESCRIPTION,
                                cx,
                            )
                        },
                    )
                    .menu(move |window, cx| {
                        let toolbar = toolbar.clone();
                        Some(ContextMenu::build_persistent(
                            window,
                            cx,
                            move |menu, _, cx| graph_options_menu(menu, &toolbar, cx),
                        ))
                    })
                    .anchor(Anchor::TopLeft)
                    .attach(Anchor::BottomLeft),
            )
            .child(button_overlay(move |bounds, window| {
                paint_dropdown_arrow(bounds, icon_color, window);
                if non_default {
                    paint_badge(bounds, badge, hole_color, window);
                }
            }))
    }

    fn render_right_corner(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let toolbar = cx.weak_entity();
        let icon_color = cx.theme().colors().icon;
        h_flex()
            .flex_none()
            .h_full()
            .px(px(TOOLBAR_HORIZONTAL_INSET))
            .py(px(TOOLBAR_VERTICAL_INSET))
            .items_center()
            .child(
                toolbar_cell().child(
                    IconButton::new("git-log-refresh", IconName::GeneralRefresh)
                        .shape(IconButtonShape::Square)
                        .icon_size(IconSize::Medium)
                        .tooltip(|_, cx| {
                            Tooltip::with_meta(
                                REFRESH_TEXT,
                                Some(&Refresh as &dyn Action),
                                REFRESH_DESCRIPTION,
                                cx,
                            )
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                ),
            )
            .child(
                toolbar_cell().child(
                    IconButton::new("git-log-cherry-pick", IconName::CherryPick)
                        .shape(IconButtonShape::Square)
                        .icon_size(IconSize::Medium)
                        .disabled(!self.cherry_pick_enabled)
                        .tooltip(Tooltip::text(CHERRY_PICK_TEXT))
                        .on_click(
                            cx.listener(|_, _, _, cx| cx.emit(LogFilterToolbarEvent::CherryPick)),
                        ),
                ),
            )
            .child(
                toolbar_cell()
                    .child(
                        PopoverMenu::new("git-log-view-options")
                            .trigger_with_tooltip(
                                IconButton::new(
                                    "git-log-view-options-button",
                                    IconName::GeneralShow,
                                )
                                .shape(IconButtonShape::Square)
                                .icon_size(IconSize::Medium),
                                |_, cx| {
                                    Tooltip::with_meta(
                                        VIEW_OPTIONS_TEXT,
                                        None,
                                        VIEW_OPTIONS_DESCRIPTION,
                                        cx,
                                    )
                                },
                            )
                            .menu(move |window, cx| {
                                let toolbar = toolbar.clone();
                                Some(ContextMenu::build_persistent(
                                    window,
                                    cx,
                                    move |menu, _, cx| view_options_menu(menu, &toolbar, cx),
                                ))
                            })
                            .anchor(Anchor::TopLeft)
                            .attach(Anchor::BottomLeft),
                    )
                    .child(button_overlay(move |bounds, window| {
                        paint_dropdown_arrow(bounds, icon_color, window)
                    })),
            )
            .child(
                toolbar_cell().child(
                    IconButton::new("git-log-go-to", IconName::GeneralSearch)
                        .shape(IconButtonShape::Square)
                        .icon_size(IconSize::Medium)
                        .tooltip(|_, cx| {
                            Tooltip::with_meta(
                                GO_TO_TEXT,
                                Some(&GoToHashOrRef as &dyn Action),
                                GO_TO_DESCRIPTION,
                                cx,
                            )
                        })
                        .on_click(
                            cx.listener(|this, _, window, cx| this.show_go_to_popup(window, cx)),
                        ),
                ),
            )
    }
}

fn is_remote_head(name: &str) -> bool {
    name == HEAD_REVISION || name.ends_with("/HEAD")
}

fn label_font(window: &Window, cx: &App) -> (gpui::Font, Pixels) {
    let font = theme::theme_settings(cx).ui_font(cx).clone();
    let font_size = TextSize::Default.rems(cx).to_pixels(window.rem_size());
    (font, font_size)
}

fn label_font_height(window: &Window, cx: &App) -> f32 {
    let (font, font_size) = label_font(window, cx);
    let text_system = window.text_system();
    let font_id = text_system.resolve_font(&font);
    let ascent = f32::from(text_system.ascent(font_id, font_size));
    let descent = f32::from(text_system.descent(font_id, font_size)).abs();
    (ascent + descent).ceil()
}

fn text_field_width(window: &Window, cx: &App) -> Pixels {
    let (font, font_size) = label_font(window, cx);
    let text_system = window.text_system();
    let font_id = text_system.resolve_font(&font);
    let column_width = text_system
        .advance(font_id, font_size, 'm')
        .map(|size| f32::from(size.width))
        .unwrap_or(f32::from(font_size));
    let width = TEXT_FIELD_COLUMNS * column_width
        + SEARCH_ICON_AREA_WIDTH
        + 2. * BUTTON_SIZE
        + 2. * TEXT_FIELD_BORDER_INSET;
    px(width.max(TEXT_FIELD_MINIMUM_WIDTH))
}

fn search_option_colors(cx: &App) -> (Hsla, Hsla) {
    if cx.theme().appearance().is_light() {
        (
            rgb(SEARCH_OPTION_SELECTED_LIGHT).into(),
            rgb(SEARCH_OPTION_SELECTED_HOVERED_LIGHT).into(),
        )
    } else {
        (
            rgb(SEARCH_OPTION_SELECTED_DARK).into(),
            rgb(SEARCH_OPTION_SELECTED_HOVERED_DARK).into(),
        )
    }
}

fn badge_color(cx: &App) -> Hsla {
    if cx.theme().appearance().is_light() {
        rgb(BADGE_SUCCESS_LIGHT).into()
    } else {
        rgb(BADGE_SUCCESS_DARK).into()
    }
}

fn toolbar_cell() -> Div {
    div()
        .relative()
        .flex_none()
        .h(px(BUTTON_SIZE + 2. * BUTTON_ORTHOGONAL_GAP))
        .px(px(BUTTON_DIRECTIONAL_GAP))
        .flex()
        .items_center()
}

fn button_overlay(paint: impl FnOnce(Bounds<Pixels>, &mut Window) + 'static) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| paint(bounds, window),
    )
    .absolute()
    .top(px(BUTTON_ORTHOGONAL_GAP))
    .left(px(BUTTON_DIRECTIONAL_GAP))
    .size(px(BUTTON_SIZE))
}

fn icon_origin(bounds: Bounds<Pixels>) -> Point<Pixels> {
    let offset = px((BUTTON_SIZE - ICON_SIZE) / 2.);
    point(bounds.origin.x + offset, bounds.origin.y + offset)
}

fn paint_dropdown_arrow(bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    let origin = icon_origin(bounds);
    let at = |x: f32, y: f32| point(origin.x + px(x + 1.), origin.y + px(y + 1.));
    let mut arrow = PathBuilder::fill();
    arrow.move_to(at(16., 12.));
    arrow.line_to(at(16., 16.));
    arrow.line_to(at(12., 16.));
    arrow.close();
    if let Some(path) = arrow.build().log_err() {
        window.paint_path(path, color);
    }
}

fn circle(center: Point<Pixels>, radius: f32) -> PathBuilder {
    let radii = point(px(radius), px(radius));
    let mut builder = PathBuilder::fill();
    builder.move_to(point(center.x + px(radius), center.y));
    builder.arc_to(
        radii,
        px(0.),
        false,
        true,
        point(center.x - px(radius), center.y),
    );
    builder.arc_to(
        radii,
        px(0.),
        false,
        true,
        point(center.x + px(radius), center.y),
    );
    builder.close();
    builder
}

fn paint_badge(bounds: Bounds<Pixels>, color: Hsla, hole_color: Hsla, window: &mut Window) {
    let origin = icon_origin(bounds);
    let center = point(
        origin.x + px(ICON_SIZE * BADGE_CENTER_X),
        origin.y + px(ICON_SIZE * BADGE_CENTER_Y),
    );
    let radius = ICON_SIZE * BADGE_RADIUS;
    if let Some(path) = circle(center, radius + ICON_SIZE * BADGE_BORDER)
        .build()
        .log_err()
    {
        window.paint_path(path, hole_color);
    }
    if let Some(path) = circle(center, radius).build().log_err() {
        window.paint_path(path, color);
    }
}

fn paint_search_history_icon(bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    let origin = bounds.origin;
    let at = |x: f32, y: f32| point(origin.x + px(x), origin.y + px(y));
    let radii = point(px(4.5), px(4.5));
    let mut lens = PathBuilder::stroke(px(1.));
    lens.move_to(at(9.5, 7.));
    lens.arc_to(radii, px(0.), false, true, at(0.5, 7.));
    lens.arc_to(radii, px(0.), false, true, at(9.5, 7.));
    lens.close();
    if let Some(path) = lens.build().log_err() {
        window.paint_path(path, color);
    }
    let mut handle = PathBuilder::stroke(px(1.));
    handle.move_to(at(8.5, 10.5));
    handle.line_to(at(11.5, 13.5));
    if let Some(path) = handle.build().log_err() {
        window.paint_path(path, color);
    }
    let mut arrow = PathBuilder::fill();
    arrow.move_to(at(11., 7.));
    arrow.line_to(at(13.5, 9.5));
    arrow.line_to(at(16., 7.));
    arrow.close();
    if let Some(path) = arrow.build().log_err() {
        window.paint_path(path, color);
    }
}

fn graph_options_menu(
    menu: ContextMenu,
    toolbar: &WeakEntity<LogFilterToolbar>,
    cx: &App,
) -> ContextMenu {
    let Some(options) = toolbar
        .upgrade()
        .map(|toolbar| toolbar.read(cx).tab_properties.graph_options)
    else {
        return menu;
    };
    let select = |target: GraphOptions| {
        let toolbar = toolbar.clone();
        move |_: &mut Window, cx: &mut App| {
            toolbar
                .update(cx, |toolbar, cx| {
                    toolbar.update_tab(cx, |tab| tab.graph_options = target)
                })
                .ok();
        }
    };
    let first_parent = {
        let toolbar = toolbar.clone();
        move |_: &mut Window, cx: &mut App| {
            toolbar
                .update(cx, |toolbar, cx| {
                    toolbar.update_tab(cx, |tab| {
                        tab.graph_options = model::first_parent_toggled(tab.graph_options)
                    })
                })
                .ok();
        }
    };
    let by_date = GraphOptions::Base(GraphSortType::Normal);
    let topologically = GraphOptions::Base(GraphSortType::Bek);
    menu.header(SORT_SEPARATOR)
        .toggleable_entry(
            SORT_BY_COMMIT_DATE_TEXT,
            options == by_date,
            IconPosition::Start,
            None,
            select(by_date),
        )
        .toggleable_entry(
            SORT_TOPOLOGICALLY_TEXT,
            options == topologically,
            IconPosition::Start,
            None,
            select(topologically),
        )
        .header(OPTIONS_SEPARATOR)
        .toggleable_entry(
            FIRST_PARENT_TEXT,
            options == GraphOptions::FirstParent,
            IconPosition::Start,
            None,
            first_parent,
        )
}

fn view_options_menu(
    menu: ContextMenu,
    toolbar: &WeakEntity<LogFilterToolbar>,
    cx: &App,
) -> ContextMenu {
    let Some((app, tab)) = toolbar.upgrade().map(|toolbar| {
        let toolbar = toolbar.read(cx);
        (
            toolbar.app_properties.clone(),
            toolbar.tab_properties.clone(),
        )
    }) else {
        return menu;
    };
    let app_toggle = |update: fn(&mut LogAppProperties)| {
        let toolbar = toolbar.clone();
        move |_: &mut Window, cx: &mut App| {
            toolbar
                .update(cx, |toolbar, cx| toolbar.update_app(cx, update))
                .ok();
        }
    };
    let tab_toggle = |update: fn(&mut LogTabProperties)| {
        let toolbar = toolbar.clone();
        move |_: &mut Window, cx: &mut App| {
            toolbar
                .update(cx, |toolbar, cx| toolbar.update_tab(cx, update))
                .ok();
        }
    };
    let columns_toolbar = toolbar.clone();
    menu.header(SHOW_SEPARATOR)
        .toggleable_entry(
            COMPACT_REFERENCES_TEXT,
            app.compact_references,
            IconPosition::Start,
            None,
            app_toggle(|app| app.compact_references = !app.compact_references),
        )
        .toggleable_entry(
            TAG_NAMES_TEXT,
            app.show_tag_names,
            IconPosition::Start,
            None,
            app_toggle(|app| app.show_tag_names = !app.show_tag_names),
        )
        .toggleable_entry(
            LONG_EDGES_TEXT,
            tab.show_long_edges,
            IconPosition::Start,
            None,
            tab_toggle(|tab| tab.show_long_edges = !tab.show_long_edges),
        )
        .toggleable_entry(
            COMMIT_TIMESTAMP_TEXT,
            app.prefer_commit_date,
            IconPosition::Start,
            None,
            app_toggle(|app| app.prefer_commit_date = !app.prefer_commit_date),
        )
        .toggleable_entry(
            REFERENCES_ON_THE_LEFT_TEXT,
            app.labels_left_aligned,
            IconPosition::Start,
            None,
            app_toggle(|app| app.labels_left_aligned = !app.labels_left_aligned),
        )
        .submenu(COLUMNS_TEXT, move |menu, _, cx| {
            columns_menu(menu, &columns_toolbar, cx)
        })
        .separator()
        .header(HIGHLIGHT_SEPARATOR)
        .toggleable_entry(
            CURRENT_BRANCH_TEXT,
            tab.highlight_current_branch,
            IconPosition::Start,
            None,
            tab_toggle(|tab| tab.highlight_current_branch = !tab.highlight_current_branch),
        )
        .toggleable_entry(
            MERGE_COMMITS_TEXT,
            tab.highlight_merge_commits,
            IconPosition::Start,
            None,
            tab_toggle(|tab| tab.highlight_merge_commits = !tab.highlight_merge_commits),
        )
        .toggleable_entry(
            MY_COMMITS_TEXT,
            tab.highlight_my_commits,
            IconPosition::Start,
            None,
            tab_toggle(|tab| tab.highlight_my_commits = !tab.highlight_my_commits),
        )
}

fn columns_menu(
    menu: ContextMenu,
    toolbar: &WeakEntity<LogFilterToolbar>,
    cx: &App,
) -> ContextMenu {
    let Some(order) = toolbar
        .upgrade()
        .map(|toolbar| toolbar.read(cx).app_properties.column_order.clone())
    else {
        return menu;
    };
    model::DYNAMIC_COLUMNS.iter().fold(menu, |menu, column| {
        let column = *column;
        let toolbar = toolbar.clone();
        menu.toggleable_entry(
            column.title(),
            order.contains(&column),
            IconPosition::Start,
            None,
            move |_, cx| {
                toolbar
                    .update(cx, |toolbar, cx| {
                        toolbar.update_app(cx, |app| {
                            app.column_order = model::toggle_column(&app.column_order, column)
                        })
                    })
                    .ok();
            },
        )
    })
}

impl EventEmitter<LogFilterToolbarEvent> for LogFilterToolbar {}

impl Focusable for LogFilterToolbar {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for LogFilterToolbar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let height = self.height(window, cx);
        let chip_height = px(label_font_height(window, cx) + 2. * CHIP_INSET);
        let text_field = self.render_text_field(height, window, cx);
        let chips: Vec<AnyElement> = FilterKind::ALL
            .iter()
            .map(|kind| self.render_chip(*kind, chip_height, window, cx))
            .collect();
        let graph_options = self.render_graph_options(cx);
        let right_corner = self.render_right_corner(cx);
        let popup = self.open_popup.as_ref().map(|popup| {
            deferred(
                anchored()
                    .position(popup.anchor)
                    .anchor(Anchor::TopLeft)
                    .snap_to_window()
                    .child(popup.view.clone()),
            )
            .with_priority(1)
        });
        h_flex()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)))
            .on_action(
                cx.listener(|this, _: &GoToHashOrRef, window, cx| {
                    this.show_go_to_popup(window, cx)
                }),
            )
            .w_full()
            .h(height)
            .flex_none()
            .child(text_field)
            .child(
                h_flex()
                    .flex_none()
                    .h_full()
                    .px(px(TOOLBAR_HORIZONTAL_INSET))
                    .py(px(TOOLBAR_VERTICAL_INSET))
                    .items_center()
                    .children(chips)
                    .child(graph_options),
            )
            .child(div().flex_1())
            .child(right_corner)
            .children(popup)
    }
}
