use std::ops::Range;
use tree_sitter::{Query, QueryMatch};

use crate::MigrationPatterns;
use crate::patterns::{SETTINGS_NESTED_KEY_VALUE_PATTERN, SETTINGS_ROOT_KEY_VALUE_PATTERN};

pub const SETTINGS_PATTERNS: MigrationPatterns = &[
    (
        SETTINGS_ROOT_KEY_VALUE_PATTERN,
        rename_root_restore_on_startup_values,
    ),
    (
        SETTINGS_NESTED_KEY_VALUE_PATTERN,
        rename_nested_restore_on_startup_values,
    ),
];

fn rename_root_restore_on_startup_values(
    contents: &str,
    mat: &QueryMatch,
    query: &Query,
) -> Option<(Range<usize>, String)> {
    rename_restore_on_startup_values(contents, mat, query, "name", "value")
}

fn rename_nested_restore_on_startup_values(
    contents: &str,
    mat: &QueryMatch,
    query: &Query,
) -> Option<(Range<usize>, String)> {
    rename_restore_on_startup_values(contents, mat, query, "setting_name", "setting_value")
}

fn rename_restore_on_startup_values(
    contents: &str,
    mat: &QueryMatch,
    query: &Query,
    name_capture: &str,
    value_capture: &str,
) -> Option<(Range<usize>, String)> {
    let setting_name_ix = query.capture_index_for_name(name_capture)?;
    let setting_name_range = mat
        .nodes_for_capture_index(setting_name_ix)
        .next()?
        .byte_range();
    if contents.get(setting_name_range)? != "restore_on_startup" {
        return None;
    }

    let setting_value_ix = query.capture_index_for_name(value_capture)?;
    let setting_value_range = mat
        .nodes_for_capture_index(setting_value_ix)
        .next()?
        .byte_range();
    let setting_value = contents.get(setting_value_range.clone())?;

    // The value includes quotes, so we check for the quoted string
    let new_value = match setting_value.trim() {
        "\"none\"" => "\"empty_tab\"",
        "\"welcome\"" => "\"launchpad\"",
        _ => return None,
    };

    Some((setting_value_range, new_value.to_string()))
}
