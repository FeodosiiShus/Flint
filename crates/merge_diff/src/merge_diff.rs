#[path = "merge_diff/by_char.rs"]
mod by_char;
#[path = "merge_diff/by_line.rs"]
mod by_line;
#[path = "merge_diff/by_word.rs"]
mod by_word;
#[path = "merge_diff/chunk_optimizer.rs"]
mod chunk_optimizer;
#[path = "merge_diff/diff.rs"]
mod diff;
#[path = "merge_diff/inner.rs"]
mod inner;
#[path = "merge_diff/lcs.rs"]
mod lcs;
#[path = "merge_diff/manager.rs"]
mod manager;
#[path = "merge_diff/merge_builder.rs"]
mod merge_builder;
#[path = "merge_diff/merge_type.rs"]
mod merge_type;
#[path = "merge_diff/range.rs"]
mod range;
#[path = "merge_diff/resolve.rs"]
mod resolve;
#[path = "merge_diff/text.rs"]
mod text;
#[path = "merge_diff/unicode_tables.rs"]
mod unicode_tables;

#[cfg(test)]
#[path = "merge_diff/tests.rs"]
mod tests;

use std::fmt;

pub use inner::{MergeInnerDifferences, compare_threeside_inner, get_inner_chunks};
pub use manager::{
    InnerFragmentsPolicy, LineFragment, compare_chars, compare_lines, compare_lines_inner,
    compare_lines_three_way, compare_lines_with_inner_policy, compare_words, merge_lines,
    word_merge,
};
pub use merge_type::{
    ConflictKind, LineTexts, MergeConflictType, ResolutionStrategy, line_merge_type,
    line_three_way_diff_type,
};
pub use range::{DiffFragment, MergeRange, Side, ThreeSide};
pub use resolve::{try_greedy_resolve, try_resolve, try_resolve_conflict};
pub use text::{ComparisonPolicy, LineOffsets, Utf16Offsets};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ComparisonError {
    DiffTooBig,
    Canceled,
}

impl fmt::Display for ComparisonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComparisonError::DiffTooBig => formatter.write_str(
                "Unable to calculate diff. File is too big and there are too many changes.",
            ),
            ComparisonError::Canceled => formatter.write_str("Comparison was canceled"),
        }
    }
}

impl std::error::Error for ComparisonError {}

pub trait CancellationChecker {
    fn is_canceled(&self) -> bool;
}

pub struct NeverCanceled;

impl CancellationChecker for NeverCanceled {
    fn is_canceled(&self) -> bool {
        false
    }
}

pub(crate) fn check_canceled(cancel: &dyn CancellationChecker) -> Result<(), ComparisonError> {
    if cancel.is_canceled() {
        Err(ComparisonError::Canceled)
    } else {
        Ok(())
    }
}
