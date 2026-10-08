#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NavigableChange {
    pub index: usize,
    pub start_line: i64,
    pub end_line: i64,
}

pub(crate) struct DifferenceNavigation<'a> {
    pub changes: &'a [NavigableChange],
    pub caret_line: i64,
    pub line_count: i64,
}

impl DifferenceNavigation<'_> {
    pub(crate) fn can_go_next(&self) -> bool {
        let Some(last_change) = self.changes.last() else {
            return false;
        };
        if self.caret_line == self.line_count - 1 {
            return false;
        }
        last_change.start_line > self.caret_line
    }

    pub(crate) fn next_change(&self) -> Option<NavigableChange> {
        self.changes
            .iter()
            .find(|change| change.start_line > self.caret_line)
            .copied()
    }

    pub(crate) fn can_go_previous(&self) -> bool {
        let Some(first_change) = self.changes.first() else {
            return false;
        };
        if self.caret_line == 0 {
            return false;
        }
        if first_change.end_line > self.caret_line {
            return false;
        }
        first_change.start_line < self.caret_line
    }

    pub(crate) fn previous_change(&self) -> Option<NavigableChange> {
        for (position, change) in self.changes.iter().enumerate() {
            let following = self.changes.get(position + 1);
            let is_last_candidate = match following {
                None => true,
                Some(next) => next.end_line > self.caret_line || next.start_line >= self.caret_line,
            };
            if is_last_candidate {
                return Some(*change);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(index: usize, start_line: i64, end_line: i64) -> NavigableChange {
        NavigableChange {
            index,
            start_line,
            end_line,
        }
    }

    fn navigation(changes: &[NavigableChange], caret_line: i64) -> DifferenceNavigation<'_> {
        DifferenceNavigation {
            changes,
            caret_line,
            line_count: 20,
        }
    }

    #[test]
    fn merge_tool_navigation_without_changes_cannot_move() {
        let navigation = navigation(&[], 5);
        assert!(!navigation.can_go_next());
        assert!(!navigation.can_go_previous());
        assert_eq!(navigation.next_change(), None);
        assert_eq!(navigation.previous_change(), None);
    }

    #[test]
    fn merge_tool_navigation_next_picks_first_change_starting_below_the_caret() {
        let changes = [change(0, 2, 4), change(1, 8, 9), change(2, 12, 14)];
        let navigation = navigation(&changes, 8);
        assert!(navigation.can_go_next());
        assert_eq!(navigation.next_change(), Some(changes[2]));
    }

    #[test]
    fn merge_tool_navigation_next_is_disabled_at_or_after_the_last_change_start() {
        let changes = [change(0, 2, 4), change(1, 12, 14)];
        assert!(!navigation(&changes, 12).can_go_next());
        assert!(!navigation(&changes, 15).can_go_next());
        assert!(navigation(&changes, 11).can_go_next());
        assert_eq!(navigation(&changes, 12).next_change(), None);
    }

    #[test]
    fn merge_tool_navigation_next_is_disabled_on_the_last_document_line() {
        let changes = [change(0, 20, 20)];
        assert!(!navigation(&changes, 19).can_go_next());
        assert!(navigation(&changes, 18).can_go_next());
    }

    #[test]
    fn merge_tool_navigation_previous_is_disabled_on_the_first_line() {
        let changes = [change(0, 2, 4)];
        assert!(!navigation(&changes, 0).can_go_previous());
    }

    #[test]
    fn merge_tool_navigation_previous_is_disabled_while_inside_or_above_the_first_change() {
        let changes = [change(0, 5, 7), change(1, 10, 12)];
        assert!(!navigation(&changes, 6).can_go_previous());
        assert!(!navigation(&changes, 5).can_go_previous());
        assert!(!navigation(&changes, 3).can_go_previous());
        assert!(navigation(&changes, 7).can_go_previous());
    }

    #[test]
    fn merge_tool_navigation_previous_is_disabled_on_an_empty_first_change_at_the_caret_line() {
        let changes = [change(0, 5, 5)];
        assert!(!navigation(&changes, 5).can_go_previous());
        assert!(navigation(&changes, 6).can_go_previous());
    }

    #[test]
    fn merge_tool_navigation_previous_picks_last_change_before_the_caret() {
        let changes = [change(0, 2, 4), change(1, 8, 9), change(2, 12, 14)];
        assert_eq!(navigation(&changes, 12).previous_change(), Some(changes[1]));
        assert_eq!(navigation(&changes, 9).previous_change(), Some(changes[1]));
        assert_eq!(navigation(&changes, 18).previous_change(), Some(changes[2]));
    }

    #[test]
    fn merge_tool_navigation_previous_inside_a_change_picks_the_change_before_it() {
        let changes = [change(0, 2, 4), change(1, 8, 12)];
        assert_eq!(navigation(&changes, 10).previous_change(), Some(changes[0]));
    }

    #[test]
    fn merge_tool_navigation_previous_skips_an_empty_change_on_the_caret_line() {
        let changes = [change(0, 2, 4), change(1, 8, 8)];
        assert_eq!(navigation(&changes, 8).previous_change(), Some(changes[0]));
        assert_eq!(navigation(&changes, 9).previous_change(), Some(changes[1]));
    }
}
