pub(crate) mod customizer;
pub(crate) mod dialog_texts;
pub(crate) mod driver;
pub(crate) mod labels;
pub(crate) mod rich_text;
#[cfg(test)]
mod tests;

pub(crate) use customizer::{RebaseCustomizerSpec, RebaseUpstream};
pub(crate) use driver::{
    ConflictContext, ConflictParams, ResolveMode, ResolveOutcome, ResolverBehavior,
    resolve_conflicts, retain_entries_on_disk, unmerged_files_on_disk,
};
