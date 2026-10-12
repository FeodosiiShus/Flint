use db::kvp::KeyValueStore;
use gpui::{App, Task};
use serde::{Deserialize, Serialize};
use util::ResultExt as _;
use workspace::Workspace;

const TABS_NAMESPACE: &str = "git_tool_window_tabs";
const CONSOLE_SOFT_WRAPS_KEY: &str = "git_tool_window_console_soft_wraps";
const UUID_VERSION_MASK: u8 = 0x0f;
const UUID_VERSION_FOUR: u8 = 0x40;
const UUID_VARIANT_MASK: u8 = 0x3f;
const UUID_VARIANT_IETF: u8 = 0x80;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PersistedTabs {
    pub log_tabs: Vec<PersistedLogTab>,
    pub selected_tab: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PersistedLogTab {
    pub id: String,
    pub has_filters: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RestorePlan {
    pub reopened: Vec<String>,
    pub dropped: Vec<String>,
}

pub(crate) fn restore_plan(saved_tabs: &[PersistedLogTab]) -> RestorePlan {
    let mut plan = RestorePlan::default();
    let mut one_tab_preserved = false;
    for tab in saved_tabs {
        if tab.has_filters {
            plan.reopened.push(tab.id.clone());
        } else if !one_tab_preserved {
            one_tab_preserved = true;
            plan.reopened.push(tab.id.clone());
        } else {
            plan.dropped.push(tab.id.clone());
        }
    }
    plan
}

pub(crate) fn workspace_key(workspace: &Workspace) -> Option<String> {
    workspace
        .database_id()
        .map(|id| i64::from(id).to_string())
        .or(workspace.session_id())
}

pub(crate) fn uuid_from_random(random: u128) -> String {
    let mut bytes = random.to_be_bytes();
    bytes[6] = (bytes[6] & UUID_VERSION_MASK) | UUID_VERSION_FOUR;
    bytes[8] = (bytes[8] & UUID_VARIANT_MASK) | UUID_VARIANT_IETF;
    let hex = format!("{:032x}", u128::from_be_bytes(bytes));
    [0..8, 8..12, 12..16, 16..20, 20..32]
        .into_iter()
        .filter_map(|range| hex.get(range))
        .collect::<Vec<_>>()
        .join("-")
}

pub(crate) fn new_log_tab_id(is_taken: impl Fn(&str) -> bool) -> String {
    loop {
        let id = uuid_from_random(rand::random::<u128>());
        if !is_taken(&id) {
            return id;
        }
    }
}

pub(crate) fn load_tabs(key: String, cx: &App) -> Task<Option<PersistedTabs>> {
    let store = KeyValueStore::global(cx);
    cx.background_spawn(async move {
        let raw = store
            .scoped(TABS_NAMESPACE)
            .read(&key)
            .log_err()
            .flatten()?;
        serde_json::from_str::<PersistedTabs>(&raw).log_err()
    })
}

pub(crate) fn save_tabs(key: String, tabs: PersistedTabs, cx: &App) -> Task<()> {
    let store = KeyValueStore::global(cx);
    cx.background_spawn(async move {
        let Some(raw) = serde_json::to_string(&tabs).log_err() else {
            return;
        };
        store.scoped(TABS_NAMESPACE).write(key, raw).await.log_err();
    })
}

pub(crate) fn console_soft_wraps(cx: &App) -> bool {
    KeyValueStore::global(cx)
        .read_kvp(CONSOLE_SOFT_WRAPS_KEY)
        .log_err()
        .flatten()
        .is_some_and(|value| value == "true")
}

pub(crate) fn save_console_soft_wraps(soft_wraps: bool, cx: &App) -> Task<()> {
    let store = KeyValueStore::global(cx);
    cx.background_spawn(async move {
        store
            .write_kvp(CONSOLE_SOFT_WRAPS_KEY.to_string(), soft_wraps.to_string())
            .await
            .log_err();
    })
}
