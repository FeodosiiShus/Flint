use std::ops::Range;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ThreeSide {
    Left,
    Base,
    Right,
}

impl ThreeSide {
    pub const ALL: [ThreeSide; 3] = [ThreeSide::Left, ThreeSide::Base, ThreeSide::Right];

    pub fn index(self) -> usize {
        match self {
            ThreeSide::Left => 0,
            ThreeSide::Base => 1,
            ThreeSide::Right => 2,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MergeRange {
    pub left: Range<usize>,
    pub base: Range<usize>,
    pub right: Range<usize>,
}

impl MergeRange {
    pub fn new(left: Range<usize>, base: Range<usize>, right: Range<usize>) -> Self {
        Self { left, base, right }
    }

    pub fn side(&self, side: ThreeSide) -> Range<usize> {
        match side {
            ThreeSide::Left => self.left.clone(),
            ThreeSide::Base => self.base.clone(),
            ThreeSide::Right => self.right.clone(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.left.is_empty() && self.base.is_empty() && self.right.is_empty()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct DiffRange {
    pub start1: usize,
    pub end1: usize,
    pub start2: usize,
    pub end2: usize,
}

impl DiffRange {
    pub(crate) fn new(start1: usize, end1: usize, start2: usize, end2: usize) -> Self {
        Self {
            start1,
            end1,
            start2,
            end2,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.start1 == self.end1 && self.start2 == self.end2
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DiffFragment {
    pub start_offset1: usize,
    pub end_offset1: usize,
    pub start_offset2: usize,
    pub end_offset2: usize,
}

impl From<DiffRange> for DiffFragment {
    fn from(range: DiffRange) -> Self {
        Self {
            start_offset1: range.start1,
            end_offset1: range.end1,
            start_offset2: range.start2,
            end_offset2: range.end2,
        }
    }
}
