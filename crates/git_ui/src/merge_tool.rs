mod conflicts_dialog;
mod merge_divider;
mod merge_view;
#[cfg(test)]
mod tests;

use std::{pin::pin, time::Duration};

use anyhow::{Result, anyhow};
use collections::HashMap;
use futures::future::{self, Either};
use git::repository::RepoPath;
use gpui::{
    App, Context, Entity, Global, SharedString, Task, TaskExt as _, WeakEntity, Window, actions,
};
use language::LineEnding;
use project::{
    Project,
    git_store::{ConflictStages, Repository, RepositoryEvent, RepositoryId, RepositorySnapshot},
};
use three_way_merge::{Counters, IgnorePolicy, MergeDocument, Revision, Side, ThreeWayMerge};
use util::ResultExt as _;
use workspace::Workspace;

pub use conflicts_dialog::ConflictsDialog;
pub use merge_view::MergeView;

actions!(
    merge_tool,
    [
        OpenConflicts,
        OpenMergeTool,
        AcceptYours,
        AcceptTheirs,
        RevertConflictResolution,
        NextDifference,
        PreviousDifference,
        AcceptLeftSide,
        AcceptRightSide,
        AppendLeftSide,
        AppendRightSide,
        IgnoreLeftSide,
        IgnoreRightSide,
        ResolveUsingLeft,
        ResolveUsingRight,
        ResolveSimpleConflict,
        ApplyNonConflictingLeft,
        ApplyNonConflictingRight,
        ApplyNonConflictingAll,
        ResolveSimpleConflicts,
        ToggleSynchronizeScrolling,
        FocusOppositePane,
        ShowSettings,
        AcceptLeft,
        AcceptRight,
        SaveAndClose,
        ApplyChanges,
    ]
);

const CONFLICT_SCAN_TIMEOUT: Duration = Duration::from_secs(5);
const SHORT_SHA_LENGTH: usize = 7;
const DETACHED_HEAD_NAME: &str = "HEAD";
const MERGE_HEAD_NAME: &str = "MERGE_HEAD";

#[derive(Default)]
struct MergeSessions {
    documents: HashMap<(RepositoryId, RepoPath), MergeDocument>,
}

impl Global for MergeSessions {}

pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _, cx| {
        if !is_available(workspace.project().read(cx), cx) {
            return;
        }
        workspace.register_action(|workspace, _: &OpenConflicts, window, cx| {
            open_conflicts_dialog(workspace, window, cx);
        });
    })
    .detach();
}

pub fn is_available(project: &Project, _cx: &App) -> bool {
    project.is_local()
}

pub fn open_conflicts_dialog(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(repository) = workspace.project().read(cx).active_repository(cx) else {
        return;
    };
    show_conflicts_dialog(workspace, repository, window, cx);
}

pub fn open_conflicts_dialog_if_conflicted(
    workspace: WeakEntity<Workspace>,
    repository: Entity<Repository>,
    window: &mut Window,
    cx: &mut App,
) {
    if has_unmerged_paths(repository.read(cx)) {
        workspace
            .update(cx, |workspace, cx| {
                show_conflicts_dialog(workspace, repository, window, cx);
            })
            .log_err();
        return;
    }

    let (conflicts_sender, conflicts_receiver) = async_channel::unbounded();
    let subscription = cx.subscribe(
        &repository,
        move |repository, event: &RepositoryEvent, cx| {
            if *event == RepositoryEvent::StatusesChanged && has_unmerged_paths(repository.read(cx))
            {
                conflicts_sender.try_send(()).log_err();
            }
        },
    );
    window
        .spawn(cx, async move |cx| {
            let timeout = pin!(cx.background_executor().timer(CONFLICT_SCAN_TIMEOUT));
            let conflicts_detected = pin!(conflicts_receiver.recv());
            let detected = matches!(
                future::select(conflicts_detected, timeout).await,
                Either::Left((Ok(()), _))
            );
            drop(subscription);
            if detected {
                workspace.update_in(cx, |workspace, window, cx| {
                    show_conflicts_dialog(workspace, repository, window, cx);
                })?;
            }
            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
}

pub fn open_merge_tool(
    workspace: &mut Workspace,
    repository: Entity<Repository>,
    repo_path: RepoPath,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Task<Result<()>> {
    MergeView::open(workspace, repository, repo_path, window, cx)
}

pub fn accept_side(
    project: Entity<Project>,
    repository: Entity<Repository>,
    repo_path: RepoPath,
    side: Side,
    cx: &mut App,
) -> Task<Result<()>> {
    let stages = repository.update(cx, |repository, cx| {
        repository.load_conflict_stages(repo_path.clone(), cx)
    });
    cx.spawn(async move |cx| {
        let stages = stages.await?;
        let text = match side {
            Side::Left => stages.ours,
            Side::Right => stages.theirs,
        };
        cx.update(|cx| write_merge_result(project, repository, repo_path, text, cx))
            .await
    })
}

pub(crate) fn show_conflicts_dialog(
    workspace: &mut Workspace,
    repository: Entity<Repository>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    if workspace.active_modal::<ConflictsDialog>(cx).is_some() {
        return;
    }
    let project = workspace.project().clone();
    let workspace_handle = workspace.weak_handle();
    workspace.toggle_modal(window, cx, |window, cx| {
        ConflictsDialog::new(workspace_handle, project, repository, window, cx)
    });
}

pub(crate) fn has_unmerged_paths(snapshot: &RepositorySnapshot) -> bool {
    snapshot.status().any(|entry| entry.status.is_conflicted())
}

pub(crate) fn write_merge_result(
    project: Entity<Project>,
    repository: Entity<Repository>,
    repo_path: RepoPath,
    text: Option<String>,
    cx: &mut App,
) -> Task<Result<()>> {
    let repository_id = repository.read(cx).id;
    let Some(project_path) = repository
        .read(cx)
        .repo_path_to_project_path(&repo_path, cx)
    else {
        return Task::ready(Err(anyhow!(
            "could not resolve repository path {repo_path:?}"
        )));
    };
    cx.spawn(async move |cx| {
        match text {
            Some(text) => {
                let buffer = project
                    .update(cx, |project, cx| project.open_buffer(project_path, cx))
                    .await?;
                buffer.update(cx, |buffer, cx| {
                    let end = buffer.len();
                    buffer.edit([(0..end, text)], None, cx);
                });
                project
                    .update(cx, |project, cx| project.save_buffer(buffer, cx))
                    .await?;
            }
            None => {
                let deletion =
                    project.update(cx, |project, cx| project.delete_file(project_path, cx));
                if let Some(deletion) = deletion {
                    deletion.await?;
                }
            }
        }
        repository
            .update(cx, |repository, cx| {
                repository.stage_entries(vec![repo_path.clone()], cx)
            })
            .await?;
        cx.update(|cx| clear_merge_session(repository_id, &repo_path, cx));
        Ok(())
    })
}

pub(crate) fn merge_for_stages(stages: &ConflictStages, policy: IgnorePolicy) -> ThreeWayMerge {
    ThreeWayMerge::new(
        normalized_stage_text(stages.base.as_deref()),
        normalized_stage_text(stages.ours.as_deref()),
        normalized_stage_text(stages.theirs.as_deref()),
        policy,
    )
}

fn normalized_stage_text(text: Option<&str>) -> String {
    let mut text = text.unwrap_or_default().to_string();
    LineEnding::normalize(&mut text);
    text
}

pub(crate) fn resolved_text(stages: &ConflictStages, text: String) -> Option<String> {
    if text.is_empty() && (stages.ours.is_none() || stages.theirs.is_none()) {
        None
    } else {
        Some(text)
    }
}

pub(crate) fn document_matches_merge(document: &MergeDocument, merge: &ThreeWayMerge) -> bool {
    let stored = document.model().merge();
    [Revision::Base, Revision::Left, Revision::Right]
        .into_iter()
        .all(|revision| stored.text(revision) == merge.text(revision))
}

pub(crate) fn merge_session(
    repository_id: RepositoryId,
    repo_path: &RepoPath,
    cx: &App,
) -> Option<MergeDocument> {
    cx.try_global::<MergeSessions>()?
        .documents
        .get(&(repository_id, repo_path.clone()))
        .cloned()
}

pub(crate) fn merge_session_counters(
    repository_id: RepositoryId,
    repo_path: &RepoPath,
    cx: &App,
) -> Option<Counters> {
    cx.try_global::<MergeSessions>()?
        .documents
        .get(&(repository_id, repo_path.clone()))
        .map(|document| document.model().counters())
}

pub(crate) fn store_merge_session(
    repository_id: RepositoryId,
    repo_path: RepoPath,
    document: MergeDocument,
    cx: &mut App,
) {
    cx.default_global::<MergeSessions>()
        .documents
        .insert((repository_id, repo_path), document);
}

pub(crate) fn clear_merge_session(repository_id: RepositoryId, repo_path: &RepoPath, cx: &mut App) {
    cx.default_global::<MergeSessions>()
        .documents
        .remove(&(repository_id, repo_path.clone()));
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MergeBranches {
    pub(crate) ours: SharedString,
    pub(crate) theirs: SharedString,
}

impl MergeBranches {
    pub(crate) fn for_snapshot(snapshot: &RepositorySnapshot) -> Self {
        let ours = snapshot
            .branch
            .as_ref()
            .map(|branch| SharedString::from(branch.name().to_string()))
            .unwrap_or_else(|| DETACHED_HEAD_NAME.into());
        let theirs = snapshot
            .merge
            .message
            .as_deref()
            .and_then(branch_from_merge_message)
            .or_else(|| merge_head_short_sha(snapshot))
            .unwrap_or_else(|| MERGE_HEAD_NAME.into());
        Self { ours, theirs }
    }
}

fn branch_from_merge_message(message: &str) -> Option<SharedString> {
    let first_line = message.lines().next()?.trim();
    let description = first_line.strip_prefix("Merge ")?;
    let (_, quoted) = description.split_once('\'')?;
    let (name, _) = quoted.split_once('\'')?;
    (!name.is_empty()).then(|| SharedString::from(name.to_string()))
}

fn merge_head_short_sha(snapshot: &RepositorySnapshot) -> Option<SharedString> {
    snapshot
        .merge
        .merge_heads_by_conflicted_path
        .values()
        .find_map(|heads| heads.first().cloned().flatten())
        .map(|sha| {
            sha.chars()
                .take(SHORT_SHA_LENGTH)
                .collect::<String>()
                .into()
        })
}
