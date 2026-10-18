use crate::{BufferOffset, Editor, HighlightKey, RangeToAnchorExt, display_map::DisplaySnapshot};
use gpui::{AppContext, Context, HighlightStyle};
use language::{BufferSnapshot, CursorShape};
use multi_buffer::{MultiBufferOffset, MultiBufferSnapshot};
use std::ops::Range;
use theme::ActiveTheme;

impl Editor {
    #[ztracing::instrument(skip_all)]
    pub fn refresh_matching_bracket_highlights(
        &mut self,
        snapshot: &DisplaySnapshot,
        cx: &mut Context<Editor>,
    ) {
        let newest_selection = self.selections.newest::<MultiBufferOffset>(&snapshot);
        // Don't highlight brackets if the selection isn't empty
        if !newest_selection.is_empty() {
            self.clear_highlights(HighlightKey::MatchingBracket, cx);
            return;
        }

        let buffer_snapshot = snapshot.buffer_snapshot();
        let head = newest_selection.head();
        if head > buffer_snapshot.len() {
            log::error!("bug: cursor offset is out of range while refreshing bracket highlights");
            return;
        }

        let mut tail = head;
        if (self.cursor_shape == CursorShape::Block || self.cursor_shape == CursorShape::Hollow)
            && head < buffer_snapshot.len()
        {
            if let Some(tail_ch) = buffer_snapshot.chars_at(tail).next() {
                tail += tail_ch.len_utf8();
            }
        }
        let task = cx.background_spawn({
            let buffer_snapshot = buffer_snapshot.clone();
            futures::future::lazy(move |_| bracket_ranges_at_caret(&buffer_snapshot, head, tail))
        });
        self.refresh_matching_bracket_highlights_task = cx.spawn({
            let buffer_snapshot = buffer_snapshot.clone();
            async move |this, cx| {
                let bracket_ranges = task.await;
                let current_ranges = this
                    .read_with(cx, |editor, cx| {
                        editor
                            .display_map
                            .read(cx)
                            .text_highlights(HighlightKey::MatchingBracket)
                            .map(|(_, ranges)| ranges.to_vec())
                    })
                    .ok()
                    .flatten();
                let new_ranges = bracket_ranges.map(|(opening_range, closing_range)| {
                    vec![
                        opening_range.to_anchors(&buffer_snapshot),
                        closing_range.to_anchors(&buffer_snapshot),
                    ]
                });

                if current_ranges != new_ranges {
                    this.update(cx, |editor, cx| {
                        editor.clear_highlights(HighlightKey::MatchingBracket, cx);
                        if let Some(new_ranges) = new_ranges {
                            editor.highlight_text(
                                HighlightKey::MatchingBracket,
                                new_ranges,
                                HighlightStyle {
                                    background_color: Some(
                                        cx.theme()
                                            .colors()
                                            .editor_document_highlight_bracket_background,
                                    ),
                                    ..Default::default()
                                },
                                cx,
                            )
                        }
                    })
                    .ok();
                }
            }
        });
    }
}

fn bracket_ranges_at_caret(
    buffer_snapshot: &MultiBufferSnapshot,
    head: MultiBufferOffset,
    tail: MultiBufferOffset,
) -> Option<(Range<MultiBufferOffset>, Range<MultiBufferOffset>)> {
    if let Some(bracket_ranges) = bracket_ranges_touching(buffer_snapshot, head, head..tail) {
        return Some(bracket_ranges);
    }

    let blanks_before_head: usize = buffer_snapshot
        .reversed_chars_at(head)
        .take_while(|character| is_blank(*character))
        .map(char::len_utf8)
        .sum();
    if blanks_before_head > 0 {
        let offset = head - blanks_before_head;
        if let Some(bracket_ranges) =
            bracket_ranges_touching(buffer_snapshot, offset, offset..offset)
        {
            return Some(bracket_ranges);
        }
    }

    let blanks_after_head: usize = buffer_snapshot
        .chars_at(head)
        .take_while(|character| is_blank(*character))
        .map(char::len_utf8)
        .sum();
    if blanks_after_head > 0 {
        let offset = head + blanks_after_head;
        return bracket_ranges_touching(buffer_snapshot, offset, offset..offset);
    }

    None
}

fn bracket_ranges_touching(
    buffer_snapshot: &MultiBufferSnapshot,
    offset: MultiBufferOffset,
    range: Range<MultiBufferOffset>,
) -> Option<(Range<MultiBufferOffset>, Range<MultiBufferOffset>)> {
    buffer_snapshot
        .innermost_enclosing_bracket_ranges(range, Some(&is_brace_pair))
        .filter(|(opening_range, closing_range)| {
            [
                opening_range.start,
                opening_range.end,
                closing_range.start,
                closing_range.end,
            ]
            .contains(&offset)
        })
}

// Grammars list string quotes as bracket pairs, but IntelliJ's brace matchers never treat
// quotes as braces. The closing delimiter is checked because grammars alias prefixed openers
// such as Rust's `b"` to a quote, while the closer is always the bare quote.
fn is_brace_pair(
    buffer: &BufferSnapshot,
    _pair: Range<BufferOffset>,
    closing: Range<BufferOffset>,
) -> bool {
    !matches!(
        buffer.chars_at(closing.start.0).next(),
        Some('"' | '\'' | '`')
    )
}

fn is_blank(character: char) -> bool {
    character == ' ' || character == '\t'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{editor_tests::init_test, test::editor_lsp_test_context::EditorLspTestContext};
    use indoc::indoc;
    use language::{BracketPair, BracketPairConfig, Language, LanguageConfig, LanguageMatcher};

    fn rust_language(brackets_query: &str) -> Language {
        Language::new(
            LanguageConfig {
                name: "Rust".into(),
                matcher: (LanguageMatcher {
                    path_suffixes: vec!["rs".to_string()],
                    ..Default::default()
                })
                .into(),
                brackets: BracketPairConfig {
                    pairs: vec![
                        BracketPair {
                            start: "{".to_string(),
                            end: "}".to_string(),
                            close: false,
                            surround: false,
                            newline: true,
                        },
                        BracketPair {
                            start: "(".to_string(),
                            end: ")".to_string(),
                            close: false,
                            surround: false,
                            newline: true,
                        },
                    ],
                    ..Default::default()
                },
                ..Default::default()
            },
            Some(tree_sitter_rust::LANGUAGE.into()),
        )
        .with_brackets_query(brackets_query)
        .unwrap()
    }

    #[gpui::test]
    async fn test_matching_bracket_highlights(cx: &mut gpui::TestAppContext) {
        init_test(cx, |_| {});

        let mut cx = EditorLspTestContext::new(
            rust_language(indoc! {r#"
                ("{" @open "}" @close)
                ("(" @open ")" @close)
                "#}),
            Default::default(),
            cx,
        )
        .await;

        // positioning cursor inside bracket highlights both
        cx.set_state(indoc! {r#"
            pub fn test(ˇ"Test argument") {
                another_test(1, 2, 3);
            }
        "#});
        cx.run_until_parked();
        cx.assert_editor_text_highlights(
            HighlightKey::MatchingBracket,
            indoc! {r#"
            pub fn test«(»"Test argument"«)» {
                another_test(1, 2, 3);
            }
        "#},
        );

        cx.set_state(indoc! {r#"
            pub fn test("Test argument") {
                another_test(1, 2, 3)ˇ;
            }
        "#});
        cx.run_until_parked();
        cx.assert_editor_text_highlights(
            HighlightKey::MatchingBracket,
            indoc! {r#"
            pub fn test("Test argument") {
                another_test«(»1, 2, 3«)»;
            }
        "#},
        );

        cx.set_state(indoc! {r#"
            pub fn test("Test argument") {
                another_test( ˇ1, 2, 3);
            }
        "#});
        cx.run_until_parked();
        cx.assert_editor_text_highlights(
            HighlightKey::MatchingBracket,
            indoc! {r#"
            pub fn test("Test argument") {
                another_test«(» 1, 2, 3«)»;
            }
        "#},
        );

        cx.set_state(indoc! {r#"
            pub fn test("Test argument") ˇ{
                another_test(1, 2, 3);
            }
        "#});
        cx.run_until_parked();
        cx.assert_editor_text_highlights(
            HighlightKey::MatchingBracket,
            indoc! {r#"
            pub fn test("Test argument") «{»
                another_test(1, 2, 3);
            «}»
        "#},
        );

        cx.set_state(indoc! {r#"
            pub fn test("Test argument") {
                another_test(1, ˇ2, 3);
            }
        "#});
        cx.run_until_parked();
        cx.assert_editor_text_highlights(
            HighlightKey::MatchingBracket,
            indoc! {r#"
            pub fn test("Test argument") {
                another_test(1, 2, 3);
            }
        "#},
        );

        cx.set_state(indoc! {r#"
            pub fn test("Test argument") {
                anotherˇ_test(1, 2, 3);
            }
        "#});
        cx.run_until_parked();
        cx.assert_editor_text_highlights(
            HighlightKey::MatchingBracket,
            indoc! {r#"
            pub fn test("Test argument") {
                another_test(1, 2, 3);
            }
        "#},
        );

        // positioning outside of brackets removes highlight
        cx.set_state(indoc! {r#"
            pub fˇn test("Test argument") {
                another_test(1, 2, 3);
            }
        "#});
        cx.run_until_parked();
        cx.assert_editor_text_highlights(
            HighlightKey::MatchingBracket,
            indoc! {r#"
            pub fn test("Test argument") {
                another_test(1, 2, 3);
            }
        "#},
        );

        // non empty selection dismisses highlight
        cx.set_state(indoc! {r#"
            pub fn test("Te«st argˇ»ument") {
                another_test(1, 2, 3);
            }
        "#});
        cx.run_until_parked();
        cx.assert_editor_text_highlights(
            HighlightKey::MatchingBracket,
            indoc! {r#"
            pub fn test«("Test argument") {
                another_test(1, 2, 3);
            }
        "#},
        );
    }

    #[gpui::test]
    async fn test_matching_bracket_highlights_skip_quotes(cx: &mut gpui::TestAppContext) {
        init_test(cx, |_| {});

        let mut cx = EditorLspTestContext::new(
            rust_language(indoc! {r#"
                ("{" @open "}" @close)
                ("(" @open ")" @close)
                ("\"" @open "\"" @close)
                "#}),
            Default::default(),
            cx,
        )
        .await;

        // after a closing quote the enclosing parenthesis wins over the quote pair
        cx.set_state(indoc! {r#"
            pub fn test("a"ˇ) {
                let x = "b";
            }
        "#});
        cx.run_until_parked();
        cx.assert_editor_text_highlights(
            HighlightKey::MatchingBracket,
            indoc! {r#"
            pub fn test«(»"a"«)» {
                let x = "b";
            }
        "#},
        );

        cx.set_state(indoc! {r#"
            pub fn test(ˇ"a") {
                let x = "b";
            }
        "#});
        cx.run_until_parked();
        cx.assert_editor_text_highlights(
            HighlightKey::MatchingBracket,
            indoc! {r#"
            pub fn test«(»"a"«)» {
                let x = "b";
            }
        "#},
        );

        // quotes alone are never highlighted as braces
        cx.set_state(indoc! {r#"
            pub fn test("a") {
                let x = "b"ˇ;
            }
        "#});
        cx.run_until_parked();
        cx.assert_editor_text_highlights(
            HighlightKey::MatchingBracket,
            indoc! {r#"
            pub fn test("a") {
                let x = "b";
            }
        "#},
        );

        cx.set_state(indoc! {r#"
            pub fn test("a") {
                let x = ˇb"b";
            }
        "#});
        cx.run_until_parked();
        cx.assert_editor_text_highlights(
            HighlightKey::MatchingBracket,
            indoc! {r#"
            pub fn test("a") {
                let x = b"b";
            }
        "#},
        );
    }
}
