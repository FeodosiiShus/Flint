use merge_diff::{ComparisonPolicy, Side};

use crate::merge_tool::merge_model::{
    change::ChangeCounters,
    messages::{
        iterative_partially_resolved_confirmation_message, partially_resolved_confirmation_message,
    },
};

pub(crate) const ACCEPT_LEFT_LABEL: &str = "Accept Left";
pub(crate) const ACCEPT_RIGHT_LABEL: &str = "Accept Right";
pub(crate) const APPLY_CHANGES_LABEL: &str = "Apply Changes";
pub(crate) const CANCEL_LABEL: &str = "Cancel";
pub(crate) const SAVE_AND_CLOSE_LABEL: &str = "Save and Close";
pub(crate) const CANCEL_MERGE_ACTION: &str = "Cancel Merge";
pub(crate) const CONTINUE_MERGE_LABEL: &str = "Continue Merge";
pub(crate) const DEFAULT_WINDOW_TITLE: &str = "Merge";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MergeResult {
    Cancel,
    Left,
    Right,
    Resolved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiscardAction {
    AcceptSide(Side),
    CancelMerge,
}

impl DiscardAction {
    pub(crate) fn action_name(self) -> &'static str {
        match self {
            Self::AcceptSide(Side::Left) => ACCEPT_LEFT_LABEL,
            Self::AcceptSide(Side::Right) => ACCEPT_RIGHT_LABEL,
            Self::CancelMerge => CANCEL_MERGE_ACTION,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PendingFinish {
    Discard(DiscardAction),
    UnprocessedChanges { counters: ChangeCounters },
    RevertResolution,
    RestartMerge(ComparisonPolicy),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfirmationIcon {
    Warning,
    Question,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfirmationText {
    pub title: String,
    pub message: String,
    pub yes_label: String,
    pub no_label: String,
    pub icon: Option<ConfirmationIcon>,
}

pub(crate) fn cancel_button_label(iterative: bool) -> &'static str {
    if iterative {
        SAVE_AND_CLOSE_LABEL
    } else {
        CANCEL_LABEL
    }
}

pub(crate) fn needs_discard_confirmation(content_modified: bool) -> bool {
    content_modified
}

pub(crate) fn cancel_needs_confirmation(iterative: bool, content_modified: bool) -> bool {
    !iterative && content_modified
}

pub(crate) fn apply_needs_confirmation(counters: Option<ChangeCounters>) -> bool {
    counters.is_some_and(|counters| counters.changes > 0 || counters.conflicts > 0)
}

pub(crate) fn apply_button_enabled(rediff_running: bool, rediff_failed: bool) -> bool {
    !rediff_running && !rediff_failed
}

pub(crate) fn writes_side_text_on_finish(result: MergeResult) -> Option<Side> {
    match result {
        MergeResult::Left => Some(Side::Left),
        MergeResult::Right => Some(Side::Right),
        MergeResult::Cancel | MergeResult::Resolved => None,
    }
}

pub(crate) fn restores_original_on_finish(result: MergeResult, iterative: bool) -> bool {
    result == MergeResult::Cancel && !iterative
}

pub(crate) fn discard_confirmation(action: DiscardAction) -> ConfirmationText {
    let action_name = action.action_name();
    ConfirmationText {
        title: action_name.to_string(),
        message: format!(
            "There are unsaved changes in the result file. Discard changes and {} anyway?",
            action_name.to_lowercase()
        ),
        yes_label: format!("Discard Changes and {action_name}"),
        no_label: CONTINUE_MERGE_LABEL.to_string(),
        icon: Some(ConfirmationIcon::Question),
    }
}

pub(crate) fn unprocessed_changes_confirmation(
    iterative: bool,
    counters: ChangeCounters,
) -> ConfirmationText {
    if iterative {
        ConfirmationText {
            title: "Unprocessed Changes".to_string(),
            message: iterative_partially_resolved_confirmation_message(
                counters.conflicts,
                counters.changes,
            ),
            yes_label: "Save Current Result".to_string(),
            no_label: "Back to Resolving".to_string(),
            icon: Some(ConfirmationIcon::Warning),
        }
    } else {
        ConfirmationText {
            title: "Apply Changes".to_string(),
            message: partially_resolved_confirmation_message(counters.changes, counters.conflicts),
            yes_label: "Apply Changes and Mark Resolved".to_string(),
            no_label: CONTINUE_MERGE_LABEL.to_string(),
            icon: Some(ConfirmationIcon::Question),
        }
    }
}

pub(crate) fn revert_confirmation() -> ConfirmationText {
    ConfirmationText {
        title: "Confirm Revert".to_string(),
        message:
            "All changes will be reverted. The file will return to its original conflicted state."
                .to_string(),
        yes_label: "Revert".to_string(),
        no_label: "Cancel".to_string(),
        icon: Some(ConfirmationIcon::Question),
    }
}

pub(crate) fn restart_confirmation() -> ConfirmationText {
    ConfirmationText {
        title: "Update Highlighting Settings".to_string(),
        message: "Changing highlighting requires the file merge restart. Discard unsaved changes and restart merge anyway?".to_string(),
        yes_label: "Discard Changes and Restart Merge".to_string(),
        no_label: CONTINUE_MERGE_LABEL.to_string(),
        icon: Some(ConfirmationIcon::Question),
    }
}

pub(crate) fn confirmation_text(pending: PendingFinish, iterative: bool) -> ConfirmationText {
    match pending {
        PendingFinish::Discard(action) => discard_confirmation(action),
        PendingFinish::UnprocessedChanges { counters } => {
            unprocessed_changes_confirmation(iterative, counters)
        }
        PendingFinish::RevertResolution => revert_confirmation(),
        PendingFinish::RestartMerge(_) => restart_confirmation(),
    }
}

pub(crate) fn window_title(request_title: &str) -> String {
    if request_title.is_empty() {
        DEFAULT_WINDOW_TITLE.to_string()
    } else {
        request_title.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counters(changes: usize, conflicts: usize) -> ChangeCounters {
        ChangeCounters { changes, conflicts }
    }

    #[test]
    fn merge_tool_finish_cancel_button_reads_save_and_close_only_when_iterative() {
        assert_eq!(cancel_button_label(true), "Save and Close");
        assert_eq!(cancel_button_label(false), "Cancel");
    }

    #[test]
    fn merge_tool_finish_cancel_asks_only_for_modified_non_iterative_merges() {
        assert!(!cancel_needs_confirmation(true, true));
        assert!(!cancel_needs_confirmation(true, false));
        assert!(!cancel_needs_confirmation(false, false));
        assert!(cancel_needs_confirmation(false, true));
    }

    #[test]
    fn merge_tool_finish_apply_asks_only_while_changes_or_conflicts_remain() {
        assert!(!apply_needs_confirmation(None));
        assert!(!apply_needs_confirmation(Some(counters(0, 0))));
        assert!(apply_needs_confirmation(Some(counters(1, 0))));
        assert!(apply_needs_confirmation(Some(counters(0, 2))));
    }

    #[test]
    fn merge_tool_finish_apply_button_is_disabled_while_rediffing_or_after_failure() {
        assert!(apply_button_enabled(false, false));
        assert!(!apply_button_enabled(true, false));
        assert!(!apply_button_enabled(false, true));
    }

    #[test]
    fn merge_tool_finish_accept_left_discard_dialog_texts() {
        let text = discard_confirmation(DiscardAction::AcceptSide(Side::Left));
        assert_eq!(text.title, "Accept Left");
        assert_eq!(
            text.message,
            "There are unsaved changes in the result file. Discard changes and accept left anyway?"
        );
        assert_eq!(text.yes_label, "Discard Changes and Accept Left");
        assert_eq!(text.no_label, "Continue Merge");
        assert_eq!(text.icon, Some(ConfirmationIcon::Question));
    }

    #[test]
    fn merge_tool_finish_cancel_merge_discard_dialog_texts() {
        let text = discard_confirmation(DiscardAction::CancelMerge);
        assert_eq!(text.title, "Cancel Merge");
        assert_eq!(
            text.message,
            "There are unsaved changes in the result file. Discard changes and cancel merge anyway?"
        );
        assert_eq!(text.yes_label, "Discard Changes and Cancel Merge");
    }

    #[test]
    fn merge_tool_finish_iterative_unprocessed_dialog_uses_conflicts_then_changes() {
        let text = unprocessed_changes_confirmation(true, counters(1, 2));
        assert_eq!(text.title, "Unprocessed Changes");
        assert_eq!(
            text.message,
            "2 unresolved conflicts and 1 unprocessed change still remain. The file will be saved exactly as shown in the Result pane."
        );
        assert_eq!(text.yes_label, "Save Current Result");
        assert_eq!(text.no_label, "Back to Resolving");
        assert_eq!(text.icon, Some(ConfirmationIcon::Warning));
    }

    #[test]
    fn merge_tool_finish_non_iterative_unprocessed_dialog_uses_changes_then_conflicts() {
        let text = unprocessed_changes_confirmation(false, counters(2, 1));
        assert_eq!(text.title, "Apply Changes");
        assert_eq!(
            text.message,
            "There are 2 changes and one conflict left unprocessed.\nSave changes and mark the conflict resolved anyway?"
        );
        assert_eq!(text.yes_label, "Apply Changes and Mark Resolved");
        assert_eq!(text.no_label, "Continue Merge");
        assert_eq!(text.icon, Some(ConfirmationIcon::Question));
    }

    #[test]
    fn merge_tool_finish_revert_dialog_texts() {
        let text = confirmation_text(PendingFinish::RevertResolution, true);
        assert_eq!(text.title, "Confirm Revert");
        assert_eq!(
            text.message,
            "All changes will be reverted. The file will return to its original conflicted state."
        );
        assert_eq!(text.yes_label, "Revert");
        assert_eq!(text.no_label, "Cancel");
        assert_eq!(text.icon, Some(ConfirmationIcon::Question));
    }

    #[test]
    fn merge_tool_finish_restart_dialog_texts() {
        let text = confirmation_text(
            PendingFinish::RestartMerge(ComparisonPolicy::TrimWhitespaces),
            false,
        );
        assert_eq!(text.title, "Update Highlighting Settings");
        assert_eq!(
            text.message,
            "Changing highlighting requires the file merge restart. Discard unsaved changes and restart merge anyway?"
        );
        assert_eq!(text.yes_label, "Discard Changes and Restart Merge");
        assert_eq!(text.no_label, "Continue Merge");
    }

    #[test]
    fn merge_tool_finish_result_application_rules() {
        assert_eq!(
            writes_side_text_on_finish(MergeResult::Left),
            Some(Side::Left)
        );
        assert_eq!(
            writes_side_text_on_finish(MergeResult::Right),
            Some(Side::Right)
        );
        assert_eq!(writes_side_text_on_finish(MergeResult::Resolved), None);
        assert_eq!(writes_side_text_on_finish(MergeResult::Cancel), None);
        assert!(restores_original_on_finish(MergeResult::Cancel, false));
        assert!(!restores_original_on_finish(MergeResult::Cancel, true));
        assert!(!restores_original_on_finish(MergeResult::Resolved, false));
    }

    #[test]
    fn merge_tool_finish_window_title_falls_back_to_merge() {
        assert_eq!(window_title(""), "Merge");
        assert_eq!(
            window_title("Merge Revisions for /tmp/a.txt"),
            "Merge Revisions for /tmp/a.txt"
        );
    }
}
