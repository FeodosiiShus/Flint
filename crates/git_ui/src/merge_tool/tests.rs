use std::{path::Path, sync::Arc};

use git::repository::repo_path;
use gpui::{
    Action, AnyWindowHandle, Entity, Focusable as _, TestAppContext, VisualTestContext,
    WindowHandle,
};
use merge_diff::Side;
use project::{FakeFs, Project, git_store::Repository};
use serde_json::json;
use settings::SettingsStore;
use theme::LoadThemes;
use util::path;
use workspace::{MultiWorkspace, Workspace};

use super::ResolveConflicts;
use super::conflict_resolution::ConflictContext;
use super::conflicts_dialog::test_driver::{OpenConflictsDialog, find_open_dialog};
use super::dialog_window::DialogWindowShell;
use super::merge_viewer::actions::{
    AcceptLeftSide, AcceptRightSide, ApplyChanges, IgnoreLeftSide, IgnoreRightSide, NextDifference,
};
use super::resolve_conflicts_after_pull;
use crate::branch_operations::BranchContext;
use crate::branch_operations::integrate::{merge_into_current, rebase_current_onto};
use crate::branch_refs::RefTarget;

const MAIN_BRANCH: &str = "main";
const FEATURE_BRANCH: &str = "feature";
const TOPIC_BRANCH: &str = "topic";

const CONFLICT_BASE: &str = "x\n";
const CONFLICT_OURS: &str = "y\n";
const CONFLICT_THEIRS: &str = "z\n";
const AUTO_BASE: &str = "a\nb\nc\n";
const AUTO_OURS: &str = "A\nb\nc\n";
const AUTO_THEIRS: &str = "a\nb\nC\n";
const AUTO_MERGED: &str = "A\nb\nC\n";
const TEN_BASE: &str = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
const TEN_OURS: &str = "a\nL1\nc\nd\ne\nf\nL2\nh\ni\nj\n";
const TEN_THEIRS: &str = "a\nR1\nc\nd\ne\nf\nR2\nh\ni\nj\n";
const TEN_CHOSEN: &str = "a\nL1\nc\nd\ne\nf\nR2\nh\ni\nj\n";

struct ConflictFile {
    path: &'static str,
    base: &'static str,
    ours: &'static str,
    theirs: &'static str,
}

const TEXT_CONFLICT: ConflictFile = ConflictFile {
    path: "a.txt",
    base: CONFLICT_BASE,
    ours: CONFLICT_OURS,
    theirs: CONFLICT_THEIRS,
};

const AUTO_RESOLVABLE: ConflictFile = ConflictFile {
    path: "a.txt",
    base: AUTO_BASE,
    ours: AUTO_OURS,
    theirs: AUTO_THEIRS,
};

const SECOND_TEXT_CONFLICT: ConflictFile = ConflictFile {
    path: "b.txt",
    base: CONFLICT_BASE,
    ours: CONFLICT_OURS,
    theirs: CONFLICT_THEIRS,
};

const TWO_CHANGE_CONFLICT: ConflictFile = ConflictFile {
    path: "a.txt",
    base: TEN_BASE,
    ours: TEN_OURS,
    theirs: TEN_THEIRS,
};

fn dot_git() -> &'static Path {
    Path::new(path!("/project/.git"))
}

fn init_test(cx: &mut TestAppContext) {
    zlog::init_test();
    cx.update(|cx| {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
        cx.set_global(db::AppDatabase::test_new());
        theme_settings::init(LoadThemes::JustBase, cx);
        editor::init(cx);
        crate::init(cx);
    });
}

fn plan_merge(fs: &FakeFs, files: &[ConflictFile]) {
    fs.set_branch_name(dot_git(), Some(MAIN_BRANCH));
    fs.insert_branches(dot_git(), &[MAIN_BRANCH, FEATURE_BRANCH]);
    for file in files {
        fs.set_merge_conflict_for_repo(
            dot_git(),
            FEATURE_BRANCH,
            file.path,
            Some(file.base),
            Some(file.ours),
            Some(file.theirs),
        );
    }
}

fn plan_rebase(fs: &FakeFs, files: &[ConflictFile]) {
    fs.set_branch_name(dot_git(), Some(TOPIC_BRANCH));
    fs.insert_branches(dot_git(), &[MAIN_BRANCH, TOPIC_BRANCH]);
    for file in files {
        fs.set_rebase_conflict_for_repo(
            dot_git(),
            MAIN_BRANCH,
            file.path,
            Some(file.base),
            Some(file.ours),
            Some(file.theirs),
        );
    }
}

fn plan_existing_conflicts(fs: &FakeFs, files: &[ConflictFile]) {
    fs.set_branch_name(dot_git(), Some(MAIN_BRANCH));
    for file in files {
        fs.set_conflict_for_repo(
            dot_git(),
            file.path,
            Some(file.base),
            Some(file.ours),
            Some(file.theirs),
        );
    }
}

fn read_file(fs: &FakeFs, path: &str) -> String {
    String::from_utf8(fs.read_file_sync(path).expect("file should exist"))
        .expect("file should be valid UTF-8")
}

fn index_text(fs: &FakeFs, path: &str) -> Option<String> {
    fs.with_git_state(dot_git(), false, |state| {
        state.index_contents.get(&repo_path(path)).cloned()
    })
    .expect("repository state should exist")
    .map(|bytes| String::from_utf8(bytes).expect("index should be valid UTF-8"))
}

fn head_text(fs: &FakeFs, path: &str) -> Option<String> {
    fs.with_git_state(dot_git(), false, |state| {
        state.head_contents.get(&repo_path(path)).cloned()
    })
    .expect("repository state should exist")
    .map(|bytes| String::from_utf8(bytes).expect("head should be valid UTF-8"))
}

fn is_unmerged(fs: &FakeFs, path: &str) -> bool {
    fs.with_git_state(dot_git(), false, |state| {
        state.unmerged_paths.contains_key(&repo_path(path))
    })
    .expect("repository state should exist")
}

fn commit_count(fs: &FakeFs) -> usize {
    fs.with_git_state(dot_git(), false, |state| state.commit_history.len())
        .expect("repository state should exist")
}

fn merge_in_progress(fs: &FakeFs) -> bool {
    fs.with_git_state(dot_git(), false, |state| {
        state.refs.contains_key("MERGE_HEAD")
    })
    .expect("repository state should exist")
}

fn rebase_in_progress(fs: &FakeFs) -> bool {
    fs.with_git_state(dot_git(), false, |state| state.rebase_session.is_some())
        .expect("repository state should exist")
}

fn current_branch(fs: &FakeFs) -> Option<String> {
    fs.with_git_state(dot_git(), false, |state| state.current_branch_name.clone())
        .expect("repository state should exist")
}

struct Harness {
    fs: Arc<FakeFs>,
    window: WindowHandle<MultiWorkspace>,
    workspace: Entity<Workspace>,
    repository: Entity<Repository>,
    cx: VisualTestContext,
}

impl Harness {
    async fn open(cx: &mut TestAppContext, setup: impl FnOnce(&FakeFs)) -> Self {
        init_test(cx);
        let fs = FakeFs::new(cx.background_executor.clone());
        fs.insert_tree(path!("/project"), json!({ ".git": {} }))
            .await;
        setup(fs.as_ref());

        let project = Project::test(fs.clone(), [Path::new(path!("/project"))], cx).await;
        let window =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = window
            .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
            .expect("window should be open");
        let mut window_cx = VisualTestContext::from_window(window.into(), cx);
        window_cx
            .update(|_, cx| {
                project
                    .read(cx)
                    .worktrees(cx)
                    .next()
                    .expect("project should have a worktree")
                    .read(cx)
                    .as_local()
                    .expect("worktree should be local")
                    .scan_complete()
            })
            .await;
        window_cx.run_until_parked();
        let repository = project
            .read_with(&window_cx, |project, cx| project.active_repository(cx))
            .expect("project should have a repository");
        Self {
            fs,
            window,
            workspace,
            repository,
            cx: window_cx,
        }
    }

    fn branch_context(&self) -> BranchContext {
        BranchContext::new(self.workspace.downgrade(), self.repository.clone())
    }

    fn conflict_context(&self) -> ConflictContext {
        ConflictContext {
            workspace: self.workspace.downgrade(),
            window: self.window.into(),
            repository: self.repository.clone(),
        }
    }

    fn merge(&mut self, branch: &'static str) {
        let context = self.branch_context();
        self.cx.update(|window, cx| {
            merge_into_current(context, RefTarget::local(branch), window, cx);
        });
        self.cx.run_until_parked();
    }

    fn rebase_onto(&mut self, branch: &'static str) {
        let context = self.branch_context();
        self.cx.update(|window, cx| {
            rebase_current_onto(context, RefTarget::local(branch), window, cx);
        });
        self.cx.run_until_parked();
    }

    fn focus_workspace(&mut self) {
        self.workspace
            .update_in(&mut self.cx, |workspace, window, cx| {
                let focus_handle = workspace.focus_handle(cx);
                window.focus(&focus_handle, cx);
            });
    }

    fn dialog(&mut self) -> OpenConflictsDialog {
        find_open_dialog(&mut self.cx).expect("the conflicts dialog should be open")
    }

    fn dialog_windows(&self) -> usize {
        self.cx
            .windows()
            .into_iter()
            .filter(|window| window.downcast::<DialogWindowShell>().is_some())
            .count()
    }

    fn workspace_blocked(&mut self) -> bool {
        self.workspace
            .update_in(&mut self.cx, |workspace, window, cx| {
                workspace.has_active_modal(window, cx)
            })
    }

    fn merge_window(&self, dialog: &OpenConflictsDialog) -> AnyWindowHandle {
        let dialog_window = AnyWindowHandle::from(dialog.window);
        self.cx
            .windows()
            .into_iter()
            .find(|window| {
                *window != dialog_window && window.downcast::<DialogWindowShell>().is_some()
            })
            .expect("the merge window should be open")
    }

    fn dispatch_in(&self, window: AnyWindowHandle, action: impl Action) {
        let mut window_cx = VisualTestContext::from_window(window, &self.cx);
        window_cx.dispatch_action(action);
        window_cx.run_until_parked();
    }

    fn place_caret_on_first_change(&self, window: AnyWindowHandle) {
        let mut window_cx = VisualTestContext::from_window(window, &self.cx);
        window_cx.update(|window, cx| {
            window.simulate_next_frame(cx);
        });
        window_cx.run_until_parked();
    }
}

#[gpui::test]
async fn merge_tool_merge_with_conflicts_opens_the_dialog_and_blocks_the_workspace(
    cx: &mut TestAppContext,
) {
    let mut harness = Harness::open(cx, |fs| plan_merge(fs, &[TEXT_CONFLICT])).await;
    assert_eq!(harness.dialog_windows(), 0);
    assert!(!harness.workspace_blocked());

    harness.merge(FEATURE_BRANCH);

    assert_eq!(harness.dialog_windows(), 1);
    assert!(harness.workspace_blocked());
    let mut dialog = harness.dialog();
    assert_eq!(dialog.unresolved_files(), vec!["a.txt".to_string()]);
    assert!(merge_in_progress(&harness.fs));

    dialog.close();
    harness.cx.run_until_parked();

    assert_eq!(harness.dialog_windows(), 0);
    assert!(!harness.workspace_blocked());
    assert!(is_unmerged(&harness.fs, "a.txt"));
    assert!(merge_in_progress(&harness.fs));
}

#[gpui::test]
async fn merge_tool_resolve_conflicts_action_opens_the_dialog_for_existing_conflicts(
    cx: &mut TestAppContext,
) {
    let mut harness = Harness::open(cx, |fs| plan_existing_conflicts(fs, &[TEXT_CONFLICT])).await;
    assert_eq!(harness.dialog_windows(), 0);

    harness.focus_workspace();
    harness.cx.dispatch_action(ResolveConflicts);
    harness.cx.run_until_parked();

    assert_eq!(harness.dialog_windows(), 1);
    assert!(harness.workspace_blocked());
    assert_eq!(
        harness.dialog().unresolved_files(),
        vec!["a.txt".to_string()]
    );
}

#[gpui::test]
async fn merge_tool_accept_yours_stages_nothing_until_the_dialog_is_closed(
    cx: &mut TestAppContext,
) {
    let mut harness = Harness::open(cx, |fs| plan_merge(fs, &[TEXT_CONFLICT])).await;
    harness.merge(FEATURE_BRANCH);
    let mut dialog = harness.dialog();

    dialog.accept(Side::Left);

    assert_eq!(
        read_file(&harness.fs, path!("/project/a.txt")),
        CONFLICT_OURS
    );
    assert!(is_unmerged(&harness.fs, "a.txt"));
    assert_eq!(index_text(&harness.fs, "a.txt"), None);
    assert_eq!(harness.dialog_windows(), 1);

    dialog.close();
    harness.cx.run_until_parked();

    assert_eq!(
        index_text(&harness.fs, "a.txt").as_deref(),
        Some(CONFLICT_OURS)
    );
    assert!(!is_unmerged(&harness.fs, "a.txt"));
    assert_eq!(harness.dialog_windows(), 0);
    assert!(!harness.workspace_blocked());
    assert!(merge_in_progress(&harness.fs));
    assert_eq!(commit_count(&harness.fs), 0);
}

#[gpui::test]
async fn merge_tool_resolve_all_simple_conflicts_resolves_only_non_overlapping_files(
    cx: &mut TestAppContext,
) {
    let mut harness = Harness::open(cx, |fs| {
        plan_merge(fs, &[AUTO_RESOLVABLE, SECOND_TEXT_CONFLICT])
    })
    .await;
    harness.merge(FEATURE_BRANCH);
    let mut dialog = harness.dialog();
    assert_eq!(
        dialog.unresolved_files(),
        vec!["a.txt".to_string(), "b.txt".to_string()]
    );

    dialog.resolve_all_simple_conflicts();

    assert_eq!(dialog.resolved_files(), vec!["a.txt".to_string()]);
    assert_eq!(dialog.unresolved_files(), vec!["b.txt".to_string()]);
    assert_eq!(read_file(&harness.fs, path!("/project/a.txt")), AUTO_MERGED);
    assert_eq!(
        dialog.auto_resolve_status().as_deref(),
        Some("2 conflicts resolved. 1 conflict in 1 file still requires attention")
    );
    assert!(is_unmerged(&harness.fs, "a.txt"));
    assert!(is_unmerged(&harness.fs, "b.txt"));
}

#[gpui::test]
async fn merge_tool_merge_window_choices_are_written_and_staged_when_the_dialog_closes(
    cx: &mut TestAppContext,
) {
    let mut harness = Harness::open(cx, |fs| plan_merge(fs, &[TWO_CHANGE_CONFLICT])).await;
    harness.merge(FEATURE_BRANCH);
    let mut dialog = harness.dialog();

    dialog.review();
    let merge_window = harness.merge_window(&dialog);
    harness.place_caret_on_first_change(merge_window);
    harness.dispatch_in(merge_window, IgnoreRightSide);
    harness.dispatch_in(merge_window, AcceptLeftSide);
    harness.dispatch_in(merge_window, NextDifference);
    harness.dispatch_in(merge_window, IgnoreLeftSide);
    harness.dispatch_in(merge_window, AcceptRightSide);
    harness.dispatch_in(merge_window, ApplyChanges);
    harness.cx.run_until_parked();

    assert_eq!(harness.dialog_windows(), 1);
    assert_eq!(read_file(&harness.fs, path!("/project/a.txt")), TEN_CHOSEN);
    assert!(is_unmerged(&harness.fs, "a.txt"));

    dialog.close();
    harness.cx.run_until_parked();

    assert_eq!(
        index_text(&harness.fs, "a.txt").as_deref(),
        Some(TEN_CHOSEN)
    );
    assert!(!is_unmerged(&harness.fs, "a.txt"));
    assert_eq!(harness.dialog_windows(), 0);
}

#[gpui::test]
async fn merge_tool_rebase_swaps_yours_and_theirs_and_continues_the_rebase_on_close(
    cx: &mut TestAppContext,
) {
    let mut harness = Harness::open(cx, |fs| plan_rebase(fs, &[TEXT_CONFLICT])).await;
    harness.rebase_onto(MAIN_BRANCH);
    let mut dialog = harness.dialog();
    assert_eq!(
        dialog.column_titles(),
        ("Yours (topic)".to_string(), "Theirs (main)".to_string())
    );
    assert!(rebase_in_progress(&harness.fs));
    assert_eq!(current_branch(&harness.fs), None);

    dialog.accept(Side::Left);

    assert_eq!(
        read_file(&harness.fs, path!("/project/a.txt")),
        CONFLICT_THEIRS
    );

    dialog.close();
    harness.cx.run_until_parked();

    assert_eq!(
        index_text(&harness.fs, "a.txt").as_deref(),
        Some(CONFLICT_THEIRS)
    );
    assert!(!rebase_in_progress(&harness.fs));
    assert_eq!(current_branch(&harness.fs).as_deref(), Some(TOPIC_BRANCH));
    assert_eq!(harness.dialog_windows(), 0);
}

#[gpui::test]
async fn merge_tool_accept_and_finish_commits_the_merge_while_merging(cx: &mut TestAppContext) {
    let mut harness = Harness::open(cx, |fs| plan_merge(fs, &[TEXT_CONFLICT])).await;
    harness.merge(FEATURE_BRANCH);
    let mut dialog = harness.dialog();
    dialog.accept(Side::Right);
    assert_eq!(commit_count(&harness.fs), 0);

    dialog.accept_and_finish();
    harness.cx.run_until_parked();

    assert_eq!(commit_count(&harness.fs), 1);
    assert!(!merge_in_progress(&harness.fs));
    assert!(!is_unmerged(&harness.fs, "a.txt"));
    assert_eq!(
        head_text(&harness.fs, "a.txt").as_deref(),
        Some(CONFLICT_THEIRS)
    );
    assert_eq!(harness.dialog_windows(), 0);
}

#[gpui::test]
async fn merge_tool_pull_without_a_merge_in_progress_resolves_conflicts_without_committing(
    cx: &mut TestAppContext,
) {
    let mut harness = Harness::open(cx, |fs| {
        plan_existing_conflicts(fs, &[TEXT_CONFLICT]);
        fs.set_operation_in_progress_for_repo(dot_git(), None);
    })
    .await;
    assert!(!merge_in_progress(&harness.fs));
    let context = harness.conflict_context();
    harness.cx.update(|_, cx| {
        resolve_conflicts_after_pull(context, false, cx);
    });
    harness.cx.run_until_parked();
    let mut dialog = harness.dialog();
    dialog.accept(Side::Right);

    dialog.close();
    harness.cx.run_until_parked();

    assert_eq!(harness.dialog_windows(), 0);
    assert_eq!(
        index_text(&harness.fs, "a.txt").as_deref(),
        Some(CONFLICT_THEIRS)
    );
    assert!(!is_unmerged(&harness.fs, "a.txt"));
    assert_eq!(commit_count(&harness.fs), 0);
}

#[gpui::test]
async fn merge_tool_pull_commits_the_merge_once_everything_is_resolved_without_a_finish_request(
    cx: &mut TestAppContext,
) {
    let mut harness = Harness::open(cx, |fs| plan_existing_conflicts(fs, &[TEXT_CONFLICT])).await;
    assert!(merge_in_progress(&harness.fs));
    let context = harness.conflict_context();
    harness.cx.update(|_, cx| {
        resolve_conflicts_after_pull(context, false, cx);
    });
    harness.cx.run_until_parked();
    let mut dialog = harness.dialog();
    dialog.accept(Side::Right);
    assert_eq!(commit_count(&harness.fs), 0);

    dialog.close();
    harness.cx.run_until_parked();

    assert_eq!(commit_count(&harness.fs), 1);
    assert!(!merge_in_progress(&harness.fs));
    assert_eq!(
        head_text(&harness.fs, "a.txt").as_deref(),
        Some(CONFLICT_THEIRS)
    );
    assert_eq!(harness.dialog_windows(), 0);
}
