use db::kvp::KeyValueStore;
use gpui::App;
use util::ResultExt as _;

const GROUP_BY_DIRECTORY_PREFIX: &str = "merge_tool_conflicts_group_by_directory";

pub(super) fn grouping_key(project_root: &str) -> String {
    format!("{GROUP_BY_DIRECTORY_PREFIX}:{project_root}")
}

fn parse_stored(value: Option<&str>) -> bool {
    value.map(str::trim) == Some("true")
}

pub(super) fn read_group_by_directory(key: &str, cx: &App) -> bool {
    let stored = KeyValueStore::global(cx).read_kvp(key).log_err().flatten();
    parse_stored(stored.as_deref())
}

pub(super) fn write_group_by_directory(key: &str, value: bool, cx: &App) {
    let store = KeyValueStore::global(cx);
    let key = key.to_string();
    db::write_and_log(cx, move || async move {
        store.write_kvp(key, value.to_string()).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[test]
    fn merge_tool_grouping_key_is_scoped_per_project_root() {
        assert_ne!(grouping_key("/work/a"), grouping_key("/work/b"));
        assert!(grouping_key("/work/a").ends_with("/work/a"));
    }

    #[test]
    fn merge_tool_stored_grouping_defaults_to_false() {
        assert!(!parse_stored(None));
        assert!(!parse_stored(Some("false")));
        assert!(!parse_stored(Some("garbage")));
        assert!(parse_stored(Some("true")));
        assert!(parse_stored(Some(" true\n")));
    }

    #[gpui::test]
    fn merge_tool_grouping_choice_round_trips_per_project_root(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(db::AppDatabase::test_new()));
        let first_key = grouping_key("/work/a");
        let second_key = grouping_key("/work/b");

        cx.update(|cx| {
            assert!(!read_group_by_directory(&first_key, cx));
            assert!(!read_group_by_directory(&second_key, cx));
            write_group_by_directory(&first_key, true, cx);
        });
        cx.run_until_parked();

        cx.update(|cx| {
            assert!(read_group_by_directory(&first_key, cx));
            assert!(!read_group_by_directory(&second_key, cx));
            write_group_by_directory(&first_key, false, cx);
        });
        cx.run_until_parked();

        cx.update(|cx| assert!(!read_group_by_directory(&first_key, cx)));
    }
}
