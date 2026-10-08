use std::{any::TypeId, ops::Range, sync::Arc};

use editor::{
    Anchor, MultiBufferSnapshot, ToPoint as _,
    display_map::{Crease, FoldPlaceholder},
};
use gpui::{
    App, Context, InteractiveElement as _, IntoElement as _, MouseButton, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div,
};
use language::Point;
use merge_diff::{Side, ThreeSide};
use multi_buffer::MultiBufferRow;
use util::ResultExt as _;

use super::{
    MergeViewer,
    folding::{
        DEFAULT_CONTEXT_RANGE, FoldGroup, FoldState, FragmentLines, HoveredFold,
        compute_fold_groups,
    },
    viewer_settings,
};
use crate::merge_tool::{merge_gutter::FoldedLine, merge_ribbons::FoldConnector};

const COLLAPSED_PLACEHOLDER: &str = "     ";

struct MergeFold;

fn region_points(snapshot: &MultiBufferSnapshot, region: &Range<usize>) -> Option<Range<Point>> {
    let max_row = snapshot.max_point().row;
    let first = u32::try_from(region.start).ok()?;
    if first > max_row {
        return None;
    }
    let last = u32::try_from(region.end.saturating_sub(1))
        .ok()?
        .min(max_row)
        .max(first);
    let last_length = snapshot.line_len(MultiBufferRow(last));
    Some(Point::new(first, 0)..Point::new(last, last_length))
}

fn anchored_region(snapshot: &MultiBufferSnapshot, region: &Range<usize>) -> Option<Range<Anchor>> {
    let points = region_points(snapshot, region)?;
    Some(snapshot.anchor_after(points.start)..snapshot.anchor_before(points.end))
}

fn fold_placeholder(
    viewer: WeakEntity<MergeViewer>,
    group_index: usize,
    block_index: usize,
) -> FoldPlaceholder {
    FoldPlaceholder {
        render: Arc::new(move |fold_id, _range, _cx| {
            let viewer = viewer.clone();
            div()
                .id(fold_id)
                .size_full()
                .cursor_pointer()
                .child(SharedString::from(COLLAPSED_PLACEHOLDER))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, window, cx| {
                    viewer
                        .update(cx, |viewer, cx| {
                            viewer.expand_fold_block(group_index, block_index, window, cx);
                        })
                        .log_err();
                    cx.stop_propagation();
                })
                .into_any_element()
        }),
        constrain_width: false,
        merge_adjacent: false,
        type_tag: Some(TypeId::of::<MergeFold>()),
        collapsed_text: Some(SharedString::from(COLLAPSED_PLACEHOLDER)),
    }
}

impl MergeViewer {
    pub(super) fn install_folds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remove_folds(cx);
        let fragments: Vec<FragmentLines> = {
            let model = self.model.read(cx);
            model
                .changes()
                .iter()
                .map(|change| FragmentLines {
                    lines: [
                        change.fragment_lines(ThreeSide::Left),
                        model.result_lines(change.index()),
                        change.fragment_lines(ThreeSide::Right),
                    ],
                })
                .collect()
        };
        let line_counts = ThreeSide::ALL.map(|side| self.editor_line_count(side, cx) as usize);
        let groups = compute_fold_groups(&fragments, line_counts, DEFAULT_CONTEXT_RANGE);
        let anchored_regions = self.capture_anchored_regions(&groups, cx);
        let group_count = groups.len();
        self.folds = Some(
            FoldState::new(groups, self.expand_by_default).with_anchored_regions(anchored_regions),
        );
        self.fold_anchors = vec![[None; 3]; group_count];
        for group_index in 0..group_count {
            self.apply_fold_group(group_index, window, cx);
        }
    }

    fn capture_anchored_regions(
        &self,
        groups: &[FoldGroup],
        cx: &App,
    ) -> Vec<Vec<[Option<Range<Anchor>>; 3]>> {
        let snapshots =
            ThreeSide::ALL.map(|side| self.editor(side).read(cx).buffer().read(cx).snapshot(cx));
        groups
            .iter()
            .map(|group| {
                group
                    .blocks
                    .iter()
                    .map(|block| {
                        ThreeSide::ALL.map(|side| {
                            let region = block.regions[side.index()].as_ref()?;
                            anchored_region(&snapshots[side.index()], region)
                        })
                    })
                    .collect()
            })
            .collect()
    }

    pub(super) fn remove_folds(&mut self, cx: &mut Context<Self>) {
        if self.folds.take().is_none() {
            return;
        }
        self.fold_anchors.clear();
        for side in ThreeSide::ALL {
            self.editor(side).update(cx, |editor, cx| {
                let snapshot = editor.buffer().read(cx).snapshot(cx);
                let whole_buffer = Point::new(0, 0)..snapshot.max_point();
                editor.remove_folds_with_type(
                    &[whole_buffer],
                    TypeId::of::<MergeFold>(),
                    false,
                    cx,
                );
            });
        }
    }

    fn apply_fold_group(
        &mut self,
        group_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.folds.as_ref() else {
            return;
        };
        let collapsed_block = state.collapsed_block(group_index);
        let group_regions =
            ThreeSide::ALL.map(|side| state.group_anchored_regions(group_index, side.index()));
        let visible_regions = ThreeSide::ALL.map(|side| {
            let block_index = collapsed_block?;
            state
                .anchored_region(group_index, block_index, side.index())
                .cloned()
        });
        if let Some(slot) = self.fold_anchors.get_mut(group_index) {
            *slot = visible_regions
                .clone()
                .map(|region| region.map(|region| region.start));
        }
        let weak_viewer = cx.weak_entity();
        for side in ThreeSide::ALL {
            let side_regions = &group_regions[side.index()];
            let visible_region = visible_regions[side.index()].clone();
            let placeholder = collapsed_block
                .map(|block_index| fold_placeholder(weak_viewer.clone(), group_index, block_index));
            self.editor(side).update(cx, |editor, cx| {
                editor.remove_folds_with_type(side_regions, TypeId::of::<MergeFold>(), false, cx);
                if let (Some(range), Some(placeholder)) = (visible_region, placeholder) {
                    editor.fold_creases(
                        vec![Crease::simple(range, placeholder)],
                        false,
                        window,
                        cx,
                    );
                }
            });
        }
    }

    pub(crate) fn expand_fold_block(
        &mut self,
        group_index: usize,
        block_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.folds.as_mut() else {
            return;
        };
        state.expand_block(group_index, block_index);
        state.clear_hover();
        self.apply_fold_group(group_index, window, cx);
        cx.notify();
    }

    pub(crate) fn collapse_unchanged_selected(&self) -> bool {
        !self.expand_by_default
    }

    pub(crate) fn set_collapse_unchanged(
        &mut self,
        collapse: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let expand = !collapse;
        if self.expand_by_default == expand {
            return;
        }
        self.expand_by_default = expand;
        viewer_settings::store_expand_by_default(expand, cx);
        let Some(state) = self.folds.as_mut() else {
            return;
        };
        state.expand_all(expand);
        let group_count = state.groups().len();
        for group_index in 0..group_count {
            self.apply_fold_group(group_index, window, cx);
        }
        cx.notify();
    }

    fn collapsed_group_start_line(
        &self,
        group_index: usize,
        side: ThreeSide,
        cx: &App,
    ) -> Option<u32> {
        let anchor = self.fold_anchors.get(group_index)?[side.index()]?;
        let snapshot = self.editor(side).read(cx).buffer().read(cx).snapshot(cx);
        Some(anchor.to_point(&snapshot).row)
    }

    fn hovered_fold_group(&self) -> Option<usize> {
        self.folds.as_ref()?.hovered().map(|hovered| hovered.group)
    }

    pub(crate) fn hover_fold(
        &mut self,
        group_index: usize,
        side: ThreeSide,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.folds.as_mut() else {
            return;
        };
        let hovered = HoveredFold {
            group: group_index,
            side,
        };
        if state.hover(hovered) {
            cx.notify();
        }
    }

    pub(crate) fn unhover_fold(&mut self, side: ThreeSide, cx: &mut Context<Self>) {
        let Some(state) = self.folds.as_mut() else {
            return;
        };
        if state.unhover(side) {
            cx.notify();
        }
    }

    pub(super) fn folded_lines(&self, side: ThreeSide, cx: &App) -> Vec<FoldedLine> {
        let hovered_group = self.hovered_fold_group();
        (0..self.fold_anchors.len())
            .filter_map(|group_index| {
                let line = self.collapsed_group_start_line(group_index, side, cx)?;
                let block = self.folds.as_ref()?.collapsed_block(group_index)?;
                Some(FoldedLine {
                    group: group_index,
                    block,
                    line,
                    hovered: hovered_group == Some(group_index),
                })
            })
            .collect()
    }

    pub(super) fn fold_connectors(&self, divider: Side, cx: &App) -> Vec<FoldConnector> {
        let (left_side, right_side) = match divider {
            Side::Left => (ThreeSide::Left, ThreeSide::Base),
            Side::Right => (ThreeSide::Base, ThreeSide::Right),
        };
        let hovered_group = self.hovered_fold_group();
        (0..self.fold_anchors.len())
            .filter_map(|group_index| {
                let left_line = self.collapsed_group_start_line(group_index, left_side, cx)?;
                let right_line = self.collapsed_group_start_line(group_index, right_side, cx)?;
                Some(FoldConnector {
                    left_line,
                    right_line,
                    hovered: hovered_group == Some(group_index),
                })
            })
            .collect()
    }
}
