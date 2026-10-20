use std::sync::{Arc, LazyLock};

use collections::HashSet;
use extension_host::{ExtensionSettings, ExtensionStore};
use gpui::{App, Context, Entity, Global};
use language::ServerActivationRule;
use project::{Project, WorktreeId};
use settings::Settings as _;
use workspace::Workspace;
use worktree::PathChange;

/// The language support Flint installs from the extension registry once a project needs it,
/// like IntelliJ's bundled Java and .NET support. An extension brings the grammar and the
/// language server (jdtls, Roslyn), which then starts for the project's files.
const PROJECT_MARKER_EXTENSIONS: &[(&str, &[&str])] = &[
    ("csharp", &["**/*.sln", "**/*.slnx", "**/*.csproj"]),
    (
        "java",
        &[
            "**/pom.xml",
            "**/build.gradle",
            "**/build.gradle.kts",
            "**/settings.gradle",
            "**/settings.gradle.kts",
        ],
    ),
];

struct ProjectMarkerExtension {
    extension_id: &'static str,
    markers: ServerActivationRule,
}

/// Extensions already requested in this session, so a failed download is not retried for
/// every file event.
#[derive(Default)]
struct RequestedMarkerInstalls(HashSet<&'static str>);

impl Global for RequestedMarkerInstalls {}

static PROJECT_MARKER_RULES: LazyLock<Vec<ProjectMarkerExtension>> = LazyLock::new(|| {
    PROJECT_MARKER_EXTENSIONS
        .iter()
        .map(|&(extension_id, markers)| ProjectMarkerExtension {
            extension_id,
            markers: ServerActivationRule::new(markers, &[]),
        })
        .collect()
});

pub(crate) fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, window, cx| {
        if window.is_none() {
            return;
        }
        watch_project_markers(workspace.project().clone(), cx);
    })
    .detach();
}

fn pending_marker_extensions(cx: &App) -> Vec<&'static ProjectMarkerExtension> {
    let extension_store = ExtensionStore::global(cx);
    let extension_store = extension_store.read(cx);
    let settings = ExtensionSettings::get_global(cx);
    let requested = cx.try_global::<RequestedMarkerInstalls>();
    PROJECT_MARKER_RULES
        .iter()
        .filter(|extension| {
            settings.should_auto_install(extension.extension_id)
                && !extension_store
                    .installed_extensions()
                    .contains_key(extension.extension_id)
                && !extension_store
                    .outstanding_operations()
                    .contains_key(extension.extension_id)
                && !requested.is_some_and(|requested| requested.0.contains(extension.extension_id))
        })
        .collect()
}

fn install_extensions_for_marker_paths<'a>(paths: impl Iterator<Item = &'a str>, cx: &mut App) {
    let mut pending = pending_marker_extensions(cx);
    let mut matched = Vec::new();
    for path in paths {
        if pending.is_empty() {
            break;
        }
        pending.retain(|extension| {
            let is_match = extension.markers.matches_workspace_file(path);
            if is_match {
                matched.push(extension.extension_id);
            }
            !is_match
        });
    }
    if matched.is_empty() {
        return;
    }

    cx.default_global::<RequestedMarkerInstalls>()
        .0
        .extend(matched.iter().copied());
    ExtensionStore::global(cx).update(cx, |extension_store, cx| {
        for extension_id in matched {
            extension_store.install_latest_extension(Arc::from(extension_id), cx);
        }
    });
}

fn install_extensions_for_worktree(
    project: &Entity<Project>,
    worktree_id: WorktreeId,
    cx: &mut App,
) {
    let Some(worktree) = project.read(cx).worktree_for_id(worktree_id, cx) else {
        return;
    };
    let snapshot = worktree.read(cx).snapshot();
    install_extensions_for_marker_paths(
        snapshot
            .entries(false, 0)
            .filter(|entry| entry.is_file())
            .map(|entry| entry.path.as_unix_str()),
        cx,
    );
}

fn watch_project_markers(project: Entity<Project>, cx: &mut Context<Workspace>) {
    if project.read(cx).is_via_collab() {
        return;
    }

    cx.subscribe(&project, |_, project, event, cx| match event {
        project::Event::WorktreeAdded(worktree_id) => {
            install_extensions_for_worktree(&project, *worktree_id, cx);
        }
        project::Event::WorktreeUpdatedEntries(_, changes) => {
            install_extensions_for_marker_paths(
                changes
                    .iter()
                    .filter(|(_, _, change)| *change != PathChange::Removed)
                    .map(|(path, _, _)| path.as_unix_str()),
                cx,
            );
        }
        _ => {}
    })
    .detach();

    let worktree_ids = project
        .read(cx)
        .worktrees(cx)
        .map(|worktree| worktree.read(cx).id())
        .collect::<Vec<_>>();
    for worktree_id in worktree_ids {
        install_extensions_for_worktree(&project, worktree_id, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extension::ExtensionHostProxy;
    use extension_host::RELOAD_DEBOUNCE_DURATION;
    use gpui::{BorrowAppContext as _, TestAppContext, VisualTestContext};
    use serde_json::json;
    use settings::SettingsStore;
    use util::path;
    use workspace::AppState;

    #[test]
    fn marker_install_covers_csharp_and_java() {
        let extension_ids = PROJECT_MARKER_RULES
            .iter()
            .map(|extension| extension.extension_id)
            .collect::<Vec<_>>();
        assert_eq!(extension_ids, vec!["csharp", "java"]);
        assert!(
            PROJECT_MARKER_RULES
                .iter()
                .all(|extension| extension.markers.has_workspace_files())
        );
    }

    #[test]
    fn dotnet_solutions_and_projects_ask_for_csharp() {
        for path in [
            "App.sln",
            "App.slnx",
            "App.csproj",
            "src/Api/Api.csproj",
            "nested/deep/Solution.sln",
        ] {
            assert_eq!(marker_extension_ids(path), vec!["csharp"], "{path}");
        }
    }

    #[test]
    fn maven_and_gradle_projects_ask_for_java() {
        for path in [
            "pom.xml",
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
            "services/api/pom.xml",
            "services/api/build.gradle.kts",
        ] {
            assert_eq!(marker_extension_ids(path), vec!["java"], "{path}");
        }
    }

    #[test]
    fn source_files_and_lookalikes_ask_for_nothing() {
        for path in [
            "Program.cs",
            "Main.java",
            "README.md",
            "Cargo.toml",
            "package.json",
            "tsconfig.json",
            "xpom.xml",
            "pom.xml.bak",
            "build.gradle.bak",
            "csproj",
            "src/sln",
        ] {
            assert_eq!(marker_extension_ids(path), Vec::<&str>::new(), "{path}");
        }
    }

    #[gpui::test]
    fn the_default_settings_install_java_and_csharp_and_keep_their_servers_small(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            let auto_install = &ExtensionSettings::get_global(cx).auto_install_extensions;
            assert_eq!(auto_install.get("java"), Some(&true));
            assert_eq!(auto_install.get("csharp"), Some(&true));
            let jdtls = project::project_settings::ProjectSettings::get_global(cx)
                .lsp
                .get(&language::LanguageServerName::new_static("jdtls"))
                .cloned()
                .expect("Flint configures jdtls");
            let settings = jdtls.settings.expect("jdtls has default settings");
            assert_eq!(settings.pointer("/min_memory"), Some(&"100m".into()));
            assert_eq!(settings.pointer("/max_memory"), Some(&"2G".into()));
            assert_eq!(
                settings.pointer("/jdk_auto_download"),
                None,
                "Corretto downloads fail, so a missing JDK must be reported as missing"
            );
            let java_options = jdtls
                .binary
                .and_then(|binary| binary.env)
                .and_then(|env| env.get("JDK_JAVA_OPTIONS").cloned())
                .unwrap_or_default();
            assert!(
                java_options.contains("-XX:+UseParallelGC"),
                "{java_options}"
            );
            let roslyn_env = project::project_settings::ProjectSettings::get_global(cx)
                .lsp
                .get(&language::LanguageServerName::new_static("roslyn"))
                .and_then(|roslyn| roslyn.binary.clone())
                .and_then(|binary| binary.env)
                .unwrap_or_default();
            assert_eq!(
                roslyn_env.get("DOTNET_gcServer").map(String::as_str),
                Some("0"),
                "Roslyn uses the workstation collector"
            );
        });
    }

    #[gpui::test]
    async fn a_dotnet_project_installs_csharp(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let cx = open_workspace_with_tree(
            &app_state,
            json!({ "src": { "App.csproj": "<Project />" } }),
            cx,
        )
        .await;

        assert_eq!(requested_marker_extensions(cx), vec!["csharp"]);
    }

    #[gpui::test]
    async fn a_nested_gradle_build_installs_java(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let cx = open_workspace_with_tree(
            &app_state,
            json!({ "services": { "api": { "build.gradle.kts": "" } } }),
            cx,
        )
        .await;

        assert_eq!(requested_marker_extensions(cx), vec!["java"]);
    }

    #[gpui::test]
    async fn a_mixed_project_installs_every_extension_it_needs(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let cx = open_workspace_with_tree(
            &app_state,
            json!({ "App.sln": "", "pom.xml": "<project />", "web": { "package.json": "{}" } }),
            cx,
        )
        .await;

        assert_eq!(requested_marker_extensions(cx), vec!["csharp", "java"]);
    }

    #[gpui::test]
    async fn source_files_without_a_build_file_install_nothing(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let cx = open_workspace_with_tree(
            &app_state,
            json!({ "Program.cs": "", "Main.java": "", "index.ts": "" }),
            cx,
        )
        .await;

        assert_eq!(requested_marker_extensions(cx), Vec::<&str>::new());
    }

    #[gpui::test]
    async fn an_extension_set_to_false_is_never_installed(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        cx.update(|cx| {
            cx.update_global::<SettingsStore, _>(|store, cx| {
                store.update_user_settings(cx, |content| {
                    content
                        .extension
                        .auto_install_extensions
                        .insert(Arc::from("csharp"), false);
                });
            });
        });
        let cx = open_workspace_with_tree(
            &app_state,
            json!({ "App.sln": "", "pom.xml": "<project />" }),
            cx,
        )
        .await;

        assert_eq!(requested_marker_extensions(cx), vec!["java"]);
    }

    #[gpui::test]
    async fn an_installed_extension_is_not_installed_again(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        app_state
            .fs
            .as_fake()
            .insert_tree(
                paths::extensions_dir().join("installed").join("csharp"),
                json!({
                    "extension.toml": "id = \"csharp\"\nname = \"C#\"\nversion = \"0.0.1\"\nschema_version = 1\n",
                }),
            )
            .await;
        cx.executor().advance_clock(RELOAD_DEBOUNCE_DURATION);
        cx.run_until_parked();
        let cx = open_workspace_with_tree(
            &app_state,
            json!({ "App.sln": "", "pom.xml": "<project />" }),
            cx,
        )
        .await;

        assert_eq!(requested_marker_extensions(cx), vec!["java"]);
    }

    #[gpui::test]
    async fn a_build_file_added_later_installs_its_extension(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let cx = open_workspace_with_tree(&app_state, json!({ "index.ts": "" }), cx).await;
        assert_eq!(requested_marker_extensions(cx), Vec::<&str>::new());

        app_state
            .fs
            .as_fake()
            .insert_file(path!("/root/pom.xml"), b"<project />".to_vec())
            .await;
        cx.executor().advance_clock(RELOAD_DEBOUNCE_DURATION);
        cx.run_until_parked();

        assert_eq!(requested_marker_extensions(cx), vec!["java"]);
    }

    fn init_test(cx: &mut TestAppContext) -> Arc<AppState> {
        cx.update(|cx| {
            let app_state = AppState::test(cx);
            AppState::set_global(app_state.clone(), cx);
            cx.set_global(db::AppDatabase::test_new());
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            extension::init(cx);
            extension_host::init(
                Arc::new(ExtensionHostProxy::new()),
                app_state.fs.clone(),
                app_state.client.clone(),
                app_state.node_runtime.clone(),
                cx,
            );
            init(cx);
            app_state
        })
    }

    async fn open_workspace_with_tree<'a>(
        app_state: &Arc<AppState>,
        tree: serde_json::Value,
        cx: &'a mut TestAppContext,
    ) -> &'a mut VisualTestContext {
        let fs = app_state.fs.as_fake();
        fs.insert_tree(path!("/root"), tree).await;
        let project = Project::test(fs, [path!("/root").as_ref()], cx).await;
        let (_, cx) = cx.add_window_view(|window, cx| {
            Workspace::new(None, project, app_state.clone(), window, cx)
        });
        cx.run_until_parked();
        cx
    }

    fn requested_marker_extensions(cx: &mut VisualTestContext) -> Vec<&'static str> {
        cx.update(|_, cx| {
            let mut requested = cx
                .try_global::<RequestedMarkerInstalls>()
                .map(|requested| requested.0.iter().copied().collect::<Vec<_>>())
                .unwrap_or_default();
            requested.sort_unstable();
            requested
        })
    }

    fn marker_extension_ids(path: &str) -> Vec<&'static str> {
        PROJECT_MARKER_RULES
            .iter()
            .filter(|extension| extension.markers.matches_workspace_file(path))
            .map(|extension| extension.extension_id)
            .collect()
    }
}
