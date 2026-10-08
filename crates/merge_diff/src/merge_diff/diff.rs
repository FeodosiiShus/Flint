use std::collections::{HashMap, HashSet};

use crate::lcs::{Bits, Myers, Patience};
use crate::range::DiffRange;
use crate::{CancellationChecker, ComparisonError};

#[derive(Clone, Debug)]
pub(crate) struct FairDiff {
    pub(crate) changes: Vec<DiffRange>,
    pub(crate) length1: usize,
    pub(crate) length2: usize,
}

impl FairDiff {
    pub(crate) fn new(changes: Vec<DiffRange>, length1: usize, length2: usize) -> Self {
        Self {
            changes,
            length1,
            length2,
        }
    }

    pub(crate) fn from_unchanged(ranges: &[DiffRange], length1: usize, length2: usize) -> Self {
        let mut changes = Vec::new();
        let mut last1 = 0;
        let mut last2 = 0;
        for range in ranges {
            if range.start1 != last1 || range.start2 != last2 {
                changes.push(DiffRange::new(last1, range.start1, last2, range.start2));
            }
            last1 = range.end1;
            last2 = range.end2;
        }
        if last1 != length1 || last2 != length2 {
            changes.push(DiffRange::new(last1, length1, last2, length2));
        }
        Self::new(changes, length1, length2)
    }

    pub(crate) fn unchanged(&self) -> Vec<DiffRange> {
        let mut ranges = Vec::new();
        let mut last1 = 0;
        let mut last2 = 0;
        for change in &self.changes {
            if change.start1 != last1 || change.start2 != last2 {
                ranges.push(DiffRange::new(last1, change.start1, last2, change.start2));
            }
            last1 = change.end1;
            last2 = change.end2;
        }
        if last1 != self.length1 || last2 != self.length2 {
            ranges.push(DiffRange::new(last1, self.length1, last2, self.length2));
        }
        ranges
    }
}

fn discard(needed: &[u32], to_discard: &[u32]) -> (Vec<u32>, Vec<usize>) {
    let needed: HashSet<u32> = needed.iter().copied().collect();
    let mut kept = Vec::new();
    let mut original_indices = Vec::new();
    for (index, value) in to_discard.iter().enumerate() {
        if needed.contains(value) {
            kept.push(*value);
            original_indices.push(index);
        }
    }
    (kept, original_indices)
}

fn enumerate_values(values: &[u32], enumerator: &mut HashMap<u32, u32>) -> Vec<u32> {
    values
        .iter()
        .map(|value| {
            let next = enumerator.len() as u32 + 1;
            *enumerator.entry(*value).or_insert(next)
        })
        .collect()
}

fn increment_changed_range(
    indices: &[usize],
    position: usize,
    changes: &mut Bits,
    length: usize,
) -> usize {
    if position + 1 < indices.len() {
        changes.set_range(indices[position] + 1, indices[position + 1], true);
    } else {
        changes.set_range(indices[position] + 1, length, true);
    }
    position + 1
}

pub(crate) fn build_changes(
    objects1: &[u32],
    objects2: &[u32],
) -> Result<Vec<DiffRange>, ComparisonError> {
    let length1 = objects1.len();
    let length2 = objects2.len();
    let mut start = 0;
    let shorter = length1.min(length2);
    while start < shorter && objects1[start] == objects2[start] {
        start += 1;
    }
    let mut end = 0;
    let remaining = length1.min(length2) - start;
    while end < remaining && objects1[length1 - end - 1] == objects2[length2 - end - 1] {
        end += 1;
    }
    let trimmed1 = length1 - start - end;
    let trimmed2 = length2 - start - end;
    if trimmed1 == 0 || trimmed2 == 0 {
        if trimmed1 != 0 || trimmed2 != 0 {
            return Ok(vec![DiffRange::new(
                start,
                start + trimmed1,
                start,
                start + trimmed2,
            )]);
        }
        return Ok(Vec::new());
    }
    let mut enumerator = HashMap::new();
    let ints1 = enumerate_values(&objects1[start..length1 - end], &mut enumerator);
    let ints2 = enumerate_values(&objects2[start..length2 - end], &mut enumerator);
    let (discarded1, old_indices1) = discard(&ints2, &ints1);
    let (discarded2, old_indices2) = discard(&discarded1, &ints2);
    if discarded1.is_empty() && discarded2.is_empty() {
        return Ok(vec![DiffRange::new(
            start,
            start + ints1.len(),
            start,
            start + ints2.len(),
        )]);
    }
    let mut myers_changes1 = Bits::new(discarded1.len());
    let mut myers_changes2 = Bits::new(discarded2.len());
    let myers_result = Myers::new(
        &discarded1,
        &discarded2,
        0,
        discarded1.len(),
        0,
        discarded2.len(),
        &mut myers_changes1,
        &mut myers_changes2,
    )
    .execute_with_threshold();
    let (changes1, changes2) = match myers_result {
        Ok(()) => (myers_changes1, myers_changes2),
        Err(ComparisonError::DiffTooBig) => {
            let mut patience = Patience::new(&discarded1, &discarded2);
            patience.execute(true)?;
            (patience.changes1, patience.changes2)
        }
        Err(error) => return Err(error),
    };
    let original_length1 = ints1.len();
    let original_length2 = ints2.len();
    let (reindexed1, reindexed2) = if discarded1.len() == original_length1
        && discarded2.len() == original_length2
    {
        (changes1, changes2)
    } else {
        let mut reindexed1 = Bits::default();
        let mut reindexed2 = Bits::default();
        let mut x = 0;
        let mut y = 0;
        while x < discarded1.len() || y < discarded2.len() {
            if x < discarded1.len() && y < discarded2.len() && !changes1.get(x) && !changes2.get(y)
            {
                x = increment_changed_range(&old_indices1, x, &mut reindexed1, original_length1);
                y = increment_changed_range(&old_indices2, y, &mut reindexed2, original_length2);
            } else if changes1.get(x) {
                reindexed1.set_range(old_indices1[x], old_indices1[x] + 1, true);
                x = increment_changed_range(&old_indices1, x, &mut reindexed1, original_length1);
            } else if changes2.get(y) {
                reindexed2.set_range(old_indices2[y], old_indices2[y] + 1, true);
                y = increment_changed_range(&old_indices2, y, &mut reindexed2, original_length2);
            } else {
                break;
            }
        }
        if discarded1.is_empty() {
            reindexed1.set_range(0, original_length1, true);
        } else {
            reindexed1.set_range(0, old_indices1[0], true);
        }
        if discarded2.is_empty() {
            reindexed2.set_range(0, original_length2, true);
        } else {
            reindexed2.set_range(0, old_indices2[0], true);
        }
        (reindexed1, reindexed2)
    };
    let mut result = Vec::new();
    let mut x = 0;
    let mut y = 0;
    let mut shift1 = 0;
    let mut shift2 = 0;
    while x < original_length1 && y < original_length2 {
        let equal_start = x;
        while x < original_length1
            && y < original_length2
            && !reindexed1.get(x)
            && !reindexed2.get(y)
        {
            x += 1;
            y += 1;
        }
        if x > equal_start {
            shift1 += x - equal_start;
            shift2 += x - equal_start;
        }
        let mut deleted = 0;
        let mut inserted = 0;
        while x < original_length1 && reindexed1.get(x) {
            deleted += 1;
            x += 1;
        }
        while y < original_length2 && reindexed2.get(y) {
            inserted += 1;
            y += 1;
        }
        if deleted != 0 || inserted != 0 {
            result.push(DiffRange::new(
                start + shift1,
                start + shift1 + deleted,
                start + shift2,
                start + shift2 + inserted,
            ));
            shift1 += deleted;
            shift2 += inserted;
        }
    }
    if x != original_length1 || y != original_length2 {
        let deleted = original_length1 - x;
        let inserted = original_length2 - y;
        result.push(DiffRange::new(
            start + shift1,
            start + shift1 + deleted,
            start + shift2,
            start + shift2 + inserted,
        ));
    }
    Ok(result)
}

pub(crate) fn diff(
    first: &[u32],
    second: &[u32],
    cancel: &dyn CancellationChecker,
) -> Result<FairDiff, ComparisonError> {
    crate::check_canceled(cancel)?;
    Ok(FairDiff::new(
        build_changes(first, second)?,
        first.len(),
        second.len(),
    ))
}

pub(crate) fn expand(
    sequence1: &[u32],
    sequence2: &[u32],
    start1: usize,
    start2: usize,
    end1: usize,
    end2: usize,
) -> DiffRange {
    let mut start1 = start1;
    let mut start2 = start2;
    let mut end1 = end1;
    let mut end2 = end2;
    while start1 < end1 && start2 < end2 && sequence1[start1] == sequence2[start2] {
        start1 += 1;
        start2 += 1;
    }
    while start1 < end1 && start2 < end2 && sequence1[end1 - 1] == sequence2[end2 - 1] {
        end1 -= 1;
        end2 -= 1;
    }
    DiffRange::new(start1, end1, start2, end2)
}

pub(crate) struct ChangeBuilder<'a> {
    length1: usize,
    length2: usize,
    index1: usize,
    index2: usize,
    changes: Vec<DiffRange>,
    expand_over: Option<(&'a [u32], &'a [u32])>,
}

impl<'a> ChangeBuilder<'a> {
    pub(crate) fn new(length1: usize, length2: usize) -> Self {
        Self {
            length1,
            length2,
            index1: 0,
            index2: 0,
            changes: Vec::new(),
            expand_over: None,
        }
    }

    pub(crate) fn expanding(
        length1: usize,
        length2: usize,
        sequence1: &'a [u32],
        sequence2: &'a [u32],
    ) -> Self {
        Self {
            expand_over: Some((sequence1, sequence2)),
            ..Self::new(length1, length2)
        }
    }

    pub(crate) fn index1(&self) -> usize {
        self.index1
    }

    pub(crate) fn index2(&self) -> usize {
        self.index2
    }

    fn add(&mut self, start1: usize, start2: usize, end1: usize, end2: usize) {
        let mut range = DiffRange::new(start1, end1, start2, end2);
        if let Some((sequence1, sequence2)) = self.expand_over {
            range = expand(sequence1, sequence2, start1, start2, end1, end2);
            if range.is_empty() {
                return;
            }
        }
        self.changes.push(range);
    }

    pub(crate) fn mark_equal(&mut self, index1: usize, index2: usize, count: usize) {
        self.mark_equal_ranges(index1, index2, index1 + count, index2 + count);
    }

    pub(crate) fn mark_equal_ranges(
        &mut self,
        index1: usize,
        index2: usize,
        end1: usize,
        end2: usize,
    ) {
        if index1 == end1 && index2 == end2 {
            return;
        }
        debug_assert!(self.index1 <= index1 && self.index2 <= index2);
        if self.index1 != index1 || self.index2 != index2 {
            self.add(self.index1, self.index2, index1, index2);
        }
        self.index1 = end1;
        self.index2 = end2;
    }

    pub(crate) fn finish(mut self) -> FairDiff {
        if self.length1 != self.index1 || self.length2 != self.index2 {
            self.add(self.index1, self.index2, self.length1, self.length2);
            self.index1 = self.length1;
            self.index2 = self.length2;
        }
        FairDiff::new(self.changes, self.length1, self.length2)
    }
}
