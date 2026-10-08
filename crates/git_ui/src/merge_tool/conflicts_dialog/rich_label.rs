use gpui::{FontWeight, HighlightStyle, Hsla, StyledText};
use std::ops::Range;
use ui::prelude::*;

use crate::merge_tool::conflict_resolution::rich_text::{RichSegment, RichStyle, RichText};

struct Flattened {
    text: String,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
}

fn flatten(rich_text: &RichText, code_background: Hsla) -> Flattened {
    let mut text = String::new();
    let mut highlights = Vec::new();
    for segment in rich_text.segments() {
        match segment {
            RichSegment::LineBreak => text.push('\n'),
            RichSegment::Text { text: part, style } => {
                let start = text.len();
                text.push_str(part);
                let range = start..text.len();
                match style {
                    RichStyle::Plain => {}
                    RichStyle::Bold => highlights.push((
                        range,
                        HighlightStyle {
                            font_weight: Some(FontWeight::BOLD),
                            ..Default::default()
                        },
                    )),
                    RichStyle::Code => highlights.push((
                        range,
                        HighlightStyle {
                            background_color: Some(code_background),
                            ..Default::default()
                        },
                    )),
                }
            }
        }
    }
    Flattened { text, highlights }
}

pub(super) fn rich_label(rich_text: &RichText, cx: &App) -> StyledText {
    let flattened = flatten(rich_text, cx.theme().colors().element_background);
    StyledText::new(flattened.text).with_highlights(flattened.highlights)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_tool_flatten_records_bold_and_code_ranges() {
        let rich = RichText::new()
            .with_plain("Merging branch ")
            .with_bold("feature")
            .with_plain(" into ")
            .with_code("abc1234")
            .with_line_break()
            .with_plain("end");
        let code_background = gpui::hsla(0.25, 0.5, 0.5, 1.);
        let flattened = flatten(&rich, code_background);
        assert_eq!(flattened.text, "Merging branch feature into abc1234\nend");
        assert_eq!(flattened.highlights.len(), 2);
        assert_eq!(flattened.highlights[0].0, 15..22);
        assert_eq!(
            flattened.highlights[0].1.font_weight,
            Some(FontWeight::BOLD)
        );
        assert_eq!(flattened.highlights[1].0, 28..35);
        assert_eq!(
            flattened.highlights[1].1.background_color,
            Some(code_background)
        );
    }

    #[test]
    fn merge_tool_flatten_of_plain_text_has_no_highlights() {
        let flattened = flatten(
            &RichText::plain("The following files have conflicts:"),
            Hsla::default(),
        );
        assert_eq!(flattened.text, "The following files have conflicts:");
        assert!(flattened.highlights.is_empty());
    }
}
