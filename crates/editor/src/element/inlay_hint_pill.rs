use gpui::{Bounds, Font, HighlightStyle, Pixels, Point, TextRun, TextStyle, point, px, size};
use project::InlayId;
use std::ops::Range;

const MARGIN: Pixels = px(2.);
const PADDING: Pixels = px(6.);
const VERTICAL_GAP: Pixels = px(1.);
const BASELINE_RAISE: Pixels = px(1.);
const FONT_SIZE_DELTA: Pixels = px(-1.);
const MINIMUM_FONT_SIZE: Pixels = px(1.);
const CORNER_ARC_AT_DEFAULT_SYSTEM_FONT_SIZE: f32 = 8.;
const DEFAULT_SYSTEM_FONT_SIZE: f32 = 13.;

pub(crate) fn inlay_hint_font_size(buffer_font_size: Pixels) -> Pixels {
    (buffer_font_size + FONT_SIZE_DELTA).max(MINIMUM_FONT_SIZE)
}

pub(super) fn inlay_hint_corner_radius(hint_font_size: Pixels) -> Pixels {
    let arc = (CORNER_ARC_AT_DEFAULT_SYSTEM_FONT_SIZE * hint_font_size.as_f32()
        / DEFAULT_SYSTEM_FONT_SIZE)
        .round();
    px(arc / 2.)
}

pub(super) fn inlay_hint_text_offset_y(
    editor_ascent: Pixels,
    editor_descent: Pixels,
    hint_ascent: Pixels,
    hint_descent: Pixels,
) -> Pixels {
    ((editor_ascent - editor_descent) - (hint_ascent - hint_descent)) / 2. - BASELINE_RAISE
}

pub(crate) fn inlay_hint_text_run(
    font: &Font,
    len: usize,
    highlight: Option<HighlightStyle>,
    text_style: &TextStyle,
) -> TextRun {
    TextRun {
        len,
        font: Font {
            family: font.family.clone(),
            features: font.features.clone(),
            fallbacks: font.fallbacks.clone(),
            weight: highlight
                .and_then(|highlight| highlight.font_weight)
                .unwrap_or(font.weight),
            style: highlight
                .and_then(|highlight| highlight.font_style)
                .unwrap_or(font.style),
        },
        color: text_style.color,
        background_color: None,
        underline: text_style.underline,
        strikethrough: text_style.strikethrough,
    }
}

fn clip_text_runs(runs: &mut Vec<TextRun>, range: &Range<usize>) {
    let mut run_start = 0;
    runs.retain_mut(|run| {
        let run_end = run_start + run.len;
        let clipped_len = run_end
            .min(range.end)
            .saturating_sub(run_start.max(range.start));
        run_start = run_end;
        run.len = clipped_len;
        clipped_len > 0
    });
}

#[derive(Default)]
pub(crate) struct InlayHintPiece {
    id: Option<InlayId>,
    text: String,
    runs: Vec<TextRun>,
}

impl InlayHintPiece {
    pub(crate) fn id(&self) -> Option<InlayId> {
        self.id
    }

    pub(crate) fn len(&self) -> usize {
        self.text.len()
    }

    pub(crate) fn push(&mut self, id: InlayId, text: &str, run: TextRun) {
        self.id = Some(id);
        self.text.push_str(text);
        self.runs.push(run);
    }

    pub(crate) fn finish<T: Default>(
        &mut self,
        shape_content: impl FnOnce(&str, &[TextRun], usize) -> (T, Pixels),
    ) -> Option<(InlayHintPill, T)> {
        if self.id.take().is_none() {
            return None;
        }
        let content = InlayHintPill::content_range(&self.text);
        clip_text_runs(&mut self.runs, &content);
        let (shaped_content, content_width) = match self.text.get(content.clone()) {
            Some(content_text) if !content_text.is_empty() => {
                shape_content(content_text, &self.runs, content.start)
            }
            _ => (T::default(), Pixels::ZERO),
        };
        let pill = InlayHintPill::new(&self.text, content_width);
        self.text.clear();
        self.runs.clear();
        Some((pill, shaped_content))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InlayHintPill {
    len: usize,
    content: Range<usize>,
    last_char_start: usize,
    text_width: Pixels,
}

impl InlayHintPill {
    pub(crate) fn content_range(text: &str) -> Range<usize> {
        let text_without_leading_whitespace = text.trim_start();
        let start = text.len() - text_without_leading_whitespace.len();
        start..start + text_without_leading_whitespace.trim_end().len()
    }

    pub(crate) fn new(text: &str, text_width: Pixels) -> Self {
        Self {
            len: text.len(),
            content: Self::content_range(text),
            last_char_start: text
                .char_indices()
                .next_back()
                .map_or(0, |(index, _)| index),
            text_width,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(super) fn has_content(&self) -> bool {
        !self.content.is_empty()
    }

    pub(crate) fn width(&self) -> Pixels {
        if self.has_content() {
            self.pill_width() + MARGIN * 2.
        } else {
            Pixels::ZERO
        }
    }

    pub(super) fn pill_width(&self) -> Pixels {
        self.text_width + PADDING * 2.
    }

    pub(super) fn text_x(&self) -> Pixels {
        PADDING
    }

    pub(super) fn pill_bounds(&self, origin: Point<Pixels>, line_height: Pixels) -> Bounds<Pixels> {
        Bounds::new(
            origin + point(Pixels::ZERO, VERTICAL_GAP),
            size(
                self.pill_width(),
                (line_height - VERTICAL_GAP * 2.).max(Pixels::ZERO),
            ),
        )
    }

    pub(super) fn content_index(&self, index: usize) -> Option<usize> {
        self.content
            .contains(&index)
            .then(|| index - self.content.start)
    }

    pub(crate) fn x_for_index(
        &self,
        index: usize,
        content_x_for_index: impl FnOnce(usize) -> Pixels,
    ) -> Pixels {
        if !self.has_content() || index == 0 || index < self.content.start {
            Pixels::ZERO
        } else if let Some(content_index) = self.content_index(index) {
            PADDING + content_x_for_index(content_index)
        } else {
            PADDING + self.text_width
        }
    }

    pub(super) fn index_for_x(
        &self,
        x: Pixels,
        content_index_for_x: impl FnOnce(Pixels) -> Option<usize>,
    ) -> usize {
        if x < PADDING {
            0
        } else if x < PADDING + self.text_width {
            content_index_for_x(x - PADDING).map_or(self.last_char_start, |content_index| {
                self.content.start + content_index
            })
        } else {
            self.last_char_start
        }
    }

    pub(crate) fn closest_index_for_x(
        &self,
        x: Pixels,
        content_glyph_positions: impl IntoIterator<Item = (usize, Pixels)>,
    ) -> usize {
        let mut closest_index = 0;
        let mut closest_distance = x.abs();
        let mut consider = |index: usize, index_x: Pixels| {
            let distance = (index_x - x).abs();
            if distance < closest_distance {
                closest_index = index;
                closest_distance = distance;
            }
        };
        if self.has_content() {
            let mut previous_content_index = None;
            for (content_index, glyph_x) in content_glyph_positions {
                let index = self.content.start + content_index;
                if index > 0 && previous_content_index != Some(content_index) {
                    consider(index, PADDING + glyph_x);
                }
                previous_content_index = Some(content_index);
            }
            if self.content.end < self.len {
                consider(self.content.end, PADDING + self.text_width);
            }
        }
        consider(self.len, self.width());
        closest_index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEBSTORM_LINE_HEIGHT: Pixels = px(22.);
    const MONOSPACE_ADVANCE: Pixels = px(4.);

    fn monospace_x_for_index(index: usize) -> Pixels {
        MONOSPACE_ADVANCE * index
    }

    fn monospace_index_for_x(content_len: usize) -> impl Fn(Pixels) -> Option<usize> {
        move |x| {
            let index = (x / MONOSPACE_ADVANCE).floor() as usize;
            (index < content_len).then_some(index)
        }
    }

    fn monospace_pill(text: &str) -> InlayHintPill {
        let content = InlayHintPill::content_range(text);
        InlayHintPill::new(text, MONOSPACE_ADVANCE * content.len())
    }

    fn baseline(line_top: Pixels, line_height: Pixels, ascent: Pixels, descent: Pixels) -> Pixels {
        line_top + (line_height - ascent - descent) / 2. + ascent
    }

    #[test]
    fn inlay_hint_pill_matches_the_webstorm_title_hint() {
        let pill = InlayHintPill::new("title:", px(24.));

        assert_eq!(
            pill.pill_width(),
            px(36.),
            "the pill is the text plus 6 px of padding on each side"
        );
        assert_eq!(
            pill.width(),
            px(40.),
            "the inlay adds a 2 px margin on each side, both ending up right of the pill"
        );
        assert_eq!(pill.text_x(), px(6.), "the text starts after the padding");
        assert_eq!(
            pill.pill_bounds(point(px(100.), px(44.)), WEBSTORM_LINE_HEIGHT),
            Bounds::new(point(px(100.), px(45.)), size(px(36.), px(20.))),
            "the pill starts at the inlay start, 1 px below the line top, and leaves 1 px at the bottom"
        );
    }

    #[test]
    fn inlay_hint_pill_ignores_lsp_padding_spaces() {
        let padded_right = InlayHintPill::new("title: ", px(24.));
        let padded_left = InlayHintPill::new(" : string", px(39.));

        assert_eq!(
            padded_right.len(),
            7,
            "the padding stays part of the inlay text"
        );
        assert_eq!(
            padded_right.width(),
            px(40.),
            "a trailing LSP padding space adds no width"
        );
        assert_eq!(InlayHintPill::content_range(" : string"), 1..9);
        assert_eq!(
            padded_left.width(),
            px(55.),
            "a leading LSP padding space adds no width"
        );
    }

    #[test]
    fn inlay_hint_pill_without_content_takes_no_space() {
        let pill = InlayHintPill::new(" ", Pixels::ZERO);

        assert!(!pill.has_content());
        assert_eq!(
            pill.width(),
            Pixels::ZERO,
            "a soft wrap can leave only a padding space of a hint on a row"
        );
        assert_eq!(pill.x_for_index(0, monospace_x_for_index), Pixels::ZERO);
    }

    #[test]
    fn inlay_hint_pill_x_for_index_maps_through_the_shaped_text() {
        let pill = monospace_pill("ab");

        assert_eq!(
            pill.x_for_index(0, monospace_x_for_index),
            Pixels::ZERO,
            "a cursor before the hint stays at the pill start"
        );
        assert_eq!(pill.x_for_index(1, monospace_x_for_index), px(10.));

        let padded = monospace_pill(" ab ");
        assert_eq!(padded.x_for_index(0, monospace_x_for_index), Pixels::ZERO);
        assert_eq!(padded.x_for_index(1, monospace_x_for_index), px(6.));
        assert_eq!(padded.x_for_index(2, monospace_x_for_index), px(10.));
        assert_eq!(
            padded.x_for_index(3, monospace_x_for_index),
            px(14.),
            "a trailing padding space starts at the end of the text"
        );
    }

    #[test]
    fn inlay_hint_pill_index_for_x_maps_padding_to_the_hint_edges() {
        let pill = monospace_pill("abc");
        let index_for_x = monospace_index_for_x(3);

        assert_eq!(pill.index_for_x(px(0.), &index_for_x), 0);
        assert_eq!(
            pill.index_for_x(px(5.9), &index_for_x),
            0,
            "the left padding belongs to the first character"
        );
        assert_eq!(pill.index_for_x(px(6.), &index_for_x), 0);
        assert_eq!(pill.index_for_x(px(10.), &index_for_x), 1);
        assert_eq!(pill.index_for_x(px(17.9), &index_for_x), 2);
        assert_eq!(
            pill.index_for_x(px(18.), &index_for_x),
            2,
            "the right padding and margins belong to the last character"
        );
        assert_eq!(pill.index_for_x(pill.width() - px(0.1), &index_for_x), 2);

        let padded = monospace_pill(" ab ");
        let padded_index_for_x = monospace_index_for_x(2);
        assert_eq!(padded.index_for_x(px(3.), &padded_index_for_x), 0);
        assert_eq!(padded.index_for_x(px(7.), &padded_index_for_x), 1);
        assert_eq!(padded.index_for_x(px(11.), &padded_index_for_x), 2);
        assert_eq!(padded.index_for_x(px(15.), &padded_index_for_x), 3);
    }

    #[test]
    fn inlay_hint_pill_index_for_x_inverts_x_for_index_inside_the_text() {
        let pill = monospace_pill(" label ");
        let index_for_x = monospace_index_for_x(5);

        for index in 1..6 {
            let x = pill.x_for_index(index, monospace_x_for_index);
            assert_eq!(
                pill.index_for_x(x, &index_for_x),
                index,
                "hovering a character of the hint must resolve to that character"
            );
        }
    }

    #[test]
    fn inlay_hint_pill_index_for_x_uses_character_boundaries() {
        let pill = InlayHintPill::new("é→", px(8.));

        assert_eq!(
            pill.index_for_x(px(15.), |_| None),
            2,
            "the right edge resolves to the start byte of the last character"
        );
        assert_eq!(pill.content_index(2), Some(2));
        assert_eq!(pill.content_index(5), None);
    }

    #[test]
    fn inlay_hint_pill_closest_index_for_x_snaps_to_the_nearest_hint_boundary() {
        let pill = monospace_pill(" ab ");
        let glyphs = [(0, px(0.)), (1, MONOSPACE_ADVANCE)];
        let closest = |x: f32| pill.closest_index_for_x(px(x), glyphs);

        assert_eq!(
            closest(2.9),
            0,
            "the left padding belongs to the hint start"
        );
        assert_eq!(closest(3.1), 1);
        assert_eq!(closest(8.1), 2);
        assert_eq!(
            closest(12.1),
            3,
            "the trailing LSP padding sits at the end of the text"
        );
        assert_eq!(closest(18.9), 3);
        assert_eq!(
            closest(19.1),
            4,
            "the right padding and margins snap to the end of the hint"
        );
        for index in 1..4 {
            assert_eq!(
                closest(pill.x_for_index(index, monospace_x_for_index).as_f32()),
                index,
                "vertical movement aims at the x the hint character is drawn at"
            );
        }
    }

    #[test]
    fn inlay_hint_pill_font_is_one_pixel_smaller_than_the_buffer_font() {
        assert_eq!(inlay_hint_font_size(px(13.)), px(12.));
        assert_eq!(
            inlay_hint_font_size(px(1.5)),
            px(1.),
            "the hint font never shrinks below 1 px"
        );
    }

    #[test]
    fn inlay_hint_pill_corner_radius_scales_with_the_hint_font() {
        assert_eq!(
            inlay_hint_corner_radius(px(12.)),
            px(3.5),
            "a 12 px hint font gives IntelliJ's 7 px arc on macOS"
        );
        assert_eq!(inlay_hint_corner_radius(px(13.)), px(4.));
    }

    #[test]
    fn inlay_hint_pill_text_baseline_sits_one_pixel_above_the_code_baseline() {
        let line_top = px(44.);
        let line_height = WEBSTORM_LINE_HEIGHT;
        let (editor_ascent, editor_descent) = (px(13.26), px(3.9));
        let (hint_ascent, hint_descent) = (px(11.625), px(2.906));

        let offset =
            inlay_hint_text_offset_y(editor_ascent, editor_descent, hint_ascent, hint_descent);
        let code_baseline = baseline(line_top, line_height, editor_ascent, editor_descent);
        let hint_baseline = baseline(line_top + offset, line_height, hint_ascent, hint_descent);

        assert!(
            (code_baseline - px(1.) - hint_baseline).abs() < px(0.001),
            "hint baseline {hint_baseline} should be 1 px above the code baseline {code_baseline}"
        );
    }

    #[test]
    fn inlay_hint_pill_clip_text_runs_drops_the_padding() {
        let run = |len| TextRun {
            len,
            ..TextRun::default()
        };
        let run_lengths = |runs: &[TextRun]| runs.iter().map(|run| run.len).collect::<Vec<_>>();

        let mut runs = vec![run(3), run(4)];
        clip_text_runs(&mut runs, &(1..6));
        assert_eq!(run_lengths(&runs), vec![2, 3]);

        let mut runs = vec![run(1), run(6)];
        clip_text_runs(&mut runs, &(1..7));
        assert_eq!(
            run_lengths(&runs),
            vec![6],
            "a run holding only the leading padding disappears"
        );

        let mut runs = vec![run(2)];
        clip_text_runs(&mut runs, &(1..1));
        assert!(runs.is_empty());
    }

    #[test]
    fn inlay_hint_pill_piece_shapes_only_the_content_of_one_hint() {
        let run = |len| TextRun {
            len,
            ..TextRun::default()
        };
        let mut piece = InlayHintPiece::default();
        piece.push(InlayId::Hint(7), " ti", run(3));
        piece.push(InlayId::Hint(7), "tle: ", run(5));
        assert_eq!(piece.id(), Some(InlayId::Hint(7)));
        assert_eq!(piece.len(), 8);

        let mut shaped = None;
        let finished = piece.finish(|content_text, content_runs, content_start| {
            shaped = Some((
                content_text.to_string(),
                content_runs.iter().map(|run| run.len).collect::<Vec<_>>(),
                content_start,
            ));
            ((), px(24.))
        });

        assert_eq!(
            shaped,
            Some(("title:".to_string(), vec![2, 4], 1)),
            "only the label is shaped, without the LSP padding spaces"
        );
        assert_eq!(
            finished.map(|(pill, ())| pill),
            Some(InlayHintPill::new(" title: ", px(24.)))
        );
        assert_eq!(piece.id(), None, "the next hint starts a new piece");
        assert_eq!(piece.len(), 0);
        assert!(piece.finish(|_, _, _| ((), px(1.))).is_none());
    }
}
