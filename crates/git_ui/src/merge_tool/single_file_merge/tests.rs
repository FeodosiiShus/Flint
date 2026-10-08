use std::{path::Path, sync::Arc};

use git::repository::{ConflictSide, RepositoryOperation, repo_path};
use gpui::{Action, Entity, TestAppContext, VisualTestContext, WindowHandle};
use project::{FakeFs, Project, git_store::Repository};
use serde_json::json;
use settings::SettingsStore;
use theme::LoadThemes;
use util::path;
use workspace::{MultiWorkspace, Workspace};

use super::apply::conflict_side_for_result;
use super::open_single_file_merge;
use super::registry::{self, MergeKey};
use crate::merge_tool::dialog_window::DialogWindowShell;
use crate::merge_tool::merge_viewer::actions::{AcceptLeft, SaveAndClose};
use crate::merge_tool::merge_window::MergeResult;

const FILE: &str = "a.txt";
const BASE: &str = "x\n";
const OURS: &str = "y\n";
const THEIRS: &str = "z\n";

struct Harness {
    fs: Arc<FakeFs>,
    repository: Entity<Repository>,
    workspace: Entity<Workspace>,
    workspace_cx: VisualTestContext,
}

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

async fn open_harness(
    base: Option<&str>,
    ours: Option<&str>,
    theirs: Option<&str>,
    operation: Option<RepositoryOperation>,
    cx: &mut TestAppContext,
) -> Harness {
    init_test(cx);
    let fs = FakeFs::new(cx.background_executor.clone());
    fs.insert_tree(path!("/project"), json!({ ".git": {} }))
        .await;
    fs.set_branch_name(dot_git(), Some("main"));
    fs.set_conflict_for_repo(dot_git(), FILE, base, ours, theirs);
    if let Some(operation) = operation {
        fs.set_operation_in_progress_for_repo(dot_git(), Some(operation));
    }

    let project = Project::test(fs.clone(), [Path::new(path!("/project"))], cx).await;
    let window_handle =
        cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
    let workspace = window_handle
        .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
        .expect("window should be open");
    let mut workspace_cx = VisualTestContext::from_window(window_handle.into(), cx);
    workspace_cx
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
    workspace_cx.run_until_parked();
    let repository = project
        .read_with(&workspace_cx, |project, cx| project.active_repository(cx))
        .expect("project should have a repository");
    Harness {
        fs,
        repository,
        workspace,
        workspace_cx,
    }
}

fn open_merge(harness: &mut Harness) {
    let workspace = harness.workspace.downgrade();
    let repository = harness.repository.clone();
    harness.workspace_cx.update(|window, cx| {
        open_single_file_merge(workspace, repository, repo_path(FILE), window, cx);
    });
    harness.workspace_cx.run_until_parked();
}

fn merge_window(harness: &mut Harness) -> Option<WindowHandle<DialogWindowShell>> {
    let repository_id = harness
        .repository
        .read_with(&harness.workspace_cx, |repository, _| repository.id);
    let key = MergeKey::new(repository_id, repo_path(FILE));
    harness
        .workspace_cx
        .update(|_, cx| registry::open_window(&key, cx))
}

fn dialog_window_count(cx: &TestAppContext) -> usize {
    cx.windows()
        .into_iter()
        .filter(|window| window.downcast::<DialogWindowShell>().is_some())
        .count()
}

fn dispatch_in_merge_window(harness: &mut Harness, action: impl Action, cx: &mut TestAppContext) {
    let window = merge_window(harness).expect("the merge window should be open");
    let mut merge_cx = VisualTestContext::from_window(window.into(), cx);
    merge_cx.dispatch_action(action);
    merge_cx.run_until_parked();
    harness.workspace_cx.run_until_parked();
}

fn working_text(harness: &Harness) -> Option<String> {
    harness
        .fs
        .read_file_sync(path!("/project/a.txt"))
        .ok()
        .map(|bytes| String::from_utf8(bytes).expect("working file should be valid UTF-8"))
}

fn index_text(harness: &Harness) -> Option<String> {
    harness
        .fs
        .with_git_state(dot_git(), false, |state| {
            state.index_contents.get(&repo_path(FILE)).cloned()
        })
        .expect("repository state should exist")
        .map(|bytes| String::from_utf8(bytes).expect("index should be valid UTF-8"))
}

fn is_conflicted(harness: &Harness) -> bool {
    harness
        .repository
        .read_with(&harness.workspace_cx, |repository, _| {
            repository
                .status_for_path(&repo_path(FILE))
                .is_some_and(|entry| entry.status.is_conflicted())
        })
}

#[test]
fn merge_tool_single_file_git_side_follows_the_result_and_the_reversal() {
    assert_eq!(
        conflict_side_for_result(MergeResult::Left, false),
        Some(ConflictSide::Ours)
    );
    assert_eq!(
        conflict_side_for_result(MergeResult::Right, false),
        Some(ConflictSide::Theirs)
    );
    assert_eq!(
        conflict_side_for_result(MergeResult::Left, true),
        Some(ConflictSide::Theirs)
    );
    assert_eq!(
        conflict_side_for_result(MergeResult::Right, true),
        Some(ConflictSide::Ours)
    );
    assert_eq!(conflict_side_for_result(MergeResult::Resolved, true), None);
}

#[gpui::test]
async fn merge_tool_single_file_opens_one_merge_window_for_a_conflicted_file(
    cx: &mut TestAppContext,
) {
    let mut harness = open_harness(Some(BASE), Some(OURS), Some(THEIRS), None, cx).await;
    open_merge(&mut harness);

    assert!(merge_window(&mut harness).is_some());
    assert_eq!(dialog_window_count(cx), 1);
    assert!(is_conflicted(&harness));
    assert_eq!(index_text(&harness), None);
}

#[gpui::test]
async fn merge_tool_single_file_accept_left_writes_ours_and_stages_the_file(
    cx: &mut TestAppContext,
) {
    let mut harness = open_harness(Some(BASE), Some(OURS), Some(THEIRS), None, cx).await;
    open_merge(&mut harness);
    dispatch_in_merge_window(&mut harness, AcceptLeft, cx);

    assert_eq!(working_text(&harness).as_deref(), Some(OURS));
    assert_eq!(index_text(&harness).as_deref(), Some(OURS));
    assert!(!is_conflicted(&harness));
    assert!(merge_window(&mut harness).is_none());
    assert_eq!(dialog_window_count(cx), 0);
}

#[gpui::test]
async fn merge_tool_single_file_cancel_leaves_the_file_unresolved_and_unchanged(
    cx: &mut TestAppContext,
) {
    let mut harness = open_harness(Some(BASE), Some(OURS), Some(THEIRS), None, cx).await;
    let conflicted_text = working_text(&harness);
    open_merge(&mut harness);
    dispatch_in_merge_window(&mut harness, SaveAndClose, cx);

    assert_eq!(working_text(&harness), conflicted_text);
    assert_eq!(index_text(&harness), None);
    assert!(is_conflicted(&harness));
    assert!(merge_window(&mut harness).is_none());
    assert_eq!(dialog_window_count(cx), 0);
}

#[gpui::test]
async fn merge_tool_single_file_accept_left_during_rebase_takes_the_git_theirs_side(
    cx: &mut TestAppContext,
) {
    let mut harness = open_harness(
        Some(BASE),
        None,
        Some(THEIRS),
        Some(RepositoryOperation::Rebase),
        cx,
    )
    .await;
    open_merge(&mut harness);
    dispatch_in_merge_window(&mut harness, AcceptLeft, cx);

    assert_eq!(working_text(&harness).as_deref(), Some(THEIRS));
    assert_eq!(index_text(&harness).as_deref(), Some(THEIRS));
    assert!(!is_conflicted(&harness));
}

#[gpui::test]
async fn merge_tool_single_file_second_open_reuses_the_existing_window(cx: &mut TestAppContext) {
    let mut harness = open_harness(Some(BASE), Some(OURS), Some(THEIRS), None, cx).await;
    open_merge(&mut harness);
    let first = merge_window(&mut harness).expect("the merge window should be open");
    open_merge(&mut harness);

    let second = merge_window(&mut harness).expect("the merge window should stay registered");
    assert!(first == second);
    assert_eq!(dialog_window_count(cx), 1);
}
