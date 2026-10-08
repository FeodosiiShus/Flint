use std::path::Path;

use git::repository::repo_path;
use git::status::{FileStatus, StatusCode, UnmergedStatus, UnmergedStatusCode};
use gpui::{Entity, TestAppContext, VisualTestContext};
use project::{FakeFs, Project, git_store::Repository};
use serde_json::json;
use settings::SettingsStore;
use theme::LoadThemes;
use util::{path, rel_path::rel_path};
use workspace::{MultiWorkspace, Workspace};

use super::{
    BannerColors, BannerKind, BannerLink, ConflictBanner, banner_kind, can_resolve_conflict,
};
use crate::merge_tool::single_file_merge::{
    cancel_active_merge_window, has_active_merge_window, open_single_file_merge,
};

const CONFLICTED_FILE: &str = "a.txt";
const PLAIN_FILE: &str = "b.txt";
const BASE: &str = "x\n";
const OURS: &str = "y\n";
const THEIRS: &str = "z\n";

fn unmerged(first_head: UnmergedStatusCode, second_head: UnmergedStatusCode) -> FileStatus {
    FileStatus::Unmerged(UnmergedStatus {
        first_head,
        second_head,
    })
}

#[test]
fn merge_tool_conflict_banner_is_suggested_for_every_resolvable_conflict_status() {
    use UnmergedStatusCode::{Added, Deleted, Updated};
    for (first_head, second_head) in [
        (Updated, Updated),
        (Added, Added),
        (Added, Updated),
        (Updated, Added),
        (Deleted, Updated),
        (Updated, Deleted),
    ] {
        assert_eq!(
            banner_kind(false, Some(unmerged(first_head, second_head))),
            Some(BannerKind::ResolveSuggested),
            "{first_head:?}/{second_head:?}"
        );
    }
}

#[test]
fn merge_tool_conflict_banner_is_not_shown_when_both_sides_deleted_the_file() {
    assert!(!can_resolve_conflict(unmerged(
        UnmergedStatusCode::Deleted,
        UnmergedStatusCode::Deleted
    )));
    assert_eq!(
        banner_kind(
            false,
            Some(unmerged(
                UnmergedStatusCode::Deleted,
                UnmergedStatusCode::Deleted
            ))
        ),
        None
    );
}

#[test]
fn merge_tool_conflict_banner_is_not_shown_for_files_without_conflict() {
    assert_eq!(banner_kind(false, None), None);
    assert_eq!(banner_kind(false, Some(FileStatus::Untracked)), None);
    assert_eq!(banner_kind(false, Some(FileStatus::Ignored)), None);
    assert_eq!(
        banner_kind(false, Some(FileStatus::index(StatusCode::Added))),
        None
    );
}

#[test]
fn merge_tool_conflict_banner_active_merge_window_takes_precedence_over_every_status() {
    let conflicted = unmerged(UnmergedStatusCode::Updated, UnmergedStatusCode::Updated);
    let both_deleted = unmerged(UnmergedStatusCode::Deleted, UnmergedStatusCode::Deleted);
    for status in [
        None,
        Some(conflicted),
        Some(both_deleted),
        Some(FileStatus::Untracked),
    ] {
        assert_eq!(
            banner_kind(true, status),
            Some(BannerKind::ResolveInProgress)
        );
    }
}

#[test]
fn merge_tool_conflict_banner_texts_and_links_match_webstorm() {
    assert_eq!(
        BannerKind::ResolveSuggested.text(),
        "File has unresolved merge conflicts"
    );
    assert_eq!(
        BannerKind::ResolveSuggested.links(),
        &[BannerLink::ResolveConflicts]
    );
    assert_eq!(
        BannerLink::ResolveConflicts.label(),
        "Resolve conflicts\u{2026}"
    );

    assert_eq!(
        BannerKind::ResolveInProgress.text(),
        "Resolving merge conflicts is in progress"
    );
    assert_eq!(
        BannerKind::ResolveInProgress.links(),
        &[BannerLink::ShowWindow, BannerLink::CancelResolve]
    );
    assert_eq!(
        BannerLink::ShowWindow.label(),
        "Show resolve conflicts window"
    );
    assert_eq!(BannerLink::CancelResolve.label(), "Cancel resolve");
}

#[test]
fn merge_tool_conflict_banner_colours_match_the_islands_banner_theme_keys() {
    assert_eq!(
        BannerKind::ResolveSuggested.colors(true),
        BannerColors {
            background: 0x44321D,
            border: 0x694820,
            foreground: 0xDFE1E5,
        }
    );
    assert_eq!(
        BannerKind::ResolveInProgress.colors(true),
        BannerColors {
            background: 0x233558,
            border: 0x2E4D89,
            foreground: 0xDFE1E5,
        }
    );
    assert_eq!(
        BannerKind::ResolveSuggested.colors(false),
        BannerColors {
            background: 0xFFF6E9,
            border: 0xF4CD9A,
            foreground: 0x000000,
        }
    );
    assert_eq!(
        BannerKind::ResolveInProgress.colors(false),
        BannerColors {
            background: 0xF7F8FF,
            border: 0xBDD3FF,
            foreground: 0x000000,
        }
    );
}

struct Harness {
    workspace: Entity<Workspace>,
    repository: Entity<Repository>,
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

async fn open_harness(cx: &mut TestAppContext) -> (Harness, &mut VisualTestContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.background_executor.clone());
    fs.insert_tree(path!("/project"), json!({ ".git": {}, "b.txt": "plain\n" }))
        .await;
    let dot_git = Path::new(path!("/project/.git"));
    fs.set_branch_name(dot_git, Some("main"));
    fs.set_conflict_for_repo(
        dot_git,
        CONFLICTED_FILE,
        Some(BASE),
        Some(OURS),
        Some(THEIRS),
    );

    let project = Project::test(fs, [Path::new(path!("/project"))], cx).await;
    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
    let workspace =
        multi_workspace.read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone());
    cx.run_until_parked();
    let repository = project
        .read_with(cx, |project, cx| project.active_repository(cx))
        .expect("project should have a repository");
    (
        Harness {
            workspace,
            repository,
        },
        cx,
    )
}

async fn open_file(harness: &Harness, name: &str, cx: &mut VisualTestContext) {
    let worktree_id = harness.workspace.read_with(cx, |workspace, cx| {
        workspace
            .project()
            .read(cx)
            .worktrees(cx)
            .next()
            .expect("project should have a worktree")
            .read(cx)
            .id()
    });
    harness
        .workspace
        .update_in(cx, |workspace, window, cx| {
            workspace.open_path((worktree_id, rel_path(name)), None, true, window, cx)
        })
        .await
        .expect("the file should open");
    cx.run_until_parked();
}

fn shown_banner(harness: &Harness, cx: &mut VisualTestContext) -> Option<BannerKind> {
    harness.workspace.read_with(cx, |workspace, cx| {
        workspace
            .active_pane()
            .read(cx)
            .toolbar()
            .read(cx)
            .item_of_type::<ConflictBanner>()
            .and_then(|banner| banner.read(cx).kind())
    })
}

fn open_merge_window(harness: &Harness, cx: &mut VisualTestContext) {
    let workspace = harness.workspace.downgrade();
    let repository = harness.repository.clone();
    cx.update(|window, cx| {
        open_single_file_merge(
            workspace,
            repository,
            repo_path(CONFLICTED_FILE),
            window,
            cx,
        );
    });
    cx.run_until_parked();
}

#[gpui::test]
async fn merge_tool_conflict_banner_suggests_resolving_a_conflicted_open_file(
    cx: &mut TestAppContext,
) {
    let (harness, cx) = open_harness(cx).await;
    open_file(&harness, CONFLICTED_FILE, cx).await;

    assert_eq!(
        shown_banner(&harness, cx),
        Some(BannerKind::ResolveSuggested)
    );
}

#[gpui::test]
async fn merge_tool_conflict_banner_is_absent_for_a_file_without_conflict(cx: &mut TestAppContext) {
    let (harness, cx) = open_harness(cx).await;
    open_file(&harness, PLAIN_FILE, cx).await;

    assert_eq!(shown_banner(&harness, cx), None);
}

#[gpui::test]
async fn merge_tool_conflict_banner_reports_progress_while_the_merge_window_is_open(
    cx: &mut TestAppContext,
) {
    let (harness, cx) = open_harness(cx).await;
    open_file(&harness, CONFLICTED_FILE, cx).await;
    open_merge_window(&harness, cx);

    let repository_id = harness
        .repository
        .read_with(cx, |repository, _| repository.id);
    assert!(cx.update(|_, cx| has_active_merge_window(
        repository_id,
        &repo_path(CONFLICTED_FILE),
        cx
    )));
    assert_eq!(
        shown_banner(&harness, cx),
        Some(BannerKind::ResolveInProgress)
    );
}

#[gpui::test]
async fn merge_tool_conflict_banner_returns_to_the_suggestion_after_cancel_resolve(
    cx: &mut TestAppContext,
) {
    let (harness, cx) = open_harness(cx).await;
    open_file(&harness, CONFLICTED_FILE, cx).await;
    open_merge_window(&harness, cx);

    let repository_id = harness
        .repository
        .read_with(cx, |repository, _| repository.id);
    cx.update(|_, cx| {
        cancel_active_merge_window(repository_id, &repo_path(CONFLICTED_FILE), cx);
    });
    cx.run_until_parked();

    assert!(!cx.update(|_, cx| has_active_merge_window(
        repository_id,
        &repo_path(CONFLICTED_FILE),
        cx
    )));
    assert_eq!(
        shown_banner(&harness, cx),
        Some(BannerKind::ResolveSuggested)
    );
}
