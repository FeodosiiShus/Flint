use editor::{Editor, SelectionEffects, scroll::Autoscroll};
use gpui::{App, Context, Entity, Focusable as _, Window};
use language::Point;
use merge_diff::ThreeSide;

use super::MergeViewer;

fn move_caret_to_line(
    editor: &Entity<Editor>,
    line: usize,
    window: &mut Window,
    cx: &mut Context<MergeViewer>,
) {
    editor.update(cx, |editor, cx| {
        let snapshot = editor.display_snapshot(cx);
        let last_row = snapshot.buffer_snapshot().max_point().row;
        let target = Point::new((line as u32).min(last_row), 0);
        editor.change_selections(SelectionEffects::no_scroll(), window, cx, |selections| {
            selections.select_ranges([target..target]);
        });
    });
}

impl MergeViewer {
    pub(crate) fn caret_point(&self, side: ThreeSide, cx: &mut App) -> Point {
        self.editor(side).update(cx, |editor, cx| {
            let snapshot = editor.display_snapshot(cx);
            editor.selections.newest::<Point>(&snapshot).head()
        })
    }

    pub(crate) fn caret_line_and_line_count(&self, side: ThreeSide, cx: &mut App) -> (i64, i64) {
        let caret_line = i64::from(self.caret_point(side, cx).row);
        (caret_line, self.editor_line_count(side, cx))
    }

    pub(super) fn scroll_to_change(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (starts, ends) = {
            let model = self.model.read(cx);
            if model.change(index).is_none() {
                return;
            }
            (
                ThreeSide::ALL.map(|side| model.start_line(index, side)),
                ThreeSide::ALL.map(|side| model.end_line(index, side)),
            )
        };
        for side in ThreeSide::ALL {
            let editor = self.editor(side).clone();
            move_caret_to_line(&editor, starts[side.index()], window, cx);
        }
        self.make_range_visible(self.current_side, starts, ends, window, cx);
    }

    pub(super) fn scroll_to_first_unresolved_change(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let first_unresolved = self
            .model
            .read(cx)
            .changes()
            .iter()
            .find(|change| !change.is_resolved())
            .map(|change| change.index());
        if let Some(index) = first_unresolved {
            self.scroll_to_change(index, window, cx);
        }
    }

    pub(crate) fn focus_opposite_pane(
        &mut self,
        scroll_to_position: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self.current_side;
        let target = match current {
            ThreeSide::Left => ThreeSide::Base,
            ThreeSide::Base => ThreeSide::Right,
            ThreeSide::Right => ThreeSide::Left,
        };
        if scroll_to_position {
            let caret = self.caret_point(current, cx);
            let caret_line = i64::from(caret.row);
            let caret_column = caret.column;
            let target_line = self
                .transfer_line_between(current, target, caret_line, cx)
                .max(0) as u32;
            self.editor(target).update(cx, |editor, cx| {
                let snapshot = editor.display_snapshot(cx);
                let last_row = snapshot.buffer_snapshot().max_point().row;
                let row = target_line.min(last_row);
                let line_length = snapshot
                    .buffer_snapshot()
                    .line_len(multi_buffer::MultiBufferRow(row));
                let position = Point::new(row, caret_column.min(line_length));
                editor.change_selections(
                    SelectionEffects::scroll(Autoscroll::fit()),
                    window,
                    cx,
                    |selections| {
                        selections.select_ranges([position..position]);
                    },
                );
            });
        } else {
            self.editor(target).update(cx, |editor, cx| {
                editor.request_autoscroll(Autoscroll::fit(), cx);
            });
        }
        self.current_side = target;
        let focus_handle = self.editor(target).focus_handle(cx);
        window.focus(&focus_handle, cx);
        cx.notify();
    }
}
