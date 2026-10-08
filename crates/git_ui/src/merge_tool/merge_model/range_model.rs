use std::collections::BTreeSet;

pub(crate) fn to_line(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

pub(crate) fn to_index(line: i32) -> usize {
    usize::try_from(line).unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UpdatedLineRange {
    pub(crate) start_line: i32,
    pub(crate) end_line: i32,
    pub(crate) damaged: bool,
}

impl UpdatedLineRange {
    fn new(start_line: i32, end_line: i32, damaged: bool) -> Self {
        Self {
            start_line,
            end_line,
            damaged,
        }
    }
}

pub(crate) fn update_range_on_modification(
    start: i32,
    end: i32,
    change_start: i32,
    change_end: i32,
    shift: i32,
) -> UpdatedLineRange {
    if end <= change_start {
        return UpdatedLineRange::new(start, end, false);
    }
    if start >= change_end {
        return UpdatedLineRange::new(start + shift, end + shift, false);
    }
    if start <= change_start && end >= change_end {
        return UpdatedLineRange::new(start, end + shift, false);
    }

    let new_change_end = change_end + shift;
    if start >= change_start && end <= change_end {
        return UpdatedLineRange::new(new_change_end, new_change_end, true);
    }
    if start < change_start {
        UpdatedLineRange::new(start, change_start, true)
    } else {
        UpdatedLineRange::new(new_change_end, end + shift, true)
    }
}

#[derive(Default)]
pub(crate) struct MergeRangeModel {
    start_lines: Vec<i32>,
    end_lines: Vec<i32>,
    pending_updates: BTreeSet<usize>,
    bulk_depth: u32,
}

impl MergeRangeModel {
    pub(crate) fn set_changes(&mut self, ranges: &[(usize, usize)]) {
        self.start_lines = ranges.iter().map(|(start, _)| to_line(*start)).collect();
        self.end_lines = ranges.iter().map(|(_, end)| to_line(*end)).collect();
    }

    pub(crate) fn len(&self) -> usize {
        self.start_lines.len()
    }

    pub(crate) fn line_start(&self, index: usize) -> i32 {
        self.start_lines.get(index).copied().unwrap_or(0)
    }

    pub(crate) fn line_end(&self, index: usize) -> i32 {
        self.end_lines.get(index).copied().unwrap_or(0)
    }

    pub(crate) fn set_range(&mut self, index: usize, start_line: i32, end_line: i32) {
        if let Some(start) = self.start_lines.get_mut(index) {
            *start = start_line;
        }
        if let Some(end) = self.end_lines.get_mut(index) {
            *end = end_line;
        }
    }

    pub(crate) fn enter_bulk(&mut self) {
        self.bulk_depth += 1;
    }

    pub(crate) fn exit_bulk(&mut self) -> Option<Vec<usize>> {
        self.bulk_depth = self.bulk_depth.saturating_sub(1);
        if self.bulk_depth == 0 {
            Some(
                std::mem::take(&mut self.pending_updates)
                    .into_iter()
                    .collect(),
            )
        } else {
            None
        }
    }

    pub(crate) fn invalidate(&mut self, index: usize) -> bool {
        if self.bulk_depth > 0 {
            self.pending_updates.insert(index);
            false
        } else {
            true
        }
    }

    pub(crate) fn process_document_change(
        &mut self,
        index: usize,
        old_line1: i32,
        old_line2: i32,
        shift: i32,
    ) -> bool {
        let line1 = self.line_start(index);
        let line2 = self.line_end(index);
        let updated = update_range_on_modification(line1, line2, old_line1, old_line2, shift);
        self.set_range(index, updated.start_line, updated.end_line);
        updated.damaged || (old_line2 >= line1 && old_line1 <= line2)
    }

    pub(crate) fn move_changes_after_insertion(
        &mut self,
        index: usize,
        new_start_line: i32,
        new_end_line: i32,
    ) -> Vec<usize> {
        let mut changed = Vec::new();
        if self.line_start(index) != new_start_line || self.line_end(index) != new_end_line {
            self.set_range(index, new_start_line, new_end_line);
            changed.push(index);
        }

        let mut before_change = true;
        for other in 0..self.len() {
            let start_line = self.line_start(other);
            let end_line = self.line_end(other);
            if end_line < new_start_line {
                continue;
            }
            if start_line > new_end_line {
                break;
            }
            if index == other {
                before_change = false;
                continue;
            }

            let (updated_start, updated_end) = if before_change {
                (start_line.min(new_start_line), end_line.min(new_start_line))
            } else {
                (new_end_line, end_line.max(new_end_line))
            };
            if start_line != updated_start || end_line != updated_end {
                self.set_range(other, updated_start, updated_end);
                changed.push(other);
            }
        }
        changed
    }

    pub(crate) fn collect_affected_changes(&self, direct_changes: &[usize]) -> Vec<usize> {
        let mut result = Vec::with_capacity(direct_changes.len());
        let mut direct_position = 0;
        let mut other_index = 0;
        while direct_position < direct_changes.len() && other_index < self.len() {
            let direct_index = direct_changes[direct_position];
            if direct_index == other_index {
                result.push(direct_index);
                other_index += 1;
                continue;
            }

            let direct_start = self.line_start(direct_index);
            let direct_end = self.line_end(direct_index);
            let other_start = self.line_start(other_index);
            let other_end = self.line_end(other_index);

            if other_end < direct_start {
                other_index += 1;
                continue;
            }
            if other_start > direct_end {
                direct_position += 1;
                continue;
            }

            result.push(other_index);
            other_index += 1;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge_tool::merge_model::{
        document_edit::{DocumentEvent, plan_modification},
        line_text::LineText,
    };

    fn range(
        start: i32,
        end: i32,
        change_start: i32,
        change_end: i32,
        shift: i32,
    ) -> (i32, i32, bool) {
        let updated = update_range_on_modification(start, end, change_start, change_end, shift);
        (updated.start_line, updated.end_line, updated.damaged)
    }

    struct Simulation {
        document: LineText,
        model: MergeRangeModel,
    }

    impl Simulation {
        fn new(lines: &[&str], ranges: &[(usize, usize)]) -> Self {
            let mut model = MergeRangeModel::default();
            model.set_changes(ranges);
            Self {
                document: LineText::new(lines.join("\n")),
                model,
            }
        }

        fn edit(&mut self, start_line: usize, end_line: usize, new_lines: &[&str]) {
            let new_lines: Vec<String> = new_lines.iter().map(|line| line.to_string()).collect();
            let Some(plan) = plan_modification(&self.document, start_line, end_line, &new_lines)
            else {
                return;
            };
            let Some(event) = DocumentEvent::from_edit(self.document.text(), &plan) else {
                return;
            };
            let (line1, line2) = event.affected_line_range(&self.document);
            let shift = event.lines_shift();
            for index in 0..self.model.len() {
                self.model
                    .process_document_change(index, line1, line2, shift);
            }
            self.document.replace_range(event.range(), &event.new_text);
        }

        fn replace_change(&mut self, index: usize, new_lines: &[&str]) {
            let start = to_index(self.model.line_start(index));
            let end = to_index(self.model.line_end(index));
            self.edit(start, end, new_lines);
            if start == end {
                self.model.move_changes_after_insertion(
                    index,
                    to_line(start),
                    to_line(start + new_lines.len()),
                );
            }
        }

        fn append_change(&mut self, index: usize, new_lines: &[&str]) {
            let start = to_index(self.model.line_start(index));
            let end = to_index(self.model.line_end(index));
            self.edit(end, end, new_lines);
            self.model.move_changes_after_insertion(
                index,
                to_line(start),
                to_line(end + new_lines.len()),
            );
        }

        fn lines(&self) -> Vec<String> {
            self.document
                .text()
                .split('\n')
                .map(str::to_string)
                .collect()
        }

        fn ranges(&self) -> Vec<(i32, i32)> {
            (0..self.model.len())
                .map(|index| (self.model.line_start(index), self.model.line_end(index)))
                .collect()
        }
    }

    fn lines_of(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn range_before_the_modification_is_untouched() {
        assert_eq!(range(0, 2, 2, 3, 4), (0, 2, false));
        assert_eq!(range(0, 0, 0, 1, 5), (0, 0, false));
    }

    #[test]
    fn range_after_the_modification_is_shifted() {
        assert_eq!(range(3, 5, 1, 3, 2), (5, 7, false));
        assert_eq!(range(3, 3, 1, 3, -1), (2, 2, false));
    }

    #[test]
    fn modification_inside_the_range_moves_only_its_end() {
        assert_eq!(range(1, 5, 2, 3, 4), (1, 9, false));
        assert_eq!(range(1, 5, 1, 5, -2), (1, 3, false));
    }

    #[test]
    fn range_fully_inside_the_modification_collapses_to_its_new_end() {
        assert_eq!(range(2, 3, 1, 5, -1), (4, 4, true));
    }

    #[test]
    fn damaged_boundaries_cut_the_range_at_the_modification() {
        assert_eq!(range(0, 3, 2, 5, 1), (0, 2, true));
        assert_eq!(range(3, 7, 2, 5, -1), (4, 6, true));
    }

    #[test]
    fn empty_ranges_follow_the_branch_order_of_the_original_formula() {
        assert_eq!(range(2, 2, 2, 3, 1), (2, 2, false));
        assert_eq!(range(3, 3, 2, 3, 1), (4, 4, false));
        assert_eq!(range(2, 2, 1, 3, 2), (5, 5, true));
    }

    #[test]
    fn replacing_an_empty_range_widens_it_and_pushes_the_touching_neighbour() {
        let mut simulation = Simulation::new(&["x", "y", "z"], &[(1, 1), (1, 2)]);
        simulation.replace_change(0, &["foo"]);
        assert_eq!(simulation.lines(), lines_of(&["x", "foo", "y", "z"]));
        assert_eq!(simulation.ranges(), vec![(1, 2), (2, 3)]);
    }

    #[test]
    fn replacing_with_more_lines_extends_the_range_and_shifts_followers() {
        let mut simulation = Simulation::new(&["a", "b", "c", "d", "e"], &[(1, 3), (4, 5)]);
        simulation.replace_change(0, &["B", "C", "C2"]);
        assert_eq!(
            simulation.lines(),
            lines_of(&["a", "B", "C", "C2", "d", "e"])
        );
        assert_eq!(simulation.ranges(), vec![(1, 4), (5, 6)]);
    }

    #[test]
    fn replacing_with_no_lines_collapses_the_range() {
        let mut simulation = Simulation::new(&["a", "b", "c", "d", "e"], &[(1, 3), (4, 5)]);
        simulation.replace_change(0, &[]);
        assert_eq!(simulation.lines(), lines_of(&["a", "d", "e"]));
        assert_eq!(simulation.ranges(), vec![(1, 1), (2, 3)]);
    }

    #[test]
    fn replacing_the_whole_document_with_one_more_line_grows_the_range() {
        let mut simulation = Simulation::new(&["a", "b", "c"], &[(0, 3)]);
        simulation.replace_change(0, &["a", "b", "c", "d"]);
        assert_eq!(simulation.lines(), lines_of(&["a", "b", "c", "d"]));
        assert_eq!(simulation.ranges(), vec![(0, 4)]);
    }

    #[test]
    fn replacing_an_empty_range_after_the_last_line_appends_at_the_end() {
        let mut simulation = Simulation::new(&["a", "b", "c"], &[(3, 3)]);
        simulation.replace_change(0, &["X"]);
        assert_eq!(simulation.lines(), lines_of(&["a", "b", "c", "X"]));
        assert_eq!(simulation.ranges(), vec![(3, 4)]);
    }

    #[test]
    fn appending_to_an_empty_range_fills_it() {
        let mut simulation = Simulation::new(&["a", "b", "c"], &[(1, 1)]);
        simulation.append_change(0, &["X"]);
        assert_eq!(simulation.lines(), lines_of(&["a", "X", "b", "c"]));
        assert_eq!(simulation.ranges(), vec![(1, 2)]);
    }

    #[test]
    fn appending_pushes_the_following_change_down() {
        let mut simulation = Simulation::new(&["a", "b", "c"], &[(0, 1), (2, 3)]);
        simulation.append_change(0, &["X"]);
        assert_eq!(simulation.lines(), lines_of(&["a", "X", "b", "c"]));
        assert_eq!(simulation.ranges(), vec![(0, 2), (3, 4)]);
    }

    #[test]
    fn deleting_the_first_line_leaves_the_deleted_change_at_the_next_line() {
        let mut simulation = Simulation::new(&["a", "b", "c"], &[(0, 1), (2, 3)]);
        simulation.replace_change(0, &[]);
        assert_eq!(simulation.lines(), lines_of(&["b", "c"]));
        assert_eq!(simulation.ranges(), vec![(1, 1), (1, 2)]);
    }

    #[test]
    fn deleting_a_line_next_to_a_touching_change_damages_the_neighbour() {
        let mut simulation = Simulation::new(&["a", "b", "c", "d"], &[(0, 1), (2, 3), (3, 4)]);
        simulation.edit(1, 2, &[]);
        assert_eq!(simulation.lines(), lines_of(&["a", "c", "d"]));
        assert_eq!(simulation.ranges(), vec![(1, 1), (1, 2), (2, 3)]);
    }

    #[test]
    fn deletion_spanning_two_changes_collapses_both() {
        let mut simulation = Simulation::new(&["a", "b", "c", "d", "e"], &[(1, 2), (3, 4)]);
        simulation.edit(1, 4, &[]);
        assert_eq!(simulation.lines(), lines_of(&["a", "e"]));
        assert_eq!(simulation.ranges(), vec![(1, 1), (1, 1)]);
    }

    #[test]
    fn insertion_before_a_change_shifts_it() {
        let mut simulation = Simulation::new(&["a", "b", "c"], &[(2, 3)]);
        simulation.edit(0, 0, &["new"]);
        assert_eq!(simulation.lines(), lines_of(&["new", "a", "b", "c"]));
        assert_eq!(simulation.ranges(), vec![(3, 4)]);
    }

    #[test]
    fn insertion_inside_a_change_extends_it() {
        let mut simulation = Simulation::new(&["a", "b", "c", "d"], &[(1, 3)]);
        simulation.edit(2, 2, &["new"]);
        assert_eq!(simulation.lines(), lines_of(&["a", "b", "new", "c", "d"]));
        assert_eq!(simulation.ranges(), vec![(1, 4)]);
    }

    #[test]
    fn insertion_after_a_change_leaves_it_untouched() {
        let mut simulation = Simulation::new(&["a", "b", "c", "d"], &[(0, 1)]);
        simulation.edit(3, 3, &["new"]);
        assert_eq!(simulation.lines(), lines_of(&["a", "b", "c", "new", "d"]));
        assert_eq!(simulation.ranges(), vec![(0, 1)]);
    }

    #[test]
    fn insertion_at_the_start_of_a_touching_range_is_absorbed_by_the_following_change() {
        let mut simulation = Simulation::new(&["a", "b", "c"], &[(0, 1), (1, 2)]);
        simulation.edit(1, 1, &["new"]);
        assert_eq!(simulation.lines(), lines_of(&["a", "new", "b", "c"]));
        assert_eq!(simulation.ranges(), vec![(0, 1), (1, 3)]);
    }

    #[test]
    fn processing_reports_whether_the_range_was_touched_and_moves_it() {
        let mut model = MergeRangeModel::default();
        model.set_changes(&[(1, 2), (5, 6)]);
        assert!(!model.process_document_change(0, 3, 4, 1));
        assert_eq!((model.line_start(0), model.line_end(0)), (1, 2));
        assert!(!model.process_document_change(1, 3, 4, 1));
        assert_eq!((model.line_start(1), model.line_end(1)), (6, 7));
        assert!(model.process_document_change(0, 2, 3, 1));
        assert_eq!((model.line_start(0), model.line_end(0)), (1, 2));
    }

    #[test]
    fn collect_affected_changes_adds_touching_neighbours() {
        let mut model = MergeRangeModel::default();
        model.set_changes(&[(0, 1), (1, 2), (4, 5), (5, 6), (9, 10)]);
        assert_eq!(model.collect_affected_changes(&[0]), vec![0, 1]);
        assert_eq!(model.collect_affected_changes(&[2]), vec![2, 3]);
        assert_eq!(model.collect_affected_changes(&[1, 4]), vec![0, 1, 4]);
        assert_eq!(model.collect_affected_changes(&[]), Vec::<usize>::new());
    }

    #[test]
    fn bulk_blocks_defer_invalidation_until_the_outermost_exit() {
        let mut model = MergeRangeModel::default();
        model.set_changes(&[(0, 1), (1, 2)]);
        assert!(model.invalidate(0));
        model.enter_bulk();
        model.enter_bulk();
        assert!(!model.invalidate(1));
        assert!(!model.invalidate(0));
        assert!(!model.invalidate(1));
        assert_eq!(model.exit_bulk(), None);
        assert_eq!(model.exit_bulk(), Some(vec![0, 1]));
        assert!(model.invalidate(0));
    }
}
