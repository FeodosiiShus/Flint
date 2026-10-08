use git::repository::RepoPath;
use gpui::{App, AppContext as _, Context, Entity, Task};
use merge_diff::ComparisonPolicy;
use std::collections::BTreeMap;

use super::{FileConflictType, MergeBuffers, MergeConflictModel, MergeModelError, MergeTexts};

#[derive(Clone, Debug)]
pub(crate) struct MergeModelRequest {
    pub(crate) texts: MergeTexts,
    pub(crate) conflict_type: FileConflictType,
    pub(crate) ignore_policy: ComparisonPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileProgress {
    pub(crate) resolved: usize,
    pub(crate) total: usize,
}

impl FileProgress {
    pub(crate) fn is_partially_resolved(&self) -> bool {
        self.resolved > 0 && self.resolved < self.total
    }
}

#[derive(Default)]
pub(crate) struct MergeConflictIterativeDataHolder {
    models: BTreeMap<RepoPath, Entity<MergeConflictModel>>,
}

impl MergeConflictIterativeDataHolder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn merge_conflict_model(
        &self,
        file: &RepoPath,
    ) -> Option<&Entity<MergeConflictModel>> {
        self.models.get(file)
    }

    pub(crate) fn is_file_resolved(&self, file: &RepoPath, cx: &App) -> bool {
        self.models
            .get(file)
            .is_some_and(|model| model.read(cx).is_fully_resolved())
    }

    pub(crate) fn is_file_reviewed(&self, file: &RepoPath, cx: &App) -> bool {
        self.models
            .get(file)
            .is_some_and(|model| model.read(cx).was_reviewed())
    }

    pub(crate) fn file_progress(&self, file: &RepoPath, cx: &App) -> Option<FileProgress> {
        let model = self.models.get(file)?.read(cx);
        Some(FileProgress {
            resolved: model.resolved_changes().len(),
            total: model.changes().len(),
        })
    }

    pub(crate) fn resolved_files_and_models(
        &self,
        cx: &App,
    ) -> Vec<(RepoPath, Entity<MergeConflictModel>)> {
        self.models
            .iter()
            .filter(|(_, model)| model.read(cx).unresolved_changes().is_empty())
            .map(|(file, model)| (file.clone(), model.clone()))
            .collect()
    }

    pub(crate) fn remove_files(&mut self, files: &[RepoPath], cx: &mut Context<Self>) {
        let removed_models: Vec<Entity<MergeConflictModel>> = files
            .iter()
            .filter_map(|file| self.models.remove(file))
            .collect();
        for model in removed_models {
            model.update(cx, |model, cx| {
                model.run_reset_all_changes(cx);
            });
        }
    }

    pub(crate) fn prepare_model_if_supported(
        &mut self,
        file: RepoPath,
        request: MergeModelRequest,
        create_buffers: impl FnOnce(&MergeTexts, &mut App) -> MergeBuffers,
        cx: &mut Context<Self>,
    ) -> Task<Result<Entity<MergeConflictModel>, MergeModelError>> {
        if let Some(model) = self.models.get(&file) {
            return Task::ready(Ok(model.clone()));
        }

        let buffers = create_buffers(&request.texts, cx);
        let model = cx
            .new(|cx| MergeConflictModel::new(buffers, &request.texts, request.conflict_type, cx));
        let rediff = model.update(cx, |model, cx| model.rediff(request.ignore_policy, cx));
        cx.spawn(async move |holder, cx| {
            rediff.await?;
            holder
                .update(cx, |holder, _| {
                    holder.models.entry(file).or_insert(model).clone()
                })
                .map_err(|_| MergeModelError::Canceled)
        })
    }

    pub(crate) fn resolve_auto_resolvable_conflicts(
        &mut self,
        file: &RepoPath,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(model) = self.models.get(file).cloned() else {
            return false;
        };
        model.update(cx, |model, cx| {
            if model.auto_resolvable_changes().is_empty() {
                return false;
            }
            model.resolve_all_changes_automatically(cx);
            true
        })
    }

    pub(crate) fn dispose(&mut self) {
        self.models.clear();
    }

    #[cfg(test)]
    pub(crate) fn insert_model(&mut self, file: RepoPath, model: Entity<MergeConflictModel>) {
        self.models.insert(file, model);
    }
}
