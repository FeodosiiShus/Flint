use std::sync::{Arc, LazyLock};

use collections::HashSet;
use db::kvp::KeyValueStore;
use editor::Editor;
use extension_host::{ExtensionSettings, ExtensionStore};
use extension_suggest::SuggestedExtension;
use gpui::{App, AppContext as _, Context, Entity, Global, SharedString};
use language::{Buffer, PLAIN_TEXT, ServerActivationRule};
use markdown::{Markdown, MarkdownElement};
use project::lsp_store::LspStoreEvent;
use project::{Project, WorktreeId};
use settings::Settings as _;
use ui::prelude::*;
use util::ResultExt;
use workspace::notifications::{
    NotificationId, markdown_style, simple_message_notification::MessageNotification,
};
use workspace::{AppState, Event as WorkspaceEvent, Workspace};
use worktree::PathChange;

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

struct ExtensionSuggestionNotification;

pub(crate) fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, window, cx| {
        if window.is_none() {
            return;
        }
        watch_project_markers(workspace.project().clone(), cx);
        let lsp_store = workspace.project().read(cx).lsp_store();
        cx.subscribe(&lsp_store, |workspace, _, event, cx| {
            if let LspStoreEvent::LanguageDetected { buffer, .. } = event {
                suggest_for_buffer(workspace, buffer.clone(), cx);
            }
        })
        .detach();
        cx.subscribe_self(|workspace, event, cx| {
            if let WorkspaceEvent::ItemAdded { item } = event
                && let Some(editor) = item.downcast::<Editor>()
                && let Some(buffer) = editor.read(cx).buffer().read(cx).as_singleton()
            {
                suggest_for_buffer(workspace, buffer, cx);
            }
        })
        .detach();
        cx.subscribe(
            &ExtensionStore::global(cx),
            |workspace, extension_store, event, cx| {
                if let extension_host::Event::ExtensionsUpdated = event {
                    let installed = extension_store
                        .read(cx)
                        .installed_extensions()
                        .keys()
                        .map(|extension_id| notification_id(extension_id))
                        .collect::<Vec<_>>();
                    for installed in installed {
                        workspace.dismiss_notification(&installed, cx);
                    }
                }
            },
        )
        .detach();
    })
    .detach();
}

fn language_extension_key(extension_id: &str) -> String {
    format!("{extension_id}_extension_suggest")
}

fn suggestion_dismissed(extension_id: &str, cx: &App) -> bool {
    KeyValueStore::global(cx)
        .read_kvp(&language_extension_key(extension_id))
        .log_err()
        != Some(None)
}

fn dismiss_suggestion(extension_id: &str, cx: &mut App) {
    let key = language_extension_key(extension_id);
    let kvp = KeyValueStore::global(cx);
    db::write_and_log(cx, move || async move {
        kvp.write_kvp(key, "dismissed".to_string()).await
    });

    let notification_id = notification_id(extension_id);
    let workspaces = AppState::global(cx)
        .workspace_store
        .read(cx)
        .workspaces()
        .cloned()
        .collect::<Vec<_>>();
    for workspace in workspaces {
        workspace
            .update(cx, |workspace, cx| {
                workspace.dismiss_notification(&notification_id, cx);
            })
            .ok();
    }
}

fn notification_id(extension_id: &str) -> NotificationId {
    NotificationId::composite::<ExtensionSuggestionNotification>(SharedString::from(extension_id))
}

fn suggest_for_buffer(
    workspace: &mut Workspace,
    buffer: Entity<Buffer>,
    cx: &mut Context<Workspace>,
) {
    if !is_active_in_some_pane(workspace, &buffer, cx) {
        return;
    }

    let buffer = buffer.read(cx);
    let Some(file) = buffer.file().cloned() else {
        return;
    };
    let language_name = buffer
        .language()
        .filter(|language| **language != *PLAIN_TEXT)
        .map(|language| language.name());

    match language_name {
        Some(language_name) => {
            let Some(suggestion) =
                extension_suggest::additional_suggestion_for_language(language_name.as_ref())
            else {
                return;
            };
            show_suggestion(workspace, suggestion.extension_id, cx, |cx| {
                let markdown = cx.new(|cx| {
                    Markdown::new(
                        SharedString::new_static(suggestion.description),
                        None,
                        None,
                        cx,
                    )
                });
                MessageNotification::new_from_builder(cx, move |window, cx| {
                    MarkdownElement::new(markdown.clone(), markdown_style(window, cx))
                        .text_size(TextSize::Default.rems(cx))
                        .into_any_element()
                })
                .with_title(suggestion.title)
                .more_info_message("Learn more")
                .more_info_url(suggestion.docs_url)
                .primary_message(suggestion.install_message)
                .secondary_message("Don't show again")
            });
        }
        None => {
            let language_exists = workspace
                .project()
                .read(cx)
                .languages()
                .language_for_file(&file, Some(buffer.as_rope()), cx)
                .is_some();
            if language_exists {
                return;
            }
            let Some(SuggestedExtension {
                extension_id,
                file_name_or_extension,
            }) = extension_suggest::suggest_extension(file.path())
            else {
                return;
            };
            show_suggestion(workspace, &extension_id, cx, |cx| {
                MessageNotification::new(
                    format!(
                        "Do you want to install the recommended '{extension_id}' extension for '{file_name_or_extension}' files?"
                    ),
                    cx,
                )
                .primary_message("Yes, install extension")
                .secondary_message("No, don't install it")
            });
        }
    }
}

fn is_active_in_some_pane(workspace: &Workspace, buffer: &Entity<Buffer>, cx: &App) -> bool {
    workspace.panes().iter().any(|pane| {
        pane.read(cx)
            .active_item()
            .and_then(|item| item.downcast::<Editor>())
            .is_some_and(|editor| {
                editor.read(cx).buffer().read(cx).as_singleton().as_ref() == Some(buffer)
            })
    })
}

fn show_suggestion(
    workspace: &mut Workspace,
    extension_id: &str,
    cx: &mut Context<Workspace>,
    build_notification: impl FnOnce(&mut Context<MessageNotification>) -> MessageNotification,
) {
    let notification_id = notification_id(extension_id);
    if workspace.has_notification(&notification_id) {
        return;
    }

    let extension_store = ExtensionStore::global(cx);
    let extension_store = extension_store.read(cx);
    if extension_store
        .installed_extensions()
        .contains_key(extension_id)
        || extension_store
            .outstanding_operations()
            .contains_key(extension_id)
        || ExtensionSettings::get_global(cx)
            .auto_install_extensions
            .get(extension_id)
            == Some(&true)
        || suggestion_dismissed(extension_id, cx)
    {
        return;
    }

    let extension_id = Arc::<str>::from(extension_id);
    workspace.show_notification(notification_id, cx, |cx| {
        cx.new(|cx| {
            build_notification(cx)
                .primary_icon(IconName::Check)
                .primary_icon_color(Color::Success)
                .primary_on_click({
                    let extension_id = extension_id.clone();
                    move |_window, cx| {
                        ExtensionStore::global(cx).update(cx, |store, cx| {
                            store.install_latest_extension(extension_id.clone(), cx);
                        });
                    }
                })
                .secondary_icon(IconName::Close)
                .secondary_icon_color(Color::Error)
                .secondary_on_click(move |_window, cx| dismiss_suggestion(&extension_id, cx))
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use extension::ExtensionHostProxy;
    use extension_host::RELOAD_DEBOUNCE_DURATION;
    use fs::RemoveOptions;
    use gpui::{AnyView, TestAppContext, VisualTestContext};
    use http_client::HttpClient as _;
    use language::{Language, LanguageConfig, LanguageMatcher};
    use project::{Project, WorktreeId};
    use serde_json::json;
    use settings::SettingsStore;
    use util::path;
    use util::rel_path::rel_path;
    use workspace::{AppState, SplitDirection};

    const EMMET_EXTENSION_ID: &str = "emmet";

    #[gpui::test]
    async fn test_language_suggestion_for_supported_language(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;

        open_file(&workspace, "index.html", cx).await;

        assert_eq!(
            notification_ids(&workspace, cx),
            vec![notification_id(EMMET_EXTENSION_ID)]
        );
    }

    #[gpui::test]
    async fn test_language_suggestion_when_language_is_detected_after_editor(
        cx: &mut TestAppContext,
    ) {
        let app_state = init_test(cx);
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;

        open_file(&workspace, "main.rs", cx).await;
        assert_eq!(notification_ids(&workspace, cx), Vec::new());

        let (project, buffer) = workspace.read_with(cx, |workspace, cx| {
            let editor = workspace.active_item_as::<Editor>(cx).unwrap();
            let buffer = editor.read(cx).buffer().read(cx).as_singleton().unwrap();
            (workspace.project().clone(), buffer)
        });
        let html = project
            .read_with(cx, |project, _| {
                project.languages().language_for_name("HTML")
            })
            .await
            .unwrap();
        project.update(cx, |project, cx| {
            project.set_language_for_buffer(&buffer, html, cx);
        });
        cx.run_until_parked();

        assert_eq!(
            notification_ids(&workspace, cx),
            vec![notification_id(EMMET_EXTENSION_ID)]
        );
    }

    #[gpui::test]
    async fn test_no_suggestion_for_buffer_without_active_editor(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;

        open_file(&workspace, "main.rs", cx).await;
        let worktree_id = worktree_id(&workspace, cx);
        let project = workspace.read_with(cx, |workspace, _| workspace.project().clone());
        project
            .update(cx, |project, cx| {
                project.open_buffer((worktree_id, rel_path("index.html")), cx)
            })
            .await
            .unwrap();
        cx.run_until_parked();

        assert_eq!(notification_ids(&workspace, cx), Vec::new());
    }

    #[gpui::test]
    async fn test_no_language_suggestion_for_unsupported_language(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;

        open_file(&workspace, "main.rs", cx).await;

        assert_eq!(notification_ids(&workspace, cx), Vec::new());
    }

    #[gpui::test]
    async fn test_no_suggestion_after_dismissal(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        cx.update(|cx| dismiss_suggestion(EMMET_EXTENSION_ID, cx));
        cx.run_until_parked();
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;

        open_file(&workspace, "index.html", cx).await;

        assert_eq!(notification_ids(&workspace, cx), Vec::new());
    }

    #[gpui::test]
    async fn test_dismissal_applies_to_already_open_workspace(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;

        open_file(&workspace, "index.html", cx).await;
        assert_eq!(
            notification_ids(&workspace, cx),
            vec![notification_id(EMMET_EXTENSION_ID)]
        );

        cx.update(|_, cx| dismiss_suggestion(EMMET_EXTENSION_ID, cx));
        cx.run_until_parked();
        workspace.update(cx, |workspace, cx| {
            workspace.dismiss_notification(&notification_id(EMMET_EXTENSION_ID), cx)
        });

        open_file(&workspace, "other.html", cx).await;

        assert_eq!(notification_ids(&workspace, cx), Vec::new());
    }

    #[gpui::test]
    async fn test_no_suggestion_when_installed(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        install_emmet_on_disk(&app_state, cx).await;
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;

        open_file(&workspace, "index.html", cx).await;

        assert_eq!(notification_ids(&workspace, cx), Vec::new());
    }

    #[gpui::test]
    async fn test_dont_show_again_dismisses_card_in_all_workspaces(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;
        let (other_workspace, cx) = open_test_workspace(&app_state, cx).await;
        open_file(&workspace, "index.html", cx).await;
        open_file(&other_workspace, "index.html", cx).await;
        assert_eq!(
            notification_ids(&workspace, cx),
            vec![notification_id(EMMET_EXTENSION_ID)]
        );
        assert_eq!(
            notification_ids(&other_workspace, cx),
            vec![notification_id(EMMET_EXTENSION_ID)]
        );

        cx.update(|_, cx| dismiss_suggestion(EMMET_EXTENSION_ID, cx));
        cx.run_until_parked();

        assert_eq!(notification_ids(&workspace, cx), Vec::new());
        assert_eq!(notification_ids(&other_workspace, cx), Vec::new());
    }

    #[gpui::test]
    async fn test_visible_suggestion_survives_unrelated_extension_install(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;

        open_file(&workspace, "index.html", cx).await;
        let shown = notification_views(&workspace, cx);
        assert_eq!(shown.len(), 1);

        install_extension_on_disk("gleam", &app_state, cx).await;

        let still_shown = notification_views(&workspace, cx);
        assert_eq!(still_shown.len(), 1);
        assert_eq!(still_shown[0].entity_id(), shown[0].entity_id());
    }

    #[gpui::test]
    async fn test_visible_suggestion_is_dismissed_when_extension_gets_installed(
        cx: &mut TestAppContext,
    ) {
        let app_state = init_test(cx);
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;

        open_file(&workspace, "index.html", cx).await;
        assert_eq!(
            notification_ids(&workspace, cx),
            vec![notification_id(EMMET_EXTENSION_ID)]
        );

        install_emmet_on_disk(&app_state, cx).await;

        assert_eq!(notification_ids(&workspace, cx), Vec::new());
    }

    #[gpui::test]
    async fn test_suggestion_returns_after_uninstall(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        install_emmet_on_disk(&app_state, cx).await;
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;

        open_file(&workspace, "index.html", cx).await;
        assert_eq!(notification_ids(&workspace, cx), Vec::new());

        app_state
            .fs
            .remove_dir(
                &paths::extensions_dir()
                    .join("installed")
                    .join(EMMET_EXTENSION_ID),
                RemoveOptions {
                    recursive: true,
                    ignore_if_not_exists: false,
                },
            )
            .await
            .unwrap();
        cx.executor().advance_clock(RELOAD_DEBOUNCE_DURATION);
        cx.run_until_parked();
        cx.update(|_, cx| {
            assert_eq!(
                ExtensionStore::global(cx)
                    .read(cx)
                    .installed_extensions()
                    .keys()
                    .collect::<Vec<_>>(),
                Vec::<&Arc<str>>::new()
            );
        });
        open_file(&workspace, "other.html", cx).await;

        assert_eq!(
            notification_ids(&workspace, cx),
            vec![notification_id(EMMET_EXTENSION_ID)]
        );
    }

    #[gpui::test]
    async fn test_no_suggestion_while_install_is_pending(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        app_state
            .client
            .http_client()
            .as_fake()
            .replace_handler(|_, _| std::future::pending());
        cx.update(|cx| {
            ExtensionStore::global(cx).update(cx, |store, cx| {
                store.install_latest_extension(Arc::from(EMMET_EXTENSION_ID), cx);
            });
        });
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(
                ExtensionStore::global(cx)
                    .read(cx)
                    .outstanding_operations()
                    .keys()
                    .collect::<Vec<_>>(),
                vec![&Arc::from(EMMET_EXTENSION_ID)]
            );
        });
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;

        open_file(&workspace, "index.html", cx).await;

        assert_eq!(notification_ids(&workspace, cx), Vec::new());
    }

    #[gpui::test]
    async fn test_suggestion_returns_after_failed_install(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        cx.update(|cx| {
            ExtensionStore::global(cx).update(cx, |store, cx| {
                store.install_latest_extension(Arc::from(EMMET_EXTENSION_ID), cx);
            });
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let store = ExtensionStore::global(cx);
            let store = store.read(cx);
            assert_eq!(store.outstanding_operations().len(), 0);
            assert_eq!(store.installed_extensions().len(), 0);
        });
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;

        open_file(&workspace, "index.html", cx).await;

        assert_eq!(
            notification_ids(&workspace, cx),
            vec![notification_id(EMMET_EXTENSION_ID)]
        );
    }

    #[gpui::test]
    async fn test_suggestion_returns_after_uninstall_through_store(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        install_emmet_on_disk(&app_state, cx).await;
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;

        open_file(&workspace, "index.html", cx).await;
        assert_eq!(notification_ids(&workspace, cx), Vec::new());

        let uninstall = cx.update(|_, cx| {
            ExtensionStore::global(cx).update(cx, |store, cx| {
                store.uninstall_extension(Arc::from(EMMET_EXTENSION_ID), cx)
            })
        });
        cx.executor().advance_clock(RELOAD_DEBOUNCE_DURATION * 4);
        uninstall.await.unwrap();
        cx.run_until_parked();
        assert_eq!(notification_ids(&workspace, cx), Vec::new());

        open_file(&workspace, "other.html", cx).await;

        assert_eq!(
            notification_ids(&workspace, cx),
            vec![notification_id(EMMET_EXTENSION_ID)]
        );
    }

    #[gpui::test]
    async fn test_no_suggestion_when_auto_install_is_enabled(cx: &mut TestAppContext) {
        assert_eq!(
            suggestions_with_auto_install_setting(true, cx).await,
            Vec::new()
        );
    }

    #[gpui::test]
    async fn test_suggestion_when_auto_install_is_disabled(cx: &mut TestAppContext) {
        assert_eq!(
            suggestions_with_auto_install_setting(false, cx).await,
            vec![notification_id(EMMET_EXTENSION_ID)]
        );
    }

    #[gpui::test]
    async fn test_visible_suggestion_is_not_rebuilt_by_next_candidate(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;

        open_file(&workspace, "index.html", cx).await;
        let shown = notification_views(&workspace, cx);
        assert_eq!(shown.len(), 1);

        open_file(&workspace, "other.html", cx).await;

        let still_shown = notification_views(&workspace, cx);
        assert_eq!(still_shown.len(), 1);
        assert_eq!(still_shown[0].entity_id(), shown[0].entity_id());
    }

    #[gpui::test]
    async fn test_closed_suggestion_returns_for_next_candidate(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;

        open_file(&workspace, "index.html", cx).await;
        let notification = notification_views(&workspace, cx)
            .pop()
            .unwrap()
            .downcast::<MessageNotification>()
            .ok()
            .unwrap();
        notification.update(cx, |notification, cx| notification.dismiss(cx));
        cx.run_until_parked();
        assert_eq!(notification_ids(&workspace, cx), Vec::new());
        assert!(!cx.update(|_, cx| suggestion_dismissed(EMMET_EXTENSION_ID, cx)));

        open_file(&workspace, "other.html", cx).await;

        assert_eq!(
            notification_ids(&workspace, cx),
            vec![notification_id(EMMET_EXTENSION_ID)]
        );
    }

    #[gpui::test]
    async fn test_suggestion_for_item_added_to_inactive_pane(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;

        open_file(&workspace, "main.rs", cx).await;
        let first_pane = workspace.read_with(cx, |workspace, _| workspace.active_pane().clone());
        let second_pane = workspace.update_in(cx, |workspace, window, cx| {
            workspace.split_pane(first_pane.clone(), SplitDirection::Right, window, cx)
        });
        cx.run_until_parked();
        let active_pane = workspace.read_with(cx, |workspace, _| workspace.active_pane().clone());
        let inactive_pane = if active_pane == first_pane {
            second_pane
        } else {
            first_pane
        };

        let worktree_id = worktree_id(&workspace, cx);
        let project = workspace.read_with(cx, |workspace, _| workspace.project().clone());
        let buffer = project
            .update(cx, |project, cx| {
                project.open_buffer((worktree_id, rel_path("index.html")), cx)
            })
            .await
            .unwrap();
        inactive_pane.update_in(cx, |pane, window, cx| {
            let editor = cx.new(|cx| Editor::for_buffer(buffer, Some(project), window, cx));
            pane.add_item(Box::new(editor), false, false, None, window, cx);
        });
        cx.run_until_parked();

        assert_eq!(
            workspace.read_with(cx, |workspace, _| workspace.active_pane().clone()),
            active_pane
        );
        assert_eq!(
            notification_ids(&workspace, cx),
            vec![notification_id(EMMET_EXTENSION_ID)]
        );
    }

    #[gpui::test]
    async fn test_file_suggestion_for_unknown_language(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;

        open_file(&workspace, "main.gleam", cx).await;

        assert_eq!(
            notification_ids(&workspace, cx),
            vec![notification_id("gleam")]
        );
    }

    #[gpui::test]
    async fn test_file_suggestion_for_untitled_buffer_saved_as_unknown_language(
        cx: &mut TestAppContext,
    ) {
        let app_state = init_test(cx);
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;
        let project = workspace.read_with(cx, |workspace, _| workspace.project().clone());

        let buffer = project
            .update(cx, |project, cx| project.create_buffer(None, false, cx))
            .await
            .unwrap();
        workspace.update_in(cx, |workspace, window, cx| {
            let editor =
                cx.new(|cx| Editor::for_buffer(buffer.clone(), Some(project.clone()), window, cx));
            workspace.add_item_to_active_pane(Box::new(editor), None, true, window, cx);
        });
        cx.run_until_parked();
        assert_eq!(notification_ids(&workspace, cx), Vec::new());

        let worktree_id = worktree_id(&workspace, cx);
        project
            .update(cx, |project, cx| {
                project.save_buffer_as(buffer, (worktree_id, rel_path("new.gleam")).into(), cx)
            })
            .await
            .unwrap();
        cx.run_until_parked();

        assert_eq!(
            notification_ids(&workspace, cx),
            vec![notification_id("gleam")]
        );
    }

    #[gpui::test]
    async fn test_no_file_suggestion_while_language_is_loading(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;
        let gleam = test_language_config("Gleam", "gleam");
        workspace.read_with(cx, |workspace, cx| {
            workspace.project().read(cx).languages().register_language(
                gleam.name,
                None,
                gleam.matcher,
                false,
                None,
                Arc::new(|| Box::pin(std::future::pending())),
            );
        });

        open_file(&workspace, "main.gleam", cx).await;

        assert_eq!(notification_ids(&workspace, cx), Vec::new());
    }

    #[test]
    fn test_marker_install_table_compiles_for_csharp_and_java_only() {
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
    fn test_marker_install_matches_dotnet_markers() {
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
    fn test_marker_install_matches_java_markers() {
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
    fn test_marker_install_ignores_source_files_and_lookalikes() {
        for path in [
            "Program.cs",
            "Main.java",
            "README.md",
            "Cargo.toml",
            "package.json",
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
    async fn test_marker_install_requests_csharp_for_project_file_in_worktree(
        cx: &mut TestAppContext,
    ) {
        let app_state = init_test(cx);
        let (_workspace, cx) = open_workspace_with_tree(
            &app_state,
            json!({ "src": { "App.csproj": "<Project />" } }),
            cx,
        )
        .await;

        assert_eq!(requested_marker_extensions(cx), vec!["csharp"]);
    }

    #[gpui::test]
    async fn test_marker_install_requests_java_for_nested_gradle_file(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let (_workspace, cx) = open_workspace_with_tree(
            &app_state,
            json!({ "services": { "api": { "build.gradle.kts": "" } } }),
            cx,
        )
        .await;

        assert_eq!(requested_marker_extensions(cx), vec!["java"]);
    }

    #[gpui::test]
    async fn test_marker_install_requests_every_extension_with_a_marker(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let (_workspace, cx) = open_workspace_with_tree(
            &app_state,
            json!({ "App.sln": "", "pom.xml": "<project />" }),
            cx,
        )
        .await;

        assert_eq!(requested_marker_extensions(cx), vec!["csharp", "java"]);
    }

    #[gpui::test]
    async fn test_marker_install_ignores_worktree_without_markers(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let (_workspace, cx) = open_workspace_with_tree(
            &app_state,
            json!({ "Program.cs": "", "Main.java": "", "main.rs": "fn main() {}" }),
            cx,
        )
        .await;

        assert_eq!(requested_marker_extensions(cx), Vec::<&str>::new());
    }

    #[gpui::test]
    async fn test_marker_install_respects_explicit_opt_out(cx: &mut TestAppContext) {
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
        let (_workspace, cx) = open_workspace_with_tree(
            &app_state,
            json!({ "App.sln": "", "pom.xml": "<project />" }),
            cx,
        )
        .await;

        assert_eq!(requested_marker_extensions(cx), vec!["java"]);
    }

    #[gpui::test]
    async fn test_marker_install_skips_installed_extension(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        install_extension_on_disk("csharp", &app_state, cx).await;
        let (_workspace, cx) = open_workspace_with_tree(
            &app_state,
            json!({ "App.sln": "", "pom.xml": "<project />" }),
            cx,
        )
        .await;

        assert_eq!(requested_marker_extensions(cx), vec!["java"]);
    }

    #[gpui::test]
    async fn test_marker_install_triggers_when_marker_file_appears(cx: &mut TestAppContext) {
        let app_state = init_test(cx);
        let (_workspace, cx) =
            open_workspace_with_tree(&app_state, json!({ "main.rs": "fn main() {}" }), cx).await;
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
            cx.update_global::<SettingsStore, _>(|store, cx| {
                store.update_user_settings(cx, |content| {
                    content
                        .extension
                        .auto_install_extensions
                        .insert(Arc::from("html"), false);
                });
            });
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
            editor::init(cx);
            crate::init(cx);
            app_state
        })
    }

    async fn install_emmet_on_disk(app_state: &Arc<AppState>, cx: &mut TestAppContext) {
        install_extension_on_disk(EMMET_EXTENSION_ID, app_state, cx).await;
    }

    async fn install_extension_on_disk(
        extension_id: &str,
        app_state: &Arc<AppState>,
        cx: &mut TestAppContext,
    ) {
        app_state
            .fs
            .as_fake()
            .insert_tree(
                paths::extensions_dir().join("installed").join(extension_id),
                json!({
                    "extension.toml": format!(
                        "id = \"{extension_id}\"\nname = \"{extension_id}\"\nversion = \"0.0.1\"\nschema_version = 1\n"
                    ),
                }),
            )
            .await;
        cx.executor().advance_clock(RELOAD_DEBOUNCE_DURATION);
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(
                ExtensionStore::global(cx)
                    .read(cx)
                    .installed_extensions()
                    .contains_key(extension_id)
            );
        });
    }

    async fn open_test_workspace<'a>(
        app_state: &Arc<AppState>,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Workspace>, &'a mut VisualTestContext) {
        open_workspace_with_tree(
            app_state,
            json!({
                "index.html": "<div></div>",
                "other.html": "<span></span>",
                "main.rs": "fn main() {}",
                "main.gleam": "pub fn main() {}",
            }),
            cx,
        )
        .await
    }

    async fn open_workspace_with_tree<'a>(
        app_state: &Arc<AppState>,
        tree: serde_json::Value,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Workspace>, &'a mut VisualTestContext) {
        let fs = app_state.fs.as_fake();
        fs.insert_tree(path!("/root"), tree).await;
        let project = Project::test(fs, [path!("/root").as_ref()], cx).await;
        project.read_with(cx, |project, _| {
            project
                .languages()
                .add(Arc::new(test_language("HTML", "html")));
            project
                .languages()
                .add(Arc::new(test_language("Rust", "rs")));
        });
        cx.add_window_view(|window, cx| {
            Workspace::new(None, project, app_state.clone(), window, cx)
        })
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

    async fn open_file(workspace: &Entity<Workspace>, file_name: &str, cx: &mut VisualTestContext) {
        let worktree_id = worktree_id(workspace, cx);
        workspace
            .update_in(cx, |workspace, window, cx| {
                workspace.open_path((worktree_id, rel_path(file_name)), None, true, window, cx)
            })
            .await
            .unwrap();
        cx.run_until_parked();
    }

    fn worktree_id(workspace: &Entity<Workspace>, cx: &VisualTestContext) -> WorktreeId {
        workspace.read_with(cx, |workspace, cx| {
            workspace
                .project()
                .read(cx)
                .worktrees(cx)
                .next()
                .unwrap()
                .read(cx)
                .id()
        })
    }

    async fn suggestions_with_auto_install_setting(
        auto_install: bool,
        cx: &mut TestAppContext,
    ) -> Vec<NotificationId> {
        let app_state = init_test(cx);
        cx.update(|cx| {
            cx.update_global::<SettingsStore, _>(|store, cx| {
                store.update_user_settings(cx, |content| {
                    content
                        .extension
                        .auto_install_extensions
                        .insert(Arc::from(EMMET_EXTENSION_ID), auto_install);
                });
            });
        });
        let (workspace, cx) = open_test_workspace(&app_state, cx).await;

        open_file(&workspace, "index.html", cx).await;

        notification_ids(&workspace, cx)
    }

    fn notification_ids(
        workspace: &Entity<Workspace>,
        cx: &VisualTestContext,
    ) -> Vec<NotificationId> {
        workspace.read_with(cx, |workspace, _| workspace.notification_ids())
    }

    fn notification_views(workspace: &Entity<Workspace>, cx: &VisualTestContext) -> Vec<AnyView> {
        workspace.read_with(cx, |workspace, _| workspace.notification_views())
    }

    fn test_language(name: &'static str, path_suffix: &str) -> Language {
        Language::new(test_language_config(name, path_suffix), None)
    }

    fn test_language_config(name: &'static str, path_suffix: &str) -> LanguageConfig {
        LanguageConfig {
            name: name.into(),
            matcher: LanguageMatcher {
                path_suffixes: vec![path_suffix.to_string()],
                ..LanguageMatcher::default()
            }
            .into(),
            ..LanguageConfig::default()
        }
    }
}
