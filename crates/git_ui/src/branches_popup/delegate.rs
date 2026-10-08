use std::rc::Rc;
use std::sync::Arc;

use collections::{HashMap, HashSet};
use futures::channel::oneshot;
use git::repository::{Branch, FetchOptions, RepositoryOperation, Tag};
use gpui::{
    Anchor, AnyElement, App, Context, DismissEvent, Entity, FocusHandle, SharedString,
    Subscription, Task, WeakEntity, Window, point,
};
use picker::{Picker, PickerDelegate};
use project::git_store::{GitStore, Repository, RepositoryEvent, RepositoryId, RepositorySnapshot};
use ui::{
    CommonAnimationExt as _, ContextMenu, Divider, HighlightedLabel, KeyBinding, KeyBindingStyle,
    ListItem, ListItemSpacing, PopoverMenu, PopoverMenuHandle, Tooltip, prelude::*,
};
use ui_input::ErasedEditor;
use util::ResultExt as _;
use workspace::Workspace;

use super::ref_menu::{self, MenuContext, MenuRunner};
use super::rows::{
    PopupRow, RefEntry, RepositoryChoice, RepositoryRow, RowKey, RowsInput, best_match,
    build_popup_rows, first_child_index, first_selectable, is_filtering, parent_index,
    position_of_current_branch, position_of_key,
};
use super::state::{self, PopupState};
use super::top_actions::{self, TopAction, TopActionKind, TopActionsInput};
use super::tree::{self, FolderRow, RefRow, SectionKind, SectionRow, TreeInput};
use crate::branch_operations::{BranchContext, integrate};
use crate::branch_refs::{RefKind, RefTarget};

const MAX_RECENT_BRANCHES: usize = 5;
const SHORT_SHA_LENGTH: usize = 8;
const MAX_COMMIT_COUNT_LABEL: u32 = 99;
const ROW_HEIGHT_PX: f32 = 24.;
const HEADER_HEIGHT_PX: f32 = 40.;
const SEARCH_BOX_HEIGHT_PX: f32 = 28.;
const ICON_SLOT_PX: f32 = 16.;
const INDENT_STEP_PX: f32 = 16.;
const SEPARATOR_HEIGHT_PX: f32 = 9.;
const ROW_GROUP: &str = "list_item";
const SEARCH_PLACEHOLDER_WITH_ACTIONS: &str = "Search for branches and actions";
const SEARCH_PLACEHOLDER_WITHOUT_ACTIONS: &str = "Search for branches";
const NOTHING_FOUND: &str = "Nothing found";
const BRANCH_NOT_FOUND: &str = "Branch not found";
const NO_REPOSITORY: &str = "No Git repository";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupToggle {
    ShowActionsInSearch,
    GroupByDirectory,
    ShowRecent,
    ShowTags,
}

impl PopupToggle {
    fn get(self, toggles: &state::PopupToggles) -> bool {
        match self {
            PopupToggle::ShowActionsInSearch => toggles.show_actions_in_search,
            PopupToggle::GroupByDirectory => toggles.group_by_directory,
            PopupToggle::ShowRecent => toggles.show_recent,
            PopupToggle::ShowTags => toggles.show_tags,
        }
    }

    fn set(self, toggles: &mut state::PopupToggles, value: bool) {
        match self {
            PopupToggle::ShowActionsInSearch => toggles.show_actions_in_search = value,
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
        }
    }
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

pub fn ref_icon(is_current: bool, is_favorite: bool, is_tag: bool) -> IconName {
    match (is_current, is_favorite) {
        (true, true) => IconName::CurrentBranchFavoriteLabel,
        (true, false) => IconName::CurrentBranchLabel,
        (false, true) => IconName::StarFilled,
        (false, false) if is_tag => IconName::TagLabel,
        (false, false) => IconName::BranchNode,
    }
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
    pub(super) fetching: bool,
    update_project_shows_options: bool,
    menu_handles: HashMap<RefTarget, PopoverMenuHandle<ContextMenu>>,
    edits_before_load: Vec<StateEdit>,
    pub(super) gear_menu_handle: PopoverMenuHandle<ContextMenu>,
    _full_load_task: Task<()>,
    _operation_load_task: Task<()>,
    _state_load_task: Task<()>,
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
            fetching: false,
            update_project_shows_options: true,
            menu_handles: HashMap::default(),
            edits_before_load: Vec::new(),
            gear_menu_handle: PopoverMenuHandle::default(),
            _full_load_task: Task::ready(()),
            _operation_load_task: Task::ready(()),
            _state_load_task: Task::ready(()),
            pending_save: Task::ready(()),
        }
    }

    #[cfg(test)]
    pub fn rows(&self) -> &[PopupRow] {
        &self.rows
    }

    #[cfg(test)]
    pub fn selected_row(&self) -> Option<&PopupRow> {
        self.rows.get(self.selected_index)
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
        self.reveal_pending = true;
        self.state_loaded = false;
        self.data_loaded = false;
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
        let Some(current) = self.data.current_branch.clone() else {
            return;
        };
        let toggles = self.state.toggles;
        let in_recent = toggles.show_recent
            && self
                .data
                .recent
                .iter()
                .take(MAX_RECENT_BRANCHES)
                .any(|name| *name == current);
        let section = if in_recent {
            SectionKind::Recent
        } else {
            SectionKind::Local
        };
        self.revealed.extend(tree::ancestor_node_ids(
            section,
            &current,
            toggles.group_by_directory,
        ));
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
                }
            })
            .collect();
        choices.sort_by(|left, right| {
            (&*left.name)
                .cmp(&*right.name)
                .then_with(|| left.id.cmp(&right.id))
        });
        choices
    }

    fn find_repository(&self, id: RepositoryId, cx: &App) -> Option<Entity<Repository>> {
        self.git_store(cx)?
            .read(cx)
            .repositories()
            .get(&id)
            .cloned()
    }

    fn rebuild_rows(&mut self, cx: &App) {
        if self.repository.is_none() {
            self.rows.clear();
            return;
        }
        self.reveal_current_branch_if_ready();
        let defaults = tree::default_favorites(&self.data.local, &self.data.remote);
        let favorites = self.state.resolve_favorites(&defaults);
        let mut expanded = self.state.expanded().clone();
        expanded.extend(self.revealed.iter().cloned());
        let tree_rows = tree::build_rows(&TreeInput {
            local: &self.data.local,
            remote: &self.data.remote,
            tags: &self.data.tags,
            recent: &self.data.recent,
            current_branch: self.data.current_branch.as_deref(),
            favorites: &favorites,
            options: self.state.toggles.tree_options(),
            expanded: &expanded,
            query: &self.query,
        });
        let top_actions = top_actions::top_actions_plan(&TopActionsInput {
            has_commits: self.data.has_commits,
            operation: self.data.operation,
            update_project_shows_options: self.update_project_shows_options,
        });
        let repositories = self.repository_choices(cx);
        self.rows = build_popup_rows(RowsInput {
            query: &self.query,
            show_actions_in_search: self.state.toggles.show_actions_in_search,
            top_actions: &top_actions,
            repositories: &repositories,
            tree_rows,
            menu_handles: &mut self.menu_handles,
        });
    }

    fn choose_selection(&self, previous_key: Option<RowKey>, query_changed: bool) -> usize {
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
        let current = position_of_current_branch(&self.rows);
        if self.preferred_selection_pending || query_changed {
            return current.or(previous).unwrap_or(first);
        }
        previous.or(current).unwrap_or(first)
    }

    fn is_node_expanded(&self, node_id: &SharedString) -> bool {
        self.state.is_expanded(node_id) || self.revealed.contains(node_id)
    }

    fn toggle_node(&mut self, node_id: SharedString, cx: &App) {
        if is_filtering(&self.query) {
            return;
        }
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
        if target.kind == RefKind::Tag {
            return false;
        }
        let favorite = !self.state.is_favorite(&target, &self.favorite_defaults());
        self.record_edit(StateEdit::Favorite(target, favorite), cx);
        true
    }

    pub fn toggle_favorite_of_selected(&mut self, cx: &App) -> bool {
        self.toggle_favorite_at(self.selected_index, cx)
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

    fn start_fetch(&mut self, window: &mut Window, cx: &mut Context<Picker<Self>>) {
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
        cx.spawn(async move |picker, cx| {
            let result = receiver.await;
            picker
                .update(cx, |picker, cx| {
                    picker.delegate.fetching = false;
                    cx.notify();
                })
                .ok();
            if let Ok(Err(error)) = result {
                cx.update(|cx| context.notify_error("Fetch", error, cx));
            }
        })
        .detach();
    }

    fn run_top_action(
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

    fn menu_plan_for(&self, target: &RefTarget) -> Vec<ref_menu::MenuItemPlan> {
        let has_tracked_branch = match target.kind {
            RefKind::Local => self
                .data
                .local
                .iter()
                .find(|branch| branch.name() == &*target.name)
                .is_some_and(|branch| branch.upstream.is_some()),
            RefKind::Remote | RefKind::Tag => false,
        };

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
            fetching: self.fetching,
        })
    }
}

impl Drop for BranchesDelegate {
    fn drop(&mut self) {
        self.flush_pending_save();
    }
}

fn chevron(expanded: bool) -> Icon {
    Icon::new(if expanded {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    })
    .size(IconSize::Small)
    .color(Color::Muted)
}

fn icon_slot(icon: Option<Icon>) -> AnyElement {
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

fn render_separator() -> AnyElement {
    h_flex()
        .w_full()
        .h(rems_from_px(SEPARATOR_HEIGHT_PX))
        .px_2()
        .child(div().w_full().child(Divider::horizontal()))
        .into_any_element()
}

fn render_section_row(ix: usize, selected: bool, section: &SectionRow) -> AnyElement {
    base_row(ix, selected)
        .start_slot(icon_slot(Some(chevron(section.expanded))))
        .child(Label::new(section.kind.label()))
        .into_any_element()
}

fn render_folder_row(ix: usize, selected: bool, folder: &FolderRow) -> AnyElement {
    let folder_icon = if folder.expanded {
        IconName::FolderOpen
    } else {
        IconName::Folder
    };
    base_row(ix, selected)
        .start_slot(
            h_flex()
                .gap_1()
                .child(indent(folder.depth))
                .child(icon_slot(Some(chevron(folder.expanded))))
                .child(icon_slot(Some(
                    Icon::new(folder_icon)
                        .size(IconSize::Small)
                        .color(Color::Muted),
                ))),
        )
        .child(Label::new(folder.label.clone()))
        .into_any_element()
}

fn render_repository_row(ix: usize, selected: bool, row: &RepositoryRow) -> AnyElement {
    let choice = &row.choice;
    let name_color = if choice.is_active {
        Color::Accent
    } else {
        Color::Default
    };
    base_row(ix, selected)
        .start_slot(icon_slot(Some(
            Icon::new(IconName::Folder)
                .size(IconSize::Small)
                .color(Color::Muted),
        )))
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .child(
                    HighlightedLabel::new(
                        choice.name.clone(),
                        char_positions_to_byte_offsets(&choice.name, &row.highlight_positions),
                    )
                    .color(name_color),
                )
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
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
        .into_any_element()
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
                .size(IconSize::Small)
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
                HighlightedLabel::new(
                    action.label.clone(),
                    char_positions_to_byte_offsets(&action.label, highlight_positions),
                )
                .color(label_color),
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
        let favorite_toggle_enabled = !is_tag;
        let offers_favorite_on_hover = favorite_toggle_enabled && !row.is_favorite;
        let icon_name = ref_icon(row.is_current, row.is_favorite, is_tag);
        let icon_color = if row.is_current || row.is_favorite {
            Color::Warning
        } else {
            Color::Muted
        };
        let base_icon = Icon::new(icon_name).size(IconSize::Small).color(icon_color);

        let icon_area = div()
            .id(("branches-popup-ref-icon", ix))
            .relative()
            .flex_none()
            .size(rems_from_px(ICON_SLOT_PX))
            .child(
                h_flex()
                    .absolute()
                    .inset_0()
                    .justify_center()
                    .when(offers_favorite_on_hover, |this| {
                        this.group_hover(ROW_GROUP, |style| style.invisible())
                    })
                    .child(base_icon),
            )
            .when(offers_favorite_on_hover, |this| {
                this.child(
                    h_flex()
                        .absolute()
                        .inset_0()
                        .justify_center()
                        .visible_on_hover(ROW_GROUP)
                        .child(
                            Icon::new(IconName::FavoriteOutline)
                                .size(IconSize::Small)
                                .color(Color::Muted),
                        ),
                )
            })
            .when(favorite_toggle_enabled, |this| {
                this.cursor_pointer()
                    .on_click(cx.listener(move |picker, _, window, cx| {
                        cx.stop_propagation();
                        if picker.delegate.toggle_favorite_at(ix, cx) {
                            picker.refresh(window, cx);
                        }
                    }))
            });

        let label = HighlightedLabel::new(
            row.label.clone(),
            char_positions_to_byte_offsets(&row.label, &row.highlight_positions),
        )
        .flex_none();

        let has_commit_counts = row.incoming > 0 || row.outgoing > 0;
        let counters_tooltip = format!(
            "{} incoming and {} outgoing commits",
            row.incoming, row.outgoing
        );
        let counters = h_flex()
            .id(("branches-popup-ref-counters", ix))
            .flex_none()
            .gap_1()
            .when(row.incoming > 0, |this| {
                this.child(
                    h_flex()
                        .gap_0p5()
                        .child(
                            Icon::new(IconName::IncomingCommits)
                                .size(IconSize::XSmall)
                                .color(Color::Info),
                        )
                        .child(
                            Label::new(commit_count_label(row.incoming))
                                .size(LabelSize::XSmall)
                                .color(Color::Info),
                        ),
                )
            })
            .when(row.outgoing > 0, |this| {
                this.child(
                    h_flex()
                        .gap_0p5()
                        .child(
                            Icon::new(IconName::OutgoingCommits)
                                .size(IconSize::XSmall)
                                .color(Color::Success),
                        )
                        .child(
                            Label::new(commit_count_label(row.outgoing))
                                .size(LabelSize::XSmall)
                                .color(Color::Success),
                        ),
                )
            })
            .when(has_commit_counts, |this| {
                this.tooltip(Tooltip::text(counters_tooltip))
            });

        let tracked = h_flex()
            .flex_1()
            .min_w_0()
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
                    .child(icon_slot(None))
                    .child(icon_area),
            )
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .child(label)
                    .child(counters)
                    .child(tracked),
            )
            .end_slot(arrow)
            .into_any_element()
    }

    fn render_ref_menu_trigger(
        &self,
        ix: usize,
        entry: &RefEntry,
        cx: &mut Context<Picker<Self>>,
    ) -> AnyElement {
        let trigger = IconButton::new(("branches-popup-ref-arrow", ix), IconName::MenuArrow)
            .icon_size(IconSize::XSmall)
            .icon_color(Color::Muted)
            .style(ButtonStyle::Transparent);
        let Some(context) = self.branch_context() else {
            return trigger.into_any_element();
        };

        let target = entry.row.target.clone();
        let plan = self.menu_plan_for(&target);
        let picker = cx.weak_entity();
        let dismiss: Rc<dyn Fn(&mut Window, &mut App)> =
            Rc::new(move |_window: &mut Window, cx: &mut App| {
                picker
                    .update(cx, |_, cx| {
                        cx.emit(DismissEvent);
                    })
                    .ok();
            });

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
                    },
                    window,
                    cx,
                ))
            })
            .anchor(Anchor::TopLeft)
            .attach(Anchor::TopRight)
            .offset(point(px(2.), px(-4.)))
            .into_any_element()
    }

    fn render_fetch_button(&self, cx: &mut Context<Picker<Self>>) -> AnyElement {
        if self.fetching {
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
        let toggles = self.state.toggles;
        let picker = cx.weak_entity();
        PopoverMenu::new("branches-popup-gear-menu")
            .with_handle(self.gear_menu_handle.clone())
            .trigger_with_tooltip(
                IconButton::new("branches-popup-gear", IconName::Settings)
                    .icon_size(IconSize::Small),
                Tooltip::text("Settings"),
            )
            .menu(move |window, cx| Some(build_gear_menu(toggles, picker.clone(), window, cx)))
            .anchor(Anchor::TopRight)
            .attach(Anchor::BottomRight)
            .offset(point(px(0.), px(2.)))
            .into_any_element()
    }
}

fn add_toggle(
    menu: ContextMenu,
    label: &'static str,
    toggled: bool,
    toggle: PopupToggle,
    picker: &WeakEntity<Picker<BranchesDelegate>>,
) -> ContextMenu {
    let picker = picker.clone();
    menu.toggleable_entry(
        label,
        toggled,
        IconPosition::Start,
        None,
        move |window, cx| {
            let picker = picker.clone();
            window
                .spawn(cx, async move |cx| {
                    picker
                        .update_in(cx, |picker, window, cx| {
                            picker.delegate.toggle_setting(toggle, cx);
                            picker.refresh_placeholder(window, cx);
                            picker.refresh(window, cx);
                        })
                        .ok();
                })
                .detach();
        },
    )
}

fn build_gear_menu(
    toggles: state::PopupToggles,
    picker: WeakEntity<Picker<BranchesDelegate>>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ContextMenu> {
    ContextMenu::build(window, cx, move |menu, _, _| {
        let menu = add_toggle(
            menu,
            "Show Actions in Search Results",
            toggles.show_actions_in_search,
            PopupToggle::ShowActionsInSearch,
            &picker,
        )
        .separator();
        let menu = add_toggle(
            menu,
            "Group by Directory",
            toggles.group_by_directory,
            PopupToggle::GroupByDirectory,
            &picker,
        );
        let menu = add_toggle(
            menu,
            "Show Recent Branches",
            toggles.show_recent,
            PopupToggle::ShowRecent,
            &picker,
        );
        add_toggle(
            menu,
            "Show Tags",
            toggles.show_tags,
            PopupToggle::ShowTags,
            &picker,
        )
        .opaque_background()
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
        _window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Task<()> {
        let query_changed = self.query != query;
        let previous_key = self.rows.get(self.selected_index).and_then(PopupRow::key);
        self.query = query;
        self.rebuild_rows(cx);
        self.selected_index = self.choose_selection(previous_key, query_changed);
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
        let collapsible = row.is_tree_container()
            && row.is_expanded() == Some(true)
            && !is_filtering(&self.query);
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
        Some(
            v_flex().w_full().child(
                h_flex()
                    .h(rems_from_px(HEADER_HEIGHT_PX))
                    .px_2()
                    .gap_1()
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
                            .child(div().flex_1().min_w_0().child(editor.render(window, cx))),
                    )
                    .child(self.render_fetch_button(cx))
                    .child(self.render_gear_button(cx)),
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
                render_repository_row(ix, selected, repository_row)
            }
            PopupRow::Section(section) => render_section_row(ix, selected, section),
            PopupRow::Folder(folder) => render_folder_row(ix, selected, folder),
            PopupRow::Ref(entry) => self.render_ref_row(ix, selected, entry, cx),
        })
    }
}
