use std::{path::Path, sync::Arc};

use git::{
    Oid,
    repository::{CommitData, InitialGraphCommitData},
};
use gpui::{TestAppContext, VisualTestContext};
use project::{FakeFs, Project};
use serde_json::json;
use settings::SettingsStore;
use smallvec::smallvec;

use super::*;

fn init_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
        cx.set_global(db::AppDatabase::test_new());
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        editor::init(cx);
        crate::init(cx);
    });
}

fn sha(byte: u8) -> Oid {
    Oid::from_bytes(&[byte; 20]).unwrap()
}

fn graph_commit(own: u8, parents: &[u8], ref_names: &[&str]) -> Arc<InitialGraphCommitData> {
    Arc::new(InitialGraphCommitData {
        sha: sha(own),
        parents: parents.iter().map(|parent| sha(*parent)).collect(),
        ref_names: ref_names
            .iter()
            .map(|name| SharedString::from(name.to_string()))
            .collect(),
        committer_timestamp: i64::from(own),
        dotted_parents: smallvec![],
    })
}

fn commit_data(own: u8, parents: &[u8], subject: &str) -> CommitData {
    CommitData {
        sha: sha(own),
        parents: parents.iter().map(|parent| sha(*parent)).collect(),
        author_name: "Author".into(),
        author_email: "author@example.com".into(),
        commit_timestamp: i64::from(own),
        author_timestamp: i64::from(own),
        committer_name: "Author".into(),
        committer_email: "author@example.com".into(),
        committer_timestamp: i64::from(own),
        subject: subject.to_string().into(),
        message: subject.to_string().into(),
    }
}

async fn project_with_commits(cx: &mut TestAppContext) -> Entity<Project> {
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree(
        Path::new("/project"),
        json!({
            ".git": {},
            "file.txt": "content",
        }),
    )
    .await;
    fs.set_graph_commits(
        Path::new("/project/.git"),
        vec![
            graph_commit(1, &[2], &["HEAD -> main"]),
            graph_commit(2, &[3], &["tag: v1.0"]),
            graph_commit(3, &[], &[]),
        ],
    );
    fs.set_commit_data(
        Path::new("/project/.git"),
        [
            (commit_data(1, &[2], "Third"), false),
            (commit_data(2, &[3], "Second"), false),
            (commit_data(3, &[], "First"), false),
        ],
    );
    let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;
    cx.run_until_parked();
    project
}

fn open_table(project: &Entity<Project>, cx: &mut VisualTestContext) -> Entity<CommitTable> {
    let repository = project.read_with(&*cx, |project, cx| {
        project
            .active_repository(cx)
            .expect("the project should have a repository")
    });
    let workspace = cx.new_window_entity(|window, cx| {
        workspace::MultiWorkspace::test_new(project.clone(), window, cx)
    });
    let workspace = workspace.read_with(&*cx, |multi, _| multi.workspace().downgrade());
    let table = cx.new_window_entity(|window, cx| {
        CommitTable::new(
            workspace,
            LogAppProperties::default(),
            LogTabProperties::default(),
            window,
            cx,
        )
    });
    table.update(cx, |table, cx| table.set_repository(Some(repository), cx));
    cx.run_until_parked();
    table
}

#[gpui::test]
async fn keyboard_selection_follows_jtable_and_copies_hashes_oldest_first(cx: &mut TestAppContext) {
    init_test(cx);
    let project = project_with_commits(cx).await;
    let cx = cx.add_empty_window();
    let table = open_table(&project, cx);

    table.read_with(&*cx, |table, _| assert_eq!(table.row_count(), 3));

    table.update_in(cx, |table, window, cx| {
        table.select_next(&menu::SelectNext, window, cx)
    });
    table.read_with(&*cx, |table, _| {
        assert_eq!(table.selected_commits(), vec![sha(1)])
    });

    table.update_in(cx, |table, window, cx| {
        table.extend_selection_down(&ExtendSelectionDown, window, cx)
    });
    table.read_with(&*cx, |table, _| {
        assert_eq!(table.selected_commits(), vec![sha(1), sha(2)])
    });

    table.update_in(cx, |table, window, cx| {
        table.copy_revision_number(&CopyRevisionNumber, window, cx)
    });
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(format!("{} {}", sha(2), sha(1)))
    );

    table.update_in(cx, |table, window, cx| {
        table.select_previous(&menu::SelectPrevious, window, cx)
    });
    table.read_with(&*cx, |table, _| {
        assert_eq!(table.selected_commits(), vec![sha(1)])
    });

    table.update_in(cx, |table, window, cx| {
        table.select_all(&SelectAll, window, cx)
    });
    table.read_with(&*cx, |table, _| {
        assert_eq!(table.selected_commits(), vec![sha(1), sha(2), sha(3)])
    });
}

#[gpui::test]
async fn jump_to_revision_prefers_references_then_hash_prefixes(cx: &mut TestAppContext) {
    init_test(cx);
    let project = project_with_commits(cx).await;
    let cx = cx.add_empty_window();
    let table = open_table(&project, cx);

    table.update_in(cx, |table, window, cx| {
        table.jump_to_revision("v1".into(), window, cx)
    });
    table.read_with(&*cx, |table, _| {
        assert_eq!(table.selected_commits(), vec![sha(2)])
    });

    table.update_in(cx, |table, window, cx| {
        table.jump_to_revision("(030303)".into(), window, cx)
    });
    table.read_with(&*cx, |table, _| {
        assert_eq!(table.selected_commits(), vec![sha(3)])
    });

    table.update_in(cx, |table, window, cx| {
        table.jump_to_revision("main".into(), window, cx)
    });
    table.read_with(&*cx, |table, _| {
        assert_eq!(table.selected_commits(), vec![sha(1)])
    });

    table.update_in(cx, |table, window, cx| {
        table.jump_to_revision("no-such-ref".into(), window, cx)
    });
    table.read_with(&*cx, |table, _| {
        assert_eq!(table.selected_commits(), vec![sha(1)]);
        assert_eq!(table.pending_jump, None);
    });
}

#[gpui::test]
async fn go_to_parent_and_child_follow_graph_edges(cx: &mut TestAppContext) {
    init_test(cx);
    let project = project_with_commits(cx).await;
    let cx = cx.add_empty_window();
    let table = open_table(&project, cx);

    table.update_in(cx, |table, window, cx| {
        table.select_first(&menu::SelectFirst, window, cx);
        table.go_to_parent_commit(&GoToParentCommit, window, cx);
    });
    table.read_with(&*cx, |table, _| {
        assert_eq!(table.selected_commits(), vec![sha(2)])
    });

    table.update_in(cx, |table, window, cx| {
        table.go_to_child_commit(&GoToChildCommit, window, cx)
    });
    table.read_with(&*cx, |table, _| {
        assert_eq!(table.selected_commits(), vec![sha(1)])
    });

    table.update_in(cx, |table, window, cx| {
        table.select_all(&SelectAll, window, cx);
        table.go_to_parent_commit(&GoToParentCommit, window, cx);
    });
    table.read_with(&*cx, |table, _| {
        assert_eq!(table.selected_commits().len(), 3)
    });
}

#[gpui::test]
async fn cherry_pick_is_enabled_only_for_selections_not_already_being_picked(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = project_with_commits(cx).await;
    let cx = cx.add_empty_window();
    let table = open_table(&project, cx);

    table.read_with(&*cx, |table, _| assert!(!table.can_cherry_pick()));

    table.update_in(cx, |table, window, cx| {
        table.select_last(&menu::SelectLast, window, cx)
    });
    table.read_with(&*cx, |table, _| assert!(table.can_cherry_pick()));

    table.update(cx, |table, _| {
        table.cherry_picks_in_progress.insert(sha(3));
    });
    table.read_with(&*cx, |table, _| assert!(!table.can_cherry_pick()));

    table.update(cx, |table, cx| table.set_repository(None, cx));
    table.read_with(&*cx, |table, _| assert!(!table.can_cherry_pick()));
}
