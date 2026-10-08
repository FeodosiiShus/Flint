use std::ops::Range;

use editor::{
    Anchor, Editor, HighlightKey, MultiBufferOffset, MultiBufferSnapshot, RowHighlightOptions,
};
use gpui::{App, Context, Hsla};
use language::Point;

use super::{ChangeKind, change_colors};

const ROW_OPTIONS: RowHighlightOptions = RowHighlightOptions {
    autoscroll: false,
    include_gutter: false,
};

const STATE_FULL: u8 = 0;
const STATE_FADED: u8 = 1;

const INLINE_KEYS: [HighlightKey; 4] = [
    HighlightKey::MergeInlineInserted,
    HighlightKey::MergeInlineDeleted,
    HighlightKey::MergeInlineModified,
    HighlightKey::MergeInlineConflict,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PaneChange {
    pub lines: Range<u32>,
    pub kind: ChangeKind,
    pub resolved: bool,
    pub word_diff_ready: bool,
    pub ai_resolved: bool,
    pub hide_without_line_numbers: bool,
    pub inner_ranges: Vec<Range<usize>>,
}

impl PaneChange {
    pub(crate) fn is_empty_range(&self) -> bool {
        self.lines.start >= self.lines.end
    }
}

struct RowSlot<const KIND: u8, const STATE: u8>;

fn slot_color<const KIND: u8, const STATE: u8>(cx: &App) -> Hsla {
    let colors = change_colors(cx, ChangeKind::from_index(KIND), false);
    if STATE == STATE_FADED {
        colors.faded
    } else {
        colors.solid
    }
}

fn highlight_slot<const KIND: u8, const STATE: u8>(
    editor: &mut Editor,
    range: Range<Anchor>,
    cx: &mut Context<Editor>,
) {
    editor.highlight_rows::<RowSlot<KIND, STATE>>(
        range,
        slot_color::<KIND, STATE>,
        ROW_OPTIONS,
        cx,
    );
}

fn clear_slot<const KIND: u8, const STATE: u8>(editor: &mut Editor) {
    editor.clear_row_highlights::<RowSlot<KIND, STATE>>();
}

fn highlight_row_range(
    editor: &mut Editor,
    kind: ChangeKind,
    word_diff_ready: bool,
    range: Range<Anchor>,
    cx: &mut Context<Editor>,
) {
    match (kind, word_diff_ready) {
        (ChangeKind::Inserted, false) => highlight_slot::<0, STATE_FULL>(editor, range, cx),
        (ChangeKind::Inserted, true) => highlight_slot::<0, STATE_FADED>(editor, range, cx),
        (ChangeKind::Deleted, false) => highlight_slot::<1, STATE_FULL>(editor, range, cx),
        (ChangeKind::Deleted, true) => highlight_slot::<1, STATE_FADED>(editor, range, cx),
        (ChangeKind::Modified, false) => highlight_slot::<2, STATE_FULL>(editor, range, cx),
        (ChangeKind::Modified, true) => highlight_slot::<2, STATE_FADED>(editor, range, cx),
        (ChangeKind::Conflict, false) => highlight_slot::<3, STATE_FULL>(editor, range, cx),
        (ChangeKind::Conflict, true) => highlight_slot::<3, STATE_FADED>(editor, range, cx),
    }
}

fn line_start_point(snapshot: &MultiBufferSnapshot, line: u32) -> Point {
    let max_point = snapshot.max_point();
    if line > max_point.row {
        max_point
    } else {
        Point::new(line, 0)
    }
}

fn line_anchor_range(snapshot: &MultiBufferSnapshot, lines: &Range<u32>) -> Range<Anchor> {
    snapshot.anchor_before(line_start_point(snapshot, lines.start))
        ..snapshot.anchor_before(line_start_point(snapshot, lines.end))
}

fn inline_anchor_range(
    snapshot: &MultiBufferSnapshot,
    base: MultiBufferOffset,
    range: &Range<usize>,
) -> Range<Anchor> {
    let length = snapshot.len().0;
    let start = (base.0 + range.start).min(length);
    let end = (base.0 + range.end).clamp(start, length);
    snapshot.anchor_before(MultiBufferOffset(start))..snapshot.anchor_after(MultiBufferOffset(end))
}

pub(crate) fn clear_pane_highlights(editor: &mut Editor, cx: &mut Context<Editor>) {
    clear_slot::<0, STATE_FULL>(editor);
    clear_slot::<0, STATE_FADED>(editor);
    clear_slot::<1, STATE_FULL>(editor);
    clear_slot::<1, STATE_FADED>(editor);
    clear_slot::<2, STATE_FULL>(editor);
    clear_slot::<2, STATE_FADED>(editor);
    clear_slot::<3, STATE_FULL>(editor);
    clear_slot::<3, STATE_FADED>(editor);
    for key in INLINE_KEYS {
        editor.clear_background_highlights(key, cx);
    }
}

pub(crate) fn apply_pane_highlights(
    editor: &mut Editor,
    changes: &[PaneChange],
    cx: &mut Context<Editor>,
) {
    clear_pane_highlights(editor, cx);
    let snapshot = editor.buffer().read(cx).snapshot(cx);
    let mut inline_ranges: [Vec<Range<Anchor>>; 4] = Default::default();
    for change in changes {
        if change.resolved {
            continue;
        }
        if !change.is_empty_range() {
            let range = line_anchor_range(&snapshot, &change.lines);
            highlight_row_range(editor, change.kind, change.word_diff_ready, range, cx);
        }
        if change.word_diff_ready {
            let base = snapshot.point_to_offset(line_start_point(&snapshot, change.lines.start));
            let kind_ranges = &mut inline_ranges[change.kind.index()];
            for range in &change.inner_ranges {
                if range.start < range.end {
                    kind_ranges.push(inline_anchor_range(&snapshot, base, range));
                }
            }
        }
    }
    for kind in ChangeKind::ALL {
        let ranges = &inline_ranges[kind.index()];
        if ranges.is_empty() {
            continue;
        }
        let color = change_colors(cx, kind, false).solid;
        editor.highlight_background(INLINE_KEYS[kind.index()], ranges, move |_, _| color, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge_tool::palette::{MergePaletteKind, MergePaletteOverride, hex_color};
    use editor::ToPoint;
    use gpui::{AppContext as _, TestAppContext};
    use language::Buffer;
    use settings::SettingsStore;
    use theme::LoadThemes;

    const TEXT: &str = "zero\none\ntwo\nthree\nfour\nfive";

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(LoadThemes::JustBase, cx);
            editor::init(cx);
            cx.set_global(MergePaletteOverride(Some(MergePaletteKind::IslandsDark)));
        });
    }

    fn pane_change(lines: Range<u32>, kind: ChangeKind) -> PaneChange {
        PaneChange {
            lines,
            kind,
            resolved: false,
            word_diff_ready: false,
            ai_resolved: false,
            hide_without_line_numbers: false,
            inner_ranges: Vec::new(),
        }
    }

    fn highlighted_row_ranges<T: 'static>(editor: &Editor, cx: &App) -> Vec<Range<Point>> {
        let snapshot = editor.buffer().read(cx).snapshot(cx);
        editor
            .highlighted_rows::<T>(cx)
            .map(|(range, _)| range.start.to_point(&snapshot)..range.end.to_point(&snapshot))
            .collect()
    }

    fn inline_highlights(
        editor: &mut Editor,
        window: &mut gpui::Window,
        cx: &mut Context<Editor>,
    ) -> Vec<(u32, u32, u32, u32, Hsla)> {
        editor
            .all_text_background_highlights(window, cx)
            .into_iter()
            .map(|(range, color)| {
                (
                    range.start.row().0,
                    range.start.column(),
                    range.end.row().0,
                    range.end.column(),
                    color,
                )
            })
            .collect()
    }

    #[test]
    fn merge_tool_pane_change_reports_empty_range() {
        let change = pane_change(4..4, ChangeKind::Inserted);
        assert!(change.is_empty_range());
        let non_empty = PaneChange {
            lines: 4..6,
            ..change
        };
        assert!(!non_empty.is_empty_range());
    }

    #[gpui::test]
    async fn merge_tool_pane_highlights_paint_unresolved_changes_by_kind_and_word_diff_state(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let buffer = cx.new(|cx| Buffer::local(TEXT, cx));
        let (editor, cx) =
            cx.add_window_view(|window, cx| Editor::for_buffer(buffer, None, window, cx));
        let changes = vec![
            pane_change(1..3, ChangeKind::Modified),
            PaneChange {
                word_diff_ready: true,
                inner_ranges: vec![0..2],
                ..pane_change(4..5, ChangeKind::Inserted)
            },
            pane_change(5..5, ChangeKind::Conflict),
            PaneChange {
                resolved: true,
                ..pane_change(0..1, ChangeKind::Deleted)
            },
        ];

        editor.update(cx, |editor, cx| apply_pane_highlights(editor, &changes, cx));

        editor.update_in(cx, |editor, window, cx| {
            assert_eq!(
                highlighted_row_ranges::<RowSlot<2, STATE_FULL>>(editor, cx),
                vec![Point::new(1, 0)..Point::new(3, 0)]
            );
            let modified_colors: Vec<Hsla> = editor
                .highlighted_rows::<RowSlot<2, STATE_FULL>>(cx)
                .map(|(_, color)| color)
                .collect();
            assert_eq!(modified_colors, vec![hex_color(0x385570)]);
            assert_eq!(
                highlighted_row_ranges::<RowSlot<0, STATE_FADED>>(editor, cx),
                vec![Point::new(4, 0)..Point::new(5, 0)]
            );
            assert!(highlighted_row_ranges::<RowSlot<0, STATE_FULL>>(editor, cx).is_empty());
            assert!(highlighted_row_ranges::<RowSlot<3, STATE_FULL>>(editor, cx).is_empty());
            assert!(highlighted_row_ranges::<RowSlot<3, STATE_FADED>>(editor, cx).is_empty());
            assert!(highlighted_row_ranges::<RowSlot<1, STATE_FULL>>(editor, cx).is_empty());
            assert!(highlighted_row_ranges::<RowSlot<1, STATE_FADED>>(editor, cx).is_empty());
            assert_eq!(
                inline_highlights(editor, window, cx),
                vec![(4, 0, 4, 2, hex_color(0x294436))]
            );
            assert!(editor.has_background_highlights(HighlightKey::MergeInlineInserted));
            assert!(!editor.has_background_highlights(HighlightKey::MergeInlineModified));
        });
    }

    #[gpui::test]
    async fn merge_tool_pane_highlights_are_replaced_on_every_application(cx: &mut TestAppContext) {
        init_test(cx);
        let buffer = cx.new(|cx| Buffer::local(TEXT, cx));
        let (editor, cx) =
            cx.add_window_view(|window, cx| Editor::for_buffer(buffer, None, window, cx));
        let first = vec![PaneChange {
            word_diff_ready: true,
            inner_ranges: vec![1..3],
            ..pane_change(1..3, ChangeKind::Modified)
        }];
        let second = vec![pane_change(2..4, ChangeKind::Deleted)];

        editor.update(cx, |editor, cx| apply_pane_highlights(editor, &first, cx));
        editor.update(cx, |editor, cx| apply_pane_highlights(editor, &second, cx));

        editor.update_in(cx, |editor, window, cx| {
            assert!(highlighted_row_ranges::<RowSlot<2, STATE_FADED>>(editor, cx).is_empty());
            assert_eq!(
                highlighted_row_ranges::<RowSlot<1, STATE_FULL>>(editor, cx),
                vec![Point::new(2, 0)..Point::new(4, 0)]
            );
            assert!(!editor.has_background_highlights(HighlightKey::MergeInlineModified));
            assert!(inline_highlights(editor, window, cx).is_empty());
        });
    }

    #[gpui::test]
    async fn merge_tool_pane_highlights_clamp_a_range_ending_past_the_last_line(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let buffer = cx.new(|cx| Buffer::local(TEXT, cx));
        let (editor, cx) =
            cx.add_window_view(|window, cx| Editor::for_buffer(buffer, None, window, cx));
        let changes = vec![pane_change(4..9, ChangeKind::Modified)];

        editor.update(cx, |editor, cx| apply_pane_highlights(editor, &changes, cx));

        editor.update_in(cx, |editor, _window, cx| {
            assert_eq!(
                highlighted_row_ranges::<RowSlot<2, STATE_FULL>>(editor, cx),
                vec![Point::new(4, 0)..Point::new(5, 4)]
            );
        });
    }
}
