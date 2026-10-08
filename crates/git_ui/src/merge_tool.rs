pub(crate) mod conflict_banner;
pub(crate) mod conflict_resolution;
pub(crate) mod conflicts_dialog;
pub(crate) mod dialog_window;
pub(crate) mod merge_gutter;
pub(crate) mod merge_model;
pub(crate) mod merge_ribbons;
pub(crate) mod merge_viewer;
pub(crate) mod merge_window;
pub(crate) mod palette;
pub(crate) mod single_file_merge;
#[cfg(test)]
mod tests;

use git::repository::{RepoPath, RepositoryOperation};
use git::stash::StashEntry;
use gpui::{App, AsyncApp, Context, Entity, Task, Window, actions};
use project::{Project, git_store::Repository};
use workspace::Workspace;

use self::conflict_resolution::{
    ConflictContext, ConflictParams, ResolveMode, ResolverBehavior, resolve_conflicts,
    unmerged_files_on_disk,
};

actions!(
    merge_tool,
    [
        ResolveConflicts,
        MergeConflictedFile,
        AcceptConflictTheirs,
        AcceptConflictYours,
    ]
);

pub fn init(cx: &mut App) {
    conflict_banner::init(cx);
    cx.observe_new(|workspace: &mut Workspace, _, cx| {
        if !is_available(workspace.project().read(cx), cx) {
            return;
        }
        workspace.register_action(|workspace, _: &ResolveConflicts, window, cx| {
            resolve_conflicts_in_workspace(workspace, window, cx);
        });
    })
    .detach();
}

pub fn is_available(project: &Project, _cx: &App) -> bool {
    project.is_local()
}

pub(crate) fn resolve_conflicts_in_workspace(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(repository) = workspace.project().read(cx).active_repository(cx) else {
        return;
    };
    let context = ConflictContext {
        workspace: workspace.weak_handle(),
        window: window.window_handle(),
        repository,
    };
    show_conflicts_dialog(context, cx);
}

pub(crate) fn show_conflicts_dialog(context: ConflictContext, cx: &mut App) {
    cx.spawn(async move |cx| {
        let reverse = repository_is_rebasing(&context.repository, cx).await;
        let resolution = cx.update(|app| {
            resolve_conflicts(
                context,
                ConflictParams::for_resolve_action(reverse),
                ResolverBehavior::Plain,
                ResolveMode::Initial,
                app,
            )
        });
        resolution.await;
    })
    .detach();
}

async fn repository_is_rebasing(repository: &Entity<Repository>, cx: &mut AsyncApp) -> bool {
    let receiver = repository.update(cx, |repository, _| repository.operation_in_progress());
    let operation = match receiver.await {
        Ok(Ok(operation)) => operation,
        Ok(Err(error)) => {
            log::warn!("could not read the operation in progress: {error:#}");
            None
        }
        Err(_) => {
            log::warn!("the operation in progress query was canceled");
            None
        }
    };
    operation == Some(RepositoryOperation::Rebase)
}

pub(crate) fn accept_conflict_sides(
    repository: Entity<Repository>,
    paths: Vec<RepoPath>,
    accept_theirs: bool,
    cx: &mut App,
) -> Task<anyhow::Result<()>> {
    cx.spawn(async move |cx| {
        let reversed = repository_is_rebasing(&repository, cx).await;
        let entries = repository
            .update(cx, |repository, cx| repository.unmerged_entries(cx))
            .await?;
        let conflicts: Vec<_> = entries
            .into_iter()
            .filter(|entry| paths.contains(&entry.path))
            .map(|entry| (entry.path, entry.stages))
            .collect();
        repository
            .update(cx, |repository, cx| {
                repository.accept_conflict_side(conflicts, accept_theirs, reversed, cx)
            })
            .await
    })
}

fn is_context_available(context: &ConflictContext, cx: &App) -> bool {
    context
        .workspace
        .upgrade()
        .is_some_and(|workspace| is_available(workspace.read(cx).project().read(cx), cx))
}

pub(crate) fn resolve_conflicts_after_pull(context: ConflictContext, rebase: bool, cx: &mut App) {
    if !is_context_available(&context, cx) {
        return;
    }
    cx.spawn(async move |cx| {
        let pending = unmerged_files_on_disk(&context, cx).await;
        match pending {
            Ok(entries) if !entries.is_empty() => {}
            Ok(_) => return,
            Err(error) => {
                log::warn!("could not check the repository for unmerged paths: {error:#}");
                return;
            }
        }
        let (params, behavior) = if rebase {
            (
                ConflictParams::for_update_by_rebase(),
                ResolverBehavior::ContinueRebase,
            )
        } else {
            (
                ConflictParams::for_update_by_merge(),
                ResolverBehavior::CommitMergeAlways,
            )
        };
        let resolution = cx
            .update(|app| resolve_conflicts(context, params, behavior, ResolveMode::Initial, app));
        resolution.await;
    })
    .detach();
}

pub(crate) fn resolve_unstash_conflicts(
    context: ConflictContext,
    stash: Option<StashEntry>,
    cx: &mut App,
) {
    if !is_context_available(&context, cx) {
        return;
    }
    let (name, message) = match stash {
        Some(stash) => (format!("stash@{{{}}}", stash.index), stash.message),
        None => (String::from("stash@{0}"), String::new()),
    };
    resolve_conflicts(
        context,
        ConflictParams::for_unstash(&name, &message),
        ResolverBehavior::Plain,
        ResolveMode::Initial,
        cx,
    )
    .detach();
}
