use std::path::PathBuf;

use super::*;
use crate::{AppState, item::test::TestItem};
use fs::{FakeFs, Fs};
use gpui::{TestAppContext, UpdateGlobal as _, VisualTestContext};
use serde_json::json;
use settings::SettingsStore;
use util::path;

fn init_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
        theme_settings::init(theme::LoadThemes::JustBase, cx);
    });
}

fn setup_multi_workspace<'a>(
    projects: &[Entity<Project>],
    cx: &'a mut TestAppContext,
) -> (Entity<MultiWorkspace>, &'a mut VisualTestContext) {
    let mut iterator = projects.iter();
    let project = iterator
        .next()
        .expect("At least one project should be provided")
        .clone();

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project, window, cx));

    for project in iterator {
        multi_workspace.update_in(cx, |multi_workspace, window, cx| {
            multi_workspace.test_add_workspace(project.clone(), window, cx);
        })
    }

    // Opening the sidebar retains the workspaces and establishes their project groups.
    multi_workspace.update(cx, |multi_workspace, cx| multi_workspace.open_sidebar(cx));
    cx.run_until_parked();

    (multi_workspace, cx)
}

#[gpui::test]
async fn test_project_group_keys_initial(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file.txt": "" })).await;
    let project = Project::test(fs, ["/root_a".as_ref()], cx).await;

    let expected_key = project.read_with(cx, |project, cx| project.project_group_key(cx));

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project, window, cx));

    multi_workspace.update(cx, |mw, cx| {
        mw.open_sidebar(cx);
    });

    multi_workspace.read_with(cx, |mw, _cx| {
        let keys: Vec<ProjectGroupKey> = mw.project_group_keys();
        assert_eq!(keys.len(), 1, "should have exactly one key on creation");
        assert_eq!(keys[0], expected_key);
    });
}

#[gpui::test]
async fn test_project_group_keys_add_workspace(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file.txt": "" })).await;
    fs.insert_tree("/root_b", json!({ "file.txt": "" })).await;
    let project_a = Project::test(fs.clone(), ["/root_a".as_ref()], cx).await;
    let project_b = Project::test(fs.clone(), ["/root_b".as_ref()], cx).await;

    let key_a = project_a.read_with(cx, |p, cx| p.project_group_key(cx));
    let key_b = project_b.read_with(cx, |p, cx| p.project_group_key(cx));
    assert_ne!(
        key_a, key_b,
        "different roots should produce different keys"
    );

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a, window, cx));

    multi_workspace.update(cx, |mw, cx| {
        mw.open_sidebar(cx);
    });

    multi_workspace.read_with(cx, |mw, _cx| {
        assert_eq!(mw.project_group_keys().len(), 1);
    });

    // Adding a workspace with a different project root adds a new key.
    multi_workspace.update_in(cx, |mw, window, cx| {
        mw.test_add_workspace(project_b, window, cx);
    });

    multi_workspace.read_with(cx, |mw, _cx| {
        let keys: Vec<ProjectGroupKey> = mw.project_group_keys();
        assert_eq!(
            keys.len(),
            2,
            "should have two keys after adding a second workspace"
        );
        assert_eq!(keys[0], key_b);
        assert_eq!(keys[1], key_a);
    });
}

#[gpui::test]
async fn test_move_active_project_group_actions(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file.txt": "" })).await;
    fs.insert_tree("/root_b", json!({ "file.txt": "" })).await;
    let project_a = Project::test(fs.clone(), ["/root_a".as_ref()], cx).await;
    let project_b = Project::test(fs, ["/root_b".as_ref()], cx).await;

    let key_a = project_a.read_with(cx, |project, cx| project.project_group_key(cx));
    let key_b = project_b.read_with(cx, |project, cx| project.project_group_key(cx));

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a, window, cx));
    multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        // Moving project groups up and down is a sidebar action.
        multi_workspace.test_register_sidebar(cx);
        multi_workspace.test_add_workspace(project_b, window, cx);
    });

    assert_eq!(
        multi_workspace.read_with(cx, |multi_workspace, _| {
            multi_workspace.project_group_keys()
        }),
        vec![key_b.clone(), key_a.clone()]
    );

    cx.dispatch_action(MoveProjectDown);
    assert_eq!(
        multi_workspace.read_with(cx, |multi_workspace, _| {
            multi_workspace.project_group_keys()
        }),
        vec![key_a.clone(), key_b.clone()]
    );

    cx.dispatch_action(MoveProjectDown);
    assert_eq!(
        multi_workspace.read_with(cx, |multi_workspace, _| {
            multi_workspace.project_group_keys()
        }),
        vec![key_a.clone(), key_b.clone()]
    );

    cx.dispatch_action(MoveProjectUp);
    assert_eq!(
        multi_workspace.read_with(cx, |multi_workspace, _| {
            multi_workspace.project_group_keys()
        }),
        vec![key_b, key_a]
    );
}

#[gpui::test]
async fn test_open_new_window_does_not_open_sidebar_on_existing_window(cx: &mut TestAppContext) {
    init_test(cx);

    let app_state = cx.update(AppState::test);
    let fs = app_state.fs.as_fake();
    fs.insert_tree(path!("/project_a"), json!({ "file.txt": "" }))
        .await;
    fs.insert_tree(path!("/project_b"), json!({ "file.txt": "" }))
        .await;

    let project = Project::test(app_state.fs.clone(), [path!("/project_a").as_ref()], cx).await;

    let window = cx.add_window(|window, cx| MultiWorkspace::test_new(project, window, cx));

    window
        .read_with(cx, |mw, _cx| {
            assert!(!mw.sidebar_open(), "sidebar should start closed",);
        })
        .unwrap();

    cx.update(|cx| {
        open_paths(
            &[PathBuf::from(path!("/project_b"))],
            app_state,
            OpenOptions {
                open_mode: OpenMode::NewWindow,
                ..OpenOptions::default()
            },
            cx,
        )
    })
    .await
    .unwrap();

    window
        .read_with(cx, |mw, _cx| {
            assert!(
                !mw.sidebar_open(),
                "opening a project in a new window must not open the sidebar on the original window",
            );
        })
        .unwrap();
}

#[gpui::test]
async fn test_open_directory_in_existing_window_retains_workspace_without_opening_sidebar(
    cx: &mut TestAppContext,
) {
    init_test(cx);

    let app_state = cx.update(AppState::test);
    let fs = app_state.fs.as_fake();
    fs.insert_tree(path!("/project_a"), json!({ "file.txt": "" }))
        .await;
    fs.insert_tree(path!("/project_b"), json!({ "file.txt": "" }))
        .await;

    let project = Project::test(app_state.fs.clone(), [path!("/project_a").as_ref()], cx).await;
    let window = cx.add_window(|window, cx| MultiWorkspace::test_new(project, window, cx));
    cx.run_until_parked();

    cx.update(|cx| {
        open_paths(
            &[PathBuf::from(path!("/project_b"))],
            app_state,
            OpenOptions::default(),
            cx,
        )
    })
    .await
    .unwrap();

    window
        .read_with(cx, |mw, _cx| {
            assert!(
                !mw.sidebar_open(),
                "the sidebar must stay closed when a directory is added to an existing window",
            );
            assert_eq!(
                mw.workspaces().count(),
                2,
                "the directory is still added to the existing window, and the workspace it \
                 replaces is retained",
            );
        })
        .unwrap();
}

#[gpui::test]
async fn test_explicit_existing_window_request_is_honored_with_the_new_window_setting(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    set_default_open_behavior(cx, settings::DefaultOpenBehavior::NewWindow);

    let app_state = cx.update(AppState::test);
    let fs = app_state.fs.as_fake();
    fs.insert_tree(path!("/project_a"), json!({ "file.txt": "" }))
        .await;
    fs.insert_tree(path!("/project_b"), json!({ "file.txt": "" }))
        .await;
    fs.insert_tree(path!("/project_c"), json!({ "file.txt": "" }))
        .await;

    let project = Project::test(app_state.fs.clone(), [path!("/project_a").as_ref()], cx).await;
    let window = cx.add_window(|window, cx| MultiWorkspace::test_new(project, window, cx));
    cx.run_until_parked();
    let workspace_a = window
        .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
        .unwrap();

    let opened = cx
        .update(|cx| {
            open_paths(
                &[PathBuf::from(path!("/project_b"))],
                app_state.clone(),
                OpenOptions {
                    add_dirs_to_sidebar: true,
                    ..OpenOptions::default()
                },
                cx,
            )
        })
        .await
        .unwrap();
    assert_eq!(
        opened.window, window,
        "`flint -e` adds to the existing window"
    );
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);

    window
        .update(cx, |multi_workspace, window, cx| {
            multi_workspace.activate(workspace_a.clone(), None, window, cx);
        })
        .unwrap();
    cx.update(|cx| {
        open_paths(
            &[PathBuf::from(path!("/project_c"))],
            app_state,
            OpenOptions {
                add_dirs_to_sidebar: true,
                ..OpenOptions::default()
            },
            cx,
        )
    })
    .await
    .unwrap();
    cx.run_until_parked();

    window
        .read_with(cx, |multi_workspace, _| {
            assert!(
                multi_workspace
                    .workspaces()
                    .any(|workspace| workspace == &opened.workspace),
                "the project added by the explicit request stays in the window after a switch"
            );
            assert_eq!(multi_workspace.workspaces().count(), 3);
        })
        .unwrap();
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);
}

#[gpui::test]
async fn test_opening_a_folder_over_the_empty_workspace_replaces_it(cx: &mut TestAppContext) {
    init_test(cx);

    let app_state = cx.update(AppState::test);
    let fs = app_state.fs.as_fake();
    fs.insert_tree(path!("/project"), json!({ "file.txt": "" }))
        .await;

    let project = Project::test(app_state.fs.clone(), [], cx).await;
    let window = cx.add_window(|window, cx| {
        let multi_workspace = MultiWorkspace::test_new(project, window, cx);
        multi_workspace.workspace().update(cx, |workspace, cx| {
            workspace.active_pane().update(cx, |pane, cx| {
                let item = cx.new(|cx| item::test::TestItem::new(cx));
                pane.add_item(Box::new(item), false, false, None, window, cx);
            });
        });
        multi_workspace
    });
    let empty_workspace = window
        .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
        .unwrap();

    let opened = cx
        .update(|cx| {
            open_paths(
                &[PathBuf::from(path!("/project"))],
                app_state,
                OpenOptions::default(),
                cx,
            )
        })
        .await
        .unwrap();
    cx.run_until_parked();

    assert_eq!(opened.window, window);
    window
        .read_with(cx, |multi_workspace, _| {
            assert_eq!(
                multi_workspace.workspaces().cloned().collect::<Vec<_>>(),
                vec![opened.workspace.clone()],
                "the empty workspace has no tab, so it leaves the window"
            );
        })
        .unwrap();
    assert!(
        empty_workspace.read_with(cx, |workspace, _| workspace.session_id().is_none()),
        "the replaced empty workspace is detached from the session"
    );
}

#[gpui::test]
async fn test_open_directory_in_empty_workspace_does_not_open_sidebar(cx: &mut TestAppContext) {
    init_test(cx);

    let app_state = cx.update(AppState::test);
    let fs = app_state.fs.as_fake();
    fs.insert_tree(path!("/project"), json!({ "file.txt": "" }))
        .await;

    let project = Project::test(app_state.fs.clone(), [], cx).await;
    let window = cx.add_window(|window, cx| {
        let mw = MultiWorkspace::test_new(project, window, cx);
        // Simulate a blank project that has an untitled editor tab,
        // so that workspace_windows_for_location finds this window.
        mw.workspace().update(cx, |workspace, cx| {
            workspace.active_pane().update(cx, |pane, cx| {
                let item = cx.new(|cx| item::test::TestItem::new(cx));
                pane.add_item(Box::new(item), false, false, None, window, cx);
            });
        });
        mw
    });

    window
        .read_with(cx, |mw, _cx| {
            assert!(!mw.sidebar_open(), "sidebar should start closed");
        })
        .unwrap();

    // Simulate what open_workspace_for_paths does for an empty workspace:
    // it downgrades OpenMode::NewWindow to Activate and sets requesting_window.
    cx.update(|cx| {
        open_paths(
            &[PathBuf::from(path!("/project"))],
            app_state,
            OpenOptions {
                requesting_window: Some(window),
                open_mode: OpenMode::Activate,
                ..OpenOptions::default()
            },
            cx,
        )
    })
    .await
    .unwrap();

    window
        .read_with(cx, |mw, _cx| {
            assert!(
                !mw.sidebar_open(),
                "opening a directory in a blank project via the file picker must not open the sidebar",
            );
        })
        .unwrap();
}

#[gpui::test]
async fn test_project_group_keys_duplicate_not_added(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file.txt": "" })).await;
    let project_a = Project::test(fs.clone(), ["/root_a".as_ref()], cx).await;
    // A second project entity pointing at the same path produces the same key.
    let project_a2 = Project::test(fs.clone(), ["/root_a".as_ref()], cx).await;

    let key_a = project_a.read_with(cx, |p, cx| p.project_group_key(cx));
    let key_a2 = project_a2.read_with(cx, |p, cx| p.project_group_key(cx));
    assert_eq!(key_a, key_a2, "same root path should produce the same key");

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a, window, cx));

    multi_workspace.update(cx, |mw, cx| {
        mw.open_sidebar(cx);
    });

    multi_workspace.update_in(cx, |mw, window, cx| {
        mw.test_add_workspace(project_a2, window, cx);
    });

    multi_workspace.read_with(cx, |mw, _cx| {
        let keys: Vec<ProjectGroupKey> = mw.project_group_keys();
        assert_eq!(
            keys.len(),
            1,
            "duplicate key should not be added when a workspace with the same root is inserted"
        );
    });
}

#[gpui::test]
async fn test_adding_worktree_updates_project_group_key(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file.txt": "" })).await;
    fs.insert_tree("/root_b", json!({ "other.txt": "" })).await;
    let project = Project::test(fs.clone(), ["/root_a".as_ref()], cx).await;

    let initial_key = project.read_with(cx, |p, cx| p.project_group_key(cx));

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

    // Open sidebar to retain the workspace and create the initial group.
    multi_workspace.update(cx, |mw, cx| {
        mw.open_sidebar(cx);
    });
    cx.run_until_parked();

    multi_workspace.read_with(cx, |mw, _cx| {
        let keys = mw.project_group_keys();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0], initial_key);
    });

    // Add a second worktree to the project. This triggers WorktreeAdded →
    // handle_workspace_key_change, which should update the group key.
    project
        .update(cx, |project, cx| {
            project.find_or_create_worktree("/root_b", true, cx)
        })
        .await
        .expect("adding worktree should succeed");
    cx.run_until_parked();

    let updated_key = project.read_with(cx, |p, cx| p.project_group_key(cx));
    assert_ne!(
        initial_key, updated_key,
        "adding a worktree should change the project group key"
    );

    multi_workspace.read_with(cx, |mw, _cx| {
        let keys = mw.project_group_keys();
        assert!(
            keys.contains(&updated_key),
            "should contain the updated key; got {keys:?}"
        );
    });
}

#[gpui::test]
async fn test_find_or_create_local_workspace_reuses_active_workspace_when_sidebar_closed(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file.txt": "" })).await;
    let project = Project::test(fs, ["/root_a".as_ref()], cx).await;

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project, window, cx));

    let active_workspace = multi_workspace.read_with(cx, |mw, cx| {
        assert!(
            mw.project_groups(cx).is_empty(),
            "sidebar-closed setup should start with no retained project groups"
        );
        mw.workspace().clone()
    });
    let active_workspace_id = active_workspace.entity_id();

    let workspace = multi_workspace
        .update_in(cx, |mw, window, cx| {
            mw.find_or_create_local_workspace(
                PathList::new(&[PathBuf::from("/root_a")]),
                None,
                None,
                OpenMode::Activate,
                None,
                window,
                cx,
            )
        })
        .await
        .expect("reopening the same local workspace should succeed");

    assert_eq!(
        workspace.entity_id(),
        active_workspace_id,
        "should reuse the current active workspace when the sidebar is closed"
    );

    multi_workspace.read_with(cx, |mw, _cx| {
        assert_eq!(
            mw.workspace().entity_id(),
            active_workspace_id,
            "active workspace should remain unchanged after reopening the same path"
        );
        assert_eq!(
            mw.workspaces().count(),
            1,
            "reusing the active workspace should not create a second open workspace"
        );
    });
}

#[gpui::test]
async fn test_find_or_create_workspace_uses_project_group_key_when_paths_are_missing(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree(
        "/project",
        json!({
            ".git": {},
            "src": {},
        }),
    )
    .await;
    cx.update(|cx| <dyn Fs>::set_global(fs.clone(), cx));
    let project = Project::test(fs.clone(), ["/project".as_ref()], cx).await;
    project
        .update(cx, |project, cx| project.git_scans_complete(cx))
        .await;

    let project_group_key = project.read_with(cx, |project, cx| project.project_group_key(cx));

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project, window, cx));

    let main_workspace = multi_workspace.read_with(cx, |mw, _cx| mw.workspace().clone());
    let main_workspace_id = main_workspace.entity_id();

    let workspace = multi_workspace
        .update_in(cx, |mw, window, cx| {
            mw.find_or_create_workspace(
                PathList::new(&[PathBuf::from("/wt-feature-a")]),
                Some(project_group_key.clone()),
                None,
                OpenMode::Activate,
                None,
                window,
                cx,
            )
        })
        .await
        .expect("opening a missing linked-worktree path should fall back to the project group key workspace");

    assert_eq!(
        workspace.entity_id(),
        main_workspace_id,
        "missing linked-worktree paths should reuse the main worktree workspace from the project group key"
    );

    multi_workspace.read_with(cx, |mw, cx| {
        assert_eq!(
            mw.workspace().entity_id(),
            main_workspace_id,
            "the active workspace should remain the main worktree workspace"
        );
        assert_eq!(
            PathList::new(&mw.workspace().read(cx).root_paths(cx)),
            project_group_key.path_list().clone(),
            "the activated workspace should use the project group key path list rather than the missing linked-worktree path"
        );
        assert_eq!(
            mw.workspaces().count(),
            1,
            "falling back to the project group key should not create a second workspace"
        );
    });
}

#[gpui::test]
async fn test_remove_fallback_via_find_or_create_skips_removed_workspaces(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file.txt": "" })).await;
    let project_a = Project::test(fs.clone(), ["/root_a".as_ref()], cx).await;
    let project_b = Project::test(fs, ["/root_a".as_ref()], cx).await;

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a, window, cx));

    let workspace_a = multi_workspace.read_with(cx, |mw, _cx| mw.workspace().clone());
    let workspace_b = multi_workspace.update_in(cx, |mw, window, cx| {
        mw.test_add_workspace(project_b, window, cx)
    });
    cx.run_until_parked();

    multi_workspace.update_in(cx, |mw, window, cx| {
        mw.activate(workspace_a.clone(), None, window, cx);
    });

    let removed = multi_workspace
        .update_in(cx, |mw, window, cx| {
            mw.remove(
                vec![workspace_a.clone()],
                RemovalIntent::CloseProject,
                window,
                cx,
            )
        })
        .await
        .expect("removing the active workspace should succeed");
    assert!(removed, "the workspace should have been removed");

    multi_workspace.read_with(cx, |mw, _cx| {
        assert_eq!(
            mw.workspace().entity_id(),
            workspace_b.entity_id(),
            "the non-excluded workspace should become active"
        );
        assert!(
            mw.workspaces()
                .all(|workspace| workspace.entity_id() != workspace_a.entity_id()),
            "the removed workspace should be gone"
        );
    });
}

#[gpui::test]
async fn test_remove_keeping_the_project_does_not_switch_projects(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file.txt": "" })).await;
    fs.insert_tree("/root_b", json!({ "file.txt": "" })).await;
    cx.update(|cx| <dyn Fs>::set_global(fs.clone(), cx));
    let project_a = Project::test(fs.clone(), ["/root_a".as_ref()], cx).await;
    let project_b = Project::test(fs, ["/root_b".as_ref()], cx).await;

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a, window, cx));

    let workspace_a = multi_workspace.read_with(cx, |mw, _cx| mw.workspace().clone());
    let _workspace_b = multi_workspace.update_in(cx, |mw, window, cx| {
        mw.test_add_workspace(project_b, window, cx)
    });
    cx.run_until_parked();

    multi_workspace.update_in(cx, |mw, window, cx| {
        mw.activate(workspace_a.clone(), None, window, cx);
    });
    cx.run_until_parked();

    multi_workspace
        .update_in(cx, |mw, window, cx| {
            mw.remove(
                vec![workspace_a.clone()],
                RemovalIntent::KeepProject,
                window,
                cx,
            )
        })
        .await
        .expect("removing the active workspace should succeed");
    cx.run_until_parked();

    multi_workspace.read_with(cx, |mw, cx| {
        assert_eq!(
            PathList::new(&mw.workspace().read(cx).root_paths(cx)),
            PathList::new(&[PathBuf::from("/root_a")]),
            "the replacement workspace should be in the removed workspace's project"
        );
    });
}

#[gpui::test]
async fn test_find_or_create_local_workspace_reuses_active_workspace_after_sidebar_open(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file.txt": "" })).await;
    let project = Project::test(fs, ["/root_a".as_ref()], cx).await;

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project, window, cx));

    multi_workspace.update(cx, |mw, cx| {
        mw.open_sidebar(cx);
    });
    cx.run_until_parked();

    let active_workspace = multi_workspace.read_with(cx, |mw, cx| {
        assert_eq!(
            mw.project_groups(cx).len(),
            1,
            "opening the sidebar should retain the active workspace in a project group"
        );
        mw.workspace().clone()
    });
    let active_workspace_id = active_workspace.entity_id();

    let workspace = multi_workspace
        .update_in(cx, |mw, window, cx| {
            mw.find_or_create_local_workspace(
                PathList::new(&[PathBuf::from("/root_a")]),
                None,
                None,
                OpenMode::Activate,
                None,
                window,
                cx,
            )
        })
        .await
        .expect("reopening the same retained local workspace should succeed");

    assert_eq!(
        workspace.entity_id(),
        active_workspace_id,
        "should reuse the retained active workspace after the sidebar is opened"
    );

    multi_workspace.read_with(cx, |mw, _cx| {
        assert_eq!(
            mw.workspaces().count(),
            1,
            "reopening the same retained workspace should not create another workspace"
        );
    });
}

#[gpui::test]
async fn test_close_workspace_prefers_already_loaded_neighboring_workspace(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file_a.txt": "" })).await;
    fs.insert_tree("/root_b", json!({ "file_b.txt": "" })).await;
    fs.insert_tree("/root_c", json!({ "file_c.txt": "" })).await;
    let project_a = Project::test(fs.clone(), ["/root_a".as_ref()], cx).await;
    let project_b = Project::test(fs.clone(), ["/root_b".as_ref()], cx).await;
    let project_b_key = project_b.read_with(cx, |project, cx| project.project_group_key(cx));
    let project_c = Project::test(fs, ["/root_c".as_ref()], cx).await;
    let project_c_key = project_c.read_with(cx, |project, cx| project.project_group_key(cx));

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a, window, cx));

    multi_workspace.update(cx, |multi_workspace, cx| {
        multi_workspace.open_sidebar(cx);
    });
    cx.run_until_parked();

    let workspace_a = multi_workspace.read_with(cx, |multi_workspace, _cx| {
        multi_workspace.workspace().clone()
    });
    let workspace_b = multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.test_add_workspace(project_b, window, cx)
    });

    multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.activate(workspace_a.clone(), None, window, cx);
        multi_workspace.test_add_project_group(ProjectGroup {
            key: project_c_key.clone(),
            workspaces: Vec::new(),
            expanded: true,
        });
    });

    multi_workspace.read_with(cx, |multi_workspace, _cx| {
        let keys = multi_workspace.project_group_keys();
        assert_eq!(
            keys.len(),
            3,
            "expected three project groups in the test setup"
        );
        assert_eq!(keys[0], project_b_key);
        assert_eq!(
            keys[1],
            workspace_a.read_with(cx, |workspace, cx| { workspace.project_group_key(cx) })
        );
        assert_eq!(keys[2], project_c_key);
        assert_eq!(
            multi_workspace.workspace().entity_id(),
            workspace_a.entity_id(),
            "workspace A should be active before closing"
        );
    });

    let closed = multi_workspace
        .update_in(cx, |multi_workspace, window, cx| {
            multi_workspace.remove(
                [workspace_a.clone()],
                RemovalIntent::CloseProject,
                window,
                cx,
            )
        })
        .await
        .expect("closing the active workspace should succeed");

    assert!(
        closed,
        "close_workspace should report that it removed a workspace"
    );

    multi_workspace.read_with(cx, |multi_workspace, cx| {
        assert_eq!(
            multi_workspace.workspace().entity_id(),
            workspace_b.entity_id(),
            "closing workspace A should activate the already-loaded workspace B instead of opening group C"
        );
        assert_eq!(
            multi_workspace.workspaces().count(),
            1,
            "only workspace B should remain loaded after closing workspace A"
        );
        assert!(
            multi_workspace
                .workspaces_for_project_group(&project_c_key, cx)
                .is_empty(),
            "the unloaded neighboring group C should remain unopened"
        );
    });
}

#[gpui::test]
async fn test_close_workspace_prefers_workspace_in_same_project_group(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/project-a", json!({})).await;
    fs.insert_tree("/project-b", json!({})).await;

    let project_a_1 = Project::test(fs.clone(), ["/project-a".as_ref()], cx).await;
    let project_a_2 = Project::test(fs.clone(), ["/project-a".as_ref()], cx).await;
    let project_b = Project::test(fs, ["/project-b".as_ref()], cx).await;
    let key_a = project_a_1.read_with(cx, |project, cx| project.project_group_key(cx));
    let (multi_workspace, cx) = setup_multi_workspace(&[project_a_1, project_a_2, project_b], cx);

    let (workspace_a_1, workspace_a_2) = multi_workspace.read_with(cx, |multi_workspace, cx| {
        let mut workspaces = multi_workspace
            .workspaces_for_project_group(&key_a, cx)
            .into_iter();
        let first = workspaces
            .next()
            .expect("project group A should have a first workspace");
        let second = workspaces
            .next()
            .expect("project group A should have a second workspace");

        assert!(
            workspaces.next().is_none(),
            "project group A should have exactly two workspaces"
        );

        (first, second)
    });

    multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.activate(workspace_a_1.clone(), None, window, cx);
    });

    let closed = multi_workspace
        .update_in(cx, |multi_workspace, window, cx| {
            multi_workspace.remove(
                [workspace_a_1.clone()],
                RemovalIntent::CloseProject,
                window,
                cx,
            )
        })
        .await
        .expect("closing the active workspace should succeed");

    assert!(closed, "close_workspace should remove the active workspace");
    multi_workspace.read_with(cx, |multi_workspace, cx| {
        assert_eq!(
            multi_workspace.workspace(),
            &workspace_a_2,
            "the second workspace for project group a should be preferred"
        );

        assert_eq!(
            multi_workspace.workspaces_for_project_group(&key_a, cx),
            vec![workspace_a_2],
            "only the fallback workspace should remain in project group A"
        );
    });
}

#[gpui::test]
async fn test_close_workspace_opens_unloaded_local_neighbor(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/project-a", json!({})).await;
    fs.insert_tree("/project-b", json!({})).await;
    cx.update(|cx| <dyn Fs>::set_global(fs.clone(), cx));

    let project_a = Project::test(fs.clone(), ["/project-a".as_ref()], cx).await;
    let project_b = Project::test(fs, ["/project-b".as_ref()], cx).await;
    let key_b = project_b.read_with(cx, |project, cx| project.project_group_key(cx));
    let (multi_workspace, cx) = setup_multi_workspace(&[project_a], cx);
    let workspace_a = multi_workspace.read_with(cx, |multi_workspace, _cx| {
        multi_workspace.workspace().clone()
    });

    multi_workspace.update(cx, |multi_workspace, _cx| {
        multi_workspace.test_add_project_group(ProjectGroup {
            key: key_b.clone(),
            workspaces: Vec::new(),
            expanded: true,
        });
    });

    let closed = multi_workspace
        .update_in(cx, |multi_workspace, window, cx| {
            multi_workspace.remove(
                [workspace_a.clone()],
                RemovalIntent::CloseProject,
                window,
                cx,
            )
        })
        .await
        .expect("closing the active workspace should succeed");

    assert!(closed, "close_workspace should remove the active workspace");
    multi_workspace.read_with(cx, |multi_workspace, cx| {
        assert_eq!(
            multi_workspace.workspace().read(cx).project_group_key(cx),
            key_b,
            "the unloaded local neighboring group should be opened"
        );
    });
}

#[gpui::test]
async fn test_remove_project_group_opens_unloaded_local_neighbor(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/project-a", json!({})).await;
    fs.insert_tree("/project-b", json!({})).await;
    cx.update(|cx| <dyn Fs>::set_global(fs.clone(), cx));

    let project_a = Project::test(fs.clone(), ["/project-a".as_ref()], cx).await;
    let project_b = Project::test(fs, ["/project-b".as_ref()], cx).await;
    let key_a = project_a.read_with(cx, |project, cx| project.project_group_key(cx));
    let key_b = project_b.read_with(cx, |project, cx| project.project_group_key(cx));
    let (multi_workspace, cx) = setup_multi_workspace(&[project_a], cx);

    multi_workspace.update(cx, |multi_workspace, _cx| {
        multi_workspace.test_add_project_group(ProjectGroup {
            key: key_b.clone(),
            workspaces: Vec::new(),
            expanded: true,
        });
    });

    let removed = multi_workspace
        .update_in(cx, |multi_workspace, window, cx| {
            multi_workspace.remove_project_group(&key_a, window, cx)
        })
        .await
        .expect("removing the active project group should succeed");

    assert!(
        removed,
        "remove_project_group should remove the active group"
    );

    multi_workspace.read_with(cx, |multi_workspace, cx| {
        assert_eq!(
            multi_workspace.workspace().read(cx).project_group_key(cx),
            key_b,
            "the unloaded local neighboring group should be opened"
        );
    });
}

#[gpui::test]
async fn test_switching_projects_with_sidebar_closed_retains_old_active_workspace(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file_a.txt": "" })).await;
    fs.insert_tree("/root_b", json!({ "file_b.txt": "" })).await;
    let project_a = Project::test(fs.clone(), ["/root_a".as_ref()], cx).await;
    let project_b = Project::test(fs, ["/root_b".as_ref()], cx).await;

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a, window, cx));

    let workspace_a = multi_workspace.read_with(cx, |mw, cx| {
        assert!(
            mw.project_groups(cx).is_empty(),
            "sidebar-closed setup should start with no retained project groups"
        );
        mw.workspace().clone()
    });
    assert!(
        workspace_a.read_with(cx, |workspace, _cx| workspace.session_id().is_some()),
        "initial active workspace should start attached to the session"
    );

    let workspace_b = multi_workspace.update_in(cx, |mw, window, cx| {
        mw.test_add_workspace(project_b, window, cx)
    });
    cx.run_until_parked();

    multi_workspace.read_with(cx, |mw, cx| {
        assert_eq!(
            mw.workspace().entity_id(),
            workspace_b.entity_id(),
            "the new workspace should become active"
        );
        assert_eq!(
            mw.workspaces().count(),
            2,
            "the previous active workspace should remain open after switching with the sidebar closed"
        );
        assert_eq!(mw.project_groups(cx).len(), 2);
    });

    assert!(
        workspace_a.read_with(cx, |workspace, _cx| workspace.session_id().is_some()),
        "the previous active workspace should remain attached when switching away with the sidebar closed"
    );
}

#[gpui::test]
async fn test_open_project_closes_empty_workspace_but_not_non_empty_ones(cx: &mut TestAppContext) {
    init_test(cx);
    let app_state = cx.update(AppState::test);
    let fs = app_state.fs.as_fake();
    fs.insert_tree(path!("/project_a"), json!({ "file_a.txt": "" }))
        .await;
    fs.insert_tree(path!("/project_b"), json!({ "file_b.txt": "" }))
        .await;

    // Start with an empty (no-worktrees) workspace.
    let project = Project::test(app_state.fs.clone(), [], cx).await;
    let window = cx.add_window(|window, cx| MultiWorkspace::test_new(project, window, cx));
    cx.run_until_parked();

    window
        .update(cx, |mw, _window, cx| mw.open_sidebar(cx))
        .unwrap();
    cx.run_until_parked();

    let empty_workspace = window
        .read_with(cx, |mw, _| mw.workspace().clone())
        .unwrap();
    let cx = &mut VisualTestContext::from_window(window.into(), cx);

    // Add a dirty untitled item to the empty workspace.
    let dirty_item = cx.new(|cx| TestItem::new(cx).with_dirty(true));
    empty_workspace.update_in(cx, |workspace, window, cx| {
        workspace.add_item_to_active_pane(Box::new(dirty_item.clone()), None, true, window, cx);
    });

    // Opening a project while the lone empty workspace has unsaved
    // changes prompts the user.
    let open_task = window
        .update(cx, |mw, window, cx| {
            mw.open_project(
                vec![PathBuf::from(path!("/project_a"))],
                OpenMode::Activate,
                window,
                cx,
            )
        })
        .unwrap();
    cx.run_until_parked();

    // Cancelling keeps the empty workspace.
    assert!(cx.has_pending_prompt(),);
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(open_task.await.unwrap(), empty_workspace);
    window
        .read_with(cx, |mw, _cx| {
            assert_eq!(mw.workspaces().count(), 1);
            assert_eq!(mw.workspace(), &empty_workspace);
            assert_eq!(mw.project_group_keys(), vec![]);
        })
        .unwrap();

    // Discarding the unsaved changes closes the empty workspace
    // and opens the new project in its place.
    let open_task = window
        .update(cx, |mw, window, cx| {
            mw.open_project(
                vec![PathBuf::from(path!("/project_a"))],
                OpenMode::Activate,
                window,
                cx,
            )
        })
        .unwrap();
    cx.run_until_parked();

    assert!(cx.has_pending_prompt(),);
    cx.simulate_prompt_answer("Don't Save");
    cx.run_until_parked();

    let workspace_a = open_task.await.unwrap();
    assert_ne!(workspace_a, empty_workspace);

    window
        .read_with(cx, |mw, _cx| {
            assert_eq!(mw.workspaces().count(), 1);
            assert_eq!(mw.workspace(), &workspace_a);
            assert_eq!(
                mw.project_group_keys(),
                vec![ProjectGroupKey::new(PathList::new(&[path!("/project_a")]))]
            );
        })
        .unwrap();
    assert!(
        empty_workspace.read_with(cx, |workspace, _cx| workspace.session_id().is_none()),
        "the detached empty workspace should no longer be attached to the session",
    );

    let dirty_item = cx.new(|cx| TestItem::new(cx).with_dirty(true));
    workspace_a.update_in(cx, |workspace, window, cx| {
        workspace.add_item_to_active_pane(Box::new(dirty_item.clone()), None, true, window, cx);
    });

    // Opening another project does not close the existing project or prompt.
    let workspace_b = window
        .update(cx, |mw, window, cx| {
            mw.open_project(
                vec![PathBuf::from(path!("/project_b"))],
                OpenMode::Activate,
                window,
                cx,
            )
        })
        .unwrap()
        .await
        .unwrap();
    cx.run_until_parked();

    assert!(!cx.has_pending_prompt());
    assert_ne!(workspace_b, workspace_a);
    window
        .read_with(cx, |mw, _cx| {
            assert_eq!(mw.workspaces().count(), 2);
            assert_eq!(mw.workspace(), &workspace_b);
            assert_eq!(
                mw.project_group_keys(),
                vec![
                    ProjectGroupKey::new(PathList::new(&[path!("/project_b")])),
                    ProjectGroupKey::new(PathList::new(&[path!("/project_a")]))
                ]
            );
        })
        .unwrap();
    assert!(workspace_a.read_with(cx, |workspace, _cx| workspace.session_id().is_some()),);
}

#[gpui::test]
async fn test_nearest_retained_workspace(cx: &mut TestAppContext) {
    init_test(cx);

    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/project-a", json!({})).await;
    fs.insert_tree("/project-b", json!({})).await;
    fs.insert_tree("/project-c", json!({})).await;
    fs.insert_tree("/project-d", json!({})).await;

    // These two projects create separate workspaces in the same project group. The second
    // workspace is activated after the first, making it the group's last active workspace.
    let project_a_1 = Project::test(fs.clone(), ["/project-a".as_ref()], cx).await;
    let project_a_2 = Project::test(fs.clone(), ["/project-a".as_ref()], cx).await;
    let project_b = Project::test(fs.clone(), ["/project-b".as_ref()], cx).await;
    let project_c = Project::test(fs.clone(), ["/project-c".as_ref()], cx).await;
    let project_d = Project::test(fs, ["/project-d".as_ref()], cx).await;
    let key_a = project_a_1.read_with(cx, |project, cx| project.project_group_key(cx));
    let key_b = project_b.read_with(cx, |project, cx| project.project_group_key(cx));
    let key_c = project_c.read_with(cx, |project, cx| project.project_group_key(cx));
    let key_d = project_d.read_with(cx, |project, cx| project.project_group_key(cx));

    let (multi_workspace, cx) = setup_multi_workspace(
        &[project_a_1, project_a_2, project_b, project_c, project_d],
        cx,
    );

    multi_workspace.update(cx, |multi_workspace, cx| {
        assert_eq!(
            multi_workspace.project_group_keys(),
            vec![key_d.clone(), key_c.clone(), key_b.clone(), key_a.clone()],
            "new project groups should be inserted before existing groups"
        );

        let group_c_index = multi_workspace
            .project_groups(cx)
            .iter()
            .position(|project_group| project_group.key == key_c)
            .expect("project group for project-c should exist");
        let workspace_b = multi_workspace
            .workspaces_for_project_group(&key_b, cx)
            .into_iter()
            .next()
            .expect("workspace for project-b should exist");
        let workspace_d = multi_workspace
            .workspaces_for_project_group(&key_d, cx)
            .into_iter()
            .next()
            .expect("workspace for project-d should exist");
        let retained_workspaces_a = multi_workspace
            .workspaces_for_project_group(&key_a, cx);
        assert_eq!(
            retained_workspaces_a.len(),
            2,
            "project group A should retain both workspaces"
        );
        let workspace_a_1 = retained_workspaces_a
            .first()
            .expect("project group A should have a retained workspace")
            .clone();
        let workspace_a_2 = multi_workspace
            .last_active_workspace_for_group(&key_a, cx)
            .expect("project group A should have a last active workspace");
        assert_ne!(
            workspace_a_1, workspace_a_2,
            "project group A's last active workspace should differ from its first retained workspace"
        );

        // Since Project Group B is the one after C, it is preferred over
        // Project Group D, even if they're at the same distance.
        assert_eq!(
            multi_workspace.nearest_retained_workspace(group_c_index, &[], cx),
            Some(workspace_b.clone()),
            "the following project group should be preferred at equal distance"
        );

        // With Project Group B being excluded, Project Group D is picked as it
        // is the one with the smallest distance.
        assert_eq!(
            multi_workspace.nearest_retained_workspace(
                group_c_index,
                std::slice::from_ref(&workspace_b),
                cx,
            ),
            Some(workspace_d.clone()),
            "the preceding project group should be used when the following workspace is excluded"
        );

        // With both adjacent Project Groups excluded, the search expands and
        // reaches Project A at distance 2 and prefers its last active workspace (A2)
        // over its first retained workspace (A1).
        assert_eq!(
            multi_workspace.nearest_retained_workspace(
                group_c_index,
                &[workspace_b.clone(), workspace_d.clone()],
                cx
            ),
            Some(workspace_a_2.clone()),
            "the farther group's last active workspace should be preferred"
        );

        // With the group's most recently activated workspace excluded, the
        // search falls back to the member activated before it.
        assert_eq!(
            multi_workspace.nearest_retained_workspace(
                group_c_index,
                &[
                    workspace_b.clone(),
                    workspace_d.clone(),
                    workspace_a_2.clone()
                ],
                cx
            ),
            Some(workspace_a_1.clone()),
            "the previously activated workspace should be used when the last active one is excluded"
        );

        // Excluding every neighboring workspace exhausts the search.
        assert_eq!(
            multi_workspace.nearest_retained_workspace(
                group_c_index,
                &[
                    workspace_b,
                    workspace_d,
                    workspace_a_1,
                    workspace_a_2,
                ],
                cx
            ),
            None,
            "no workspace should be returned when every candidate is excluded"
        );
    });
}

fn set_default_open_behavior(cx: &mut TestAppContext, behavior: settings::DefaultOpenBehavior) {
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, cx| {
            store.update_user_settings(cx, |settings| {
                settings.workspace.default_open_behavior = Some(behavior);
            });
        });
    });
}

#[gpui::test]
async fn test_switching_projects_without_a_sidebar_keeps_both_in_the_window(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file_a.txt": "" })).await;
    fs.insert_tree("/root_b", json!({ "file_b.txt": "" })).await;
    let project_a = Project::test(fs.clone(), ["/root_a".as_ref()], cx).await;
    let project_b = Project::test(fs, ["/root_b".as_ref()], cx).await;

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a, window, cx));
    let workspace_a = multi_workspace.read_with(cx, |multi_workspace, cx| {
        assert!(
            multi_workspace.sidebar().is_none(),
            "Flint ships no sidebar, so the window under test has none either"
        );
        assert!(
            multi_workspace.retains_workspaces(cx),
            "the default `existing_window` behaviour keeps every project in the window"
        );
        multi_workspace.workspace().clone()
    });

    let workspace_b = multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.test_add_workspace(project_b, window, cx)
    });
    cx.run_until_parked();
    multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.activate(workspace_a.clone(), None, window, cx);
    });
    cx.run_until_parked();

    multi_workspace.read_with(cx, |multi_workspace, _cx| {
        assert_eq!(
            multi_workspace.workspaces().cloned().collect::<Vec<_>>(),
            vec![workspace_a.clone(), workspace_b.clone()],
            "switching projects keeps both of them in the window"
        );
        assert_eq!(multi_workspace.workspace(), &workspace_a);
    });
    for workspace in [&workspace_a, &workspace_b] {
        assert!(
            workspace.read_with(cx, |workspace, _cx| workspace.session_id().is_some()),
            "a project kept in the window stays part of the session"
        );
    }
}

#[gpui::test]
async fn test_new_window_setting_replaces_the_project_but_never_drops_unsaved_edits(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    set_default_open_behavior(cx, settings::DefaultOpenBehavior::NewWindow);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file_a.txt": "" })).await;
    fs.insert_tree("/root_b", json!({ "file_b.txt": "" })).await;
    fs.insert_tree("/root_c", json!({ "file_c.txt": "" })).await;
    let project_a = Project::test(fs.clone(), ["/root_a".as_ref()], cx).await;
    let project_b = Project::test(fs.clone(), ["/root_b".as_ref()], cx).await;
    let project_c = Project::test(fs, ["/root_c".as_ref()], cx).await;

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a, window, cx));
    let workspace_a = multi_workspace.read_with(cx, |multi_workspace, cx| {
        assert!(!multi_workspace.retains_workspaces(cx));
        multi_workspace.workspace().clone()
    });

    let workspace_b = multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.test_add_workspace(project_b, window, cx)
    });
    cx.run_until_parked();
    multi_workspace.read_with(cx, |multi_workspace, _cx| {
        assert_eq!(
            multi_workspace.workspaces().cloned().collect::<Vec<_>>(),
            vec![workspace_b.clone()],
            "with `new_window` a project without unsaved edits is replaced, as before"
        );
    });
    assert!(workspace_a.read_with(cx, |workspace, _cx| workspace.session_id().is_none()));

    let dirty_item = cx.new(|cx| TestItem::new(cx).with_dirty(true));
    workspace_b.update_in(cx, |workspace, window, cx| {
        workspace.add_item_to_active_pane(Box::new(dirty_item), None, true, window, cx);
    });
    let workspace_c = multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.test_add_workspace(project_c, window, cx)
    });
    cx.run_until_parked();

    multi_workspace.read_with(cx, |multi_workspace, _cx| {
        assert_eq!(
            multi_workspace.workspaces().cloned().collect::<Vec<_>>(),
            vec![workspace_b.clone(), workspace_c.clone()],
            "a project with unsaved edits is never dropped without a save prompt"
        );
        assert_eq!(multi_workspace.workspace(), &workspace_c);
    });
    assert!(workspace_b.read_with(cx, |workspace, _cx| workspace.session_id().is_some()));
}

#[gpui::test]
async fn test_open_project_focuses_a_project_open_in_another_window(cx: &mut TestAppContext) {
    init_test(cx);
    let app_state = cx.update(AppState::test);
    let fs = app_state.fs.as_fake();
    fs.insert_tree(path!("/project_a"), json!({ "file_a.txt": "" }))
        .await;
    fs.insert_tree(path!("/project_b"), json!({ "file_b.txt": "" }))
        .await;
    let project_a = Project::test(app_state.fs.clone(), [path!("/project_a").as_ref()], cx).await;
    let project_b = Project::test(app_state.fs.clone(), [path!("/project_b").as_ref()], cx).await;

    let window_a = cx.add_window(|window, cx| MultiWorkspace::test_new(project_a, window, cx));
    let window_b = cx.add_window(|window, cx| MultiWorkspace::test_new(project_b, window, cx));
    cx.run_until_parked();
    let workspace_a = window_a
        .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
        .unwrap();

    let opened = window_b
        .update(cx, |multi_workspace, window, cx| {
            multi_workspace.open_project(
                vec![PathBuf::from(path!("/project_a"))],
                OpenMode::Activate,
                window,
                cx,
            )
        })
        .unwrap()
        .await
        .unwrap();
    cx.run_until_parked();

    assert_eq!(
        opened, workspace_a,
        "a project open in another window is focused there instead of opened twice"
    );
    window_b
        .read_with(cx, |multi_workspace, _| {
            assert_eq!(multi_workspace.workspaces().count(), 1);
        })
        .unwrap();
    assert_eq!(cx.update(|cx| cx.windows().len()), 2);
}

#[gpui::test]
async fn test_next_and_previous_project_cycle_the_project_tabs(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file_a.txt": "" })).await;
    fs.insert_tree("/root_b", json!({ "file_b.txt": "" })).await;
    fs.insert_tree("/root_c", json!({ "file_c.txt": "" })).await;
    let project_a = Project::test(fs.clone(), ["/root_a".as_ref()], cx).await;
    let project_b = Project::test(fs.clone(), ["/root_b".as_ref()], cx).await;
    let project_c = Project::test(fs, ["/root_c".as_ref()], cx).await;

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a, window, cx));
    let workspace_a =
        multi_workspace.read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone());
    let workspace_b = multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.test_add_workspace(project_b, window, cx)
    });
    let workspace_c = multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.test_add_workspace(project_c, window, cx)
    });
    cx.run_until_parked();

    for (action, expected) in [
        (true, &workspace_a),
        (true, &workspace_b),
        (false, &workspace_a),
        (false, &workspace_c),
    ] {
        if action {
            cx.dispatch_action(NextProject);
        } else {
            cx.dispatch_action(PreviousProject);
        }
        cx.run_until_parked();
        multi_workspace.read_with(cx, |multi_workspace, _| {
            assert_eq!(
                multi_workspace.workspace(),
                expected,
                "Next and Previous Project walk the project tabs in order and wrap around"
            );
        });
    }
}

#[gpui::test]
async fn test_closing_a_project_tab_activates_its_neighbour_and_forgets_the_project(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file_a.txt": "" })).await;
    fs.insert_tree("/root_b", json!({ "file_b.txt": "" })).await;
    fs.insert_tree("/root_c", json!({ "file_c.txt": "" })).await;
    cx.update(|cx| <dyn Fs>::set_global(fs.clone(), cx));
    let project_a = Project::test(fs.clone(), ["/root_a".as_ref()], cx).await;
    let project_b = Project::test(fs.clone(), ["/root_b".as_ref()], cx).await;
    let project_c = Project::test(fs, ["/root_c".as_ref()], cx).await;
    let key_b = project_b.read_with(cx, |project, cx| project.project_group_key(cx));

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a, window, cx));
    let workspace_a =
        multi_workspace.read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone());
    let workspace_b = multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.test_add_workspace(project_b, window, cx)
    });
    let workspace_c = multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.test_add_workspace(project_c, window, cx)
    });
    multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.activate(workspace_b.clone(), None, window, cx);
    });
    cx.run_until_parked();

    let close = |workspace: &Entity<Workspace>, cx: &mut VisualTestContext| {
        multi_workspace.update_in(cx, |multi_workspace, window, cx| {
            multi_workspace.close_workspace(workspace, window, cx)
        })
    };

    assert!(close(&workspace_b, cx).await.unwrap());
    cx.run_until_parked();
    multi_workspace.read_with(cx, |multi_workspace, _| {
        assert_eq!(
            multi_workspace.workspaces().cloned().collect::<Vec<_>>(),
            vec![workspace_a.clone(), workspace_c.clone()],
            "closing a tab closes only that project"
        );
        assert_eq!(
            multi_workspace.workspace(),
            &workspace_c,
            "the tab after the closed one becomes active"
        );
        assert!(
            !multi_workspace.project_group_keys().contains(&key_b),
            "a closed project is not reopened by a later close"
        );
    });

    assert!(close(&workspace_c, cx).await.unwrap());
    cx.run_until_parked();
    multi_workspace.read_with(cx, |multi_workspace, _| {
        assert_eq!(
            multi_workspace.workspace(),
            &workspace_a,
            "closing the last tab activates the tab before it"
        );
    });

    assert!(close(&workspace_a, cx).await.unwrap());
    cx.run_until_parked();
    multi_workspace.read_with(cx, |multi_workspace, cx| {
        assert_eq!(multi_workspace.workspaces().count(), 1);
        assert_ne!(multi_workspace.workspace(), &workspace_a);
        assert!(
            multi_workspace.project_tabs(cx).is_empty(),
            "closing the last project leaves the empty welcome workspace"
        );
        assert!(multi_workspace.project_group_keys().is_empty());
    });
}

#[gpui::test]
async fn test_closing_a_project_tab_stops_only_its_language_servers(cx: &mut TestAppContext) {
    use futures::{StreamExt as _, future};

    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree(path!("/root_a"), json!({ "a.rs": "fn a() {}" }))
        .await;
    fs.insert_tree(path!("/root_b"), json!({ "b.rs": "fn b() {}" }))
        .await;
    cx.update(|cx| <dyn Fs>::set_global(fs.clone(), cx));
    let project_a = Project::test(fs.clone(), [path!("/root_a").as_ref()], cx).await;
    let project_b = Project::test(fs, [path!("/root_b").as_ref()], cx).await;
    let register_fake_server = |project: &Entity<Project>, cx: &mut TestAppContext| {
        let language_registry = project.read_with(cx, |project, _| project.languages().clone());
        language_registry.add(language::rust_lang());
        language_registry.register_fake_lsp("Rust", language::FakeLspAdapter::default())
    };
    let mut fake_servers_a = register_fake_server(&project_a, cx);
    let mut fake_servers_b = register_fake_server(&project_b, cx);

    let (buffer_a, _handle_a) = project_a
        .update(cx, |project, cx| {
            project.open_local_buffer_with_lsp(path!("/root_a/a.rs"), cx)
        })
        .await
        .unwrap();
    let mut fake_server_a = fake_servers_a.next().await.unwrap();
    let (buffer_b, handle_b) = project_b
        .update(cx, |project, cx| {
            project.open_local_buffer_with_lsp(path!("/root_b/b.rs"), cx)
        })
        .await
        .unwrap();
    let mut fake_server_b = fake_servers_b.next().await.unwrap();
    fake_server_b.set_request_handler::<lsp::request::Shutdown, _, _>(|_, _| future::ready(Ok(())));

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a.clone(), window, cx));
    let workspace_b = multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.test_add_workspace(project_b.clone(), window, cx)
    });
    multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.activate(workspace_b.clone(), None, window, cx);
    });
    cx.run_until_parked();

    fake_server_a.notify::<lsp::notification::PublishDiagnostics>(lsp::PublishDiagnosticsParams {
        uri: lsp::Uri::from_file_path(path!("/root_a/a.rs")).unwrap(),
        version: None,
        diagnostics: vec![lsp::Diagnostic {
            range: lsp::Range::new(lsp::Position::new(0, 3), lsp::Position::new(0, 4)),
            severity: Some(lsp::DiagnosticSeverity::ERROR),
            message: lsp::DiagnosticMessage::String("background error".to_string()),
            ..Default::default()
        }],
    });
    cx.run_until_parked();
    assert_eq!(
        project_a.read_with(cx, |project, cx| project
            .diagnostic_summary(false, cx)
            .error_count),
        1,
        "the project in the background tab keeps receiving its server's diagnostics"
    );

    let lsp_store_b = project_b.read_with(cx, |project, _| project.lsp_store().downgrade());
    let weak_project_b = project_b.downgrade();
    drop((project_b, buffer_b, handle_b));
    let close = multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.close_workspace(&workspace_b, window, cx)
    });
    assert!(close.await.unwrap());
    drop(workspace_b);
    // Dropped entities are released at the end of the next effect cycle.
    cx.update(|_, _| {});
    cx.run_until_parked();

    weak_project_b.assert_released();
    lsp_store_b.assert_released();
    fake_server_b
        .receive_notification::<lsp::notification::Exit>()
        .await;

    buffer_a.update(cx, |buffer, cx| buffer.edit([(0..0, "// ")], None, cx));
    assert_eq!(
        fake_server_a
            .receive_notification::<lsp::notification::DidChangeTextDocument>()
            .await
            .text_document
            .uri,
        lsp::Uri::from_file_path(path!("/root_a/a.rs")).unwrap(),
        "the remaining project's server keeps running"
    );
}

#[gpui::test]
async fn test_closing_a_project_tab_with_unsaved_edits_asks_first(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file_a.txt": "" })).await;
    fs.insert_tree("/root_b", json!({ "file_b.txt": "" })).await;
    let project_a = Project::test(fs.clone(), ["/root_a".as_ref()], cx).await;
    let project_b = Project::test(fs, ["/root_b".as_ref()], cx).await;

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a, window, cx));
    let workspace_b = multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.test_add_workspace(project_b, window, cx)
    });
    let dirty_item = cx.new(|cx| TestItem::new(cx).with_dirty(true));
    workspace_b.update_in(cx, |workspace, window, cx| {
        workspace.add_item_to_active_pane(Box::new(dirty_item), None, true, window, cx);
    });
    cx.run_until_parked();

    let close_task = multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.close_workspace(&workspace_b, window, cx)
    });
    cx.run_until_parked();
    assert!(cx.has_pending_prompt(), "unsaved edits ask before closing");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();

    assert!(!close_task.await.unwrap());
    multi_workspace.read_with(cx, |multi_workspace, _| {
        assert_eq!(multi_workspace.workspaces().count(), 2);
        assert_eq!(multi_workspace.workspace(), &workspace_b);
    });
}

#[gpui::test]
async fn test_close_other_project_tabs_keeps_only_the_chosen_project(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file_a.txt": "" })).await;
    fs.insert_tree("/root_b", json!({ "file_b.txt": "" })).await;
    fs.insert_tree("/root_c", json!({ "file_c.txt": "" })).await;
    let project_a = Project::test(fs.clone(), ["/root_a".as_ref()], cx).await;
    let project_b = Project::test(fs.clone(), ["/root_b".as_ref()], cx).await;
    let project_c = Project::test(fs, ["/root_c".as_ref()], cx).await;

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a, window, cx));
    let workspace_b = multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.test_add_workspace(project_b, window, cx)
    });
    multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.test_add_workspace(project_c, window, cx);
    });
    cx.run_until_parked();

    let closed = multi_workspace
        .update_in(cx, |multi_workspace, window, cx| {
            multi_workspace.close_other_workspaces(&workspace_b, window, cx)
        })
        .await
        .unwrap();
    cx.run_until_parked();

    assert!(closed);
    multi_workspace.read_with(cx, |multi_workspace, _| {
        assert_eq!(
            multi_workspace.workspaces().cloned().collect::<Vec<_>>(),
            vec![workspace_b.clone()]
        );
        assert_eq!(multi_workspace.workspace(), &workspace_b);
    });
}

#[gpui::test]
async fn test_moving_a_project_tab_to_a_new_window_moves_only_that_project(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let app_state = cx.update(AppState::test);
    let fs = app_state.fs.as_fake();
    fs.insert_tree(path!("/project_a"), json!({ "file_a.txt": "" }))
        .await;
    fs.insert_tree(path!("/project_b"), json!({ "file_b.txt": "" }))
        .await;
    let project_a = Project::test(app_state.fs.clone(), [path!("/project_a").as_ref()], cx).await;
    let project_b = Project::test(app_state.fs.clone(), [path!("/project_b").as_ref()], cx).await;

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a, window, cx));
    let workspace_a =
        multi_workspace.read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone());
    let workspace_b = multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.test_add_workspace(project_b, window, cx)
    });
    cx.run_until_parked();

    multi_workspace
        .update_in(cx, |multi_workspace, window, cx| {
            multi_workspace.move_workspace_to_new_window(&workspace_b, window, cx)
        })
        .await
        .unwrap();
    cx.run_until_parked();

    multi_workspace.read_with(cx, |multi_workspace, _| {
        assert_eq!(
            multi_workspace.workspaces().cloned().collect::<Vec<_>>(),
            vec![workspace_a.clone()],
            "the moved project leaves this window"
        );
    });
    let moved_project_paths = cx.update(|window, cx| {
        let this_window_id = window.window_handle().window_id();
        cx.windows()
            .into_iter()
            .filter_map(|window| window.downcast::<MultiWorkspace>())
            .filter(|window| window.window_id() != this_window_id)
            .filter_map(|window| {
                window
                    .read(cx)
                    .ok()
                    .map(|multi_workspace| multi_workspace.workspace().read(cx).root_paths(cx))
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(
        moved_project_paths,
        vec![vec![Arc::from(Path::new(path!("/project_b")))]],
        "the project opens in a new window of its own"
    );
}

#[gpui::test]
async fn test_dragging_a_project_tab_reorders_the_tabs(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file_a.txt": "" })).await;
    fs.insert_tree("/root_b", json!({ "file_b.txt": "" })).await;
    fs.insert_tree("/root_c", json!({ "file_c.txt": "" })).await;
    let project_a = Project::test(fs.clone(), ["/root_a".as_ref()], cx).await;
    let project_b = Project::test(fs.clone(), ["/root_b".as_ref()], cx).await;
    let project_c = Project::test(fs, ["/root_c".as_ref()], cx).await;

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a, window, cx));
    let workspace_a =
        multi_workspace.read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone());
    let workspace_b = multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.test_add_workspace(project_b, window, cx)
    });
    let workspace_c = multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.test_add_workspace(project_c, window, cx)
    });
    cx.run_until_parked();

    let tab_order = |cx: &mut VisualTestContext| {
        multi_workspace.read_with(cx, |multi_workspace, cx| {
            multi_workspace
                .project_tabs(cx)
                .into_iter()
                .map(|tab| tab.workspace)
                .collect::<Vec<_>>()
        })
    };

    multi_workspace.update(cx, |multi_workspace, cx| {
        multi_workspace.move_workspace_to_tab_of(&workspace_c, &workspace_a, cx);
    });
    assert_eq!(
        tab_order(cx),
        vec![
            workspace_c.clone(),
            workspace_a.clone(),
            workspace_b.clone()
        ]
    );

    multi_workspace.update(cx, |multi_workspace, cx| {
        multi_workspace.move_workspace_to_tab_of(&workspace_c, &workspace_b, cx);
    });
    assert_eq!(
        tab_order(cx),
        vec![workspace_a, workspace_b, workspace_c.clone()]
    );
    multi_workspace.read_with(cx, |multi_workspace, _| {
        assert_eq!(
            multi_workspace.workspace(),
            &workspace_c,
            "reordering the tabs keeps the active project"
        );
    });
}

#[gpui::test]
async fn test_project_tabs_tell_projects_with_the_same_name_apart_like_intellij(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/work/flint", json!({ "main.rs": "" }))
        .await;
    fs.insert_tree("/worktrees/feature/flint", json!({ "main.rs": "" }))
        .await;
    fs.insert_tree("/work/zed", json!({ "main.rs": "" })).await;
    let main_checkout = Project::test(fs.clone(), ["/work/flint".as_ref()], cx).await;
    let worktree_checkout =
        Project::test(fs.clone(), ["/worktrees/feature/flint".as_ref()], cx).await;
    let other_project = Project::test(fs.clone(), ["/work/zed".as_ref()], cx).await;
    let empty_project = Project::test(fs, [], cx).await;

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(main_checkout, window, cx));
    multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.test_add_workspace(worktree_checkout, window, cx);
        multi_workspace.test_add_workspace(other_project, window, cx);
        multi_workspace.test_add_workspace(empty_project, window, cx);
    });
    cx.run_until_parked();

    let titles = multi_workspace.read_with(cx, |multi_workspace, cx| {
        multi_workspace
            .project_tabs(cx)
            .into_iter()
            .map(|tab| tab.title.to_string())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        titles,
        vec![
            "flint [/work/flint]".to_string(),
            "flint [/worktrees/feature/flint]".to_string(),
            "zed".to_string(),
        ],
        "same-named projects get their folder in brackets, and an empty workspace gets no tab"
    );
}

#[gpui::test]
async fn test_jetbrains_keymap_switches_projects_like_next_and_previous_project_window(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root_a", json!({ "file_a.txt": "" })).await;
    fs.insert_tree("/root_b", json!({ "file_b.txt": "" })).await;
    fs.insert_tree("/root_c", json!({ "file_c.txt": "" })).await;
    let project_a = Project::test(fs.clone(), ["/root_a".as_ref()], cx).await;
    let project_b = Project::test(fs.clone(), ["/root_b".as_ref()], cx).await;
    let project_c = Project::test(fs, ["/root_c".as_ref()], cx).await;

    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a, window, cx));
    let workspace_a =
        multi_workspace.read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone());
    multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.test_add_workspace(project_b, window, cx);
    });
    let workspace_c = multi_workspace.update_in(cx, |multi_workspace, window, cx| {
        multi_workspace.test_add_workspace(project_c, window, cx)
    });
    cx.update(|_, cx| {
        for asset_path in ["keymaps/default-macos.json", "keymaps/macos/jetbrains.json"] {
            cx.bind_keys(
                settings::KeymapFile::load_asset_allow_partial_failure(asset_path, cx)
                    .expect("the shipped keymap loads"),
            );
        }
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("cmd-alt-`");
    multi_workspace.read_with(cx, |multi_workspace, _| {
        assert_eq!(
            multi_workspace.workspace(),
            &workspace_a,
            "cmd-alt-` is IntelliJ's Next Project Window"
        );
    });

    // macOS reports cmd-alt-shift-` as cmd-alt-~.
    cx.simulate_keystrokes("cmd-alt-~");
    multi_workspace.read_with(cx, |multi_workspace, _| {
        assert_eq!(
            multi_workspace.workspace(),
            &workspace_c,
            "cmd-alt-shift-` is IntelliJ's Previous Project Window"
        );
    });
}
