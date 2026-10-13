use std::ops::Range;

use editor::Anchor;
use merge_diff::ThreeSide;

pub(crate) const DEFAULT_CONTEXT_RANGE: usize = 4;

const PANE_COUNT: usize = 3;
const NESTED_BLOCK_COUNT: usize = 3;
const MINIMUM_FOLDED_LINES: i64 = 2;

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct FragmentLines {
    pub lines: [Range<usize>; 3],
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct FoldBlock {
    pub regions: [Option<Range<usize>>; 3],
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct FoldGroup {
    pub blocks: Vec<FoldBlock>,
}

fn to_signed(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn nested_block_shift(context_range: usize, nesting_number: usize) -> Option<i64> {
    let multiplier = match nesting_number {
        0 => 1,
        1 => 2,
        2 => 4,
        _ => return None,
    };
    Some(to_signed(context_range).saturating_mul(multiplier))
}

fn clamp_to_pane(value: i64, line_count: usize) -> i64 {
    value.min(to_signed(line_count)).max(0)
}

fn create_block(starts: [i64; 3], ends: [i64; 3]) -> Option<FoldBlock> {
    let mut regions: [Option<Range<usize>>; 3] = [None, None, None];
    for pane in 0..PANE_COUNT {
        if ends[pane] - starts[pane] < MINIMUM_FOLDED_LINES {
            continue;
        }
        let start = usize::try_from(starts[pane]).unwrap_or(0);
        let end = usize::try_from(ends[pane]).unwrap_or(0);
        regions[pane] = Some(start..end);
    }
    if regions.iter().any(Option::is_some) {
        Some(FoldBlock { regions })
    } else {
        None
    }
}

fn build_group(
    starts: [i64; 3],
    ends: [i64; 3],
    line_counts: [usize; 3],
    context_range: usize,
) -> Option<FoldGroup> {
    let mut blocks = Vec::with_capacity(NESTED_BLOCK_COUNT);
    for nesting_number in 0..NESTED_BLOCK_COUNT {
        let Some(shift) = nested_block_shift(context_range, nesting_number) else {
            break;
        };
        let mut range_starts = [0; 3];
        let mut range_ends = [0; 3];
        for pane in 0..PANE_COUNT {
            range_starts[pane] =
                clamp_to_pane(starts[pane].saturating_add(shift), line_counts[pane]);
            range_ends[pane] = clamp_to_pane(ends[pane].saturating_sub(shift), line_counts[pane]);
        }
        if let Some(block) = create_block(range_starts, range_ends) {
            blocks.push(block);
        }
    }
    if blocks.is_empty() {
        None
    } else {
        Some(FoldGroup { blocks })
    }
}

pub(crate) fn compute_fold_groups(
    fragments: &[FragmentLines],
    line_counts: [usize; 3],
    context_range: usize,
) -> Vec<FoldGroup> {
    let mut groups = Vec::new();
    let mut last = [i64::MIN; 3];

    for fragment in fragments {
        let mut starts = [0; 3];
        let mut ends = [0; 3];
        for pane in 0..PANE_COUNT {
            starts[pane] = last[pane];
            ends[pane] = to_signed(fragment.lines[pane].start);
            last[pane] = to_signed(fragment.lines[pane].end);
        }
        groups.extend(build_group(starts, ends, line_counts, context_range));
    }

    groups.extend(build_group(last, [i64::MAX; 3], line_counts, context_range));
    groups
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HoveredFold {
    pub group: usize,
    pub side: ThreeSide,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct FoldedGroupState {
    expanded: Option<Range<usize>>,
    collapsed: Option<Range<usize>>,
}

impl FoldedGroupState {
    fn line_range(&self) -> Option<&Range<usize>> {
        self.expanded.as_ref().or(self.collapsed.as_ref())
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct FoldingCache {
    expand_by_default: bool,
    ranges: [Vec<FoldedGroupState>; PANE_COUNT],
}

fn contains_lines(outer: &Range<usize>, inner: &Range<usize>) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

impl FoldingCache {
    fn capture(
        expanded_blocks: &[usize],
        region_lines: &[Vec<[Option<Range<usize>>; 3]>],
        expand_by_default: bool,
    ) -> Self {
        let ranges: [Vec<FoldedGroupState>; PANE_COUNT] = std::array::from_fn(|pane| {
            region_lines
                .iter()
                .zip(expanded_blocks)
                .filter_map(|(blocks, &expanded_count)| {
                    let mut expanded = None;
                    let mut collapsed = None;
                    for (block_index, block) in blocks.iter().enumerate() {
                        let Some(lines) = block.get(pane).cloned().flatten() else {
                            continue;
                        };
                        if block_index >= expanded_count {
                            collapsed = Some(lines);
                            break;
                        }
                        if expanded.is_none() {
                            expanded = Some(lines);
                        }
                    }
                    (expanded.is_some() || collapsed.is_some()).then_some(FoldedGroupState {
                        expanded,
                        collapsed,
                    })
                })
                .collect()
        });
        Self {
            expand_by_default,
            ranges,
        }
    }
}

struct ExpandSuggester<'a> {
    cache: Option<&'a FoldingCache>,
    cursors: [usize; PANE_COUNT],
    expanded_by_default: bool,
}

impl<'a> ExpandSuggester<'a> {
    fn new(cache: Option<&'a FoldingCache>, expanded_by_default: bool) -> Self {
        Self {
            cache,
            cursors: [0; PANE_COUNT],
            expanded_by_default,
        }
    }

    fn expanded_block_count(&mut self, group: &FoldGroup) -> usize {
        let expanded: Vec<bool> = group
            .blocks
            .iter()
            .map(|block| self.is_expanded(block))
            .collect();
        expanded
            .iter()
            .position(|block_expanded| !block_expanded)
            .unwrap_or(expanded.len())
    }

    fn is_expanded(&mut self, block: &FoldBlock) -> bool {
        let Some(cache) = self.cache else {
            return self.expanded_by_default;
        };
        if cache.expand_by_default != self.expanded_by_default {
            return self.expanded_by_default;
        }
        let mut state = None;
        for (pane, region) in block.regions.iter().enumerate() {
            let Some(region) = region else {
                continue;
            };
            let Some(pane_state) = self.cached_expanded(cache, region, pane) else {
                continue;
            };
            match state {
                None => state = Some(pane_state),
                Some(previous) if previous != pane_state => return self.expanded_by_default,
                Some(_) => {}
            }
        }
        state.unwrap_or(self.expanded_by_default)
    }

    fn cached_expanded(
        &mut self,
        cache: &FoldingCache,
        region: &Range<usize>,
        pane: usize,
    ) -> Option<bool> {
        let group = self.cached_state(cache, region, pane)?;
        if group
            .collapsed
            .as_ref()
            .is_some_and(|collapsed| contains_lines(collapsed, region))
        {
            return Some(false);
        }
        if group
            .expanded
            .as_ref()
            .is_some_and(|expanded| contains_lines(expanded, region))
        {
            return Some(true);
        }
        None
    }

    fn cached_state<'c>(
        &mut self,
        cache: &'c FoldingCache,
        region: &Range<usize>,
        pane: usize,
    ) -> Option<&'c FoldedGroupState> {
        if region.start == region.end {
            return None;
        }
        let states = cache.ranges.get(pane)?;
        let cursor = self.cursors.get_mut(pane)?;
        while let Some(state) = states.get(*cursor) {
            let line_range = state.line_range()?;
            if line_range.end > region.start {
                if contains_lines(line_range, region) {
                    return Some(state);
                }
                if line_range.start >= region.start {
                    return None;
                }
            }
            *cursor += 1;
        }
        None
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FoldState {
    groups: Vec<FoldGroup>,
    expanded_blocks: Vec<usize>,
    anchored_regions: Vec<Vec<[Option<Range<Anchor>>; 3]>>,
    hovered: Option<HoveredFold>,
    cache: Option<FoldingCache>,
}

impl FoldState {
    pub(crate) fn new(
        groups: Vec<FoldGroup>,
        expanded_by_default: bool,
        cache: Option<FoldingCache>,
    ) -> Self {
        let mut suggester = ExpandSuggester::new(cache.as_ref(), expanded_by_default);
        let expanded_blocks = groups
            .iter()
            .map(|group| suggester.expanded_block_count(group))
            .collect();
        Self {
            groups,
            expanded_blocks,
            anchored_regions: Vec::new(),
            hovered: None,
            cache,
        }
    }

    pub(crate) fn into_cache(
        self,
        region_lines: &[Vec<[Option<Range<usize>>; 3]>],
        expand_by_default: bool,
    ) -> Option<FoldingCache> {
        if self.groups.is_empty() {
            return self.cache;
        }
        Some(FoldingCache::capture(
            &self.expanded_blocks,
            region_lines,
            expand_by_default,
        ))
    }

    pub(crate) fn anchored_regions(&self) -> &[Vec<[Option<Range<Anchor>>; 3]>] {
        &self.anchored_regions
    }

    pub(crate) fn with_anchored_regions(
        mut self,
        anchored_regions: Vec<Vec<[Option<Range<Anchor>>; 3]>>,
    ) -> Self {
        self.anchored_regions = anchored_regions;
        self
    }

    pub(crate) fn groups(&self) -> &[FoldGroup] {
        &self.groups
    }

    pub(crate) fn collapsed_block(&self, group_index: usize) -> Option<usize> {
        let block_count = self.groups.get(group_index)?.blocks.len();
        let expanded = *self.expanded_blocks.get(group_index)?;
        (expanded < block_count).then_some(expanded)
    }

    pub(crate) fn anchored_region(
        &self,
        group_index: usize,
        block_index: usize,
        pane: usize,
    ) -> Option<&Range<Anchor>> {
        self.anchored_regions
            .get(group_index)?
            .get(block_index)?
            .get(pane)?
            .as_ref()
    }

    pub(crate) fn group_anchored_regions(
        &self,
        group_index: usize,
        pane: usize,
    ) -> Vec<Range<Anchor>> {
        self.anchored_regions
            .get(group_index)
            .into_iter()
            .flatten()
            .filter_map(|block| block[pane].clone())
            .collect()
    }

    pub(crate) fn expand_block(&mut self, group_index: usize, block_index: usize) {
        let Some(block_count) = self.groups.get(group_index).map(|group| group.blocks.len()) else {
            return;
        };
        if let Some(expanded) = self.expanded_blocks.get_mut(group_index) {
            *expanded = (*expanded).max(block_index + 1).min(block_count);
        }
    }

    pub(crate) fn expand_all(&mut self, expanded: bool) {
        for (group, expanded_blocks) in self.groups.iter().zip(self.expanded_blocks.iter_mut()) {
            *expanded_blocks = if expanded { group.blocks.len() } else { 0 };
        }
        self.hovered = None;
    }

    pub(crate) fn hovered(&self) -> Option<HoveredFold> {
        self.hovered
    }

    pub(crate) fn hover(&mut self, hovered: HoveredFold) -> bool {
        let changed = self.hovered != Some(hovered);
        self.hovered = Some(hovered);
        changed
    }

    pub(crate) fn unhover(&mut self, side: ThreeSide) -> bool {
        if self.hovered.is_some_and(|hovered| hovered.side == side) {
            self.hovered = None;
            return true;
        }
        false
    }

    pub(crate) fn clear_hover(&mut self) {
        self.hovered = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_panes(range: Range<usize>) -> [Option<Range<usize>>; 3] {
        [Some(range.clone()), Some(range.clone()), Some(range)]
    }

    fn middle_fragment_groups() -> Vec<FoldGroup> {
        let fragments = [FragmentLines {
            lines: [10..12, 10..12, 10..12],
        }];
        compute_fold_groups(&fragments, [30, 30, 30], DEFAULT_CONTEXT_RANGE)
    }

    fn block_regions(group: &FoldGroup) -> Vec<[Option<Range<usize>>; 3]> {
        group
            .blocks
            .iter()
            .map(|block| block.regions.clone())
            .collect()
    }

    #[test]
    fn merge_tool_folding_without_fragments_makes_one_group_of_three_full_blocks() {
        let groups = compute_fold_groups(&[], [20, 20, 20], DEFAULT_CONTEXT_RANGE);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].blocks.len(), 3);
        for block in &groups[0].blocks {
            assert_eq!(block.regions, all_panes(0..20));
        }
    }

    #[test]
    fn merge_tool_folding_middle_fragment_makes_two_groups_with_clamped_context() {
        let groups = middle_fragment_groups();
        assert_eq!(groups.len(), 2);
        assert_eq!(
            block_regions(&groups[0]),
            vec![all_panes(0..6), all_panes(0..2)]
        );
        assert_eq!(
            block_regions(&groups[1]),
            vec![all_panes(16..30), all_panes(20..30), all_panes(28..30)]
        );
    }

    #[test]
    fn merge_tool_folding_regions_shorter_than_two_lines_become_none() {
        let fragments = [FragmentLines {
            lines: [3..5, 2..4, 3..5],
        }];
        let groups = compute_fold_groups(&fragments, [30, 30, 30], 1);
        assert_eq!(
            groups[0].blocks,
            vec![FoldBlock {
                regions: [Some(0..2), None, Some(0..2)]
            }]
        );
    }

    #[test]
    fn merge_tool_folding_group_without_any_region_is_dropped() {
        let fragments = [FragmentLines {
            lines: [5..6, 5..6, 5..6],
        }];
        let groups = compute_fold_groups(&fragments, [6, 6, 6], DEFAULT_CONTEXT_RANGE);
        assert!(groups.is_empty());
    }

    #[test]
    fn merge_tool_folding_each_pane_continues_from_its_own_previous_fragment_end() {
        let fragments = [
            FragmentLines {
                lines: [5..7, 5..8, 5..7],
            },
            FragmentLines {
                lines: [20..21, 24..26, 20..21],
            },
        ];
        let groups = compute_fold_groups(&fragments, [30, 30, 30], DEFAULT_CONTEXT_RANGE);
        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups[0].blocks,
            vec![FoldBlock {
                regions: [Some(11..16), Some(12..20), Some(11..16)]
            }]
        );
        assert_eq!(
            groups[1].blocks,
            vec![FoldBlock {
                regions: [Some(25..30), None, Some(25..30)]
            }]
        );
    }

    #[test]
    fn merge_tool_folding_state_expands_nested_blocks_one_at_a_time() {
        let mut state = FoldState::new(middle_fragment_groups(), false, None);
        assert_eq!(state.collapsed_block(0), Some(0));
        assert_eq!(state.collapsed_block(1), Some(0));
        state.expand_block(1, 0);
        assert_eq!(state.collapsed_block(1), Some(1));
        state.expand_block(1, 1);
        assert_eq!(state.collapsed_block(1), Some(2));
        state.expand_block(1, 2);
        assert_eq!(state.collapsed_block(1), None);
        assert_eq!(state.collapsed_block(0), Some(0));
    }

    #[test]
    fn merge_tool_folding_state_expanding_an_inner_block_expands_the_outer_ones() {
        let mut state = FoldState::new(middle_fragment_groups(), false, None);
        state.expand_block(1, 1);
        assert_eq!(state.collapsed_block(1), Some(2));
    }

    #[test]
    fn merge_tool_folding_state_expanding_an_outer_block_again_keeps_inner_blocks_expanded() {
        let mut state = FoldState::new(middle_fragment_groups(), false, None);
        state.expand_block(1, 1);
        state.expand_block(1, 0);
        assert_eq!(state.collapsed_block(1), Some(2));
    }

    #[test]
    fn merge_tool_folding_state_expand_all_toggles_every_group() {
        let mut state = FoldState::new(middle_fragment_groups(), true, None);
        assert_eq!(state.collapsed_block(0), None);
        assert_eq!(state.collapsed_block(1), None);
        state.expand_all(false);
        assert_eq!(state.collapsed_block(0), Some(0));
        assert_eq!(state.collapsed_block(1), Some(0));
        state.expand_all(true);
        assert_eq!(state.collapsed_block(0), None);
        assert_eq!(state.collapsed_block(1), None);
    }

    #[test]
    fn merge_tool_folding_state_ignores_unknown_groups() {
        let mut state = FoldState::new(middle_fragment_groups(), false, None);
        assert_eq!(state.groups().len(), 2);
        assert_eq!(state.collapsed_block(7), None);
        state.expand_block(7, 0);
        assert_eq!(state.collapsed_block(7), None);
        assert_eq!(state.collapsed_block(0), Some(0));
        assert_eq!(state.collapsed_block(1), Some(0));
    }

    #[test]
    fn merge_tool_folding_hover_is_cleared_only_by_the_pane_that_set_it() {
        let mut state = FoldState::new(middle_fragment_groups(), false, None);
        let hovered = HoveredFold {
            group: 1,
            side: ThreeSide::Left,
        };
        assert!(state.hover(hovered));
        assert!(!state.hover(hovered));
        assert!(!state.unhover(ThreeSide::Right));
        assert_eq!(state.hovered(), Some(hovered));
        assert!(state.unhover(ThreeSide::Left));
        assert_eq!(state.hovered(), None);
    }

    #[test]
    fn merge_tool_folding_expand_all_drops_the_hover() {
        let mut state = FoldState::new(middle_fragment_groups(), false, None);
        state.hover(HoveredFold {
            group: 0,
            side: ThreeSide::Base,
        });
        state.expand_all(true);
        assert_eq!(state.hovered(), None);
    }

    fn unedited_region_lines(groups: &[FoldGroup]) -> Vec<Vec<[Option<Range<usize>>; 3]>> {
        groups.iter().map(block_regions).collect()
    }

    fn cache_with_second_group_partly_expanded() -> Option<FoldingCache> {
        let groups = middle_fragment_groups();
        let region_lines = unedited_region_lines(&groups);
        let mut state = FoldState::new(groups, false, None);
        state.expand_block(1, 0);
        state.into_cache(&region_lines, false)
    }

    #[test]
    fn merge_tool_folding_rediff_restores_the_expansion_state_from_the_cache() {
        let restored = FoldState::new(
            middle_fragment_groups(),
            false,
            cache_with_second_group_partly_expanded(),
        );
        assert_eq!(restored.collapsed_block(0), Some(0));
        assert_eq!(restored.collapsed_block(1), Some(1));
    }

    #[test]
    fn merge_tool_folding_cache_is_ignored_after_the_default_changed() {
        let restored = FoldState::new(
            middle_fragment_groups(),
            true,
            cache_with_second_group_partly_expanded(),
        );
        assert_eq!(restored.collapsed_block(0), None);
        assert_eq!(restored.collapsed_block(1), None);
    }

    #[test]
    fn merge_tool_folding_state_without_groups_keeps_the_previous_cache() {
        let cache = cache_with_second_group_partly_expanded();
        let empty = FoldState::new(Vec::new(), false, cache.clone());
        assert_eq!(empty.into_cache(&[], false), cache);
    }
}
