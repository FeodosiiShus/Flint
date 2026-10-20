use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use fs::{FakeGitOperation, FakePush, Fs as _};
use git::repository::{
    CommitSummary, GitFailureKind, PushOptions, RepositoryOperation, UpstreamTracking,
    UpstreamTrackingStatus,
};
use gpui::{
    BorrowAppContext as _, Entity, SharedString, TaskExt as _, TestAppContext, VisualTestContext,
    Window, WindowHandle,
};
use project::git_store::{Repository, RepositoryEvent};
use project::{FakeFs, Project};
use serde_json::json;
use settings::SettingsStore;
use util::path;
use workspace::{MultiWorkspace, OpenOptions, Workspace};

use super::branch_notification::BranchNotification;
use super::delete_dialogs::DeleteRemoteBranchDialog;
use super::integrate::UpdateMethod;
use super::ongoing::{OngoingOperationAction, run_ongoing_action};
use super::push_dialog::PushDialog;
use super::smart_operation::{SmartChoice, SmartOperationDialog};
use super::update_dialog::FixTrackedBranchDialog;
use super::*;
use crate::branch_refs::RefTarget;

const ROOT: &str = path!("/project");

fn commit(sha: &str) -> CommitSummary {
    CommitSummary {
        sha: SharedString::from(sha.to_string()),
        subject: SharedString::from(format!("subject {sha}")),
        commit_timestamp: 0,
        author_name: SharedString::from("author"),
        has_parent: true,
    }
}

fn tracked() -> UpstreamTracking {
    UpstreamTracking::Tracked(UpstreamTrackingStatus {
        ahead: 0,
        behind: 0,
    })
}

struct Fixture {
    fs: Arc<FakeFs>,
    dot_git: PathBuf,
    workspace: Entity<Workspace>,
    repository: Entity<Repository>,
    context: BranchContext,
    window: WindowHandle<MultiWorkspace>,
    cx: VisualTestContext,
}

impl Fixture {
    async fn open(cx: &mut TestAppContext, configure: impl FnOnce(&FakeFs, &Path)) -> Self {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            cx.set_global(db::AppDatabase::test_new());
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            editor::init(cx);
        });
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            ROOT,
            json!({
                ".git": {},
                "file.txt": "content",
                "a.txt": "a",
                "b.txt": "b",
            }),
        )
        .await;
        let dot_git = Path::new(ROOT).join(".git");
        fs.set_head_and_index_for_repo(
            &dot_git,
            &[
                ("file.txt", "content".to_string()),
                ("a.txt", "a".to_string()),
                ("b.txt", "b".to_string()),
            ],
        );
        fs.insert_branches(&dot_git, &["main", "topic"]);
        fs.set_remote_for_repo(&dot_git, "origin", "https://example.com/repo.git");
        configure(&fs, &dot_git);

        let project = Project::test(fs.clone(), [Path::new(ROOT)], cx).await;
        cx.run_until_parked();
        let repository = project
            .read_with(cx, |project, cx| project.active_repository(cx))
            .expect("the project has a repository");
        let window =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = window
            .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
            .expect("the window is open");
        let visual_cx = VisualTestContext::from_window(window.into(), cx);
        let context = BranchContext::new(workspace.downgrade(), repository.clone());
        let fixture = Self {
            fs,
            dot_git,
            workspace,
            repository,
            context,
            window,
            cx: visual_cx,
        };
        fixture.cx.run_until_parked();
        fixture
    }

    fn start(&mut self, operation: impl FnOnce(BranchContext, &mut Window, &mut App)) {
        let context = self.context.clone();
        self.cx.update(|window, cx| operation(context, window, cx));
    }

    fn run(&mut self, operation: impl FnOnce(BranchContext, &mut Window, &mut App)) {
        self.start(operation);
        self.cx.run_until_parked();
    }

    fn notifications(&mut self) -> Vec<Entity<BranchNotification>> {
        self.workspace.read_with(&self.cx, |workspace, _| {
            workspace
                .notification_views()
                .into_iter()
                .filter_map(|view| view.downcast::<BranchNotification>().ok())
                .collect()
        })
    }

    fn notices(&mut self) -> Vec<BranchNotice> {
        let notifications = self.notifications();
        notifications
            .iter()
            .map(|notification| notification.read_with(&self.cx, |n, _| n.notice().clone()))
            .collect()
    }

    fn notice_texts(&mut self) -> Vec<(Option<String>, String)> {
        self.notices()
            .into_iter()
            .map(|notice| {
                (
                    notice.title.map(|title| title.to_string()),
                    notice.message.to_string(),
                )
            })
            .collect()
    }

    fn find_notification(&mut self, message: &str) -> Entity<BranchNotification> {
        let notifications = self.notifications();
        notifications
            .into_iter()
            .find(|notification| {
                notification.read_with(&self.cx, |notification, _| {
                    notification.notice().message.as_ref() == message
                        || notification.notice().title.as_deref() == Some(message)
                })
            })
            .unwrap_or_else(|| panic!("no notification {message:?}: {:?}", self.notice_texts()))
    }

    fn click_action(&mut self, message: &str, label: &str) {
        let notification = self.find_notification(message);
        let index = notification.read_with(&self.cx, |notification, _| {
            notification
                .notice()
                .actions
                .iter()
                .position(|action| action.label.as_ref() == label)
                .unwrap_or_else(|| panic!("{message:?} has no action {label:?}"))
        });
        notification.update_in(&mut self.cx, |notification, window, cx| {
            notification.invoke_action(index, window, cx)
        });
        self.cx.run_until_parked();
    }

    fn modal<V: workspace::ModalView + 'static>(&mut self) -> Option<Entity<V>> {
        self.workspace
            .read_with(&self.cx, |workspace, cx| workspace.active_modal::<V>(cx))
    }

    fn progress_titles(&mut self) -> Vec<String> {
        self.workspace.read_with(&self.cx, |workspace, cx| {
            workspace
                .background_task_titles(cx)
                .into_iter()
                .map(|title| title.to_string())
                .collect()
        })
    }

    fn current_branch(&mut self) -> Option<String> {
        self.fs
            .with_git_state(&self.dot_git, false, |state| {
                state.current_branch_name.clone()
            })
            .expect("the repository exists")
    }

    fn open_file_names(&mut self) -> Vec<String> {
        self.workspace.read_with(&self.cx, |workspace, cx| {
            workspace
                .items(cx)
                .filter_map(|item| item.project_path(cx))
                .map(|path| path.path.as_unix_str().to_string())
                .collect()
        })
    }

    fn open_file(&mut self, name: &str) {
        let path = PathBuf::from(ROOT).join(name);
        let opening = self
            .workspace
            .update_in(&mut self.cx, |workspace, window, cx| {
                workspace.open_abs_path(path, OpenOptions::default(), window, cx)
            });
        self.cx.run_until_parked();
        drop(opening);
    }

    fn close_file(&mut self, name: &str) {
        let name = name.to_string();
        self.workspace
            .update_in(&mut self.cx, |workspace, window, cx| {
                for pane in workspace.panes().to_vec() {
                    let item_id = pane.read(cx).items().find_map(|item| {
                        let path = item.project_path(cx)?;
                        (path.path.as_unix_str() == name).then(|| item.item_id())
                    });
                    if let Some(item_id) = item_id {
                        pane.update(cx, |pane, cx| {
                            pane.close_item_by_id(item_id, workspace::SaveIntent::Close, window, cx)
                                .detach_and_log_err(cx)
                        });
                    }
                }
            });
        self.cx.run_until_parked();
    }

    fn update_settings(&mut self, update: impl FnOnce(&mut settings::SettingsContent)) {
        self.cx.update(|_, cx| {
            cx.update_global::<SettingsStore, _>(|store, cx| store.update_user_settings(cx, update))
        });
        self.cx.run_until_parked();
    }
}

fn configure_update(fs: &FakeFs, dot_git: &Path) {
    fs.insert_branches(dot_git, &["origin/main"]);
    fs.set_ref_for_repo(dot_git, "HEAD", "c1");
    fs.set_ref_for_repo(dot_git, "refs/heads/main", "c1");
    fs.set_ref_for_repo(dot_git, "refs/remotes/origin/main", "c2");
    fs.set_ref_history_for_repo(dot_git, "main", vec![commit("c1")]);
    fs.set_ref_history_for_repo(dot_git, "origin/main", vec![commit("c2"), commit("c1")]);
}

#[gpui::test]
async fn several_remote_branches_are_deleted_after_one_confirmation(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        fs.insert_branches(dot_git, &["a", "b", "origin/a", "origin/b"]);
        fs.set_upstream_for_repo(dot_git, "a", "origin/a", tracked());
        fs.set_upstream_for_repo(dot_git, "b", "origin/b", tracked());
    })
    .await;

    fixture.run(|context, window, cx| {
        manage::delete_refs(
            context,
            vec![RefTarget::remote("origin/a"), RefTarget::remote("origin/b")],
            window,
            cx,
        )
    });

    let dialog = fixture
        .modal::<DeleteRemoteBranchDialog>()
        .expect("one dialog asks about every selected remote branch");
    dialog.read_with(&fixture.cx, |dialog, _| {
        assert_eq!(
            dialog.remote_branches(),
            &[
                SharedString::from("origin/a"),
                SharedString::from("origin/b")
            ]
        );
        assert_eq!(
            dialog.tracking_branches(),
            &[SharedString::from("a"), SharedString::from("b")]
        );
    });

    dialog.update_in(&mut fixture.cx, |dialog, window, cx| {
        dialog.set_delete_tracking_branches(true);
        dialog.submit(window, cx);
    });
    fixture.cx.run_until_parked();

    let branches = fixture
        .fs
        .with_git_state(&fixture.dot_git, false, |state| state.branches.clone())
        .expect("the repository exists");
    for deleted in ["a", "b", "origin/a", "origin/b"] {
        assert!(!branches.contains(deleted), "{deleted} was not deleted");
    }
    let texts = fixture.notice_texts();
    assert!(texts.contains(&(
        Some("Deleted remote branch origin/a".to_string()),
        "Also deleted local branch: a".to_string()
    )));
    assert!(texts.contains(&(
        Some("Deleted remote branch origin/b".to_string()),
        "Also deleted local branch: b".to_string()
    )));
}

#[gpui::test]
async fn deleting_a_tag_on_remotes_touches_only_the_remotes_that_have_it(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        fs.set_remote_for_repo(dot_git, "fork", "https://example.com/fork.git");
        fs.insert_tags(dot_git, &[("v1", "aaa111"), ("v2", "bbb222")]);
        fs.with_git_state(dot_git, false, |state| {
            state.remote_tags.insert(
                ("origin".to_string(), "v1".to_string()),
                "aaa111".to_string(),
            );
        })
        .expect("the repository exists");
    })
    .await;

    fixture
        .run(|context, window, cx| manage::delete_ref(context, RefTarget::tag("v1"), window, cx));
    fixture.click_action("Deleted Tag: v1", "Delete on Remotes");

    let remote_tags = fixture
        .fs
        .with_git_state(&fixture.dot_git, false, |state| state.remote_tags.clone())
        .expect("the repository exists");
    assert!(remote_tags.is_empty(), "{remote_tags:?}");
    assert!(
        fixture
            .notice_texts()
            .contains(&(None, "Deleted Tag on Remotes: v1".to_string())),
        "{:?}",
        fixture.notice_texts()
    );

    fixture
        .run(|context, window, cx| manage::delete_ref(context, RefTarget::tag("v2"), window, cx));
    fixture.click_action("Deleted Tag: v2", "Delete on Remotes");
    assert!(
        fixture
            .notice_texts()
            .contains(&(None, "Tag Doesn't Exist on Remotes: v2".to_string())),
        "{:?}",
        fixture.notice_texts()
    );
}

#[gpui::test]
async fn a_failed_remote_listing_names_the_single_remote_in_the_title(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        fs.insert_tags(dot_git, &[("v1", "aaa111")]);
    })
    .await;

    fixture
        .run(|context, window, cx| manage::delete_ref(context, RefTarget::tag("v1"), window, cx));
    fixture.fs.set_simulated_git_failure_for_repo(
        &fixture.dot_git,
        FakeGitOperation::GetRemotes,
        Some((GitFailureKind::Other, "fatal: unable to read config")),
    );
    fixture.click_action("Deleted Tag: v1", "Delete on Remote");

    let titles: Vec<Option<String>> = fixture
        .notice_texts()
        .into_iter()
        .map(|(title, _)| title)
        .collect();
    assert!(
        titles.contains(&Some("Failed to delete tag v1 on Remote".to_string())),
        "{titles:?}"
    );
}

#[gpui::test]
async fn checkout_saves_modified_editors_before_switching(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, |_, _| {}).await;
    let project = fixture
        .workspace
        .read_with(&fixture.cx, |workspace, _| workspace.project().clone());
    let buffer = project
        .update(&mut fixture.cx, |project, cx| {
            project.open_local_buffer(path!("/project/file.txt"), cx)
        })
        .await
        .expect("the file opens");
    buffer.update(&mut fixture.cx, |buffer, cx| {
        buffer.edit([(0..0, "edited ")], None, cx)
    });
    assert!(buffer.read_with(&fixture.cx, |buffer, _| buffer.is_dirty()));

    fixture.run(|context, window, cx| {
        checkout::checkout(context, RefTarget::local("topic"), window, cx)
    });

    assert_eq!(fixture.current_branch().as_deref(), Some("topic"));
    assert!(!buffer.read_with(&fixture.cx, |buffer, _| buffer.is_dirty()));
    let saved = fixture
        .fs
        .load(Path::new(path!("/project/file.txt")))
        .await
        .expect("the file exists");
    assert_eq!(saved, "edited content");
}

#[gpui::test]
async fn every_branch_operation_shows_its_intellij_progress_title(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        fs.insert_branches(dot_git, &["feature"]);
    })
    .await;

    fixture.start(|context, window, cx| {
        checkout::checkout(context, RefTarget::local("topic"), window, cx)
    });
    assert_eq!(
        fixture.progress_titles(),
        vec!["Checking out topic\u{2026}"]
    );
    fixture.cx.run_until_parked();
    assert!(fixture.progress_titles().is_empty());

    fixture.start(|context, window, cx| {
        integrate::merge_into_current(context, RefTarget::local("feature"), window, cx)
    });
    assert_eq!(fixture.progress_titles(), vec!["Merging feature\u{2026}"]);
    fixture.cx.run_until_parked();

    fixture.start(|context, window, cx| {
        manage::delete_ref(context, RefTarget::local("feature"), window, cx)
    });
    assert_eq!(fixture.progress_titles(), vec!["Deleting feature\u{2026}"]);
    fixture.cx.run_until_parked();

    fixture.start(|context, window, cx| {
        manage::perform_rename(context, "main".into(), "trunk".into(), false, window, cx)
    });
    assert_eq!(
        fixture.progress_titles(),
        vec!["Renaming main to trunk\u{2026}"]
    );
    fixture.cx.run_until_parked();
    assert!(fixture.progress_titles().is_empty());
}

#[gpui::test]
async fn force_push_asks_first_and_can_stop_warning_about_the_target(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        fs.insert_branches(dot_git, &["feature", "origin/feature"]);
        fs.set_upstream_for_repo(dot_git, "feature", "origin/feature", tracked());
    })
    .await;

    fixture.run(|context, window, cx| {
        manage::push_branch(context, RefTarget::local("feature"), window, cx)
    });
    let dialog = fixture
        .modal::<PushDialog>()
        .expect("the push dialog is open");
    dialog.update_in(&mut fixture.cx, |dialog, window, cx| {
        assert!(!dialog.is_force_push_prohibited(cx));
        dialog.click_force_push(window, cx);
    });
    assert_eq!(
        dialog.read_with(&fixture.cx, |dialog, _| dialog.force_push_confirmation()),
        Some((SharedString::from("origin/feature"), false))
    );
    let pushes = |fixture: &mut Fixture| {
        fixture
            .fs
            .with_git_state(&fixture.dot_git, false, |state| state.pushes.clone())
            .expect("the repository exists")
    };
    assert!(pushes(&mut fixture).is_empty(), "pushed before confirming");

    dialog.update_in(&mut fixture.cx, |dialog, window, cx| {
        dialog.set_dont_warn(true);
        dialog.submit(window, cx);
    });
    fixture.cx.run_until_parked();
    assert_eq!(
        pushes(&mut fixture),
        vec![FakePush {
            branch: "feature".to_string(),
            remote_branch: "feature".to_string(),
            remote: "origin".to_string(),
            options: Some(PushOptions::Force),
        }]
    );

    fixture.run(|context, window, cx| {
        manage::push_branch(context, RefTarget::local("feature"), window, cx)
    });
    let dialog = fixture
        .modal::<PushDialog>()
        .expect("the push dialog is open");
    dialog.update_in(&mut fixture.cx, |dialog, window, cx| {
        dialog.click_force_push(window, cx)
    });
    fixture.cx.run_until_parked();
    assert_eq!(
        pushes(&mut fixture).len(),
        2,
        "a target marked \"Don't warn\" is force-pushed without asking"
    );
}

#[gpui::test]
async fn the_force_push_confirmation_answers_escape_and_enter_from_the_keyboard(
    cx: &mut TestAppContext,
) {
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        fs.insert_branches(dot_git, &["feature", "origin/feature"]);
        fs.set_upstream_for_repo(dot_git, "feature", "origin/feature", tracked());
    })
    .await;
    fixture.run(|context, window, cx| {
        manage::push_branch(context, RefTarget::local("feature"), window, cx)
    });
    let dialog = fixture
        .modal::<PushDialog>()
        .expect("the push dialog is open");

    dialog.update_in(&mut fixture.cx, |dialog, window, cx| {
        dialog.click_force_push(window, cx)
    });
    fixture.cx.run_until_parked();
    fixture.cx.dispatch_action(menu::Cancel);
    assert_eq!(
        dialog.read_with(&fixture.cx, |dialog, _| dialog.force_push_confirmation()),
        None,
        "Escape leaves the confirmation"
    );
    assert!(
        fixture.modal::<PushDialog>().is_some(),
        "and goes back to the push dialog"
    );
    let editor_focused = dialog.update_in(&mut fixture.cx, |dialog, window, cx| {
        gpui::Focusable::focus_handle(dialog, cx).is_focused(window)
    });
    assert!(editor_focused, "the branch editor has the focus again");

    dialog.update_in(&mut fixture.cx, |dialog, window, cx| {
        dialog.click_force_push(window, cx)
    });
    fixture.cx.run_until_parked();
    fixture.cx.dispatch_action(menu::Confirm);
    fixture.cx.run_until_parked();
    let pushes = fixture
        .fs
        .with_git_state(&fixture.dot_git, false, |state| state.pushes.clone())
        .expect("the repository exists");
    assert_eq!(
        pushes.iter().map(|push| push.options).collect::<Vec<_>>(),
        vec![Some(PushOptions::Force)],
        "Enter confirms the force push"
    );
}

#[gpui::test]
async fn the_force_push_action_refuses_protected_targets_and_asks_for_others(
    cx: &mut TestAppContext,
) {
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        fs.insert_branches(dot_git, &["origin/main", "feature", "origin/feature"]);
        fs.set_upstream_for_repo(dot_git, "main", "origin/main", tracked());
        fs.set_upstream_for_repo(dot_git, "feature", "origin/feature", tracked());
    })
    .await;
    let pushes = |fixture: &mut Fixture| {
        fixture
            .fs
            .with_git_state(&fixture.dot_git, false, |state| state.pushes.clone())
            .expect("the repository exists")
    };

    // `git: force push`, as the command palette and key bindings run it.
    let run_force_push_action = |fixture: &mut Fixture| {
        let workspace = fixture.workspace.clone();
        workspace.update_in(&mut fixture.cx, |workspace, window, cx| {
            crate::force_push(workspace, &git::ForcePush, window, cx)
        });
        fixture.cx.run_until_parked();
    };
    run_force_push_action(&mut fixture);
    assert!(pushes(&mut fixture).is_empty(), "main is protected");
    assert!(fixture.modal::<PushDialog>().is_none());
    let messages: Vec<String> = fixture
        .notices()
        .into_iter()
        .map(|notice| notice.message.to_string())
        .collect();
    assert_eq!(messages, vec!["Force push to origin/main is prohibited"]);

    fixture
        .fs
        .set_branch_name(&fixture.dot_git, Some("feature"));
    fixture.cx.run_until_parked();
    run_force_push_action(&mut fixture);
    let dialog = fixture
        .modal::<PushDialog>()
        .expect("the push dialog opens at the force push confirmation");
    assert_eq!(
        dialog.read_with(&fixture.cx, |dialog, _| dialog.force_push_confirmation()),
        Some((SharedString::from("origin/feature"), false))
    );
    assert!(pushes(&mut fixture).is_empty(), "pushed before confirming");
}

#[gpui::test]
async fn force_push_to_a_protected_branch_is_prohibited(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        fs.insert_branches(dot_git, &["origin/main", "release", "origin/release"]);
        fs.set_upstream_for_repo(dot_git, "main", "origin/main", tracked());
        fs.set_upstream_for_repo(dot_git, "release", "origin/release", tracked());
    })
    .await;

    fixture.run(|context, window, cx| {
        manage::push_branch(context, RefTarget::local("main"), window, cx)
    });
    let dialog = fixture
        .modal::<PushDialog>()
        .expect("the push dialog is open");
    dialog.update_in(&mut fixture.cx, |dialog, window, cx| {
        assert!(dialog.is_force_push_prohibited(cx));
        dialog.click_force_push(window, cx);
        assert_eq!(dialog.force_push_confirmation(), None);
    });

    fixture.update_settings(|settings| {
        settings.git.get_or_insert_default().protected_branches = Some(vec!["release".to_string()]);
    });
    dialog.update_in(&mut fixture.cx, |dialog, _, cx| {
        assert!(
            !dialog.is_force_push_prohibited(cx),
            "the protected branches come from the git.protected_branches setting"
        );
    });
    fixture.cx.update(|window, cx| {
        dialog.update(cx, |_, cx| cx.emit(gpui::DismissEvent));
        window.refresh();
    });
    fixture.cx.run_until_parked();
    fixture.run(|context, window, cx| {
        manage::push_branch(context, RefTarget::local("release"), window, cx)
    });
    let dialog = fixture
        .modal::<PushDialog>()
        .expect("the push dialog is open");
    dialog.update_in(&mut fixture.cx, |dialog, _, cx| {
        assert!(dialog.is_force_push_prohibited(cx));
    });
}

#[gpui::test]
async fn a_branch_used_by_another_worktree_can_be_checked_out_anyway(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        fs.set_simulated_git_failure_for_repo(
            dot_git,
            FakeGitOperation::Checkout,
            Some((
                GitFailureKind::BranchCheckedOutInWorktree {
                    path: "/worktrees/topic".to_string(),
                },
                "fatal: 'topic' is already used by worktree at '/worktrees/topic'",
            )),
        );
    })
    .await;

    fixture.run(|context, window, cx| {
        checkout::checkout(context, RefTarget::local("topic"), window, cx)
    });
    assert_eq!(
        fixture.cx.pending_prompt(),
        Some((
            "Worktree Conflict".to_string(),
            "Branch 'topic' is already checked out in a worktree at:\n/worktrees/topic".to_string()
        ))
    );
    fixture.cx.simulate_prompt_answer("Cancel");
    fixture.cx.run_until_parked();
    assert_eq!(fixture.current_branch().as_deref(), Some("main"));
    assert!(fixture.notices().is_empty(), "{:?}", fixture.notice_texts());

    fixture.run(|context, window, cx| {
        checkout::checkout(context, RefTarget::local("topic"), window, cx)
    });
    fixture.cx.simulate_prompt_answer("Checkout Anyway");
    fixture.cx.run_until_parked();
    assert_eq!(fixture.current_branch().as_deref(), Some("topic"));
    assert!(
        fixture
            .notice_texts()
            .contains(&(None, "Checked out topic".to_string()))
    );
}

#[gpui::test]
async fn open_existing_worktree_adds_a_project_tab_instead_of_opening_a_window(
    cx: &mut TestAppContext,
) {
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        fs.set_simulated_git_failure_for_repo(
            dot_git,
            FakeGitOperation::Checkout,
            Some((
                GitFailureKind::BranchCheckedOutInWorktree {
                    path: "/worktrees/topic".to_string(),
                },
                "fatal: 'topic' is already used by worktree at '/worktrees/topic'",
            )),
        );
    })
    .await;
    let worktree_path = PathBuf::from(path!("/worktrees/topic"));
    fixture
        .fs
        .insert_tree(&worktree_path, json!({ "file.txt": "content" }))
        .await;
    let fs = fixture.fs.clone();
    fixture.cx.update(|_, cx| <dyn fs::Fs>::set_global(fs, cx));
    let windows_before = fixture.cx.update(|_, cx| cx.windows().len());

    fixture.run(|context, window, cx| {
        checkout::checkout(context, RefTarget::local("topic"), window, cx)
    });
    fixture.cx.simulate_prompt_answer("Open Existing Worktree");
    fixture.cx.run_until_parked();

    assert_eq!(
        fixture.cx.update(|_, cx| cx.windows().len()),
        windows_before,
        "Open Existing Worktree never opens a window"
    );
    fixture
        .window
        .read_with(&fixture.cx, |multi_workspace, cx| {
            assert_eq!(multi_workspace.workspaces().count(), 2);
            assert_eq!(
                multi_workspace.workspace().read(cx).root_paths(cx),
                vec![Arc::from(worktree_path.as_path())]
            );
        })
        .expect("window is alive");
    assert_eq!(fixture.current_branch().as_deref(), Some("main"));
}

#[gpui::test]
async fn force_checkout_after_checkout_anyway_keeps_ignoring_the_other_worktree(
    cx: &mut TestAppContext,
) {
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        fs.set_simulated_git_failure_for_repo(
            dot_git,
            FakeGitOperation::Checkout,
            Some((
                GitFailureKind::BranchCheckedOutInWorktree {
                    path: "/worktrees/topic".to_string(),
                },
                "fatal: 'topic' is already used by worktree at '/worktrees/topic'",
            )),
        );
        fs.set_simulated_git_failure_for_repo(
            dot_git,
            FakeGitOperation::CheckoutIgnoringOtherWorktrees,
            Some((
                GitFailureKind::LocalChangesWouldBeOverwritten {
                    files: vec!["a.txt".to_string()],
                },
                "error: Your local changes to the following files would be overwritten by checkout:\n\ta.txt",
            )),
        );
    })
    .await;

    fixture.run(|context, window, cx| {
        checkout::checkout(context, RefTarget::local("topic"), window, cx)
    });
    fixture.cx.simulate_prompt_answer("Checkout Anyway");
    fixture.cx.run_until_parked();
    let dialog = fixture
        .modal::<SmartOperationDialog>()
        .expect("the local changes ask for Smart or Force Checkout");
    dialog.update(&mut fixture.cx, |dialog, cx| {
        dialog.choose(SmartChoice::Force, cx)
    });
    fixture.cx.run_until_parked();

    assert_eq!(fixture.current_branch().as_deref(), Some("topic"));
    assert!(
        fixture
            .notice_texts()
            .contains(&(None, "Checked out topic".to_string())),
        "{:?}",
        fixture.notice_texts()
    );
}

#[gpui::test]
async fn an_update_by_merge_stashes_silently_only_when_needed(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        configure_update(fs, dot_git);
        fs.set_upstream_for_repo(dot_git, "main", "origin/main", tracked());
        fs.set_index_for_repo(dot_git, &[("file.txt", "staged".to_string())]);
    })
    .await;
    fixture
        .fs
        .write(Path::new(path!("/project/file.txt")), b"staged")
        .await
        .expect("the file is written");
    fixture.cx.run_until_parked();

    let stash_events = Rc::new(Cell::new(0));
    let _subscription = fixture.cx.update(|_, cx| {
        let stash_events = stash_events.clone();
        cx.subscribe(&fixture.repository, move |_, event: &RepositoryEvent, _| {
            if matches!(event, RepositoryEvent::StashEntriesChanged) {
                stash_events.set(stash_events.get() + 1);
            }
        })
    });

    fixture.run(|context, window, cx| {
        integrate::update_current_branch(context, UpdateMethod::Merge, window, cx)
    });

    assert!(
        fixture.modal::<SmartOperationDialog>().is_none(),
        "IntelliJ never shows the Smart Merge dialog for an update"
    );
    assert!(
        stash_events.get() > 0,
        "staged changes are stashed before the update merges"
    );
    let head = fixture
        .fs
        .with_git_state(&fixture.dot_git, false, |state| {
            state.refs.get("HEAD").cloned()
        })
        .expect("the repository exists");
    assert_eq!(head.as_deref(), Some("c2"));
    let stash_left = fixture.repository.read_with(&fixture.cx, |repository, _| {
        repository.cached_stash().entries.len()
    });
    assert_eq!(stash_left, 0, "the stashed changes were restored");
    let notice = fixture.find_notification("Updated main from origin/main");
    let labels = notice.read_with(&fixture.cx, |notification, _| {
        notification
            .notice()
            .actions
            .iter()
            .map(|action| action.label.to_string())
            .collect::<Vec<_>>()
    });
    assert_eq!(labels, vec!["View Commits".to_string()]);
}

#[gpui::test]
async fn an_update_without_a_tracked_branch_lets_the_user_choose_one(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        configure_update(fs, dot_git);
        fs.clear_upstream_for_repo(dot_git, "main");
    })
    .await;

    fixture.run(|context, window, cx| {
        integrate::update_current_branch(context, UpdateMethod::Merge, window, cx)
    });
    assert!(
        fixture.notice_texts().contains(&(
            Some("Cannot update".to_string()),
            "main has no tracked branch".to_string()
        )),
        "{:?}",
        fixture.notice_texts()
    );
    fixture.click_action("main has no tracked branch", "Choose upstream branch");

    let dialog = fixture
        .modal::<FixTrackedBranchDialog>()
        .expect("the Update dialog asks for the branch to update from");
    dialog.update_in(&mut fixture.cx, |dialog, window, cx| {
        dialog.set_as_tracked(true);
        dialog.submit(window, cx);
    });
    fixture.cx.run_until_parked();

    let upstream = fixture
        .fs
        .with_git_state(&fixture.dot_git, false, |state| {
            state
                .upstreams
                .get("main")
                .map(|upstream| upstream.ref_name.to_string())
        })
        .expect("the repository exists");
    assert_eq!(upstream.as_deref(), Some("refs/remotes/origin/main"));
    let head = fixture
        .fs
        .with_git_state(&fixture.dot_git, false, |state| {
            state.refs.get("HEAD").cloned()
        })
        .expect("the repository exists");
    assert_eq!(head.as_deref(), Some("c2"));
}

#[gpui::test]
async fn updating_several_branches_reports_one_fetch(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        fs.insert_branches(dot_git, &["a", "b", "origin/a", "origin/b"]);
        fs.set_upstream_for_repo(dot_git, "a", "origin/a", tracked());
        fs.set_upstream_for_repo(dot_git, "b", "origin/b", tracked());
    })
    .await;

    fixture.start(|context, window, cx| {
        integrate::update_branches(
            context,
            vec![RefTarget::local("a"), RefTarget::local("b")],
            window,
            cx,
        )
    });
    assert_eq!(fixture.progress_titles(), vec!["Updating branches\u{2026}"]);
    fixture.cx.run_until_parked();
    assert_eq!(
        fixture.notice_texts(),
        vec![(None, "Fetched 2 targets".to_string())]
    );
}

#[gpui::test]
async fn abort_merge_restores_only_the_stash_flint_made(cx: &mut TestAppContext) {
    const FOREIGN_STASH: &str = "On main: Uncommitted changes before Checkout at 10/20/26, 3:45 PM";
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        fs.set_operation_in_progress_for_repo(dot_git, Some(RepositoryOperation::Merge));
        fs.with_git_state(dot_git, true, |state| {
            state.stash_entries = git::stash::GitStash {
                entries: vec![git::stash::StashEntry {
                    index: 0,
                    oid: git::Oid::from_bytes(&[0xee; 20]).expect("a valid commit id"),
                    message: FOREIGN_STASH.to_string(),
                    branch: Some("main".to_string()),
                    timestamp: 0,
                }]
                .into(),
            };
        })
        .expect("the repository exists");
    })
    .await;
    let stash_messages = |fixture: &mut Fixture| {
        fixture
            .fs
            .with_git_state(&fixture.dot_git, false, |state| {
                state
                    .stash_entries
                    .entries
                    .iter()
                    .map(|entry| entry.message.clone())
                    .collect::<Vec<_>>()
            })
            .expect("the repository exists")
    };

    fixture.run(|context, window, cx| {
        run_ongoing_action(context, OngoingOperationAction::AbortMerge, window, cx)
    });
    fixture.cx.simulate_prompt_answer("Abort");
    fixture.cx.run_until_parked();
    assert_eq!(
        stash_messages(&mut fixture),
        vec![FOREIGN_STASH.to_string()],
        "a stash with IntelliJ's message that Flint did not make is left alone"
    );

    fixture
        .fs
        .insert_file(Path::new(ROOT).join("a.txt"), b"changed".to_vec())
        .await;
    fixture.cx.run_until_parked();
    let stashing = fixture.cx.update(|_, cx| {
        smart_operation::stash_local_changes(
            &fixture.context,
            smart_operation::stash_message_now("Merge"),
            cx,
        )
    });
    assert!(stashing.await.expect("the changes are stashed"));
    fixture.cx.run_until_parked();
    assert_eq!(stash_messages(&mut fixture).len(), 2);
    fixture
        .fs
        .set_operation_in_progress_for_repo(&fixture.dot_git, Some(RepositoryOperation::Merge));
    fixture.cx.run_until_parked();

    fixture.run(|context, window, cx| {
        run_ongoing_action(context, OngoingOperationAction::AbortMerge, window, cx)
    });
    fixture.cx.simulate_prompt_answer("Abort");
    fixture.cx.run_until_parked();
    assert_eq!(
        stash_messages(&mut fixture),
        vec![FOREIGN_STASH.to_string()],
        "the stash Flint made for the merge is restored"
    );
}

#[gpui::test]
async fn cherry_pick_in_progress_can_be_aborted_or_continued(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        fs.set_operation_in_progress_for_repo(dot_git, Some(RepositoryOperation::CherryPick));
    })
    .await;

    fixture.run(|context, window, cx| {
        run_ongoing_action(context, OngoingOperationAction::AbortCherryPick, window, cx)
    });
    assert_eq!(
        fixture.cx.pending_prompt(),
        Some((
            "Abort Cherry-Pick".to_string(),
            "Abort cherry-pick?".to_string()
        ))
    );
    fixture.cx.simulate_prompt_answer("Abort");
    fixture.cx.run_until_parked();
    let aborted = fixture
        .fs
        .with_git_state(&fixture.dot_git, false, |state| {
            state.aborted_operations.clone()
        })
        .expect("the repository exists");
    assert_eq!(aborted, vec![RepositoryOperation::CherryPick]);
    assert!(
        fixture
            .notice_texts()
            .contains(&(None, "Cherry-Pick abort succeeded".to_string()))
    );

    fixture.fs.set_operation_in_progress_for_repo(
        &fixture.dot_git,
        Some(RepositoryOperation::CherryPick),
    );
    fixture.cx.run_until_parked();
    fixture.run(|context, window, cx| {
        run_ongoing_action(
            context,
            OngoingOperationAction::ContinueCherryPick,
            window,
            cx,
        )
    });
    let in_progress = fixture
        .fs
        .with_git_state(&fixture.dot_git, false, |state| {
            state.refs.contains_key("CHERRY_PICK_HEAD")
        })
        .expect("the repository exists");
    assert!(!in_progress, "the cherry-pick was not continued");
    assert!(
        fixture
            .notices()
            .iter()
            .any(|notice| notice.title.as_deref() == Some("Cherry-pick continue successful")),
        "{:?}",
        fixture.notice_texts()
    );
}

#[gpui::test]
async fn switching_back_to_a_branch_restores_its_editors_with_a_rollback(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, |_, _| {}).await;
    fixture.open_file("a.txt");
    assert_eq!(fixture.open_file_names(), vec!["a.txt".to_string()]);

    fixture.run(|context, window, cx| {
        checkout::checkout(context, RefTarget::local("topic"), window, cx)
    });
    assert_eq!(fixture.current_branch().as_deref(), Some("topic"));
    fixture.close_file("a.txt");
    fixture.open_file("b.txt");
    assert_eq!(fixture.open_file_names(), vec!["b.txt".to_string()]);

    fixture.run(|context, window, cx| {
        checkout::checkout(context, RefTarget::local("main"), window, cx)
    });
    assert_eq!(fixture.open_file_names(), vec!["a.txt".to_string()]);
    let restored = "Workspace associated with branch 'main' has been restored";
    assert!(
        fixture
            .notice_texts()
            .contains(&(None, restored.to_string())),
        "{:?}",
        fixture.notice_texts()
    );

    fixture.click_action(restored, "Rollback");
    assert_eq!(fixture.open_file_names(), vec!["b.txt".to_string()]);
}

#[gpui::test]
async fn a_file_selected_while_the_checkout_ran_stays_selected_after_the_restore(
    cx: &mut TestAppContext,
) {
    let mut fixture = Fixture::open(cx, |_, _| {}).await;
    fixture.open_file("a.txt");
    let pending = fixture
        .cx
        .update(|_, cx| branch_workspace::branch_will_change(&fixture.context, cx));
    fixture.cx.run_until_parked();
    fixture.open_file("b.txt");

    let context = fixture.context.clone();
    let restore = fixture.cx.update(|window, cx| {
        window.spawn(cx, async move |cx| {
            branch_workspace::branch_has_changed(&context, pending, "main", cx).await
        })
    });
    fixture.cx.run_until_parked();
    restore.await.expect("the workspace is restored");

    assert_eq!(
        fixture.open_file_names(),
        vec!["a.txt".to_string(), "b.txt".to_string()]
    );
    let active = fixture.workspace.read_with(&fixture.cx, |workspace, cx| {
        workspace
            .active_item(cx)
            .and_then(|item| item.project_path(cx))
            .map(|path| path.path.as_unix_str().to_string())
    });
    assert_eq!(active.as_deref(), Some("b.txt"));
}

#[gpui::test]
async fn editors_are_left_alone_when_workspace_restoring_is_off(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, |_, _| {}).await;
    fixture.update_settings(|settings| {
        settings
            .git
            .get_or_insert_default()
            .restore_workspace_on_branch_switch = Some(false);
    });
    fixture.open_file("a.txt");
    fixture.run(|context, window, cx| {
        checkout::checkout(context, RefTarget::local("topic"), window, cx)
    });
    fixture.close_file("a.txt");
    fixture.open_file("b.txt");
    fixture.run(|context, window, cx| {
        checkout::checkout(context, RefTarget::local("main"), window, cx)
    });
    assert_eq!(fixture.open_file_names(), vec!["b.txt".to_string()]);
}

#[gpui::test]
async fn checkout_and_rebase_refuses_to_start_during_an_unfinished_merge(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        fs.set_operation_in_progress_for_repo(dot_git, Some(RepositoryOperation::Merge));
    })
    .await;

    fixture.run(|context, window, cx| {
        checkout::checkout_and_rebase_onto_current(context, RefTarget::local("topic"), window, cx)
    });

    assert_eq!(fixture.current_branch().as_deref(), Some("main"));
    let notice = fixture
        .notices()
        .into_iter()
        .find(|notice| notice.title.as_deref() == Some("Rebase not allowed"))
        .unwrap_or_else(|| panic!("{:?}", fixture.notice_texts()));
    assert!(
        notice
            .message
            .ends_with("You should complete the merge before starting a rebase."),
        "{}",
        notice.message
    );
}

#[gpui::test]
async fn a_restored_tag_needs_no_extra_notice(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        fs.insert_tags(dot_git, &[("v1", "aaa111")]);
        fs.set_ref_for_repo(dot_git, "aaa111", "aaa111");
    })
    .await;

    fixture
        .run(|context, window, cx| manage::delete_ref(context, RefTarget::tag("v1"), window, cx));
    fixture.click_action("Deleted Tag: v1", "Restore");

    let tags = fixture
        .fs
        .with_git_state(&fixture.dot_git, false, |state| {
            state
                .tags
                .iter()
                .map(|tag| tag.name.to_string())
                .collect::<Vec<_>>()
        })
        .expect("the repository exists");
    assert_eq!(tags, vec!["v1".to_string()], "the tag was not restored");
    assert!(
        fixture.notice_texts().is_empty(),
        "IntelliJ only closes the notification after a restore: {:?}",
        fixture.notice_texts()
    );
}

#[gpui::test]
async fn compare_with_current_shows_the_changes_of_the_selected_commit(cx: &mut TestAppContext) {
    let topic_commit = "1111111111111111111111111111111111111111";
    let main_commit = "2222222222222222222222222222222222222222";
    let mut fixture = Fixture::open(cx, |fs, dot_git| {
        fs.set_ref_for_repo(dot_git, "refs/heads/topic", topic_commit);
        fs.set_ref_history_for_repo(dot_git, "topic", vec![commit(topic_commit)]);
        fs.set_ref_history_for_repo(dot_git, "main", vec![commit(main_commit)]);
    })
    .await;

    fixture.run(|context, window, cx| {
        compare::compare_with_current(context, RefTarget::local("topic"), window, cx)
    });
    let view = fixture
        .workspace
        .read_with(&fixture.cx, |workspace, cx| {
            workspace
                .items_of_type::<compare_view::CompareView>(cx)
                .next()
        })
        .expect("the compare tab is open");
    view.update(&mut fixture.cx, |view, cx| {
        view.select_commit(compare_view::CompareSide::SelectedOnly, 0, cx)
    });
    fixture.cx.run_until_parked();

    let expected = topic_commit.parse::<git::Oid>().expect("a valid commit id");
    view.read_with(&fixture.cx, |view, cx| {
        assert_eq!(
            view.selected_commit(compare_view::CompareSide::SelectedOnly),
            Some(expected)
        );
        assert_eq!(
            view.changes_browser(compare_view::CompareSide::SelectedOnly)
                .read(cx)
                .commits(),
            &[expected]
        );
        assert!(
            view.changes_browser(compare_view::CompareSide::CurrentOnly)
                .read(cx)
                .commits()
                .is_empty()
        );
    });
}
