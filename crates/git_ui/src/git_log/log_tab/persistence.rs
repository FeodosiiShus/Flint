use std::path::Path;

use db::kvp::KeyValueStore;
use gpui::{App, AppContext as _, Task};
use util::ResultExt as _;
use workspace::SERIALIZATION_THROTTLE_TIME;

use crate::git_log::properties::{LogAppProperties, LogTabProperties};

const APP_PROPERTIES_KEY: &str = "git_log_app_properties";
const TAB_PROPERTIES_NAMESPACE_PREFIX: &str = "git_log_tab_properties";

fn tab_properties_namespace(tab_id: &str) -> String {
    format!("{TAB_PROPERTIES_NAMESPACE_PREFIX}-{tab_id}")
}

fn repository_key(repository_path: &Path) -> String {
    repository_path.to_string_lossy().into_owned()
}

pub(super) fn load_app_properties(cx: &App) -> LogAppProperties {
    KeyValueStore::global(cx)
        .read_kvp(APP_PROPERTIES_KEY)
        .log_err()
        .flatten()
        .and_then(|raw| deserialize_app_properties(&raw))
        .unwrap_or_default()
}

fn deserialize_app_properties(raw: &str) -> Option<LogAppProperties> {
    let properties = serde_json::from_str::<LogAppProperties>(raw).log_err()?;
    Some(LogAppProperties {
        branches_splitter_proportion: properties.branches_splitter_proportion.clamp(0., 1.),
        changes_splitter_proportion: properties.changes_splitter_proportion.clamp(0., 1.),
        details_splitter_proportion: properties.details_splitter_proportion.clamp(0., 1.),
        ..properties
    })
}

pub(super) fn load_tab_properties(
    tab_id: &str,
    repository_path: &Path,
    is_main_tab: bool,
    cx: &App,
) -> LogTabProperties {
    let namespace = tab_properties_namespace(tab_id);
    KeyValueStore::global(cx)
        .scoped(&namespace)
        .read(&repository_key(repository_path))
        .log_err()
        .flatten()
        .and_then(|raw| serde_json::from_str::<LogTabProperties>(&raw).log_err())
        .unwrap_or_else(|| LogTabProperties::new(is_main_tab))
}

pub(super) fn save_app_properties(properties: &LogAppProperties, cx: &App) -> Task<()> {
    let Some(raw) = serde_json::to_string(properties).log_err() else {
        return Task::ready(());
    };
    let store = KeyValueStore::global(cx);
    let executor = cx.background_executor().clone();
    cx.background_spawn(async move {
        executor.timer(SERIALIZATION_THROTTLE_TIME).await;
        store
            .write_kvp(APP_PROPERTIES_KEY.to_string(), raw)
            .await
            .log_err();
    })
}

pub(super) fn save_tab_properties(
    tab_id: &str,
    repository_path: &Path,
    properties: &LogTabProperties,
    cx: &App,
) -> Task<()> {
    let Some(raw) = serde_json::to_string(properties).log_err() else {
        return Task::ready(());
    };
    let namespace = tab_properties_namespace(tab_id);
    let key = repository_key(repository_path);
    let store = KeyValueStore::global(cx);
    let executor = cx.background_executor().clone();
    cx.background_spawn(async move {
        executor.timer(SERIALIZATION_THROTTLE_TIME).await;
        store.scoped(&namespace).write(key, raw).await.log_err();
    })
}

pub(super) fn delete_tab_properties(tab_id: &str, cx: &App) {
    let namespace = tab_properties_namespace(tab_id);
    let store = KeyValueStore::global(cx);
    db::write_and_log(cx, move || async move {
        store.scoped(&namespace).delete_all().await
    });
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;
    use crate::git_log::properties::{
        BranchesSingleClickAction, GraphOptions, LogColumn, LogFilterState, USER_FILTER_ME,
    };

    fn customized_tab_properties() -> LogTabProperties {
        LogTabProperties {
            show_details: false,
            show_long_edges: true,
            graph_options: GraphOptions::FirstParent,
            filters: LogFilterState {
                branches: vec!["HEAD".to_string(), "origin/main".to_string()],
                users: vec![USER_FILTER_ME.to_string()],
                date_after: Some(1_700_000_000),
                date_before: None,
                paths: vec!["crates/".to_string()],
                text: "fix".to_string(),
            },
            text_filter_regex: true,
            column_widths: vec![(LogColumn::Author, 120.), (LogColumn::Date, 96.5)],
            show_branches: false,
            branches_single_click: BranchesSingleClickAction::NavigateLog,
            ..LogTabProperties::new(true)
        }
    }

    #[gpui::test]
    async fn properties_survive_a_save_and_load_round_trip(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(db::AppDatabase::test_new()));
        let repository = Path::new("/projects/flint");
        let tab_properties = customized_tab_properties();
        let app_properties = LogAppProperties {
            compact_references: false,
            prefer_commit_date: true,
            branches_splitter_proportion: 0.25,
            details_splitter_proportion: 0.55,
            ..LogAppProperties::default()
        };

        let tab_save = cx.update(|cx| save_tab_properties("MAIN", repository, &tab_properties, cx));
        let app_save = cx.update(|cx| save_app_properties(&app_properties, cx));
        cx.executor().advance_clock(SERIALIZATION_THROTTLE_TIME);
        tab_save.await;
        app_save.await;

        assert_eq!(
            cx.update(|cx| load_tab_properties("MAIN", repository, true, cx)),
            tab_properties
        );
        assert_eq!(cx.update(|cx| load_app_properties(cx)), app_properties);
        assert_eq!(
            cx.update(|cx| load_tab_properties("MAIN", Path::new("/projects/other"), true, cx)),
            LogTabProperties::new(true),
            "tab properties are stored per repository"
        );
        assert_eq!(
            cx.update(|cx| load_tab_properties("1", repository, false, cx)),
            LogTabProperties::new(false),
            "tab properties are stored per tab"
        );

        cx.update(|cx| delete_tab_properties("MAIN", cx));
        cx.run_until_parked();
        assert_eq!(
            cx.update(|cx| load_tab_properties("MAIN", repository, true, cx)),
            LogTabProperties::new(true),
            "a forgotten tab starts from the defaults again"
        );
        assert_eq!(cx.update(|cx| load_app_properties(cx)), app_properties);
    }

    #[test]
    fn tab_properties_round_trip_through_json() {
        let properties = customized_tab_properties();
        let raw = serde_json::to_string(&properties).expect("tab properties serialize");
        assert_eq!(
            serde_json::from_str::<LogTabProperties>(&raw).expect("tab properties deserialize"),
            properties
        );
    }

    #[test]
    fn stored_app_properties_fill_missing_fields_and_clamp_proportions() {
        let properties = deserialize_app_properties(
            r#"{"compact_references":false,"branches_splitter_proportion":1.5,"changes_splitter_proportion":-0.25}"#,
        )
        .expect("partial app properties deserialize");
        let defaults = LogAppProperties::default();
        assert!(!properties.compact_references);
        assert_eq!(properties.branches_splitter_proportion, 1.);
        assert_eq!(properties.changes_splitter_proportion, 0.);
        assert_eq!(
            properties.details_splitter_proportion,
            defaults.details_splitter_proportion
        );
        assert_eq!(properties.column_order, defaults.column_order);
        assert_eq!(deserialize_app_properties("not json"), None);
    }
}
