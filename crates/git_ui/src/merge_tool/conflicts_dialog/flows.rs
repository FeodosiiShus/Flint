use std::collections::VecDeque;
use std::sync::{Arc, atomic::AtomicBool};

use anyhow::{Result, anyhow};
use git::repository::{ConflictSide, RepoPath, split_conflicts_for_resolution};
use gpui::{
    AnyWindowHandle, App, AsyncApp, Context, Entity, PromptLevel, Task, WeakEntity, Window,
};
use merge_diff::Side;
use project::{Project, git_store::Repository};
use util::ResultExt as _;

use super::ConflictsDialog;
use super::messages;
use super::model_loading::{
    ModelContext, PreparedModel, Unsupported, prepare_model, read_working_tree_text,
};
use super::ordering::sorted_files;
use super::overlay::ProgressOverlay;
use super::pane_titles::{TitleContext, merge_pane_titles};
use super::state::{ModelSummary, TreeStateStrategy};
use crate::merge_tool::dialog_window::{CloseDecision, DialogOwner, close_dialog_window};
use crate::merge_tool::merge_model::{MergeConflictModel, MergeModelError};
use crate::merge_tool::merge_window::{MergeRequest, MergeResult, open_merge_window};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Resolution {
    AcceptedYours,
    AcceptedTheirs,
    Merged,
}

pub(super) fn resolution_for_chosen_side(side: Option<Side>) -> Resolution {
    match side {
        Some(Side::Left) => Resolution::AcceptedYours,
        Some(Side::Right) => Resolution::AcceptedTheirs,
        None => Resolution::Merged,
    }
}

pub(super) fn conflict_side_for(resolution: Resolution, reversed: bool) -> Option<ConflictSide> {
    match resolution {
        Resolution::Merged => None,
        Resolution::AcceptedYours => Some(ConflictSide::for_accepted_version(false, reversed)),
        Resolution::AcceptedTheirs => Some(ConflictSide::for_accepted_version(true, reversed)),
    }
}

fn chosen_side_after_merge(result: MergeResult, previous: Option<Side>) -> Option<Side> {
    match result {
        MergeResult::Cancel => previous,
        MergeResult::Left => Some(Side::Left),
        MergeResult::Right => Some(Side::Right),
        MergeResult::Resolved => None,
    }
}

fn summarize(model: &MergeConflictModel) -> ModelSummary {
    ModelSummary {
        total: model.changes().len(),
        resolved: model.resolved_changes().len(),
        unresolved: model.unresolved_changes().len(),
        auto_resolvable: model.auto_resolvable_changes().len(),
        reviewed: model.was_reviewed(),
        chosen_side: model.chosen_side(),
    }
}

fn write_file_text(
    project: Entity<Project>,
    repository: Entity<Repository>,
    repo_path: RepoPath,
    text: String,
    cx: &mut App,
) -> Task<Result<()>> {
    let Some(project_path) = repository
        .read(cx)
        .repo_path_to_project_path(&repo_path, cx)
    else {
        return Task::ready(Err(anyhow!(
            "could not resolve repository path {repo_path:?}"
        )));
    };
    cx.spawn(async move |cx| {
        let buffer = project
            .update(cx, |project, cx| project.open_buffer(project_path, cx))
            .await?;
        let changed = buffer.update(cx, |buffer, cx| {
            if buffer.text() == text {
                return false;
            }
            let length = buffer.len();
            buffer.edit([(0..length, text.as_str())], None, cx);
            true
        });
        if changed {
            project
                .update(cx, |project, cx| project.save_buffer(buffer, cx))
                .await?;
        }
        Ok(())
    })
}

impl ConflictsDialog {
    fn model_context(&self) -> ModelContext {
        ModelContext {
            project: self.project.clone(),
            repository: self.repository.clone(),
            holder: self.holder.clone(),
            reversed: self.state.is_reversed(),
        }
    }

    fn in_window<R>(
        this: &WeakEntity<Self>,
        window_handle: AnyWindowHandle,
        cx: &mut AsyncApp,
        update: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) -> R,
    ) -> Result<R> {
        window_handle.update(cx, |_, window, app| {
            this.update(app, |dialog, dialog_cx| update(dialog, window, dialog_cx))
        })?
    }

    fn refresh_summaries(&mut self, cx: &mut Context<Self>) {
        let files = self.state.files().to_vec();
        for file in files {
            let model = self.holder.read(cx).merge_conflict_model(&file).cloned();
            if let Some(model) = model {
                let summary = summarize(model.read(cx));
                self.state.set_model_summary(file, summary);
            }
        }
    }

    pub(super) fn update_model_from_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_summaries(cx);
        if self.state.has_nothing_left_to_process() {
            self.do_cancel_action(window, cx);
            return;
        }
        self.state.rebuild(TreeStateStrategy::ModelChange);
        self.scroll_selection_into_view();
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn show_error_prompt(
        &self,
        title: &str,
        message: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let answer = window.prompt(PromptLevel::Critical, title, Some(message), &["OK"], cx);
        cx.spawn(async move |_, _| {
            answer.await.ok();
        })
        .detach();
    }

    fn report_error(
        this: &WeakEntity<Self>,
        window_handle: AnyWindowHandle,
        cx: &mut AsyncApp,
        title: &'static str,
        message: String,
    ) {
        Self::in_window(this, window_handle, cx, move |dialog, window, cx| {
            dialog.hide_progress(cx);
            dialog.show_error_prompt(title, &message, window, cx);
        })
        .log_err();
    }

    fn write_result_task(&self, file: &RepoPath, cx: &mut Context<Self>) -> Task<Result<()>> {
        let Some(model) = self.holder.read(cx).merge_conflict_model(file).cloned() else {
            return Task::ready(Ok(()));
        };
        let text = model.read(cx).current_result_text(cx);
        write_file_text(
            self.project.clone(),
            self.repository.clone(),
            file.clone(),
            text,
            cx,
        )
    }

    fn stage_task(
        &self,
        file: &RepoPath,
        resolution: Resolution,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let presence = self.state.presence_of(file);
        let side = conflict_side_for(resolution, self.state.is_reversed());
        let (to_add, to_remove) = split_conflicts_for_resolution(&[(file.clone(), presence)], side);
        self.repository.update(cx, |repository, cx| {
            repository.mark_conflicts_resolved(to_add, to_remove, cx)
        })
    }

    fn accept_revision_task(
        &self,
        files: &[RepoPath],
        side: Side,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let conflicts = self.state.conflicts_for(files);
        let reversed = self.state.is_reversed();
        self.repository.update(cx, |repository, cx| {
            repository.accept_conflict_side(conflicts, side == Side::Right, reversed, cx)
        })
    }

    async fn accept_revision(
        this: &WeakEntity<Self>,
        window_handle: AnyWindowHandle,
        files: Vec<RepoPath>,
        side: Side,
        cx: &mut AsyncApp,
    ) {
        if files.is_empty() {
            return;
        }
        let task = this.update(cx, |dialog, dialog_cx| {
            dialog.accept_revision_task(&files, side, dialog_cx)
        });
        let Ok(task) = task else {
            return;
        };
        match task.await {
            Ok(()) => {
                this.update(cx, |dialog, cx| {
                    dialog.state.mark_processed(&files);
                    cx.notify();
                })
                .log_err();
            }
            Err(error) => {
                Self::report_error(
                    this,
                    window_handle,
                    cx,
                    messages::ERROR_TITLE,
                    messages::error_saving_merged_data(&format!("{error:#}")),
                );
            }
        }
    }

    pub(super) fn accept_for_resolution(
        &mut self,
        side: Side,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay.is_some() || !self.table_enabled() || self.closing {
            return;
        }
        let files = self.state.selected_files();
        if files.is_empty() {
            return;
        }
        self.auto_resolve_status = None;
        self.show_progress(
            ProgressOverlay::indeterminate(messages::RESOLVING_CONFLICTS),
            cx,
        );
        let window_handle = self.window_handle;
        let side_label = match side {
            Side::Left => self.texts.yours_column.to_string(),
            Side::Right => self.texts.theirs_column.to_string(),
        };
        cx.spawn(async move |this, cx| {
            Self::run_accept(&this, window_handle, files, side, side_label, cx).await;
            this.update(cx, |dialog, cx| {
                if !dialog.closing {
                    dialog.hide_progress(cx);
                }
                dialog.auto_resolve_status = None;
                cx.notify();
            })
            .log_err();
        })
        .detach();
    }

    async fn prepare_models(
        this: &WeakEntity<Self>,
        window_handle: AnyWindowHandle,
        files: &[RepoPath],
        cx: &mut AsyncApp,
    ) -> Option<Vec<(RepoPath, PreparedModel)>> {
        let mut prepared = Vec::new();
        for file in files {
            let task = this
                .update(cx, |dialog, dialog_cx| {
                    prepare_model(
                        dialog.model_context(),
                        file.clone(),
                        dialog.state.presence_of(file),
                        dialog_cx,
                    )
                })
                .ok()?;
            match task.await {
                Ok(model) => prepared.push((file.clone(), model)),
                Err(error) => {
                    Self::report_error(
                        this,
                        window_handle,
                        cx,
                        messages::ERROR_TITLE,
                        messages::error_loading_revisions(&format!("{error:#}")),
                    );
                    return None;
                }
            }
        }
        Some(prepared)
    }

    async fn run_accept(
        this: &WeakEntity<Self>,
        window_handle: AnyWindowHandle,
        files: Vec<RepoPath>,
        side: Side,
        side_label: String,
        cx: &mut AsyncApp,
    ) {
        Self::accept_files(this, window_handle, files, side, &side_label, cx).await;
        Self::in_window(this, window_handle, cx, |dialog, window, cx| {
            dialog.update_model_from_files(window, cx);
        })
        .log_err();
    }

    async fn accept_files(
        this: &WeakEntity<Self>,
        window_handle: AnyWindowHandle,
        files: Vec<RepoPath>,
        side: Side,
        side_label: &str,
        cx: &mut AsyncApp,
    ) {
        let Some(prepared) = Self::prepare_models(this, window_handle, &files, cx).await else {
            return;
        };
        let mut binary_files = Vec::new();
        let mut non_iterative_files = Vec::new();
        let mut files_with_model = Vec::new();
        for (file, prepared) in prepared {
            match prepared {
                PreparedModel::Model(model) => files_with_model.push((file, model)),
                PreparedModel::Unsupported(Unsupported::Binary) => binary_files.push(file),
                PreparedModel::Unsupported(_) => non_iterative_files.push(file),
            }
        }
        Self::accept_revision(this, window_handle, binary_files, side, cx).await;
        if Self::overwrite_confirmed(this, &files_with_model, side_label, cx).await
            && !Self::accept_into_models(this, window_handle, &files_with_model, side, cx).await
        {
            return;
        }
        Self::accept_revision(this, window_handle, non_iterative_files, side, cx).await;
    }

    async fn overwrite_confirmed(
        this: &WeakEntity<Self>,
        files_with_model: &[(RepoPath, Entity<MergeConflictModel>)],
        side_label: &str,
        cx: &mut AsyncApp,
    ) -> bool {
        let any_resolved = files_with_model
            .iter()
            .any(|(_, model)| model.read_with(cx, |model, _| !model.resolved_changes().is_empty()));
        if !any_resolved {
            return true;
        }
        let receiver = this.update(cx, |dialog, dialog_cx| {
            dialog.ask_confirmation(
                messages::OVERWRITE_TITLE,
                messages::overwrite_message(files_with_model.len(), side_label),
                messages::OVERWRITE_YES,
                messages::CANCEL,
                dialog_cx,
            )
        });
        let accepted = match receiver {
            Ok(receiver) => receiver.await.unwrap_or(false),
            Err(_) => false,
        };
        this.update(cx, |dialog, cx| {
            dialog.show_progress(
                ProgressOverlay::indeterminate(messages::RESOLVING_CONFLICTS),
                cx,
            );
        })
        .log_err();
        accepted
    }

    async fn accept_into_models(
        this: &WeakEntity<Self>,
        window_handle: AnyWindowHandle,
        files_with_model: &[(RepoPath, Entity<MergeConflictModel>)],
        side: Side,
        cx: &mut AsyncApp,
    ) -> bool {
        for (file, model) in files_with_model {
            model.update(cx, |model, cx| {
                model.accept_revision_for_side(side, cx);
            });
            let write = this.update(cx, |dialog, dialog_cx| {
                dialog.write_result_task(file, dialog_cx)
            });
            if let Ok(write) = write
                && let Err(error) = write.await
            {
                Self::report_error(
                    this,
                    window_handle,
                    cx,
                    messages::ERROR_TITLE,
                    messages::error_saving_merged_data(&format!("{error:#}")),
                );
                return false;
            }
        }
        true
    }

    pub(super) fn resolve_all_simple_conflicts(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay.is_some() || self.state.is_resolving() || self.closing {
            return;
        }
        if !self.state.resolve_all_enabled() {
            return;
        }
        self.state.begin_resolve_all();
        self.auto_resolve_status = None;
        let files = sorted_files(
            &self.state.unresolved_group_files(),
            !self.state.group_by_directory(),
        );
        let resolved_before = self.state.total_resolved_changes();
        let cancellation = Arc::new(AtomicBool::new(false));
        self.show_progress(
            ProgressOverlay {
                title: gpui::SharedString::from(messages::RESOLVING_SIMPLE_CONFLICTS.to_string()),
                text: None,
                steps: Some((0, files.len())),
                cancellation: Some(cancellation.clone()),
            },
            cx,
        );
        let window_handle = self.window_handle;
        cx.spawn(async move |this, cx| {
            let cancelled =
                Self::run_resolve_all(&this, window_handle, files, &cancellation, cx).await;
            Self::in_window(&this, window_handle, cx, move |dialog, _window, cx| {
                dialog.finish_resolve_all(resolved_before, cancelled, cx);
            })
            .log_err();
        })
        .detach();
    }

    fn finish_resolve_all(
        &mut self,
        resolved_before: usize,
        cancelled: bool,
        cx: &mut Context<Self>,
    ) {
        self.refresh_summaries(cx);
        if cancelled {
            self.state.cancel_resolve_all();
        } else {
            self.state.finish_resolve_all();
        }
        self.hide_progress(cx);
        let resolved_after = self.state.total_resolved_changes();
        self.auto_resolve_status = messages::auto_resolve_status(
            resolved_after.saturating_sub(resolved_before),
            self.state.total_unresolved_changes(),
            self.state.files_with_unresolved_changes(),
        );
        if !cancelled {
            self.state.rebuild(TreeStateStrategy::Default);
            self.scroll_selection_into_view();
        }
        cx.notify();
    }

    async fn run_resolve_all(
        this: &WeakEntity<Self>,
        window_handle: AnyWindowHandle,
        files: Vec<RepoPath>,
        cancellation: &Arc<AtomicBool>,
        cx: &mut AsyncApp,
    ) -> bool {
        let total = files.len();
        for (index, file) in files.into_iter().enumerate() {
            if cancellation.load(std::sync::atomic::Ordering::Relaxed) {
                return true;
            }
            this.update(cx, |dialog, cx| {
                dialog.show_progress(
                    ProgressOverlay {
                        title: gpui::SharedString::from(
                            messages::RESOLVING_SIMPLE_CONFLICTS.to_string(),
                        ),
                        text: Some(gpui::SharedString::from(messages::resolving_file_progress(
                            index + 1,
                            total,
                        ))),
                        steps: Some((index, total)),
                        cancellation: Some(cancellation.clone()),
                    },
                    cx,
                );
            })
            .log_err();
            let task = this.update(cx, |dialog, dialog_cx| {
                prepare_model(
                    dialog.model_context(),
                    file.clone(),
                    dialog.state.presence_of(&file),
                    dialog_cx,
                )
            });
            let Ok(task) = task else {
                return false;
            };
            let prepared = match task.await {
                Ok(prepared) => prepared,
                Err(error) => {
                    Self::report_error(
                        this,
                        window_handle,
                        cx,
                        messages::ERROR_TITLE,
                        messages::error_loading_revisions(&format!("{error:#}")),
                    );
                    return false;
                }
            };
            let PreparedModel::Model(model) = prepared else {
                continue;
            };
            let resolved =
                model.update(cx, |model, cx| model.resolve_all_changes_automatically(cx));
            if resolved {
                let write = this.update(cx, |dialog, dialog_cx| {
                    dialog.write_result_task(&file, dialog_cx)
                });
                if let Ok(write) = write
                    && let Err(error) = write.await
                {
                    Self::report_error(
                        this,
                        window_handle,
                        cx,
                        messages::ERROR_TITLE,
                        messages::error_saving_merged_data(&format!("{error:#}")),
                    );
                    return false;
                }
            }
            Self::in_window(this, window_handle, cx, |dialog, window, cx| {
                dialog.refresh_summaries(cx);
                dialog.state.rebuild(TreeStateStrategy::ModelChange);
                window.focus(&dialog.focus_handle, cx);
                cx.notify();
            })
            .log_err();
        }
        false
    }

    pub(super) fn revert_selected_resolution(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay.is_some() || !self.table_enabled() || self.closing {
            return;
        }
        let derived = self.state.derived();
        if !derived.only_revertable_files_selected {
            return;
        }
        let files = derived.selected_files;
        let receiver = self.ask_confirmation(
            messages::REVERT_TITLE,
            messages::revert_message(files.len()),
            messages::REVERT_YES,
            messages::CANCEL,
            cx,
        );
        let window_handle = self.window_handle;
        cx.spawn(async move |this, cx| {
            let accepted = receiver.await.unwrap_or(false);
            if !accepted {
                return;
            }
            Self::in_window(&this, window_handle, cx, move |dialog, window, cx| {
                dialog.auto_resolve_status = None;
                dialog
                    .holder
                    .update(cx, |holder, cx| holder.remove_files(&files, cx));
                dialog.state.remove_model_summaries(&files);
                dialog.state.reset_resolve_all_pressed();
                dialog.update_model_from_files(window, cx);
            })
            .log_err();
        })
        .detach();
    }

    pub(super) fn review_button_pressed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.auto_resolve_status = None;
        self.open_merge_windows(window, cx);
    }

    pub(super) fn open_merge_windows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay.is_some() || !self.table_enabled() || self.closing {
            return;
        }
        let files = self.state.files_to_open(true);
        if files.is_empty() {
            return;
        }
        self.show_merge_chain(VecDeque::from(files), window, cx);
    }

    fn show_merge_chain(
        &mut self,
        mut remaining: VecDeque<RepoPath>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(file) = remaining.pop_front() else {
            self.update_model_from_files(window, cx);
            return;
        };
        self.show_progress(
            ProgressOverlay::indeterminate(messages::LOADING_REVISIONS),
            cx,
        );
        let window_handle = self.window_handle;
        let context = self.model_context();
        let task = prepare_model(
            context.clone(),
            file.clone(),
            self.state.presence_of(&file),
            cx,
        );
        let original_task = read_working_tree_text(&context, file.clone(), cx);
        cx.spawn(async move |this, cx| {
            let prepared = task.await;
            let original_content = original_task.await;
            Self::in_window(&this, window_handle, cx, move |dialog, window, cx| {
                dialog.hide_progress(cx);
                match prepared {
                    Ok(PreparedModel::Model(model)) => {
                        dialog.open_merge_window_for(
                            file,
                            model,
                            original_content,
                            remaining,
                            window,
                            cx,
                        );
                    }
                    Ok(PreparedModel::Unsupported(reason)) => {
                        dialog.report_unsupported(reason, window, cx);
                        dialog.update_model_from_files(window, cx);
                    }
                    Err(error) => {
                        dialog.show_error_prompt(
                            messages::ERROR_TITLE,
                            &messages::error_loading_revisions(&format!("{error:#}")),
                            window,
                            cx,
                        );
                        dialog.update_model_from_files(window, cx);
                    }
                }
            })
            .log_err();
        })
        .detach();
    }

    fn report_unsupported(&self, reason: Unsupported, window: &mut Window, cx: &mut Context<Self>) {
        let (title, message) = match reason {
            Unsupported::Binary | Unsupported::NotUtf8 => (
                messages::CANNOT_SHOW_MERGE_DIALOG,
                messages::BINARY_FILE_CANNOT_BE_MERGED.to_string(),
            ),
            Unsupported::Unavailable => (
                messages::ERROR_TITLE,
                messages::error_loading_revisions(messages::NOT_A_REGULAR_FILE),
            ),
            Unsupported::TooBig => (
                messages::CANNOT_SHOW_MERGE_DIALOG,
                MergeModelError::DiffTooBig.to_string(),
            ),
            Unsupported::ReadOnly => (
                messages::CANNOT_SHOW_MERGE_DIALOG,
                messages::READ_ONLY_FILE.to_string(),
            ),
        };
        self.show_error_prompt(title, &message, window, cx);
    }

    fn open_merge_window_for(
        &mut self,
        file: RepoPath,
        model: Entity<MergeConflictModel>,
        original_content: String,
        remaining: VecDeque<RepoPath>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dialog = cx.weak_entity();
        let window_handle = self.window_handle;
        let system_path = self
            .repository
            .read(cx)
            .snapshot()
            .work_directory_abs_path
            .join(file.as_std_path())
            .to_string_lossy()
            .into_owned();
        let finished_file = file.clone();
        let title_context = TitleContext {
            repository: self.repository.clone(),
            file: file.clone(),
            absolute_path: gpui::SharedString::from(system_path.clone()),
        };
        let request = MergeRequest {
            window_title: gpui::SharedString::from(messages::merge_window_title(&system_path)),
            pane_titles: merge_pane_titles(&self.texts.pane_titles, &title_context),
            model,
            file_name: gpui::SharedString::from(file.as_unix_str().to_string()),
            iterative: true,
            original_content,
            on_finished: Box::new(move |result, cx| {
                window_handle
                    .update(cx, |_, window, cx| {
                        dialog
                            .update(cx, |dialog, cx| {
                                dialog.merge_window_finished(
                                    finished_file,
                                    result,
                                    remaining,
                                    window,
                                    cx,
                                );
                            })
                            .log_err();
                    })
                    .log_err();
            }),
        };
        open_merge_window(request, DialogOwner::dialog_window(window_handle), cx);
    }

    fn merge_window_finished(
        &mut self,
        file: RepoPath,
        result: MergeResult,
        remaining: VecDeque<RepoPath>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(model) = self.holder.read(cx).merge_conflict_model(&file).cloned() else {
            self.update_model_from_files(window, cx);
            return;
        };
        let (content_modified, previous_side, resolved) = model.read_with(cx, |model, _| {
            (
                model.content_modified(),
                model.chosen_side(),
                model.is_fully_resolved(),
            )
        });
        let write_needed = (result != MergeResult::Cancel || content_modified)
            && (resolved || !matches!(result, MergeResult::Left | MergeResult::Right));
        let chosen_side = chosen_side_after_merge(result, previous_side);
        model.update(cx, |model, _| {
            model.mark_reviewed();
            model.set_chosen_side(chosen_side);
        });
        self.refresh_summaries(cx);
        let write = if write_needed {
            Some(self.write_result_task(&file, cx))
        } else {
            None
        };
        let window_handle = self.window_handle;
        if result != MergeResult::Cancel && !resolved {
            self.show_progress(
                ProgressOverlay::indeterminate(messages::RESOLVING_CONFLICTS),
                cx,
            );
        }
        cx.spawn(async move |this, cx| {
            if let Some(write) = write
                && let Err(error) = write.await
            {
                Self::report_error(
                    &this,
                    window_handle,
                    cx,
                    messages::ERROR_TITLE,
                    messages::error_saving_merged_data(&format!("{error:#}")),
                );
            }
            if result != MergeResult::Cancel && !resolved {
                Self::process_unresolved_file(&this, window_handle, &file, result, cx).await;
            }
            Self::in_window(&this, window_handle, cx, move |dialog, window, cx| {
                dialog.hide_progress(cx);
                if result == MergeResult::Cancel {
                    dialog.update_model_from_files(window, cx);
                } else {
                    dialog.refresh_summaries(cx);
                    dialog.show_merge_chain(remaining, window, cx);
                }
            })
            .log_err();
        })
        .detach();
    }

    async fn process_unresolved_file(
        this: &WeakEntity<Self>,
        window_handle: AnyWindowHandle,
        file: &RepoPath,
        result: MergeResult,
        cx: &mut AsyncApp,
    ) {
        match result {
            MergeResult::Left => {
                Self::accept_revision(this, window_handle, vec![file.clone()], Side::Left, cx)
                    .await;
            }
            MergeResult::Right => {
                Self::accept_revision(this, window_handle, vec![file.clone()], Side::Right, cx)
                    .await;
            }
            MergeResult::Resolved => {
                let task = this.update(cx, |dialog, dialog_cx| {
                    dialog.stage_task(file, Resolution::Merged, dialog_cx)
                });
                let outcome = match task {
                    Ok(task) => task.await,
                    Err(error) => Err(error),
                };
                match outcome {
                    Ok(()) => {
                        this.update(cx, |dialog, cx| {
                            dialog.state.mark_processed(std::slice::from_ref(file));
                            cx.notify();
                        })
                        .log_err();
                    }
                    Err(error) => Self::report_error(
                        this,
                        window_handle,
                        cx,
                        messages::ERROR_TITLE,
                        messages::error_saving_merged_data(&format!("{error:#}")),
                    ),
                }
            }
            MergeResult::Cancel => {}
        }
    }

    pub(super) fn close_button_pressed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay.is_some() || !self.table_enabled() || self.closing {
            return;
        }
        if !self.state.has_partially_resolved_files() {
            self.do_cancel_action(window, cx);
            return;
        }
        let receiver = self.ask_confirmation(
            messages::CLOSE_TITLE,
            messages::CLOSE_MESSAGE.to_string(),
            messages::CLOSE_YES,
            messages::CLOSE_NO,
            cx,
        );
        let window_handle = self.window_handle;
        cx.spawn(async move |this, cx| {
            if receiver.await.unwrap_or(false) {
                Self::in_window(&this, window_handle, cx, |dialog, window, cx| {
                    dialog.do_cancel_action(window, cx);
                })
                .log_err();
            }
        })
        .detach();
    }

    pub(super) fn silent_close_requested(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> CloseDecision {
        if self.closing || self.overlay.is_some() {
            return CloseDecision::Keep;
        }
        self.do_cancel_action(window, cx);
        CloseDecision::Keep
    }

    pub(super) fn accept_and_finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay.is_some() || self.closing || !self.state.accept_and_finish_enabled() {
            return;
        }
        self.begin_closing(true, window, cx);
    }

    pub(super) fn do_cancel_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_closing(false, window, cx);
    }

    fn begin_closing(&mut self, finish_merge: bool, _window: &mut Window, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        self.closing = true;
        self.context_menu = None;
        self.refresh_summaries(cx);
        let resolved: Vec<_> = self
            .state
            .resolved_files_and_models()
            .into_iter()
            .filter(|(file, _)| !self.state.processed_files().contains(file))
            .collect();
        if !resolved.is_empty() {
            self.show_progress(
                ProgressOverlay::indeterminate(messages::APPLYING_RESOLUTIONS),
                cx,
            );
        }
        let window_handle = self.window_handle;
        cx.spawn(async move |this, cx| {
            let failures = Self::apply_resolutions(&this, resolved, cx).await;
            Self::in_window(&this, window_handle, cx, move |dialog, window, cx| {
                if failures.is_empty() {
                    dialog.should_finish_merge |= finish_merge;
                    close_dialog_window(window);
                    dialog.report_closed(cx);
                } else {
                    dialog.closing = false;
                    dialog.hide_progress(cx);
                    dialog.update_model_from_files(window, cx);
                    dialog.show_error_prompt(
                        messages::ERROR_TITLE,
                        &messages::error_saving_merged_data(&failures.join("\n")),
                        window,
                        cx,
                    );
                }
            })
            .log_err();
        })
        .detach();
    }

    async fn apply_resolutions(
        this: &WeakEntity<Self>,
        resolved: Vec<(RepoPath, ModelSummary)>,
        cx: &mut AsyncApp,
    ) -> Vec<String> {
        let mut failures = Vec::new();
        let mut saved = Vec::new();
        for (file, summary) in resolved {
            let write = this.update(cx, |dialog, dialog_cx| {
                dialog.write_result_task(&file, dialog_cx)
            });
            let outcome = match write {
                Ok(write) => write.await,
                Err(error) => Err(error),
            };
            match outcome {
                Ok(()) => saved.push((file, summary)),
                Err(error) => {
                    log::error!("could not save the resolved file {file:?}: {error:#}");
                    failures.push(format!("{}: {error:#}", file.as_unix_str()));
                }
            }
        }
        for (file, summary) in saved {
            let resolution = resolution_for_chosen_side(summary.chosen_side);
            let task = this.update(cx, |dialog, dialog_cx| {
                dialog.stage_task(&file, resolution, dialog_cx)
            });
            let outcome = match task {
                Ok(task) => task.await,
                Err(error) => Err(error),
            };
            match outcome {
                Ok(()) => {
                    this.update(cx, |dialog, _| {
                        dialog.state.mark_processed(std::slice::from_ref(&file));
                    })
                    .log_err();
                }
                Err(error) => {
                    log::error!("could not mark {file:?} as resolved: {error:#}");
                    failures.push(format!("{}: {error:#}", file.as_unix_str()));
                }
            }
        }
        failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_tool_resolution_follows_chosen_side() {
        assert_eq!(
            resolution_for_chosen_side(Some(Side::Left)),
            Resolution::AcceptedYours
        );
        assert_eq!(
            resolution_for_chosen_side(Some(Side::Right)),
            Resolution::AcceptedTheirs
        );
        assert_eq!(resolution_for_chosen_side(None), Resolution::Merged);
    }

    #[test]
    fn merge_tool_conflict_side_follows_reversal_without_double_swap() {
        assert_eq!(
            conflict_side_for(Resolution::AcceptedYours, false),
            Some(ConflictSide::Ours)
        );
        assert_eq!(
            conflict_side_for(Resolution::AcceptedTheirs, false),
            Some(ConflictSide::Theirs)
        );
        assert_eq!(
            conflict_side_for(Resolution::AcceptedYours, true),
            Some(ConflictSide::Theirs)
        );
        assert_eq!(
            conflict_side_for(Resolution::AcceptedTheirs, true),
            Some(ConflictSide::Ours)
        );
        assert_eq!(conflict_side_for(Resolution::Merged, true), None);
    }

    #[test]
    fn merge_tool_chosen_side_after_merge_matches_dialog_callback() {
        assert_eq!(
            chosen_side_after_merge(MergeResult::Cancel, Some(Side::Right)),
            Some(Side::Right)
        );
        assert_eq!(chosen_side_after_merge(MergeResult::Cancel, None), None);
        assert_eq!(
            chosen_side_after_merge(MergeResult::Left, None),
            Some(Side::Left)
        );
        assert_eq!(
            chosen_side_after_merge(MergeResult::Right, Some(Side::Left)),
            Some(Side::Right)
        );
        assert_eq!(
            chosen_side_after_merge(MergeResult::Resolved, Some(Side::Left)),
            None
        );
    }
}
