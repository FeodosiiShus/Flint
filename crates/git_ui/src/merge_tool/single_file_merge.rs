mod apply;
mod registry;
#[cfg(test)]
mod tests;

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use git::repository::{RepoPath, UnmergedEntry, UnmergedStagePresence};
use gpui::{
    AnyWindowHandle, App, AppContext as _, AsyncApp, Context, Entity, PromptLevel, SharedString,
    Subscription, WeakEntity, Window,
};
use project::{
    Fs, Project,
    git_store::{Repository, RepositoryId},
};
use util::ResultExt as _;
use workspace::Workspace;

use self::apply::{AppliedMerge, apply_merge_result};
use self::registry::{Claim, MergeKey};
use crate::branch_operations::{BranchContext, BranchNotice};
use crate::merge_tool::conflict_resolution::{
    customizer::{CustomizerKind, build_dialog_texts, load_dialog_texts},
    dialog_texts::DialogTexts,
    labels::RepositoryFacts,
};
use crate::merge_tool::conflicts_dialog::{
    messages,
    model_loading::{
        ModelContext, PreparedModel, Unsupported, prepare_model, read_working_tree_text,
    },
    pane_titles::{TitleContext, merge_pane_titles},
};
use crate::merge_tool::dialog_window::{CloseRequest, DialogOwner};
use crate::merge_tool::merge_model::{
    MergeConflictModel, MergeModelError, iterative_data_holder::MergeConflictIterativeDataHolder,
};
use crate::merge_tool::merge_window::{MergeRequest, MergeResult, open_merge_window_with};
use crate::merge_tool::repository_is_rebasing;

const CANNOT_RESOLVE_CONFLICT_TITLE: &str = "Cannot Resolve Conflict";

fn cannot_find_file_message(path: &str) -> String {
    format!("Cannot find file for {path}")
}

struct OpenRequest {
    workspace: WeakEntity<Workspace>,
    repository: Entity<Repository>,
    repo_path: RepoPath,
    owner_window: AnyWindowHandle,
    key: MergeKey,
}

struct PreparedMerge {
    project: Entity<Project>,
    model: Entity<MergeConflictModel>,
    texts: DialogTexts,
    original_content: String,
    presence: UnmergedStagePresence,
    reversed: bool,
    system_path: String,
}

pub(crate) fn open_single_file_merge(
    workspace: WeakEntity<Workspace>,
    repository: Entity<Repository>,
    repo_path: RepoPath,
    window: &mut Window,
    cx: &mut App,
) {
    let key = MergeKey::new(repository.read(cx).id, repo_path.clone());
    if registry::claim(&key, cx) != Claim::Claimed {
        return;
    }
    let request = OpenRequest {
        workspace,
        repository,
        repo_path,
        owner_window: window.window_handle(),
        key,
    };
    cx.spawn(async move |cx| open_merge(request, cx).await)
        .detach();
}

pub(crate) fn observe_active_merge_windows<T: 'static>(
    cx: &mut Context<T>,
    callback: impl FnMut(&mut T, &mut Context<T>) + 'static,
) -> Subscription {
    cx.observe_global::<registry::OpenMergeWindows>(callback)
}

pub(crate) fn has_active_merge_window(
    repository: RepositoryId,
    repo_path: &RepoPath,
    cx: &App,
) -> bool {
    registry::open_window(&MergeKey::new(repository, repo_path.clone()), cx).is_some()
}

pub(crate) fn show_active_merge_window(
    repository: RepositoryId,
    repo_path: &RepoPath,
    cx: &mut App,
) {
    let key = MergeKey::new(repository, repo_path.clone());
    if let Some(window) = registry::open_window(&key, cx) {
        window
            .update(cx, |_, window, _| window.activate_window())
            .log_err();
    }
}

pub(crate) fn cancel_active_merge_window(
    repository: RepositoryId,
    repo_path: &RepoPath,
    cx: &mut App,
) {
    let key = MergeKey::new(repository, repo_path.clone());
    if let Some(window) = registry::open_window(&key, cx) {
        window
            .update(cx, |shell, window, cx| {
                shell.close_from_shortcut(CloseRequest::TitleBarButton, window, cx);
            })
            .log_err();
    }
}

async fn open_merge(request: OpenRequest, cx: &mut AsyncApp) {
    let prepared = match prepare_merge(&request, cx).await {
        Ok(Some(prepared)) => prepared,
        Ok(None) => {
            release_slot(&request.key, cx);
            return;
        }
        Err(error) => {
            report_error(
                request.owner_window,
                messages::ERROR_TITLE,
                &messages::error_loading_revisions(&format!("{error:#}")),
                cx,
            );
            release_slot(&request.key, cx);
            return;
        }
    };
    cx.update(|app| show_merge_window(request, prepared, app));
}

fn release_slot(key: &MergeKey, cx: &mut AsyncApp) {
    cx.update(|app| registry::release(key, app));
}

async fn prepare_merge(request: &OpenRequest, cx: &mut AsyncApp) -> Result<Option<PreparedMerge>> {
    let (project, fs) = request.workspace.read_with(cx, |workspace, cx| {
        let project = workspace.project().clone();
        let fs = project.read(cx).fs().clone();
        (project, fs)
    })?;
    let Some(entry) = find_unmerged_entry(&request.repository, &request.repo_path, cx).await?
    else {
        log::info!(
            "{:?} is not unmerged anymore, the merge window is not opened",
            request.repo_path
        );
        return Ok(None);
    };
    let absolute_path = request.repository.read_with(cx, |repository, _| {
        repository.repo_path_to_abs_path(&request.repo_path)
    });
    if !can_show_merge_window(&fs, &absolute_path, &entry, request, cx).await? {
        return Ok(None);
    }

    let reversed = repository_is_rebasing(&request.repository, cx).await;
    let texts = load_texts(&request.repository, &fs, reversed, cx).await;
    let holder = cx.update(|app| app.new(|_| MergeConflictIterativeDataHolder::new()));
    let model_context = ModelContext {
        project: project.clone(),
        repository: request.repository.clone(),
        holder,
        reversed,
    };
    let prepared_model = cx
        .update(|app| {
            prepare_model(
                model_context.clone(),
                request.repo_path.clone(),
                entry.stages,
                app,
            )
        })
        .await?;
    let model = match prepared_model {
        PreparedModel::Model(model) => model,
        PreparedModel::Unsupported(reason) => {
            report_unsupported(request.owner_window, reason, cx);
            return Ok(None);
        }
    };
    let original_content = cx
        .update(|app| read_working_tree_text(&model_context, request.repo_path.clone(), app))
        .await;
    Ok(Some(PreparedMerge {
        project,
        model,
        texts,
        original_content,
        presence: entry.stages,
        reversed,
        system_path: absolute_path.to_string_lossy().into_owned(),
    }))
}

async fn find_unmerged_entry(
    repository: &Entity<Repository>,
    repo_path: &RepoPath,
    cx: &mut AsyncApp,
) -> Result<Option<UnmergedEntry>> {
    let entries = repository
        .update(cx, |repository, cx| repository.unmerged_entries(cx))
        .await?;
    Ok(entries.into_iter().find(|entry| &entry.path == repo_path))
}

async fn can_show_merge_window(
    fs: &Arc<dyn Fs>,
    absolute_path: &Path,
    entry: &UnmergedEntry,
    request: &OpenRequest,
    cx: &mut AsyncApp,
) -> Result<bool> {
    let Some(metadata) = fs.metadata(absolute_path).await? else {
        notify_missing_file(request, absolute_path, cx);
        return Ok(false);
    };
    if metadata.is_dir {
        log::info!("{absolute_path:?} is a directory, conflicts in submodules cannot be merged");
        return Ok(false);
    }
    if !entry.stages.ours && !entry.stages.theirs {
        log::info!("{absolute_path:?} is deleted on both sides, there is nothing to merge");
        return Ok(false);
    }
    Ok(true)
}

async fn load_texts(
    repository: &Entity<Repository>,
    fs: &Arc<dyn Fs>,
    reversed: bool,
    cx: &mut AsyncApp,
) -> DialogTexts {
    let kind = CustomizerKind::GitDefault;
    let loading = cx.update(|app| load_dialog_texts(repository, fs, &kind, reversed, app));
    loading
        .await
        .warn_on_err()
        .unwrap_or_else(|| build_dialog_texts(&kind, &RepositoryFacts::default(), None, reversed))
}

fn show_merge_window(request: OpenRequest, prepared: PreparedMerge, cx: &mut App) {
    let OpenRequest {
        workspace,
        repository,
        repo_path,
        owner_window,
        key,
    } = request;
    let PreparedMerge {
        project,
        model,
        texts,
        original_content,
        presence,
        reversed,
        system_path,
    } = prepared;
    let title_context = TitleContext {
        repository: repository.clone(),
        file: repo_path.clone(),
        absolute_path: SharedString::from(system_path.clone()),
    };
    let finished_key = key.clone();
    let finished_model = model.clone();
    let merge_request = MergeRequest {
        window_title: SharedString::from(messages::merge_window_title(&system_path)),
        pane_titles: merge_pane_titles(&texts.pane_titles, &title_context),
        model,
        file_name: SharedString::from(repo_path.as_unix_str().to_string()),
        iterative: false,
        original_content,
        on_finished: Box::new(move |result, cx| {
            registry::release(&finished_key, cx);
            if result == MergeResult::Cancel {
                return;
            }
            let applied = AppliedMerge {
                project,
                repository,
                repo_path,
                presence,
                reversed,
                result,
                text: finished_model.read(cx).current_result_text(cx),
            };
            let applying = apply_merge_result(applied, cx);
            cx.spawn(async move |cx| {
                if let Err(error) = applying.await {
                    report_error(
                        owner_window,
                        messages::ERROR_TITLE,
                        &messages::error_saving_merged_data(&format!("{error:#}")),
                        cx,
                    );
                }
            })
            .detach();
        }),
    };
    open_merge_window_with(
        merge_request,
        DialogOwner::workspace(workspace, owner_window),
        cx,
        Some(Box::new(move |opened, cx| {
            registry::mark_open(&key, opened.window, cx);
        })),
    );
}

fn notify_missing_file(request: &OpenRequest, absolute_path: &Path, cx: &mut AsyncApp) {
    let notice = BranchNotice::error(cannot_find_file_message(&absolute_path.to_string_lossy()))
        .title(CANNOT_RESOLVE_CONFLICT_TITLE);
    let context = BranchContext::new(request.workspace.clone(), request.repository.clone());
    cx.update(|app| context.notify(notice, app));
}

fn report_unsupported(owner_window: AnyWindowHandle, reason: Unsupported, cx: &mut AsyncApp) {
    match reason {
        Unsupported::Binary | Unsupported::NotUtf8 => report_error(
            owner_window,
            messages::CANNOT_SHOW_MERGE_DIALOG,
            messages::BINARY_FILE_CANNOT_BE_MERGED,
            cx,
        ),
        Unsupported::Unavailable => report_error(
            owner_window,
            messages::ERROR_TITLE,
            &messages::error_loading_revisions(messages::NOT_A_REGULAR_FILE),
            cx,
        ),
        Unsupported::TooBig => report_error(
            owner_window,
            messages::CANNOT_SHOW_MERGE_DIALOG,
            &MergeModelError::DiffTooBig.to_string(),
            cx,
        ),
        Unsupported::ReadOnly => report_error(
            owner_window,
            messages::CANNOT_SHOW_MERGE_DIALOG,
            messages::READ_ONLY_FILE,
            cx,
        ),
    }
}

fn report_error(owner_window: AnyWindowHandle, title: &str, message: &str, cx: &mut AsyncApp) {
    cx.update(|app| show_error_prompt(owner_window, title, message, app));
}

fn show_error_prompt(owner_window: AnyWindowHandle, title: &str, message: &str, cx: &mut App) {
    let prompted = owner_window.update(cx, |_, window, app| {
        window.prompt(PromptLevel::Critical, title, Some(message), &["OK"], app)
    });
    match prompted {
        Ok(answer) => cx
            .spawn(async move |_| {
                answer.await.ok();
            })
            .detach(),
        Err(error) => {
            log::error!("could not show the error prompt {title:?} ({message}): {error:#}");
        }
    }
}
