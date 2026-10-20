use std::rc::Rc;
use std::sync::Arc;

use collections::{HashMap, HashSet};
use futures::channel::oneshot;
use git::repository::{Branch, FetchOptions, RepositoryOperation, Tag, Worktree};
use gpui::{
    Anchor, AnyElement, App, Context, DismissEvent, Entity, EntityId, FocusHandle, Global,
    HighlightStyle, Hsla, Pixels, SharedString, Subscription, Task, TextStyle, WeakEntity, Window,
    point, rgb,
};
use picker::{Picker, PickerDelegate};
use project::git_store::{GitStore, Repository, RepositoryEvent, RepositoryId, RepositorySnapshot};
use ui::{
    CommonAnimationExt as _, ContextMenu, Divider, HighlightedLabel, KeyBinding, KeyBindingStyle,
    ListItem, ListItemSpacing, PopoverMenu, PopoverMenuHandle, TextSize, Tooltip, prelude::*,
};
use ui_input::ErasedEditor;
use util::ResultExt as _;
use workspace::Workspace;

use super::ref_menu::{self, MenuAction, MenuContext, MenuItemPlan, MenuRunner};
use super::rows::{
    PopupRow, RefEntry, RepositoryChoice, RepositoryRow, RowKey, RowsInput, best_match,
    build_popup_rows, first_child_index, first_selectable, is_filtering, parent_index,
    position_of_current_branch, position_of_key, position_of_local_branch,
};
use super::state::{self, PopupSize, PopupState};
use super::top_actions::{self, TopAction, TopActionKind, TopActionsInput};
use super::tree::{self, FolderRow, RefRow, SectionKind, SectionRow, TreeInput};
use crate::branch_operations::{BranchContext, BranchNotice, integrate};
use crate::branch_refs::{RefKind, RefTarget};

const MAX_RECENT_BRANCHES: usize = 5;
const SHORT_SHA_LENGTH: usize = 8;
const MAX_COMMIT_COUNT_LABEL: u32 = 99;
pub(super) const ROW_HEIGHT_PX: f32 = 24.;
// IntelliJ's new UI header puts an 8 px drag area with 2 px top and bottom borders above the field.
pub(super) const HEADER_TOP_INSET_PX: f32 = 12.;
pub(super) const HEADER_LEFT_INSET_PX: f32 = 10.;
pub(super) const HEADER_RIGHT_INSET_PX: f32 = 8.;
pub(super) const HEADER_TOOLBAR_GAP_PX: f32 = 6.;
pub(super) const SEARCH_BOX_HEIGHT_PX: f32 = 28.;
const ICON_SLOT_PX: f32 = 16.;
const INDENT_STEP_PX: f32 = 16.;
// Separators are tree rows of the full row height in IntelliJ, with the line half a row down.
pub(super) const SEPARATOR_HEIGHT_PX: f32 = ROW_HEIGHT_PX;
pub(super) const FOLDER_ICON: IconName = IconName::Folder;
pub(super) const ROW_ICON_SIZE: IconSize = IconSize::Medium;
// `GitDefaultBranchesTreeRenderer` borders: incoming (1, 10, 0, 1), outgoing (1, 2, 0, 10), 10 px
// before the tracked branch, and a 1 px gap between each arrow and its count.
const INCOMING_MARGIN_LEFT_PX: f32 = 10.;
const INCOMING_MARGIN_RIGHT_PX: f32 = 1.;
const OUTGOING_MARGIN_LEFT_PX: f32 = 2.;
const OUTGOING_MARGIN_RIGHT_PX: f32 = 10.;
const COUNTER_MARGIN_TOP_PX: f32 = 1.;
const COUNTER_ICON_TEXT_GAP_PX: f32 = 1.;
const SECONDARY_TEXT_MARGIN_LEFT_PX: f32 = 10.;
const REPOSITORY_COLOR_SIDE_PX: f32 = 14.;
const REPOSITORY_COLOR_RADIUS_PX: f32 = 2.;
// `RepositoryColors.NEW_UI_REPOSITORY_COLORS` as (light, dark).
const REPOSITORY_COLORS: [(u32, u32); 14] = [
    (0xFF6285, 0x865762),
    (0x89C398, 0x436946),
    (0x6D5FF3, 0x5B53AF),
    (0xF2B181, 0x825845),
    (0x32E0D9, 0x389895),
    (0xFED277, 0x826A41),
    (0xF95CFF, 0x88458A),
    (0xBE906D, 0x947259),
    (0x4FF2B7, 0x178E63),
    (0xA177F4, 0x583D7A),
    (0x3FB3B8, 0x227985),
    (0xB979B9, 0x875887),
    (0xECCA88, 0x8C7B58),
    (0xC9CCD6, 0x5A5D6B),
];
// `GitIncomingOutgoingUi` as (light, dark).
const INCOMING_COLOR: (u32, u32) = (0x3574F0, 0x548AF7);
const OUTGOING_COLOR: (u32, u32) = (0x369650, 0x57965C);
// `SearchMatch.startBackground` of the New UI themes and the text `SimpleColoredComponent` draws on
// it, as (light, dark).
const SEARCH_MATCH_BACKGROUND: (u32, u32) = (0xFEE6B1, 0xBA9752);
const SEARCH_MATCH_FOREGROUND: (u32, u32) = (0x323232, 0x000000);
const FETCH_SUCCESSFUL: &str = "Fetch successful";
const FETCH_FAILED: &str = "Fetch failed";
const SEARCH_PLACEHOLDER_WITH_ACTIONS: &str = "Search for branches and actions";
const SEARCH_PLACEHOLDER_WITHOUT_ACTIONS: &str = "Search for branches";
const NOTHING_FOUND: &str = "Nothing found";
const BRANCH_NOT_FOUND: &str = "Branch not found";
const NO_REPOSITORY: &str = "No Git repository";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupToggle {
    ShowActionsInSearch,
    ShowRepositoriesInSearch,
    GroupByDirectory,
    ShowRecent,
    ShowTags,
}

impl PopupToggle {
    fn get(self, toggles: &state::PopupToggles) -> bool {
        match self {
            PopupToggle::ShowActionsInSearch => toggles.show_actions_in_search,
            PopupToggle::ShowRepositoriesInSearch => toggles.show_repositories_in_search,
            PopupToggle::GroupByDirectory => toggles.group_by_directory,
            PopupToggle::ShowRecent => toggles.show_recent,
            PopupToggle::ShowTags => toggles.show_tags,
        }
    }

    fn set(self, toggles: &mut state::PopupToggles, value: bool) {
        match self {
            PopupToggle::ShowActionsInSearch => toggles.show_actions_in_search = value,
            PopupToggle::ShowRepositoriesInSearch => toggles.show_repositories_in_search = value,
            PopupToggle::GroupByDirectory => toggles.group_by_directory = value,
            PopupToggle::ShowRecent => toggles.show_recent = value,
            PopupToggle::ShowTags => toggles.show_tags = value,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum StateEdit {
    Toggle(PopupToggle, bool),
    Favorite(RefTarget, bool),
    Expanded(SharedString, bool),
    Size(Option<PopupSize>),
}

impl StateEdit {
    fn apply(&self, state: &mut PopupState, favorite_defaults: &HashSet<RefTarget>) {
        match self {
            StateEdit::Toggle(toggle, value) => toggle.set(&mut state.toggles, *value),
            StateEdit::Favorite(target, favorite) => {
                if state.is_favorite(target, favorite_defaults) != *favorite {
                    state.toggle_favorite(target, favorite_defaults);
                }
            }
            StateEdit::Expanded(node_id, expanded) => {
                state.set_expanded(node_id.clone(), *expanded)
            }
            StateEdit::Size(size) => state.size = *size,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeafDirection {
    Next,
    Previous,
    First,
    Last,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoadScope {
    All,
    Operation,
}

#[derive(Default)]
pub struct RepoData {
    pub local: Vec<Branch>,
    pub remote: Vec<Branch>,
    pub tags: Vec<Tag>,
    pub recent: Vec<SharedString>,
    pub remote_names: Vec<SharedString>,
    pub operation: Option<RepositoryOperation>,
    pub current_branch: Option<SharedString>,
    pub head_short_sha: Option<SharedString>,
    pub head_sha: Option<SharedString>,
    pub has_commits: bool,
    pub other_worktrees: Arc<[Worktree]>,
}

impl RepoData {
    pub fn refresh_from_snapshot(&mut self, snapshot: &RepositorySnapshot) {
        self.local = snapshot
            .branch_list
            .iter()
            .filter(|branch| !branch.is_remote())
            .cloned()
            .collect();
        self.remote = snapshot
            .branch_list
            .iter()
            .filter(|branch| branch.is_remote())
            .cloned()
            .collect();
        self.current_branch = snapshot
            .branch
            .as_ref()
            .map(|branch| SharedString::from(branch.name().to_string()));
        self.head_short_sha = snapshot.head_commit.as_ref().map(|commit| {
            SharedString::from(
                commit
                    .sha
                    .chars()
                    .take(SHORT_SHA_LENGTH)
                    .collect::<String>(),
            )
        });
        self.head_sha = snapshot
            .head_commit
            .as_ref()
            .map(|commit| commit.sha.clone());
        self.has_commits = snapshot.head_commit.is_some();
        self.other_worktrees = snapshot.linked_worktrees.clone();
    }
}

type FullLoad = (
    oneshot::Receiver<anyhow::Result<Vec<Tag>>>,
    oneshot::Receiver<anyhow::Result<Vec<SharedString>>>,
    oneshot::Receiver<anyhow::Result<Vec<git::repository::Remote>>>,
);

type LoadedFull = (
    Option<Vec<Tag>>,
    Option<Vec<SharedString>>,
    Option<Vec<git::repository::Remote>>,
);

async fn receive<T>(receiver: oneshot::Receiver<anyhow::Result<T>>) -> Option<T> {
    receiver.await.ok()?.log_err()
}

pub fn char_positions_to_byte_offsets(text: &str, positions: &[usize]) -> Vec<usize> {
    if positions.is_empty() {
        return Vec::new();
    }
    let offsets: Vec<usize> = text.char_indices().map(|(offset, _)| offset).collect();
    positions
        .iter()
        .filter_map(|position| offsets.get(*position).copied())
        .collect()
}

pub fn commit_count_label(count: u32) -> String {
    if count > MAX_COMMIT_COUNT_LABEL {
        format!("{MAX_COMMIT_COUNT_LABEL}+")
    } else {
        count.to_string()
    }
}

pub fn commit_counts_tooltip(incoming: u32, outgoing: u32) -> String {
    let commits = |count: u32| if count == 1 { "commit" } else { "commits" };
    match (incoming, outgoing) {
        (incoming, outgoing) if incoming > 0 && outgoing > 0 => {
            format!("{incoming} incoming and {outgoing} outgoing commits")
        }
        (incoming, _) if incoming > 0 => format!("{incoming} incoming {}", commits(incoming)),
        (_, outgoing) => format!("{outgoing} outgoing {}", commits(outgoing)),
    }
}

pub fn ref_icon(is_current: bool, is_favorite: bool, is_tag: bool) -> IconName {
    match (is_current, is_favorite) {
        (true, true) => IconName::CurrentBranchFavoriteLabel,
        (true, false) => IconName::CurrentBranchLabel,
        (false, true) => IconName::StarFilled,
        (false, false) if is_tag => IconName::TagLabel,
        (false, false) => IconName::BranchNode,
    }
}

/// `GitBranchesTreeIconProvider.forRef`: the selected row offers to add a favorite with a hollow
/// star, even on the current branch.
pub fn ref_row_icon(is_current: bool, is_favorite: bool, is_tag: bool, selected: bool) -> IconName {
    if selected && !is_favorite {
        IconName::FavoriteOutline
    } else {
        ref_icon(is_current, is_favorite, is_tag)
    }
}

fn appearance_color((light, dark): (u32, u32), cx: &App) -> Hsla {
    if cx.theme().appearance().is_light() {
        rgb(light).into()
    } else {
        rgb(dark).into()
    }
}

/// The filled background IntelliJ draws behind speed-search matches.
pub fn search_match_style(cx: &App) -> HighlightStyle {
    HighlightStyle {
        color: Some(appearance_color(SEARCH_MATCH_FOREGROUND, cx)),
        background_color: Some(appearance_color(SEARCH_MATCH_BACKGROUND, cx)),
        ..Default::default()
    }
}

/// The incoming and outgoing counter colors of `GitIncomingOutgoingUi`.
pub fn commit_counter_colors(cx: &App) -> (Hsla, Hsla) {
    (
        appearance_color(INCOMING_COLOR, cx),
        appearance_color(OUTGOING_COLOR, cx),
    )
}

/// IntelliJ colors the repositories by their position among the repositories sorted by name.
pub fn repository_color(index: usize, cx: &App) -> Hsla {
    let palette_index = index % REPOSITORY_COLORS.len();
    let colors = REPOSITORY_COLORS
        .get(palette_index)
        .copied()
        .unwrap_or(REPOSITORY_COLORS[0]);
    appearance_color(colors, cx)
}

/// Fetches started from a branches popup, by git store: like IntelliJ's project-wide
/// `GitFetchSupport.isFetchRunning`, a popup opened while a fetch runs shows it as running.
#[derive(Default)]
pub(super) struct RunningFetches(HashSet<EntityId>);

impl Global for RunningFetches {}

fn is_fetch_running(git_store: Option<&Entity<GitStore>>, cx: &App) -> bool {
    git_store.is_some_and(|git_store| {
        cx.try_global::<RunningFetches>()
            .is_some_and(|running| running.0.contains(&git_store.entity_id()))
    })
}

/// `GitFetchResult.showNotification`: a title-only notice on success, the errors on failure.
pub fn fetch_result_notice(failures: &[(SharedString, String)], several: bool) -> BranchNotice {
    if failures.is_empty() {
        return BranchNotice::info("").title(FETCH_SUCCESSFUL);
    }
    let message = failures
        .iter()
        .map(|(repository, error)| {
            if several {
                format!("{repository}: {error}")
            } else {
                error.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    BranchNotice::error(message).title(FETCH_FAILED)
}

pub fn search_placeholder(show_actions: bool) -> &'static str {
    if show_actions {
        SEARCH_PLACEHOLDER_WITH_ACTIONS
    } else {
        SEARCH_PLACEHOLDER_WITHOUT_ACTIONS
    }
}

pub struct BranchesDelegate {
    pub(super) workspace: WeakEntity<Workspace>,
    pub(super) repository: Option<Entity<Repository>>,
    pub(super) _repository_subscription: Option<Subscription>,
    pub(super) focus_handle: FocusHandle,
    pub(super) data: RepoData,
    pub(super) state: PopupState,
    pub(super) state_key: Option<String>,
    pub(super) revealed: HashSet<SharedString>,
    pub(super) reveal_pending: bool,
    pub(super) state_loaded: bool,
    pub(super) data_loaded: bool,
    pub(super) rows: Vec<PopupRow>,
    pub(super) selected_index: usize,
    pub(super) preferred_selection_pending: bool,
    pub(super) query: String,
    pub(super) previous_branch: Option<String>,
    pub(super) section_repository_name: Option<SharedString>,
    pub(super) pack_requested: bool,
    pub(super) packed_size: Option<gpui::Size<Pixels>>,
    filtered_leaves: Vec<RowKey>,
    pending_selection: Option<RowKey>,
    pub(super) _fetch_observation: Option<Subscription>,
    update_project_shows_options: bool,
    menu_handles: HashMap<(SectionKind, RefTarget), PopoverMenuHandle<ContextMenu>>,
    edits_before_load: Vec<StateEdit>,
    pub(super) gear_menu_handle: PopoverMenuHandle<ContextMenu>,
    _full_load_task: Task<()>,
    _operation_load_task: Task<()>,
    _state_load_task: Task<()>,
    _previous_branch_load_task: Task<()>,
    pending_save: Task<()>,
}

impl BranchesDelegate {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        repository: Option<Entity<Repository>>,
        focus_handle: FocusHandle,
        cx: &App,
    ) -> Self {
        let mut data = RepoData::default();
        if let Some(repository) = &repository {
            data.refresh_from_snapshot(repository.read(cx));
        }
        Self {
            workspace,
            repository,
            _repository_subscription: None,
            focus_handle,
            data,
            state: PopupState::default(),
            state_key: None,
            revealed: HashSet::default(),
            reveal_pending: true,
            state_loaded: false,
            data_loaded: false,
            rows: Vec::new(),
            selected_index: 0,
            preferred_selection_pending: true,
            query: String::new(),
            previous_branch: None,
            section_repository_name: None,
            pack_requested: true,
            packed_size: None,
            filtered_leaves: Vec::new(),
            pending_selection: None,
            _fetch_observation: None,
            update_project_shows_options: true,
            menu_handles: HashMap::default(),
            edits_before_load: Vec::new(),
            gear_menu_handle: PopoverMenuHandle::default(),
            _full_load_task: Task::ready(()),
            _operation_load_task: Task::ready(()),
            _state_load_task: Task::ready(()),
            _previous_branch_load_task: Task::ready(()),
            pending_save: Task::ready(()),
        }
    }

    pub(super) fn fetching(&self, cx: &App) -> bool {
        is_fetch_running(self.git_store(cx).as_ref(), cx)
    }

    /// IntelliJ's search field turns its text red when nothing matches the query.
    pub(super) fn search_text_color(&self, cx: &App) -> Hsla {
        if self.nothing_matches() {
            Color::Error.color(cx)
        } else {
            cx.theme().colors().text
        }
    }

    /// Whether the search field shows the "nothing matches" error color (`updateSpeedSearchColors`).
    pub fn nothing_matches(&self) -> bool {
        self.repository.is_some() && is_filtering(&self.query) && self.rows.is_empty()
    }

    #[cfg(test)]
    pub fn rows(&self) -> &[PopupRow] {
        &self.rows
    }

    #[cfg(test)]
    pub fn selected_row(&self) -> Option<&PopupRow> {
        self.rows.get(self.selected_index)
    }

    #[cfg(test)]
    pub fn has_another_open_menu_for_tests(&self, _cx: &App) -> bool {
        self.gear_menu_handle.is_deployed()
            || self
                .rows
                .iter()
                .any(|row| matches!(row, PopupRow::Ref(entry) if entry.menu_handle.is_deployed()))
    }

    pub fn branch_context(&self) -> Option<BranchContext> {
        self.repository
            .clone()
            .map(|repository| BranchContext::new(self.workspace.clone(), repository))
    }

    pub fn attach_repository(&mut self, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        self.flush_pending_save();
        self._repository_subscription = None;
        self._full_load_task = Task::ready(());
        self._operation_load_task = Task::ready(());
        self._state_load_task = Task::ready(());
        self._previous_branch_load_task = Task::ready(());
        self.reveal_pending = true;
        self.state_loaded = false;
        self.data_loaded = false;
        self.previous_branch = None;
        self.revealed.clear();
        self.edits_before_load.clear();
        self.update_project_shows_options = integrate::should_show_update_options(cx);

        let Some(repository) = self.repository.clone() else {
            self.data = RepoData::default();
            self.state_key = None;
            self.state_loaded = true;
            self.data_loaded = true;
            return;
        };

        self.data.refresh_from_snapshot(repository.read(cx));
        self._repository_subscription = Some(cx.subscribe_in(
            &repository,
            window,
            |picker, _repository, event: &RepositoryEvent, window, cx| {
                Self::handle_repository_event(picker, event, window, cx);
            },
        ));

        let key = state::serialization_key(&repository.read(cx).common_dir_abs_path);
        self.state_key = Some(key.clone());
        let saved = cx
            .try_global::<state::SavedPopupStates>()
            .and_then(|saved| saved.get(&key))
            .cloned();
        if let Some(saved) = saved {
            self.apply_loaded_state(Some(saved), cx);
        } else {
            let persisted = state::load(key, cx);
            self._state_load_task = cx.spawn_in(window, async move |picker, cx| {
                let persisted = persisted.await;
                picker
                    .update_in(cx, |picker, window, cx| {
                        picker.delegate.apply_loaded_state(persisted, cx);
                        picker.refresh_placeholder(window, cx);
                        picker.refresh(window, cx);
                    })
                    .ok();
            });
        }

        let previous_branch_key =
            state::previous_branch_key(&repository.read(cx).work_directory_abs_path);
        match state::remembered_previous_branch(&previous_branch_key, cx) {
            Some(previous_branch) => self.previous_branch = Some(previous_branch),
            None => {
                let load = state::load_previous_branch(previous_branch_key, cx);
                self._previous_branch_load_task = cx.spawn_in(window, async move |picker, cx| {
                    let Some(previous_branch) = load.await else {
                        return;
                    };
                    picker
                        .update_in(cx, |picker, window, cx| {
                            picker.delegate.previous_branch = Some(previous_branch);
                            if picker.delegate.preferred_selection_pending {
                                picker.delegate.reveal_pending = true;
                                picker.refresh(window, cx);
                            }
                        })
                        .ok();
                });
            }
        }

        self.start_loading(LoadScope::All, window, cx);
    }

    fn handle_repository_event(
        picker: &mut Picker<Self>,
        event: &RepositoryEvent,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) {
        match event {
            RepositoryEvent::BranchListChanged | RepositoryEvent::HeadChanged => {
                picker.delegate.refresh_snapshot_data(cx);
                picker.delegate.start_loading(LoadScope::All, window, cx);
                picker.refresh(window, cx);
            }
            RepositoryEvent::StatusesChanged => {
                picker
                    .delegate
                    .start_loading(LoadScope::Operation, window, cx);
            }
            RepositoryEvent::GitWorktreeListChanged => {
                picker.delegate.refresh_snapshot_data(cx);
                picker.refresh(window, cx);
            }
            _ => {}
        }
    }

    pub(super) fn refresh_snapshot_data(&mut self, cx: &App) {
        if let Some(repository) = &self.repository {
            self.data.refresh_from_snapshot(repository.read(cx));
        }
    }

    fn start_loading(
        &mut self,
        scope: LoadScope,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) {
        let Some(repository) = self.repository.clone() else {
            return;
        };
        let (operation, full): (_, Option<FullLoad>) = repository.update(cx, |repository, _| {
            let operation = repository.operation_in_progress();
            let full = (scope == LoadScope::All).then(|| {
                (
                    repository.tags(),
                    repository.recent_branches(MAX_RECENT_BRANCHES),
                    repository.get_remotes(None, false),
                )
            });
            (operation, full)
        });

        let task = cx.spawn_in(window, async move |picker, cx| {
            let operation = receive(operation).await;
            let full = match full {
                Some((tags, recent, remotes)) => {
                    let tags = receive(tags).await;
                    let recent = receive(recent).await;
                    let remotes = receive(remotes).await;
                    Some((tags, recent, remotes))
                }
                None => None,
            };
            picker
                .update_in(cx, |picker, window, cx| {
                    picker.delegate.apply_loaded(operation, full);
                    picker.refresh(window, cx);
                })
                .ok();
        });
        match scope {
            LoadScope::All => self._full_load_task = task,
            LoadScope::Operation => self._operation_load_task = task,
        }
    }

    fn apply_loaded(
        &mut self,
        operation: Option<Option<RepositoryOperation>>,
        full: Option<LoadedFull>,
    ) {
        if let Some(operation) = operation {
            self.data.operation = operation;
        }
        if let Some((tags, recent, remotes)) = full {
            if let Some(tags) = tags {
                self.data.tags = tags;
            }
            if let Some(recent) = recent {
                self.data.recent = recent;
            }
            if let Some(remotes) = remotes {
                self.data.remote_names = remotes.into_iter().map(|remote| remote.name).collect();
            }
            self.data_loaded = true;
        }
    }

    fn reveal_current_branch_if_ready(&mut self) {
        if !self.reveal_pending || !self.state_loaded || !self.data_loaded {
            return;
        }
        self.reveal_pending = false;
        self.revealed.clear();
        let current = self.data.current_branch.clone();
        let previous = self.previous_local_branch().map(SharedString::from);
        for name in [current, previous].into_iter().flatten() {
            self.reveal_local_branch(&name);
        }
    }

    fn reveal_local_branch(&mut self, name: &str) {
        let toggles = self.state.toggles;
        let in_recent = toggles.show_recent
            && self
                .data
                .recent
                .iter()
                .take(MAX_RECENT_BRANCHES)
                .any(|recent| recent == name);
        let section = if in_recent {
            SectionKind::Recent
        } else {
            SectionKind::Local
        };
        self.revealed.extend(tree::ancestor_node_ids(
            section,
            name,
            toggles.group_by_directory,
        ));
    }

    /// IntelliJ preselects the branch HEAD was on before the last checkout, when it is still a
    /// local branch other than the current one (`GitBranchesTreeModelUtil.getPreferredBranch`).
    fn previous_local_branch(&self) -> Option<&str> {
        let previous = self.previous_branch.as_deref()?;
        let exists = self
            .data
            .local
            .iter()
            .any(|branch| branch.name() == previous);
        (exists && self.data.current_branch.as_deref() != Some(previous)).then_some(previous)
    }

    fn git_store(&self, cx: &App) -> Option<Entity<GitStore>> {
        self.repository
            .as_ref()
            .and_then(|repository| repository.read(cx).git_store())
    }

    pub fn repository_choices(&self, cx: &App) -> Vec<RepositoryChoice> {
        let Some(git_store) = self.git_store(cx) else {
            return Vec::new();
        };
        let active_id = self
            .repository
            .as_ref()
            .map(|repository| repository.read(cx).id);
        let mut choices: Vec<RepositoryChoice> = git_store
            .read(cx)
            .repositories()
            .values()
            .map(|repository| {
                let repository = repository.read(cx);
                let branch = match (repository.branch.as_ref(), repository.head_commit.as_ref()) {
                    (Some(branch), _) => Some(SharedString::from(branch.name().to_string())),
                    (None, Some(commit)) => Some(SharedString::from(
                        commit
                            .sha
                            .chars()
                            .take(SHORT_SHA_LENGTH)
                            .collect::<String>(),
                    )),
                    (None, None) => None,
                };
                RepositoryChoice {
                    id: repository.id,
                    name: repository.display_name(),
                    branch,
                    is_active: Some(repository.id) == active_id,
                    color_index: 0,
                }
            })
            .collect();
        choices.sort_by(|left, right| {
            (&*left.name)
                .cmp(&*right.name)
                .then_with(|| left.id.cmp(&right.id))
        });
        for (index, choice) in choices.iter_mut().enumerate() {
            choice.color_index = index;
        }
        choices
    }

    fn find_repository(&self, id: RepositoryId, cx: &App) -> Option<Entity<Repository>> {
        self.git_store(cx)?
            .read(cx)
            .repositories()
            .get(&id)
            .cloned()
    }

    fn head_tag(&self) -> Option<&str> {
        if self.data.current_branch.is_some() {
            return None;
        }
        let head_sha = self.data.head_sha.as_deref()?;
        self.data
            .tags
            .iter()
            .find(|tag| &*tag.commit_sha == head_sha)
            .map(|tag| &*tag.name)
    }

    fn build_rows_with(
        &self,
        expanded: &HashSet<SharedString>,
        expand_all: bool,
        repositories: &[RepositoryChoice],
        menu_handles: &mut HashMap<(SectionKind, RefTarget), PopoverMenuHandle<ContextMenu>>,
    ) -> Vec<PopupRow> {
        let defaults = tree::default_favorites(&self.data.local, &self.data.remote);
        let favorites = self.state.resolve_favorites(&defaults);
        let tree_rows = tree::build_rows(&TreeInput {
            local: &self.data.local,
            remote: &self.data.remote,
            tags: &self.data.tags,
            recent: &self.data.recent,
            current_branch: self.data.current_branch.as_deref(),
            head_tag: self.head_tag(),
            favorites: &favorites,
            options: self.state.toggles.tree_options(),
            expanded,
            expand_all,
            query: &self.query,
        });
        let top_actions = top_actions::top_actions_plan(&TopActionsInput {
            has_commits: self.data.has_commits,
            operation: self.data.operation,
            update_project_shows_options: self.update_project_shows_options,
        });
        build_popup_rows(RowsInput {
            query: &self.query,
            show_actions_in_search: self.state.toggles.show_actions_in_search,
            show_repositories_in_search: self.state.toggles.show_repositories_in_search,
            top_actions: &top_actions,
            repositories,
            tree_rows,
            menu_handles,
        })
    }

    fn rebuild_rows(&mut self, reveal_matches: bool, cx: &App) {
        if self.repository.is_none() {
            self.rows.clear();
            self.filtered_leaves.clear();
            return;
        }
        self.reveal_current_branch_if_ready();
        let repositories = self.repository_choices(cx);
        self.section_repository_name = (repositories.len() > 1)
            .then(|| repositories.iter().find(|choice| choice.is_active))
            .flatten()
            .map(|choice| choice.name.clone());
        self.filtered_leaves.clear();
        if is_filtering(&self.query) {
            let all_rows = self.build_rows_with(
                &HashSet::default(),
                true,
                &repositories,
                &mut HashMap::default(),
            );
            self.filtered_leaves = all_rows
                .iter()
                .filter(|row| row.is_selectable_while_filtering())
                .filter_map(PopupRow::key)
                .collect();
            if reveal_matches {
                self.reveal_for_query(&all_rows);
            }
        }
        let mut expanded = self.state.expanded().clone();
        expanded.extend(self.revealed.iter().cloned());
        let mut menu_handles = std::mem::take(&mut self.menu_handles);
        self.rows = self.build_rows_with(&expanded, false, &repositories, &mut menu_handles);
        self.menu_handles = menu_handles;
    }

    /// `GitBranchesPopupBase.applySearchPattern`: a search opens the paths of the current branch,
    /// of the recent branches and of the best match; other sections and folders keep the state
    /// they had, and what this opens stays open for the rest of the popup's life.
    fn reveal_for_query(&mut self, all_rows: &[PopupRow]) {
        let group_by_directory = self.state.toggles.group_by_directory;
        let mut reveal = Vec::new();
        if let Some(current) = &self.data.current_branch {
            reveal.push(current.clone());
        }
        if self.state.toggles.show_recent {
            reveal.extend(self.data.recent.iter().take(MAX_RECENT_BRANCHES).cloned());
        }
        for name in reveal {
            self.reveal_local_branch(&name);
        }
        if let Some(PopupRow::Ref(entry)) = best_match(all_rows).and_then(|ix| all_rows.get(ix)) {
            self.revealed.extend(tree::ancestor_node_ids(
                entry.row.section,
                &entry.row.target.name,
                group_by_directory,
            ));
        }
    }

    /// Moves the selection to the next leaf of the search results, opening the sections and
    /// folders on the way, like `GitBranchesTreeUtil.selectNext` and friends.
    pub fn select_leaf(&mut self, direction: LeafDirection) -> bool {
        let count = self.filtered_leaves.len();
        if count == 0 {
            return false;
        }
        let current = self
            .rows
            .get(self.selected_index)
            .and_then(PopupRow::key)
            .and_then(|key| self.filtered_leaves.iter().position(|leaf| *leaf == key));
        let target = match direction {
            LeafDirection::Next => current.map_or(0, |position| (position + 1) % count),
            LeafDirection::Previous => {
                current.map_or(count - 1, |position| (position + count - 1) % count)
            }
            LeafDirection::First => 0,
            LeafDirection::Last => count - 1,
        };
        let Some(key) = self.filtered_leaves.get(target).cloned() else {
            return false;
        };
        if let RowKey::Ref(section, target) = &key {
            self.revealed.extend(tree::ancestor_node_ids(
                *section,
                &target.name,
                self.state.toggles.group_by_directory,
            ));
        }
        self.pending_selection = Some(key);
        true
    }

    fn choose_selection(&mut self, previous_key: Option<RowKey>, query_changed: bool) -> usize {
        if let Some(key) = self.pending_selection.take()
            && let Some(position) = position_of_key(&self.rows, &key)
        {
            return position;
        }
        let first = first_selectable(&self.rows).unwrap_or(0);
        let top_match = best_match(&self.rows).unwrap_or(first);
        let previous = previous_key.and_then(|key| position_of_key(&self.rows, &key));
        if is_filtering(&self.query) {
            return if query_changed {
                top_match
            } else {
                previous.unwrap_or(top_match)
            };
        }
        let preferred = self
            .previous_local_branch()
            .and_then(|name| position_of_local_branch(&self.rows, name))
            .or_else(|| position_of_current_branch(&self.rows));
        if self.preferred_selection_pending || query_changed {
            return preferred.or(previous).unwrap_or(first);
        }
        previous.or(preferred).unwrap_or(first)
    }

    fn is_node_expanded(&self, node_id: &SharedString) -> bool {
        self.state.is_expanded(node_id) || self.revealed.contains(node_id)
    }

    fn toggle_node(&mut self, node_id: SharedString, cx: &App) {
        let expand = !self.is_node_expanded(&node_id);
        if !expand {
            self.revealed.remove(&node_id);
        }
        self.record_edit(StateEdit::Expanded(node_id, expand), cx);
    }

    fn favorite_defaults(&self) -> HashSet<RefTarget> {
        tree::default_favorites(&self.data.local, &self.data.remote)
    }

    fn record_edit(&mut self, edit: StateEdit, cx: &App) {
        let defaults = self.favorite_defaults();
        edit.apply(&mut self.state, &defaults);
        if self.state_loaded {
            self.persist_state(cx);
        } else {
            self.edits_before_load.push(edit);
        }
    }

    fn apply_loaded_state(&mut self, persisted: Option<state::PersistedPopupState>, cx: &App) {
        let mut loaded = persisted
            .map(PopupState::from_persisted)
            .unwrap_or_default();
        let defaults = self.favorite_defaults();
        let edits = std::mem::take(&mut self.edits_before_load);
        for edit in &edits {
            edit.apply(&mut loaded, &defaults);
        }
        self.state = loaded;
        self.state_loaded = true;
        if !edits.is_empty() {
            self.persist_state(cx);
        }
    }

    fn persist_state(&mut self, cx: &App) {
        if let Some(key) = &self.state_key {
            self.pending_save = state::save(key.clone(), self.state.to_persisted(), cx);
        }
    }

    fn flush_pending_save(&mut self) {
        std::mem::replace(&mut self.pending_save, Task::ready(())).detach();
    }

    pub fn toggle_favorite_at(&mut self, ix: usize, cx: &App) -> bool {
        let Some(PopupRow::Ref(entry)) = self.rows.get(ix) else {
            return false;
        };
        let target = entry.row.target.clone();
        let favorite = !self.state.is_favorite(&target, &self.favorite_defaults());
        self.record_edit(StateEdit::Favorite(target, favorite), cx);
        true
    }

    pub fn toggle_favorite_of_selected(&mut self, cx: &App) -> bool {
        self.toggle_favorite_at(self.selected_index, cx)
    }

    /// IntelliJ's `visibleRowCount`: the top-level rows plus the children of Recent (or of Local
    /// without recent branches) plus one.
    pub fn visible_row_count(&self) -> usize {
        let top_level = self
            .rows
            .iter()
            .filter(|row| matches!(row.tree_depth(), None | Some(0)))
            .count();
        let defaults = self.favorite_defaults();
        let favorites = self.state.resolve_favorites(&defaults);
        let expanded = HashSet::default();
        let input = TreeInput {
            local: &self.data.local,
            remote: &self.data.remote,
            tags: &self.data.tags,
            recent: &self.data.recent,
            current_branch: self.data.current_branch.as_deref(),
            head_tag: self.head_tag(),
            favorites: &favorites,
            options: self.state.toggles.tree_options(),
            expanded: &expanded,
            expand_all: false,
            query: "",
        };
        let section = if self.state.toggles.show_recent {
            SectionKind::Recent
        } else {
            SectionKind::Local
        };
        top_level + tree::section_child_count(&input, section) + 1
    }

    /// The width of the widest row, which IntelliJ's `pack` gives the popup.
    pub fn preferred_row_width(&self, window: &mut Window, cx: &App) -> Pixels {
        let text_style = window.text_style();
        let rem_size = window.rem_size();
        let default_size = TextSize::Default.rems(cx).to_pixels(rem_size);
        let small_size = TextSize::Small.rems(cx).to_pixels(rem_size);
        let measure = |text: &str, small: bool, window: &mut Window| -> Pixels {
            let font_size = if small { small_size } else { default_size };
            text_width(text, font_size, &text_style, window)
        };
        let metrics = ui::PopupRowMetrics::for_style(ui::PopupRowStyle::List);
        let chrome = (metrics.pill_inset_x + metrics.content_padding_x) * 2.;
        let start_gap = DynamicSpacing::Base06.px(cx);
        let slot = px(ICON_SLOT_PX);
        let slot_gap = px(4.);
        let arrow = px(22.) + px(4.);
        let tree_prefix = |depth: usize| px(INDENT_STEP_PX * depth as f32) + (slot + slot_gap) * 2.;
        let mut widest = px(0.);
        for row in &self.rows {
            let content = match row {
                PopupRow::Separator => px(0.),
                PopupRow::Action { action, .. } => {
                    let shortcut = action
                        .shortcut_action()
                        .and_then(|shortcut_action| {
                            KeyBinding::for_action_in(
                                shortcut_action.as_ref(),
                                &self.focus_handle,
                                cx,
                            )
                            .keyboard_shortcut_text(window, cx)
                        })
                        .map(|text| px(4.) + measure(&text, false, window))
                        .unwrap_or(px(0.));
                    slot + start_gap + measure(&action.label, false, window) + shortcut
                }
                PopupRow::Repository(repository_row) => {
                    let branch = repository_row
                        .choice
                        .branch
                        .as_deref()
                        .map(|branch| {
                            px(SECONDARY_TEXT_MARGIN_LEFT_PX) + measure(branch, false, window)
                        })
                        .unwrap_or(px(0.));
                    slot + start_gap
                        + measure(&repository_row.choice.name, false, window)
                        + branch
                        + arrow
                }
                PopupRow::Section(section) => {
                    let title = section.kind.title(self.section_repository_name.as_deref());
                    slot + start_gap + measure(&title, false, window)
                }
                PopupRow::Folder(folder) => {
                    tree_prefix(folder.depth) + start_gap + measure(&folder.label, false, window)
                }
                PopupRow::Ref(entry) => {
                    let ref_row = &entry.row;
                    let counter = |count: u32, margins: f32, window: &mut Window| {
                        if count == 0 {
                            return px(0.);
                        }
                        px(margins + COUNTER_ICON_TEXT_GAP_PX)
                            + slot
                            + measure(&commit_count_label(count), true, window)
                    };
                    let incoming = counter(
                        ref_row.incoming,
                        INCOMING_MARGIN_LEFT_PX + INCOMING_MARGIN_RIGHT_PX,
                        window,
                    );
                    let outgoing = counter(
                        ref_row.outgoing,
                        OUTGOING_MARGIN_LEFT_PX + OUTGOING_MARGIN_RIGHT_PX,
                        window,
                    );
                    let tracked = ref_row
                        .tracked
                        .as_deref()
                        .map(|tracked| {
                            px(SECONDARY_TEXT_MARGIN_LEFT_PX) + measure(tracked, false, window)
                        })
                        .unwrap_or(px(0.));
                    tree_prefix(ref_row.depth)
                        + start_gap
                        + measure(&ref_row.label, false, window)
                        + incoming
                        + outgoing
                        + tracked
                        + arrow
                }
            };
            widest = widest.max(content + chrome);
        }
        widest
    }

    /// The popup sizes itself once the saved state and the refs it shows are known, like
    /// IntelliJ's `pack` at show time.
    pub fn is_ready_to_pack(&self) -> bool {
        self.state_loaded && self.data_loaded
    }

    pub fn user_size(&self) -> Option<PopupSize> {
        self.state.size
    }

    pub fn set_user_size(&mut self, size: PopupSize, cx: &App) {
        self.record_edit(StateEdit::Size(Some(size)), cx);
    }

    /// "Restore Popup Size": forget the user's size and size the popup to its rows again.
    pub fn restore_popup_size(&mut self, cx: &App) {
        self.record_edit(StateEdit::Size(None), cx);
        self.pack_requested = true;
    }

    /// IntelliJ's `pack`: once the rows are known, the popup takes the size they need.
    fn pack_if_requested(&mut self, window: &mut Window, cx: &App) {
        if !self.pack_requested || !self.is_ready_to_pack() {
            return;
        }
        self.pack_requested = false;
        self.packed_size = Some(super::packed_size(
            self.preferred_row_width(window, cx),
            self.visible_row_count(),
            DynamicSpacing::Base04.px(cx),
        ));
    }

    pub fn toggle_setting(&mut self, toggle: PopupToggle, cx: &App) {
        let value = !toggle.get(&self.state.toggles);
        self.record_edit(StateEdit::Toggle(toggle, value), cx);
    }

    pub fn close_open_ref_menu(&self, window: &Window, cx: &mut App) -> bool {
        for row in &self.rows {
            if let PopupRow::Ref(entry) = row
                && entry.menu_handle.is_focused(window, cx)
            {
                entry.menu_handle.hide(cx);
                return true;
            }
        }
        false
    }

    fn switch_repository(
        &mut self,
        id: RepositoryId,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) {
        let Some(repository) = self.find_repository(id, cx) else {
            return;
        };
        let already_active = self
            .repository
            .as_ref()
            .is_some_and(|current| current.entity_id() == repository.entity_id());
        if already_active {
            return;
        }
        self.repository = Some(repository);
        self.data = RepoData::default();
        self.state = PopupState::default();
        self.preferred_selection_pending = true;
        self.attach_repository(window, cx);
        cx.defer_in(window, |picker, window, cx| {
            picker.refresh_placeholder(window, cx);
            picker.refresh(window, cx);
        });
    }

    /// IntelliJ's popup Fetch fetches every repository of the project (`GitFetch.performFetch`).
    pub(super) fn start_fetch(&mut self, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        let (Some(context), Some(git_store)) = (self.branch_context(), self.git_store(cx)) else {
            return;
        };
        if is_fetch_running(Some(&git_store), cx) {
            return;
        }
        let mut repositories: Vec<(SharedString, Entity<Repository>)> = git_store
            .read(cx)
            .repositories()
            .values()
            .map(|repository| (repository.read(cx).display_name(), repository.clone()))
            .collect();
        repositories.sort_by(|left, right| left.0.cmp(&right.0));
        let several = repositories.len() > 1;
        let git_store_id = git_store.entity_id();
        cx.default_global::<RunningFetches>().0.insert(git_store_id);
        cx.notify();

        let receivers: Vec<_> = repositories
            .into_iter()
            .map(|(name, repository)| {
                let askpass = context.askpass_delegate("git fetch", window, cx);
                let receiver = repository.update(cx, |repository, cx| {
                    repository.fetch(FetchOptions::All, askpass, cx)
                });
                (name, receiver)
            })
            .collect();
        cx.spawn(async move |_, cx| {
            let mut failures = Vec::new();
            let mut cancelled = false;
            for (name, receiver) in receivers {
                let error = match receiver.await {
                    Ok(Ok(_)) => continue,
                    Ok(Err(error)) => error,
                    Err(canceled) => anyhow::Error::new(canceled),
                };
                match crate::branch_operations::failure_text(&error) {
                    Some(text) => failures.push((name, text.trim().to_string())),
                    None => cancelled = true,
                }
            }
            cx.update(|cx| {
                cx.default_global::<RunningFetches>()
                    .0
                    .remove(&git_store_id);
                if cancelled && failures.is_empty() {
                    return;
                }
                context.notify(fetch_result_notice(&failures, several), cx);
            });
        })
        .detach();
    }

    /// Runs `Delete` on the ref whose menu is open (the selected ref when no menu is open) when
    /// its menu offers it, for IntelliJ's `$Delete` shortcut in the ref menu.
    pub fn delete_selected_ref(&mut self, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        // Hovering keeps moving the selection while a ref menu is open, so the selection may
        // be on another row than the menu.
        let menu_entry = self.rows.iter().find_map(|row| match row {
            PopupRow::Ref(entry) if entry.menu_handle.is_deployed() => Some(entry),
            _ => None,
        });
        let entry = menu_entry.or_else(|| match self.rows.get(self.selected_index) {
            Some(PopupRow::Ref(entry)) => Some(entry),
            _ => None,
        });
        let Some(entry) = entry else {
            return;
        };
        let target = entry.row.target.clone();
        let deletable = self.menu_plan_for(&target, cx).iter().any(|planned| {
            matches!(
                planned,
                MenuItemPlan::Item(item) if item.action == MenuAction::Delete && item.enabled
            )
        });
        let Some(context) = self.branch_context() else {
            return;
        };
        if !deletable {
            return;
        }
        cx.emit(DismissEvent);
        window.defer(cx, move |window, cx| {
            ref_menu::run_menu_action(MenuAction::Delete, context, target, window, cx)
        });
    }

    pub(super) fn run_top_action(
        &mut self,
        kind: TopActionKind,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) {
        let Some(context) = self.branch_context() else {
            return;
        };
        let shift_held = window.modifiers().shift;
        cx.emit(DismissEvent);
        window.defer(cx, move |window, cx| {
            top_actions::run_top_action(kind, shift_held, context, window, cx)
        });
    }

    fn select_row_later(&self, ix: usize, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        cx.defer_in(window, move |picker, window, cx| {
            picker.set_selected_index(ix, None, true, window, cx);
        });
    }

    fn refresh_later(&self, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        cx.defer_in(window, |picker, window, cx| picker.refresh(window, cx));
    }

    pub(super) fn menu_plan_for(&self, target: &RefTarget, cx: &App) -> Vec<MenuItemPlan> {
        let protected_branches = integrate::protected_branch_patterns(cx);
        let local_branch = match target.kind {
            RefKind::Local => self
                .data
                .local
                .iter()
                .find(|branch| branch.name() == &*target.name),
            RefKind::Remote | RefKind::Tag => None,
        };
        let has_tracked_branch = local_branch.is_some_and(|branch| branch.upstream.is_some());
        let tracked_branch = local_branch.and_then(ref_menu::tracked_remote_branch);
        let other_worktree = local_branch.and_then(|branch| {
            ref_menu::worktree_with_branch(branch.name(), &self.data.other_worktrees)
        });

        let tag_commit_sha = match target.kind {
            RefKind::Tag => self
                .data
                .tags
                .iter()
                .find(|tag| tag.name == target.name)
                .map(|tag| &*tag.commit_sha),
            RefKind::Local | RefKind::Remote => None,
        };

        ref_menu::menu_plan(&MenuContext {
            target,
            current_branch: self.data.current_branch.as_deref(),
            head_short_sha: self.data.head_short_sha.as_deref(),
            head_sha: self.data.head_sha.as_deref(),
            tag_commit_sha,
            has_commits: self.data.has_commits,
            remotes: &self.data.remote_names,
            has_tracked_branch,
            tracked_branch: tracked_branch.as_deref(),
            other_worktree,
            fetching: self.fetching(cx),
            protected_branches: &protected_branches,
        })
    }
}

impl Drop for BranchesDelegate {
    fn drop(&mut self) {
        self.flush_pending_save();
    }
}

fn text_width(
    text: &str,
    font_size: Pixels,
    text_style: &TextStyle,
    window: &mut Window,
) -> Pixels {
    if text.is_empty() {
        return px(0.);
    }
    let run = text_style.to_run(text.len());
    window
        .text_system()
        .shape_line(
            SharedString::from(text.to_string()),
            font_size,
            &[run],
            None,
        )
        .width
}

fn chevron(expanded: bool) -> Icon {
    Icon::new(if expanded {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    })
    .size(ROW_ICON_SIZE)
    .color(Color::Muted)
}

fn icon_slot(icon: Option<impl IntoElement>) -> AnyElement {
    h_flex()
        .flex_none()
        .size(rems_from_px(ICON_SLOT_PX))
        .justify_center()
        .children(icon)
        .into_any_element()
}

fn indent(depth: usize) -> Div {
    div()
        .flex_none()
        .w(rems_from_px(INDENT_STEP_PX * depth as f32))
}

fn base_row(ix: usize, selected: bool) -> ListItem {
    ListItem::new(("branches-popup-row", ix))
        .inset(true)
        .height(rems_from_px(ROW_HEIGHT_PX))
        .spacing(ListItemSpacing::Dense)
        .toggle_state(selected)
}

fn menu_arrow(ix: usize) -> IconButton {
    IconButton::new(("branches-popup-ref-arrow", ix), IconName::MenuArrow)
        .icon_size(IconSize::XSmall)
        .icon_color(Color::Muted)
        .style(ButtonStyle::Transparent)
}

/// Where a ref submenu opens relative to the arrow it is attached to: IntelliJ shows it 2 px
/// left of the popup's right edge, level with the row (`GitBranchesPopupBase.handleNextStep`).
/// The arrow sits inside the row's pill and content padding and is centered in the row; the
/// popup's 1 px border lies outside the row.
pub fn ref_menu_offset() -> gpui::Point<Pixels> {
    let metrics = ui::PopupRowMetrics::for_style(ui::PopupRowStyle::List);
    let arrow_to_row_right = metrics.pill_inset_x + metrics.content_padding_x;
    let arrow_height = px(22.);
    let arrow_top_in_row = (px(ROW_HEIGHT_PX) - arrow_height) / 2.;
    point(arrow_to_row_right - px(1.), -arrow_top_in_row)
}

fn highlighted(label: &SharedString, positions: &[usize], cx: &App) -> HighlightedLabel {
    HighlightedLabel::new(
        label.clone(),
        char_positions_to_byte_offsets(label, positions),
    )
    .highlight_style(search_match_style(cx))
}

fn render_separator() -> AnyElement {
    h_flex()
        .w_full()
        .h(rems_from_px(SEPARATOR_HEIGHT_PX))
        .px_2()
        .child(div().w_full().child(Divider::horizontal()))
        .into_any_element()
}

fn render_section_row(
    ix: usize,
    selected: bool,
    section: &SectionRow,
    repository_name: Option<&str>,
) -> AnyElement {
    base_row(ix, selected)
        .start_slot(icon_slot(Some(chevron(section.expanded))))
        .child(Label::new(section.kind.title(repository_name)).truncate())
        .into_any_element()
}

fn render_folder_row(ix: usize, selected: bool, folder: &FolderRow) -> AnyElement {
    base_row(ix, selected)
        .start_slot(
            h_flex()
                .gap_1()
                .child(indent(folder.depth))
                .child(icon_slot(Some(chevron(folder.expanded))))
                .child(icon_slot(Some(
                    Icon::new(FOLDER_ICON)
                        .size(ROW_ICON_SIZE)
                        .color(Color::Muted),
                ))),
        )
        .child(Label::new(folder.label.clone()).truncate())
        .into_any_element()
}

/// `GitDefaultBranchesTreeRenderer`: a repository row shows the repository color, its branch as
/// secondary text, and the arrow of its nested popup.
fn render_repository_row(ix: usize, selected: bool, row: &RepositoryRow, cx: &App) -> AnyElement {
    let choice = &row.choice;
    let color = div()
        .size(rems_from_px(REPOSITORY_COLOR_SIDE_PX))
        .rounded(px(REPOSITORY_COLOR_RADIUS_PX))
        .bg(repository_color(choice.color_index, cx));
    base_row(ix, selected)
        .start_slot(icon_slot(Some(color)))
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .child(
                    h_flex()
                        .min_w_0()
                        .flex_shrink_1()
                        .child(highlighted(&choice.name, &row.highlight_positions, cx).truncate()),
                )
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .pl(rems_from_px(SECONDARY_TEXT_MARGIN_LEFT_PX))
                        .justify_end()
                        .overflow_hidden()
                        .children(
                            choice
                                .branch
                                .clone()
                                .map(|branch| Label::new(branch).color(Color::Muted).truncate()),
                        ),
                ),
        )
        .end_slot(menu_arrow(ix))
        .into_any_element()
}

fn render_commit_counter(
    icon: IconName,
    count: u32,
    color: Hsla,
    margin_left: f32,
    margin_right: f32,
) -> impl IntoElement {
    h_flex()
        .flex_none()
        .mt(rems_from_px(COUNTER_MARGIN_TOP_PX))
        .ml(rems_from_px(margin_left))
        .mr(rems_from_px(margin_right))
        .gap(rems_from_px(COUNTER_ICON_TEXT_GAP_PX))
        .child(
            Icon::new(icon)
                .size(ROW_ICON_SIZE)
                .color(Color::Custom(color)),
        )
        .child(
            Label::new(commit_count_label(count))
                .size(LabelSize::Small)
                .color(Color::Custom(color)),
        )
}

impl BranchesDelegate {
    fn render_action_row(
        &self,
        ix: usize,
        selected: bool,
        action: &TopAction,
        highlight_positions: &[usize],
        cx: &App,
    ) -> AnyElement {
        let label_color = if action.enabled {
            Color::Default
        } else {
            Color::Disabled
        };
        let icon = action.icon.map(|icon| {
            Icon::new(icon)
                .size(ROW_ICON_SIZE)
                .color(if action.enabled {
                    Color::Muted
                } else {
                    Color::Disabled
                })
        });
        let shortcut = action.shortcut_action().map(|shortcut_action| {
            KeyBinding::for_action_in(shortcut_action.as_ref(), &self.focus_handle, cx)
                .style(KeyBindingStyle::Label)
                .color(Color::Muted)
                .disabled(!action.enabled)
        });
        let disabled_reason = action.disabled_reason.filter(|_| !action.enabled);

        base_row(ix, selected)
            .start_slot(icon_slot(icon))
            .child(
                highlighted(&action.label, highlight_positions, cx)
                    .color(label_color)
                    .truncate(),
            )
            .when_some(shortcut, |item, shortcut| item.end_slot(shortcut))
            .when_some(disabled_reason, |item, reason| {
                item.tooltip(Tooltip::text(reason))
            })
            .into_any_element()
    }

    fn render_ref_row(
        &self,
        ix: usize,
        selected: bool,
        entry: &RefEntry,
        cx: &mut Context<Picker<Self>>,
    ) -> AnyElement {
        let row: &RefRow = &entry.row;
        let is_tag = row.target.kind == RefKind::Tag;
        let icon_name = ref_row_icon(row.is_current, row.is_favorite, is_tag, selected);
        let icon_color = if icon_name == IconName::FavoriteOutline {
            Color::Muted
        } else if row.is_current || row.is_favorite {
            Color::Warning
        } else {
            Color::Muted
        };

        let icon_area = h_flex()
            .id(("branches-popup-ref-icon", ix))
            .flex_none()
            .size(rems_from_px(ICON_SLOT_PX))
            .justify_center()
            .child(Icon::new(icon_name).size(ROW_ICON_SIZE).color(icon_color))
            .cursor_pointer()
            .on_click(cx.listener(move |picker, _, window, cx| {
                cx.stop_propagation();
                if picker.delegate.toggle_favorite_at(ix, cx) {
                    picker.refresh(window, cx);
                }
            }));

        let label = h_flex()
            .debug_selector(move || format!("branches-popup-ref-label-{ix}"))
            .min_w_0()
            .flex_shrink_1()
            .child(highlighted(&row.label, &row.highlight_positions, cx).truncate());

        let has_commit_counts = row.incoming > 0 || row.outgoing > 0;
        let (incoming_color, outgoing_color) = commit_counter_colors(cx);
        let incoming = (row.incoming > 0).then(|| {
            render_commit_counter(
                IconName::IncomingCommits,
                row.incoming,
                incoming_color,
                INCOMING_MARGIN_LEFT_PX,
                INCOMING_MARGIN_RIGHT_PX,
            )
        });
        let outgoing = (row.outgoing > 0).then(|| {
            render_commit_counter(
                IconName::OutgoingCommits,
                row.outgoing,
                outgoing_color,
                OUTGOING_MARGIN_LEFT_PX,
                OUTGOING_MARGIN_RIGHT_PX,
            )
        });

        let tracked = h_flex()
            .flex_1()
            .min_w_0()
            .pl(rems_from_px(SECONDARY_TEXT_MARGIN_LEFT_PX))
            .justify_end()
            .overflow_hidden()
            .children(
                row.tracked
                    .clone()
                    .map(|tracked| Label::new(tracked).color(Color::Muted).truncate()),
            );

        let arrow = self.render_ref_menu_trigger(ix, entry, cx);

        base_row(ix, selected)
            .start_slot(
                h_flex()
                    .gap_1()
                    .child(indent(row.depth))
                    .child(icon_slot(None::<Icon>))
                    .child(icon_area),
            )
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .child(label)
                    .children(incoming)
                    .children(outgoing)
                    .child(tracked),
            )
            .end_slot(arrow)
            .when(has_commit_counts, |item| {
                item.tooltip(Tooltip::text(commit_counts_tooltip(
                    row.incoming,
                    row.outgoing,
                )))
            })
            .into_any_element()
    }

    fn render_ref_menu_trigger(
        &self,
        ix: usize,
        entry: &RefEntry,
        cx: &mut Context<Picker<Self>>,
    ) -> AnyElement {
        let trigger = menu_arrow(ix);
        let Some(context) = self.branch_context() else {
            return trigger.into_any_element();
        };

        let target = entry.row.target.clone();
        let plan = self.menu_plan_for(&target, cx);
        let picker = cx.weak_entity();
        let dismiss: Rc<dyn Fn(&mut Window, &mut App)> =
            Rc::new(move |_window: &mut Window, cx: &mut App| {
                picker
                    .update(cx, |_, cx| {
                        cx.emit(DismissEvent);
                    })
                    .ok();
            });

        div()
            .debug_selector(move || format!("branches-popup-ref-arrow-{ix}"))
            .child(
                PopoverMenu::new(("branches-popup-ref-menu", ix))
                    .with_handle(entry.menu_handle.clone())
                    .trigger(trigger)
                    .menu(move |window, cx| {
                        Some(ref_menu::build_ref_menu(
                            plan.clone(),
                            MenuRunner {
                                context: context.clone(),
                                target: target.clone(),
                                dismiss: dismiss.clone(),
                                handles_delete_shortcut: true,
                            },
                            window,
                            cx,
                        ))
                    })
                    .anchor(Anchor::TopLeft)
                    .attach(Anchor::TopRight)
                    .offset(ref_menu_offset()),
            )
            .into_any_element()
    }

    fn render_fetch_button(&self, cx: &mut Context<Picker<Self>>) -> AnyElement {
        if self.fetching(cx) {
            return h_flex()
                .id("branches-popup-fetching")
                .flex_none()
                .justify_center()
                .size(rems_from_px(SEARCH_BOX_HEIGHT_PX))
                .child(
                    Icon::new(IconName::ArrowCircle)
                        .size(IconSize::Small)
                        .color(Color::Muted)
                        .with_rotate_animation(2),
                )
                .tooltip(Tooltip::text("Fetching…"))
                .into_any_element();
        }
        IconButton::new("branches-popup-fetch", IconName::Fetch)
            .icon_size(IconSize::Small)
            .tooltip(Tooltip::text("Fetch"))
            .on_click(cx.listener(|picker, _, window, cx| {
                picker.delegate.start_fetch(window, cx);
            }))
            .into_any_element()
    }

    fn render_gear_button(&self, cx: &mut Context<Picker<Self>>) -> AnyElement {
        let options = GearMenuOptions {
            toggles: self.state.toggles,
            several_repositories: self.section_repository_name.is_some(),
            user_resized: self.state.size.is_some(),
        };
        let picker = cx.weak_entity();
        PopoverMenu::new("branches-popup-gear-menu")
            .with_handle(self.gear_menu_handle.clone())
            .trigger_with_tooltip(
                IconButton::new("branches-popup-gear", IconName::Settings)
                    .icon_size(IconSize::Small),
                Tooltip::text("Settings"),
            )
            .menu(move |window, cx| Some(build_gear_menu(options, picker.clone(), window, cx)))
            .anchor(Anchor::TopRight)
            .attach(Anchor::BottomRight)
            .offset(point(px(0.), px(2.)))
            .into_any_element()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GearMenuOptions {
    pub toggles: state::PopupToggles,
    pub several_repositories: bool,
    pub user_resized: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GearMenuItem {
    Header(&'static str),
    Toggle {
        label: &'static str,
        toggle: PopupToggle,
        checked: bool,
    },
    RestorePopupSize {
        enabled: bool,
    },
    Separator,
}

/// `Git.Branches.Popup.Settings`: with several repositories the search filters get an
/// "Add in Search Results" header and a "Repositories" filter.
pub fn gear_menu_items(options: GearMenuOptions) -> Vec<GearMenuItem> {
    let toggles = options.toggles;
    let toggle = |label, toggle: PopupToggle| GearMenuItem::Toggle {
        label,
        toggle,
        checked: toggle.get(&toggles),
    };
    let mut items = Vec::new();
    if options.several_repositories {
        items.push(GearMenuItem::Header("Add in Search Results"));
        items.push(toggle("Actions", PopupToggle::ShowActionsInSearch));
        items.push(toggle(
            "Repositories",
            PopupToggle::ShowRepositoriesInSearch,
        ));
    } else {
        items.push(toggle(
            "Show Actions in Search Results",
            PopupToggle::ShowActionsInSearch,
        ));
    }
    items.push(GearMenuItem::Separator);
    items.push(toggle("Group by Directory", PopupToggle::GroupByDirectory));
    items.push(toggle("Show Recent Branches", PopupToggle::ShowRecent));
    items.push(toggle("Show Tags", PopupToggle::ShowTags));
    items.push(GearMenuItem::RestorePopupSize {
        enabled: options.user_resized,
    });
    items
}

fn update_picker_later(
    picker: &WeakEntity<Picker<BranchesDelegate>>,
    window: &mut Window,
    cx: &mut App,
    update: impl FnOnce(
        &mut Picker<BranchesDelegate>,
        &mut Window,
        &mut Context<Picker<BranchesDelegate>>,
    ) + 'static,
) {
    let picker = picker.clone();
    window
        .spawn(cx, async move |cx| {
            picker.update_in(cx, update).ok();
        })
        .detach();
}

fn build_gear_menu(
    options: GearMenuOptions,
    picker: WeakEntity<Picker<BranchesDelegate>>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ContextMenu> {
    ContextMenu::build(window, cx, move |menu, _, _| {
        gear_menu_items(options)
            .into_iter()
            .fold(menu, |menu, item| match item {
                GearMenuItem::Header(title) => menu.header(title),
                GearMenuItem::Separator => menu.separator(),
                GearMenuItem::Toggle {
                    label,
                    toggle,
                    checked,
                } => {
                    let picker = picker.clone();
                    menu.toggleable_entry(
                        label,
                        checked,
                        IconPosition::Start,
                        None,
                        move |window, cx| {
                            update_picker_later(&picker, window, cx, move |picker, window, cx| {
                                picker.delegate.toggle_setting(toggle, cx);
                                picker.refresh_placeholder(window, cx);
                                picker.refresh(window, cx);
                            });
                        },
                    )
                }
                GearMenuItem::RestorePopupSize { enabled } => {
                    let picker = picker.clone();
                    menu.item(
                        ui::ContextMenuEntry::new("Restore Popup Size")
                            .disabled(!enabled)
                            .handler(move |window, cx| {
                                update_picker_later(&picker, window, cx, |picker, window, cx| {
                                    picker.delegate.restore_popup_size(cx);
                                    picker.refresh(window, cx);
                                });
                            }),
                    )
                }
            })
    })
}

impl PickerDelegate for BranchesDelegate {
    type ListItem = AnyElement;

    fn name() -> &'static str {
        "branches_popup"
    }

    fn match_count(&self) -> usize {
        self.rows.len()
    }

    fn selected_index(&self) -> usize {
        self.selected_index
    }

    fn set_selected_index(
        &mut self,
        ix: usize,
        _window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) {
        self.selected_index = ix.min(self.rows.len().saturating_sub(1));
        self.preferred_selection_pending = false;
        cx.notify();
    }

    fn can_select(&self, ix: usize, _window: &mut Window, _cx: &mut Context<Picker<Self>>) -> bool {
        self.rows.get(ix).is_some_and(PopupRow::is_selectable)
    }

    fn placeholder_text(&self, _window: &mut Window, _cx: &mut App) -> Arc<str> {
        search_placeholder(self.state.toggles.show_actions_in_search).into()
    }

    fn no_matches_text(&self, _window: &mut Window, _cx: &mut App) -> Option<SharedString> {
        let text = if self.repository.is_none() {
            NO_REPOSITORY
        } else if self.state.toggles.show_actions_in_search {
            NOTHING_FOUND
        } else {
            BRANCH_NOT_FOUND
        };
        Some(SharedString::new_static(text))
    }

    fn update_matches(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Task<()> {
        let query_changed = self.query != query;
        let previous_key = self.rows.get(self.selected_index).and_then(PopupRow::key);
        self.query = query;
        self.rebuild_rows(query_changed, cx);
        self.selected_index = self.choose_selection(previous_key, query_changed);
        self.pack_if_requested(window, cx);
        Task::ready(())
    }

    fn confirm(&mut self, _secondary: bool, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        let Some(row) = self.rows.get(self.selected_index).cloned() else {
            return;
        };
        match row {
            PopupRow::Separator => {}
            PopupRow::Action { action, .. } => {
                if action.enabled {
                    self.run_top_action(action.kind, window, cx);
                }
            }
            PopupRow::Repository(repository_row) => {
                self.switch_repository(repository_row.choice.id, window, cx);
            }
            PopupRow::Section(_) | PopupRow::Folder(_) => {
                if let Some(node_id) = row.node_id() {
                    self.toggle_node(node_id, cx);
                    self.refresh_later(window, cx);
                }
            }
            PopupRow::Ref(entry) => entry.menu_handle.show(window, cx),
        }
    }

    fn dismissed(&mut self, _window: &mut Window, _cx: &mut Context<Picker<Self>>) {}

    fn select_child(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Option<String> {
        let ix = self.selected_index;
        let row = self.rows.get(ix).cloned()?;
        match &row {
            PopupRow::Section(_) | PopupRow::Folder(_) => {
                if row.is_expanded() == Some(true) {
                    if let Some(child) = first_child_index(&self.rows, ix) {
                        self.select_row_later(child, window, cx);
                    }
                } else if let Some(node_id) = row.node_id() {
                    self.toggle_node(node_id, cx);
                    self.refresh_later(window, cx);
                }
            }
            PopupRow::Ref(entry) => entry.menu_handle.show(window, cx),
            PopupRow::Separator | PopupRow::Action { .. } | PopupRow::Repository(_) => {}
        }
        None
    }

    fn select_parent(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Option<String> {
        let ix = self.selected_index;
        let row = self.rows.get(ix).cloned()?;
        let collapsible = row.is_tree_container() && row.is_expanded() == Some(true);
        if collapsible {
            if let Some(node_id) = row.node_id() {
                self.toggle_node(node_id, cx);
                self.refresh_later(window, cx);
            }
        } else if let Some(parent) = parent_index(&self.rows, ix) {
            self.select_row_later(parent, window, cx);
        }
        None
    }

    fn has_another_open_menu(&self, _window: &Window, _cx: &App) -> bool {
        self.gear_menu_handle.is_deployed()
            || self
                .rows
                .iter()
                .any(|row| matches!(row, PopupRow::Ref(entry) if entry.menu_handle.is_deployed()))
    }

    fn render_editor(
        &self,
        editor: &Arc<dyn ErasedEditor>,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Option<Div> {
        let border_color = cx.theme().colors().border_variant;
        let text_color = self.search_text_color(cx);
        Some(
            v_flex().w_full().child(
                h_flex()
                    .pt(rems_from_px(HEADER_TOP_INSET_PX))
                    .pl(rems_from_px(HEADER_LEFT_INSET_PX))
                    .pr(rems_from_px(HEADER_RIGHT_INSET_PX))
                    .flex_none()
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .h(rems_from_px(SEARCH_BOX_HEIGHT_PX))
                            .px_2()
                            .gap_1p5()
                            .rounded_md()
                            .border_1()
                            .border_color(border_color)
                            .child(
                                Icon::new(IconName::MagnifyingGlass)
                                    .size(IconSize::Small)
                                    .color(Color::Muted),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(editor.render_with_text_color(text_color, window, cx)),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .ml(rems_from_px(HEADER_TOOLBAR_GAP_PX))
                            .child(self.render_fetch_button(cx))
                            .child(self.render_gear_button(cx)),
                    ),
            ),
        )
    }

    fn render_match(
        &self,
        ix: usize,
        selected: bool,
        _window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Option<Self::ListItem> {
        let row = self.rows.get(ix)?;
        Some(match row {
            PopupRow::Separator => render_separator(),
            PopupRow::Action {
                action,
                highlight_positions,
                ..
            } => self.render_action_row(ix, selected, action, highlight_positions, cx),
            PopupRow::Repository(repository_row) => {
                render_repository_row(ix, selected, repository_row, cx)
            }
            PopupRow::Section(section) => render_section_row(
                ix,
                selected,
                section,
                self.section_repository_name.as_deref(),
            ),
            PopupRow::Folder(folder) => render_folder_row(ix, selected, folder),
            PopupRow::Ref(entry) => self.render_ref_row(ix, selected, entry, cx),
        })
    }
}
