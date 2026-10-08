use crate::diff::FairDiff;
use crate::range::MergeRange;

#[derive(Clone, Copy)]
pub(crate) struct TrueEquality<'a> {
    pub(crate) left: &'a [&'a str],
    pub(crate) base: &'a [&'a str],
    pub(crate) right: &'a [&'a str],
}

struct MergeBuilder<'a> {
    true_equality: Option<TrueEquality<'a>>,
    index: [usize; 3],
    changes: Vec<MergeRange>,
}

impl MergeBuilder<'_> {
    fn add_change(&mut self, start: [usize; 3], end: [usize; 3]) {
        if start[0] == end[0] && start[1] == end[1] && start[2] == end[2] {
            return;
        }
        self.changes.push(MergeRange::new(
            start[0]..end[0],
            start[1]..end[1],
            start[2]..end[2],
        ));
    }

    fn add_ignored_changes(
        &mut self,
        true_equality: TrueEquality,
        start: [usize; 3],
        end: [usize; 3],
    ) {
        let count = end[1] - start[1];
        debug_assert!(end[0] - start[0] == count && end[2] - start[2] == count);
        let mut first_ignored_count: Option<usize> = None;
        for offset in 0..count {
            let base_content = true_equality.base[start[1] + offset];
            let equal = base_content == true_equality.left[start[0] + offset]
                && base_content == true_equality.right[start[2] + offset];
            let is_ignored = !equal;
            if is_ignored && first_ignored_count.is_none() {
                first_ignored_count = Some(offset);
            }
            if !is_ignored && let Some(first) = first_ignored_count.take() {
                self.add_change(
                    [start[0] + first, start[1] + first, start[2] + first],
                    [start[0] + offset, start[1] + offset, start[2] + offset],
                );
            }
        }
        if let Some(first) = first_ignored_count {
            self.add_change(
                [start[0] + first, start[1] + first, start[2] + first],
                [start[0] + count, start[1] + count, start[2] + count],
            );
        }
    }

    fn process_change(&mut self, start: [usize; 3], end: [usize; 3]) {
        if let Some(true_equality) = self.true_equality {
            let last_end = match self.changes.last() {
                Some(last) => [last.left.end, last.base.end, last.right.end],
                None => [0, 0, 0],
            };
            self.add_ignored_changes(true_equality, last_end, start);
        }
        self.add_change(start, end);
    }

    fn mark_equal(&mut self, start: [usize; 3], end: [usize; 3]) {
        debug_assert!(
            self.index[0] <= start[0] && self.index[1] <= start[1] && self.index[2] <= start[2]
        );
        self.process_change(self.index, start);
        self.index = end;
    }
}

pub(crate) fn build_merge(
    base_to_left: &FairDiff,
    base_to_right: &FairDiff,
    true_equality: Option<TrueEquality>,
) -> Vec<MergeRange> {
    debug_assert!(base_to_left.length1 == base_to_right.length1);
    let unchanged1 = base_to_left.unchanged();
    let unchanged2 = base_to_right.unchanged();
    let mut builder = MergeBuilder {
        true_equality,
        index: [0, 0, 0],
        changes: Vec::new(),
    };
    let mut position1 = 0;
    let mut position2 = 0;
    while position1 < unchanged1.len() && position2 < unchanged2.len() {
        let range1 = unchanged1[position1];
        let range2 = unchanged2[position2];
        if range1.end1 <= range2.start1 {
            position1 += 1;
            continue;
        }
        if range2.end1 <= range1.start1 {
            position2 += 1;
            continue;
        }
        let start_base = range1.start1.max(range2.start1);
        let end_base = range1.end1.min(range2.end1);
        let count = end_base - start_base;
        let start_left = range1.start2 + start_base - range1.start1;
        let start_right = range2.start2 + start_base - range2.start1;
        builder.mark_equal(
            [start_left, start_base, start_right],
            [start_left + count, end_base, start_right + count],
        );
        if range1.end1 <= range2.end1 {
            position1 += 1;
        } else {
            position2 += 1;
        }
    }
    debug_assert!(
        builder.index[0] <= base_to_left.length2
            && builder.index[1] <= base_to_left.length1
            && builder.index[2] <= base_to_right.length2
    );
    builder.process_change(
        builder.index,
        [
            base_to_left.length2,
            base_to_left.length1,
            base_to_right.length2,
        ],
    );
    builder.changes
}
