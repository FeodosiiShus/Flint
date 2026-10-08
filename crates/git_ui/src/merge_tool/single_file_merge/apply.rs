use anyhow::{Result, anyhow};
use git::repository::{
    ConflictSide, RepoPath, UnmergedStagePresence, split_conflicts_for_resolution,
};
use gpui::{App, Entity, Task};
use project::{Project, git_store::Repository};

use crate::merge_tool::merge_window::MergeResult;

pub(super) struct AppliedMerge {
    pub(super) project: Entity<Project>,
    pub(super) repository: Entity<Repository>,
    pub(super) repo_path: RepoPath,
    pub(super) presence: UnmergedStagePresence,
    pub(super) reversed: bool,
    pub(super) result: MergeResult,
    pub(super) text: String,
}

pub(super) fn conflict_side_for_result(
    result: MergeResult,
    reversed: bool,
) -> Option<ConflictSide> {
    match result {
        MergeResult::Left => Some(ConflictSide::for_accepted_version(false, reversed)),
        MergeResult::Right => Some(ConflictSide::for_accepted_version(true, reversed)),
        MergeResult::Resolved | MergeResult::Cancel => None,
    }
}

pub(super) fn apply_merge_result(applied: AppliedMerge, cx: &mut App) -> Task<Result<()>> {
    let AppliedMerge {
        project,
        repository,
        repo_path,
        presence,
        reversed,
        result,
        text,
    } = applied;
    let write = write_file_text(project, repository.clone(), repo_path.clone(), text, cx);
    cx.spawn(async move |cx| {
        write.await?;
        let side = conflict_side_for_result(result, reversed);
        let (to_add, to_remove) = split_conflicts_for_resolution(&[(repo_path, presence)], side);
        repository
            .update(cx, |repository, cx| {
                repository.mark_conflicts_resolved(to_add, to_remove, cx)
            })
            .await
    })
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
        let needs_save = buffer.update(cx, |buffer, cx| {
            if buffer.text() != text {
                let length = buffer.len();
                buffer.edit([(0..length, text.as_str())], None, cx);
            }
            buffer.is_dirty()
        });
        if needs_save {
            project
                .update(cx, |project, cx| project.save_buffer(buffer, cx))
                .await?;
        }
        Ok(())
    })
}
