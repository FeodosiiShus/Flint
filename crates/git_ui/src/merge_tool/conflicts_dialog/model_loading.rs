use std::sync::Arc;

use crate::merge_tool::merge_viewer::viewer_settings::stored_ignore_policy;
use anyhow::Result;
use git::repository::{RepoPath, UnmergedStagePresence, is_binary_content};
use gpui::{App, Entity, Task};
use language::Language;
use project::{Project, git_store::Repository};
use util::ResultExt as _;

use crate::merge_tool::merge_model::{
    FileConflictType, MergeBuffers, MergeConflictModel, MergeModelError, MergeTexts,
    iterative_data_holder::{MergeConflictIterativeDataHolder, MergeModelRequest},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unsupported {
    Binary,
    NotUtf8,
    Unavailable,
    TooBig,
    ReadOnly,
}

pub(crate) enum PreparedModel {
    Model(Entity<MergeConflictModel>),
    Unsupported(Unsupported),
}

#[derive(Clone)]
pub(crate) struct ModelContext {
    pub(crate) project: Entity<Project>,
    pub(crate) repository: Entity<Repository>,
    pub(crate) holder: Entity<MergeConflictIterativeDataHolder>,
    pub(crate) reversed: bool,
}

pub(crate) fn file_conflict_type(
    presence: UnmergedStagePresence,
    reversed: bool,
) -> FileConflictType {
    match (presence.base, presence.ours, presence.theirs) {
        (true, true, false) if reversed => FileConflictType::DeletedModified,
        (true, true, false) => FileConflictType::ModifiedDeleted,
        (true, false, true) if reversed => FileConflictType::ModifiedDeleted,
        (true, false, true) => FileConflictType::DeletedModified,
        (false, true, true) => FileConflictType::AddedAdded,
        _ => FileConflictType::Default,
    }
}

pub(crate) fn arrange_sides(ours: &str, base: &str, theirs: &str, reversed: bool) -> MergeTexts {
    if reversed {
        MergeTexts::new(theirs, base, ours)
    } else {
        MergeTexts::new(ours, base, theirs)
    }
}

pub(crate) fn has_merge_conflict_markers(content: &[u8]) -> bool {
    contains_bytes(content, b"<<<<<<<")
        && contains_bytes(content, b"\n=======")
        && contains_bytes(content, b"\n>>>>>>>")
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn decode_text(content: Option<Vec<u8>>) -> Result<String, Unsupported> {
    match content {
        None => Ok(String::new()),
        Some(bytes) if is_binary_content(&bytes) => Err(Unsupported::Binary),
        Some(bytes) => String::from_utf8(bytes).map_err(|_| Unsupported::NotUtf8),
    }
}

fn model_error_to_unsupported(error: MergeModelError) -> Result<Unsupported, MergeModelError> {
    match error {
        MergeModelError::DiffTooBig => Ok(Unsupported::TooBig),
        MergeModelError::ReadOnlyResult => Ok(Unsupported::ReadOnly),
        MergeModelError::Canceled | MergeModelError::InconsistentFragments => Err(error),
    }
}

async fn merge_base_with_head(
    repository: &Entity<Repository>,
    other_revision: String,
    cx: &mut gpui::AsyncApp,
) -> Option<String> {
    repository
        .update(cx, |repository, cx| {
            repository.merge_base("HEAD".to_string(), other_revision, cx)
        })
        .await
        .log_err()
        .flatten()
}

async fn original_revision(
    repository: &Entity<Repository>,
    cx: &mut gpui::AsyncApp,
) -> Option<String> {
    for other_head in ["MERGE_HEAD", "CHERRY_PICK_HEAD"] {
        if let Some(revision) = merge_base_with_head(repository, other_head.to_string(), cx).await {
            return Some(revision);
        }
    }
    let replayed_commit = repository
        .update(cx, |repository, cx| repository.rebase_current_commit(cx))
        .await
        .log_err()
        .flatten()?;
    merge_base_with_head(repository, replayed_commit, cx).await
}

async fn better_base_content(
    repository: &Entity<Repository>,
    repo_path: &RepoPath,
    base: Vec<u8>,
    cx: &mut gpui::AsyncApp,
) -> Vec<u8> {
    if !has_merge_conflict_markers(&base) {
        return base;
    }
    let Some(revision) = original_revision(repository, cx).await else {
        return base;
    };
    let loaded = repository
        .update(cx, |repository, cx| {
            repository
                .load_revision_contents(vec![format!("{revision}:{}", repo_path.as_unix_str())], cx)
        })
        .await;
    match loaded {
        Ok(mut contents) => contents.pop().flatten().unwrap_or(base),
        Err(error) => {
            log::info!(
                "could not load a better base revision for {repo_path:?} from {revision}: {error:#}"
            );
            base
        }
    }
}

async fn working_tree_content(
    project: &Entity<Project>,
    repository: &Entity<Repository>,
    repo_path: &RepoPath,
    cx: &mut gpui::AsyncApp,
) -> Vec<u8> {
    let project_path = repository.read_with(cx, |repository, cx| {
        repository.repo_path_to_project_path(repo_path, cx)
    });
    let Some(project_path) = project_path else {
        return Vec::new();
    };
    let buffer = project
        .update(cx, |project, cx| project.open_buffer(project_path, cx))
        .await
        .log_err();
    match buffer {
        Some(buffer) => buffer.read_with(cx, |buffer, _| buffer.text().into_bytes()),
        None => Vec::new(),
    }
}

pub(crate) fn read_working_tree_text(
    context: &ModelContext,
    repo_path: RepoPath,
    cx: &mut App,
) -> Task<String> {
    let project = context.project.clone();
    let repository = context.repository.clone();
    cx.spawn(async move |cx| {
        let bytes = working_tree_content(&project, &repository, &repo_path, cx).await;
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

pub(crate) fn prepare_model(
    context: ModelContext,
    repo_path: RepoPath,
    presence: UnmergedStagePresence,
    cx: &mut App,
) -> Task<Result<PreparedModel>> {
    if let Some(model) = context.holder.read(cx).merge_conflict_model(&repo_path) {
        return Task::ready(Ok(PreparedModel::Model(model.clone())));
    }
    if !presence.ours && !presence.theirs {
        return Task::ready(Ok(PreparedModel::Unsupported(Unsupported::Unavailable)));
    }
    let languages = context.project.read(cx).languages().clone();
    let ignore_policy = stored_ignore_policy(cx);
    cx.spawn(async move |cx| {
        let blobs = context
            .repository
            .update(cx, |repository, cx| {
                repository.load_conflict_blobs(repo_path.clone(), cx)
            })
            .await?;
        if presence.ours && blobs.ours.is_none() || presence.theirs && blobs.theirs.is_none() {
            return Ok(PreparedModel::Unsupported(Unsupported::Unavailable));
        }
        let base_bytes = match blobs.base {
            Some(base) => better_base_content(&context.repository, &repo_path, base, cx).await,
            None => {
                working_tree_content(&context.project, &context.repository, &repo_path, cx).await
            }
        };
        let decoded = decode_text(blobs.ours).and_then(|ours| {
            let base = decode_text(Some(base_bytes))?;
            let theirs = decode_text(blobs.theirs)?;
            Ok((ours, base, theirs))
        });
        let (ours, base, theirs) = match decoded {
            Ok(texts) => texts,
            Err(unsupported) => return Ok(PreparedModel::Unsupported(unsupported)),
        };
        let texts = arrange_sides(&ours, &base, &theirs, context.reversed);
        let language: Option<Arc<Language>> = languages
            .load_language_for_file_path(repo_path.as_std_path())
            .await
            .ok();
        let request = MergeModelRequest {
            texts,
            conflict_type: file_conflict_type(presence, context.reversed),
            ignore_policy,
        };
        let project = context.project.clone();
        let prepared = context
            .holder
            .update(cx, |holder, cx| {
                holder.prepare_model_if_supported(
                    repo_path.clone(),
                    request,
                    move |texts, cx| {
                        MergeBuffers::in_project(texts, language.clone(), &project, cx)
                    },
                    cx,
                )
            })
            .await;
        match prepared {
            Ok(model) => Ok(PreparedModel::Model(model)),
            Err(error) => match model_error_to_unsupported(error) {
                Ok(unsupported) => Ok(PreparedModel::Unsupported(unsupported)),
                Err(error) => Err(anyhow::Error::new(error)),
            },
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge_tool::merge_model::line_separator::LineSeparator;

    fn presence(base: bool, ours: bool, theirs: bool) -> UnmergedStagePresence {
        UnmergedStagePresence { base, ours, theirs }
    }

    #[test]
    fn merge_tool_conflict_type_follows_stage_presence() {
        assert_eq!(
            file_conflict_type(presence(true, true, true), false),
            FileConflictType::Default
        );
        assert_eq!(
            file_conflict_type(presence(false, true, true), false),
            FileConflictType::AddedAdded
        );
        assert_eq!(
            file_conflict_type(presence(true, true, false), false),
            FileConflictType::ModifiedDeleted
        );
        assert_eq!(
            file_conflict_type(presence(true, false, true), false),
            FileConflictType::DeletedModified
        );
    }

    #[test]
    fn merge_tool_conflict_type_swaps_delete_kinds_for_reversed_roots() {
        assert_eq!(
            file_conflict_type(presence(true, true, false), true),
            FileConflictType::DeletedModified
        );
        assert_eq!(
            file_conflict_type(presence(true, false, true), true),
            FileConflictType::ModifiedDeleted
        );
        assert_eq!(
            file_conflict_type(presence(false, true, true), true),
            FileConflictType::AddedAdded
        );
    }

    #[test]
    fn merge_tool_conflict_type_falls_back_to_default_for_exotic_stage_sets() {
        assert_eq!(
            file_conflict_type(presence(false, true, false), false),
            FileConflictType::Default
        );
        assert_eq!(
            file_conflict_type(presence(true, false, false), false),
            FileConflictType::Default
        );
    }

    #[test]
    fn merge_tool_sides_are_swapped_for_reversed_roots() {
        let forward = arrange_sides("ours\n", "base\n", "theirs\n", false);
        assert_eq!(forward.left, "ours\n");
        assert_eq!(forward.base, "base\n");
        assert_eq!(forward.right, "theirs\n");
        let reversed = arrange_sides("ours\n", "base\n", "theirs\n", true);
        assert_eq!(reversed.left, "theirs\n");
        assert_eq!(reversed.base, "base\n");
        assert_eq!(reversed.right, "ours\n");
    }

    #[test]
    fn merge_tool_reversed_roots_move_each_side_line_separator_with_its_text() {
        let forward = arrange_sides("ours\r\n", "base\n", "theirs\n", false);
        assert_eq!(
            forward.line_separators,
            [
                Some(LineSeparator::Crlf),
                Some(LineSeparator::Lf),
                Some(LineSeparator::Lf)
            ]
        );
        let reversed = arrange_sides("ours\r\n", "base\n", "theirs\n", true);
        assert_eq!(
            reversed.line_separators,
            [
                Some(LineSeparator::Lf),
                Some(LineSeparator::Lf),
                Some(LineSeparator::Crlf)
            ]
        );
        assert_eq!(reversed.right, "ours\n");
    }

    #[test]
    fn merge_tool_marker_detection_requires_all_three_patterns() {
        assert!(has_merge_conflict_markers(
            b"a\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> other\n"
        ));
        assert!(!has_merge_conflict_markers(
            b"x\n=======\ny\n>>>>>>> other\n"
        ));
        assert!(!has_merge_conflict_markers(
            b"<<<<<<< HEAD\nx\n>>>>>>> other\n"
        ));
        assert!(!has_merge_conflict_markers(
            b"<<<<<<< HEAD\nx\n=======\ny\n"
        ));
        assert!(!has_merge_conflict_markers(
            b"<<<<<<< HEAD\nx =======\n>>>>>>> other\n"
        ));
        assert!(!has_merge_conflict_markers(b"plain text\n"));
    }

    #[test]
    fn merge_tool_decode_rejects_binary_and_invalid_utf8() {
        assert_eq!(decode_text(None), Ok(String::new()));
        assert_eq!(decode_text(Some(b"abc".to_vec())), Ok("abc".to_string()));
        assert_eq!(
            decode_text(Some(vec![0x61, 0x00, 0x62])),
            Err(Unsupported::Binary)
        );
        assert_eq!(
            decode_text(Some(vec![0xff, 0xfe, 0x41])),
            Err(Unsupported::NotUtf8)
        );
    }

    #[test]
    fn merge_tool_model_errors_map_to_unsupported_reasons() {
        assert_eq!(
            model_error_to_unsupported(MergeModelError::DiffTooBig),
            Ok(Unsupported::TooBig)
        );
        assert_eq!(
            model_error_to_unsupported(MergeModelError::ReadOnlyResult),
            Ok(Unsupported::ReadOnly)
        );
        assert_eq!(
            model_error_to_unsupported(MergeModelError::Canceled),
            Err(MergeModelError::Canceled)
        );
    }
}
