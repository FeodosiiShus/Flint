use std::collections::HashMap;

use crate::ComparisonError;

const MYERS_BASE_THRESHOLD: i64 = 20000;

#[derive(Clone, Debug, Default)]
pub(crate) struct Bits {
    bits: Vec<bool>,
}

impl Bits {
    pub(crate) fn new(length: usize) -> Self {
        Self {
            bits: vec![false; length],
        }
    }

    pub(crate) fn set_range(&mut self, start: usize, end: usize, value: bool) {
        if start >= end {
            return;
        }
        if end > self.bits.len() {
            self.bits.resize(end, false);
        }
        self.bits[start..end].fill(value);
    }

    pub(crate) fn get(&self, index: usize) -> bool {
        self.bits.get(index).copied().unwrap_or(false)
    }
}

pub(crate) struct Myers<'a> {
    first: &'a [u32],
    second: &'a [u32],
    start1: usize,
    count1: usize,
    start2: usize,
    count2: usize,
    changes1: &'a mut Bits,
    changes2: &'a mut Bits,
    forward: Vec<i64>,
    backward: Vec<i64>,
}

impl<'a> Myers<'a> {
    pub(crate) fn new(
        first: &'a [u32],
        second: &'a [u32],
        start1: usize,
        count1: usize,
        start2: usize,
        count2: usize,
        changes1: &'a mut Bits,
        changes2: &'a mut Bits,
    ) -> Self {
        changes1.set_range(start1, start1 + count1, true);
        changes2.set_range(start2, start2 + count2, true);
        let total = count1 + count2;
        Self {
            first,
            second,
            start1,
            count1,
            start2,
            count2,
            changes1,
            changes2,
            forward: vec![0; total + 2],
            backward: vec![0; total + 2],
        }
    }

    pub(crate) fn execute_with_threshold(&mut self) -> Result<(), ComparisonError> {
        let total = (self.count1 + self.count2) as f64;
        let threshold =
            (MYERS_BASE_THRESHOLD + 10 * (total.sqrt() as i64)).max(MYERS_BASE_THRESHOLD);
        self.execute(threshold, true)
    }

    pub(crate) fn execute(&mut self, threshold: i64, throw: bool) -> Result<(), ComparisonError> {
        if self.count1 == 0 || self.count2 == 0 {
            return Ok(());
        }
        let count1 = self.count1 as i64;
        let count2 = self.count2 as i64;
        self.run(0, count1, 0, count2, threshold.min(count1 + count2), throw)
    }

    fn run(
        &mut self,
        old_start: i64,
        old_end: i64,
        new_start: i64,
        new_end: i64,
        diff_estimate: i64,
        throw: bool,
    ) -> Result<(), ComparisonError> {
        debug_assert!(old_start <= old_end && new_start <= new_end);
        if old_start < old_end && new_start < new_end {
            let old_length = old_end - old_start;
            let new_length = new_end - new_start;
            self.forward[(new_length + 1) as usize] = 0;
            self.backward[(new_length + 1) as usize] = 0;
            let half_diff = (diff_estimate + 1) / 2;
            let mut found_x = -1i64;
            let mut found_k = -1i64;
            let mut total_diff = -1i64;
            let mut found = false;
            let mut diff = 0i64;
            while diff <= half_diff {
                let left = new_length + (-diff).max(-new_length + ((diff ^ new_length) & 1));
                let right = new_length + diff.min(old_length - ((diff ^ old_length) & 1));
                let mut k = left;
                while k <= right {
                    let x_start = if k == left
                        || (k != right
                            && self.forward[(k - 1) as usize] < self.forward[(k + 1) as usize])
                    {
                        self.forward[(k + 1) as usize]
                    } else {
                        self.forward[(k - 1) as usize] + 1
                    };
                    let y = x_start - k + new_length;
                    let x = x_start
                        + self.common_subsequence_forward(
                            old_start + x_start,
                            new_start + y,
                            (old_end - old_start - x_start).min(new_end - new_start - y),
                        );
                    self.forward[k as usize] = x;
                    k += 2;
                }
                if (old_length - new_length) % 2 != 0 {
                    let mut k = left;
                    while k <= right {
                        if old_length - (diff - 1) <= k
                            && k <= old_length + (diff - 1)
                            && self.forward[k as usize]
                                + self.backward[(new_length + old_length - k) as usize]
                                >= old_length
                        {
                            found_x = self.forward[k as usize];
                            found_k = k;
                            total_diff = 2 * diff - 1;
                            found = true;
                            break;
                        }
                        k += 2;
                    }
                    if found {
                        break;
                    }
                }
                let mut k = left;
                while k <= right {
                    let x_start = if k == left
                        || (k != right
                            && self.backward[(k - 1) as usize] < self.backward[(k + 1) as usize])
                    {
                        self.backward[(k + 1) as usize]
                    } else {
                        self.backward[(k - 1) as usize] + 1
                    };
                    let y = x_start - k + new_length;
                    let x = x_start
                        + self.common_subsequence_backward(
                            old_end - 1 - x_start,
                            new_end - 1 - y,
                            (old_end - old_start - x_start).min(new_end - new_start - y),
                        );
                    self.backward[k as usize] = x;
                    k += 2;
                }
                if (old_length - new_length) % 2 == 0 {
                    let mut k = left;
                    while k <= right {
                        if old_length - diff <= k
                            && k <= old_length + diff
                            && self.forward[(old_length + new_length - k) as usize]
                                + self.backward[k as usize]
                                >= old_length
                        {
                            found_x = old_length - self.backward[k as usize];
                            found_k = old_length + new_length - k;
                            total_diff = 2 * diff;
                            found = true;
                            break;
                        }
                        k += 2;
                    }
                    if found {
                        break;
                    }
                }
                diff += 1;
            }
            if total_diff > 1 {
                let found_y = found_x - found_k + new_length;
                let old_diff = (total_diff + 1) / 2;
                if 0 < found_x && 0 < found_y {
                    self.run(
                        old_start,
                        old_start + found_x,
                        new_start,
                        new_start + found_y,
                        old_diff,
                        throw,
                    )?;
                }
                if old_start + found_x < old_end && new_start + found_y < new_end {
                    self.run(
                        old_start + found_x,
                        old_end,
                        new_start + found_y,
                        new_end,
                        total_diff - old_diff,
                        throw,
                    )?;
                }
            } else if total_diff >= 0 {
                let mut x = old_start;
                let mut y = new_start;
                while x < old_end && y < new_end {
                    let common =
                        self.common_subsequence_forward(x, y, (old_end - x).min(new_end - y));
                    if common > 0 {
                        self.changes1.set_range(
                            self.start1 + x as usize,
                            self.start1 + (x + common) as usize,
                            false,
                        );
                        self.changes2.set_range(
                            self.start2 + y as usize,
                            self.start2 + (y + common) as usize,
                            false,
                        );
                        x += common;
                        y += common;
                    } else if old_end - old_start > new_end - new_start {
                        x += 1;
                    } else {
                        y += 1;
                    }
                }
            } else if throw {
                return Err(ComparisonError::DiffTooBig);
            }
        }
        Ok(())
    }

    fn common_subsequence_forward(&self, old_index: i64, new_index: i64, max_length: i64) -> i64 {
        let max_length =
            max_length.min((self.count1 as i64 - old_index).min(self.count2 as i64 - new_index));
        let mut x = old_index;
        let mut y = new_index;
        while x - old_index < max_length
            && self.first[self.start1 + x as usize] == self.second[self.start2 + y as usize]
        {
            x += 1;
            y += 1;
        }
        x - old_index
    }

    fn common_subsequence_backward(&self, old_index: i64, new_index: i64, max_length: i64) -> i64 {
        let max_length = max_length.min(old_index.min(new_index) + 1);
        let mut x = old_index;
        let mut y = new_index;
        while old_index - x < max_length
            && self.first[self.start1 + x as usize] == self.second[self.start2 + y as usize]
        {
            x -= 1;
            y -= 1;
        }
        old_index - x
    }
}

pub(crate) struct UniqueMatching {
    pub(crate) first_indices: Vec<usize>,
    pub(crate) second_indices: Vec<usize>,
}

pub(crate) fn unique_lcs(
    first: &[u32],
    second: &[u32],
    start1: usize,
    count1: usize,
    start2: usize,
    count2: usize,
) -> Option<UniqueMatching> {
    let mut occurrences: HashMap<u32, i64> = HashMap::new();
    let mut matches = vec![0usize; count1];
    for index in 0..count1 {
        let value = first[start1 + index];
        let current = occurrences.get(&value).copied().unwrap_or(0);
        if current == -1 {
            continue;
        }
        if current == 0 {
            occurrences.insert(value, index as i64 + 1);
        } else {
            occurrences.insert(value, -1);
        }
    }
    let mut count = 0usize;
    for index in 0..count2 {
        let value = second[start2 + index];
        let current = occurrences.get(&value).copied().unwrap_or(0);
        if current == 0 || current == -1 {
            continue;
        }
        let first_position = (current - 1) as usize;
        if matches[first_position] == 0 {
            matches[first_position] = index + 1;
            count += 1;
        } else {
            matches[first_position] = 0;
            occurrences.insert(value, -1);
            count -= 1;
        }
    }
    if count == 0 {
        return None;
    }
    let mut sequence = vec![0usize; count];
    let mut last_element = vec![0usize; count];
    let mut predecessor = vec![0isize; count1];
    let mut length = 0usize;
    for index in 0..count1 {
        if matches[index] == 0 {
            continue;
        }
        let mut low = 0isize;
        let mut high = length as isize - 1;
        while low <= high {
            let middle = (low + high) / 2;
            if sequence[middle as usize] < matches[index] {
                low = middle + 1;
            } else {
                high = middle - 1;
            }
        }
        let position = low as usize;
        if position == length || matches[index] < sequence[position] {
            sequence[position] = matches[index];
            last_element[position] = index;
            predecessor[index] = if position > 0 {
                last_element[position - 1] as isize
            } else {
                -1
            };
            if position == length {
                length += 1;
            }
        }
    }
    let mut first_indices = vec![0usize; length];
    let mut second_indices = vec![0usize; length];
    let mut slot = length as isize - 1;
    let mut current = last_element[length - 1] as isize;
    while current != -1 {
        first_indices[slot as usize] = current as usize;
        second_indices[slot as usize] = matches[current as usize] - 1;
        slot -= 1;
        current = predecessor[current as usize];
    }
    Some(UniqueMatching {
        first_indices,
        second_indices,
    })
}

pub(crate) struct Patience<'a> {
    first: &'a [u32],
    second: &'a [u32],
    count1: usize,
    count2: usize,
    pub(crate) changes1: Bits,
    pub(crate) changes2: Bits,
}

impl<'a> Patience<'a> {
    pub(crate) fn new(first: &'a [u32], second: &'a [u32]) -> Self {
        Self {
            first,
            second,
            count1: first.len(),
            count2: second.len(),
            changes1: Bits::default(),
            changes2: Bits::default(),
        }
    }

    pub(crate) fn execute(&mut self, fail_on_small_reduction: bool) -> Result<(), ComparisonError> {
        let counter = if fail_on_small_reduction { 2 } else { -1 };
        self.run(0, self.count1, 0, self.count2, counter)
    }

    fn check_reduction(&self, count1: usize, count2: usize) -> Result<(), ComparisonError> {
        if count1 * 2 < self.count1 {
            return Ok(());
        }
        if count2 * 2 < self.count2 {
            return Ok(());
        }
        Err(ComparisonError::DiffTooBig)
    }

    fn add_change(&mut self, start1: usize, count1: usize, start2: usize, count2: usize) {
        self.changes1.set_range(start1, start1 + count1, true);
        self.changes2.set_range(start2, start2 + count2, true);
    }

    fn run(
        &mut self,
        start1: usize,
        count1: usize,
        start2: usize,
        count2: usize,
        counter: i64,
    ) -> Result<(), ComparisonError> {
        if count1 == 0 && count2 == 0 {
            return Ok(());
        }
        if count1 == 0 || count2 == 0 {
            self.add_change(start1, count1, start2, count2);
            return Ok(());
        }
        let mut start_offset = 0;
        for index in 0..count1.min(count2) {
            if self.first[start1 + index] != self.second[start2 + index] {
                break;
            }
            start_offset += 1;
        }
        let start1 = start1 + start_offset;
        let start2 = start2 + start_offset;
        let count1 = count1 - start_offset;
        let count2 = count2 - start_offset;
        let mut end_offset = 0;
        for index in 1..=count1.min(count2) {
            if self.first[start1 + count1 - index] != self.second[start2 + count2 - index] {
                break;
            }
            end_offset += 1;
        }
        let count1 = count1 - end_offset;
        let count2 = count2 - end_offset;
        if count1 == 0 || count2 == 0 {
            self.add_change(start1, count1, start2, count2);
            return Ok(());
        }
        if counter == 0 {
            self.check_reduction(count1, count2)?;
        }
        let counter = (counter - 1).max(-1);
        let matching = unique_lcs(self.first, self.second, start1, count1, start2, count2);
        let Some(matching) = matching else {
            if counter >= 0 {
                self.check_reduction(count1, count2)?;
            }
            let first = self.first;
            let second = self.second;
            let threshold = MYERS_BASE_THRESHOLD + 10 * ((count1 + count2) as f64).sqrt() as i64;
            let mut myers = Myers::new(
                first,
                second,
                start1,
                count1,
                start2,
                count2,
                &mut self.changes1,
                &mut self.changes2,
            );
            return myers.execute(threshold, false);
        };
        let matched = matching.first_indices.len();
        let first_count1 = matching.first_indices[0];
        let first_count2 = matching.second_indices[0];
        self.run(start1, first_count1, start2, first_count2, counter)?;
        for index in 1..matched {
            let next_start1 = matching.first_indices[index - 1] + 1;
            let next_start2 = matching.second_indices[index - 1] + 1;
            let gap1 = matching.first_indices[index] - next_start1;
            let gap2 = matching.second_indices[index] - next_start2;
            if gap1 > 0 || gap2 > 0 {
                self.run(
                    start1 + next_start1,
                    gap1,
                    start2 + next_start2,
                    gap2,
                    counter,
                )?;
            }
        }
        let (tail_start1, tail_count1) = if matching.first_indices[matched - 1] == count1 - 1 {
            (count1 - 1, 0)
        } else {
            let tail_start = matching.first_indices[matched - 1] + 1;
            (tail_start, count1 - tail_start)
        };
        let (tail_start2, tail_count2) = if matching.second_indices[matched - 1] == count2 - 1 {
            (count2 - 1, 0)
        } else {
            let tail_start = matching.second_indices[matched - 1] + 1;
            (tail_start, count2 - tail_start)
        };
        self.run(
            start1 + tail_start1,
            tail_count1,
            start2 + tail_start2,
            tail_count2,
            counter,
        )
    }
}
