use settings::SemanticTokenRules;

pub(crate) fn semantic_token_rules() -> SemanticTokenRules {
    let content = grammars::get_file("go/semantic_token_rules.json")
        .expect("missing go/semantic_token_rules.json");
    let json = std::str::from_utf8(&content.data).expect("invalid utf-8 in semantic_token_rules");
    settings::parse_json_with_comments::<SemanticTokenRules>(json)
        .expect("failed to parse go semantic_token_rules.json")
}

#[cfg(test)]
mod tests {
    use crate::language;
    use gpui::{AppContext, TestAppContext};
    use unindent::Unindent as _;

    #[gpui::test]
    fn test_go_outline_includes_methods_with_receiver_forms(cx: &mut TestAppContext) {
        let language = language("go", tree_sitter_go::LANGUAGE.into());

        let source = r#"
        package main

        type v2 struct{}

        func (v2) BrokenMethod() {
            println("start")
        }

        func (_ v2) UnderscoreReceiverMethod() {
            println("start")
        }

        func (v v2) NamedReceiverMethod() {
            println("start")
        }

        func (v *v2) PointerReceiverMethod() {
            println("start")
        }

        func WorkingFunction() {
            println("start")
        }
        "#
        .unindent();

        let buffer =
            cx.new(|cx| crate::Buffer::local(source.clone(), cx).with_language(language, cx));
        let snapshot = buffer.read_with(cx, |buffer, _| buffer.snapshot());
        let outline = snapshot.outline(None);

        assert_eq!(
            outline
                .items
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            &[
                "type v2",
                "func (v2) BrokenMethod",
                "func (_ v2) UnderscoreReceiverMethod",
                "func (v v2) NamedReceiverMethod",
                "func (v *v2) PointerReceiverMethod",
                "func WorkingFunction",
            ]
        );

        for (method_name, expected_symbol) in [
            ("BrokenMethod", "func (v2) BrokenMethod"),
            (
                "UnderscoreReceiverMethod",
                "func (_ v2) UnderscoreReceiverMethod",
            ),
            ("NamedReceiverMethod", "func (v v2) NamedReceiverMethod"),
            (
                "PointerReceiverMethod",
                "func (v *v2) PointerReceiverMethod",
            ),
            ("WorkingFunction", "func WorkingFunction"),
        ] {
            let method_position = source
                .find(&format!("{method_name}()"))
                .expect("method should exist in source");
            let body_position = source[method_position..]
                .find("println")
                .map(|body_offset| method_position + body_offset)
                .expect("method should contain a body");
            let symbols = snapshot.symbols_containing(body_position, None);
            assert_eq!(
                symbols
                    .last()
                    .map(|item| item.text.as_str())
                    .expect("method should have an outline symbol"),
                expected_symbol
            );
        }
    }
}
