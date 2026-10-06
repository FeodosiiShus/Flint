use std::{ops::Range, path::Path, sync::Arc};

use editor::{
    Editor, SelectionEffects, ToPoint as _,
    actions::{Redo, Undo},
};
use git::{
    repository::repo_path,
    status::{UnmergedStatus, UnmergedStatusCode},
};
use gpui::{Action, Entity, Focusable as _, Modifiers, TestAppContext, VisualTestContext, point};
use language::Point;
use project::{FakeFs, Project, git_store::Repository};
use serde_json::json;
use settings::SettingsStore;
use theme::LoadThemes;
use three_way_merge::{ChunkState, IgnorePolicy, Side, SideState};
use util::path;
use workspace::{MultiWorkspace, Workspace};

use super::{
    AcceptLeftSide, AcceptRightSide, AcceptTheirs, AcceptYours, AppendRightSide, ApplyChanges,
    ApplyNonConflictingAll, ApplyNonConflictingLeft, ApplyNonConflictingRight, ConflictsDialog,
    IgnoreLeftSide, IgnoreRightSide, MergeBranches, MergeView, NextDifference, PreviousDifference,
    ResolveSimpleConflict, ResolveSimpleConflicts, ResolveUsingLeft, ResolveUsingRight,
    RevertConflictResolution, SaveAndClose, ToggleSynchronizeScrolling, accept_side,
    branch_from_merge_message,
    merge_view::{MergeConflictRows, MergeModifiedRows},
    open_conflicts_dialog, open_conflicts_dialog_if_conflicted, open_merge_tool,
};
use crate::git_panel::GitPanel;

const BASE: &str = "a\nb\nc\nd\ne\nf\ng\nh\ni\n";
const OURS: &str = "A\nb\nc\nd\nE\nf\ng\nh\nI\n";
const THEIRS: &str = "a\nb\nC\nd\nX\nf\ng\nh\nI\n";
const MERGE_MESSAGE: &str = "Merge branch 'feature'\n";

struct Conflict {
    path: &'static str,
    base: Option<String>,
    ours: Option<String>,
    theirs: Option<String>,
}

impl Conflict {
    fn new(
        path: &'static str,
        base: Option<&str>,
        ours: Option<&str>,
        theirs: Option<&str>,
    ) -> Self {
        Self {
            path,
            base: base.map(str::to_string),
            ours: ours.map(str::to_string),
            theirs: theirs.map(str::to_string),
        }
    }
}

fn fixture(path: &'static str) -> Conflict {
    Conflict::new(path, Some(BASE), Some(OURS), Some(THEIRS))
}

struct MergeTest {
    fs: Arc<FakeFs>,
    project: Entity<Project>,
    workspace: Entity<Workspace>,
    repository: Entity<Repository>,
    cx: VisualTestContext,
}

fn dot_git() -> &'static Path {
    Path::new(path!("/project/.git"))
}

fn init_test(cx: &mut TestAppContext) {
    zlog::init_test();
    cx.update(|cx| {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
        theme_settings::init(LoadThemes::JustBase, cx);
        editor::init(cx);
        crate::init(cx);
    });
}

async fn setup(
    conflicts: &[Conflict],
    merge_message: Option<&str>,
    cx: &mut TestAppContext,
) -> MergeTest {
    init_test(cx);
    let fs = FakeFs::new(cx.background_executor.clone());
    fs.insert_tree(path!("/project"), json!({ ".git": {} }))
        .await;
    fs.set_branch_name(dot_git(), Some("main"));
    fs.set_merge_message_for_repo(dot_git(), merge_message);
    for conflict in conflicts {
        fs.set_conflict_for_repo(
            dot_git(),
            conflict.path,
            conflict.base.as_deref(),
            conflict.ours.as_deref(),
            conflict.theirs.as_deref(),
        );
    }

    let project = Project::test(fs.clone(), [Path::new(path!("/project"))], cx).await;
    let window_handle =
        cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
    let workspace = window_handle
        .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
        .expect("window should be open");
    let mut cx = VisualTestContext::from_window(window_handle.into(), cx);
    cx.read(|cx| {
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
    cx.run_until_parked();
    let repository = project
        .read_with(&cx, |project, cx| project.active_repository(cx))
        .expect("project should have a repository");

    MergeTest {
        fs,
        project,
        workspace,
        repository,
        cx,
    }
}

async fn open_view(
    workspace: &Entity<Workspace>,
    repository: &Entity<Repository>,
    path: &str,
    cx: &mut VisualTestContext,
) -> Entity<MergeView> {
    let task = workspace.update_in(cx, |workspace, window, cx| {
        open_merge_tool(workspace, repository.clone(), repo_path(path), window, cx)
    });
    task.await.expect("merge tool should open");
    cx.run_until_parked();
    let view = workspace
        .read_with(cx, |workspace, cx| {
            workspace.active_item_as::<MergeView>(cx)
        })
        .expect("merge view should be the active item");
    assert_eq!(
        view.read_with(cx, |view, _| view.repo_path.clone()),
        repo_path(path)
    );
    view
}

fn active_dialog(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Option<Entity<ConflictsDialog>> {
    workspace.read_with(cx, |workspace, cx| {
        workspace.active_modal::<ConflictsDialog>(cx)
    })
}

fn open_dialog(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Entity<ConflictsDialog> {
    workspace.update_in(cx, |workspace, window, cx| {
        open_conflicts_dialog(workspace, window, cx);
    });
    cx.run_until_parked();
    active_dialog(workspace, cx).expect("conflicts dialog should be open")
}

fn dialog_paths(
    dialog: &Entity<ConflictsDialog>,
    cx: &mut VisualTestContext,
) -> (Vec<String>, Vec<String>) {
    dialog.read_with(cx, |dialog, _| {
        let paths = |rows: &[super::conflicts_dialog::ConflictRow]| {
            rows.iter()
                .map(|row| row.repo_path.as_unix_str().to_string())
                .collect::<Vec<_>>()
        };
        (paths(&dialog.unresolved), paths(&dialog.resolved))
    })
}

fn merge_view_count(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> usize {
    workspace.read_with(cx, |workspace, cx| {
        workspace.items_of_type::<MergeView>(cx).count()
    })
}

fn result_text(view: &Entity<MergeView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |view, cx| view.result_editor.read(cx).text(cx))
}

fn status_text(view: &Entity<MergeView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |view, _| view.model.counters().status_text())
}

fn chunk_state(
    view: &Entity<MergeView>,
    chunk_index: usize,
    cx: &mut VisualTestContext,
) -> Option<ChunkState> {
    view.read_with(cx, |view, _| view.model.chunk_state(chunk_index))
}

fn focus_result(view: &Entity<MergeView>, cx: &mut VisualTestContext) {
    view.update_in(cx, |view, window, cx| {
        let focus_handle = view.result_editor.focus_handle(cx);
        window.focus(&focus_handle, cx);
    });
    cx.run_until_parked();
}

fn place_cursor(view: &Entity<MergeView>, row: u32, cx: &mut VisualTestContext) {
    view.update_in(cx, |view, window, cx| {
        let editor = view.result_editor.clone();
        let focus_handle = editor.focus_handle(cx);
        window.focus(&focus_handle, cx);
        editor.update(cx, |editor, cx| {
            editor.change_selections(SelectionEffects::no_scroll(), window, cx, |selections| {
                selections.select_ranges([Point::new(row, 0)..Point::new(row, 0)]);
            });
        });
    });
    cx.run_until_parked();
}

fn run_at<A: Action>(view: &Entity<MergeView>, row: u32, action: A, cx: &mut VisualTestContext) {
    place_cursor(view, row, cx);
    cx.dispatch_action(action);
}

fn cursor_row(view: &Entity<MergeView>, cx: &mut VisualTestContext) -> u32 {
    view.update(cx, |view, cx| {
        view.result_editor.update(cx, |editor, cx| {
            let snapshot = editor.display_snapshot(cx);
            editor.selections.newest::<Point>(&snapshot).head().row
        })
    })
}

fn highlighted_rows<T: 'static>(
    editor: &Entity<Editor>,
    cx: &mut VisualTestContext,
) -> Vec<Range<u32>> {
    editor.read_with(cx, |editor, cx| {
        let snapshot = editor.buffer().read(cx).snapshot(cx);
        editor
            .highlighted_rows::<T>(cx)
            .map(|(range, _)| {
                range.start.to_point(&snapshot).row..range.end.to_point(&snapshot).row
            })
            .collect()
    })
}

fn result_conflict_rows(view: &Entity<MergeView>, cx: &mut VisualTestContext) -> Vec<Range<u32>> {
    let editor = view.read_with(cx, |view, _| view.result_editor.clone());
    highlighted_rows::<MergeConflictRows>(&editor, cx)
}

fn scroll_result_to(view: &Entity<MergeView>, row: f64, cx: &mut VisualTestContext) {
    view.update_in(cx, |view, window, cx| {
        view.result_editor.update(cx, |editor, cx| {
            editor.set_scroll_position(point(0., row), window, cx);
        });
    });
    cx.run_until_parked();
}

fn scroll_tops(view: &Entity<MergeView>, cx: &mut VisualTestContext) -> (f64, f64, f64) {
    view.update(cx, |view, cx| {
        let left = view
            .left_editor
            .update(cx, |editor, cx| editor.scroll_position(cx).y);
        let result = view
            .result_editor
            .update(cx, |editor, cx| editor.scroll_position(cx).y);
        let right = view
            .right_editor
            .update(cx, |editor, cx| editor.scroll_position(cx).y);
        (left, result, right)
    })
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

fn is_conflicted(repository: &Entity<Repository>, path: &str, cx: &mut VisualTestContext) -> bool {
    repository.read_with(cx, |repository, _| {
        repository
            .status_for_path(&repo_path(path))
            .is_some_and(|entry| entry.status.is_conflicted())
    })
}

#[gpui::test]
async fn test_merge_view_shows_revisions_and_counters(cx: &mut TestAppContext) {
    let MergeTest {
        workspace,
        repository,
        mut cx,
        ..
    } = setup(&[fixture("a.txt")], Some(MERGE_MESSAGE), cx).await;
    let view = open_view(&workspace, &repository, "a.txt", &mut cx).await;

    view.read_with(&mut cx, |view, cx| {
        assert_eq!(view.left_editor.read(cx).text(cx), OURS);
        assert_eq!(view.result_editor.read(cx).text(cx), BASE);
        assert_eq!(view.right_editor.read(cx).text(cx), THEIRS);
        assert_eq!(
            view.branches,
            MergeBranches {
                ours: "main".into(),
                theirs: "feature".into(),
            }
        );
    });
    assert_eq!(status_text(&view, &mut cx), "3 changes. 1 conflict.");

    let (left_editor, result_editor, right_editor) = view.read_with(&mut cx, |view, _| {
        (
            view.left_editor.clone(),
            view.result_editor.clone(),
            view.right_editor.clone(),
        )
    });
    assert_eq!(
        highlighted_rows::<MergeModifiedRows>(&left_editor, &mut cx),
        vec![0..1, 8..9]
    );
    assert_eq!(
        highlighted_rows::<MergeConflictRows>(&left_editor, &mut cx),
        vec![4..5]
    );
    assert_eq!(
        highlighted_rows::<MergeModifiedRows>(&result_editor, &mut cx),
        vec![0..1, 2..3, 8..9]
    );
    assert_eq!(
        highlighted_rows::<MergeConflictRows>(&result_editor, &mut cx),
        vec![4..5]
    );
    assert_eq!(
        highlighted_rows::<MergeModifiedRows>(&right_editor, &mut cx),
        vec![2..3, 8..9]
    );
    assert_eq!(
        highlighted_rows::<MergeConflictRows>(&right_editor, &mut cx),
        vec![4..5]
    );

    let reopened = open_view(&workspace, &repository, "a.txt", &mut cx).await;
    assert_eq!(reopened, view);
    assert_eq!(merge_view_count(&workspace, &mut cx), 1);
}

#[gpui::test]
async fn test_accepting_and_appending_sides(cx: &mut TestAppContext) {
    let MergeTest {
        workspace,
        repository,
        mut cx,
        ..
    } = setup(&[fixture("a.txt")], Some(MERGE_MESSAGE), cx).await;
    let view = open_view(&workspace, &repository, "a.txt", &mut cx).await;

    run_at(&view, 0, AcceptLeftSide, &mut cx);
    assert_eq!(result_text(&view, &mut cx), "A\nb\nc\nd\ne\nf\ng\nh\ni\n");
    assert_eq!(status_text(&view, &mut cx), "2 changes. 1 conflict.");

    run_at(&view, 2, AcceptRightSide, &mut cx);
    assert_eq!(result_text(&view, &mut cx), "A\nb\nC\nd\ne\nf\ng\nh\ni\n");
    assert_eq!(status_text(&view, &mut cx), "1 change. 1 conflict.");

    run_at(&view, 4, AppendRightSide, &mut cx);
    assert_eq!(
        result_text(&view, &mut cx),
        "A\nb\nC\nd\ne\nX\nf\ng\nh\ni\n"
    );
    assert_eq!(status_text(&view, &mut cx), "1 change. 1 conflict.");
    assert_eq!(
        chunk_state(&view, 2, &mut cx),
        Some(ChunkState {
            left: SideState::Pending,
            right: SideState::Applied,
        })
    );

    run_at(&view, 4, AcceptLeftSide, &mut cx);
    assert_eq!(
        result_text(&view, &mut cx),
        "A\nb\nC\nd\ne\nX\nE\nf\ng\nh\ni\n"
    );
    assert_eq!(status_text(&view, &mut cx), "1 change. 0 conflicts.");
    assert_eq!(
        result_conflict_rows(&view, &mut cx),
        Vec::<Range<u32>>::new()
    );

    run_at(&view, 10, IgnoreRightSide, &mut cx);
    assert_eq!(
        result_text(&view, &mut cx),
        "A\nb\nC\nd\ne\nX\nE\nf\ng\nh\ni\n"
    );
    assert_eq!(
        status_text(&view, &mut cx),
        "All changes have been processed."
    );
    assert!(!cx.has_pending_prompt());
}

#[gpui::test]
async fn test_ignoring_and_resolving_with_one_side(cx: &mut TestAppContext) {
    let MergeTest {
        workspace,
        repository,
        mut cx,
        ..
    } = setup(&[fixture("a.txt")], Some(MERGE_MESSAGE), cx).await;
    let view = open_view(&workspace, &repository, "a.txt", &mut cx).await;

    run_at(&view, 0, IgnoreLeftSide, &mut cx);
    assert_eq!(result_text(&view, &mut cx), BASE);
    assert_eq!(status_text(&view, &mut cx), "2 changes. 1 conflict.");
    assert_eq!(
        chunk_state(&view, 0, &mut cx),
        Some(ChunkState {
            left: SideState::Ignored,
            right: SideState::Pending,
        })
    );

    run_at(&view, 4, ResolveUsingRight, &mut cx);
    assert_eq!(result_text(&view, &mut cx), "a\nb\nc\nd\nX\nf\ng\nh\ni\n");
    assert_eq!(status_text(&view, &mut cx), "2 changes. 0 conflicts.");
    assert_eq!(
        chunk_state(&view, 2, &mut cx),
        Some(ChunkState {
            left: SideState::Ignored,
            right: SideState::Applied,
        })
    );

    run_at(&view, 2, ResolveUsingLeft, &mut cx);
    assert_eq!(result_text(&view, &mut cx), "a\nb\nc\nd\nX\nf\ng\nh\ni\n");
    assert_eq!(status_text(&view, &mut cx), "1 change. 0 conflicts.");
    assert_eq!(
        chunk_state(&view, 1, &mut cx),
        Some(ChunkState {
            left: SideState::Pending,
            right: SideState::Ignored,
        })
    );
}

#[gpui::test]
async fn test_apply_non_conflicting_changes_with_undo_and_redo(cx: &mut TestAppContext) {
    let MergeTest {
        workspace,
        repository,
        mut cx,
        ..
    } = setup(&[fixture("a.txt")], Some(MERGE_MESSAGE), cx).await;
    let view = open_view(&workspace, &repository, "a.txt", &mut cx).await;
    focus_result(&view, &mut cx);

    cx.dispatch_action(ApplyNonConflictingLeft);
    assert_eq!(result_text(&view, &mut cx), "A\nb\nc\nd\ne\nf\ng\nh\nI\n");
    assert_eq!(status_text(&view, &mut cx), "1 change. 1 conflict.");

    cx.dispatch_action(ApplyNonConflictingRight);
    assert_eq!(result_text(&view, &mut cx), "A\nb\nC\nd\ne\nf\ng\nh\nI\n");
    assert_eq!(status_text(&view, &mut cx), "0 changes. 1 conflict.");

    cx.dispatch_action(Undo);
    assert_eq!(result_text(&view, &mut cx), "A\nb\nc\nd\ne\nf\ng\nh\nI\n");
    assert_eq!(status_text(&view, &mut cx), "1 change. 1 conflict.");
    assert_eq!(chunk_state(&view, 1, &mut cx), Some(ChunkState::default()));

    cx.dispatch_action(Undo);
    assert_eq!(result_text(&view, &mut cx), BASE);
    assert_eq!(status_text(&view, &mut cx), "3 changes. 1 conflict.");

    cx.dispatch_action(Redo);
    assert_eq!(result_text(&view, &mut cx), "A\nb\nc\nd\ne\nf\ng\nh\nI\n");
    assert_eq!(status_text(&view, &mut cx), "1 change. 1 conflict.");

    cx.dispatch_action(Redo);
    assert_eq!(result_text(&view, &mut cx), "A\nb\nC\nd\ne\nf\ng\nh\nI\n");
    assert_eq!(status_text(&view, &mut cx), "0 changes. 1 conflict.");

    cx.dispatch_action(Undo);
    cx.dispatch_action(Undo);
    assert_eq!(result_text(&view, &mut cx), BASE);
    assert_eq!(status_text(&view, &mut cx), "3 changes. 1 conflict.");

    cx.dispatch_action(ApplyNonConflictingAll);
    assert_eq!(result_text(&view, &mut cx), "A\nb\nC\nd\ne\nf\ng\nh\nI\n");
    assert_eq!(status_text(&view, &mut cx), "0 changes. 1 conflict.");
}

#[gpui::test]
async fn test_undo_and_redo_restore_resolution_states(cx: &mut TestAppContext) {
    let MergeTest {
        workspace,
        repository,
        mut cx,
        ..
    } = setup(&[fixture("a.txt")], Some(MERGE_MESSAGE), cx).await;
    let view = open_view(&workspace, &repository, "a.txt", &mut cx).await;
    let resolved_text = "a\nb\nc\nd\nE\nf\ng\nh\ni\n";

    run_at(&view, 4, ResolveUsingLeft, &mut cx);
    assert_eq!(result_text(&view, &mut cx), resolved_text);
    assert_eq!(status_text(&view, &mut cx), "3 changes. 0 conflicts.");
    assert_eq!(
        result_conflict_rows(&view, &mut cx),
        Vec::<Range<u32>>::new()
    );

    cx.dispatch_action(Undo);
    assert_eq!(result_text(&view, &mut cx), BASE);
    assert_eq!(status_text(&view, &mut cx), "3 changes. 1 conflict.");
    assert_eq!(chunk_state(&view, 2, &mut cx), Some(ChunkState::default()));
    assert_eq!(result_conflict_rows(&view, &mut cx), vec![4..5]);

    cx.dispatch_action(Redo);
    assert_eq!(result_text(&view, &mut cx), resolved_text);
    assert_eq!(status_text(&view, &mut cx), "3 changes. 0 conflicts.");
    assert_eq!(
        chunk_state(&view, 2, &mut cx),
        Some(ChunkState {
            left: SideState::Applied,
            right: SideState::Ignored,
        })
    );

    run_at(&view, 0, IgnoreLeftSide, &mut cx);
    assert_eq!(result_text(&view, &mut cx), resolved_text);
    assert_eq!(status_text(&view, &mut cx), "2 changes. 0 conflicts.");

    cx.dispatch_action(Undo);
    assert_eq!(result_text(&view, &mut cx), resolved_text);
    assert_eq!(status_text(&view, &mut cx), "3 changes. 0 conflicts.");
    assert_eq!(chunk_state(&view, 0, &mut cx), Some(ChunkState::default()));

    cx.dispatch_action(Redo);
    assert_eq!(result_text(&view, &mut cx), resolved_text);
    assert_eq!(status_text(&view, &mut cx), "2 changes. 0 conflicts.");
    assert_eq!(
        chunk_state(&view, 0, &mut cx),
        Some(ChunkState {
            left: SideState::Ignored,
            right: SideState::Pending,
        })
    );
}

#[gpui::test]
async fn test_resolving_simple_conflicts(cx: &mut TestAppContext) {
    let base = "x = 1 + 2\nkeep\nname\n";
    let MergeTest {
        workspace,
        repository,
        mut cx,
        ..
    } = setup(
        &[Conflict::new(
            "a.txt",
            Some(base),
            Some("y = 1 + 2\nkeep\nleft\n"),
            Some("x = 1 + 3\nkeep\nright\n"),
        )],
        Some(MERGE_MESSAGE),
        cx,
    )
    .await;
    let view = open_view(&workspace, &repository, "a.txt", &mut cx).await;
    assert_eq!(status_text(&view, &mut cx), "0 changes. 2 conflicts.");
    focus_result(&view, &mut cx);

    cx.dispatch_action(ResolveSimpleConflicts);
    assert_eq!(result_text(&view, &mut cx), "y = 1 + 3\nkeep\nname\n");
    assert_eq!(status_text(&view, &mut cx), "0 changes. 1 conflict.");

    cx.dispatch_action(Undo);
    assert_eq!(result_text(&view, &mut cx), base);
    assert_eq!(status_text(&view, &mut cx), "0 changes. 2 conflicts.");

    run_at(&view, 0, ResolveSimpleConflict, &mut cx);
    assert_eq!(result_text(&view, &mut cx), "y = 1 + 3\nkeep\nname\n");
    assert_eq!(status_text(&view, &mut cx), "0 changes. 1 conflict.");

    run_at(&view, 2, ResolveSimpleConflict, &mut cx);
    assert_eq!(result_text(&view, &mut cx), "y = 1 + 3\nkeep\nname\n");
    assert_eq!(status_text(&view, &mut cx), "0 changes. 1 conflict.");
}

#[gpui::test]
async fn test_apply_changes_writes_result_and_opens_next_conflict(cx: &mut TestAppContext) {
    let MergeTest {
        fs,
        workspace,
        repository,
        mut cx,
        ..
    } = setup(
        &[fixture("a.txt"), fixture("b.txt")],
        Some(MERGE_MESSAGE),
        cx,
    )
    .await;
    let view = open_view(&workspace, &repository, "a.txt", &mut cx).await;
    focus_result(&view, &mut cx);
    cx.dispatch_action(ApplyNonConflictingAll);
    run_at(&view, 4, ResolveUsingRight, &mut cx);
    assert_eq!(
        status_text(&view, &mut cx),
        "All changes have been processed."
    );

    cx.dispatch_action(ApplyChanges);
    cx.run_until_parked();

    let merged = "A\nb\nC\nd\nX\nf\ng\nh\nI\n";
    assert!(!cx.has_pending_prompt());
    assert_eq!(read_file(&fs, path!("/project/a.txt")), merged);
    assert_eq!(index_text(&fs, "a.txt").as_deref(), Some(merged));
    assert!(!is_conflicted(&repository, "a.txt", &mut cx));
    assert!(is_conflicted(&repository, "b.txt", &mut cx));
    let next_view = workspace
        .read_with(&mut cx, |workspace, cx| {
            workspace.active_item_as::<MergeView>(cx)
        })
        .expect("the next conflicted file should be open");
    assert_eq!(
        next_view.read_with(&mut cx, |view, _| view.repo_path.clone()),
        repo_path("b.txt")
    );
    assert_eq!(merge_view_count(&workspace, &mut cx), 1);
}

#[gpui::test]
async fn test_apply_changes_with_unresolved_changes_asks_for_confirmation(cx: &mut TestAppContext) {
    let MergeTest {
        fs,
        workspace,
        repository,
        mut cx,
        ..
    } = setup(&[fixture("a.txt")], Some(MERGE_MESSAGE), cx).await;
    let conflicted_text = read_file(&fs, path!("/project/a.txt"));
    let view = open_view(&workspace, &repository, "a.txt", &mut cx).await;
    let resolved_text = "a\nb\nc\nd\nE\nf\ng\nh\ni\n";
    run_at(&view, 4, ResolveUsingLeft, &mut cx);

    cx.dispatch_action(ApplyChanges);
    cx.run_until_parked();
    assert_eq!(
        cx.pending_prompt(),
        Some((
            "Unresolved changes remain".to_string(),
            "3 changes and 0 conflicts are not resolved. Apply the result anyway?".to_string(),
        ))
    );
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(read_file(&fs, path!("/project/a.txt")), conflicted_text);
    assert!(is_conflicted(&repository, "a.txt", &mut cx));
    assert_eq!(result_text(&view, &mut cx), resolved_text);
    assert_eq!(merge_view_count(&workspace, &mut cx), 1);
    assert!(active_dialog(&workspace, &mut cx).is_none());

    cx.dispatch_action(ApplyChanges);
    cx.run_until_parked();
    cx.simulate_prompt_answer("Apply");
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(read_file(&fs, path!("/project/a.txt")), resolved_text);
    assert!(!is_conflicted(&repository, "a.txt", &mut cx));
    assert_eq!(merge_view_count(&workspace, &mut cx), 0);
    assert!(active_dialog(&workspace, &mut cx).is_some());
}

#[gpui::test]
async fn test_save_and_close_keeps_the_partial_result(cx: &mut TestAppContext) {
    let MergeTest {
        fs,
        workspace,
        repository,
        mut cx,
        ..
    } = setup(&[fixture("a.txt")], Some(MERGE_MESSAGE), cx).await;
    let conflicted_text = read_file(&fs, path!("/project/a.txt"));
    let view = open_view(&workspace, &repository, "a.txt", &mut cx).await;
    let resolved_text = "a\nb\nc\nd\nE\nf\ng\nh\ni\n";
    run_at(&view, 4, ResolveUsingLeft, &mut cx);

    cx.dispatch_action(SaveAndClose);
    cx.run_until_parked();
    assert_eq!(merge_view_count(&workspace, &mut cx), 0);
    assert_eq!(read_file(&fs, path!("/project/a.txt")), conflicted_text);
    assert!(is_conflicted(&repository, "a.txt", &mut cx));
    let dialog = active_dialog(&workspace, &mut cx).expect("conflicts dialog should be open");
    dialog.read_with(&mut cx, |dialog, cx| {
        let row = dialog
            .unresolved
            .first()
            .expect("the file should still be unresolved");
        assert_eq!(row.repo_path, repo_path("a.txt"));
        assert_eq!(dialog.badge(row, cx).as_deref(), Some("1/4"));
    });

    cx.dispatch_action(menu::Cancel);
    cx.run_until_parked();
    assert!(active_dialog(&workspace, &mut cx).is_none());

    let reopened = open_view(&workspace, &repository, "a.txt", &mut cx).await;
    assert_eq!(result_text(&reopened, &mut cx), resolved_text);
    assert_eq!(status_text(&reopened, &mut cx), "3 changes. 0 conflicts.");
    assert_eq!(
        chunk_state(&reopened, 2, &mut cx),
        Some(ChunkState {
            left: SideState::Applied,
            right: SideState::Ignored,
        })
    );
}

#[gpui::test]
async fn test_conflicts_dialog_accepts_sides_and_finishes(cx: &mut TestAppContext) {
    let MergeTest {
        fs,
        workspace,
        mut cx,
        ..
    } = setup(
        &[
            fixture("a.txt"),
            Conflict::new("b.txt", Some("one\n"), Some("ours\n"), Some("theirs\n")),
        ],
        Some(MERGE_MESSAGE),
        cx,
    )
    .await;
    let async_window = cx.update(|window, cx| window.to_async(cx));
    let panel = GitPanel::load(workspace.downgrade(), async_window)
        .await
        .expect("git panel should load");
    workspace.update_in(&mut cx, |workspace, window, cx| {
        workspace.add_panel(panel.clone(), window, cx);
    });
    cx.run_until_parked();

    let dialog = open_dialog(&workspace, &mut cx);
    assert_eq!(
        dialog_paths(&dialog, &mut cx),
        (vec!["a.txt".to_string(), "b.txt".to_string()], Vec::new())
    );
    dialog.read_with(&mut cx, |dialog, cx| {
        assert_eq!(
            dialog.branches,
            MergeBranches {
                ours: "main".into(),
                theirs: "feature".into(),
            }
        );
        assert_eq!(dialog.selected, Some(repo_path("a.txt")));
        let modified = Some(UnmergedStatus {
            first_head: UnmergedStatusCode::Updated,
            second_head: UnmergedStatusCode::Updated,
        });
        assert_eq!(dialog.unresolved[0].status, modified);
        assert_eq!(
            dialog.badge(&dialog.unresolved[0], cx).as_deref(),
            Some("0/4")
        );
        assert_eq!(
            dialog.badge(&dialog.unresolved[1], cx).as_deref(),
            Some("0/1")
        );
    });

    cx.dispatch_action(AcceptYours);
    cx.run_until_parked();
    assert_eq!(read_file(&fs, path!("/project/a.txt")), OURS);
    assert_eq!(
        dialog_paths(&dialog, &mut cx),
        (vec!["b.txt".to_string()], vec!["a.txt".to_string()])
    );

    dialog.update_in(&mut cx, |dialog, window, cx| {
        dialog.accept_and_finish(window, cx);
    });
    cx.run_until_parked();
    assert!(active_dialog(&workspace, &mut cx).is_some());

    dialog.update(&mut cx, |dialog, cx| dialog.select(repo_path("b.txt"), cx));
    cx.dispatch_action(AcceptTheirs);
    cx.run_until_parked();
    assert_eq!(read_file(&fs, path!("/project/b.txt")), "theirs\n");
    assert_eq!(
        dialog_paths(&dialog, &mut cx),
        (Vec::new(), vec!["a.txt".to_string(), "b.txt".to_string()])
    );

    dialog.update_in(&mut cx, |dialog, window, cx| {
        dialog.accept_and_finish(window, cx);
    });
    cx.run_until_parked();
    assert!(active_dialog(&workspace, &mut cx).is_none());
    assert!(cx.update(|window, cx| {
        panel
            .read(cx)
            .commit_editor
            .focus_handle(cx)
            .is_focused(window)
    }));
}

#[gpui::test]
async fn test_conflicts_dialog_reverts_resolution_and_resolves_manually(cx: &mut TestAppContext) {
    let MergeTest {
        fs,
        workspace,
        repository,
        mut cx,
        ..
    } = setup(
        &[fixture("a.txt"), fixture("b.txt")],
        Some(MERGE_MESSAGE),
        cx,
    )
    .await;
    let conflicted_text = read_file(&fs, path!("/project/a.txt"));
    let dialog = open_dialog(&workspace, &mut cx);

    cx.dispatch_action(AcceptYours);
    cx.run_until_parked();
    assert!(!is_conflicted(&repository, "a.txt", &mut cx));
    assert_eq!(
        dialog_paths(&dialog, &mut cx),
        (vec!["b.txt".to_string()], vec!["a.txt".to_string()])
    );

    dialog.update(&mut cx, |dialog, cx| dialog.select(repo_path("a.txt"), cx));
    cx.dispatch_action(RevertConflictResolution);
    cx.run_until_parked();
    assert!(is_conflicted(&repository, "a.txt", &mut cx));
    assert_eq!(read_file(&fs, path!("/project/a.txt")), conflicted_text);
    assert_eq!(
        dialog_paths(&dialog, &mut cx),
        (vec!["a.txt".to_string(), "b.txt".to_string()], Vec::new())
    );

    dialog.update(&mut cx, |dialog, cx| dialog.select(repo_path("b.txt"), cx));
    cx.dispatch_action(menu::Confirm);
    cx.run_until_parked();
    assert!(active_dialog(&workspace, &mut cx).is_none());
    let view = workspace
        .read_with(&mut cx, |workspace, cx| {
            workspace.active_item_as::<MergeView>(cx)
        })
        .expect("resolving manually should open the merge tool");
    assert_eq!(
        view.read_with(&mut cx, |view, _| view.repo_path.clone()),
        repo_path("b.txt")
    );
}

#[gpui::test]
async fn test_resolve_all_simple_conflicts_from_the_dialog(cx: &mut TestAppContext) {
    let MergeTest {
        fs,
        workspace,
        repository,
        mut cx,
        ..
    } = setup(
        &[
            Conflict::new(
                "a.txt",
                Some("let value = compute(a, b);\n"),
                Some("let total = compute(a, b);\n"),
                Some("let value = compute(a, c);\n"),
            ),
            fixture("b.txt"),
        ],
        Some(MERGE_MESSAGE),
        cx,
    )
    .await;
    let dialog = open_dialog(&workspace, &mut cx);

    dialog.update_in(&mut cx, |dialog, window, cx| {
        dialog.resolve_all_simple_conflicts(window, cx);
    });
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(
        read_file(&fs, path!("/project/a.txt")),
        "let total = compute(a, c);\n"
    );
    assert!(!is_conflicted(&repository, "a.txt", &mut cx));
    assert!(is_conflicted(&repository, "b.txt", &mut cx));
    assert_eq!(
        dialog_paths(&dialog, &mut cx),
        (vec!["b.txt".to_string()], vec!["a.txt".to_string()])
    );
    dialog.read_with(&mut cx, |dialog, cx| {
        assert_eq!(
            dialog.badge(&dialog.unresolved[0], cx).as_deref(),
            Some("3/4")
        );
    });

    cx.dispatch_action(menu::Cancel);
    cx.run_until_parked();
    let view = open_view(&workspace, &repository, "b.txt", &mut cx).await;
    assert_eq!(result_text(&view, &mut cx), "A\nb\nC\nd\ne\nf\ng\nh\nI\n");
    assert_eq!(status_text(&view, &mut cx), "0 changes. 1 conflict.");
}

#[gpui::test]
async fn test_accept_side_deletes_the_file_when_that_side_is_missing(cx: &mut TestAppContext) {
    let MergeTest {
        fs,
        project,
        repository,
        mut cx,
        ..
    } = setup(
        &[Conflict::new(
            "d.txt",
            Some("one\n"),
            None,
            Some("one\ntwo\n"),
        )],
        Some(MERGE_MESSAGE),
        cx,
    )
    .await;
    assert!(is_conflicted(&repository, "d.txt", &mut cx));

    let task = cx.update(|_, cx| {
        accept_side(
            project.clone(),
            repository.clone(),
            repo_path("d.txt"),
            Side::Left,
            cx,
        )
    });
    task.await.expect("accepting yours should succeed");
    cx.run_until_parked();

    assert!(fs.read_file_sync(path!("/project/d.txt")).is_err());
    assert!(!is_conflicted(&repository, "d.txt", &mut cx));
}

#[gpui::test]
async fn test_next_and_previous_difference_move_the_cursor(cx: &mut TestAppContext) {
    let MergeTest {
        workspace,
        repository,
        mut cx,
        ..
    } = setup(&[fixture("a.txt")], Some(MERGE_MESSAGE), cx).await;
    let view = open_view(&workspace, &repository, "a.txt", &mut cx).await;
    place_cursor(&view, 0, &mut cx);

    for expected_row in [2, 4, 8, 8] {
        cx.dispatch_action(NextDifference);
        assert_eq!(cursor_row(&view, &mut cx), expected_row);
    }
    for expected_row in [4, 2, 0, 0] {
        cx.dispatch_action(PreviousDifference);
        assert_eq!(cursor_row(&view, &mut cx), expected_row);
    }

    cx.dispatch_action(ApplyNonConflictingAll);
    place_cursor(&view, 0, &mut cx);
    for expected_row in [4, 4] {
        cx.dispatch_action(NextDifference);
        assert_eq!(cursor_row(&view, &mut cx), expected_row);
    }
}

#[gpui::test]
async fn test_divider_buttons_follow_click_modifiers(cx: &mut TestAppContext) {
    let MergeTest {
        workspace,
        repository,
        mut cx,
        ..
    } = setup(&[fixture("a.txt")], Some(MERGE_MESSAGE), cx).await;
    let view = open_view(&workspace, &repository, "a.txt", &mut cx).await;

    view.update_in(&mut cx, |view, window, cx| {
        view.accept_chunk_from_divider(2, Side::Right, Modifiers::alt(), window, cx);
    });
    assert_eq!(
        result_text(&view, &mut cx),
        "a\nb\nc\nd\ne\nX\nf\ng\nh\ni\n"
    );
    assert_eq!(
        chunk_state(&view, 2, &mut cx),
        Some(ChunkState {
            left: SideState::Pending,
            right: SideState::Applied,
        })
    );

    view.update_in(&mut cx, |view, window, cx| {
        view.accept_chunk_from_divider(2, Side::Left, Modifiers::command(), window, cx);
    });
    assert_eq!(result_text(&view, &mut cx), "a\nb\nc\nd\nE\nf\ng\nh\ni\n");
    assert_eq!(status_text(&view, &mut cx), "3 changes. 0 conflicts.");

    view.update_in(&mut cx, |view, window, cx| {
        view.ignore_chunk_from_divider(0, Side::Left, Modifiers::command(), window, cx);
    });
    assert_eq!(result_text(&view, &mut cx), "a\nb\nc\nd\nE\nf\ng\nh\ni\n");
    assert_eq!(status_text(&view, &mut cx), "2 changes. 0 conflicts.");

    view.update_in(&mut cx, |view, window, cx| {
        view.accept_chunk_from_divider(1, Side::Right, Modifiers::none(), window, cx);
    });
    assert_eq!(result_text(&view, &mut cx), "a\nb\nC\nd\nE\nf\ng\nh\ni\n");
    assert_eq!(status_text(&view, &mut cx), "1 change. 0 conflicts.");

    view.update_in(&mut cx, |view, window, cx| {
        view.ignore_chunk_from_divider(3, Side::Right, Modifiers::none(), window, cx);
    });
    assert_eq!(result_text(&view, &mut cx), "a\nb\nC\nd\nE\nf\ng\nh\ni\n");
    assert_eq!(
        status_text(&view, &mut cx),
        "All changes have been processed."
    );
}

#[gpui::test]
async fn test_synchronized_scrolling_maps_lines_between_panes(cx: &mut TestAppContext) {
    let base = (0..200)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    let extra = (0..10)
        .map(|line| format!("extra {line}\n"))
        .collect::<String>();
    let ours = format!("{extra}{base}");
    let MergeTest {
        workspace,
        repository,
        mut cx,
        ..
    } = setup(
        &[Conflict::new(
            "long.txt",
            Some(base.as_str()),
            Some(ours.as_str()),
            Some(base.as_str()),
        )],
        Some(MERGE_MESSAGE),
        cx,
    )
    .await;
    let view = open_view(&workspace, &repository, "long.txt", &mut cx).await;

    scroll_result_to(&view, 50., &mut cx);
    assert_eq!(scroll_tops(&view, &mut cx), (60., 50., 50.));

    focus_result(&view, &mut cx);
    cx.dispatch_action(ToggleSynchronizeScrolling);
    assert!(!view.read_with(&mut cx, |view, _| view.synchronize_scrolling));
    scroll_result_to(&view, 70., &mut cx);
    assert_eq!(scroll_tops(&view, &mut cx), (60., 70., 50.));
}

#[gpui::test]
async fn test_ignore_policy_recomputes_chunks_until_an_operation_is_applied(
    cx: &mut TestAppContext,
) {
    let base = "fn main() {\n    call(a, b);\n}\n";
    let ours = "fn main() {\n\tcall(a, b);\n}\n";
    let MergeTest {
        workspace,
        repository,
        mut cx,
        ..
    } = setup(
        &[Conflict::new(
            "main.txt",
            Some(base),
            Some(ours),
            Some(base),
        )],
        Some(MERGE_MESSAGE),
        cx,
    )
    .await;
    let view = open_view(&workspace, &repository, "main.txt", &mut cx).await;
    assert_eq!(status_text(&view, &mut cx), "1 change. 0 conflicts.");

    view.update_in(&mut cx, |view, window, cx| {
        view.set_ignore_policy(IgnorePolicy::TrimWhitespaces, window, cx);
    });
    assert_eq!(
        status_text(&view, &mut cx),
        "All changes have been processed."
    );
    assert_eq!(
        view.read_with(&mut cx, |view, _| view.model.chunks().len()),
        0
    );

    view.update_in(&mut cx, |view, window, cx| {
        view.set_ignore_policy(IgnorePolicy::None, window, cx);
    });
    assert_eq!(status_text(&view, &mut cx), "1 change. 0 conflicts.");

    focus_result(&view, &mut cx);
    cx.dispatch_action(ApplyNonConflictingAll);
    assert_eq!(result_text(&view, &mut cx), ours);
    assert_eq!(
        status_text(&view, &mut cx),
        "All changes have been processed."
    );

    view.update_in(&mut cx, |view, window, cx| {
        view.set_ignore_policy(IgnorePolicy::TrimWhitespaces, window, cx);
    });
    assert_eq!(
        view.read_with(&mut cx, |view, _| view.model.merge().policy()),
        IgnorePolicy::None
    );
    assert_eq!(
        view.read_with(&mut cx, |view, _| view.model.chunks().len()),
        1
    );
    assert_eq!(result_text(&view, &mut cx), ours);
}

#[gpui::test]
async fn test_conflicts_dialog_opens_once_conflicts_appear(cx: &mut TestAppContext) {
    let MergeTest {
        fs,
        workspace,
        repository,
        mut cx,
        ..
    } = setup(&[], Some(MERGE_MESSAGE), cx).await;

    cx.update(|window, cx| {
        open_conflicts_dialog_if_conflicted(workspace.downgrade(), repository.clone(), window, cx);
    });
    cx.run_until_parked();
    assert!(active_dialog(&workspace, &mut cx).is_none());

    fs.set_conflict_for_repo(dot_git(), "a.txt", Some(BASE), Some(OURS), Some(THEIRS));
    cx.run_until_parked();
    let dialog = active_dialog(&workspace, &mut cx).expect("conflicts dialog should open");
    assert_eq!(
        dialog_paths(&dialog, &mut cx),
        (vec!["a.txt".to_string()], Vec::new())
    );
}

#[gpui::test]
async fn test_merge_branches_fall_back_to_the_merge_head(cx: &mut TestAppContext) {
    let MergeTest {
        repository, mut cx, ..
    } = setup(&[fixture("a.txt")], None, cx).await;
    let branches = repository.read_with(&mut cx, |repository, _| {
        MergeBranches::for_snapshot(repository)
    });
    assert_eq!(
        branches,
        MergeBranches {
            ours: "main".into(),
            theirs: "4d45524".into(),
        }
    );
}

#[test]
fn test_branch_names_are_parsed_from_merge_messages() {
    assert_eq!(
        branch_from_merge_message("Merge branch 'feature'\n"),
        Some("feature".into())
    );
    assert_eq!(
        branch_from_merge_message("Merge branch 'feature' into main"),
        Some("feature".into())
    );
    assert_eq!(
        branch_from_merge_message(
            "Merge remote-tracking branch 'origin/release'\n\n# Conflicts:\n#\ta.txt\n"
        ),
        Some("origin/release".into())
    );
    assert_eq!(
        branch_from_merge_message("Merge tag 'v1.2'"),
        Some("v1.2".into())
    );
    assert_eq!(branch_from_merge_message("Revert \"change\""), None);
    assert_eq!(branch_from_merge_message("Merge branch ''"), None);
}
