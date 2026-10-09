use std::{cell::RefCell, rc::Rc, sync::Arc, time::Duration};

use editor::Editor;
use futures::StreamExt;
use gpui::{App, Entity, Focusable, TestAppContext, VisualContext, VisualTestContext};
use language::{FakeLspAdapter, Language, LanguageConfig, LanguageMatcher};
use lsp::OneOf;
use picker::{Direction, Picker, PickerDelegate};
use project::{FakeFs, Project, ProjectEntryId, ProjectPath};
use serde_json::json;
use settings::{KeymapFile, SettingsStore, ThemeColorsContent, ThemeStyleContent};
use theme::ActiveTheme as _;
use util::{path, rel_path::rel_path};
use workspace::{AppState, MultiWorkspace, Workspace};
use zed_actions::search_everywhere::{Tab, Toggle};

use crate::{
    SearchEverywhere,
    delegate::{Entry, SEARCH_DEBOUNCE, SearchEverywhereDelegate, Section},
};

pub(crate) fn init_test(cx: &mut TestAppContext) -> Arc<AppState> {
    cx.update(|cx| {
        let app_state = AppState::test(cx);
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        release_channel::init(semver::Version::new(0, 0, 0), cx);
        editor::init(cx);
        crate::init(cx);
        cx.bind_keys(KeymapFile::load_panic_on_failure(
            r#"[
                {
                    "context": "Picker > Editor",
                    "bindings": {
                        "tab": "picker::ConfirmCompletion",
                        "enter": "menu::Confirm",
                        "up": "menu::SelectPrevious",
                        "down": "menu::SelectNext"
                    }
                },
                {
                    "context": "SearchEverywhere > Picker > Editor",
                    "bindings": {
                        "tab": "search_everywhere::NextTab",
                        "shift-tab": "search_everywhere::PreviousTab",
                        "ctrl-down": "search_everywhere::NextSection",
                        "ctrl-up": "search_everywhere::PreviousSection"
                    }
                }
            ]"#,
            cx,
        ));
        app_state
    })
}

pub(crate) fn build_workspace(
    project: Entity<Project>,
    cx: &mut TestAppContext,
) -> (Entity<Workspace>, &mut VisualTestContext) {
    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project, window, cx));
    let workspace =
        multi_workspace.read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone());
    (workspace, cx)
}

fn active_picker(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Entity<Picker<SearchEverywhereDelegate>> {
    workspace.update(cx, |workspace, cx| {
        workspace
            .active_modal::<SearchEverywhere>(cx)
            .expect("search everywhere is not open")
            .read(cx)
            .picker
            .clone()
    })
}

fn open_search_everywhere(
    tab: Option<Tab>,
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Entity<Picker<SearchEverywhereDelegate>> {
    cx.dispatch_action(Toggle { tab });
    cx.run_until_parked();
    active_picker(workspace, cx)
}

pub(crate) fn settle(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(SEARCH_DEBOUNCE);
    cx.run_until_parked();
}

pub(crate) fn type_query(cx: &mut VisualTestContext, query: &str) {
    cx.simulate_input(query);
    settle(cx);
}

fn describe_entry(delegate: &SearchEverywhereDelegate, entry: Entry) -> String {
    match entry {
        Entry::Header(section) => format!("# {section:?}"),
        Entry::More(section) => format!("more {section:?}"),
        Entry::OpenInTextFinder => "open in text finder".to_string(),
        Entry::Item(section, index) => match section {
            Section::RecentFiles => delegate.recent_files[index]
                .project
                .path
                .as_unix_str()
                .to_string(),
            Section::Files => delegate.results.files[index].path.as_unix_str().to_string(),
            Section::Classes => {
                let candidate_id = delegate.results.classes[index].candidate_id;
                delegate.results.symbols[candidate_id].name.clone()
            }
            Section::Symbols => {
                let candidate_id = delegate.results.other_symbols[index].candidate_id;
                delegate.results.symbols[candidate_id].name.clone()
            }
            Section::Actions => delegate.results.actions[index].string.to_string(),
            Section::Text => {
                let search_match = &delegate.results.text[index];
                format!(
                    "{}:{}",
                    search_match.path.path.as_unix_str(),
                    search_match.line_number
                )
            }
        },
    }
}

pub(crate) fn entries(
    picker: &Entity<Picker<SearchEverywhereDelegate>>,
    cx: &mut VisualTestContext,
) -> Vec<String> {
    picker.read_with(cx, |picker, _| {
        picker
            .delegate
            .entries
            .iter()
            .map(|entry| describe_entry(&picker.delegate, *entry))
            .collect()
    })
}

pub(crate) fn selected_entry(
    picker: &Entity<Picker<SearchEverywhereDelegate>>,
    cx: &mut VisualTestContext,
) -> String {
    picker.read_with(cx, |picker, _| {
        let delegate = &picker.delegate;
        describe_entry(delegate, delegate.entries[delegate.selected_index])
    })
}

fn symbol_information(name: &str, kind: lsp::SymbolKind, path: &str) -> lsp::SymbolInformation {
    #[allow(deprecated)]
    lsp::SymbolInformation {
        name: name.to_string(),
        kind,
        tags: None,
        deprecated: None,
        container_name: None,
        location: lsp::Location::new(
            lsp::Uri::from_file_path(path).unwrap(),
            lsp::Range::new(lsp::Position::new(0, 0), lsp::Position::new(0, 0)),
        ),
    }
}

async fn project_with_symbols(
    symbols: Vec<lsp::SymbolInformation>,
    slow_query: Option<&'static str>,
    cx: &mut TestAppContext,
) -> (
    Entity<Project>,
    (
        Entity<language::Buffer>,
        project::lsp_store::OpenLspBufferHandle,
    ),
) {
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree(path!("/dir"), json!({ "test.rs": "" }))
        .await;
    let project = Project::test(fs, [path!("/dir").as_ref()], cx).await;

    let language_registry = project.read_with(cx, |project, _| project.languages().clone());
    language_registry.add(Arc::new(Language::new(
        LanguageConfig {
            name: "Rust".into(),
            matcher: LanguageMatcher {
                path_suffixes: vec!["rs".to_string()],
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
        None,
    )));
    let mut fake_servers = language_registry.register_fake_lsp(
        "Rust",
        FakeLspAdapter {
            capabilities: lsp::ServerCapabilities {
                workspace_symbol_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            ..Default::default()
        },
    );

    let buffer = project
        .update(cx, |project, cx| {
            project.open_local_buffer_with_lsp(path!("/dir/test.rs"), cx)
        })
        .await
        .unwrap();

    let fake_server = fake_servers.next().await.unwrap();
    fake_server.set_request_handler::<lsp::WorkspaceSymbolRequest, _, _>(
        move |params: lsp::WorkspaceSymbolParams, cx| {
            let executor = cx.background_executor().clone();
            let symbols = symbols.clone();
            async move {
                if slow_query == Some(params.query.as_str()) {
                    executor.timer(Duration::from_secs(1)).await;
                }
                let query = params.query.to_lowercase();
                Ok(Some(lsp::WorkspaceSymbolResponse::Flat(
                    symbols
                        .into_iter()
                        .filter(|symbol| is_subsequence(&query, &symbol.name.to_lowercase()))
                        .collect(),
                )))
            }
        },
    );
    (project, buffer)
}

fn is_subsequence(query: &str, candidate: &str) -> bool {
    let mut candidate_chars = candidate.chars();
    query
        .chars()
        .all(|query_char| candidate_chars.any(|candidate_char| candidate_char == query_char))
}

#[gpui::test]
async fn test_empty_query_lists_recent_files(cx: &mut TestAppContext) {
    let app_state = init_test(cx);
    app_state
        .fs
        .as_fake()
        .insert_tree(
            path!("/dir"),
            json!({ "first.txt": "", "second.txt": "", "third.txt": "" }),
        )
        .await;
    let project = Project::test(app_state.fs.clone(), [path!("/dir").as_ref()], cx).await;
    let (workspace, cx) = build_workspace(project.clone(), cx);
    let worktree_id = cx.read(|cx| project.read(cx).worktrees(cx).next().unwrap().read(cx).id());

    for file_name in ["first.txt", "second.txt"] {
        workspace
            .update_in(cx, |workspace, window, cx| {
                workspace.open_path(
                    ProjectPath {
                        worktree_id,
                        path: rel_path(file_name).into(),
                    },
                    None,
                    true,
                    window,
                    cx,
                )
            })
            .await
            .unwrap();
    }

    let picker = open_search_everywhere(None, &workspace, cx);
    assert_eq!(
        entries(&picker, cx),
        vec!["# RecentFiles", "second.txt", "first.txt"],
        "an empty query on the All tab lists recently opened files, most recent first"
    );
    assert_eq!(selected_entry(&picker, cx), "second.txt");

    cx.dispatch_action(Toggle {
        tab: Some(Tab::Files),
    });
    cx.run_until_parked();
    assert_eq!(
        entries(&picker, cx),
        vec!["second.txt", "first.txt"],
        "the Files tab lists recent files without a section header"
    );

    cx.dispatch_action(Toggle {
        tab: Some(Tab::Classes),
    });
    cx.run_until_parked();
    assert!(
        entries(&picker, cx).is_empty(),
        "tabs other than All and Files have nothing to show for an empty query"
    );
}

#[gpui::test]
async fn test_all_tab_groups_results_and_more_row_switches_tab(cx: &mut TestAppContext) {
    let app_state = init_test(cx);
    app_state
        .fs
        .as_fake()
        .insert_tree(
            path!("/dir"),
            json!({
                "qzalpha1.txt": "",
                "qzalpha2.txt": "",
                "qzalpha3.txt": "",
                "qzalpha4.txt": "",
                "qzalpha5.txt": "",
                "qzalpha6.txt": "",
                "qzalpha7.txt": "",
                "qzalpha8.txt": "",
            }),
        )
        .await;
    let project = Project::test(app_state.fs.clone(), [path!("/dir").as_ref()], cx).await;
    let (workspace, cx) = build_workspace(project, cx);

    let picker = open_search_everywhere(None, &workspace, cx);
    type_query(cx, "qzalpha");

    let all_entries = entries(&picker, cx);
    assert_eq!(all_entries.first().map(String::as_str), Some("# Files"));
    assert_eq!(
        all_entries.len(),
        8,
        "the All tab shows a header, six files and a more row: {all_entries:?}"
    );
    assert_eq!(all_entries.last().map(String::as_str), Some("more Files"));
    assert!(
        !all_entries.iter().any(|entry| entry == "# Text"),
        "no file contains the query, so the All tab has no text section"
    );

    picker.update_in(cx, |picker, window, cx| {
        assert_eq!(
            picker.delegate.selected_index, 1,
            "the first item is selected"
        );
        assert!(!picker.delegate.can_select(0, window, cx));
        picker.set_selected_index(0, Some(Direction::Up), true, window, cx);
        assert_eq!(
            picker.delegate.selected_index, 7,
            "moving up from the header skips it and wraps to the last row"
        );
    });
    assert_eq!(selected_entry(&picker, cx), "more Files");

    cx.dispatch_action(menu::Confirm);
    settle(cx);

    picker.read_with(cx, |picker, cx| {
        assert_eq!(picker.delegate.tab(), Tab::Files);
        assert_eq!(picker.query(cx), "qzalpha");
    });
    let file_entries = entries(&picker, cx);
    assert_eq!(file_entries.len(), 8, "{file_entries:?}");
    assert!(
        file_entries
            .iter()
            .all(|entry| entry.starts_with("qzalpha")),
        "the Files tab lists every match without headers: {file_entries:?}"
    );
}

#[gpui::test]
async fn test_file_query_with_position_moves_caret(cx: &mut TestAppContext) {
    let app_state = init_test(cx);
    app_state
        .fs
        .as_fake()
        .insert_tree(
            path!("/src"),
            json!({
                "test": {
                    "first.rs": "// First Rust file",
                    "second.rs": "// Second Rust file",
                }
            }),
        )
        .await;
    let project = Project::test(app_state.fs.clone(), [path!("/src").as_ref()], cx).await;
    let (workspace, cx) = build_workspace(project, cx);

    let picker = open_search_everywhere(Some(Tab::Files), &workspace, cx);
    type_query(cx, "first:1:3");
    assert_eq!(selected_entry(&picker, cx), "test/first.rs");

    cx.dispatch_action(menu::Confirm);
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();

    let editor = cx.update(|_, cx| workspace.read(cx).active_item_as::<Editor>(cx).unwrap());
    editor.update(cx, |editor, cx| {
        assert_eq!(editor.title(cx), "first.rs");
        let selections = editor.selections.all_adjusted(&editor.display_snapshot(cx));
        assert_eq!(selections.len(), 1);
        let caret = &selections[0];
        assert_eq!(caret.start, caret.end, "the caret is collapsed");
        assert_eq!(caret.start.row, 0, "row 1 is the first line");
        assert_eq!(caret.start.column, 2, "column 3 is the third character");
    });
    workspace.read_with(cx, |workspace, cx| {
        assert!(workspace.active_modal::<SearchEverywhere>(cx).is_none());
    });
}

#[gpui::test]
async fn test_folder_confirm_reveals_it_in_project_panel(cx: &mut TestAppContext) {
    let app_state = init_test(cx);
    app_state
        .fs
        .as_fake()
        .insert_tree(path!("/dir"), json!({ "qzfolder": { "inner.txt": "" } }))
        .await;
    let project = Project::test(app_state.fs.clone(), [path!("/dir").as_ref()], cx).await;
    let (workspace, cx) = build_workspace(project.clone(), cx);

    let revealed: Rc<RefCell<Vec<ProjectEntryId>>> = Rc::default();
    cx.update(|_, cx| {
        let revealed = revealed.clone();
        cx.subscribe(&project, move |_, event, _| {
            if let project::Event::RevealInProjectPanel(entry_id) = event {
                revealed.borrow_mut().push(*entry_id);
            }
        })
        .detach();
    });

    let picker = open_search_everywhere(Some(Tab::Files), &workspace, cx);
    type_query(cx, "qzfolder");
    assert_eq!(entries(&picker, cx), vec!["qzfolder", "qzfolder/inner.txt"]);
    assert_eq!(selected_entry(&picker, cx), "qzfolder");

    cx.dispatch_action(menu::Confirm);
    cx.run_until_parked();

    let folder_entry = cx.read(|cx: &App| {
        let worktree = project.read(cx).worktrees(cx).next().unwrap();
        worktree
            .read(cx)
            .entry_for_path(rel_path("qzfolder"))
            .unwrap()
            .id
    });
    assert_eq!(*revealed.borrow(), vec![folder_entry]);
    workspace.read_with(cx, |workspace, cx| {
        assert!(workspace.active_modal::<SearchEverywhere>(cx).is_none());
        assert!(
            workspace.active_item(cx).is_none(),
            "confirming a folder does not open an editor"
        );
    });
}

#[gpui::test]
async fn test_symbol_tabs_filter_by_kind(cx: &mut TestAppContext) {
    init_test(cx);
    let (project, _buffer) = project_with_symbols(
        vec![
            symbol_information("QzWidget", lsp::SymbolKind::CLASS, path!("/dir/test.rs")),
            symbol_information(
                "qz_widget_factory",
                lsp::SymbolKind::FUNCTION,
                path!("/dir/test.rs"),
            ),
        ],
        None,
        cx,
    )
    .await;
    let (workspace, cx) = build_workspace(project, cx);

    let picker = open_search_everywhere(Some(Tab::Classes), &workspace, cx);
    type_query(cx, "qzwid");
    assert_eq!(
        entries(&picker, cx),
        vec!["QzWidget"],
        "the Classes tab only lists class-like symbols"
    );

    cx.dispatch_action(Toggle {
        tab: Some(Tab::Symbols),
    });
    settle(cx);
    let mut symbol_entries = entries(&picker, cx);
    symbol_entries.sort();
    assert_eq!(
        symbol_entries,
        vec!["QzWidget", "qz_widget_factory"],
        "the Symbols tab lists every symbol kind"
    );

    cx.dispatch_action(Toggle {
        tab: Some(Tab::All),
    });
    settle(cx);
    assert_eq!(
        entries(&picker, cx),
        vec!["# Classes", "QzWidget", "# Symbols", "qz_widget_factory"],
        "the All tab shows classes and the remaining symbols in their own sections"
    );
    picker.read_with(cx, |picker, _| {
        assert!(
            !picker.delegate.include_non_project_items,
            "switching to a different tab does not toggle non-project items"
        );
    });
}

#[gpui::test]
async fn test_stale_symbol_results_are_dropped(cx: &mut TestAppContext) {
    init_test(cx);
    let (project, _buffer) = project_with_symbols(
        vec![symbol_information(
            "QzWidget",
            lsp::SymbolKind::CLASS,
            path!("/dir/test.rs"),
        )],
        Some("qzwid"),
        cx,
    )
    .await;
    let (workspace, cx) = build_workspace(project, cx);
    let picker = open_search_everywhere(Some(Tab::Classes), &workspace, cx);

    let first_search = picker.update_in(cx, |picker, window, cx| {
        picker
            .delegate
            .update_matches("qzwid".to_string(), window, cx)
    });
    cx.run_until_parked();
    let second_search = picker.update_in(cx, |picker, window, cx| {
        picker
            .delegate
            .update_matches("qzmissing".to_string(), window, cx)
    });
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();

    picker.read_with(cx, |picker, _| {
        assert_eq!(picker.delegate.query, "qzmissing");
        assert!(
            picker.delegate.results.classes.is_empty(),
            "the slow response for the superseded query must not replace newer results"
        );
        assert!(picker.delegate.entries.is_empty());
    });
    drop(first_search);
    drop(second_search);
}

#[gpui::test]
async fn test_actions_run_on_previously_focused_editor(cx: &mut TestAppContext) {
    let app_state = init_test(cx);
    let project = Project::test(app_state.fs.clone(), [], cx).await;
    let (workspace, cx) = build_workspace(project, cx);

    let editor = cx.new_window_entity(|window, cx| {
        let mut editor = Editor::single_line(window, cx);
        editor.set_text("abc", window, cx);
        editor
    });
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.add_item_to_active_pane(Box::new(editor.clone()), None, true, window, cx);
        editor.update(cx, |editor, cx| window.focus(&editor.focus_handle(cx), cx));
    });

    let picker = open_search_everywhere(Some(Tab::Actions), &workspace, cx);
    picker.read_with(cx, |picker, _| {
        assert!(
            picker.delegate.entries.len() > 5,
            "the Actions tab lists all actions for an empty query"
        );
    });
    type_query(cx, "bcksp");
    assert_eq!(selected_entry(&picker, cx), "editor: backspace");

    cx.dispatch_action(menu::Confirm);
    cx.run_until_parked();

    workspace.update(cx, |workspace, cx| {
        assert!(workspace.active_modal::<SearchEverywhere>(cx).is_none());
        assert_eq!(editor.read(cx).text(cx), "ab");
    });
}

#[gpui::test]
async fn test_tab_keys_cycle_tabs_and_keep_query(cx: &mut TestAppContext) {
    let app_state = init_test(cx);
    let project = Project::test(app_state.fs.clone(), [], cx).await;
    let (workspace, cx) = build_workspace(project, cx);

    let picker = open_search_everywhere(None, &workspace, cx);
    type_query(cx, "qz");

    let tab_after = |keystrokes: &str, cx: &mut VisualTestContext| {
        cx.simulate_keystrokes(keystrokes);
        settle(cx);
        picker.read_with(cx, |picker, cx| {
            assert_eq!(picker.query(cx), "qz", "switching tabs keeps the query");
            picker.delegate.tab()
        })
    };

    assert_eq!(tab_after("tab", cx), Tab::Classes);
    assert_eq!(tab_after("tab", cx), Tab::Files);
    assert_eq!(tab_after("shift-tab", cx), Tab::Classes);
    assert_eq!(tab_after("shift-tab", cx), Tab::All);
    assert_eq!(
        tab_after("shift-tab", cx),
        Tab::Text,
        "moving back from the first tab wraps to the last one"
    );
    assert_eq!(tab_after("tab", cx), Tab::All);
}

#[gpui::test]
async fn test_toggle_with_tab_preselects_and_repeat_toggles_non_project_items(
    cx: &mut TestAppContext,
) {
    let app_state = init_test(cx);
    app_state
        .fs
        .as_fake()
        .insert_tree(
            path!("/dir"),
            json!({
                ".gitignore": "qzignored.txt",
                "qzignored.txt": "",
                "qzvisible.txt": "",
            }),
        )
        .await;
    let project = Project::test(app_state.fs.clone(), [path!("/dir").as_ref()], cx).await;
    let (workspace, cx) = build_workspace(project, cx);

    let picker = open_search_everywhere(Some(Tab::Files), &workspace, cx);
    picker.read_with(cx, |picker, _| {
        assert_eq!(picker.delegate.tab(), Tab::Files);
        assert!(!picker.delegate.include_non_project_items);
    });
    type_query(cx, "qz");
    assert_eq!(entries(&picker, cx), vec!["qzvisible.txt"]);

    cx.dispatch_action(Toggle {
        tab: Some(Tab::Files),
    });
    settle(cx);
    picker.read_with(cx, |picker, _| {
        assert!(
            picker.delegate.include_non_project_items,
            "toggling the active tab again includes non-project items"
        );
        assert_eq!(picker.delegate.tab(), Tab::Files);
    });
    let mut file_entries = entries(&picker, cx);
    file_entries.sort();
    assert_eq!(file_entries, vec!["qzignored.txt", "qzvisible.txt"]);

    cx.dispatch_action(Toggle { tab: None });
    settle(cx);
    assert_eq!(
        entries(&picker, cx),
        vec!["qzvisible.txt"],
        "toggling again excludes non-project items"
    );
}

#[gpui::test]
async fn test_text_results_in_text_tab_and_all_tab(cx: &mut TestAppContext) {
    let app_state = init_test(cx);
    app_state
        .fs
        .as_fake()
        .insert_tree(
            path!("/dir"),
            json!({ "notes.md": "first line\nfind the qzneedle here\n" }),
        )
        .await;
    let project = Project::test(app_state.fs.clone(), [path!("/dir").as_ref()], cx).await;
    let (workspace, cx) = build_workspace(project, cx);

    let picker = open_search_everywhere(Some(Tab::Text), &workspace, cx);
    type_query(cx, "qzneedle");
    assert_eq!(
        entries(&picker, cx),
        vec!["notes.md:2", "open in text finder"]
    );

    cx.dispatch_action(Toggle {
        tab: Some(Tab::All),
    });
    settle(cx);
    assert_eq!(
        entries(&picker, cx),
        vec!["# Text", "notes.md:2"],
        "the All tab shows text matches in their own section"
    );
}

#[gpui::test]
async fn test_all_tab_shows_text_matches_even_when_other_sections_are_full(
    cx: &mut TestAppContext,
) {
    let app_state = init_test(cx);
    app_state
        .fs
        .as_fake()
        .insert_tree(
            path!("/dir"),
            json!({
                "notes.md": "qzneedle",
                "qzneedle1.txt": "",
                "qzneedle2.txt": "",
                "qzneedle3.txt": "",
                "qzneedle4.txt": "",
                "qzneedle5.txt": "",
                "qzneedle6.txt": "",
                "qzneedle7.txt": "",
            }),
        )
        .await;
    let project = Project::test(app_state.fs.clone(), [path!("/dir").as_ref()], cx).await;
    let (workspace, cx) = build_workspace(project, cx);

    let picker = open_search_everywhere(None, &workspace, cx);
    type_query(cx, "qzneedle");
    let all_entries = entries(&picker, cx);
    assert_eq!(all_entries.first().map(String::as_str), Some("# Files"));
    assert!(
        all_entries.iter().any(|entry| entry == "more Files"),
        "the files section is capped and offers a more row: {all_entries:?}"
    );
    assert_eq!(
        &all_entries[all_entries.len() - 2..],
        ["# Text", "notes.md:1"],
        "text matches are listed after the full files section: {all_entries:?}"
    );
}

#[gpui::test]
async fn test_all_tab_aggregates_files_classes_symbols_actions_and_text(cx: &mut TestAppContext) {
    init_test(cx);
    let (project, _buffer) = project_with_symbols(
        vec![
            symbol_information("BckspWidget", lsp::SymbolKind::CLASS, path!("/dir/test.rs")),
            symbol_information(
                "bcksp_factory",
                lsp::SymbolKind::FUNCTION,
                path!("/dir/test.rs"),
            ),
        ],
        None,
        cx,
    )
    .await;
    project
        .read_with(cx, |project, _| project.fs().clone())
        .as_fake()
        .insert_file(path!("/dir/bcksp_notes.md"), "a bcksp line".into())
        .await;
    cx.run_until_parked();
    let (workspace, cx) = build_workspace(project, cx);
    let editor = cx.new_window_entity(|window, cx| Editor::single_line(window, cx));
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.add_item_to_active_pane(Box::new(editor.clone()), None, true, window, cx);
        editor.update(cx, |editor, cx| window.focus(&editor.focus_handle(cx), cx));
    });

    let picker = open_search_everywhere(None, &workspace, cx);
    type_query(cx, "bcksp");

    let headers = entries(&picker, cx)
        .into_iter()
        .filter(|entry| entry.starts_with("# "))
        .collect::<Vec<_>>();
    assert_eq!(
        headers,
        ["# Classes", "# Files", "# Symbols", "# Actions", "# Text"],
        "every provider contributes a section to the All tab"
    );
}

#[gpui::test]
async fn test_search_everywhere_surface_is_opaque_with_translucent_theme(cx: &mut TestAppContext) {
    let app_state = init_test(cx);
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, cx| {
            store.update_user_settings(cx, |settings| {
                settings.theme.experimental_theme_overrides = Some(ThemeStyleContent {
                    colors: ThemeColorsContent {
                        elevated_surface_background: Some("#22272f99".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                });
            });
        });
    });
    let project = Project::test(app_state.fs.clone(), [], cx).await;
    let (workspace, cx) = build_workspace(project, cx);
    cx.run_until_parked();
    let theme_surface = cx.update(|_, cx| cx.theme().colors().elevated_surface_background);
    assert!(
        theme_surface.a < 1.0,
        "the theme override makes the elevated surface translucent"
    );

    let picker = open_search_everywhere(None, &workspace, cx);
    let surface = picker.read_with(cx, |picker, cx| picker.opaque_surface_background(cx));
    let surface = surface.expect("the search everywhere picker paints an opaque surface");
    assert_eq!(surface.a, 1.0);
    assert_eq!(surface.h, theme_surface.h);
    assert_eq!(surface.s, theme_surface.s);
    assert_eq!(surface.l, theme_surface.l);
}

#[gpui::test]
async fn test_section_navigation_jumps_between_sections(cx: &mut TestAppContext) {
    init_test(cx);
    let (project, _buffer) = project_with_symbols(
        vec![
            symbol_information("QzWidget", lsp::SymbolKind::CLASS, path!("/dir/test.rs")),
            symbol_information(
                "qz_widget_factory",
                lsp::SymbolKind::FUNCTION,
                path!("/dir/test.rs"),
            ),
        ],
        None,
        cx,
    )
    .await;
    let (workspace, cx) = build_workspace(project, cx);

    let picker = open_search_everywhere(None, &workspace, cx);
    type_query(cx, "qzwid");
    assert_eq!(selected_entry(&picker, cx), "QzWidget");

    cx.simulate_keystrokes("ctrl-down");
    assert_eq!(selected_entry(&picker, cx), "qz_widget_factory");
    cx.simulate_keystrokes("ctrl-down");
    assert_eq!(
        selected_entry(&picker, cx),
        "QzWidget",
        "jumping past the last section wraps to the first"
    );
    cx.simulate_keystrokes("ctrl-up");
    assert_eq!(selected_entry(&picker, cx), "qz_widget_factory");
}
