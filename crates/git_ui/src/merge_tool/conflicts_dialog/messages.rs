pub(crate) const LOADING_DESCRIPTION: &str = "Loading merge details\u{2026}";
pub(crate) const ALL_RESOLVED_DESCRIPTION: &str = "All conflicts have been resolved";
pub(crate) const RESOLVE_ALL_SIMPLE_CONFLICTS: &str = "Resolve All Simple Conflicts";
pub(crate) const RESOLVING_CONFLICTS: &str = "Resolving Conflicts\u{2026}";
pub(crate) const RESOLVE_ALL_DISABLED_TOOLTIP: &str = "There are no simple conflicts to resolve";
pub(crate) const RESOLVE_ALL_TOOLTIP: &str = "Apply all non-conflicting changes and resolve simple conflicts (ones that can be resolved through static analysis) across all files";
pub(crate) const VIEW_OPTIONS: &str = "View Options";
pub(crate) const GROUP_BY_DIRECTORY: &str = "Group by directory";
pub(crate) const ACCEPT: &str = "Accept";
pub(crate) const CLOSE: &str = "Close";
pub(crate) const ACCEPT_AND_FINISH: &str = "Accept and Finish";
pub(crate) const RESOLVE_MANUALLY: &str = "Resolve Manually";
pub(crate) const REVIEW_CHANGES: &str = "Review Changes";
pub(crate) const REVERT_CONFLICT_RESOLUTION: &str = "Revert Conflict Resolution";
pub(crate) const UNRESOLVED_GROUP: &str = "Unresolved";
pub(crate) const RESOLVED_GROUP: &str = "Resolved";
pub(crate) const MODIFIED: &str = "Modified";
pub(crate) const DELETED: &str = "Deleted";
pub(crate) const THIN_SPACE_SEPARATOR: &str = " \u{2009}";

pub(crate) const CANCEL: &str = "Cancel";
pub(crate) const OVERWRITE_TITLE: &str = "Overwrite Changes";
pub(crate) const OVERWRITE_YES: &str = "Discard and Accept";
pub(crate) const REVERT_TITLE: &str = "Confirm Revert";
pub(crate) const REVERT_YES: &str = "Revert";
pub(crate) const CLOSE_TITLE: &str = "Discard Changes?";
pub(crate) const CLOSE_MESSAGE: &str =
    "Closing the dialog will discard your changes in partially resolved files";
pub(crate) const CLOSE_YES: &str = "Discard Changes";
pub(crate) const CLOSE_NO: &str = "Continue Merge";

pub(crate) const LOADING_REVISIONS: &str = "Loading Revisions\u{2026}";
pub(crate) const APPLYING_RESOLUTIONS: &str = "Applying Resolutions\u{2026}";
pub(crate) const RESOLVING_SIMPLE_CONFLICTS: &str = "Resolving simple conflicts\u{2026}";
pub(crate) const NO_CONFLICTS_RESOLVED: &str = "No conflicts were resolved automatically";
pub(crate) const ALL_CONFLICTS_RESOLVED: &str = "All conflicts were resolved automatically";
pub(crate) const CANNOT_SHOW_MERGE_DIALOG: &str = "Cannot Show Merge Dialog";
pub(crate) const READ_ONLY_FILE: &str = "Cannot resolve conflicts in a read-only file";
pub(crate) const BINARY_FILE_CANNOT_BE_MERGED: &str = "Binary files cannot be merged manually";
pub(crate) const NOT_A_REGULAR_FILE: &str = "the conflicted path is not a regular text file";
pub(crate) const ERROR_TITLE: &str = "Error";
pub(crate) const FILTER_BY_CONFLICTED_FILE: &str = "Filter by conflicted file";
pub(crate) const COLLECTING_COMMIT_DETAILS: &str = "Collecting Commits Details\u{2026}";

fn singular_below_two(value: usize) -> bool {
    value < 2
}

fn grouped(value: usize) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

pub(crate) fn files_count(count: usize) -> String {
    match count {
        0 => "0 files".to_string(),
        1 => "1 file".to_string(),
        count => format!("{} files", grouped(count)),
    }
}

pub(crate) fn badge_value(resolved: usize, total: usize) -> String {
    format!("{}/{}", grouped(resolved), grouped(total))
}

pub(crate) fn merged_changes_tooltip(resolved: usize, total: usize) -> String {
    match resolved {
        0 => changes_count(total),
        1 => "1 change has been merged".to_string(),
        resolved => format!("{} changes have been merged", grouped(resolved)),
    }
}

fn changes_count(total: usize) -> String {
    if singular_below_two(total) {
        "1 change".to_string()
    } else {
        format!("{} changes", grouped(total))
    }
}

fn conflicts_count(count: usize) -> String {
    if singular_below_two(count) {
        "1 conflict".to_string()
    } else {
        format!("{} conflicts", grouped(count))
    }
}

fn files_in_status(count: usize) -> String {
    if singular_below_two(count) {
        "1 file".to_string()
    } else {
        format!("{} files", grouped(count))
    }
}

pub(crate) fn partially_resolved_status(
    resolved_by_auto_resolve: usize,
    total_unresolved: usize,
    files_with_unresolved: usize,
) -> String {
    let verb = if singular_below_two(total_unresolved) {
        "requires"
    } else {
        "require"
    };
    format!(
        "{} resolved. {} in {} still {verb} attention",
        conflicts_count(resolved_by_auto_resolve),
        conflicts_count(total_unresolved),
        files_in_status(files_with_unresolved),
    )
}

pub(crate) fn auto_resolve_status(
    resolved_by_auto_resolve: usize,
    total_unresolved: usize,
    files_with_unresolved: usize,
) -> Option<String> {
    if resolved_by_auto_resolve == 0 {
        Some(NO_CONFLICTS_RESOLVED.to_string())
    } else if total_unresolved == 0 {
        Some(ALL_CONFLICTS_RESOLVED.to_string())
    } else {
        Some(partially_resolved_status(
            resolved_by_auto_resolve,
            total_unresolved,
            files_with_unresolved,
        ))
    }
}

pub(crate) fn accept_menu_label(side_label: &str) -> String {
    format!("Accept \u{2018}{side_label}\u{2019}")
}

pub(crate) fn overwrite_message(files_with_model: usize, side_label: &str) -> String {
    let subject = if singular_below_two(files_with_model) {
        "This file already contains merged changes"
    } else {
        "These files already contain merged changes"
    };
    format!(
        "{subject}. Accepting changes from \u{2018}{side_label}\u{2019} will discard and overwrite them."
    )
}

pub(crate) fn revert_message(selected_files: usize) -> String {
    let subject = if singular_below_two(selected_files) {
        "file will return to its"
    } else {
        "selected files will return to their"
    };
    format!("All changes will be reverted. The {subject} original conflicted state.")
}

pub(crate) fn resolving_file_progress(index: usize, total: usize) -> String {
    format!("File {} of {}", grouped(index), grouped(total))
}

pub(crate) fn error_loading_revisions(message: &str) -> String {
    format!("Error loading revisions to merge: {message}")
}

pub(crate) fn error_saving_merged_data(message: &str) -> String {
    format!("Error saving merged data: {message}")
}

pub(crate) fn merge_window_title(system_path: &str) -> String {
    format!("Merge Revisions for {system_path}")
}

pub(crate) fn column_status_text(present: bool) -> &'static str {
    if present { MODIFIED } else { DELETED }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_tool_files_count_follows_choice_format() {
        assert_eq!(files_count(0), "0 files");
        assert_eq!(files_count(1), "1 file");
        assert_eq!(files_count(2), "2 files");
        assert_eq!(files_count(17), "17 files");
    }

    #[test]
    fn merge_tool_badge_value_joins_resolved_and_total() {
        assert_eq!(badge_value(0, 0), "0/0");
        assert_eq!(badge_value(1, 3), "1/3");
    }

    #[test]
    fn merge_tool_counts_use_digit_grouping_like_message_format() {
        assert_eq!(files_count(1000), "1,000 files");
        assert_eq!(files_count(12345), "12,345 files");
        assert_eq!(badge_value(1000, 2000), "1,000/2,000");
        assert_eq!(merged_changes_tooltip(0, 1000), "1,000 changes");
        assert_eq!(
            merged_changes_tooltip(1234, 5),
            "1,234 changes have been merged"
        );
        assert_eq!(
            partially_resolved_status(1000, 2, 3),
            "1,000 conflicts resolved. 2 conflicts in 3 files still require attention"
        );
    }

    #[test]
    fn merge_tool_tooltip_formats_nested_choice_for_zero_resolved() {
        assert_eq!(merged_changes_tooltip(0, 1), "1 change");
        assert_eq!(merged_changes_tooltip(0, 2), "2 changes");
        assert_eq!(merged_changes_tooltip(0, 9), "9 changes");
        assert_eq!(merged_changes_tooltip(0, 0), "1 change");
    }

    #[test]
    fn merge_tool_tooltip_reports_merged_changes() {
        assert_eq!(merged_changes_tooltip(1, 4), "1 change has been merged");
        assert_eq!(merged_changes_tooltip(2, 4), "2 changes have been merged");
        assert_eq!(merged_changes_tooltip(5, 5), "5 changes have been merged");
    }

    #[test]
    fn merge_tool_partial_status_matches_spec_examples() {
        assert_eq!(
            partially_resolved_status(2, 3, 1),
            "2 conflicts resolved. 3 conflicts in 1 file still require attention"
        );
        assert_eq!(
            partially_resolved_status(1, 1, 1),
            "1 conflict resolved. 1 conflict in 1 file still requires attention"
        );
        assert_eq!(
            partially_resolved_status(4, 2, 2),
            "4 conflicts resolved. 2 conflicts in 2 files still require attention"
        );
    }

    #[test]
    fn merge_tool_auto_resolve_status_selects_variant_by_counts() {
        assert_eq!(
            auto_resolve_status(0, 5, 2).as_deref(),
            Some(NO_CONFLICTS_RESOLVED)
        );
        assert_eq!(
            auto_resolve_status(3, 0, 0).as_deref(),
            Some(ALL_CONFLICTS_RESOLVED)
        );
        assert_eq!(
            auto_resolve_status(1, 2, 1).as_deref(),
            Some("1 conflict resolved. 2 conflicts in 1 file still require attention")
        );
    }

    #[test]
    fn merge_tool_accept_labels_use_typographic_quotes() {
        assert_eq!(
            accept_menu_label("Yours (main)"),
            "Accept \u{2018}Yours (main)\u{2019}"
        );
    }

    #[test]
    fn merge_tool_overwrite_message_pluralizes_subject() {
        assert_eq!(
            overwrite_message(1, "Theirs (feature)"),
            "This file already contains merged changes. Accepting changes from \u{2018}Theirs (feature)\u{2019} will discard and overwrite them."
        );
        assert_eq!(
            overwrite_message(3, "Yours"),
            "These files already contain merged changes. Accepting changes from \u{2018}Yours\u{2019} will discard and overwrite them."
        );
    }

    #[test]
    fn merge_tool_revert_message_pluralizes_subject() {
        assert_eq!(
            revert_message(1),
            "All changes will be reverted. The file will return to its original conflicted state."
        );
        assert_eq!(
            revert_message(4),
            "All changes will be reverted. The selected files will return to their original conflicted state."
        );
    }

    #[test]
    fn merge_tool_titles_and_errors_match_bundle_texts() {
        assert_eq!(
            merge_window_title("/work/src/a.rs"),
            "Merge Revisions for /work/src/a.rs"
        );
        assert_eq!(
            error_loading_revisions("boom"),
            "Error loading revisions to merge: boom"
        );
        assert_eq!(
            error_saving_merged_data("boom"),
            "Error saving merged data: boom"
        );
        assert_eq!(resolving_file_progress(2, 5), "File 2 of 5");
    }

    #[test]
    fn merge_tool_column_status_depends_on_stage_presence() {
        assert_eq!(column_status_text(true), "Modified");
        assert_eq!(column_status_text(false), "Deleted");
    }
}
