use std::{path::Path, sync::Arc};

use git::{
    Oid,
    repository::{CommitData, InitialGraphCommitData},
};
use gpui::{TestAppContext, VisualTestContext, point, px, size};
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
    let table = create_table(project, cx);
    draw_table(&table, cx);
    table
}

fn create_table(project: &Entity<Project>, cx: &mut VisualTestContext) -> Entity<CommitTable> {
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

fn draw_table(table: &Entity<CommitTable>, cx: &mut VisualTestContext) {
    let table = table.clone();
    cx.draw(point(px(0.), px(0.)), size(px(800.), px(600.)), |_, _| {
        table.into_any_element()
    });
    cx.run_until_parked();
}

#[gpui::test]
async fn a_history_streamed_in_chunks_is_shown_whole_once_loaded(cx: &mut TestAppContext) {
    const COMMIT_COUNT: u32 = 5 * git::repository::GRAPH_CHUNK_SIZE as u32 + 7;
    init_test(cx);
    let numbered_sha =
        |number: u32| Oid::from_bytes(&[&number.to_be_bytes()[..], &[1; 16]].concat()).unwrap();
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree(
        Path::new("/project"),
        json!({ ".git": {}, "file.txt": "content" }),
    )
    .await;
    fs.set_graph_commits(
        Path::new("/project/.git"),
        (0..COMMIT_COUNT)
            .map(|number| {
                Arc::new(InitialGraphCommitData {
                    sha: numbered_sha(number),
                    parents: (number + 1 < COMMIT_COUNT)
                        .then(|| numbered_sha(number + 1))
                        .into_iter()
                        .collect(),
                    ref_names: if number == 0 {
                        vec!["HEAD -> main".into()]
                    } else {
                        Vec::new()
                    },
                    committer_timestamp: i64::from(COMMIT_COUNT - number),
                    dotted_parents: smallvec![],
                })
            })
            .collect(),
    );
    let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;
    cx.run_until_parked();
    let cx = cx.add_empty_window();
    let table = open_table(&project, cx);

    table.read_with(&*cx, |table, _| {
        assert_eq!(table.row_count(), COMMIT_COUNT as usize);
        assert_eq!(
            table
                .displayed
                .as_ref()
                .and_then(|displayed| displayed.contained_in_head.as_ref())
                .map(|contained| contained.iter().filter(|reachable| **reachable).count()),
            Some(COMMIT_COUNT as usize),
            "every commit of a linear history is reachable from HEAD"
        );
    });
}

#[gpui::test]
async fn history_loads_once_the_table_is_first_drawn(cx: &mut TestAppContext) {
    init_test(cx);
    let project = project_with_commits(cx).await;
    let cx = cx.add_empty_window();
    let table = create_table(&project, cx);
    let repository = project.read_with(&*cx, |project, cx| {
        project
            .active_repository(cx)
            .expect("the project should have a repository")
    });
    let log_source = table.read_with(&*cx, |table, _| table.log_source());

    table.read_with(&*cx, |table, _| assert_eq!(table.row_count(), 0));
    repository.read_with(&*cx, |repository, _| {
        assert!(
            repository
                .get_graph_data(log_source.clone(), LOG_ORDER)
                .is_none(),
            "a table that was never drawn shouldn't load the history"
        )
    });

    draw_table(&table, cx);
    table.read_with(&*cx, |table, _| assert_eq!(table.row_count(), 3));
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
