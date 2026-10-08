use git::repository::{RepoPath, UnmergedEntry};
use gpui::{AnyWindowHandle, App, Entity, Task, WeakEntity};
use project::git_store::Repository;
use workspace::Workspace;

use crate::merge_tool::conflict_resolution::dialog_texts::DialogTexts;
use crate::merge_tool::conflict_resolution::rich_text::RichText;

pub(crate) struct ConflictsDialogResult {
    pub(crate) processed_files: Vec<RepoPath>,
    pub(crate) should_finish_merge: bool,
}

pub(crate) struct ConflictsDialogRequest {
    pub(crate) workspace: WeakEntity<Workspace>,
    pub(crate) owner_window: AnyWindowHandle,
    pub(crate) repository: Entity<Repository>,
    pub(crate) entries: Vec<UnmergedEntry>,
    pub(crate) texts: DialogTexts,
    pub(crate) description: Task<RichText>,
    pub(crate) reversed: bool,
    pub(crate) on_closed: Box<dyn FnOnce(ConflictsDialogResult, &mut App)>,
}
