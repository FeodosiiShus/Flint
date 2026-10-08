use std::{cell::RefCell, path::Path, rc::Rc, sync::Arc};

use git::repository::repo_path;
use gpui::{AnyWindowHandle, App, Entity, Task, TestAppContext, VisualTestContext};
use merge_diff::Side;
use project::{FakeFs, Project, git_store::Repository};
use serde_json::json;
use settings::SettingsStore;
use theme::LoadThemes;
use util::path;
use workspace::{MultiWorkspace, Workspace};

use super::overlay::Overlay;
use super::state::{DefaultButton, GroupKind, RowId, RowViewKind};
use super::test_driver::{OpenConflictsDialog, find_open_dialog};
use super::{ConflictsDialogRequest, ConflictsDialogResult, open_conflicts_dialog};
use crate::merge_tool::conflict_resolution::dialog_texts::{
    DialogTexts, PaneTitleText, PaneTitleTexts,
};
use crate::merge_tool::conflict_resolution::rich_text::RichText;
use crate::merge_tool::dialog_window::DialogWindowShell;

const MERGE_MESSAGE: &str = "Merge branch 'feature'\n";
const AUTO_BASE: &str = "a\nb\nc\n";
const AUTO_OURS: &str = "A\nb\nc\n";
const AUTO_THEIRS: &str = "a\nb\nC\n";
const AUTO_MERGED: &str = "A\nb\nC\n";
const CONFLICT_BASE: &str = "x\n";
const CONFLICT_OURS: &str = "y\n";
const CONFLICT_THEIRS: &str = "z\n";
const PARTIAL_BASE: &str = "a\nb\nc\nd\ne\n";
const PARTIAL_OURS: &str = "A\nb\nc\nd\nE\n";
const PARTIAL_THEIRS: &str = "a\nb\nc\nd\nX\n";

struct Conflict {
    path: &'static str,
    base: Option<&'static str>,
    ours: Option<&'static str>,
    theirs: Option<&'static str>,
}

fn conflict(
    path: &'static str,
    base: &'static str,
    ours: &'static str,
    theirs: &'static str,
) -> Conflict {
    Conflict {
        path,
        base: Some(base),
        ours: Some(ours),
        theirs: Some(theirs),
    }
}

struct Harness {
    fs: Arc<FakeFs>,
    repository: Entity<Repository>,
    workspace: Entity<Workspace>,
    owner_window: AnyWindowHandle,
    cx: VisualTestContext,
}

struct OpenDialog {
    harness: Harness,
    dialog: OpenConflictsDialog,
    closed: Rc<RefCell<Option<ConflictsDialogResult>>>,
}

#[derive(Debug, PartialEq)]
struct ShownConfirmation {
    title: String,
    message: String,
    accept: String,
    decline: String,
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

fn test_texts() -> DialogTexts {
    DialogTexts {
        dialog_title: "Conflicts".into(),
        yours_column: "Yours (main)".into(),
        theirs_column: "Theirs (feature)".into(),
        pane_titles: PaneTitleTexts {
            left: PaneTitleText::plain("Changes from main"),
            result: PaneTitleText::plain("Result"),
            right: PaneTitleText::plain("Changes from feature"),
        },
    }
}

async fn open_harness(conflicts: &[Conflict], cx: &mut TestAppContext) -> Harness {
    init_test(cx);
    let fs = FakeFs::new(cx.background_executor.clone());
    fs.insert_tree(
        path!("/project"),
        json!({ ".git": {}, "docs": {}, "src": {} }),
    )
    .await;
    fs.set_branch_name(dot_git(), Some("main"));
    fs.set_merge_message_for_repo(dot_git(), Some(MERGE_MESSAGE));
    for conflict in conflicts {
        fs.set_conflict_for_repo(
            dot_git(),
            conflict.path,
            conflict.base,
            conflict.ours,
            conflict.theirs,
        );
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
        owner_window: window_handle.into(),
        cx: workspace_cx,
    }
}

async fn show_dialog(mut harness: Harness) -> OpenDialog {
    let entries = harness
        .repository
        .update(&mut harness.cx, |repository, cx| {
            repository.unmerged_entries(cx)
        })
        .await
        .expect("unmerged entries should load");

    let closed = Rc::new(RefCell::new(None));
    let request = ConflictsDialogRequest {
        workspace: harness.workspace.downgrade(),
        owner_window: harness.owner_window,
        repository: harness.repository.clone(),
        entries,
        texts: test_texts(),
        description: Task::ready(RichText::plain("Merging branch feature into branch main")),
        reversed: false,
        on_closed: {
            let closed = closed.clone();
            Box::new(move |result: ConflictsDialogResult, _: &mut App| {
                *closed.borrow_mut() = Some(result);
            })
        },
    };
    harness
        .cx
        .update(|_, cx| open_conflicts_dialog(request, cx));
    harness.cx.run_until_parked();

    let dialog =
        find_open_dialog(&mut harness.cx).expect("the conflicts dialog should have been opened");
    OpenDialog {
        harness,
        dialog,
        closed,
    }
}

async fn open_dialog(conflicts: &[Conflict], cx: &mut TestAppContext) -> OpenDialog {
    let harness = open_harness(conflicts, cx).await;
    show_dialog(harness).await
}

async fn reopen_dialog(open: OpenDialog) -> OpenDialog {
    show_dialog(open.harness).await
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

fn is_conflicted(open: &OpenDialog, path: &str) -> bool {
    open.harness
        .repository
        .read_with(&open.harness.cx, |repository, _| {
            repository
                .status_for_path(&repo_path(path))
                .is_some_and(|entry| entry.status.is_conflicted())
        })
}

fn closed_result(open: &OpenDialog) -> Option<(Vec<String>, bool)> {
    open.closed.borrow().as_ref().map(|result| {
        (
            result
                .processed_files
                .iter()
                .map(|file| file.as_unix_str().to_string())
                .collect(),
            result.should_finish_merge,
        )
    })
}

fn is_dialog_open(open: &OpenDialog) -> bool {
    open.harness
        .cx
        .windows()
        .contains(&AnyWindowHandle::from(open.dialog.window))
}

fn find_merge_window(open: &OpenDialog) -> Option<AnyWindowHandle> {
    let dialog_window = AnyWindowHandle::from(open.dialog.window);
    open.harness
        .cx
        .windows()
        .into_iter()
        .find(|window| *window != dialog_window && window.downcast::<DialogWindowShell>().is_some())
}

fn shown_confirmation(open: &OpenDialog) -> Option<ShownConfirmation> {
    open.dialog.read(|dialog| match dialog.overlay.as_ref() {
        Some(Overlay::Confirm(confirmation)) => Some(ShownConfirmation {
            title: confirmation.title.to_string(),
            message: confirmation.message.to_string(),
            accept: confirmation.yes_label.to_string(),
            decline: confirmation.no_label.to_string(),
        }),
        Some(Overlay::Progress(_)) | None => None,
    })
}

fn confirmation(
    title: &str,
    message: &str,
    accept: &str,
    decline: &str,
) -> Option<ShownConfirmation> {
    Some(ShownConfirmation {
        title: title.to_string(),
        message: message.to_string(),
        accept: accept.to_string(),
        decline: decline.to_string(),
    })
}

fn badge(path: &str, value: Option<&str>) -> (String, Option<String>) {
    (path.to_string(), value.map(str::to_string))
}

fn file_badges(open: &OpenDialog) -> Vec<(String, Option<String>)> {
    open.dialog.read(|dialog| {
        dialog
            .state
            .row_views()
            .into_iter()
            .filter_map(|row| match row.kind {
                RowViewKind::File {
                    path,
                    badge: file_badge,
                    ..
                } => Some((
                    path.as_unix_str().to_string(),
                    file_badge.map(|view| view.value),
                )),
                RowViewKind::Group { .. } | RowViewKind::Directory { .. } => None,
            })
            .collect()
    })
}

fn group_row(group: GroupKind) -> RowId {
    RowId::Group(group)
}

fn directory_row(path: &str) -> RowId {
    RowId::Directory(GroupKind::Unresolved, path.to_string())
}

fn file_row(path: &str) -> RowId {
    RowId::File(repo_path(path))
}

fn visible_row_ids(open: &OpenDialog) -> Vec<RowId> {
    open.dialog.read(|dialog| {
        dialog
            .state
            .visible_rows()
            .into_iter()
            .map(|row| row.id.clone())
            .collect()
    })
}

fn selected_row_ids(open: &OpenDialog) -> Vec<RowId> {
    open.dialog
        .read(|dialog| dialog.state.selected_ids().to_vec())
}

fn groups_by_directory(open: &OpenDialog) -> bool {
    open.dialog.read(|dialog| dialog.state.group_by_directory())
}

fn default_button(open: &OpenDialog) -> DefaultButton {
    open.dialog.read(|dialog| dialog.state.default_button())
}

fn has_partially_resolved_files(open: &OpenDialog) -> bool {
    open.dialog
        .read(|dialog| dialog.state.has_partially_resolved_files())
}

fn resolve_all_enabled(open: &OpenDialog) -> bool {
    open.dialog
        .read(|dialog| dialog.state.resolve_all_enabled())
}

fn is_file_reviewed(open: &OpenDialog, path: &str) -> bool {
    open.dialog
        .read(|dialog| dialog.state.is_file_reviewed(&repo_path(path)))
}

fn select_file(open: &mut OpenDialog, path: &str) {
    open.dialog.update_in(|dialog, _, cx| {
        dialog.state.select_only(file_row(path));
        cx.notify();
    });
}

#[gpui::test]
async fn merge_tool_dialog_lists_every_conflict_as_unresolved_without_badges_and_selects_the_header(
    cx: &mut TestAppContext,
) {
    let open = open_dialog(
        &[
            conflict("b.txt", CONFLICT_BASE, CONFLICT_OURS, CONFLICT_THEIRS),
            conflict("a.txt", AUTO_BASE, AUTO_OURS, AUTO_THEIRS),
        ],
        cx,
    )
    .await;

    assert_eq!(
        open.dialog.unresolved_files(),
        vec!["a.txt".to_string(), "b.txt".to_string()]
    );
    assert!(open.dialog.resolved_files().is_empty());
    assert_eq!(
        file_badges(&open),
        vec![badge("a.txt", None), badge("b.txt", None)]
    );
    assert_eq!(
        selected_row_ids(&open),
        vec![group_row(GroupKind::Unresolved)]
    );
    let selected_files = open.dialog.read(|dialog| dialog.state.selected_files());
    assert_eq!(selected_files, vec![repo_path("a.txt"), repo_path("b.txt")]);
    assert!(is_dialog_open(&open));
    assert!(closed_result(&open).is_none());
}

#[gpui::test]
async fn merge_tool_accept_yours_writes_the_file_but_leaves_the_index_conflicted(
    cx: &mut TestAppContext,
) {
    let mut open = open_dialog(
        &[
            conflict("a.txt", AUTO_BASE, AUTO_OURS, AUTO_THEIRS),
            conflict("b.txt", CONFLICT_BASE, CONFLICT_OURS, CONFLICT_THEIRS),
        ],
        cx,
    )
    .await;

    open.dialog.accept(Side::Left);

    assert!(open.dialog.unresolved_files().is_empty());
    assert_eq!(
        open.dialog.resolved_files(),
        vec!["a.txt".to_string(), "b.txt".to_string()]
    );
    assert_eq!(
        file_badges(&open),
        vec![badge("a.txt", Some("2/2")), badge("b.txt", Some("1/1"))]
    );
    assert_eq!(
        read_file(&open.harness.fs, path!("/project/a.txt")),
        AUTO_OURS
    );
    assert_eq!(
        read_file(&open.harness.fs, path!("/project/b.txt")),
        CONFLICT_OURS
    );
    assert!(is_conflicted(&open, "a.txt"));
    assert!(is_conflicted(&open, "b.txt"));
    assert!(closed_result(&open).is_none());
}

#[gpui::test]
async fn merge_tool_accept_theirs_uses_the_theirs_side_of_every_selected_file(
    cx: &mut TestAppContext,
) {
    let mut open = open_dialog(
        &[
            conflict("a.txt", AUTO_BASE, AUTO_OURS, AUTO_THEIRS),
            conflict("b.txt", CONFLICT_BASE, CONFLICT_OURS, CONFLICT_THEIRS),
        ],
        cx,
    )
    .await;

    open.dialog.accept(Side::Right);

    assert_eq!(
        read_file(&open.harness.fs, path!("/project/a.txt")),
        AUTO_THEIRS
    );
    assert_eq!(
        read_file(&open.harness.fs, path!("/project/b.txt")),
        CONFLICT_THEIRS
    );
    assert_eq!(
        open.dialog.resolved_files(),
        vec!["a.txt".to_string(), "b.txt".to_string()]
    );
    assert!(is_conflicted(&open, "a.txt"));
    assert!(is_conflicted(&open, "b.txt"));
}

#[gpui::test]
async fn merge_tool_closing_the_dialog_stages_resolved_files_and_reports_them(
    cx: &mut TestAppContext,
) {
    let mut open = open_dialog(
        &[
            conflict("a.txt", AUTO_BASE, AUTO_OURS, AUTO_THEIRS),
            conflict("b.txt", CONFLICT_BASE, CONFLICT_OURS, CONFLICT_THEIRS),
        ],
        cx,
    )
    .await;
    open.dialog.accept(Side::Left);

    open.dialog.close();

    assert!(!is_dialog_open(&open));
    assert_eq!(
        closed_result(&open),
        Some((vec!["a.txt".to_string(), "b.txt".to_string()], false))
    );
    assert_eq!(
        index_text(&open.harness.fs, "a.txt").as_deref(),
        Some(AUTO_OURS)
    );
    assert_eq!(
        index_text(&open.harness.fs, "b.txt").as_deref(),
        Some(CONFLICT_OURS)
    );
    assert!(!is_conflicted(&open, "a.txt"));
    assert!(!is_conflicted(&open, "b.txt"));
}

#[gpui::test]
async fn merge_tool_enter_accepts_and_finishes_once_every_file_is_resolved_and_reviewed(
    cx: &mut TestAppContext,
) {
    let mut open = open_dialog(&[conflict("a.txt", AUTO_BASE, AUTO_OURS, AUTO_THEIRS)], cx).await;
    assert_eq!(default_button(&open), DefaultButton::ReviewOrResolve);

    open.dialog.accept(Side::Right);

    assert_eq!(default_button(&open), DefaultButton::AcceptAndFinish);
    assert!(closed_result(&open).is_none());

    open.dialog.dispatch(menu::Confirm);

    assert!(!is_dialog_open(&open));
    assert_eq!(
        closed_result(&open),
        Some((vec!["a.txt".to_string()], true))
    );
    assert_eq!(
        index_text(&open.harness.fs, "a.txt").as_deref(),
        Some(AUTO_THEIRS)
    );
}

#[gpui::test]
async fn merge_tool_accept_and_finish_is_ignored_while_files_are_unresolved(
    cx: &mut TestAppContext,
) {
    let mut open = open_dialog(&[conflict("a.txt", AUTO_BASE, AUTO_OURS, AUTO_THEIRS)], cx).await;

    open.dialog.accept_and_finish();

    assert!(is_dialog_open(&open));
    assert!(closed_result(&open).is_none());
    assert!(is_conflicted(&open, "a.txt"));
}

#[gpui::test]
async fn merge_tool_binary_files_are_accepted_through_git_immediately(cx: &mut TestAppContext) {
    let mut open = open_dialog(
        &[Conflict {
            path: "image.bin",
            base: Some("base\0"),
            ours: Some("ours\0"),
            theirs: Some("theirs\0"),
        }],
        cx,
    )
    .await;

    open.dialog.accept(Side::Right);

    assert!(!is_dialog_open(&open));
    assert_eq!(
        closed_result(&open),
        Some((vec!["image.bin".to_string()], false))
    );
    assert_eq!(
        index_text(&open.harness.fs, "image.bin").as_deref(),
        Some("theirs\0")
    );
    assert!(!is_conflicted(&open, "image.bin"));
}

#[gpui::test]
async fn merge_tool_resolve_all_simple_conflicts_applies_what_it_can_and_reports_the_remainder(
    cx: &mut TestAppContext,
) {
    let mut open = open_dialog(
        &[
            conflict("a.txt", AUTO_BASE, AUTO_OURS, AUTO_THEIRS),
            conflict("b.txt", CONFLICT_BASE, CONFLICT_OURS, CONFLICT_THEIRS),
            conflict("p.txt", PARTIAL_BASE, PARTIAL_OURS, PARTIAL_THEIRS),
        ],
        cx,
    )
    .await;
    let untouched_conflict = read_file(&open.harness.fs, path!("/project/b.txt"));
    select_file(&mut open, "b.txt");

    open.dialog.resolve_all_simple_conflicts();

    assert_eq!(open.dialog.resolved_files(), vec!["a.txt".to_string()]);
    assert_eq!(
        open.dialog.unresolved_files(),
        vec!["b.txt".to_string(), "p.txt".to_string()]
    );
    assert_eq!(
        file_badges(&open),
        vec![
            badge("b.txt", Some("0/1")),
            badge("p.txt", Some("1/2")),
            badge("a.txt", Some("2/2")),
        ]
    );
    assert_eq!(
        open.dialog.auto_resolve_status().as_deref(),
        Some("3 conflicts resolved. 2 conflicts in 2 files still require attention")
    );
    assert_eq!(
        selected_row_ids(&open),
        vec![group_row(GroupKind::Unresolved)]
    );
    assert_eq!(
        read_file(&open.harness.fs, path!("/project/a.txt")),
        AUTO_MERGED
    );
    assert_eq!(
        read_file(&open.harness.fs, path!("/project/b.txt")),
        untouched_conflict
    );
    assert!(is_conflicted(&open, "a.txt"));
    assert!(is_conflicted(&open, "b.txt"));
    assert!(is_conflicted(&open, "p.txt"));
}

#[gpui::test]
async fn merge_tool_resolve_all_simple_conflicts_reports_when_nothing_was_resolved(
    cx: &mut TestAppContext,
) {
    let mut open = open_dialog(
        &[conflict(
            "b.txt",
            CONFLICT_BASE,
            CONFLICT_OURS,
            CONFLICT_THEIRS,
        )],
        cx,
    )
    .await;

    open.dialog.resolve_all_simple_conflicts();

    assert_eq!(
        open.dialog.auto_resolve_status().as_deref(),
        Some("No conflicts were resolved automatically")
    );
    assert!(!resolve_all_enabled(&open));
}

#[gpui::test]
async fn merge_tool_revert_returns_resolved_files_to_the_unresolved_group_and_reenables_resolve_all(
    cx: &mut TestAppContext,
) {
    let mut open = open_dialog(
        &[
            conflict("a.txt", AUTO_BASE, AUTO_OURS, AUTO_THEIRS),
            conflict("b.txt", CONFLICT_BASE, CONFLICT_OURS, CONFLICT_THEIRS),
        ],
        cx,
    )
    .await;
    open.dialog.resolve_all_simple_conflicts();
    assert_eq!(open.dialog.resolved_files(), vec!["a.txt".to_string()]);
    assert!(!resolve_all_enabled(&open));
    select_file(&mut open, "a.txt");

    open.dialog
        .update_in(|dialog, window, cx| dialog.revert_selected_resolution(window, cx));

    assert_eq!(
        shown_confirmation(&open),
        confirmation(
            "Confirm Revert",
            "All changes will be reverted. The file will return to its original conflicted state.",
            "Revert",
            "Cancel",
        )
    );

    open.dialog.dispatch(menu::Confirm);

    assert_eq!(shown_confirmation(&open), None);
    assert!(open.dialog.resolved_files().is_empty());
    assert_eq!(
        open.dialog.unresolved_files(),
        vec!["a.txt".to_string(), "b.txt".to_string()]
    );
    assert_eq!(
        file_badges(&open),
        vec![badge("a.txt", None), badge("b.txt", Some("0/1"))]
    );
    assert!(resolve_all_enabled(&open));
    assert_eq!(
        read_file(&open.harness.fs, path!("/project/a.txt")),
        AUTO_MERGED
    );
    assert!(is_conflicted(&open, "a.txt"));
}

#[gpui::test]
async fn merge_tool_close_button_asks_before_discarding_partially_resolved_files(
    cx: &mut TestAppContext,
) {
    let mut open = open_dialog(
        &[conflict(
            "p.txt",
            PARTIAL_BASE,
            PARTIAL_OURS,
            PARTIAL_THEIRS,
        )],
        cx,
    )
    .await;
    open.dialog.resolve_all_simple_conflicts();
    assert!(has_partially_resolved_files(&open));

    open.dialog.close();

    assert_eq!(
        shown_confirmation(&open),
        confirmation(
            "Discard Changes?",
            "Closing the dialog will discard your changes in partially resolved files",
            "Discard Changes",
            "Continue Merge",
        )
    );
    assert!(closed_result(&open).is_none());

    open.dialog.dispatch(menu::Cancel);

    assert_eq!(shown_confirmation(&open), None);
    assert!(is_dialog_open(&open));
    assert!(closed_result(&open).is_none());

    open.dialog.close();
    open.dialog.dispatch(menu::Confirm);

    assert!(!is_dialog_open(&open));
    assert_eq!(closed_result(&open), Some((Vec::new(), false)));
    assert!(is_conflicted(&open, "p.txt"));
}

#[gpui::test]
async fn merge_tool_escape_closes_silently_and_applies_only_fully_resolved_files(
    cx: &mut TestAppContext,
) {
    let mut open = open_dialog(
        &[
            conflict("a.txt", AUTO_BASE, AUTO_OURS, AUTO_THEIRS),
            conflict("p.txt", PARTIAL_BASE, PARTIAL_OURS, PARTIAL_THEIRS),
        ],
        cx,
    )
    .await;
    open.dialog.resolve_all_simple_conflicts();
    assert_eq!(open.dialog.resolved_files(), vec!["a.txt".to_string()]);
    assert!(has_partially_resolved_files(&open));

    open.dialog.dispatch(menu::Cancel);

    assert!(!is_dialog_open(&open));
    assert_eq!(
        closed_result(&open),
        Some((vec!["a.txt".to_string()], false))
    );
    assert_eq!(
        index_text(&open.harness.fs, "a.txt").as_deref(),
        Some(AUTO_MERGED)
    );
    assert!(!is_conflicted(&open, "a.txt"));
    assert_eq!(index_text(&open.harness.fs, "p.txt"), None);
    assert!(is_conflicted(&open, "p.txt"));
}

#[gpui::test]
async fn merge_tool_accepting_over_partially_merged_files_asks_before_overwriting(
    cx: &mut TestAppContext,
) {
    let mut open = open_dialog(
        &[conflict(
            "p.txt",
            PARTIAL_BASE,
            PARTIAL_OURS,
            PARTIAL_THEIRS,
        )],
        cx,
    )
    .await;
    open.dialog.resolve_all_simple_conflicts();
    let partially_merged = read_file(&open.harness.fs, path!("/project/p.txt"));
    assert_ne!(partially_merged, PARTIAL_THEIRS);
    select_file(&mut open, "p.txt");

    open.dialog.accept(Side::Right);

    assert_eq!(
        shown_confirmation(&open),
        confirmation(
            "Overwrite Changes",
            "This file already contains merged changes. Accepting changes from \u{2018}Theirs (feature)\u{2019} will discard and overwrite them.",
            "Discard and Accept",
            "Cancel",
        )
    );

    open.dialog.dispatch(menu::Cancel);

    assert_eq!(shown_confirmation(&open), None);
    assert!(is_dialog_open(&open));
    assert_eq!(open.dialog.unresolved_files(), vec!["p.txt".to_string()]);
    assert_eq!(
        read_file(&open.harness.fs, path!("/project/p.txt")),
        partially_merged
    );

    open.dialog.accept(Side::Right);
    open.dialog.dispatch(menu::Confirm);

    assert_eq!(shown_confirmation(&open), None);
    assert_eq!(open.dialog.resolved_files(), vec!["p.txt".to_string()]);
    assert_eq!(file_badges(&open), vec![badge("p.txt", Some("2/2"))]);
    assert_eq!(
        read_file(&open.harness.fs, path!("/project/p.txt")),
        PARTIAL_THEIRS
    );
    assert!(is_conflicted(&open, "p.txt"));
}

#[gpui::test]
async fn merge_tool_group_by_directory_is_persisted_and_applied_when_the_dialog_reopens(
    cx: &mut TestAppContext,
) {
    let flat_rows = || {
        vec![
            group_row(GroupKind::Unresolved),
            file_row("src/a.txt"),
            file_row("docs/b.txt"),
        ]
    };
    let directory_rows = || {
        vec![
            group_row(GroupKind::Unresolved),
            directory_row(""),
            directory_row("docs"),
            file_row("docs/b.txt"),
            directory_row("src"),
            file_row("src/a.txt"),
        ]
    };
    let mut open = open_dialog(
        &[
            conflict("src/a.txt", AUTO_BASE, AUTO_OURS, AUTO_THEIRS),
            conflict("docs/b.txt", CONFLICT_BASE, CONFLICT_OURS, CONFLICT_THEIRS),
        ],
        cx,
    )
    .await;
    assert!(!groups_by_directory(&open));
    assert_eq!(visible_row_ids(&open), flat_rows());

    open.dialog
        .update_in(|dialog, window, cx| dialog.toggle_group_by_directory(window, cx));

    assert!(groups_by_directory(&open));
    assert_eq!(visible_row_ids(&open), directory_rows());

    open.dialog.close();
    assert!(!is_dialog_open(&open));
    let mut open = reopen_dialog(open).await;

    assert!(is_dialog_open(&open));
    assert!(groups_by_directory(&open));
    assert_eq!(visible_row_ids(&open), directory_rows());

    open.dialog
        .update_in(|dialog, window, cx| dialog.toggle_group_by_directory(window, cx));
    open.dialog.close();
    let open = reopen_dialog(open).await;

    assert!(!groups_by_directory(&open));
    assert_eq!(visible_row_ids(&open), flat_rows());
}

#[test]
fn merge_tool_outer_height_grows_when_more_than_six_files_conflict() {
    assert_eq!(super::outer_height(0), 505.);
    assert_eq!(super::outer_height(6), 505.);
    assert_eq!(super::outer_height(7), 605.);
}

#[gpui::test]
async fn merge_tool_enter_resolves_manually_and_closing_the_merge_window_marks_the_file_reviewed(
    cx: &mut TestAppContext,
) {
    let mut open = open_dialog(&[conflict("a.txt", AUTO_BASE, AUTO_OURS, AUTO_THEIRS)], cx).await;
    assert_eq!(default_button(&open), DefaultButton::ReviewOrResolve);

    open.dialog.dispatch(menu::Confirm);

    let merge_window =
        find_merge_window(&open).expect("the merge window should open for the selected file");
    assert!(!is_file_reviewed(&open, "a.txt"));

    let mut merge_cx = VisualTestContext::from_window(merge_window, &open.harness.cx);
    merge_cx.dispatch_action(menu::Cancel);
    merge_cx.run_until_parked();

    assert!(is_file_reviewed(&open, "a.txt"));
    assert!(!open.harness.cx.windows().contains(&merge_window));
    assert!(is_dialog_open(&open));
    assert!(is_conflicted(&open, "a.txt"));
}
