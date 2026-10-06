use std::sync::Arc;

use collections::HashMap;
use fs::Fs;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use language::{LanguageRegistry, ServerActivationRule};
use lsp::LanguageServerName;
use parking_lot::Mutex;
use settings::WorktreeId;
use worktree::{PathChange, Snapshot, UpdatedEntriesSet};

const PACKAGE_MANIFEST_NAME: &str = "package.json";
const MAX_PACKAGE_MANIFESTS: usize = 64;

pub(crate) struct ProbeRequest {
    pub(crate) worktree_id: WorktreeId,
    pub(crate) server_name: LanguageServerName,
    pub(crate) rule: Arc<ServerActivationRule>,
    pub(crate) snapshot: Snapshot,
}

#[derive(Default)]
struct GateEntry {
    active: Option<bool>,
    probing: bool,
    stale: bool,
}

impl GateEntry {
    fn send_probe(
        &mut self,
        probe_requests: &UnboundedSender<ProbeRequest>,
        worktree_id: WorktreeId,
        server_name: &LanguageServerName,
        rule: Arc<ServerActivationRule>,
        snapshot: Snapshot,
    ) {
        let request = ProbeRequest {
            worktree_id,
            server_name: server_name.clone(),
            rule,
            snapshot,
        };
        if probe_requests.unbounded_send(request).is_ok() {
            self.probing = true;
            self.stale = false;
        }
    }
}

pub(crate) struct ServerActivationGate {
    languages: Arc<LanguageRegistry>,
    entries: Mutex<HashMap<(WorktreeId, LanguageServerName), GateEntry>>,
    probe_requests: UnboundedSender<ProbeRequest>,
}

impl ServerActivationGate {
    pub(crate) fn new(
        languages: Arc<LanguageRegistry>,
    ) -> (Arc<Self>, UnboundedReceiver<ProbeRequest>) {
        let (probe_requests, receiver) = unbounded();
        let gate = Arc::new(Self {
            languages,
            entries: Mutex::default(),
            probe_requests,
        });
        (gate, receiver)
    }

    pub(crate) fn is_active(
        &self,
        worktree_id: WorktreeId,
        server_name: &LanguageServerName,
        snapshot: impl FnOnce() -> Option<Snapshot>,
    ) -> bool {
        let Some(rule) = self.languages.server_activation_rule(server_name) else {
            return true;
        };

        let mut entries = self.entries.lock();
        let entry = entries
            .entry((worktree_id, server_name.clone()))
            .or_default();

        if !entry.probing
            && (entry.active.is_none() || entry.stale)
            && let Some(snapshot) = snapshot()
        {
            entry.send_probe(
                &self.probe_requests,
                worktree_id,
                server_name,
                rule,
                snapshot,
            );
        }

        entry.active.unwrap_or(false)
    }

    pub(crate) fn record(
        &self,
        worktree_id: WorktreeId,
        server_name: &LanguageServerName,
        active: Option<bool>,
    ) -> bool {
        let mut entries = self.entries.lock();
        let Some(entry) = entries.get_mut(&(worktree_id, server_name.clone())) else {
            return false;
        };
        let previous = entry.active.unwrap_or(false);
        let resolved = active.or(entry.active).unwrap_or(false);
        entry.active = Some(resolved);
        entry.probing = false;
        previous != resolved || entry.stale
    }

    pub(crate) fn forget_worktree(&self, worktree_id: WorktreeId) {
        self.entries
            .lock()
            .retain(|(entry_worktree_id, _), _| *entry_worktree_id != worktree_id);
    }

    pub(crate) fn reprobe_affected(
        &self,
        worktree_id: WorktreeId,
        changes: &UpdatedEntriesSet,
        snapshot: &Snapshot,
    ) {
        let rules = self.languages.server_activation_rules();
        if rules.is_empty() {
            return;
        }

        let mut entries = self.entries.lock();
        for (server_name, rule) in rules {
            let Some(entry) = entries.get_mut(&(worktree_id, server_name.clone())) else {
                continue;
            };
            if !changes_affect_rule(&rule, changes, snapshot) {
                continue;
            }
            if entry.probing {
                entry.stale = true;
            } else {
                entry.send_probe(
                    &self.probe_requests,
                    worktree_id,
                    &server_name,
                    rule,
                    snapshot.clone(),
                );
            }
        }
    }
}

fn changes_affect_rule(
    rule: &ServerActivationRule,
    changes: &UpdatedEntriesSet,
    snapshot: &Snapshot,
) -> bool {
    changes.iter().any(|(path, _, change)| {
        rule.matches_workspace_file(path.as_unix_str())
            || (rule.has_package_dependencies()
                && path.file_name() == Some(PACKAGE_MANIFEST_NAME)
                && (*change == PathChange::Removed
                    || snapshot
                        .entry_for_path(path)
                        .is_some_and(|entry| !entry.is_ignored)))
    })
}

pub(crate) async fn probe(
    rule: &ServerActivationRule,
    snapshot: &Snapshot,
    fs: &dyn Fs,
) -> Option<bool> {
    if rule.has_workspace_files()
        && snapshot
            .entries(true, 0)
            .any(|entry| rule.matches_workspace_file(entry.path.as_unix_str()))
    {
        return Some(true);
    }

    if !rule.has_package_dependencies() {
        return Some(false);
    }

    let mut manifests = snapshot
        .files(false, 0)
        .filter(|entry| entry.path.file_name() == Some(PACKAGE_MANIFEST_NAME))
        .collect::<Vec<_>>();
    manifests.sort_by_key(|entry| entry.path.components().count());
    let manifests = manifests
        .into_iter()
        .take(MAX_PACKAGE_MANIFESTS)
        .map(|entry| snapshot.absolutize(&entry.path))
        .collect::<Vec<_>>();

    let mut inconclusive = false;
    for manifest in manifests {
        let Ok(contents) = fs.load(&manifest).await else {
            inconclusive = true;
            continue;
        };
        match rule.manifest_declares_dependency(&contents) {
            Some(true) => return Some(true),
            Some(false) => {}
            None => inconclusive = true,
        }
    }

    if inconclusive { None } else { Some(false) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Project;
    use fs::FakeFs;
    use gpui::TestAppContext;
    use serde_json::json;
    use settings::SettingsStore;
    use util::path;

    fn tailwind_server() -> LanguageServerName {
        LanguageServerName::new_static("tailwindcss-language-server")
    }

    fn tailwind_rule() -> ServerActivationRule {
        ServerActivationRule::new(
            &["**/tailwind.config.{js,ts}"],
            &["tailwindcss", "@tailwindcss/*"],
        )
    }

    fn gate_with_tailwind_rule(cx: &TestAppContext) -> Arc<ServerActivationGate> {
        let languages = Arc::new(LanguageRegistry::test(cx.executor()));
        languages.register_server_activation(tailwind_server(), tailwind_rule());
        ServerActivationGate::new(languages).0
    }

    #[gpui::test]
    fn servers_without_a_rule_are_always_active(cx: &mut TestAppContext) {
        let gate = gate_with_tailwind_rule(cx);
        let worktree_id = WorktreeId::from_usize(1);

        assert!(gate.is_active(
            worktree_id,
            &LanguageServerName::new_static("vtsls"),
            || None
        ));
    }

    #[gpui::test]
    fn a_server_stays_inactive_until_its_probe_reports(cx: &mut TestAppContext) {
        let gate = gate_with_tailwind_rule(cx);
        let worktree_id = WorktreeId::from_usize(1);

        assert!(!gate.is_active(worktree_id, &tailwind_server(), || None));
        assert!(gate.record(worktree_id, &tailwind_server(), Some(true)));
        assert!(gate.is_active(worktree_id, &tailwind_server(), || None));
    }

    #[gpui::test]
    fn a_negative_probe_result_is_not_reported_as_a_change(cx: &mut TestAppContext) {
        let gate = gate_with_tailwind_rule(cx);
        let worktree_id = WorktreeId::from_usize(1);

        assert!(!gate.is_active(worktree_id, &tailwind_server(), || None));
        assert!(!gate.record(worktree_id, &tailwind_server(), Some(false)));
    }

    #[gpui::test]
    fn an_inconclusive_probe_keeps_the_previous_answer(cx: &mut TestAppContext) {
        let gate = gate_with_tailwind_rule(cx);
        let worktree_id = WorktreeId::from_usize(1);

        gate.is_active(worktree_id, &tailwind_server(), || None);
        gate.record(worktree_id, &tailwind_server(), Some(true));

        assert!(!gate.record(worktree_id, &tailwind_server(), None));
        assert!(gate.is_active(worktree_id, &tailwind_server(), || None));
    }

    #[gpui::test]
    fn forgetting_a_worktree_returns_it_to_the_unknown_state(cx: &mut TestAppContext) {
        let gate = gate_with_tailwind_rule(cx);
        let worktree_id = WorktreeId::from_usize(1);
        let other_worktree_id = WorktreeId::from_usize(2);

        gate.is_active(worktree_id, &tailwind_server(), || None);
        gate.is_active(other_worktree_id, &tailwind_server(), || None);
        gate.record(worktree_id, &tailwind_server(), Some(true));
        gate.record(other_worktree_id, &tailwind_server(), Some(true));
        gate.forget_worktree(worktree_id);

        assert!(!gate.is_active(worktree_id, &tailwind_server(), || None));
        assert!(gate.is_active(other_worktree_id, &tailwind_server(), || None));
    }

    #[test]
    fn rules_match_dependencies_by_name_and_by_prefix() {
        let rule = tailwind_rule();

        assert!(rule.is_dependency("tailwindcss"));
        assert!(!rule.is_dependency("tailwindcss-animate"));
        assert!(rule.is_dependency("@tailwindcss/vite"));
        assert!(!rule.is_dependency("@types/node"));
    }

    #[test]
    fn manifests_declare_dependencies_in_any_dependency_section() {
        let rule = tailwind_rule();

        assert_eq!(
            rule.manifest_declares_dependency(r#"{ "devDependencies": { "tailwindcss": "^4" } }"#),
            Some(true)
        );
        assert_eq!(
            rule.manifest_declares_dependency(
                r#"{ "dependencies": { "@tailwindcss/vite": "^4" } }"#
            ),
            Some(true)
        );
        assert_eq!(
            rule.manifest_declares_dependency(
                r#"{ "scripts": { "build": "tailwindcss -i in.css" }, "dependencies": { "react": "19" } }"#
            ),
            Some(false)
        );
        assert_eq!(rule.manifest_declares_dependency("not json"), None);
    }

    #[test]
    fn workspace_file_patterns_match_at_any_depth() {
        let rule = ServerActivationRule::new(&["**/tailwind.config.{js,ts}", "**/.obsidian"], &[]);

        assert!(rule.matches_workspace_file("tailwind.config.js"));
        assert!(rule.matches_workspace_file("apps/web/tailwind.config.ts"));
        assert!(rule.matches_workspace_file(".obsidian"));
        assert!(rule.matches_workspace_file("notes/.obsidian"));
        assert!(!rule.matches_workspace_file("src/tailwind.rs"));
        assert!(!rule.matches_workspace_file("tailwind.config.json"));
    }

    #[gpui::test]
    async fn probe_reads_nested_manifests_and_marker_directories(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
        });
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            path!("/root"),
            json!({
                "apps": {
                    "web": {
                        "package.json": r#"{ "devDependencies": { "tailwindcss": "^4" } }"#,
                    },
                },
                "notes": {
                    ".obsidian": {},
                    "index.md": "",
                },
                "plain": {
                    "package.json": r#"{ "dependencies": { "react": "19" } }"#,
                },
            }),
        )
        .await;
        let project = Project::test(
            fs.clone(),
            [
                path!("/root/apps").as_ref(),
                path!("/root/notes").as_ref(),
                path!("/root/plain").as_ref(),
            ],
            cx,
        )
        .await;
        let snapshots = project.read_with(cx, |project, cx| {
            project
                .worktrees(cx)
                .map(|worktree| {
                    let worktree = worktree.read(cx);
                    (
                        worktree.root_name().as_unix_str().to_string(),
                        worktree.snapshot(),
                    )
                })
                .collect::<Vec<_>>()
        });
        let snapshot_for = |name: &str| {
            &snapshots
                .iter()
                .find(|(root_name, _)| root_name.as_str() == name)
                .expect("worktree should exist")
                .1
        };
        let notes_rule = ServerActivationRule::new(&["**/.obsidian"], &[]);

        assert_eq!(
            probe(&tailwind_rule(), snapshot_for("apps"), fs.as_ref()).await,
            Some(true)
        );
        assert_eq!(
            probe(&tailwind_rule(), snapshot_for("plain"), fs.as_ref()).await,
            Some(false)
        );
        assert_eq!(
            probe(&notes_rule, snapshot_for("notes"), fs.as_ref()).await,
            Some(true)
        );
        assert_eq!(
            probe(&notes_rule, snapshot_for("plain"), fs.as_ref()).await,
            Some(false)
        );
    }
}
