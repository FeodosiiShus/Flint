use std::rc::Rc;

use gpui::{App, Context, Window};
use merge_diff::{ConflictKind, MergeConflictType, Side, ThreeSide};
use util::ResultExt as _;

use super::MergeViewer;
use crate::merge_tool::{
    merge_gutter::{GutterIconClick, GutterIconKind, GutterIconSpec},
    merge_ribbons::RibbonChange,
    palette::{
        ChangeKind,
        highlights::{PaneChange, apply_pane_highlights},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GutterAction {
    Accept(Side),
    Ignore(Side),
    Resolve,
}

pub(super) fn change_kind(conflict_type: &MergeConflictType) -> ChangeKind {
    match conflict_type.kind {
        ConflictKind::Inserted => ChangeKind::Inserted,
        ConflictKind::Deleted => ChangeKind::Deleted,
        ConflictKind::Modified => ChangeKind::Modified,
        ConflictKind::Conflict => ChangeKind::Conflict,
    }
}

impl MergeViewer {
    pub(super) fn pane_changes(&self, side: ThreeSide, cx: &App) -> Vec<PaneChange> {
        let model = self.model.read(cx);
        model
            .changes()
            .iter()
            .filter(|change| change.is_change(side))
            .map(|change| {
                let index = change.index();
                let lines =
                    model.start_line(index, side) as u32..model.end_line(index, side) as u32;
                let resolved = change.is_resolved_for(side);
                let inner = self
                    .inner_fragments
                    .get(index)
                    .and_then(|fragments| fragments.as_ref());
                let inner_side_ranges = inner.and_then(|inner| match side {
                    ThreeSide::Left => inner.left.as_ref(),
                    ThreeSide::Base => inner.base.as_ref(),
                    ThreeSide::Right => inner.right.as_ref(),
                });
                PaneChange {
                    lines,
                    kind: change_kind(change.conflict_type()),
                    resolved,
                    word_diff_ready: !resolved && inner.is_some(),
                    ai_resolved: false,
                    hide_without_line_numbers: side == ThreeSide::Base
                        && !change.is_change(ThreeSide::Left)
                        && change.is_change(ThreeSide::Right),
                    inner_ranges: if resolved {
                        Vec::new()
                    } else {
                        inner_side_ranges.cloned().unwrap_or_default()
                    },
                }
            })
            .collect()
    }

    pub(super) fn ribbon_changes(&self, divider: Side, cx: &App) -> Vec<RibbonChange> {
        let model = self.model.read(cx);
        let outer_side = match divider {
            Side::Left => ThreeSide::Left,
            Side::Right => ThreeSide::Right,
        };
        model
            .changes()
            .iter()
            .filter(|change| change.is_change(outer_side))
            .map(|change| {
                let index = change.index();
                let outer = model.start_line(index, outer_side) as u32
                    ..model.end_line(index, outer_side) as u32;
                let result = model.start_line(index, ThreeSide::Base) as u32
                    ..model.end_line(index, ThreeSide::Base) as u32;
                let (left_lines, right_lines) = match divider {
                    Side::Left => (outer, result),
                    Side::Right => (result, outer),
                };
                RibbonChange {
                    left_lines,
                    right_lines,
                    kind: change_kind(change.conflict_type()),
                    resolved: change.is_side_resolved(divider),
                    ai_resolved: false,
                }
            })
            .collect()
    }

    pub(super) fn gutter_icons(
        &self,
        side: ThreeSide,
        cx: &mut Context<Self>,
    ) -> Vec<GutterIconSpec> {
        let weak_viewer = cx.weak_entity();
        let model = self.model.read(cx);
        let mut icons = Vec::new();
        for change in model.changes() {
            let index = change.index();
            let line = model.start_line(index, side) as u32;
            let conflict = change.is_conflict();
            let mut push = |kind: GutterIconKind, action: GutterAction| {
                let weak_viewer = weak_viewer.clone();
                icons.push(GutterIconSpec {
                    line,
                    kind,
                    conflict,
                    enabled: true,
                    on_click: Rc::new(
                        move |click: GutterIconClick, window: &mut Window, cx: &mut App| {
                            weak_viewer
                                .update(cx, |viewer, cx| {
                                    viewer.run_gutter_action(action, index, click.ctrl, window, cx);
                                })
                                .log_err();
                        },
                    ),
                });
            };
            match side {
                ThreeSide::Base => {
                    if !change.is_resolved()
                        && conflict
                        && model.can_resolve_change_automatically(index, ThreeSide::Base)
                    {
                        push(GutterIconKind::Resolve, GutterAction::Resolve);
                    }
                }
                ThreeSide::Left | ThreeSide::Right => {
                    let source = if side == ThreeSide::Left {
                        Side::Left
                    } else {
                        Side::Right
                    };
                    if change.is_side_resolved(source) || !change.is_change(side) {
                        continue;
                    }
                    let accept_kind = match (source, change.is_oneside_applied_conflict()) {
                        (Side::Left, false) => GutterIconKind::AcceptFromLeft,
                        (Side::Left, true) => GutterIconKind::AppendFromLeft,
                        (Side::Right, false) => GutterIconKind::AcceptFromRight,
                        (Side::Right, true) => GutterIconKind::AppendFromRight,
                    };
                    push(accept_kind, GutterAction::Accept(source));
                    push(GutterIconKind::Ignore, GutterAction::Ignore(source));
                }
            }
        }
        icons
    }

    pub(super) fn refresh_highlights(&mut self, cx: &mut Context<Self>) {
        for side in ThreeSide::ALL {
            let changes = self.pane_changes(side, cx);
            self.editor(side)
                .update(cx, |editor, cx| apply_pane_highlights(editor, &changes, cx));
        }
        cx.notify();
    }
}
