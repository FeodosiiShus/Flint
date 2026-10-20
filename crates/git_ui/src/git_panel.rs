use git_ui_core::askpass_modal::AskPassModal;
pub(crate) use git_ui_core::notifications::{open_output, show_error_toast};

use crate::branch_operations::push_result::{
    PushTarget, push_failed_notice, push_not_started_notice, push_succeeded_notice,
    upstream_ahead_count,
};
use crate::branch_operations::smart_operation::stash_pop_left_conflicts;
use crate::branch_operations::{BranchContext, OperationProgress, PUSH_PROGRESS_TITLE};
use crate::commit_modal::CommitModal;
use crate::git_panel_settings::GitPanelSettings;
use crate::git_tool_window::GitToolWindow;
use crate::merge_tool::{self, conflict_resolution::ConflictContext};
use crate::remote_output::{self, RemoteAction, SuccessMessage};
use crate::{picker_prompt, render_remote_button};
use anyhow::Context as _;
use askpass::AskPassDelegate;
use collections::{BTreeMap, HashSet};
use db::kvp::KeyValueStore;
use editor::{Editor, EditorMode, EditorStyle, MultiBuffer, MultiBufferOffset, RewrapOptions};
use git::repository::{
    CommitDetails, CommitOptions, FetchOptions, GitCommitTemplate, PushOptions, Remote,
    RemoteCommandOutput, ResetMode, Upstream, UpstreamTracking,
};
use git::status::StageStatus;
use git::{
    ExpandCommitEditor, GitHostingProviderRegistry, Signoff, SkipHooks, StageAll, StashAll,
    StashApply, StashPop, StashStaged, StashTracked, UnstageAll, repository::RepoPath,
    status::FileStatus,
};
use gpui::{
    AbsoluteLength, Action, AnyElement, AnyView, AsyncWindowContext, DismissEvent, Entity,
    EventEmitter, FocusHandle, Focusable, MouseButton, Pixels, PromptLevel, Subscription, Task,
    TaskExt, TextStyle, WeakEntity, actions,
};
use itertools::Itertools;
use language::{Buffer, BufferEvent, File};
use menu;
use multi_buffer::ExcerptBoundaryInfo;
use notifications::status_toast::StatusToast;
use project::{
    Fs, Project,
    git_store::{GitStoreEvent, Repository, RepositoryEvent, pending_op},
};
use serde::{Deserialize, Serialize};
use settings::{Settings, update_settings_file};
use std::future::Future;
use std::path::Path;
use std::{sync::Arc, time::Duration};
use strum::{IntoEnumIterator, VariantNames};
use theme_settings::ThemeSettings;
use ui::{Checkbox, ContextMenu, ElevationIndex, PopoverMenuHandle, prelude::*};
use util::{ResultExt, TryFutureExt, maybe};
use workspace::SERIALIZATION_THROTTLE_TIME;
use workspace::{
    ModalView, Workspace,
    dock::{DockPosition, Panel, PanelEvent},
    notifications::NotificationId,
};
use zed_actions::{
    DecreaseBufferFontSize, IncreaseBufferFontSize, ResetBufferFontSize, git_panel::ToggleFocus,
};

const GIT_PANEL_KEY: &str = "GitPanel";
const UPDATE_DEBOUNCE: Duration = Duration::from_millis(50);

actions!(
    git_panel,
    [
        /// Closes the git panel.
        Close,
        /// Toggles the git panel.
        Toggle,
    ]
);

actions!(
    dev,
    [
        /// Shows the current git job queue debug state for the active repository.
        ShowGitJobQueue,
    ]
);

fn prompt<T>(
    msg: &str,
    detail: Option<&str>,
    window: &mut Window,
    cx: &mut App,
) -> Task<anyhow::Result<T>>
where
    T: IntoEnumIterator + VariantNames + 'static,
{
    let rx = window.prompt(PromptLevel::Info, msg, detail, T::VARIANTS, cx);
    cx.spawn(async move |_| Ok(T::iter().nth(rx.await?).unwrap()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StashKind {
    All,
    Tracked,
    Staged,
}

impl StashKind {
    fn title(self) -> &'static str {
        match self {
            StashKind::All => "Stash All",
            StashKind::Tracked => "Stash Tracked",
            StashKind::Staged => "Stash Staged",
        }
    }

    fn error_action(self) -> &'static str {
        match self {
            StashKind::All => "stash",
            StashKind::Tracked => "stash tracked",
            StashKind::Staged => "stash staged",
        }
    }
}

/// Prompts for an optional stash name. Confirming with an empty editor stashes
/// without `--message`, letting git generate its usual "WIP on ..." description.
struct StashMessageModal {
    editor: Entity<Editor>,
    panel: WeakEntity<GitPanel>,
    kind: StashKind,
}

impl StashMessageModal {
    fn new(
        panel: WeakEntity<GitPanel>,
        kind: StashKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Optionally provide a stash message", window, cx);
            editor
        });
        Self {
            editor,
            panel,
            kind,
        }
    }

    fn cancel(&mut self, _: &menu::Cancel, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn confirm(&mut self, _: &menu::Confirm, _window: &mut Window, cx: &mut Context<Self>) {
        let message = self.editor.read(cx).text(cx).trim().to_owned();
        let message = (!message.is_empty()).then_some(message);
        let kind = self.kind;
        self.panel
            .update(cx, |panel, cx| panel.perform_stash(kind, message, cx))
            .ok();
        cx.emit(DismissEvent);
    }
}

impl EventEmitter<DismissEvent> for StashMessageModal {}
impl ModalView for StashMessageModal {}
impl Focusable for StashMessageModal {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for StashMessageModal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .key_context("StashMessageModal")
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .elevation_2(cx)
            .w(rems(34.))
            .child(
                h_flex()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .w_full()
                    .gap_1p5()
                    .child(Icon::new(IconName::GitBranch).size(IconSize::XSmall))
                    .child(Headline::new(self.kind.title()).size(HeadlineSize::XSmall)),
            )
            .child(div().px_3().pb_3().w_full().child(self.editor.clone()))
    }
}

// We only allow a single remote operation at a time to avoid concurrent
// credential prompts and competing ref/working-tree updates.
#[derive(Clone, Copy)]
pub(crate) enum RemoteOperationKind {
    Fetch,
    Pull,
    Push,
}

/// Shows the Git tool window with a new log tab limited to `revisions`, the way
/// IntelliJ's "View Commits" opens the updated commits in the log.
pub(crate) fn show_log_for_revisions(
    workspace: &WeakEntity<Workspace>,
    revisions: Vec<SharedString>,
    window: &mut Window,
    cx: &mut App,
) {
    workspace
        .update(cx, |workspace, cx| {
            let Some(panel) = workspace.focus_panel::<GitPanel>(window, cx) else {
                return;
            };
            let tool_window = panel.read(cx).tool_window.clone();
            tool_window.update(cx, |tool_window, cx| {
                tool_window.open_log_tab_with_revisions(revisions, window, cx)
            });
        })
        .log_err();
}

pub fn register(workspace: &mut Workspace) {
    workspace.register_action(|workspace, _: &ToggleFocus, window, cx| {
        workspace.toggle_panel_focus::<GitPanel>(window, cx);
    });
    workspace.register_action(|workspace, _: &Toggle, window, cx| {
        if !workspace.toggle_panel_focus::<GitPanel>(window, cx) {
            workspace.close_panel::<GitPanel>(window, cx);
        }
    });
    workspace.register_action(|workspace, _: &ExpandCommitEditor, window, cx| {
        CommitModal::toggle(workspace, None, window, cx)
    });
    workspace.register_action(|workspace, _: &git::Init, window, cx| {
        if let Some(panel) = workspace.panel::<GitPanel>(cx) {
            panel.update(cx, |panel, cx| panel.git_init(window, cx));
        }
    });
    workspace.register_action(|workspace, _: &ShowGitJobQueue, window, cx| {
        if let Some(panel) = workspace.panel::<GitPanel>(cx) {
            panel.update(cx, |panel, cx| {
                panel.show_git_job_queue(window, cx);
            });
        }
    });
}

#[derive(Debug, Clone)]
pub enum Event {
    Focus,
}

#[derive(Default, Serialize, Deserialize)]
struct SerializedGitPanel {
    #[serde(default)]
    signoff_enabled: bool,
    #[serde(default)]
    commit_messages: BTreeMap<String, SerializedCommitMessage>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct SerializedCommitMessage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    original_message: Option<String>,
    #[serde(default)]
    amend_pending: bool,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct GitStatusEntry {
    pub(crate) repo_path: RepoPath,
    pub(crate) status: FileStatus,
    pub(crate) staging: StageStatus,
}

pub struct GitPanel {
    pub(crate) active_repository: Option<Entity<Repository>>,
    pub(crate) commit_editor: Entity<Editor>,
    status_entries: Vec<GitStatusEntry>,
    single_staged_entry: Option<GitStatusEntry>,
    single_tracked_entry: Option<GitStatusEntry>,
    conflicted_staged_count: usize,
    new_staged_count: usize,
    tracked_count: usize,
    tracked_staged_count: usize,
    changes_count: usize,
    focus_handle: FocusHandle,
    fs: Arc<dyn Fs>,
    pending_commit: Option<Task<()>>,
    pending_remote_operation: Option<RemoteOperationKind>,
    amend_pending: bool,
    original_commit_message: Option<String>,
    pending_commit_message_restores: BTreeMap<String, SerializedCommitMessage>,
    signoff_enabled: bool,
    skip_hooks_enabled: bool,
    pending_serialization: Task<()>,
    pub(crate) project: Entity<Project>,
    update_status_entries_task: Task<()>,
    reopen_commit_buffer_task: Task<()>,
    pub(crate) workspace: WeakEntity<Workspace>,
    commit_template: Option<GitCommitTemplate>,
    tool_window: Entity<GitToolWindow>,
    _commit_message_buffer_subscription: Option<Subscription>,
    remote_action_menu_handle: PopoverMenuHandle<ContextMenu>,
}

const COMMIT_EDITOR_LINES: usize = 18;

pub(crate) fn commit_message_editor(
    commit_message_buffer: Entity<Buffer>,
    project: Entity<Project>,
    window: &mut Window,
    cx: &mut Context<Editor>,
) -> Editor {
    let buffer = cx.new(|cx| MultiBuffer::singleton(commit_message_buffer, cx));
    let mut commit_editor = Editor::new(
        EditorMode::AutoHeight {
            min_lines: COMMIT_EDITOR_LINES,
            max_lines: Some(COMMIT_EDITOR_LINES),
        },
        buffer,
        None,
        window,
        cx,
    );
    commit_editor.set_collaboration_hub(Box::new(project));
    commit_editor.set_use_autoclose(false);
    commit_editor.set_show_gutter(false, cx);
    commit_editor.set_use_modal_editing(true);
    commit_editor.set_show_wrap_guides(false, cx);
    commit_editor.set_show_indent_guides(false, cx);
    commit_editor.set_placeholder_text("Enter commit message", window, cx);
    commit_editor.set_custom_context_menu(|editor, _point, window, cx| {
        let has_selection = editor.has_non_empty_selection(&editor.display_snapshot(cx));
        let focus_handle = editor.focus_handle(cx);

        Some(ContextMenu::build(window, cx, |menu, _, _| {
            menu.context(focus_handle)
                .action_disabled_when(!has_selection, "Cut", Box::new(editor::actions::Cut))
                .action_disabled_when(!has_selection, "Copy", Box::new(editor::actions::Copy))
                .action("Paste", Box::new(editor::actions::Paste))
        }))
    });
    commit_editor
}

impl GitPanel {
    // Only the test-support constructors call this thin wrapper now; production
    // registration goes through `new_with_serialized_panel` directly. Gate it to
    // the same cfg as `new_test` so the non-test lib build doesn't see it as dead.
    #[cfg(any(test, feature = "test-support"))]
    fn new(
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Entity<Self> {
        Self::new_with_serialized_panel(workspace, None, window, cx)
    }

    fn new_with_serialized_panel(
        workspace: &mut Workspace,
        serialized_panel: Option<SerializedGitPanel>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Entity<Self> {
        let project = workspace.project().clone();
        let app_state = workspace.app_state().clone();
        let fs = app_state.fs.clone();
        let workspace_handle = workspace.weak_handle();
        let git_store = project.read(cx).git_store().clone();
        let active_repository = project.read(cx).active_repository(cx);
        let signoff_enabled = serialized_panel
            .as_ref()
            .is_some_and(|panel| panel.signoff_enabled);
        let active_work_directory_abs_path = active_repository.as_ref().map(|repository| {
            repository
                .read(cx)
                .work_directory_abs_path
                .to_string_lossy()
                .into_owned()
        });
        let active_draft = serialized_panel.as_ref().and_then(|panel| {
            let path = active_work_directory_abs_path.as_ref()?;
            panel.commit_messages.get(path)
        });
        // Seed the placeholder editor with the restored draft when the active
        // repository already matches the serialized one, so the message is
        // present immediately on restart instead of only after the commit
        // buffer finishes loading in `reopen_commit_buffer`. Sourced from the
        // serialized draft rather than a live buffer snapshot and scoped to the
        // matching repository, so it neither replays cleared text nor leaks a
        // draft across repositories. `reopen_commit_buffer` still performs the
        // one-shot restore into the loaded buffer; applying the same draft
        // there is idempotent.
        let amend_pending = active_draft.is_some_and(|draft| draft.amend_pending);
        let original_commit_message = active_draft.and_then(|draft| draft.original_message.clone());
        let initial_commit_message = active_draft
            .and_then(|draft| draft.message.clone())
            .unwrap_or_default();
        let pending_commit_message_restores = serialized_panel
            .map(|panel| panel.commit_messages)
            .unwrap_or_default();

        cx.new(|cx| {
            let focus_handle = cx.focus_handle();
            cx.on_focus(&focus_handle, window, Self::focus_in).detach();

            // just to let us render a placeholder editor.
            // Once the active git repo is set, this buffer will be replaced.
            let temporary_buffer = cx.new(|cx| Buffer::local(initial_commit_message, cx));
            let commit_editor =
                cx.new(|cx| commit_message_editor(temporary_buffer, project.clone(), window, cx));
            let tool_window = cx.new(|cx| {
                GitToolWindow::new(workspace_handle.clone(), project.clone(), window, cx)
            });

            cx.subscribe_in(
                &git_store,
                window,
                move |this, _git_store, event, window, cx| match event {
                    GitStoreEvent::RepositoryUpdated(
                        _,
                        RepositoryEvent::StatusesChanged | RepositoryEvent::HeadChanged,
                        true,
                    )
                    | GitStoreEvent::RepositoryAdded
                    | GitStoreEvent::RepositoryRemoved(_)
                    | GitStoreEvent::ActiveRepositoryChanged(_)
                    | GitStoreEvent::GlobalConfigurationUpdated => {
                        this.schedule_update(window, cx);
                    }
                    GitStoreEvent::IndexWriteError(error) => {
                        this.workspace
                            .update(cx, |workspace, cx| {
                                workspace.show_error(format!("{error}"), cx);
                            })
                            .ok();
                    }
                    GitStoreEvent::RepositoryUpdated(_, _, _) => {}
                    GitStoreEvent::JobsUpdated
                    | GitStoreEvent::ConflictsUpdated
                    | GitStoreEvent::DiffBaseChanged(_)
                    | GitStoreEvent::ConsoleUpdated => {}
                },
            )
            .detach();

            let mut this = Self {
                active_repository,
                commit_editor,
                status_entries: Vec::new(),
                single_staged_entry: None,
                single_tracked_entry: None,
                conflicted_staged_count: 0,
                new_staged_count: 0,
                tracked_count: 0,
                tracked_staged_count: 0,
                changes_count: 0,
                focus_handle,
                fs,
                pending_commit: None,
                pending_remote_operation: None,
                amend_pending,
                original_commit_message,
                pending_commit_message_restores,
                signoff_enabled,
                skip_hooks_enabled: false,
                pending_serialization: Task::ready(()),
                project,
                update_status_entries_task: Task::ready(()),
                reopen_commit_buffer_task: Task::ready(()),
                workspace: workspace_handle,
                commit_template: None,
                tool_window,
                _commit_message_buffer_subscription: None,
                remote_action_menu_handle: PopoverMenuHandle::default(),
            };

            this.schedule_update(window, cx);
            this
        })
    }

    fn serialization_key(workspace: &Workspace) -> Option<String> {
        workspace
            .database_id()
            .map(|id| i64::from(id).to_string())
            .or(workspace.session_id())
            .map(|id| format!("{}-{:?}", GIT_PANEL_KEY, id))
    }

    fn serialize(&mut self, cx: &mut Context<Self>) {
        let signoff_enabled = self.signoff_enabled;
        let commit_messages = self.serialized_commit_messages(cx);
        let kvp = KeyValueStore::global(cx);

        self.pending_serialization = cx.spawn(async move |git_panel, cx| {
            cx.background_executor()
                .timer(SERIALIZATION_THROTTLE_TIME)
                .await;
            let Some(serialization_key) = git_panel
                .update(cx, |git_panel, cx| {
                    git_panel
                        .workspace
                        .read_with(cx, |workspace, _| Self::serialization_key(workspace))
                        .ok()
                        .flatten()
                })
                .ok()
                .flatten()
            else {
                return;
            };
            cx.background_spawn(
                async move {
                    kvp.write_kvp(
                        serialization_key,
                        serde_json::to_string(&SerializedGitPanel {
                            signoff_enabled,
                            commit_messages,
                        })?,
                    )
                    .await?;
                    anyhow::Ok(())
                }
                .log_err(),
            )
            .await;
        });
    }

    fn serialized_commit_messages(&self, cx: &App) -> BTreeMap<String, SerializedCommitMessage> {
        let active_work_directory_abs_path = self.active_repository.as_ref().map(|repository| {
            repository
                .read(cx)
                .work_directory_abs_path
                .to_string_lossy()
                .into_owned()
        });
        let git_store = self.project.read(cx).git_store().clone();
        let mut commit_messages = self.pending_commit_message_restores.clone();
        for repository in git_store.read(cx).repositories().values() {
            let repository = repository.read(cx);
            let work_directory_abs_path = repository
                .work_directory_abs_path
                .to_string_lossy()
                .into_owned();
            if active_work_directory_abs_path.as_deref() == Some(work_directory_abs_path.as_str()) {
                continue;
            }
            if let Some(buffer) = repository.commit_message_buffer() {
                let text = buffer.read(cx).text();
                if text.trim().is_empty() {
                    commit_messages.remove(&work_directory_abs_path);
                } else {
                    commit_messages.insert(
                        work_directory_abs_path,
                        SerializedCommitMessage {
                            message: Some(text),
                            original_message: None,
                            amend_pending: false,
                        },
                    );
                }
            }
        }
        if let Some(work_directory_abs_path) = active_work_directory_abs_path {
            let text = self.commit_message_buffer(cx).read(cx).text();
            let trimmed_text = text.trim();
            let use_buffer_text = !trimmed_text.is_empty()
                && self
                    .commit_template
                    .as_ref()
                    .is_none_or(|commit_template| commit_template.template.trim() != trimmed_text);
            let message = use_buffer_text.then_some(text);
            let original_message = self.original_commit_message.clone();
            let amend_pending = self.amend_pending;
            if message.is_some() || original_message.is_some() || amend_pending {
                commit_messages.insert(
                    work_directory_abs_path,
                    SerializedCommitMessage {
                        message,
                        original_message,
                        amend_pending,
                    },
                );
            } else {
                commit_messages.remove(&work_directory_abs_path);
            }
        }
        commit_messages
    }

    fn close_panel(&mut self, _: &Close, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PanelEvent::Close);
    }

    fn focus_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.contains_focused(window, cx) {
            cx.emit(Event::Focus);
        }
    }

    fn conflict_context(&self, repository: Entity<Repository>, window: &Window) -> ConflictContext {
        ConflictContext {
            workspace: self.workspace.clone(),
            window: window.window_handle(),
            repository,
        }
    }

    fn resolve_unstash_conflicts(
        &self,
        repository: Entity<Repository>,
        stash: Option<git::stash::StashEntry>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        merge_tool::resolve_unstash_conflicts(self.conflict_context(repository, window), stash, cx);
    }

    fn resolve_conflicts_after_pull(
        &self,
        repository: Entity<Repository>,
        rebase: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        merge_tool::resolve_conflicts_after_pull(
            self.conflict_context(repository, window),
            rebase,
            cx,
        );
    }

    fn change_all_files_stage(&mut self, stage: bool, cx: &mut Context<Self>) {
        let Some(active_repository) = self.active_repository.clone() else {
            return;
        };
        cx.spawn({
            async move |this, cx| {
                let result = this
                    .update(cx, |_this, cx| {
                        active_repository.update(cx, |repo, cx| {
                            if stage {
                                repo.stage_all(cx)
                            } else {
                                repo.unstage_all(cx)
                            }
                        })
                    })?
                    .await;

                this.update(cx, |this, cx| {
                    if let Err(err) = result {
                        this.show_error_toast(if stage { "add" } else { "reset" }, err, cx);
                    }
                    this.update_counts(active_repository.read(cx));
                    cx.notify()
                })
            }
        })
        .detach();
    }

    fn stage_status_for_entry(entry: &GitStatusEntry, repo: &Repository) -> StageStatus {
        // Checking for current staged/unstaged file status is a chained operation:
        // 1. first, we check for any pending operation recorded in repository
        // 2. if there are no pending ops either running or finished, we then ask the repository
        //    for the most up-to-date file status read from disk - we do this since `entry` arg to this function `render_entry`
        //    is likely to be staled, and may lead to weird artifacts in the form of subsecond auto-uncheck/check on
        //    the checkbox's state (or flickering) which is undesirable.
        // 3. finally, if there is no info about this `entry` in the repo, we fall back to whatever status is encoded
        //    in `entry` arg.
        repo.pending_ops_for_path(&entry.repo_path)
            .and_then(|ops| {
                // In case the last operation in the list of pending operations
                // failed, we can't assume the stage status for this entry and
                // need to fallback to the actual state in the repo.
                if ops.last_op_errored() {
                    return None;
                }

                if ops.staging() || ops.staged() {
                    Some(StageStatus::Staged)
                } else {
                    Some(StageStatus::Unstaged)
                }
            })
            .or_else(|| {
                repo.status_for_path(&entry.repo_path)
                    .map(|status| status.status.staging())
            })
            .unwrap_or(entry.staging)
    }

    pub fn stage_all(&mut self, _: &StageAll, _window: &mut Window, cx: &mut Context<Self>) {
        self.change_all_files_stage(true, cx);
    }

    pub fn unstage_all(&mut self, _: &UnstageAll, _window: &mut Window, cx: &mut Context<Self>) {
        self.change_all_files_stage(false, cx);
    }

    fn toggle_staged_for_entry(&mut self, entry: &GitStatusEntry, cx: &mut Context<Self>) {
        let Some(active_repository) = self.active_repository.as_ref() else {
            return;
        };
        let stage = match Self::stage_status_for_entry(entry, active_repository.read(cx)) {
            StageStatus::Staged => false,
            StageStatus::Unstaged | StageStatus::PartiallyStaged => true,
        };
        self.change_file_stage(stage, vec![entry.clone()], cx);
    }

    fn change_file_stage(
        &mut self,
        stage: bool,
        entries: Vec<GitStatusEntry>,
        cx: &mut Context<Self>,
    ) {
        let Some(active_repository) = self.active_repository.clone() else {
            return;
        };
        cx.spawn({
            async move |this, cx| {
                let result = this
                    .update(cx, |this, cx| {
                        let task = active_repository.update(cx, |repo, cx| {
                            let repo_paths = entries
                                .iter()
                                .map(|entry| entry.repo_path.clone())
                                .unique()
                                .collect();
                            if stage {
                                repo.stage_entries(repo_paths, cx)
                            } else {
                                repo.unstage_entries(repo_paths, cx)
                            }
                        });
                        this.update_counts(active_repository.read(cx));
                        cx.notify();
                        task
                    })?
                    .await;

                this.update(cx, |this, cx| {
                    if let Err(err) = result {
                        this.show_error_toast(if stage { "add" } else { "reset" }, err, cx);
                        this.update_counts(active_repository.read(cx));
                    }
                    cx.notify();
                })
            }
        })
        .detach();
    }

    pub fn total_staged_count(&self) -> usize {
        self.tracked_staged_count + self.new_staged_count + self.conflicted_staged_count
    }

    pub fn stash_pop(&mut self, _: &StashPop, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active_repository) = self.active_repository.clone() else {
            return;
        };

        cx.spawn_in(window, {
            async move |this, cx| {
                let stash = active_repository.update(cx, |repo, _| {
                    repo.cached_stash()
                        .entries
                        .iter()
                        .min_by_key(|entry| entry.index)
                        .cloned()
                });
                let stash_task = active_repository
                    .update(cx, |repo, cx| repo.stash_pop(None, cx))
                    .await;
                this.update_in(cx, |this, window, cx| {
                    match stash_task {
                        Ok(()) => {}
                        Err(error) if stash_pop_left_conflicts(&error) => {
                            this.resolve_unstash_conflicts(active_repository, stash, window, cx);
                        }
                        Err(error) => this.show_error_toast("stash pop", error, cx),
                    }
                    cx.notify();
                })
            }
        })
        .detach();
    }

    pub fn stash_apply(&mut self, _: &StashApply, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active_repository) = self.active_repository.clone() else {
            return;
        };

        cx.spawn_in(window, {
            async move |this, cx| {
                let stash = active_repository.update(cx, |repo, _| {
                    repo.cached_stash()
                        .entries
                        .iter()
                        .min_by_key(|entry| entry.index)
                        .cloned()
                });
                let stash_task = active_repository
                    .update(cx, |repo, cx| repo.stash_apply(None, cx))
                    .await;
                this.update_in(cx, |this, window, cx| {
                    match stash_task {
                        Ok(()) => {}
                        Err(error) if stash_pop_left_conflicts(&error) => {
                            this.resolve_unstash_conflicts(active_repository, stash, window, cx);
                        }
                        Err(error) => this.show_error_toast("stash apply", error, cx),
                    }
                    cx.notify();
                })
            }
        })
        .detach();
    }

    pub fn stash_all(&mut self, _: &StashAll, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_for_stash_message(StashKind::All, window, cx);
    }

    pub fn stash_tracked(&mut self, _: &StashTracked, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_for_stash_message(StashKind::Tracked, window, cx);
    }

    pub fn stash_staged(&mut self, _: &StashStaged, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_for_stash_message(StashKind::Staged, window, cx);
    }

    fn prompt_for_stash_message(
        &mut self,
        kind: StashKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_repository.is_none() {
            return;
        }
        // `git::StashAll` is also registered on the workspace, which dispatches it while
        // `Workspace` is leased, so opening the modal inline would re-enter that update.
        cx.defer_in(window, move |this, window, cx| {
            let panel = cx.entity().downgrade();
            this.workspace
                .update(cx, |workspace, cx| {
                    workspace.toggle_modal(window, cx, |window, cx| {
                        StashMessageModal::new(panel, kind, window, cx)
                    });
                })
                .ok();
        });
    }

    fn perform_stash(&mut self, kind: StashKind, message: Option<String>, cx: &mut Context<Self>) {
        let Some(active_repository) = self.active_repository.clone() else {
            return;
        };

        cx.spawn({
            async move |this, cx| {
                let stash_task = active_repository
                    .update(cx, |repo, cx| match kind {
                        StashKind::All => repo.stash_all(message, cx),
                        StashKind::Tracked => repo.stash_tracked(message, cx),
                        StashKind::Staged => repo.stash_staged(message, cx),
                    })
                    .await;
                this.update(cx, |this, cx| {
                    stash_task
                        .map_err(|e| {
                            this.show_error_toast(kind.error_action(), e, cx);
                        })
                        .ok();
                    cx.notify();
                })
            }
        })
        .detach();
    }

    pub fn commit_message_buffer(&self, cx: &App) -> Entity<Buffer> {
        self.commit_editor
            .read(cx)
            .buffer()
            .read(cx)
            .as_singleton()
            .unwrap()
    }

    /// Commits staged changes with the current commit message.
    /// When `amend_pending` is true, performs an amend commit instead.
    ///
    /// Returns `true` if the commit was executed, `false` otherwise.
    pub(crate) fn commit(
        &mut self,
        commit_editor_focus_handle: &FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if commit_editor_focus_handle.contains_focused(window, cx) {
            if !self.has_commit_message(cx) {
                return false;
            }
            let options = self.commit_options();
            self.commit_changes(options, window, cx);
            true
        } else {
            cx.propagate();
            false
        }
    }

    /// Commits staged changes with the current commit message and, once the commit succeeds,
    /// pushes the branch, like IntelliJ's "Commit and Push…".
    pub(crate) fn commit_and_push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let options = self.commit_options();
        let Some(completion) = self.commit_changes_with_completion(options, false, window, cx)
        else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(())) = completion.await {
                this.update_in(cx, |this, window, cx| this.push(false, false, window, cx))?;
            }
            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
    }

    /// Enters the amend state on first invocation, loading the last commit
    /// message for editing. On second invocation, performs the amend commit
    /// by delegating to [`Self::commit`]. Returns `true` if a commit was
    /// executed.
    pub(crate) fn amend(
        &mut self,
        commit_editor_focus_handle: &FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if commit_editor_focus_handle.contains_focused(window, cx) {
            if self.head_commit(cx).is_some() {
                if !self.amend_pending {
                    self.toggle_amend_pending(cx);
                } else {
                    return self.commit(commit_editor_focus_handle, window, cx);
                }
            }
            false
        } else {
            cx.propagate();
            false
        }
    }
    pub fn head_commit(&self, cx: &App) -> Option<CommitDetails> {
        self.active_repository
            .as_ref()
            .and_then(|repo| repo.read(cx).head_commit.as_ref())
            .cloned()
    }

    pub fn load_last_commit_message(&mut self, cx: &mut Context<Self>) {
        let Some(head_commit) = self.head_commit(cx) else {
            return;
        };

        let recent_sha = head_commit.sha.to_string();
        let detail_task = self.load_commit_details(recent_sha, cx);
        cx.spawn(async move |this, cx| {
            if let Ok(message) = detail_task.await.map(|detail| detail.message) {
                this.update(cx, |this, cx| {
                    this.commit_message_buffer(cx).update(cx, |buffer, cx| {
                        let start = buffer.anchor_before(0);
                        let end = buffer.anchor_after(buffer.len());
                        buffer.edit([(start..end, message)], None, cx);
                    });
                })
                .log_err();
            }
        })
        .detach();
    }

    fn custom_or_suggested_commit_message(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let git_commit_language = self
            .commit_editor
            .read(cx)
            .language_at(MultiBufferOffset(0), cx);
        let message = self.commit_editor.read(cx).text(cx);
        if message.is_empty() {
            return self
                .suggest_commit_message(cx)
                .filter(|message| !message.trim().is_empty());
        } else if message.trim().is_empty() {
            return None;
        }
        let buffer = cx.new(|cx| {
            let mut buffer = Buffer::local(message, cx);
            buffer.set_language(git_commit_language, cx);
            buffer
        });
        let editor = cx.new(|cx| Editor::for_buffer(buffer, None, window, cx));
        let wrapped_message = editor.update(cx, |editor, cx| {
            editor.select_all(&Default::default(), window, cx);
            editor.rewrap(
                RewrapOptions {
                    override_language_settings: false,
                    preserve_existing_whitespace: true,
                    line_length: None,
                },
                cx,
            );
            editor.text(cx)
        });
        if wrapped_message.trim().is_empty() {
            return None;
        }
        Some(wrapped_message)
    }

    fn has_commit_message(&self, cx: &mut Context<Self>) -> bool {
        let text = self.commit_editor.read(cx).text(cx);
        if !text.trim().is_empty() {
            true
        } else if text.is_empty() {
            self.suggest_commit_message(cx)
                .is_some_and(|text| !text.trim().is_empty())
        } else {
            false
        }
    }

    pub(crate) fn commit_options(&self) -> CommitOptions {
        CommitOptions {
            amend: self.amend_pending,
            signoff: self.signoff_enabled,
            allow_empty: false,
            no_verify: self.skip_hooks_enabled,
        }
    }

    /// Buffers of files on disk with edits that aren't saved yet.
    fn edited_buffers(&self, cx: &App) -> HashSet<Entity<Buffer>> {
        self.project
            .read(cx)
            .opened_buffers(cx)
            .into_iter()
            .filter(|buffer| {
                let buffer = buffer.read(cx);
                buffer.has_unsaved_edits()
                    && buffer.file().is_some_and(|file| file.disk_state().exists())
            })
            .collect()
    }

    pub(crate) fn commit_changes(
        &mut self,
        options: CommitOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        drop(self.commit_changes_with_completion(options, false, window, cx));
    }

    pub(crate) fn commit_changes_with_completion(
        &mut self,
        options: CommitOptions,
        staged_only: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<futures::channel::oneshot::Receiver<anyhow::Result<()>>> {
        if self.pending_commit.is_some() {
            return None;
        }
        let Some(active_repository) = self.active_repository.clone() else {
            return None;
        };
        let error_spawn = |message, window: &mut Window, cx: &mut App| {
            let prompt = window.prompt(PromptLevel::Warning, message, None, &["OK"], cx);
            cx.spawn(async move |_| {
                prompt.await.ok();
            })
            .detach();
        };

        if self.has_unstaged_conflicts() {
            error_spawn(
                "There are still conflicts. You must stage these before committing",
                window,
                cx,
            );
            return None;
        }

        let askpass = self.askpass_delegate("git commit", window, cx);
        let commit_message = self.custom_or_suggested_commit_message(window, cx);

        let Some(message) = commit_message else {
            return None;
        };

        // Like IntelliJ's `saveAllDocuments` before a commit: the commit must contain what the
        // editors show, or the committed files would still look changed afterwards.
        let edited_buffers = self.edited_buffers(cx);
        let save_task = self.project.update(cx, |project, cx| {
            project.save_buffers(edited_buffers.clone(), cx)
        });
        let task = if self.has_staged_changes() || staged_only {
            cx.spawn(async move |_, cx| {
                save_task.await?;
                let commit_task = active_repository.update(cx, |repo, cx| {
                    repo.commit(message.into(), None, options, askpass, cx)
                });
                commit_task.await?
            })
        } else {
            let has_changed_files = self
                .status_entries
                .iter()
                .any(|status_entry| !status_entry.status.is_created());
            if !has_changed_files && edited_buffers.is_empty() && !options.amend {
                error_spawn("No changes to commit", window, cx);
                return None;
            }

            cx.spawn(async move |_, cx| {
                save_task.await?;
                // The statuses must include the files that were only just saved.
                let reload = active_repository.update(cx, |repo, cx| {
                    repo.rescan(cx);
                    repo.barrier()
                });
                reload.await?;
                let changed_files = active_repository.read_with(cx, |repo, _| {
                    repo.cached_status()
                        .filter(|entry| !entry.status.is_created())
                        .map(|entry| entry.repo_path)
                        .collect::<Vec<_>>()
                });
                if changed_files.is_empty() && !options.amend {
                    anyhow::bail!("No changes to commit");
                }
                let stage_task =
                    active_repository.update(cx, |repo, cx| repo.stage_entries(changed_files, cx));
                stage_task.await?;
                let commit_task = active_repository.update(cx, |repo, cx| {
                    repo.commit(message.into(), None, options, askpass, cx)
                });
                commit_task.await?
            })
        };
        let (completion, receiver) = futures::channel::oneshot::channel();
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.pending_commit.take();

                match &result {
                    Ok(()) => {
                        this.set_skip_hooks_enabled(false, cx);
                        if options.amend {
                            this.set_amend_pending(false, cx);
                        } else {
                            this.commit_editor
                                .update(cx, |editor, cx| editor.clear(window, cx));
                            this.original_commit_message = None;
                            this.serialize(cx);
                        }
                    }
                    // The batch caller reports failures with its accumulated commit results.
                    Err(_) if staged_only => {}
                    Err(error) => this.show_error_toast("commit", anyhow::anyhow!("{error:#}"), cx),
                }
            })
            .log_err();
            if completion.send(result).is_err() {
                log::debug!("commit completion receiver was dropped");
            }
        });

        self.pending_commit = Some(task);
        Some(receiver)
    }

    pub(crate) fn uncommit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.active_repository.clone() else {
            return;
        };

        let confirmation = self.check_for_pushed_commits(window, cx);
        let prior_head = self.load_commit_details("HEAD".to_string(), cx);

        let task = cx.spawn_in(window, async move |this, cx| {
            let result = maybe!(async {
                if let Ok(true) = confirmation.await {
                    let prior_head = prior_head.await?;

                    repo.update(cx, |repo, cx| {
                        repo.reset("HEAD^".to_string(), ResetMode::Soft, cx)
                    })
                    .await??;

                    Ok(Some(prior_head))
                } else {
                    Ok(None)
                }
            })
            .await;

            this.update_in(cx, |this, window, cx| {
                this.pending_commit.take();
                match result {
                    Ok(None) => {}
                    Ok(Some(prior_commit)) => {
                        this.commit_editor.update(cx, |editor, cx| {
                            editor.set_text(prior_commit.message, window, cx)
                        });
                    }
                    Err(e) => this.show_error_toast("reset", e, cx),
                }
            })
            .ok();
        });

        self.pending_commit = Some(task);
    }

    fn check_for_pushed_commits(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl Future<Output = anyhow::Result<bool>> + use<> {
        let repo = self.active_repository.clone();
        let mut cx = window.to_async(cx);

        async move {
            let repo = repo.context("No active repository")?;

            let pushed_to: Vec<SharedString> = repo
                .update(&mut cx, |repo, _| repo.check_for_pushed_commits())
                .await??;

            if pushed_to.is_empty() {
                Ok(true)
            } else {
                #[derive(strum::EnumIter, strum::VariantNames)]
                #[strum(serialize_all = "title_case")]
                enum CancelUncommit {
                    Uncommit,
                    Cancel,
                }
                let detail = format!(
                    "This commit was already pushed to {}.",
                    pushed_to.into_iter().join(", ")
                );
                let result = cx
                    .update(|window, cx| prompt("Are you sure?", Some(&detail), window, cx))?
                    .await?;

                match result {
                    CancelUncommit::Cancel => Ok(false),
                    CancelUncommit::Uncommit => Ok(true),
                }
            }
        }
    }

    /// Suggests a commit message based on the changed files and their statuses
    pub fn suggest_commit_message(&self, cx: &App) -> Option<String> {
        if let Some(merge_message) = self
            .active_repository
            .as_ref()
            .and_then(|repo| repo.read(cx).merge.message.as_ref())
        {
            return Some(merge_message.to_string());
        }

        let git_status_entry = if let Some(staged_entry) = &self.single_staged_entry {
            Some(staged_entry)
        } else if self.total_staged_count() == 0
            && let Some(single_tracked_entry) = &self.single_tracked_entry
        {
            Some(single_tracked_entry)
        } else {
            None
        }?;

        let action_text = if git_status_entry.status.is_deleted() {
            Some("Delete")
        } else if git_status_entry.status.is_created() {
            Some("Create")
        } else if git_status_entry.status.is_modified() {
            Some("Update")
        } else {
            None
        }?;

        let file_name = git_status_entry
            .repo_path
            .file_name()
            .unwrap_or_default()
            .to_string();

        Some(format!("{} {}", action_text, file_name))
    }

    fn get_fetch_options(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Option<FetchOptions>> {
        let repo = self.active_repository.clone();
        let workspace = self.workspace.clone();

        cx.spawn_in(window, async move |_, cx| {
            let repo = repo?;
            let remotes = repo
                .update(cx, |repo, _| repo.get_remotes(None, false))
                .await
                .ok()?
                .log_err()?;

            let mut remotes: Vec<_> = remotes.into_iter().map(FetchOptions::Remote).collect();
            if remotes.len() > 1 {
                remotes.push(FetchOptions::All);
            }
            let selection = cx
                .update(|window, cx| {
                    picker_prompt::prompt(
                        "Pick which remote to fetch",
                        remotes.iter().map(|r| r.name()).collect(),
                        workspace,
                        window,
                        cx,
                    )
                })
                .ok()?
                .await?;
            remotes.get(selection).cloned()
        })
    }

    pub(crate) fn fetch(
        &mut self,
        is_fetch_all: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_push_and_pull(cx) {
            return;
        }

        let Some(repo) = self.active_repository.clone() else {
            return;
        };
        if !self.start_remote_operation(RemoteOperationKind::Fetch, cx) {
            return;
        }

        let askpass = self.askpass_delegate("git fetch", window, cx);
        let this = cx.weak_entity();

        let fetch_options = if is_fetch_all {
            Task::ready(Some(FetchOptions::All))
        } else {
            self.get_fetch_options(window, cx)
        };

        window
            .spawn(cx, async move |cx| {
                let _clear_pending_remote_operation = cx.on_drop(&this, |this, cx| {
                    this.clear_remote_operation(cx);
                });

                let Some(fetch_options) = fetch_options.await else {
                    return Ok(());
                };
                let fetch = repo.update(cx, |repo, cx| {
                    repo.fetch(fetch_options.clone(), askpass, cx)
                });

                let remote_message = fetch.await?;
                this.update(cx, |this, cx| {
                    let action = match fetch_options {
                        FetchOptions::All | FetchOptions::Unshallow => RemoteAction::Fetch(None),
                        FetchOptions::Remote(remote) => RemoteAction::Fetch(Some(remote)),
                    };
                    match remote_message {
                        Ok(remote_message) => this.show_remote_output(action, remote_message, cx),
                        Err(e) => {
                            log::error!("Error while fetching {:?}", e);
                            this.show_error_toast(action.name(), e, cx)
                        }
                    }

                    anyhow::Ok(())
                })
                .ok();
                anyhow::Ok(())
            })
            .detach_and_log_err(cx);
    }

    pub(crate) fn git_clone(&mut self, repo: String, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.workspace.clone();

        crate::clone::clone_and_open(
            repo.into(),
            workspace,
            window,
            cx,
            Arc::new(|_workspace: &mut workspace::Workspace, _window, _cx| {}),
        );
    }

    pub(crate) fn git_init(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let worktrees = self
            .project
            .read(cx)
            .visible_worktrees(cx)
            .filter(|worktree| !worktree.read(cx).is_single_file())
            .collect::<Vec<_>>();

        let worktree = if let [worktree] = worktrees.as_slice() {
            Task::ready(Some(worktree.clone()))
        } else if worktrees.is_empty() {
            let result = window.prompt(
                PromptLevel::Warning,
                "Unable to initialize a git repository",
                Some("Open a directory first"),
                &["OK"],
                cx,
            );
            cx.background_executor()
                .spawn(async move {
                    result.await.ok();
                })
                .detach();
            return;
        } else {
            let worktree_directories = worktrees
                .iter()
                .map(|worktree| worktree.read(cx).abs_path())
                .map(|worktree_abs_path| {
                    if let Ok(path) = worktree_abs_path.strip_prefix(util::paths::home_dir()) {
                        Path::new("~")
                            .join(path)
                            .to_string_lossy()
                            .to_string()
                            .into()
                    } else {
                        worktree_abs_path.to_string_lossy().into_owned().into()
                    }
                })
                .collect_vec();
            let prompt = picker_prompt::prompt(
                "Where would you like to initialize this git repository?",
                worktree_directories,
                self.workspace.clone(),
                window,
                cx,
            );

            cx.spawn(async move |_, _| prompt.await.map(|ix| worktrees[ix].clone()))
        };

        cx.spawn_in(window, async move |this, cx| {
            let worktree = match worktree.await {
                Some(worktree) => worktree,
                None => {
                    return;
                }
            };

            let Ok(result) = this.update(cx, |this, cx| {
                let fallback_branch_name = GitPanelSettings::get_global(cx)
                    .fallback_branch_name
                    .clone();
                this.project.read(cx).git_init(
                    worktree.read(cx).abs_path(),
                    fallback_branch_name,
                    cx,
                )
            }) else {
                return;
            };

            let result = result.await;

            this.update_in(cx, |this, _, cx| match result {
                Ok(()) => {}
                Err(e) => this.show_error_toast("init", e, cx),
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn pull(&mut self, rebase: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_push_and_pull(cx) {
            return;
        }
        let Some(repo) = self.active_repository.clone() else {
            return;
        };
        let Some(branch) = repo.read(cx).branch.clone() else {
            return;
        };
        if !self.start_remote_operation(RemoteOperationKind::Pull, cx) {
            return;
        }

        let remote = self.get_remote(false, false, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let _clear_pending_remote_operation = cx.on_drop(&this, |this, cx| {
                this.clear_remote_operation(cx);
            });

            let remote = match remote.await {
                Ok(Some(remote)) => remote,
                Ok(None) => {
                    return Ok(());
                }
                Err(e) => {
                    log::error!("Failed to get current remote: {}", e);
                    this.update(cx, |this, cx| this.show_error_toast("pull", e, cx))
                        .ok();
                    return Ok(());
                }
            };

            let askpass = this.update_in(cx, |this, window, cx| {
                this.askpass_delegate(format!("git pull {}", remote.name), window, cx)
            })?;

            let branch_name = branch
                .upstream
                .is_none()
                .then(|| branch.name().to_owned().into());

            let pull = repo.update(cx, |repo, cx| {
                repo.pull(branch_name, remote.name.clone(), rebase, askpass, cx)
            });

            let remote_message = pull.await?;

            let action = RemoteAction::Pull(remote);
            this.update_in(cx, |this, window, cx| {
                match remote_message {
                    Ok(remote_message) => this.show_remote_output(action, remote_message, cx),
                    Err(e) => {
                        log::error!("Error while pulling {:?}", e);
                        this.show_error_toast(action.name(), e, cx)
                    }
                }
                this.resolve_conflicts_after_pull(repo, rebase, window, cx);
            })
            .ok();

            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
    }

    pub(crate) fn push(
        &mut self,
        force_push: bool,
        select_remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_push_and_pull(cx) {
            return;
        }
        let Some(repo) = self.active_repository.clone() else {
            return;
        };
        let Some(branch) = repo.read(cx).branch.clone() else {
            return;
        };
        if !self.start_remote_operation(RemoteOperationKind::Push, cx) {
            return;
        }

        let options = if force_push {
            Some(PushOptions::Force)
        } else {
            match branch.upstream {
                Some(Upstream {
                    tracking: UpstreamTracking::Gone,
                    ..
                })
                | None => Some(PushOptions::SetUpstream),
                _ => None,
            }
        };
        let remote = self.get_remote(select_remote, true, window, cx);
        let context = BranchContext::new(self.workspace.clone(), repo.clone());

        cx.spawn_in(window, async move |this, cx| {
            let _clear_pending_remote_operation = cx.on_drop(&this, |this, cx| {
                this.clear_remote_operation(cx);
            });

            let remote = remote.await.and_then(|remote| {
                remote.context(
                    "No remote available to push to. Add a remote to be able to publish changes.",
                )
            });
            let remote = match remote {
                Ok(remote) => remote,
                Err(error) => {
                    log::error!("Failed to get current remote: {}", error);
                    cx.update(|_, cx| {
                        if let Some(notice) = push_not_started_notice(&error) {
                            context.notify(notice, cx);
                        }
                    })?;
                    return Ok(());
                }
            };

            let askpass_delegate = this.update_in(cx, |this, window, cx| {
                this.askpass_delegate(format!("git push {}", remote.name), window, cx)
            })?;

            let target = PushTarget {
                branch: branch.name().to_owned().into(),
                remote: remote.name,
                remote_branch: branch
                    .upstream
                    .as_ref()
                    .filter(|u| matches!(u.tracking, UpstreamTracking::Tracked(_)))
                    .and_then(|u| u.branch_name())
                    .unwrap_or_else(|| branch.name())
                    .to_owned()
                    .into(),
            };
            let pushed_commits = upstream_ahead_count(&branch, &target);
            let (progress, progress_sender) = cx.update(|_, cx| {
                OperationProgress::start_reporting(&context.workspace, PUSH_PROGRESS_TITLE, cx)
            })?;

            let push = repo.update(cx, |repo, cx| {
                repo.push(
                    target.branch.clone(),
                    target.remote_branch.clone(),
                    target.remote.clone(),
                    options,
                    askpass_delegate,
                    Some(progress_sender),
                    cx,
                )
            });

            let remote_output = push.await?;
            drop(progress);

            cx.update(|_, cx| {
                let notice = match remote_output {
                    Ok(output) => Some(push_succeeded_notice(&target, pushed_commits, &output)),
                    Err(error) => {
                        log::error!("Error while pushing {:?}", error);
                        push_failed_notice(&target, &error, context.workspace.clone())
                    }
                };
                if let Some(notice) = notice {
                    context.notify(notice, cx);
                }
            })?;

            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
    }

    pub(crate) fn show_console(
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        let Some(panel) = workspace.focus_panel::<Self>(window, cx) else {
            return;
        };
        let tool_window = panel.read(cx).tool_window.clone();
        tool_window.update(cx, |tool_window, cx| {
            tool_window.show_console_tab(window, cx)
        });
    }

    pub fn create_pull_request(&self, window: &mut Window, cx: &mut Context<Self>) {
        let result = (|| -> anyhow::Result<()> {
            let repo = self
                .active_repository
                .clone()
                .ok_or_else(|| anyhow::anyhow!("No active repository"))?;

            let (branch, remote_origin, remote_upstream) = {
                let repository = repo.read(cx);
                (
                    repository.branch.clone(),
                    repository.remote_origin_url.clone(),
                    repository.remote_upstream_url.clone(),
                )
            };

            let branch = branch.ok_or_else(|| anyhow::anyhow!("No active branch"))?;
            let source_branch = branch
                .upstream
                .as_ref()
                .filter(|upstream| matches!(upstream.tracking, UpstreamTracking::Tracked(_)))
                .and_then(|upstream| upstream.branch_name())
                .ok_or_else(|| anyhow::anyhow!("No remote configured for repository"))?;
            let source_branch = source_branch.to_string();

            let remote_url = branch
                .upstream
                .as_ref()
                .and_then(|upstream| match upstream.remote_name() {
                    Some("upstream") => remote_upstream.as_deref(),
                    Some(_) => remote_origin.as_deref(),
                    None => None,
                })
                .or(remote_origin.as_deref())
                .or(remote_upstream.as_deref())
                .ok_or_else(|| anyhow::anyhow!("No remote configured for repository"))?;
            let remote_url = remote_url.to_string();

            let provider_registry = GitHostingProviderRegistry::global(cx);
            let Some((provider, parsed_remote)) =
                git::parse_git_remote_url(provider_registry, &remote_url)
            else {
                return Err(anyhow::anyhow!("Unsupported remote URL: {}", remote_url));
            };

            let Some(url) = provider.build_create_pull_request_url(&parsed_remote, &source_branch)
            else {
                return Err(anyhow::anyhow!("Unable to construct pull request URL"));
            };

            cx.open_url(url.as_str());
            Ok(())
        })();

        if let Err(err) = result {
            log::error!("Error while creating pull request {:?}", err);
            cx.defer_in(window, |panel, _window, cx| {
                panel.show_error_toast("create pull request", err, cx);
            });
        }
    }

    fn askpass_delegate(
        &self,
        operation: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AskPassDelegate {
        let workspace = self.workspace.clone();
        let operation = operation.into();
        let window = window.window_handle();
        AskPassDelegate::new_with_cancellation(
            &mut cx.to_async(),
            move |prompt, tx, cancellation, cx| {
                window
                    .update(cx, |_, window, cx| {
                        workspace.update(cx, |workspace, cx| {
                            workspace.toggle_modal(window, cx, |window, cx| {
                                AskPassModal::new(
                                    operation.clone(),
                                    prompt.into(),
                                    tx,
                                    cancellation,
                                    window,
                                    cx,
                                )
                            });
                        })
                    })
                    .ok();
            },
        )
    }

    fn can_push_and_pull(&self, cx: &App) -> bool {
        !self.project.read(cx).is_via_collab()
    }

    fn start_remote_operation(
        &mut self,
        kind: RemoteOperationKind,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.pending_remote_operation.is_some() {
            return false;
        }

        self.pending_remote_operation = Some(kind);
        cx.notify();
        true
    }

    fn clear_remote_operation(&mut self, cx: &mut Context<Self>) {
        self.pending_remote_operation.take();
        cx.notify();
    }

    fn get_remote(
        &mut self,
        always_select: bool,
        is_push: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl Future<Output = anyhow::Result<Option<Remote>>> + use<> {
        let repo = self.active_repository.clone();
        let workspace = self.workspace.clone();
        let mut cx = window.to_async(cx);

        async move {
            let repo = repo.context("No active repository")?;
            let current_remotes: Vec<Remote> = repo
                .update(&mut cx, |repo, _| {
                    let current_branch = if always_select {
                        None
                    } else {
                        let current_branch = repo.branch.as_ref().context("No active branch")?;
                        Some(current_branch.name().to_string())
                    };
                    anyhow::Ok(repo.get_remotes(current_branch, is_push))
                })?
                .await??;

            let current_remotes: Vec<_> = current_remotes
                .into_iter()
                .map(|remotes| remotes.name)
                .collect();
            let selection = cx
                .update(|window, cx| {
                    picker_prompt::prompt(
                        "Pick which remote to push to",
                        current_remotes.clone(),
                        workspace,
                        window,
                        cx,
                    )
                })?
                .await;

            Ok(selection.map(|selection| Remote {
                name: current_remotes[selection].clone(),
            }))
        }
    }

    pub(crate) fn increase_font_size(
        &mut self,
        action: &IncreaseBufferFontSize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_font_size_action(action.persist, px(1.0), cx);
    }

    pub(crate) fn decrease_font_size(
        &mut self,
        action: &DecreaseBufferFontSize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_font_size_action(action.persist, px(-1.0), cx);
    }

    fn handle_font_size_action(&mut self, persist: bool, delta: Pixels, cx: &mut Context<Self>) {
        if persist {
            update_settings_file(self.fs.clone(), cx, move |settings, cx| {
                let git_commit_buffer_font_size =
                    ThemeSettings::get_global(cx).git_commit_buffer_font_size(cx) + delta;

                let _ = settings.theme.git_commit_buffer_font_size.insert(
                    f32::from(theme_settings::clamp_font_size(git_commit_buffer_font_size)).into(),
                );
            });
        } else {
            theme_settings::adjust_git_commit_buffer_font_size(cx, |size| size + delta);
        }
    }

    pub(crate) fn reset_font_size(
        &mut self,
        action: &ResetBufferFontSize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if action.persist {
            update_settings_file(self.fs.clone(), cx, move |settings, _| {
                settings.theme.git_commit_buffer_font_size = None;
            });
        } else {
            theme_settings::reset_git_commit_buffer_font_size(cx);
        }
    }

    fn schedule_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let new_active_repository = self.project.read(cx).active_repository(cx);
        let active_repository_changed = self.active_repository.as_ref().map(Entity::entity_id)
            != new_active_repository.as_ref().map(Entity::entity_id);
        if active_repository_changed {
            self.set_skip_hooks_enabled(false, cx);
            if self.amend_pending {
                // Leaving a repository with a pending amend: undo it so the amend
                // state doesn't carry over to the newly active repository. The
                // commit editor still holds the previous repository's buffer here
                // (`reopen_commit_buffer` swaps it asynchronously below), so this
                // restores the pre-amend draft into that repository's buffer.
                self.set_amend_pending(false, cx);
            }
        }
        self.active_repository = new_active_repository;
        self.reopen_commit_buffer(window, cx);
        self.update_status_entries_task = cx.spawn(async move |git_panel, cx| {
            cx.background_executor().timer(UPDATE_DEBOUNCE).await;
            git_panel
                .update(cx, |git_panel, cx| git_panel.update_status_entries(cx))
                .ok();
        });
    }

    fn reopen_commit_buffer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active_repo) = self.active_repository.as_ref() else {
            self.reopen_commit_buffer_task = Task::ready(());
            return;
        };
        let active_repository_abs_path = active_repo
            .read(cx)
            .work_directory_abs_path
            .to_string_lossy()
            .into_owned();
        let load_buffer = active_repo.update(cx, |active_repo, cx| {
            let project = self.project.read(cx);
            active_repo.open_commit_buffer(
                Some(project.languages().clone()),
                project.buffer_store().clone(),
                cx,
            )
        });
        let load_template = self.load_commit_template(cx);

        self.reopen_commit_buffer_task = cx.spawn_in(window, async move |git_panel, cx| {
            let result = async {
                // Set up the buffer before awaiting the commit template as the
                // request may never resolve (for example, a collab host that
                // doesn't know about `LoadCommitTemplate`) and must not block the
                // commit editor from attaching to the shared buffer.
                let buffer = load_buffer.await?;
                git_panel.update_in(cx, |git_panel, window, cx| {
                    if git_panel
                        .commit_editor
                        .read(cx)
                        .buffer()
                        .read(cx)
                        .as_singleton()
                        .as_ref()
                        != Some(&buffer)
                    {
                        git_panel.commit_editor = cx.new(|cx| {
                            commit_message_editor(
                                buffer.clone(),
                                git_panel.project.clone(),
                                window,
                                cx,
                            )
                        });
                    }

                    // Create subscription such that, any edit on the commit
                    // editor's buffer will be serialized and saved to the database
                    // in order to be able to restore it in case there's a
                    // disconnect.
                    git_panel._commit_message_buffer_subscription =
                        Some(cx.subscribe(&buffer, |git_panel, _, event, cx| {
                            if matches!(event, BufferEvent::Edited { .. }) {
                                git_panel.serialize(cx);
                            }
                        }));
                })?;

                // Check whether there's a pending commit message for this
                // repository and, if that's the case, update the buffer's
                // text.
                git_panel.update(cx, |git_panel, cx| {
                    if let Some(restored_commit_message) = git_panel
                        .pending_commit_message_restores
                        .remove(&active_repository_abs_path)
                    {
                        git_panel.amend_pending = restored_commit_message.amend_pending;
                        git_panel.original_commit_message =
                            restored_commit_message.original_message;
                        cx.notify();

                        if let Some(message) = restored_commit_message.message
                            && buffer.read(cx).text().trim().is_empty()
                        {
                            buffer.update(cx, |buffer, cx| {
                                buffer.set_text(message, cx);
                            });
                        }
                    }
                })?;

                // Only apply the template if it's non-empty and the buffer has no
                // content, so we never override a commit message that was already
                // in progress.
                let commit_template = load_template.await?;
                git_panel.update(cx, |git_panel, cx| {
                    git_panel.commit_template = commit_template;

                    if let Some(commit_template) = git_panel.commit_template.as_ref()
                        && !commit_template.template.is_empty()
                        && buffer.read(cx).text().trim().is_empty()
                    {
                        buffer.update(cx, |buffer, cx| {
                            buffer.set_text(commit_template.template.clone(), cx);
                        });
                    }
                })?;

                anyhow::Ok(())
            }
            .await;

            result.log_err();
        });
    }

    fn update_status_entries(&mut self, cx: &mut Context<Self>) {
        self.active_repository = self.project.read(cx).active_repository(cx);
        self.status_entries.clear();
        self.single_staged_entry = None;
        self.single_tracked_entry = None;
        self.changes_count = 0;
        self.conflicted_staged_count = 0;
        self.new_staged_count = 0;
        self.tracked_count = 0;
        self.tracked_staged_count = 0;

        let Some(repo) = self.active_repository.as_ref() else {
            cx.notify();
            return;
        };
        let repo = repo.read(cx);

        let mut has_conflicts = false;
        let mut staged_count = 0;
        let mut last_staged_index = None;
        let mut tracked_change_count = 0;
        let mut last_tracked_index = None;
        for status_entry in repo.cached_status() {
            if let Some(pending) = repo.pending_ops_for_path(&status_entry.repo_path)
                && pending
                    .ops
                    .iter()
                    .any(|op| op.git_status == pending_op::GitStatus::Reverted && op.finished())
            {
                continue;
            }
            self.changes_count += 1;

            let is_conflict = repo.had_conflict_on_last_merge_head_change(&status_entry.repo_path);
            let staging = status_entry.status.staging();
            has_conflicts |= is_conflict;
            if !is_conflict && !status_entry.status.is_created() {
                tracked_change_count += 1;
                last_tracked_index = Some(self.status_entries.len());
            }
            if staging.has_staged() {
                staged_count += 1;
                last_staged_index = Some(self.status_entries.len());
            }
            self.status_entries.push(GitStatusEntry {
                repo_path: status_entry.repo_path,
                status: status_entry.status,
                staging,
            });
        }

        if !has_conflicts {
            if staged_count == 1
                && let Some(entry) =
                    last_staged_index.and_then(|index| self.status_entries.get(index))
            {
                if repo
                    .pending_ops_for_path(&entry.repo_path)
                    .is_none_or(|ops| ops.staged())
                {
                    self.single_staged_entry = Some(entry.clone());
                }
            } else if repo.pending_ops_summary().item_summary.staging_count == 1
                && let Some(ops) = repo.pending_ops().find(|ops| ops.staging())
            {
                self.single_staged_entry =
                    repo.status_for_path(&ops.repo_path)
                        .map(|status| GitStatusEntry {
                            repo_path: ops.repo_path.clone(),
                            status: status.status,
                            staging: StageStatus::Staged,
                        });
            }
        }

        if tracked_change_count == 1 {
            self.single_tracked_entry = last_tracked_index
                .and_then(|index| self.status_entries.get(index))
                .cloned();
        }

        self.update_counts(repo);
        cx.notify();
    }

    fn update_counts(&mut self, repo: &Repository) {
        self.conflicted_staged_count = 0;
        self.new_staged_count = 0;
        self.tracked_count = 0;
        self.tracked_staged_count = 0;

        for status_entry in &self.status_entries {
            let has_staged = GitPanel::stage_status_for_entry(status_entry, repo).has_staged();
            if repo.had_conflict_on_last_merge_head_change(&status_entry.repo_path) {
                if has_staged {
                    self.conflicted_staged_count += 1;
                }
            } else if status_entry.status.is_created() {
                if has_staged {
                    self.new_staged_count += 1;
                }
            } else {
                self.tracked_count += 1;
                if has_staged {
                    self.tracked_staged_count += 1;
                }
            }
        }
    }

    pub(crate) fn has_staged_changes(&self) -> bool {
        self.tracked_staged_count > 0
            || self.new_staged_count > 0
            || self.conflicted_staged_count > 0
    }

    pub(crate) fn has_unstaged_changes(&self) -> bool {
        self.status_entries
            .iter()
            .any(|entry| entry.staging.has_unstaged())
    }

    fn has_tracked_changes(&self) -> bool {
        self.tracked_count > 0
    }

    pub fn has_unstaged_conflicts(&self) -> bool {
        self.status_entries
            .iter()
            .any(|entry| entry.status.is_conflicted() && entry.staging.has_unstaged())
    }

    fn show_error_toast(&self, action: impl Into<SharedString>, e: anyhow::Error, cx: &mut App) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        show_error_toast(workspace, action, e, cx)
    }

    fn show_git_job_queue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.active_repository.as_ref() else {
            let workspace = self.workspace.clone();
            cx.defer(move |cx| {
                if let Some(workspace) = workspace.upgrade() {
                    workspace.update(cx, |workspace, cx| {
                        struct GitJobQueueToast;
                        workspace.show_toast(
                            workspace::Toast::new(
                                NotificationId::unique::<GitJobQueueToast>(),
                                "No active repository",
                            )
                            .autohide(),
                            cx,
                        );
                    });
                }
            });
            return;
        };

        let repo_path = repo.read(cx).work_directory_abs_path.display().to_string();
        let queue_value = repo.read(cx).job_debug_queue().to_debug_value();
        let title = format!("Git Job Queue: {repo_path}");

        let json_language = self.project.read(cx).languages().language_for_name("JSON");
        let project = self.project.clone();
        let workspace = self.workspace.clone();

        window
            .spawn(cx, async move |cx| {
                let json_language = json_language.await.ok();

                // Best-effort: gather runtime diagnostics off the main thread.
                // Any failure inside `gather` is logged and produces an empty
                // section; this `.await` itself cannot meaningfully fail and
                // must never prevent us from showing the queue dump.
                let diagnostics = cx
                    .background_spawn(crate::git_runtime_diagnostics::gather())
                    .await;

                let mut combined = queue_value;
                if let serde_json::Value::Object(ref mut map) = combined
                    && let serde_json::Value::Object(diag_map) = diagnostics
                    && !diag_map.is_empty()
                {
                    map.insert(
                        "runtime_diagnostics".into(),
                        serde_json::Value::Object(diag_map),
                    );
                }

                let text = serde_json::to_string_pretty(&combined).unwrap_or_default();

                let buffer = project
                    .update(cx, |project, cx| {
                        project.create_buffer(json_language, false, cx)
                    })
                    .await?;

                buffer.update(cx, |buffer, cx| {
                    buffer.set_text(text, cx);
                    buffer.set_capability(language::Capability::ReadWrite, cx);
                });

                workspace.update_in(cx, |workspace, window, cx| {
                    let buffer =
                        cx.new(|cx| MultiBuffer::singleton(buffer, cx).with_title(title.clone()));

                    workspace.add_item_to_active_pane(
                        Box::new(cx.new(|cx| {
                            let mut editor =
                                Editor::for_multibuffer(buffer, Some(project.clone()), window, cx);
                            editor.set_breadcrumb_header(title);
                            editor.disable_mouse_wheel_zoom();
                            editor
                        })),
                        None,
                        true,
                        window,
                        cx,
                    );
                })?;

                anyhow::Ok(())
            })
            .detach_and_log_err(cx);
    }

    fn show_remote_output(
        &mut self,
        action: RemoteAction,
        info: RemoteCommandOutput,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        workspace.update(cx, |workspace, cx| {
            let SuccessMessage { message, style } = remote_output::format_output(&action, info);
            let workspace_weak = cx.weak_entity();
            let operation = action.name();

            let status_toast = StatusToast::new(message, cx, move |this, _cx| {
                use remote_output::SuccessStyle::*;
                let this = this.icon(
                    Icon::new(IconName::GitBranch)
                        .size(IconSize::Small)
                        .color(Color::Muted),
                );
                match style {
                    Toast => this,
                    ToastWithLog { output } => this.action("View Log", move |window, cx| {
                        let output = output.clone();
                        let output =
                            format!("stdout:\n{}\nstderr:\n{}", output.stdout, output.stderr);
                        workspace_weak
                            .update(cx, move |workspace, cx| {
                                open_output(operation, workspace, &output, window, cx)
                            })
                            .ok();
                    }),
                }
                .dismiss_button(true)
            });
            workspace.toggle_status_toast(status_toast, cx)
        });
    }

    pub fn can_stage_all(&self) -> bool {
        self.has_unstaged_changes()
    }

    pub fn can_unstage_all(&self) -> bool {
        self.has_staged_changes()
    }

    pub fn configure_commit_button(&self, cx: &mut Context<Self>) -> (bool, &'static str) {
        self.configure_commit_button_for_index(false, cx)
    }

    pub(crate) fn configure_commit_button_for_index(
        &self,
        allow_empty_index: bool,
        cx: &mut Context<Self>,
    ) -> (bool, &'static str) {
        if self.has_unstaged_conflicts() {
            (false, "You must resolve conflicts before committing")
        } else if !allow_empty_index
            && !self.has_staged_changes()
            && !self.has_tracked_changes()
            && !self.amend_pending
        {
            (false, "No changes to commit")
        } else if self.pending_commit.is_some() {
            (false, "Commit in progress")
        } else if !self.has_commit_message(cx) {
            (false, "No commit message")
        } else if !self.has_write_access(cx) {
            (false, "You do not have write access to this project")
        } else {
            (true, self.commit_button_title())
        }
    }

    pub fn commit_button_title(&self) -> &'static str {
        if self.amend_pending {
            if self.has_staged_changes() {
                "Amend"
            } else if self.has_tracked_changes() {
                "Amend Tracked"
            } else {
                "Amend"
            }
        } else if self.has_staged_changes() {
            "Commit"
        } else {
            "Commit Tracked"
        }
    }

    pub(crate) fn render_remote_button(&self, cx: &App) -> Option<AnyElement> {
        let branch = self.active_repository.as_ref()?.read(cx).branch.clone();
        if !self.can_push_and_pull(cx) {
            return None;
        }
        Some(
            h_flex()
                .gap_1()
                .flex_shrink_0()
                .when_some(branch, |this, branch| {
                    let focus_handle = Some(self.focus_handle(cx));

                    this.children(render_remote_button(
                        "remote-button",
                        &branch,
                        focus_handle,
                        true,
                        self.pending_remote_operation,
                        self.remote_action_menu_handle.clone(),
                    ))
                })
                .into_any_element(),
        )
    }

    fn render_buffer_header_controls(
        &self,
        entity: &Entity<Self>,
        file: &Arc<dyn File>,
        _: &Window,
        cx: &App,
    ) -> Option<AnyElement> {
        let repo = self.active_repository.as_ref()?.read(cx);
        let project_path = (file.worktree_id(cx), file.path().clone()).into();
        let repo_path = repo.project_path_to_repo_path(&project_path, cx)?;
        let status = repo.status_for_path(&repo_path)?;
        let staging = status.status.staging();
        let is_staging_or_staged = repo
            .pending_ops_for_path(&repo_path)
            .map(|ops| !ops.last_op_errored() && (ops.staging() || ops.staged()))
            .or_else(|| staging.as_bool());

        let entry = GitStatusEntry {
            repo_path,
            status: status.status,
            staging,
        };
        let checkbox = Checkbox::new("stage-file", is_staging_or_staged.into())
            .disabled(!self.has_write_access(cx))
            .fill()
            .elevation(ElevationIndex::Surface)
            .on_click({
                let git_panel = entity.downgrade();
                move |_, _, cx| {
                    git_panel
                        .update(cx, |this, cx| {
                            this.toggle_staged_for_entry(&entry, cx);
                            cx.stop_propagation();
                        })
                        .ok();
                }
            });
        Some(
            h_flex()
                .id("start-slot")
                .text_lg()
                .child(checkbox)
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                    // prevent the list item active state triggering when toggling checkbox
                    cx.stop_propagation();
                })
                .into_any_element(),
        )
    }

    pub fn load_commit_details(
        &self,
        sha: String,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<CommitDetails>> {
        let Some(repo) = self.active_repository.clone() else {
            return Task::ready(Err(anyhow::anyhow!("no active repo")));
        };
        repo.update(cx, |repo, cx| repo.show_commit(sha, cx))
    }

    fn has_write_access(&self, cx: &App) -> bool {
        !self.project.read(cx).is_read_only(cx)
    }

    pub fn load_commit_template(
        &self,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<Option<GitCommitTemplate>>> {
        let Some(repo) = self.active_repository.clone() else {
            return Task::ready(Err(anyhow::anyhow!("no active repo")));
        };
        repo.update(cx, |repo, cx| {
            let rx = repo.load_commit_template_text();
            cx.spawn(async move |_, _| rx.await?)
        })
    }

    pub fn amend_pending(&self) -> bool {
        self.amend_pending
    }

    /// Sets the pending amend state, ensuring that the original commit message
    /// is either saved, when `value` is `true` and there's no pending amend, or
    /// restored, when `value` is `false` and there's a pending amend.
    pub fn set_amend_pending(&mut self, value: bool, cx: &mut Context<Self>) {
        if value && !self.amend_pending {
            let current_message = self.commit_message_buffer(cx).read(cx).text();
            self.original_commit_message = if current_message.trim().is_empty() {
                None
            } else {
                Some(current_message)
            };
        } else if !value && self.amend_pending {
            let message = self.original_commit_message.take().unwrap_or_default();
            self.commit_message_buffer(cx).update(cx, |buffer, cx| {
                let start = buffer.anchor_before(0);
                let end = buffer.anchor_after(buffer.len());
                buffer.edit([(start..end, message)], None, cx);
            });
        }

        self.amend_pending = value;
        self.serialize(cx);
        cx.notify();
    }

    pub fn signoff_enabled(&self) -> bool {
        self.signoff_enabled
    }

    pub fn skip_hooks_enabled(&self) -> bool {
        self.skip_hooks_enabled
    }

    fn set_skip_hooks_enabled(&mut self, value: bool, cx: &mut Context<Self>) {
        if self.skip_hooks_enabled != value {
            self.skip_hooks_enabled = value;
            cx.notify();
        }
    }

    pub fn toggle_skip_hooks(
        &mut self,
        _: &SkipHooks,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_skip_hooks_enabled(!self.skip_hooks_enabled, cx);
    }

    pub fn set_signoff_enabled(&mut self, value: bool, cx: &mut Context<Self>) {
        self.signoff_enabled = value;
        self.serialize(cx);
        cx.notify();
    }

    pub fn toggle_signoff_enabled(
        &mut self,
        _: &Signoff,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_signoff_enabled(!self.signoff_enabled, cx);
    }

    pub async fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> anyhow::Result<Entity<Self>> {
        let serialized_panel = match workspace
            .read_with(&cx, |workspace, cx| {
                Self::serialization_key(workspace).map(|key| (key, KeyValueStore::global(cx)))
            })
            .ok()
            .flatten()
        {
            Some((serialization_key, kvp)) => cx
                .background_spawn(async move { kvp.read_kvp(&serialization_key) })
                .await
                .context("loading git panel")
                .log_err()
                .flatten()
                .map(|panel| serde_json::from_str::<SerializedGitPanel>(&panel))
                .transpose()
                .log_err()
                .flatten(),
            None => None,
        };

        workspace.update_in(&mut cx, |workspace, window, cx| {
            GitPanel::new_with_serialized_panel(workspace, serialized_panel, window, cx)
        })
    }

    pub(crate) fn toggle_amend_pending(&mut self, cx: &mut Context<Self>) {
        self.set_amend_pending(!self.amend_pending, cx);
        if self.amend_pending {
            self.load_last_commit_message(cx);
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl GitPanel {
    pub fn take_status_update_for_test(&mut self) -> Task<()> {
        std::mem::replace(&mut self.update_status_entries_task, Task::ready(()))
    }

    pub fn new_test(
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Entity<Self> {
        Self::new(workspace, window, cx)
    }

    pub fn active_repository(&self) -> Option<&Entity<Repository>> {
        self.active_repository.as_ref()
    }
}

impl Render for GitPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("GitPanel")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close_panel))
            .size_full()
            .child(self.tool_window.clone())
    }
}

impl Focusable for GitPanel {
    fn focus_handle(&self, _cx: &App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<Event> for GitPanel {}

impl EventEmitter<PanelEvent> for GitPanel {}

pub(crate) struct GitPanelAddon {
    pub(crate) workspace: WeakEntity<Workspace>,
}

impl editor::Addon for GitPanelAddon {
    fn to_any(&self) -> &dyn std::any::Any {
        self
    }

    fn render_buffer_header_controls(
        &self,
        _excerpt_info: &ExcerptBoundaryInfo,
        buffer: &language::BufferSnapshot,
        window: &Window,
        cx: &App,
    ) -> Option<AnyElement> {
        let file = buffer.file()?;
        let git_panel = self.workspace.upgrade()?.read(cx).panel::<GitPanel>(cx)?;

        git_panel
            .read(cx)
            .render_buffer_header_controls(&git_panel, file, window, cx)
    }
}

impl Panel for GitPanel {
    fn activation_focus_handle(&self, cx: &App) -> FocusHandle {
        self.tool_window.focus_handle(cx)
    }

    fn persistent_name() -> &'static str {
        "GitPanel"
    }

    fn panel_key() -> &'static str {
        GIT_PANEL_KEY
    }

    fn position(&self, _: &Window, cx: &App) -> DockPosition {
        GitPanelSettings::get_global(cx).dock
    }

    fn position_is_valid(&self, _: DockPosition) -> bool {
        true
    }

    fn set_position(&mut self, position: DockPosition, _: &mut Window, cx: &mut Context<Self>) {
        settings::update_settings_file(self.fs.clone(), cx, move |settings, _| {
            settings.git_panel.get_or_insert_default().dock = Some(position.into())
        });
    }

    fn default_size(&self, window: &Window, cx: &App) -> Pixels {
        let settings = GitPanelSettings::get_global(cx);
        match self.position(window, cx) {
            DockPosition::Left | DockPosition::Right => settings.default_width,
            DockPosition::Bottom => settings.default_height,
        }
    }

    fn icon(&self, _: &Window, cx: &App) -> Option<ui::IconName> {
        GitPanelSettings::get_global(cx)
            .button
            .then_some(ui::IconName::GitBranch)
    }

    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        Some("Git")
    }

    fn icon_label(&self, _: &Window, cx: &App) -> Option<String> {
        if !GitPanelSettings::get_global(cx).show_count_badge {
            return None;
        }
        let total = self.changes_count;
        (total > 0).then(|| total.to_string())
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(ToggleFocus)
    }

    fn starts_open(&self, _: &Window, cx: &App) -> bool {
        GitPanelSettings::get_global(cx).starts_open
    }

    fn activation_priority(&self) -> u32 {
        2
    }

    fn hide_button_setting(&self, _: &App) -> Option<workspace::HideButtonSetting> {
        Some(workspace::HideButtonSetting::new(|settings| {
            settings.git_panel.get_or_insert_default().button = Some(false);
        }))
    }

    fn header_content(&self, _window: &Window, cx: &App) -> Option<AnyView> {
        Some(self.tool_window.read(cx).header_tabs())
    }
}

pub(crate) fn git_commit_editor_style(font_size: gpui::Pixels, cx: &App) -> EditorStyle {
    let settings = ThemeSettings::get_global(cx);

    EditorStyle {
        background: cx.theme().colors().editor_background,
        local_player: cx.theme().players().local(),
        text: TextStyle {
            color: cx.theme().colors().text,
            font_family: settings.buffer_font.family.clone(),
            font_fallbacks: settings.buffer_font.fallbacks.clone(),
            font_features: settings.buffer_font.features.clone(),
            font_size: AbsoluteLength::from(font_size),
            font_weight: settings.buffer_font.weight,
            line_height: (font_size * settings.buffer_line_height.value()).into(),
            ..Default::default()
        },
        syntax: cx.theme().syntax().clone(),
        ..Default::default()
    }
}

pub(crate) fn commit_title_exceeds_limit(title: &str, max_length: usize) -> bool {
    max_length > 0 && title.chars().count() > max_length
}

#[cfg(test)]
mod tests {
    use git::{repository::repo_path, status::StatusCode};
    use gpui::{Modifiers, TestAppContext, UpdateGlobal, VisualTestContext, px};
    use project::FakeFs;
    use serde_json::json;
    use settings::SettingsStore;
    use theme::LoadThemes;
    use util::path;
    use workspace::MultiWorkspace;

    use super::*;

    fn init_test(cx: &mut gpui::TestAppContext) {
        zlog::init_test();

        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(LoadThemes::JustBase, cx);
            editor::init(cx);
            crate::init(cx);
        });
    }

    fn register_git_commit_language(project: &Entity<Project>, cx: &mut VisualTestContext) {
        project.read_with(cx, |project, _| {
            project.languages().add(Arc::new(language::Language::new(
                language::LanguageConfig {
                    name: "Git Commit".into(),
                    ..Default::default()
                },
                None,
            )));
        });
    }

    async fn await_git_panel_entries(panel: &Entity<GitPanel>, cx: &mut VisualTestContext) {
        let handle = cx.update_window_entity(panel, |panel, _, _| {
            std::mem::replace(&mut panel.update_status_entries_task, Task::ready(()))
        });
        cx.executor().advance_clock(2 * UPDATE_DEBOUNCE);
        handle.await;
    }

    async fn setup_git_panel_with_changes(
        cx: &mut TestAppContext,
        tree: serde_json::Value,
        status_entries: &[(&str, git::status::StatusCode)],
    ) -> (
        Arc<FakeFs>,
        Entity<Project>,
        Entity<Workspace>,
        Entity<GitPanel>,
        VisualTestContext,
    ) {
        let fs = FakeFs::new(cx.background_executor.clone());
        fs.insert_tree(path!("/project"), tree).await;

        if !status_entries.is_empty() {
            fs.set_status_for_repo(
                path!("/project/.git").as_ref(),
                &status_entries
                    .iter()
                    .map(|(path, status)| (*path, status.worktree()))
                    .collect::<Vec<_>>(),
            );
        }

        let project = Project::test(fs.clone(), [Path::new(path!("/project"))], cx).await;
        let window_handle =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = window_handle
            .read_with(cx, |mw, _| mw.workspace().clone())
            .unwrap();
        let mut cx = VisualTestContext::from_window(window_handle.into(), cx);

        cx.read(|cx| {
            project
                .read(cx)
                .worktrees(cx)
                .next()
                .unwrap()
                .read(cx)
                .as_local()
                .unwrap()
                .scan_complete()
        })
        .await;

        cx.executor().run_until_parked();

        let panel = workspace.update_in(&mut cx, GitPanel::new);
        await_git_panel_entries(&panel, &mut cx).await;

        (fs, project, workspace, panel, cx)
    }

    #[gpui::test]
    async fn test_commit_modal_shows_the_intellij_commit_buttons(cx: &mut TestAppContext) {
        init_test(cx);
        let (_, _, workspace, panel, mut cx) = setup_git_panel_with_changes(
            cx,
            json!({
                ".git": {},
                "file": "modified\n",
            }),
            &[("file", StatusCode::Modified)],
        )
        .await;

        workspace.update_in(&mut cx, |workspace, window, cx| {
            workspace.add_panel(panel.clone(), window, cx);
            CommitModal::toggle(
                workspace,
                Some(crate::commit_modal::ForceMode::Commit),
                window,
                cx,
            );
        });
        cx.run_until_parked();

        assert!(
            cx.debug_bounds("commit-modal-commit-button").is_some(),
            "the footer shows the filled Commit button"
        );
        assert!(
            cx.debug_bounds("commit-modal-commit-and-push-button")
                .is_some(),
            "the footer shows the outlined Commit and Push… button"
        );
        assert!(
            cx.debug_bounds("commit-modal-options").is_some(),
            "the footer shows the commit options button"
        );
    }

    #[gpui::test]
    async fn test_commit_modal_hides_commit_and_push_in_batch_mode(cx: &mut TestAppContext) {
        init_test(cx);
        let (_, project, workspace, panel, mut cx) = setup_git_panel_with_changes(
            cx,
            json!({
                ".git": {},
                "file": "modified\n",
            }),
            &[("file", StatusCode::Modified)],
        )
        .await;

        workspace.update_in(&mut cx, |workspace, window, cx| {
            workspace.add_panel(panel.clone(), window, cx);
            CommitModal::toggle(
                workspace,
                Some(crate::commit_modal::ForceMode::Commit),
                window,
                cx,
            );
        });
        cx.run_until_parked();

        let modal = workspace
            .read_with(&cx, |workspace, cx| {
                workspace.active_modal::<CommitModal>(cx)
            })
            .expect("the commit modal is open");
        let repository = project
            .read_with(&cx, |project, cx| project.active_repository(cx))
            .expect("the project has an active repository");
        let (sender, _receiver) = futures::channel::oneshot::channel();
        modal.update(&mut cx, |modal, cx| {
            modal.set_completion(repository, sender);
            cx.notify();
        });
        cx.run_until_parked();

        assert!(cx.debug_bounds("commit-modal-commit-button").is_some());
        assert_eq!(
            cx.debug_bounds("commit-modal-commit-and-push-button"),
            None,
            "a batch commit has no push action to chain"
        );
    }

    #[gpui::test]
    async fn test_commit_and_push_without_a_commit_message_does_nothing(cx: &mut TestAppContext) {
        init_test(cx);
        let (fs, _, _, panel, mut cx) = setup_git_panel_with_changes(
            cx,
            json!({
                ".git": {},
                "file": "modified\n",
                "other": "modified\n",
            }),
            &[
                ("file", StatusCode::Modified),
                ("other", StatusCode::Modified),
            ],
        )
        .await;

        panel.update_in(&mut cx, |panel, window, cx| {
            panel.commit_and_push(window, cx);
        });
        cx.run_until_parked();

        let commit_count = fs
            .with_git_state(path!("/project/.git").as_ref(), false, |state| {
                state.commit_history.len()
            })
            .expect("fake repository should exist");
        assert_eq!(
            commit_count, 0,
            "an empty commit message must not start a commit, and no push may follow it"
        );
    }

    #[gpui::test]
    async fn test_skip_hooks_toggle(cx: &mut TestAppContext) {
        init_test(cx);
        let (_, _, _, panel, mut cx) = setup_git_panel_with_changes(
            cx,
            json!({
                ".git": {},
                "file": "modified\n",
            }),
            &[("file", StatusCode::Modified)],
        )
        .await;

        panel.update_in(&mut cx, |panel, window, cx| {
            assert!(!panel.skip_hooks_enabled());
            panel.toggle_skip_hooks(&SkipHooks, window, cx);
            assert!(panel.skip_hooks_enabled());
            assert!(panel.commit_options().no_verify);

            panel.toggle_skip_hooks(&SkipHooks, window, cx);
            assert!(!panel.skip_hooks_enabled());
            assert!(!panel.commit_options().no_verify);
        });
    }

    #[gpui::test]
    async fn test_git_panel_changes_count_skips_reverted_entries(cx: &mut TestAppContext) {
        init_test(cx);
        let (_, _, _, panel, mut cx) = setup_git_panel_with_changes(
            cx,
            json!({
                ".git": {},
                "file": "modified\n",
                "other": "modified\n",
            }),
            &[
                ("file", StatusCode::Modified),
                ("other", StatusCode::Modified),
            ],
        )
        .await;
        panel.read_with(&cx, |panel, _| {
            assert_eq!(panel.status_entries.len(), 2);
            assert_eq!(panel.changes_count, 2);
        });

        let repository = panel.read_with(&cx, |panel, _| {
            panel
                .active_repository
                .clone()
                .expect("the panel has an active repository")
        });
        repository
            .update(&mut cx, |repository, cx| {
                repository.checkout_files("HEAD", vec![repo_path("file")], cx)
            })
            .await
            .expect("reverting the file succeeds");
        cx.run_until_parked();

        panel.update(&mut cx, |panel, cx| panel.update_status_entries(cx));
        panel.read_with(&cx, |panel, _| {
            assert_eq!(panel.status_entries.len(), 1);
            assert_eq!(panel.changes_count, panel.status_entries.len());
        });
    }

    #[gpui::test]
    async fn test_commit_saves_edited_files_first_and_commits_them_like_intellij(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let fs = FakeFs::new(cx.background_executor.clone());
        fs.insert_tree(
            path!("/project"),
            json!({
                ".git": {},
                "changed.txt": "changed on disk\n",
                "edited.txt": "committed\n",
            }),
        )
        .await;
        let dot_git = Path::new(path!("/project/.git"));
        fs.set_head_and_index_for_repo(
            dot_git,
            &[
                ("changed.txt", "committed\n".into()),
                ("edited.txt", "committed\n".into()),
            ],
        );
        let project = Project::test(fs.clone(), [Path::new(path!("/project"))], cx).await;
        let window_handle =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = window_handle
            .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
            .unwrap();
        let mut cx = VisualTestContext::from_window(window_handle.into(), cx);
        project
            .update(&mut cx, |project, cx| project.git_scans_complete(cx))
            .await;
        cx.executor().run_until_parked();
        let panel = workspace.update_in(&mut cx, GitPanel::new);
        await_git_panel_entries(&panel, &mut cx).await;

        let edited_buffer = project
            .update(&mut cx, |project, cx| {
                project.open_local_buffer(path!("/project/edited.txt"), cx)
            })
            .await
            .unwrap();
        edited_buffer.update(&mut cx, |buffer, cx| {
            buffer.edit([(0..0, "edited in the editor\n")], None, cx)
        });

        panel.update_in(&mut cx, |panel, window, cx| {
            panel
                .commit_message_buffer(cx)
                .update(cx, |buffer, cx| buffer.set_text("Commit everything", cx));
            let focus_handle = panel.commit_editor.focus_handle(cx);
            focus_handle.focus(window, cx);
            assert!(panel.commit(&focus_handle, window, cx));
        });
        cx.run_until_parked();

        assert!(
            !edited_buffer.read_with(&cx, |buffer, _| buffer.is_dirty()),
            "the edited file is saved before the commit"
        );
        let head_contents = fs
            .with_git_state(dot_git, false, |state| state.head_contents.clone())
            .expect("fake repository should exist");
        assert_eq!(
            head_contents
                .get(&repo_path("edited.txt"))
                .map(|contents| String::from_utf8_lossy(contents).into_owned())
                .as_deref(),
            Some("edited in the editor\ncommitted\n"),
            "the commit contains what the editor showed"
        );
        assert_eq!(
            head_contents
                .get(&repo_path("changed.txt"))
                .map(|contents| String::from_utf8_lossy(contents).into_owned())
                .as_deref(),
            Some("changed on disk\n")
        );
        let repository = project.read_with(&cx, |project, cx| {
            project
                .active_repository(cx)
                .expect("the project has a repository")
        });
        repository.read_with(&cx, |repository, _| {
            assert_eq!(
                repository.cached_status().count(),
                0,
                "nothing looks uncommitted after the commit"
            );
        });
    }

    #[gpui::test]
    async fn test_skip_hooks_clears_after_successful_commit(cx: &mut TestAppContext) {
        init_test(cx);
        let (fs, _, _, panel, mut cx) = setup_git_panel_with_changes(
            cx,
            json!({
                ".git": {},
                "file": "modified\n",
            }),
            &[("file", StatusCode::Modified)],
        )
        .await;

        panel.update_in(&mut cx, |panel, window, cx| {
            panel
                .commit_message_buffer(cx)
                .update(cx, |buffer, cx| buffer.set_text("Test commit", cx));
            panel.toggle_skip_hooks(&SkipHooks, window, cx);
            assert!(panel.commit_options().no_verify);

            let focus_handle = panel.commit_editor.focus_handle(cx);
            focus_handle.focus(window, cx);
            assert!(panel.commit(&focus_handle, window, cx));
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, _| {
            assert!(!panel.skip_hooks_enabled());
        });
        let commit_count = fs
            .with_git_state(path!("/project/.git").as_ref(), false, |state| {
                state.commit_history.len()
            })
            .expect("fake repository should exist");
        assert_eq!(commit_count, 1);
    }

    #[gpui::test]
    async fn test_skip_hooks_remains_enabled_after_failed_commit(cx: &mut TestAppContext) {
        init_test(cx);
        let (fs, _, _, panel, mut cx) = setup_git_panel_with_changes(
            cx,
            json!({
                ".git": {},
                "file": "modified\n",
            }),
            &[("file", StatusCode::Modified)],
        )
        .await;

        fs.set_status_for_repo(
            path!("/project/.git").as_ref(),
            &[("file", StatusCode::Modified.index())],
        );
        cx.run_until_parked();
        await_git_panel_entries(&panel, &mut cx).await;
        fs.with_git_state(path!("/project/.git").as_ref(), false, |state| {
            state.index_contents = state.head_contents.clone();
        })
        .expect("fake repository should exist");

        panel.update_in(&mut cx, |panel, window, cx| {
            panel
                .commit_message_buffer(cx)
                .update(cx, |buffer, cx| buffer.set_text("Test commit", cx));
            panel.toggle_skip_hooks(&SkipHooks, window, cx);
            assert!(panel.commit_options().no_verify);

            let focus_handle = panel.commit_editor.focus_handle(cx);
            focus_handle.focus(window, cx);
            assert!(panel.commit(&focus_handle, window, cx));
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, _| {
            assert!(panel.skip_hooks_enabled());
        });
        let commit_count = fs
            .with_git_state(path!("/project/.git").as_ref(), false, |state| {
                state.commit_history.len()
            })
            .expect("fake repository should exist");
        assert_eq!(commit_count, 0);
    }

    #[gpui::test]
    async fn test_git_init_ignores_single_file_worktrees(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.background_executor.clone());
        fs.insert_tree(path!("/demo"), json!({ "plain.txt": "hello" }))
            .await;

        let project = Project::test(fs.clone(), [Path::new(path!("/demo/plain.txt"))], cx).await;
        let window_handle =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = window_handle
            .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
            .unwrap();
        let cx = &mut VisualTestContext::from_window(window_handle.into(), cx);
        cx.run_until_parked();

        let panel = workspace.update_in(cx, GitPanel::new);
        panel.update_in(cx, |panel, window, cx| panel.git_init(window, cx));
        cx.run_until_parked();

        assert!(
            cx.has_pending_prompt(),
            "git init should ask for a directory instead of running in a single-file worktree"
        );
        assert!(
            !fs.is_dir(Path::new(path!("/demo/plain.txt/.git"))).await,
            "git init should not run inside a single-file worktree"
        );
    }

    #[gpui::test]
    async fn test_entry_worktree_paths(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.background_executor.clone());
        fs.insert_tree(
            "/root",
            json!({
                "zed": {
                    ".git": {},
                    "crates": {
                        "gpui": {
                            "gpui.rs": "fn main() {}"
                        },
                        "util": {
                            "util.rs": "fn do_it() {}"
                        }
                    }
                },
            }),
        )
        .await;

        fs.set_status_for_repo(
            Path::new(path!("/root/zed/.git")),
            &[
                ("crates/gpui/gpui.rs", StatusCode::Modified.worktree()),
                ("crates/util/util.rs", StatusCode::Modified.worktree()),
            ],
        );
        let project =
            Project::test(fs.clone(), [path!("/root/zed/crates/gpui").as_ref()], cx).await;
        let window_handle =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = window_handle
            .read_with(cx, |mw, _| mw.workspace().clone())
            .unwrap();
        let cx = &mut VisualTestContext::from_window(window_handle.into(), cx);

        cx.read(|cx| {
            project
                .read(cx)
                .worktrees(cx)
                .next()
                .unwrap()
                .read(cx)
                .as_local()
                .unwrap()
                .scan_complete()
        })
        .await;

        cx.executor().run_until_parked();

        let panel = workspace.update_in(cx, GitPanel::new);
        await_git_panel_entries(&panel, cx).await;

        let status_entries = panel.read_with(cx, |panel, _| panel.status_entries.clone());
        pretty_assertions::assert_eq!(
            status_entries,
            [
                GitStatusEntry {
                    repo_path: repo_path("crates/gpui/gpui.rs"),
                    status: StatusCode::Modified.worktree(),
                    staging: StageStatus::Unstaged,
                },
                GitStatusEntry {
                    repo_path: repo_path("crates/util/util.rs"),
                    status: StatusCode::Modified.worktree(),
                    staging: StageStatus::Unstaged,
                },
            ],
        );
    }

    #[gpui::test]
    async fn test_amend_commit_message_handling(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.background_executor.clone());
        fs.insert_tree(
            "/root",
            json!({
                "project": {
                    ".git": {},
                    "src": {
                        "main.rs": "fn main() {}"
                    }
                }
            }),
        )
        .await;

        fs.set_status_for_repo(
            Path::new(path!("/root/project/.git")),
            &[("src/main.rs", StatusCode::Modified.worktree())],
        );

        let project = Project::test(fs.clone(), [Path::new(path!("/root/project"))], cx).await;
        let window_handle =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = window_handle
            .read_with(cx, |mw, _| mw.workspace().clone())
            .unwrap();
        let cx = &mut VisualTestContext::from_window(window_handle.into(), cx);

        let panel = workspace.update_in(cx, GitPanel::new);

        // Test: User has commit message, enables amend (saves message), then disables (restores message)
        panel.update(cx, |panel, cx| {
            panel.commit_message_buffer(cx).update(cx, |buffer, cx| {
                let start = buffer.anchor_before(0);
                let end = buffer.anchor_after(buffer.len());
                buffer.edit([(start..end, "Initial commit message")], None, cx);
            });

            panel.set_amend_pending(true, cx);
            assert!(panel.original_commit_message.is_some());

            panel.set_amend_pending(false, cx);
            let current_message = panel.commit_message_buffer(cx).read(cx).text();
            assert_eq!(current_message, "Initial commit message");
            assert!(panel.original_commit_message.is_none());
        });

        // Test: User has empty commit message, enables amend, then disables (clears message)
        panel.update(cx, |panel, cx| {
            panel.commit_message_buffer(cx).update(cx, |buffer, cx| {
                let start = buffer.anchor_before(0);
                let end = buffer.anchor_after(buffer.len());
                buffer.edit([(start..end, "")], None, cx);
            });

            panel.set_amend_pending(true, cx);
            assert!(panel.original_commit_message.is_none());

            panel.commit_message_buffer(cx).update(cx, |buffer, cx| {
                let start = buffer.anchor_before(0);
                let end = buffer.anchor_after(buffer.len());
                buffer.edit([(start..end, "Previous commit message")], None, cx);
            });

            panel.set_amend_pending(false, cx);
            let current_message = panel.commit_message_buffer(cx).read(cx).text();
            assert_eq!(current_message, "");
        });
    }

    #[gpui::test]
    async fn test_commit_message_restored_after_reconnect(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.background_executor.clone());
        fs.insert_tree(
            "/root",
            json!({
                "project-a": {
                    ".git": {},
                    "src": {
                        "main.rs": "fn main() {}"
                    }
                },
                "project-b": {
                    ".git": {},
                    "src": {
                        "main.rs": "fn main() {}"
                    }
                }
            }),
        )
        .await;

        fs.set_status_for_repo(
            Path::new(path!("/root/project-a/.git")),
            &[("src/main.rs", StatusCode::Modified.worktree())],
        );
        fs.set_status_for_repo(
            Path::new(path!("/root/project-b/.git")),
            &[("src/main.rs", StatusCode::Modified.worktree())],
        );

        let project = Project::test(
            fs.clone(),
            [
                Path::new(path!("/root/project-a")),
                Path::new(path!("/root/project-b")),
            ],
            cx,
        )
        .await;
        let (repository_a, repository_b) = project.read_with(cx, |project, cx| {
            let git_store = project.git_store().clone();
            let mut repository_a = None;
            let mut repository_b = None;
            for repository in git_store.read(cx).repositories().values() {
                let work_directory_abs_path = &repository.read(cx).work_directory_abs_path;
                if work_directory_abs_path.as_ref() == Path::new(path!("/root/project-a")) {
                    repository_a = Some(repository.clone());
                } else if work_directory_abs_path.as_ref() == Path::new(path!("/root/project-b")) {
                    repository_b = Some(repository.clone());
                }
            }
            (
                repository_a.expect("should have repository for project-a"),
                repository_b.expect("should have repository for project-b"),
            )
        });
        repository_a.update(cx, |repository, cx| repository.set_as_active_repository(cx));

        let window_handle =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = window_handle
            .read_with(cx, |mw, _| mw.workspace().clone())
            .unwrap();
        let cx = &mut VisualTestContext::from_window(window_handle.into(), cx);

        register_git_commit_language(&project, cx);
        let panel = workspace.update_in(cx, GitPanel::new);
        cx.run_until_parked();

        let message_a = "Restore repository A message";
        panel.update(cx, |panel, cx| {
            panel.commit_message_buffer(cx).update(cx, |buffer, cx| {
                let start = buffer.anchor_before(0);
                let end = buffer.anchor_after(buffer.len());
                buffer.edit([(start..end, message_a)], None, cx);
            });
        });

        repository_b.update(cx, |repository, cx| repository.set_as_active_repository(cx));
        cx.run_until_parked();

        let message_b = "Restore repository B message";
        let serialized_panel = panel.update(cx, |panel, cx| {
            panel.commit_message_buffer(cx).update(cx, |buffer, cx| {
                let start = buffer.anchor_before(0);
                let end = buffer.anchor_after(buffer.len());
                buffer.edit([(start..end, message_b)], None, cx);
            });

            SerializedGitPanel {
                signoff_enabled: false,
                commit_messages: panel.serialized_commit_messages(cx),
            }
        });

        for repository in [&repository_a, &repository_b] {
            let buffer = repository.read_with(cx, |repository, _| {
                repository
                    .commit_message_buffer()
                    .expect("repository commit message buffer should be open")
                    .clone()
            });
            buffer.update(cx, |buffer, cx| {
                let start = buffer.anchor_before(0);
                let end = buffer.anchor_after(buffer.len());
                buffer.edit([(start..end, "")], None, cx);
            });
        }

        let restored_panel = workspace.update_in(cx, |workspace, window, cx| {
            GitPanel::new_with_serialized_panel(workspace, Some(serialized_panel), window, cx)
        });
        cx.run_until_parked();

        restored_panel.read_with(cx, |panel, cx| {
            assert_eq!(panel.commit_message_buffer(cx).read(cx).text(), message_b);
        });

        repository_a.update(cx, |repository, cx| repository.set_as_active_repository(cx));
        cx.run_until_parked();

        restored_panel.read_with(cx, |panel, cx| {
            assert_eq!(panel.commit_message_buffer(cx).read(cx).text(), message_a);
        });

        restored_panel.update(cx, |panel, cx| {
            panel.commit_message_buffer(cx).update(cx, |buffer, cx| {
                let start = buffer.anchor_before(0);
                let end = buffer.anchor_after(buffer.len());
                buffer.edit([(start..end, "")], None, cx);
            });
        });

        let mismatched_serialized_panel = SerializedGitPanel {
            signoff_enabled: false,
            commit_messages: BTreeMap::from_iter([(
                path!("/root/other-project").to_string(),
                SerializedCommitMessage {
                    message: Some(message_a.to_string()),
                    original_message: None,
                    ..Default::default()
                },
            )]),
        };
        let mismatched_panel = workspace.update_in(cx, |workspace, window, cx| {
            GitPanel::new_with_serialized_panel(
                workspace,
                Some(mismatched_serialized_panel),
                window,
                cx,
            )
        });
        cx.run_until_parked();

        mismatched_panel.read_with(cx, |panel, cx| {
            // The draft is not restored because the serialized work directory
            // does not match the active repository, so it cannot leak across
            // repositories.
            assert_eq!(panel.commit_message_buffer(cx).read(cx).text(), "");
        });
    }

    #[gpui::test]
    async fn test_commit_template_applied_fresh_after_template_file_change(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let fs = FakeFs::new(cx.background_executor.clone());
        fs.insert_tree(
            "/root",
            json!({
                "project": {
                    ".git": {},
                    "src": { "main.rs": "fn main() {}" }
                }
            }),
        )
        .await;
        fs.set_status_for_repo(
            Path::new(path!("/root/project/.git")),
            &[("src/main.rs", StatusCode::Modified.worktree())],
        );
        fs.with_git_state(Path::new(path!("/root/project/.git")), false, |state| {
            state.commit_template = Some(GitCommitTemplate {
                template: "Template1\n".to_string(),
            })
        })
        .unwrap();

        let project = Project::test(fs.clone(), [Path::new(path!("/root/project"))], cx).await;
        let repository = project.read_with(cx, |project, cx| {
            project
                .git_store()
                .read(cx)
                .repositories()
                .values()
                .next()
                .unwrap()
                .clone()
        });
        repository.update(cx, |repository, cx| repository.set_as_active_repository(cx));

        let window_handle =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = window_handle
            .read_with(cx, |mw, _| mw.workspace().clone())
            .unwrap();
        let cx = &mut VisualTestContext::from_window(window_handle.into(), cx);
        register_git_commit_language(&project, cx);

        let panel = workspace.update_in(cx, GitPanel::new);
        cx.run_until_parked();

        panel.read_with(cx, |panel, cx| {
            assert_eq!(
                panel.commit_message_buffer(cx).read(cx).text(),
                "Template1\n",
                "template should be applied initially"
            );
        });

        // Update the commit template returned by the fake git repository and
        // simulate restart: empty the active commit buffer and reload the panel
        // from the same serialized state the previous session would have written.
        fs.with_git_state(Path::new(path!("/root/project/.git")), false, |state| {
            state.commit_template = Some(GitCommitTemplate {
                template: "Template2\n".to_string(),
            })
        })
        .unwrap();
        let serialized_panel = panel.update(cx, |panel, cx| SerializedGitPanel {
            signoff_enabled: false,
            commit_messages: panel.serialized_commit_messages(cx),
        });
        let buffer = repository.read_with(cx, |repository, _| {
            repository.commit_message_buffer().unwrap().clone()
        });
        buffer.update(cx, |buffer, cx| {
            let start = buffer.anchor_before(0);
            let end = buffer.anchor_after(buffer.len());
            buffer.edit([(start..end, "")], None, cx);
        });

        let restored_panel = workspace.update_in(cx, |workspace, window, cx| {
            GitPanel::new_with_serialized_panel(workspace, Some(serialized_panel), window, cx)
        });
        cx.run_until_parked();

        restored_panel.read_with(cx, |panel, cx| {
            assert_eq!(
                panel.commit_message_buffer(cx).read(cx).text(),
                "Template2\n",
                "updated commit template should be re-applied across restarts"
            );
        });
    }

    #[gpui::test]
    async fn test_user_draft_preserved_when_template_changes(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.background_executor.clone());
        fs.insert_tree(
            "/root",
            json!({
                "project": {
                    ".git": {},
                    "src": { "main.rs": "fn main() {}" }
                }
            }),
        )
        .await;
        fs.set_status_for_repo(
            Path::new(path!("/root/project/.git")),
            &[("src/main.rs", StatusCode::Modified.worktree())],
        );
        fs.with_git_state(Path::new(path!("/root/project/.git")), false, |state| {
            state.commit_template = Some(GitCommitTemplate {
                template: "Template1\n".to_string(),
            })
        })
        .unwrap();

        let project = Project::test(fs.clone(), [Path::new(path!("/root/project"))], cx).await;
        let repository = project.read_with(cx, |project, cx| {
            project
                .git_store()
                .read(cx)
                .repositories()
                .values()
                .next()
                .unwrap()
                .clone()
        });
        repository.update(cx, |repository, cx| repository.set_as_active_repository(cx));

        let window_handle =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = window_handle
            .read_with(cx, |mw, _| mw.workspace().clone())
            .unwrap();
        let cx = &mut VisualTestContext::from_window(window_handle.into(), cx);
        register_git_commit_language(&project, cx);

        let panel = workspace.update_in(cx, GitPanel::new);
        cx.run_until_parked();

        // User edits the buffer after the template was applied.
        let user_message = "fix: my actual commit\n";
        panel.update(cx, |panel, cx| {
            panel.commit_message_buffer(cx).update(cx, |buffer, cx| {
                let start = buffer.anchor_before(0);
                let end = buffer.anchor_after(buffer.len());
                buffer.edit([(start..end, user_message)], None, cx);
            });
        });

        // The commit template returned by the fake git repository changes
        // after the user has already started typing. We must not clobber the
        // user's draft.
        fs.with_git_state(Path::new(path!("/root/project/.git")), false, |state| {
            state.commit_template = Some(GitCommitTemplate {
                template: "Template2\n".to_string(),
            })
        })
        .unwrap();
        let serialized_panel = panel.update(cx, |panel, cx| SerializedGitPanel {
            signoff_enabled: false,
            commit_messages: panel.serialized_commit_messages(cx),
        });

        // Simulate a restart and restore from the serialized state.
        let buffer = repository.read_with(cx, |repository, _| {
            repository.commit_message_buffer().unwrap().clone()
        });
        buffer.update(cx, |buffer, cx| {
            let start = buffer.anchor_before(0);
            let end = buffer.anchor_after(buffer.len());
            buffer.edit([(start..end, "")], None, cx);
        });

        let restored_panel = workspace.update_in(cx, |workspace, window, cx| {
            GitPanel::new_with_serialized_panel(workspace, Some(serialized_panel), window, cx)
        });
        cx.run_until_parked();

        restored_panel.read_with(cx, |panel, cx| {
            assert_eq!(
                panel.commit_message_buffer(cx).read(cx).text(),
                user_message,
                "user-typed draft must be restored verbatim, never overwritten by a later template"
            );
        });
    }

    #[gpui::test]
    async fn test_pending_commit_state_is_per_repository(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.background_executor.clone());
        fs.insert_tree(
            "/root",
            json!({
                "project-a": {
                    ".git": {},
                    "src": {
                        "main.rs": "fn main() {}"
                    }
                },
                "project-b": {
                    ".git": {},
                    "src": {
                        "main.rs": "fn main() {}"
                    }
                }
            }),
        )
        .await;

        fs.set_status_for_repo(
            Path::new(path!("/root/project-a/.git")),
            &[("src/main.rs", StatusCode::Modified.worktree())],
        );
        fs.set_status_for_repo(
            Path::new(path!("/root/project-b/.git")),
            &[("src/main.rs", StatusCode::Modified.worktree())],
        );

        let project = Project::test(
            fs.clone(),
            [
                Path::new(path!("/root/project-a")),
                Path::new(path!("/root/project-b")),
            ],
            cx,
        )
        .await;
        let (repository_a, repository_b) = project.read_with(cx, |project, cx| {
            let git_store = project.git_store().clone();
            let mut repository_a = None;
            let mut repository_b = None;
            for repository in git_store.read(cx).repositories().values() {
                let work_directory_abs_path = &repository.read(cx).work_directory_abs_path;
                if work_directory_abs_path.as_ref() == Path::new(path!("/root/project-a")) {
                    repository_a = Some(repository.clone());
                } else if work_directory_abs_path.as_ref() == Path::new(path!("/root/project-b")) {
                    repository_b = Some(repository.clone());
                }
            }
            (
                repository_a.expect("should have repository for project-a"),
                repository_b.expect("should have repository for project-b"),
            )
        });
        repository_a.update(cx, |repository, cx| repository.set_as_active_repository(cx));

        let window_handle =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = window_handle
            .read_with(cx, |mw, _| mw.workspace().clone())
            .unwrap();
        let cx = &mut VisualTestContext::from_window(window_handle.into(), cx);

        register_git_commit_language(&project, cx);
        let panel = workspace.update_in(cx, GitPanel::new);
        cx.run_until_parked();

        // Enter an amend on repository A, then simulate the amend flow loading
        // the last commit message into the editor.
        panel.update(cx, |panel, cx| {
            panel.commit_message_buffer(cx).update(cx, |buffer, cx| {
                let start = buffer.anchor_before(0);
                let end = buffer.anchor_after(buffer.len());
                buffer.edit([(start..end, "Draft for A")], None, cx);
            });
            panel.set_amend_pending(true, cx);
            panel.commit_message_buffer(cx).update(cx, |buffer, cx| {
                let start = buffer.anchor_before(0);
                let end = buffer.anchor_after(buffer.len());
                buffer.edit([(start..end, "Amended message")], None, cx);
            });
            assert!(panel.amend_pending());
            panel.set_skip_hooks_enabled(true, cx);
            assert!(panel.skip_hooks_enabled());
        });

        // Switching the active repository away exits the amend state instead of
        // carrying it over to repository B.
        repository_b.update(cx, |repository, cx| repository.set_as_active_repository(cx));
        cx.run_until_parked();

        panel.update(cx, |panel, cx| {
            assert!(!panel.amend_pending());
            assert!(!panel.skip_hooks_enabled());
            // Only the active repository may serialize a pending amend, and we
            // just left repository A's amend, so nothing is left pending.
            let serialized = panel.serialized_commit_messages(cx);
            assert!(serialized.values().all(|message| !message.amend_pending));
        });

        // Repository A's pre-amend draft is restored, discarding the amend edit.
        let buffer_a = repository_a.read_with(cx, |repository, _| {
            repository
                .commit_message_buffer()
                .expect("repository commit message buffer should be open")
                .clone()
        });
        buffer_a.read_with(cx, |buffer, _| {
            assert_eq!(buffer.text(), "Draft for A");
        });
    }

    #[gpui::test]
    async fn test_amend(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.background_executor.clone());
        fs.insert_tree(
            "/root",
            json!({
                "project": {
                    ".git": {},
                    "src": {
                        "main.rs": "fn main() {}"
                    }
                }
            }),
        )
        .await;

        fs.set_status_for_repo(
            Path::new(path!("/root/project/.git")),
            &[("src/main.rs", StatusCode::Modified.worktree())],
        );

        let project = Project::test(fs.clone(), [Path::new(path!("/root/project"))], cx).await;
        let window_handle =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = window_handle
            .read_with(cx, |mw, _| mw.workspace().clone())
            .unwrap();
        let cx = &mut VisualTestContext::from_window(window_handle.into(), cx);

        // Wait for the project scanning to finish so that `head_commit(cx)` is
        // actually set, otherwise no head commit would be available from which
        // to fetch the latest commit message from.
        cx.executor().run_until_parked();

        let panel = workspace.update_in(cx, GitPanel::new);
        panel.read_with(cx, |panel, cx| {
            assert!(panel.active_repository.is_some());
            assert!(panel.head_commit(cx).is_some());
        });

        panel.update_in(cx, |panel, window, cx| {
            // Update the commit editor's message to ensure that its contents
            // are later restored, after amending is finished.
            panel.commit_message_buffer(cx).update(cx, |buffer, cx| {
                buffer.set_text("refactor: update main.rs", cx);
            });

            // Start amending the previous commit.
            let focus_handle = panel.commit_editor.focus_handle(cx);
            focus_handle.focus(window, cx);
            panel.amend(&focus_handle, window, cx);
        });

        // Since `GitPanel.amend` attempts to fetch the latest commit message in
        // a background task, we need to wait for it to complete before being
        // able to assert that the commit message editor's state has been
        // updated.
        cx.run_until_parked();

        panel.update_in(cx, |panel, window, cx| {
            assert_eq!(
                panel.commit_message_buffer(cx).read(cx).text(),
                "initial commit"
            );
            assert_eq!(
                panel.original_commit_message,
                Some("refactor: update main.rs".to_string())
            );

            // Finish amending the previous commit.
            let focus_handle = panel.commit_editor.focus_handle(cx);
            focus_handle.focus(window, cx);
            panel.amend(&focus_handle, window, cx);
        });

        // Since the actual commit logic is run in a background task, we need to
        // await its completion to actually ensure that the commit message
        // editor's contents are set to the original message and haven't been
        // cleared.
        cx.run_until_parked();

        panel.update_in(cx, |panel, _window, cx| {
            // After amending, the commit editor's message should be restored to
            // the original message.
            assert_eq!(
                panel.commit_message_buffer(cx).read(cx).text(),
                "refactor: update main.rs"
            );
            assert!(panel.original_commit_message.is_none());
        });
    }

    #[gpui::test]
    async fn test_remote_operation_serialization(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.background_executor.clone());
        fs.insert_tree(
            path!("/project"),
            json!({
                ".git": {},
            }),
        )
        .await;

        let project = Project::test(fs.clone(), [Path::new(path!("/project"))], cx).await;
        let window_handle =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = window_handle
            .read_with(cx, |mw, _| mw.workspace().clone())
            .unwrap();
        let cx = &mut VisualTestContext::from_window(window_handle.into(), cx);
        let panel = workspace.update_in(cx, GitPanel::new);

        panel.update(cx, |panel, cx| {
            // The first remote operation starts and records its kind, which the
            // button uses to render an "in progress" tooltip.
            assert!(panel.start_remote_operation(RemoteOperationKind::Fetch, cx));
            assert!(matches!(
                panel.pending_remote_operation,
                Some(RemoteOperationKind::Fetch)
            ));

            // A second remote operation is refused while one is pending, even a
            // different kind: we serialize all remote ops.
            assert!(!panel.start_remote_operation(RemoteOperationKind::Push, cx));

            // Clearing the pending operation re-opens the gate.
            panel.clear_remote_operation(cx);
            assert!(panel.pending_remote_operation.is_none());
            assert!(panel.start_remote_operation(RemoteOperationKind::Pull, cx));
        });
    }

    #[gpui::test]
    async fn test_suggest_commit_message(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.background_executor.clone());
        fs.insert_tree(
            path!("/project"),
            json!({
                ".git": {},
                "tracked": "tracked\n",
                "untracked": "\n",
            }),
        )
        .await;

        fs.set_head_and_index_for_repo(
            path!("/project/.git").as_ref(),
            &[("tracked", "old tracked\n".into())],
        );

        let project = Project::test(fs.clone(), [Path::new(path!("/project"))], cx).await;
        let window_handle =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = window_handle
            .read_with(cx, |mw, _| mw.workspace().clone())
            .unwrap();
        let cx = &mut VisualTestContext::from_window(window_handle.into(), cx);
        let panel = workspace.update_in(cx, GitPanel::new);

        let handle = cx.update_window_entity(&panel, |panel, _, _| {
            std::mem::replace(&mut panel.update_status_entries_task, Task::ready(()))
        });
        cx.executor().advance_clock(2 * UPDATE_DEBOUNCE);
        handle.await;

        let entries = panel.read_with(cx, |panel, _| panel.status_entries.clone());

        // GitPanel
        // - Tracked:
        // - [] tracked
        // - Untracked
        // - [] untracked
        //
        // The commit message should now read:
        // "Update tracked"
        let message = panel.update(cx, |panel, cx| panel.suggest_commit_message(cx));
        assert_eq!(message, Some("Update tracked".to_string()));

        let first_status_entry = entries[0].clone();
        panel.update(cx, |panel, cx| {
            panel.toggle_staged_for_entry(&first_status_entry, cx);
        });

        cx.read(|cx| {
            project
                .read(cx)
                .worktrees(cx)
                .next()
                .unwrap()
                .read(cx)
                .as_local()
                .unwrap()
                .scan_complete()
        })
        .await;

        cx.executor().run_until_parked();

        let handle = cx.update_window_entity(&panel, |panel, _, _| {
            std::mem::replace(&mut panel.update_status_entries_task, Task::ready(()))
        });
        cx.executor().advance_clock(2 * UPDATE_DEBOUNCE);
        handle.await;

        // GitPanel
        // - Tracked:
        // - [x] tracked
        // - Untracked
        // - [] untracked
        //
        // The commit message should still read:
        // "Update tracked"
        let message = panel.update(cx, |panel, cx| panel.suggest_commit_message(cx));
        assert_eq!(message, Some("Update tracked".to_string()));

        let second_status_entry = entries[1].clone();
        panel.update(cx, |panel, cx| {
            panel.toggle_staged_for_entry(&second_status_entry, cx);
        });

        cx.read(|cx| {
            project
                .read(cx)
                .worktrees(cx)
                .next()
                .unwrap()
                .read(cx)
                .as_local()
                .unwrap()
                .scan_complete()
        })
        .await;

        cx.executor().run_until_parked();

        let handle = cx.update_window_entity(&panel, |panel, _, _| {
            std::mem::replace(&mut panel.update_status_entries_task, Task::ready(()))
        });
        cx.executor().advance_clock(2 * UPDATE_DEBOUNCE);
        handle.await;

        // GitPanel
        // - Tracked:
        // - [x] tracked
        // - Untracked
        // - [x] untracked
        //
        // The commit message should now read:
        // "Enter commit message"
        // (which means we should see None returned).
        let message = panel.update(cx, |panel, cx| panel.suggest_commit_message(cx));
        assert!(message.is_none());

        panel.update(cx, |panel, cx| {
            panel.toggle_staged_for_entry(&first_status_entry, cx);
        });

        cx.read(|cx| {
            project
                .read(cx)
                .worktrees(cx)
                .next()
                .unwrap()
                .read(cx)
                .as_local()
                .unwrap()
                .scan_complete()
        })
        .await;

        cx.executor().run_until_parked();

        let handle = cx.update_window_entity(&panel, |panel, _, _| {
            std::mem::replace(&mut panel.update_status_entries_task, Task::ready(()))
        });
        cx.executor().advance_clock(2 * UPDATE_DEBOUNCE);
        handle.await;

        // GitPanel
        // - Tracked:
        // - [] tracked
        // - Untracked
        // - [x] untracked
        //
        // The commit message should now read:
        // "Update untracked"
        let message = panel.update(cx, |panel, cx| panel.suggest_commit_message(cx));
        assert_eq!(message, Some("Create untracked".to_string()));

        panel.update(cx, |panel, cx| {
            panel.toggle_staged_for_entry(&second_status_entry, cx);
        });

        cx.read(|cx| {
            project
                .read(cx)
                .worktrees(cx)
                .next()
                .unwrap()
                .read(cx)
                .as_local()
                .unwrap()
                .scan_complete()
        })
        .await;

        cx.executor().run_until_parked();

        let handle = cx.update_window_entity(&panel, |panel, _, _| {
            std::mem::replace(&mut panel.update_status_entries_task, Task::ready(()))
        });
        cx.executor().advance_clock(2 * UPDATE_DEBOUNCE);
        handle.await;

        // GitPanel
        // - Tracked:
        // - [] tracked
        // - Untracked
        // - [] untracked
        //
        // The commit message should now read:
        // "Update tracked"
        let message = panel.update(cx, |panel, cx| panel.suggest_commit_message(cx));
        assert_eq!(message, Some("Update tracked".to_string()));
    }

    #[test]
    fn test_git_output_handler_strips_ansi_codes() {
        let cases = [
            ("no escape codes here\n", "no escape codes here\n"),
            ("\x1b[31mhello\x1b[0m", "hello"),
            ("\x1b[1;32mfoo\x1b[0m bar", "foo bar"),
            ("progress 10%\rprogress 100%\n", "progress 100%\n"),
        ];

        for (input, expected) in cases {
            assert_eq!(terminal::strip_ansi_text(input.as_bytes()), expected);
        }
    }

    #[test]
    fn test_commit_title_exceeds_limit() {
        // ASCII only
        let within_ascii = "abcde";
        let exceeds_ascii = "abcdef";
        assert!(!commit_title_exceeds_limit(within_ascii, 5));
        assert!(commit_title_exceeds_limit(exceeds_ascii, 5));

        // Multi-byte characters are counted as grapheme clusters
        let within_japanese = "あいうえお"; // 5 chars, 15 bytes
        let exceeds_japanese = "あいうえおか"; // 6 chars, 18 bytes
        assert!(!commit_title_exceeds_limit(within_japanese, 5));
        assert!(commit_title_exceeds_limit(exceeds_japanese, 5));

        // Mixed ASCII + multi-byte
        let within_mixed = "abcあ";
        let exceeds_mixed = "abcああ";
        assert!(!commit_title_exceeds_limit(within_mixed, 4));
        assert!(commit_title_exceeds_limit(exceeds_mixed, 4));

        // Emoji counts as one character each
        let within_emoji = "🚀";
        let exceeds_emoji = "🚀🚀";
        assert!(!commit_title_exceeds_limit(within_emoji, 1));
        assert!(commit_title_exceeds_limit(exceeds_emoji, 1));

        // A max_length of 0 disables the limit check
        assert!(!commit_title_exceeds_limit(
            "anything goes when disabled",
            0
        ));
        assert!(!commit_title_exceeds_limit("", 0));

        // Empty title never exceeds a positive limit
        assert!(!commit_title_exceeds_limit("", 72));
    }

    #[gpui::test]
    async fn test_commit_editor_context_menu_clipboard_actions(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.background_executor.clone());
        let project = Project::test(fs, [], cx).await;

        let window_handle = cx.add_window(|window, cx| {
            let buffer = cx.new(|cx| Buffer::local("commit message", cx));
            commit_message_editor(buffer, project.clone(), window, cx)
        });
        let editor = window_handle.root(cx).expect("commit editor should exist");
        let cx = &mut VisualTestContext::from_window(window_handle.into(), cx);
        editor.update_in(cx, |editor, window, cx| {
            editor.focus_handle(cx).focus(window, cx);
            editor.select_all(&Default::default(), window, cx);
        });
        cx.run_until_parked();

        let position = editor.read_with(cx, |editor, _| {
            editor
                .last_bounds()
                .expect("editor should be rendered")
                .origin
                + gpui::point(px(10.), px(10.))
        });
        let open_menu = |cx: &mut VisualTestContext| {
            cx.simulate_mouse_down(position, gpui::MouseButton::Right, Modifiers::none());
            // MouseContextMenu waits two frames before focusing its deferred element.
            for _ in 0..2 {
                cx.update(|window, cx| {
                    window.simulate_next_frame(cx);
                });
            }
            editor.update_in(cx, |editor, window, cx| {
                assert!(editor.has_mouse_context_menu());
                assert!(editor.mouse_menu_is_focused(window, cx));
            });
        };
        open_menu(cx);
        editor.update_in(cx, |editor, window, cx| {
            assert!(
                editor.mouse_menu_is_focused(window, cx),
                "menu should have focus"
            );
            assert!(
                editor.has_non_empty_selection(&editor.display_snapshot(cx)),
                "selection should remain"
            );
        });
        cx.dispatch_action(menu::SelectFirst);
        cx.dispatch_action(menu::SelectNext);
        cx.dispatch_action(menu::Confirm);
        assert!(
            editor.read_with(cx, |editor, _| !editor.has_mouse_context_menu()),
            "menu should close after Copy"
        );
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("commit message".to_string())
        );

        cx.write_to_clipboard(gpui::ClipboardItem::new_string("replacement".to_string()));
        open_menu(cx);
        cx.dispatch_action(menu::SelectLast);
        cx.dispatch_action(menu::Confirm);
        assert_eq!(
            editor.read_with(cx, |editor, cx| editor.text(cx)),
            "replacement"
        );

        cx.write_to_clipboard(gpui::ClipboardItem::new_string(" suffix".to_string()));
        open_menu(cx);
        cx.dispatch_action(menu::SelectFirst);
        cx.dispatch_action(menu::Confirm);
        assert_eq!(
            editor.read_with(cx, |editor, cx| editor.text(cx)),
            "replacement suffix",
            "with no selection, menu navigation should skip disabled Cut and Copy and select Paste"
        );
        editor.update_in(cx, |editor, window, cx| {
            editor.select_all(&Default::default(), window, cx);
        });
        open_menu(cx);
        cx.dispatch_action(menu::SelectFirst);
        cx.dispatch_action(menu::Confirm);
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("replacement suffix".to_string())
        );
        assert_eq!(editor.read_with(cx, |editor, cx| editor.text(cx)), "");
        assert!(editor.read_with(cx, |editor, _| !editor.has_mouse_context_menu()));
        editor.update_in(cx, |editor, window, _| {
            assert!(editor.is_focused(window));
        });
    }

    #[gpui::test]
    async fn test_default_size_follows_dock_axis(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.background_executor.clone());
        let project = Project::test(fs, [], cx).await;
        let window_handle =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = window_handle
            .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
            .unwrap();
        let cx = &mut VisualTestContext::from_window(window_handle.into(), cx);
        let panel = workspace.update_in(cx, GitPanel::new);

        for (position, expected_size) in [
            (DockPosition::Bottom, px(320.)),
            (DockPosition::Left, px(360.)),
            (DockPosition::Right, px(360.)),
        ] {
            cx.update(|_window, cx| {
                SettingsStore::update_global(cx, |store, cx| {
                    store.update_user_settings(cx, |settings| {
                        settings.git_panel.get_or_insert_default().dock = Some(position.into());
                    });
                });
            });
            panel.update_in(cx, |panel, window, cx| {
                assert!(panel.position_is_valid(position));
                assert_eq!(panel.position(window, cx), position);
                assert_eq!(panel.default_size(window, cx), expected_size);
            });
        }
    }
}
