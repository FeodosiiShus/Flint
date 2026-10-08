use std::collections::BTreeSet;

use editor::Editor;
use gpui::{App, Entity};
use language::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CaretSnapshot {
    pub start_line: usize,
    pub end_line: usize,
    pub has_selection: bool,
    pub end_at_document_end: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SelectedLines {
    lines: BTreeSet<usize>,
}

impl SelectedLines {
    pub(crate) fn from_carets(carets: &[CaretSnapshot], total_lines: usize) -> Self {
        let mut lines = BTreeSet::new();
        for caret in carets {
            if caret.has_selection {
                lines.extend(caret.start_line..=caret.end_line);
            } else {
                let last_line = caret.end_line.max(caret.start_line + 1);
                lines.extend(caret.start_line..last_line);
            }
            if caret.end_at_document_end {
                lines.insert(total_lines);
            }
        }
        Self { lines }
    }

    pub(crate) fn selects_range(&self, start_line: usize, end_line: usize) -> bool {
        if start_line == end_line {
            return self.lines.contains(&start_line);
        }
        self.lines.range(start_line..end_line).next().is_some()
    }
}

pub(crate) fn is_some_range_selected(
    carets: &[CaretSnapshot],
    selected: &SelectedLines,
    condition: impl Fn(&SelectedLines) -> bool,
) -> bool {
    if carets.len() != 1 {
        return true;
    }
    if carets[0].has_selection {
        return true;
    }
    condition(selected)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditorSelection {
    pub carets: Vec<CaretSnapshot>,
    pub lines: SelectedLines,
}

impl EditorSelection {
    pub(crate) fn is_some_range_selected(
        &self,
        condition: impl Fn(&SelectedLines) -> bool,
    ) -> bool {
        is_some_range_selected(&self.carets, &self.lines, condition)
    }
}

pub(crate) fn read_editor_selection(editor: &Entity<Editor>, cx: &mut App) -> EditorSelection {
    editor.update(cx, |editor, cx| selection_of_editor(editor, cx))
}

pub(crate) fn selection_of_editor(editor: &mut Editor, cx: &mut App) -> EditorSelection {
    let display_snapshot = editor.display_snapshot(cx);
    let buffer_snapshot = display_snapshot.buffer_snapshot();
    let document_end = buffer_snapshot.max_point();
    let total_lines = document_end.row as usize + 1;
    let selections = editor.selections.all::<Point>(&display_snapshot);
    let carets: Vec<CaretSnapshot> = selections
        .iter()
        .map(|selection| {
            let start = selection.start.min(selection.end);
            let end = selection.start.max(selection.end);
            CaretSnapshot {
                start_line: start.row as usize,
                end_line: end.row as usize,
                has_selection: start != end,
                end_at_document_end: end == document_end,
            }
        })
        .collect();
    let lines = SelectedLines::from_carets(&carets, total_lines);
    EditorSelection { carets, lines }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caret(start_line: usize, end_line: usize, has_selection: bool) -> CaretSnapshot {
        CaretSnapshot {
            start_line,
            end_line,
            has_selection,
            end_at_document_end: false,
        }
    }

    #[test]
    fn merge_tool_selection_caret_without_selection_selects_its_line() {
        let carets = [caret(4, 4, false)];
        let lines = SelectedLines::from_carets(&carets, 10);
        assert!(lines.selects_range(4, 5));
        assert!(!lines.selects_range(5, 7));
        assert!(!lines.selects_range(2, 4));
    }

    #[test]
    fn merge_tool_selection_range_selection_includes_both_boundary_lines() {
        let carets = [caret(3, 6, true)];
        let lines = SelectedLines::from_carets(&carets, 10);
        assert!(lines.selects_range(6, 8));
        assert!(lines.selects_range(0, 4));
        assert!(!lines.selects_range(0, 3));
        assert!(!lines.selects_range(7, 9));
    }

    #[test]
    fn merge_tool_selection_empty_range_is_selected_only_on_its_own_line() {
        let carets = [caret(3, 3, false)];
        let lines = SelectedLines::from_carets(&carets, 10);
        assert!(lines.selects_range(3, 3));
        assert!(!lines.selects_range(4, 4));
    }

    #[test]
    fn merge_tool_selection_caret_at_document_end_also_selects_the_line_past_the_end() {
        let carets = [CaretSnapshot {
            start_line: 9,
            end_line: 9,
            has_selection: false,
            end_at_document_end: true,
        }];
        let lines = SelectedLines::from_carets(&carets, 10);
        assert!(lines.selects_range(10, 10));
        assert!(lines.selects_range(10, 12));
    }

    #[test]
    fn merge_tool_selection_selection_reaching_document_end_also_selects_the_line_past_the_end() {
        let carets = [CaretSnapshot {
            start_line: 3,
            end_line: 6,
            has_selection: true,
            end_at_document_end: true,
        }];
        let lines = SelectedLines::from_carets(&carets, 10);
        assert!(lines.selects_range(10, 10));
        assert!(!lines.selects_range(7, 10));
    }

    #[test]
    fn merge_tool_selection_multiple_carets_select_the_union_of_their_lines() {
        let carets = [caret(1, 1, false), caret(5, 5, false)];
        let lines = SelectedLines::from_carets(&carets, 10);
        assert!(lines.selects_range(1, 2));
        assert!(lines.selects_range(5, 6));
        assert!(lines.selects_range(2, 6));
        assert!(!lines.selects_range(2, 5));
    }

    #[test]
    fn merge_tool_selection_multiple_carets_or_selections_always_count_as_selected() {
        let two_carets = [caret(1, 1, false), caret(5, 5, false)];
        let lines = SelectedLines::from_carets(&two_carets, 10);
        assert!(is_some_range_selected(&two_carets, &lines, |_| false));

        let selection = [caret(1, 3, true)];
        let selected = SelectedLines::from_carets(&selection, 10);
        assert!(is_some_range_selected(&selection, &selected, |_| false));
    }

    #[test]
    fn merge_tool_selection_single_caret_defers_to_the_condition() {
        let single = [caret(2, 2, false)];
        let lines = SelectedLines::from_carets(&single, 10);
        assert!(is_some_range_selected(&single, &lines, |selected| {
            selected.selects_range(2, 3)
        }));
        assert!(!is_some_range_selected(&single, &lines, |selected| {
            selected.selects_range(6, 8)
        }));
    }
}
