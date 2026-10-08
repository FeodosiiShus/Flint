use std::ops::Range;

use super::{
    line_text::{LineText, count_newlines},
    range_model::to_line,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditKind {
    Insert,
    Delete,
    Replace,
    SetText,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlannedEdit {
    pub(crate) range: Range<usize>,
    pub(crate) new_text: String,
    pub(crate) kind: EditKind,
}

impl PlannedEdit {
    pub(crate) fn insert(offset: usize, new_text: String) -> Self {
        Self {
            range: offset..offset,
            new_text,
            kind: EditKind::Insert,
        }
    }

    pub(crate) fn delete(range: Range<usize>) -> Self {
        Self {
            range,
            new_text: String::new(),
            kind: EditKind::Delete,
        }
    }

    pub(crate) fn replace(range: Range<usize>, new_text: String) -> Self {
        Self {
            range,
            new_text,
            kind: EditKind::Replace,
        }
    }

    pub(crate) fn set_text(current_len: usize, new_text: String) -> Self {
        Self {
            range: 0..current_len,
            new_text,
            kind: EditKind::SetText,
        }
    }

    pub(crate) fn from_buffer_edit(range: Range<usize>, new_text: String) -> Self {
        match (range.is_empty(), new_text.is_empty()) {
            (true, _) => Self::insert(range.start, new_text),
            (false, true) => Self::delete(range),
            (false, false) => Self::replace(range, new_text),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DocumentEvent {
    pub(crate) offset: usize,
    pub(crate) old_len: usize,
    pub(crate) new_text: String,
    pub(crate) old_newlines: usize,
    pub(crate) whole_text_replaced: bool,
}

impl DocumentEvent {
    pub(crate) fn from_edit(document: &str, edit: &PlannedEdit) -> Option<Self> {
        let old_fragment = document.get(edit.range.clone())?;
        let document_len = document.len();
        let (offset, old_len, new_text) = match edit.kind {
            EditKind::Insert | EditKind::Delete => {
                (edit.range.start, old_fragment.len(), edit.new_text.clone())
            }
            EditKind::Replace | EditKind::SetText => {
                let (prefix, suffix) = common_affix_lengths(old_fragment, &edit.new_text);
                let new_end = edit.new_text.len() - suffix;
                (
                    edit.range.start + prefix,
                    old_fragment.len() - prefix - suffix,
                    edit.new_text.get(prefix..new_end)?.to_string(),
                )
            }
        };

        if old_len == 0 && new_text.is_empty() {
            return match edit.kind {
                EditKind::SetText => Some(Self {
                    offset: document_len,
                    old_len: 0,
                    new_text,
                    old_newlines: 0,
                    whole_text_replaced: document_len != 0,
                }),
                EditKind::Insert | EditKind::Delete | EditKind::Replace => None,
            };
        }

        let whole_text_replaced = match edit.kind {
            EditKind::SetText => document_len != 0,
            EditKind::Replace => document_len != 0 && offset == 0 && old_len == document_len,
            EditKind::Insert | EditKind::Delete => false,
        };
        let old_newlines = count_newlines(document.get(offset..offset + old_len)?);

        Some(Self {
            offset,
            old_len,
            new_text,
            old_newlines,
            whole_text_replaced,
        })
    }

    pub(crate) fn range(&self) -> Range<usize> {
        self.offset..self.offset + self.old_len
    }

    pub(crate) fn changes_content(&self) -> bool {
        self.old_len != 0 || !self.new_text.is_empty()
    }

    pub(crate) fn lines_shift(&self) -> i32 {
        to_line(count_newlines(&self.new_text)) - to_line(self.old_newlines)
    }

    pub(crate) fn affected_line_range(&self, document: &LineText) -> (i32, i32) {
        (
            to_line(document.line_of_offset(self.offset)),
            to_line(document.line_of_offset(self.offset + self.old_len)) + 1,
        )
    }
}

fn common_affix_lengths(old: &str, new: &str) -> (usize, usize) {
    let prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(old_char, new_char)| old_char == new_char)
        .map(|(old_char, _)| old_char.len_utf8())
        .sum();
    let suffix: usize = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(old_char, new_char)| old_char == new_char)
        .map(|(old_char, _)| old_char.len_utf8())
        .sum();
    (prefix, suffix)
}

pub(crate) fn plan_modification(
    document: &LineText,
    start_line: usize,
    end_line: usize,
    new_lines: &[String],
) -> Option<PlannedEdit> {
    if start_line == end_line && new_lines.is_empty() {
        return None;
    }
    let joined = new_lines.join("\n");
    if start_line == end_line {
        if start_line >= document.line_count() {
            Some(PlannedEdit::insert(document.len(), format!("\n{joined}")))
        } else {
            Some(PlannedEdit::insert(
                document.line_start(start_line),
                format!("{joined}\n"),
            ))
        }
    } else if new_lines.is_empty() {
        let range = document.lines_range(start_line, end_line, false);
        let mut delete_start = range.start;
        let mut delete_end = range.end;
        if delete_start > 0 {
            delete_start -= 1;
        } else if delete_end < document.len() {
            delete_end += 1;
        }
        Some(PlannedEdit::delete(delete_start..delete_end))
    } else {
        Some(PlannedEdit::replace(
            document.lines_range(start_line, end_line, false),
            joined,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(document: &str, edit: PlannedEdit) -> Option<DocumentEvent> {
        DocumentEvent::from_edit(document, &edit)
    }

    #[test]
    fn replace_trims_common_prefix_and_suffix() {
        let produced = event("abc", PlannedEdit::replace(0..3, "abXc".to_string()));
        assert_eq!(
            produced,
            Some(DocumentEvent {
                offset: 2,
                old_len: 0,
                new_text: "X".to_string(),
                old_newlines: 0,
                whole_text_replaced: false,
            })
        );
    }

    #[test]
    fn replace_with_identical_text_produces_no_event() {
        assert_eq!(
            event("abc", PlannedEdit::replace(0..3, "abc".to_string())),
            None
        );
        assert_eq!(event("abc", PlannedEdit::insert(1, String::new())), None);
        assert_eq!(event("abc", PlannedEdit::delete(1..1)), None);
    }

    #[test]
    fn set_text_with_identical_text_produces_the_degenerate_end_event() {
        assert_eq!(
            event("a\nb", PlannedEdit::set_text(3, "a\nb".to_string())),
            Some(DocumentEvent {
                offset: 3,
                old_len: 0,
                new_text: String::new(),
                old_newlines: 0,
                whole_text_replaced: true,
            })
        );
    }

    #[test]
    fn set_text_on_an_empty_document_is_not_a_whole_text_replacement() {
        let produced = event("", PlannedEdit::set_text(0, "x".to_string()));
        assert_eq!(
            produced,
            Some(DocumentEvent {
                offset: 0,
                old_len: 0,
                new_text: "x".to_string(),
                old_newlines: 0,
                whole_text_replaced: false,
            })
        );
    }

    #[test]
    fn replace_covering_the_whole_text_is_flagged_but_delete_is_not() {
        let replaced = event("abc", PlannedEdit::replace(0..3, "xyz".to_string()));
        assert_eq!(replaced.map(|event| event.whole_text_replaced), Some(true));
        let deleted = event("abc", PlannedEdit::delete(0..3));
        assert_eq!(deleted.map(|event| event.whole_text_replaced), Some(false));
    }

    #[test]
    fn trimming_never_splits_multibyte_characters() {
        let inserted = event("aéb", PlannedEdit::replace(0..4, "aéxb".to_string()));
        assert_eq!(
            inserted.map(|event| (event.offset, event.old_len, event.new_text)),
            Some((3, 0, "x".to_string()))
        );
        let substituted = event("aéb", PlannedEdit::replace(0..4, "aèb".to_string()));
        assert_eq!(
            substituted.map(|event| (event.offset, event.old_len, event.new_text)),
            Some((1, 2, "è".to_string()))
        );
    }

    #[test]
    fn affected_lines_and_shift_follow_the_old_document() {
        let document = LineText::new("a\nb\nc");
        let produced = event(
            document.text(),
            PlannedEdit::replace(2..5, "x\ny\nz\nw".to_string()),
        );
        let produced = produced.expect("event");
        assert_eq!(produced.affected_line_range(&document), (1, 3));
        assert_eq!(produced.lines_shift(), 2);
    }

    #[test]
    fn insert_at_end_event_points_at_the_last_line() {
        let document = LineText::new("a\nb\nc");
        let plan = plan_modification(&document, 3, 3, &["X".to_string()]).expect("plan");
        assert_eq!(plan.range, 5..5);
        assert_eq!(plan.new_text, "\nX");
        let produced = DocumentEvent::from_edit(document.text(), &plan).expect("event");
        assert_eq!(produced.affected_line_range(&document), (2, 3));
        assert_eq!(produced.lines_shift(), 1);
    }

    #[test]
    fn delete_of_the_first_line_eats_the_following_newline() {
        let document = LineText::new("a\nb\nc");
        let plan = plan_modification(&document, 0, 1, &[]).expect("plan");
        assert_eq!(plan.range, 0..2);
        assert_eq!(plan.kind, EditKind::Delete);
    }

    #[test]
    fn delete_of_a_later_line_eats_the_preceding_newline() {
        let document = LineText::new("a\nb\nc");
        let plan = plan_modification(&document, 1, 2, &[]).expect("plan");
        assert_eq!(plan.range, 1..3);
    }

    #[test]
    fn empty_range_with_empty_content_plans_nothing() {
        let document = LineText::new("a\nb");
        assert_eq!(plan_modification(&document, 1, 1, &[]), None);
    }

    #[test]
    fn insert_before_a_line_appends_a_newline_to_the_inserted_text() {
        let document = LineText::new("a\nb\nc");
        let plan =
            plan_modification(&document, 1, 1, &["x".to_string(), "y".to_string()]).expect("plan");
        assert_eq!(plan.range, 2..2);
        assert_eq!(plan.new_text, "x\ny\n");
    }

    #[test]
    fn replacing_with_one_empty_line_keeps_the_line() {
        let document = LineText::new("a\nb\nc");
        let plan = plan_modification(&document, 1, 2, &[String::new()]).expect("plan");
        assert_eq!(plan.range, 2..3);
        assert_eq!(plan.new_text, "");
        assert_eq!(plan.kind, EditKind::Replace);
    }

    #[test]
    fn buffer_edits_become_insertions_deletions_and_replacements() {
        let inserted = event("abc", PlannedEdit::from_buffer_edit(1..1, "Z".to_string()));
        assert_eq!(
            inserted.map(|event| (event.offset, event.old_len, event.new_text)),
            Some((1, 0, "Z".to_string()))
        );
        let deleted = event("abc", PlannedEdit::from_buffer_edit(0..3, String::new()));
        assert_eq!(
            deleted.map(|event| (event.offset, event.old_len, event.whole_text_replaced)),
            Some((0, 3, false))
        );
        let replaced = event(
            "abc",
            PlannedEdit::from_buffer_edit(0..3, "xyz".to_string()),
        );
        assert_eq!(
            replaced.map(|event| (event.offset, event.old_len, event.whole_text_replaced)),
            Some((0, 3, true))
        );
    }

    #[test]
    fn buffer_edits_that_change_nothing_produce_no_event() {
        assert_eq!(
            event("abc", PlannedEdit::from_buffer_edit(1..1, String::new())),
            None
        );
        assert_eq!(
            event(
                "abc",
                PlannedEdit::from_buffer_edit(0..3, "abc".to_string())
            ),
            None
        );
    }

    #[test]
    fn deleting_across_lines_reports_the_removed_newlines_as_a_negative_shift() {
        let document = LineText::new("a\nb\nc");
        let produced = event(document.text(), PlannedEdit::delete(1..4)).expect("event");
        assert_eq!(produced.old_newlines, 2);
        assert_eq!(produced.affected_line_range(&document), (0, 3));
        assert_eq!(produced.lines_shift(), -2);
    }
}
