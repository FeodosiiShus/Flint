use crate::by_word::{compare_words, compare_words_three_way};
use crate::merge_type::{compare_word_merge_contents, get_word_merge_type};
use crate::range::{DiffRange, MergeRange, ThreeSide};
use crate::text::{ComparisonPolicy, is_equal_texts};
use crate::{ComparisonError, NeverCanceled};

struct SimpleHelper<'a> {
    texts: [&'a str; 3],
    new_content: Vec<u8>,
    last: [usize; 3],
}

impl<'a> SimpleHelper<'a> {
    fn new(left: &'a str, base: &'a str, right: &'a str) -> Self {
        Self {
            texts: [left, base, right],
            new_content: Vec::new(),
            last: [0, 0, 0],
        }
    }

    fn text_bytes(&self) -> [&'a [u8]; 3] {
        [
            self.texts[0].as_bytes(),
            self.texts[1].as_bytes(),
            self.texts[2].as_bytes(),
        ]
    }

    fn execute(&mut self, policy: ComparisonPolicy) -> Result<Option<String>, ComparisonError> {
        let changes = compare_words_three_way(
            self.texts[0],
            self.texts[1],
            self.texts[2],
            policy,
            &NeverCanceled,
        )?;
        for fragment in &changes {
            let base_range = self.next_merge_range([
                fragment.left.start,
                fragment.base.start,
                fragment.right.start,
            ]);
            if self.append_base(&base_range).is_none() {
                return Ok(None);
            }
            let conflict_range =
                self.next_merge_range([fragment.left.end, fragment.base.end, fragment.right.end]);
            match self.append_conflict(&conflict_range, policy) {
                Some(true) => {}
                Some(false) | None => return Ok(None),
            }
        }
        let trailing_range = self.next_merge_range([
            self.texts[0].len(),
            self.texts[1].len(),
            self.texts[2].len(),
        ]);
        if self.append_base(&trailing_range).is_none() {
            return Ok(None);
        }
        Ok(String::from_utf8(std::mem::take(&mut self.new_content)).ok())
    }

    fn next_merge_range(&mut self, ends: [usize; 3]) -> MergeRange {
        let range = MergeRange::new(
            self.last[0]..ends[0],
            self.last[1]..ends[1],
            self.last[2]..ends[2],
        );
        self.last = ends;
        range
    }

    fn append_base(&mut self, range: &MergeRange) -> Option<()> {
        if range.is_empty() {
            return Some(());
        }
        let policy = ComparisonPolicy::Default;
        if self.is_unchanged_range(range, policy) {
            self.append(range, ThreeSide::Base);
        } else {
            let conflict_type = get_word_merge_type(range, self.text_bytes(), policy)?;
            if conflict_type.is_change(ThreeSide::Left) {
                self.append(range, ThreeSide::Left);
            } else if conflict_type.is_change(ThreeSide::Right) {
                self.append(range, ThreeSide::Right);
            } else {
                self.append(range, ThreeSide::Base);
            }
        }
        Some(())
    }

    fn append_conflict(&mut self, range: &MergeRange, policy: ComparisonPolicy) -> Option<bool> {
        let conflict_type = get_word_merge_type(range, self.text_bytes(), policy)?;
        if conflict_type.is_conflict() {
            return Some(false);
        }
        if conflict_type.is_change(ThreeSide::Left) {
            self.append(range, ThreeSide::Left);
        } else {
            self.append(range, ThreeSide::Right);
        }
        Some(true)
    }

    fn append(&mut self, range: &MergeRange, side: ThreeSide) {
        let text = self.texts[side.index()].as_bytes();
        self.new_content.extend_from_slice(&text[range.side(side)]);
    }

    fn is_unchanged_range(&self, range: &MergeRange, policy: ComparisonPolicy) -> bool {
        let texts = self.text_bytes();
        compare_word_merge_contents(range, texts, policy, ThreeSide::Base, ThreeSide::Left)
            && compare_word_merge_contents(range, texts, policy, ThreeSide::Base, ThreeSide::Right)
    }
}

struct GreedyHelper<'a> {
    left_text: &'a str,
    base_text: &'a str,
    right_text: &'a str,
    new_content: Vec<u8>,
    last_base_offset: usize,
    index1: usize,
    index2: usize,
}

impl<'a> GreedyHelper<'a> {
    fn new(left: &'a str, base: &'a str, right: &'a str) -> Self {
        Self {
            left_text: left,
            base_text: base,
            right_text: right,
            new_content: Vec::new(),
            last_base_offset: 0,
            index1: 0,
            index2: 0,
        }
    }

    fn execute(&mut self, policy: ComparisonPolicy) -> Result<Option<String>, ComparisonError> {
        let fragments1 = compare_words(self.base_text, self.left_text, policy, &NeverCanceled)?;
        let fragments2 = compare_words(self.base_text, self.right_text, policy, &NeverCanceled)?;
        loop {
            let change_start1 = fragments1.get(self.index1).map(|fragment| fragment.start1);
            let change_start2 = fragments2.get(self.index2).map(|fragment| fragment.start1);
            match (change_start1, change_start2) {
                (None, None) => {
                    self.append_base(self.base_text.len());
                    break;
                }
                (Some(start1), Some(start2)) => self.append_base(start1.min(start2)),
                (Some(start1), None) => self.append_base(start1),
                (None, Some(start2)) => self.append_base(start2),
            }
            let mut base_offset_end = self.last_base_offset;
            let mut end1 = self.index1;
            let mut end2 = self.index2;
            loop {
                if let Some(next1) = fragments1.get(end1)
                    && next1.start1 <= base_offset_end
                {
                    base_offset_end = base_offset_end.max(next1.end1);
                    end1 += 1;
                    continue;
                }
                if let Some(next2) = fragments2.get(end2)
                    && next2.start1 <= base_offset_end
                {
                    base_offset_end = base_offset_end.max(next2.end1);
                    end2 += 1;
                    continue;
                }
                break;
            }
            debug_assert!(self.index1 != end1 || self.index2 != end2);
            let inserted1 = inserted_content(&fragments1[self.index1..end1], self.left_text);
            let inserted2 = inserted_content(&fragments2[self.index2..end2], self.right_text);
            self.index1 = end1;
            self.index2 = end2;
            self.last_base_offset = base_offset_end;
            if inserted1.is_empty() && inserted2.is_empty() {
                continue;
            }
            if inserted2.is_empty() {
                self.new_content.extend_from_slice(&inserted1);
                continue;
            }
            if inserted1.is_empty() {
                self.new_content.extend_from_slice(&inserted2);
                continue;
            }
            if is_equal_texts(&inserted1, &inserted2, policy) {
                let shorter = if inserted1.len() <= inserted2.len() {
                    &inserted1
                } else {
                    &inserted2
                };
                self.new_content.extend_from_slice(shorter);
                continue;
            }
            return Ok(None);
        }
        Ok(String::from_utf8(std::mem::take(&mut self.new_content)).ok())
    }

    fn append_base(&mut self, end_offset: usize) {
        if self.last_base_offset == end_offset {
            return;
        }
        if end_offset > self.last_base_offset {
            self.new_content
                .extend_from_slice(&self.base_text.as_bytes()[self.last_base_offset..end_offset]);
        }
        self.last_base_offset = end_offset;
    }
}

fn inserted_content(fragments: &[DiffRange], text: &str) -> Vec<u8> {
    let mut content = Vec::new();
    for fragment in fragments {
        content.extend_from_slice(&text.as_bytes()[fragment.start2..fragment.end2]);
    }
    content
}

pub fn try_resolve(left: &str, base: &str, right: &str) -> Option<String> {
    let default_pass = SimpleHelper::new(left, base, right).execute(ComparisonPolicy::Default);
    match default_pass {
        Ok(Some(resolved)) => Some(resolved),
        Ok(None) => SimpleHelper::new(left, base, right)
            .execute(ComparisonPolicy::IgnoreWhitespaces)
            .ok()
            .flatten(),
        Err(_) => None,
    }
}

pub fn try_greedy_resolve(left: &str, base: &str, right: &str) -> Option<String> {
    let default_pass = GreedyHelper::new(left, base, right).execute(ComparisonPolicy::Default);
    match default_pass {
        Ok(Some(resolved)) => Some(resolved),
        Ok(None) => GreedyHelper::new(left, base, right)
            .execute(ComparisonPolicy::IgnoreWhitespaces)
            .ok()
            .flatten(),
        Err(_) => None,
    }
}

pub fn try_resolve_conflict(left: &str, base: &str, right: &str) -> Option<String> {
    try_resolve(left, base, right)
}
