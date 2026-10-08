use std::path::Path;

use collections::HashSet;
use db::kvp::KeyValueStore;
use gpui::{App, AppContext as _, SharedString, Task};
use serde::{Deserialize, Serialize};
use util::ResultExt as _;

use super::tree::{self, TreeOptions};
use crate::branch_refs::{RefKind, RefTarget};

const STATE_KEY_PREFIX: &str = "branches_popup";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PopupToggles {
    pub show_actions_in_search: bool,
    pub group_by_directory: bool,
    pub show_recent: bool,
    pub show_tags: bool,
}

impl Default for PopupToggles {
    fn default() -> Self {
        Self {
            show_actions_in_search: true,
            group_by_directory: true,
            show_recent: true,
            show_tags: true,
        }
    }
}

impl PopupToggles {
    pub fn tree_options(&self) -> TreeOptions {
        TreeOptions {
            group_by_directory: self.group_by_directory,
            show_recent: self.show_recent,
            show_tags: self.show_tags,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PersistedRefKind {
    Local,
    Remote,
    Tag,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PersistedRef {
    kind: PersistedRefKind,
    name: String,
}

impl From<&RefTarget> for PersistedRef {
    fn from(target: &RefTarget) -> Self {
        let kind = match target.kind {
            RefKind::Local => PersistedRefKind::Local,
            RefKind::Remote => PersistedRefKind::Remote,
            RefKind::Tag => PersistedRefKind::Tag,
        };
        Self {
            kind,
            name: target.name.to_string(),
        }
    }
}

impl From<&PersistedRef> for RefTarget {
    fn from(persisted: &PersistedRef) -> Self {
        match persisted.kind {
            PersistedRefKind::Local => RefTarget::local(persisted.name.clone()),
            PersistedRefKind::Remote => RefTarget::remote(persisted.name.clone()),
            PersistedRefKind::Tag => RefTarget::tag(persisted.name.clone()),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PersistedPopupState {
    pub added_favorites: Vec<PersistedRef>,
    pub removed_favorites: Vec<PersistedRef>,
    pub expanded: Option<Vec<String>>,
    pub toggles: PopupToggles,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupState {
    pub toggles: PopupToggles,
    added_favorites: HashSet<RefTarget>,
    removed_favorites: HashSet<RefTarget>,
    expanded: HashSet<SharedString>,
}

impl Default for PopupState {
    fn default() -> Self {
        Self {
            toggles: PopupToggles::default(),
            added_favorites: HashSet::default(),
            removed_favorites: HashSet::default(),
            expanded: tree::default_expanded(),
        }
    }
}

impl PopupState {
    pub fn from_persisted(persisted: PersistedPopupState) -> Self {
        let expanded = match persisted.expanded {
            Some(ids) => ids.into_iter().map(SharedString::from).collect(),
            None => tree::default_expanded(),
        };
        Self {
            toggles: persisted.toggles,
            added_favorites: persisted
                .added_favorites
                .iter()
                .map(RefTarget::from)
                .collect(),
            removed_favorites: persisted
                .removed_favorites
                .iter()
                .map(RefTarget::from)
                .collect(),
            expanded,
        }
    }

    pub fn to_persisted(&self) -> PersistedPopupState {
        let mut added_favorites: Vec<PersistedRef> = self
            .added_favorites
            .iter()
            .map(PersistedRef::from)
            .collect();
        added_favorites.sort();
        let mut removed_favorites: Vec<PersistedRef> = self
            .removed_favorites
            .iter()
            .map(PersistedRef::from)
            .collect();
        removed_favorites.sort();
        let mut expanded: Vec<String> = self.expanded.iter().map(|id| id.to_string()).collect();
        expanded.sort();
        PersistedPopupState {
            added_favorites,
            removed_favorites,
            expanded: Some(expanded),
            toggles: self.toggles,
        }
    }

    pub fn expanded(&self) -> &HashSet<SharedString> {
        &self.expanded
    }

    pub fn is_expanded(&self, node_id: &SharedString) -> bool {
        self.expanded.contains(node_id)
    }

    pub fn set_expanded(&mut self, node_id: SharedString, expanded: bool) {
        if expanded {
            self.expanded.insert(node_id);
        } else {
            self.expanded.remove(&node_id);
        }
    }

    pub fn resolve_favorites(&self, defaults: &HashSet<RefTarget>) -> HashSet<RefTarget> {
        let mut favorites: HashSet<RefTarget> = defaults
            .iter()
            .filter(|target| !self.removed_favorites.contains(*target))
            .cloned()
            .collect();
        favorites.extend(self.added_favorites.iter().cloned());
        favorites
    }

    pub fn is_favorite(&self, target: &RefTarget, defaults: &HashSet<RefTarget>) -> bool {
        if self.added_favorites.contains(target) {
            return true;
        }
        defaults.contains(target) && !self.removed_favorites.contains(target)
    }

    pub fn toggle_favorite(&mut self, target: &RefTarget, defaults: &HashSet<RefTarget>) -> bool {
        let was_favorite = self.is_favorite(target, defaults);
        if was_favorite {
            self.added_favorites.remove(target);
            if defaults.contains(target) {
                self.removed_favorites.insert(target.clone());
            }
        } else {
            self.removed_favorites.remove(target);
            if !defaults.contains(target) {
                self.added_favorites.insert(target.clone());
            }
        }
        !was_favorite
    }
}

pub fn serialization_key(common_dir_abs_path: &Path) -> String {
    format!(
        "{STATE_KEY_PREFIX}-{}",
        common_dir_abs_path.to_string_lossy()
    )
}

pub fn deserialize(raw: &str) -> Option<PersistedPopupState> {
    serde_json::from_str(raw).log_err()
}

pub fn serialize(state: &PersistedPopupState) -> Option<String> {
    serde_json::to_string(state).log_err()
}

pub fn load(key: String, cx: &App) -> Task<Option<PersistedPopupState>> {
    let store = KeyValueStore::global(cx);
    cx.background_spawn(async move {
        let raw = store.read_kvp(&key).log_err().flatten()?;
        deserialize(&raw)
    })
}

pub fn save(key: String, state: PersistedPopupState, cx: &App) -> Task<()> {
    let store = KeyValueStore::global(cx);
    cx.background_spawn(async move {
        if let Some(raw) = serialize(&state) {
            store.write_kvp(key, raw).await.log_err();
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree::{SectionKind, node_id_for_folder, node_id_for_section};

    fn defaults() -> HashSet<RefTarget> {
        [RefTarget::local("main"), RefTarget::remote("origin/main")]
            .into_iter()
            .collect()
    }

    #[test]
    fn default_state_expands_only_the_recent_section_and_enables_every_toggle() {
        let state = PopupState::default();
        assert_eq!(state.toggles, PopupToggles::default());
        assert!(state.toggles.show_actions_in_search);
        assert!(state.toggles.group_by_directory);
        assert!(state.toggles.show_recent);
        assert!(state.toggles.show_tags);
        assert_eq!(state.expanded().len(), 1);
        assert!(state.is_expanded(&node_id_for_section(SectionKind::Recent)));
    }

    #[test]
    fn default_favorites_are_kept_when_the_user_changed_nothing() {
        let state = PopupState::default();
        assert_eq!(state.resolve_favorites(&defaults()), defaults());
    }

    #[test]
    fn removing_a_default_favorite_survives_when_defaults_are_recomputed() {
        let mut state = PopupState::default();
        let main = RefTarget::local("main");
        assert!(!state.toggle_favorite(&main, &defaults()));
        let favorites = state.resolve_favorites(&defaults());
        assert!(!favorites.contains(&main));
        assert!(favorites.contains(&RefTarget::remote("origin/main")));
        assert!(!state.is_favorite(&main, &defaults()));
    }

    #[test]
    fn re_adding_a_removed_default_favorite_clears_the_removal_without_adding_it_twice() {
        let mut state = PopupState::default();
        let main = RefTarget::local("main");
        state.toggle_favorite(&main, &defaults());
        assert!(state.toggle_favorite(&main, &defaults()));
        let persisted = state.to_persisted();
        assert!(persisted.added_favorites.is_empty());
        assert!(persisted.removed_favorites.is_empty());
        assert!(state.resolve_favorites(&defaults()).contains(&main));
    }

    #[test]
    fn adding_and_removing_a_custom_favorite_round_trips() {
        let mut state = PopupState::default();
        let feature = RefTarget::local("feature/login");
        assert!(state.toggle_favorite(&feature, &defaults()));
        assert!(state.resolve_favorites(&defaults()).contains(&feature));
        assert!(!state.toggle_favorite(&feature, &defaults()));
        assert!(!state.resolve_favorites(&defaults()).contains(&feature));
        assert!(state.to_persisted().added_favorites.is_empty());
    }

    #[test]
    fn a_default_favorite_that_disappears_from_the_defaults_is_no_longer_favorite() {
        let state = PopupState::default();
        let without_main: HashSet<RefTarget> =
            [RefTarget::remote("origin/main")].into_iter().collect();
        assert!(
            !state
                .resolve_favorites(&without_main)
                .contains(&RefTarget::local("main"))
        );
    }

    #[test]
    fn favorites_of_different_kinds_with_the_same_name_stay_distinct() {
        let mut state = PopupState::default();
        assert!(state.toggle_favorite(&RefTarget::tag("release"), &defaults()));
        let favorites = state.resolve_favorites(&defaults());
        assert!(favorites.contains(&RefTarget::tag("release")));
        assert!(!favorites.contains(&RefTarget::local("release")));
    }

    #[test]
    fn persisted_state_round_trips_through_json() {
        let mut state = PopupState::default();
        state.toggle_favorite(&RefTarget::local("main"), &defaults());
        state.toggle_favorite(&RefTarget::remote("origin/feature/x"), &defaults());
        state.toggle_favorite(&RefTarget::tag("v1.0"), &defaults());
        state.set_expanded(node_id_for_section(SectionKind::Remote), true);
        state.set_expanded(node_id_for_folder(SectionKind::Remote, "origin"), true);
        state.toggles.group_by_directory = false;
        state.toggles.show_tags = false;

        let raw = serialize(&state.to_persisted()).expect("state serializes");
        let restored = PopupState::from_persisted(deserialize(&raw).expect("state deserializes"));

        assert_eq!(restored, state);
    }

    #[test]
    fn serialization_is_deterministic_regardless_of_insertion_order() {
        let mut first = PopupState::default();
        first.toggle_favorite(&RefTarget::local("b"), &defaults());
        first.toggle_favorite(&RefTarget::local("a"), &defaults());
        let mut second = PopupState::default();
        second.toggle_favorite(&RefTarget::local("a"), &defaults());
        second.toggle_favorite(&RefTarget::local("b"), &defaults());
        assert_eq!(
            serialize(&first.to_persisted()),
            serialize(&second.to_persisted())
        );
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let persisted = deserialize("{}").expect("empty object deserializes");
        let state = PopupState::from_persisted(persisted);
        assert_eq!(state, PopupState::default());
    }

    #[test]
    fn a_partial_toggle_object_keeps_the_other_toggles_enabled() {
        let persisted = deserialize(r#"{"toggles":{"show_tags":false}}"#).expect("deserializes");
        assert!(!persisted.toggles.show_tags);
        assert!(persisted.toggles.show_recent);
        assert!(persisted.toggles.group_by_directory);
        assert!(persisted.toggles.show_actions_in_search);
    }

    #[test]
    fn corrupt_json_is_rejected_instead_of_panicking() {
        assert_eq!(deserialize("not json"), None);
        assert_eq!(deserialize(r#"{"toggles": 3}"#), None);
    }

    #[test]
    fn an_explicitly_empty_expanded_list_stays_collapsed() {
        let persisted = deserialize(r#"{"expanded":[]}"#).expect("deserializes");
        let state = PopupState::from_persisted(persisted);
        assert!(state.expanded().is_empty());
    }

    #[test]
    fn serialization_key_is_scoped_by_the_repository_common_dir() {
        let first = serialization_key(Path::new("/work/one/.git"));
        let second = serialization_key(Path::new("/work/two/.git"));
        assert_ne!(first, second);
        assert!(first.contains("/work/one/.git"));
    }
}
