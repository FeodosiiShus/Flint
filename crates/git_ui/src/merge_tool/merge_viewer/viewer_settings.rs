use db::kvp::KeyValueStore;
use gpui::App;
use merge_diff::ComparisonPolicy;
use util::ResultExt as _;

use super::Appearance;

const NAMESPACE: &str = "merge_tool_viewer_settings";
const IGNORE_POLICY_KEY: &str = "ignore_policy";
const HIGHLIGHT_BY_WORD_KEY: &str = "highlight_by_word";
const EXPAND_BY_DEFAULT_KEY: &str = "expand_by_default";
const SOFT_WRAP_KEY: &str = "soft_wrap";
const INDENT_GUIDES_KEY: &str = "indent_guides";
const WHITESPACES_KEY: &str = "whitespaces";
const LINE_NUMBERS_KEY: &str = "line_numbers";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ViewerSettings {
    pub ignore_policy: ComparisonPolicy,
    pub highlight_by_word: bool,
    pub expand_by_default: bool,
    pub appearance: Appearance,
}

impl Default for ViewerSettings {
    fn default() -> Self {
        Self {
            ignore_policy: ComparisonPolicy::Default,
            highlight_by_word: true,
            expand_by_default: true,
            appearance: Appearance::default(),
        }
    }
}

fn ignore_policy_value(policy: ComparisonPolicy) -> &'static str {
    match policy {
        ComparisonPolicy::Default => "default",
        ComparisonPolicy::TrimWhitespaces => "trim_whitespaces",
        ComparisonPolicy::IgnoreWhitespaces => "ignore_whitespaces",
    }
}

fn parse_ignore_policy(value: Option<&str>) -> ComparisonPolicy {
    match value.map(str::trim) {
        Some("trim_whitespaces") => ComparisonPolicy::TrimWhitespaces,
        Some("ignore_whitespaces") => ComparisonPolicy::IgnoreWhitespaces,
        _ => ComparisonPolicy::Default,
    }
}

fn parse_flag(value: Option<&str>, default: bool) -> bool {
    match value.map(str::trim) {
        Some("true") => true,
        Some("false") => false,
        _ => default,
    }
}

fn read_value(key: &str, cx: &App) -> Option<String> {
    let key_value_store = KeyValueStore::global(cx);
    let store = key_value_store.scoped(NAMESPACE);
    store.read(key).log_err().flatten()
}

fn write_value(key: &'static str, value: String, cx: &App) {
    let key_value_store = KeyValueStore::global(cx);
    db::write_and_log(cx, move || async move {
        key_value_store
            .scoped(NAMESPACE)
            .write(key.to_string(), value)
            .await
    });
}

fn read_flag(key: &str, default: bool, cx: &App) -> bool {
    parse_flag(read_value(key, cx).as_deref(), default)
}

pub(crate) fn stored_ignore_policy(cx: &App) -> ComparisonPolicy {
    parse_ignore_policy(read_value(IGNORE_POLICY_KEY, cx).as_deref())
}

pub(crate) fn load(cx: &App) -> ViewerSettings {
    let defaults = ViewerSettings::default();
    ViewerSettings {
        ignore_policy: stored_ignore_policy(cx),
        highlight_by_word: read_flag(HIGHLIGHT_BY_WORD_KEY, defaults.highlight_by_word, cx),
        expand_by_default: read_flag(EXPAND_BY_DEFAULT_KEY, defaults.expand_by_default, cx),
        appearance: Appearance {
            soft_wrap: read_flag(SOFT_WRAP_KEY, defaults.appearance.soft_wrap, cx),
            indent_guides: read_flag(INDENT_GUIDES_KEY, defaults.appearance.indent_guides, cx),
            show_whitespaces: read_flag(WHITESPACES_KEY, defaults.appearance.show_whitespaces, cx),
            line_numbers: read_flag(LINE_NUMBERS_KEY, defaults.appearance.line_numbers, cx),
        },
    }
}

pub(crate) fn store_ignore_policy(policy: ComparisonPolicy, cx: &App) {
    write_value(
        IGNORE_POLICY_KEY,
        ignore_policy_value(policy).to_string(),
        cx,
    );
}

pub(crate) fn store_highlight_by_word(by_word: bool, cx: &App) {
    write_value(HIGHLIGHT_BY_WORD_KEY, by_word.to_string(), cx);
}

pub(crate) fn store_expand_by_default(expand: bool, cx: &App) {
    write_value(EXPAND_BY_DEFAULT_KEY, expand.to_string(), cx);
}

pub(crate) fn store_appearance(appearance: Appearance, cx: &App) {
    write_value(SOFT_WRAP_KEY, appearance.soft_wrap.to_string(), cx);
    write_value(INDENT_GUIDES_KEY, appearance.indent_guides.to_string(), cx);
    write_value(WHITESPACES_KEY, appearance.show_whitespaces.to_string(), cx);
    write_value(LINE_NUMBERS_KEY, appearance.line_numbers.to_string(), cx);
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;

    #[test]
    fn merge_tool_viewer_settings_defaults_match_the_merge_place_defaults() {
        let defaults = ViewerSettings::default();
        assert_eq!(defaults.ignore_policy, ComparisonPolicy::Default);
        assert!(defaults.highlight_by_word);
        assert!(defaults.expand_by_default);
        assert!(!defaults.appearance.soft_wrap);
        assert!(!defaults.appearance.indent_guides);
        assert!(!defaults.appearance.show_whitespaces);
        assert!(defaults.appearance.line_numbers);
    }

    #[test]
    fn merge_tool_viewer_settings_ignore_policy_survives_a_round_trip() {
        for policy in [
            ComparisonPolicy::Default,
            ComparisonPolicy::TrimWhitespaces,
            ComparisonPolicy::IgnoreWhitespaces,
        ] {
            assert_eq!(
                parse_ignore_policy(Some(ignore_policy_value(policy))),
                policy
            );
        }
    }

    #[test]
    fn merge_tool_viewer_settings_unknown_stored_values_fall_back_to_the_defaults() {
        assert_eq!(parse_ignore_policy(None), ComparisonPolicy::Default);
        assert_eq!(
            parse_ignore_policy(Some("garbage")),
            ComparisonPolicy::Default
        );
        assert!(parse_flag(None, true));
        assert!(!parse_flag(None, false));
        assert!(parse_flag(Some("garbage"), true));
        assert!(!parse_flag(Some(" false\n"), true));
        assert!(parse_flag(Some(" true\n"), false));
    }

    #[gpui::test]
    fn merge_tool_viewer_settings_are_persisted_across_loads(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(db::AppDatabase::test_new()));
        let changed = ViewerSettings {
            ignore_policy: ComparisonPolicy::IgnoreWhitespaces,
            highlight_by_word: false,
            expand_by_default: false,
            appearance: Appearance {
                soft_wrap: true,
                indent_guides: true,
                show_whitespaces: true,
                line_numbers: false,
            },
        };

        cx.update(|cx| {
            assert_eq!(load(cx), ViewerSettings::default());
            store_ignore_policy(changed.ignore_policy, cx);
            store_highlight_by_word(changed.highlight_by_word, cx);
            store_expand_by_default(changed.expand_by_default, cx);
            store_appearance(changed.appearance, cx);
        });
        cx.run_until_parked();

        cx.update(|cx| {
            assert_eq!(load(cx), changed);
            assert_eq!(
                stored_ignore_policy(cx),
                ComparisonPolicy::IgnoreWhitespaces
            );
        });
    }
}
