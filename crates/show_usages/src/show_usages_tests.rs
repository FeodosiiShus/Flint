use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use editor::{
    Editor,
    actions::ShowUsages,
    test::{editor_lsp_test_context::EditorLspTestContext, editor_test_context::EditorTestContext},
};
use futures::{StreamExt as _, channel::oneshot};
use gpui::{
    BorrowAppContext as _, Entity, Focusable as _, KeyBinding, Modifiers, Pixels, Point,
    SharedString, TestAppContext, point,
};
use indoc::indoc;
use pretty_assertions::assert_eq;
use project::Location;
use serde_json::json;
use settings::{CodeLens, SettingsStore};
use util::path;
use workspace::{Item as _, ModalView as _};

use crate::{
    OpenInFindWindow, ToggleWriteAccess, UsagesKind, UsagesTarget,
    hint::UsagesHint,
    model::{
        DisplayRow, ShowUsagesState, USAGES_PAGE_SIZE, UsageAccess, UsageFilters, UsageRow,
        UsageScope,
    },
    popup::ShowUsagesPopup,
};

const POPUP_DELAY: Duration = Duration::from_millis(300);

const MAIN_SOURCE: &str = indoc! {"
    fn helper() -> u32 {
        1
    }

    fn main() {
        let total = heˇlper() + helper();
        let other = helper();
    }
"};

const OTHER_SOURCE: &str = indoc! {"
    fn first() -> u32 {
        helper()
    }

    fn second() -> u32 {
        helper() + 1
    }
"};

const SCOPED_MAIN_SOURCE: &str = indoc! {"
    fn helper() -> u32 {
        1
    }

    fn main() {
        let first = heˇlper();
        let second = helper();
    }
"};

const SCOPED_OTHER_SOURCE: &str = indoc! {"
    fn uses() -> u32 {
        helper()
    }
"};

const OUTSIDE_SOURCE: &str = indoc! {"
    fn external() -> u32 {
        helper()
    }
"};

const COUNTER_SOURCE: &str = indoc! {"
    fn main() {
        let mut counter = 0;
        counter = 5;
        let first = couˇnter;
        let second = counter;
    }
"};

const SPEED_SEARCH_SOURCE: &str = indoc! {"
    fn heˇlper() -> u32 {
        1
    }

    fn main() {
        let alpha = helper();
        let beta = helper();
        let gamma = helper();
        let gamma_total = gamma + helper();
    }
"};

const LENS_SOURCE: &str = indoc! {"
    ˇfunction hello() {}
    const first = 1;
    const second = 2;
    const third = 3;
    const fourth = 4;
    const fifth = 5;
    const sixth = 6;
    const seventh = 7;
    const eighth = 8;
    const ninth = 9;
    hello();
"};

struct PopupState {
    title: SharedString,
    kind: UsagesKind,
    scope: UsageScope,
    searching: bool,
    status: Option<SharedString>,
    display_rows: Vec<DisplayRow>,
    rows: Vec<UsageRow<Location>>,
    selected: Option<usize>,
    query: String,
    filters: UsageFilters,
    anchor: Option<Point<Pixels>>,
}

struct HintState {
    message: SharedString,
    warning: bool,
    second_invocation: Option<SharedString>,
    scope: UsageScope,
}

fn init_show_usages(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::init(cx);
        crate::save_state(&ShowUsagesState::default(), cx);
    });
}

async fn show_usages_context(
    capabilities: lsp::ServerCapabilities,
    cx: &mut TestAppContext,
) -> EditorLspTestContext {
    init_show_usages(cx);
    EditorLspTestContext::new_rust(capabilities, cx).await
}

async fn two_file_context(cx: &mut TestAppContext) -> (EditorLspTestContext, lsp::Uri, lsp::Uri) {
    let mut cx = show_usages_context(references_and_definition_capabilities(), cx).await;
    insert_tree(
        &mut cx,
        path!("/root/dir"),
        json!({ "other.rs": OTHER_SOURCE }),
    )
    .await;
    cx.set_state(MAIN_SOURCE);
    let main_uri = cx.buffer_lsp_url.clone();
    let other_uri = lsp::Uri::from_file_path(path!("/root/dir/other.rs")).unwrap();
    handle_definition(&cx, location(&main_uri, 0, 3, 9));
    (cx, main_uri, other_uri)
}

fn two_file_usages(main_uri: &lsp::Uri, other_uri: &lsp::Uri) -> Vec<lsp::Location> {
    vec![
        location(other_uri, 5, 4, 10),
        location(main_uri, 6, 16, 22),
        location(other_uri, 1, 4, 10),
        location(main_uri, 5, 27, 33),
        location(main_uri, 0, 3, 9),
        location(main_uri, 5, 16, 22),
    ]
}

fn references_capabilities() -> lsp::ServerCapabilities {
    lsp::ServerCapabilities {
        references_provider: Some(lsp::OneOf::Left(true)),
        ..lsp::ServerCapabilities::default()
    }
}

fn references_and_definition_capabilities() -> lsp::ServerCapabilities {
    lsp::ServerCapabilities {
        references_provider: Some(lsp::OneOf::Left(true)),
        definition_provider: Some(lsp::OneOf::Left(true)),
        ..lsp::ServerCapabilities::default()
    }
}

async fn insert_tree(cx: &mut EditorLspTestContext, path: &str, tree: serde_json::Value) {
    let fs = cx.update_workspace(|workspace, _, cx| workspace.project().read(cx).fs().clone());
    fs.as_fake().insert_tree(path, tree).await;
    cx.run_until_parked();
}

fn location(uri: &lsp::Uri, row: u32, start_column: u32, end_column: u32) -> lsp::Location {
    lsp::Location {
        uri: uri.clone(),
        range: lsp::Range::new(
            lsp::Position::new(row, start_column),
            lsp::Position::new(row, end_column),
        ),
    }
}

fn handle_references(cx: &EditorLspTestContext, locations: Vec<lsp::Location>) -> Arc<AtomicUsize> {
    let requests = Arc::new(AtomicUsize::new(0));
    cx.lsp
        .set_request_handler::<lsp::request::References, _, _>({
            let requests = requests.clone();
            move |_, _| {
                requests.fetch_add(1, Ordering::SeqCst);
                let locations = locations.clone();
                async move { Ok(Some(locations)) }
            }
        });
    requests
}

fn handle_definition(cx: &EditorLspTestContext, declaration: lsp::Location) {
    cx.lsp
        .set_request_handler::<lsp::request::GotoDefinition, _, _>(move |_, _| {
            let declaration = declaration.clone();
            async move { Ok(Some(lsp::GotoDefinitionResponse::Scalar(declaration))) }
        });
}

fn active_popup(cx: &mut EditorLspTestContext) -> Option<Entity<ShowUsagesPopup>> {
    let workspace = cx.workspace.clone();
    cx.update(|_, cx| workspace.read(cx).active_modal::<ShowUsagesPopup>(cx))
}

fn active_hint(cx: &mut EditorLspTestContext) -> Option<Entity<UsagesHint>> {
    let workspace = cx.workspace.clone();
    cx.update(|_, cx| workspace.read(cx).active_modal::<UsagesHint>(cx))
}

fn active_editor(cx: &mut EditorLspTestContext) -> Option<Entity<Editor>> {
    let workspace = cx.workspace.clone();
    cx.update(|_, cx| workspace.read(cx).active_item_as::<Editor>(cx))
}

fn is_focused(editor: &Entity<Editor>, cx: &mut EditorLspTestContext) -> bool {
    cx.update(|window, cx| editor.focus_handle(cx).is_focused(window))
}

fn popup_state(popup: &Entity<ShowUsagesPopup>, cx: &mut EditorLspTestContext) -> PopupState {
    cx.update(|_, cx| {
        let popup = popup.read(cx);
        let session = popup.session();
        PopupState {
            title: session.title(),
            kind: session.kind,
            scope: session.scope,
            searching: popup.is_searching(),
            status: popup.status_for_test(),
            display_rows: popup.display_rows_for_test(),
            rows: popup.usage_rows_for_test(),
            selected: popup.selected_for_test(),
            query: popup.query_for_test().to_string(),
            filters: popup.filters_for_test(),
            anchor: popup.anchor_position(),
        }
    })
}

fn hint_state(hint: &Entity<UsagesHint>, cx: &mut EditorLspTestContext) -> HintState {
    cx.update(|_, cx| {
        let hint = hint.read(cx);
        HintState {
            message: hint.message_for_test(),
            warning: hint.is_warning_for_test(),
            second_invocation: hint.second_invocation_for_test(),
            scope: hint.session().scope,
        }
    })
}

fn row_positions(rows: &[UsageRow<Location>]) -> Vec<(String, u32, u32)> {
    rows.iter()
        .map(|row| (row.file_path.to_string(), row.row, row.column))
        .collect()
}

fn position(file_path: &str, row: u32, column: u32) -> (String, u32, u32) {
    (file_path.to_string(), row, column)
}

fn usage_display_rows(count: usize) -> Vec<DisplayRow> {
    (0..count).map(DisplayRow::Usage).collect()
}

fn lens_click_position(cx: &mut EditorLspTestContext) -> Point<Pixels> {
    let declaration_position = cx.pixel_position(&LENS_SOURCE.replace("ˇfunction", "funcˇtion"));
    let line_height = cx.update_editor(|editor, window, cx| {
        editor
            .style(cx)
            .text
            .line_height_in_pixels(window.rem_size())
    });
    point(
        declaration_position.x,
        declaration_position.y - line_height * 0.75,
    )
}

#[gpui::test]
async fn test_show_usages_lists_sorted_merged_usages_below_the_caret(cx: &mut TestAppContext) {
    let (mut cx, main_uri, other_uri) = two_file_context(cx).await;
    let (release_references, references_released) = oneshot::channel::<()>();
    let mut references_released = Some(references_released);
    let usages = two_file_usages(&main_uri, &other_uri);
    cx.lsp
        .set_request_handler::<lsp::request::References, _, _>(move |_, _| {
            let released = references_released.take();
            let usages = usages.clone();
            async move {
                if let Some(released) = released {
                    released.await.unwrap();
                }
                Ok(Some(usages))
            }
        });
    cx.run_until_parked();

    let cursor = cx
        .editor(|editor, _, cx| editor.pixel_position_of_cursor(cx))
        .expect("the caret should be painted before Show Usages starts");
    let line_height = cx.update_editor(|editor, window, cx| {
        editor
            .style(cx)
            .text
            .line_height_in_pixels(window.rem_size())
    });

    cx.dispatch_action(ShowUsages::default());
    assert!(
        active_popup(&mut cx).is_none(),
        "the popup should wait for the search before the popup delay elapses"
    );

    cx.executor().advance_clock(POPUP_DELAY);
    cx.run_until_parked();
    let popup = active_popup(&mut cx).expect("a slow search should show the popup after the delay");
    let searching = popup_state(&popup, &mut cx);
    assert!(searching.searching, "the popup should still be searching");
    assert_eq!(searching.title, "Usages of 'helper'");
    assert_eq!(searching.status, None);
    assert_eq!(
        searching.anchor,
        Some(point(cursor.x, cursor.y + line_height / 2.)),
        "the popup should be anchored just below the caret line"
    );
    assert!(
        searching.anchor.is_some_and(|anchor| anchor.y > cursor.y),
        "the popup should open below the caret"
    );

    release_references.send(()).unwrap();
    cx.run_until_parked();

    assert_eq!(
        active_popup(&mut cx).map(|active| active.entity_id()),
        Some(popup.entity_id()),
        "the results should land in the popup that was already open"
    );
    let state = popup_state(&popup, &mut cx);
    assert!(!state.searching);
    assert_eq!(state.title, "Usages of 'helper'");
    assert_eq!(
        row_positions(&state.rows),
        vec![
            position("dir/file.rs", 5, 16),
            position("dir/file.rs", 6, 16),
            position("dir/other.rs", 1, 4),
            position("dir/other.rs", 5, 4),
        ],
        "rows should be sorted by file and line, without the declaration"
    );
    assert!(
        !state
            .rows
            .iter()
            .any(|row| row.file_path == "dir/file.rs" && row.row == 0),
        "the declaration should not be listed"
    );
    assert_eq!(
        state
            .rows
            .iter()
            .map(|row| row.locations.len())
            .collect::<Vec<_>>(),
        vec![2, 1, 1, 1],
        "both usages on the caret line should be merged into one row"
    );
    assert_eq!(
        state
            .rows
            .iter()
            .map(|row| row.is_origin)
            .collect::<Vec<_>>(),
        vec![true, false, false, false],
        "the row under the caret should be marked as the origin"
    );
    assert_eq!(state.display_rows, usage_display_rows(4));
    assert_eq!(
        state.selected,
        Some(1),
        "the origin row should not be selected initially"
    );
    assert_eq!(state.status.as_deref(), Some("4 usages"));
}

#[gpui::test]
async fn test_show_usages_navigates_to_the_only_usage(cx: &mut TestAppContext) {
    let mut cx = show_usages_context(references_and_definition_capabilities(), cx).await;
    cx.set_state(indoc! {"
        fn heˇlper() -> u32 {
            1
        }

        fn main() {
            let total = helper();
        }
    "});
    let uri = cx.buffer_lsp_url.clone();
    handle_definition(&cx, location(&uri, 0, 3, 9));
    handle_references(
        &cx,
        vec![location(&uri, 0, 3, 9), location(&uri, 5, 16, 22)],
    );

    cx.dispatch_action(ShowUsages::default());
    cx.run_until_parked();

    assert!(
        active_popup(&mut cx).is_none(),
        "a single usage should not open the popup"
    );
    assert_eq!(active_editor(&mut cx), Some(cx.editor.clone()));
    cx.assert_editor_state(indoc! {"
        fn helper() -> u32 {
            1
        }

        fn main() {
            let total = «helperˇ»();
        }
    "});
    let hint = active_hint(&mut cx).expect("navigating to the only usage should show a hint");
    let hint = hint_state(&hint, &mut cx);
    assert_eq!(hint.message, "It's the only usage in Project Files");
    assert!(!hint.warning);
    assert_eq!(hint.scope, UsageScope::ProjectFiles);
}

#[gpui::test]
async fn test_show_usages_without_usages_restarts_in_all_places(cx: &mut TestAppContext) {
    let mut cx = show_usages_context(references_capabilities(), cx).await;
    cx.update(|_, cx| {
        cx.bind_keys([KeyBinding::new(
            "cmd-alt-f7",
            ShowUsages::default(),
            Some("Editor"),
        )]);
    });
    cx.set_state(indoc! {"
        fn heˇlper() -> u32 {
            1
        }
    "});
    let reference_requests = handle_references(&cx, Vec::new());

    cx.dispatch_action(ShowUsages::default());
    cx.run_until_parked();

    assert!(active_popup(&mut cx).is_none());
    let hint = active_hint(&mut cx).expect("a search without usages should show a hint");
    let first = hint_state(&hint, &mut cx);
    assert_eq!(first.message, "No usages found in Project Files");
    assert!(!first.warning);
    assert_eq!(
        first.second_invocation.as_deref(),
        Some("Press ⌥⌘F7 again to search in 'All Places'")
    );
    assert_eq!(first.scope, UsageScope::ProjectFiles);

    cx.dispatch_action(ShowUsages::default());
    cx.run_until_parked();

    let restarted = active_hint(&mut cx).expect("the restarted search should show a hint");
    assert_ne!(restarted.entity_id(), hint.entity_id());
    let second = hint_state(&restarted, &mut cx);
    assert_eq!(second.message, "No usages found in All Places");
    assert_eq!(second.second_invocation, None);
    assert_eq!(second.scope, UsageScope::AllPlaces);
    assert_eq!(
        reference_requests.load(Ordering::SeqCst),
        2,
        "invoking Show Usages again should rerun the search"
    );
}

#[gpui::test]
async fn test_show_usages_confirm_navigates_and_cancel_restores_focus(cx: &mut TestAppContext) {
    let (mut cx, main_uri, other_uri) = two_file_context(cx).await;
    handle_references(&cx, two_file_usages(&main_uri, &other_uri));
    let origin_editor = cx.editor.clone();

    cx.dispatch_action(ShowUsages::default());
    cx.run_until_parked();
    let popup = active_popup(&mut cx).expect("several usages should open the popup");
    assert!(!is_focused(&origin_editor, &mut cx));

    cx.dispatch_action(menu::Cancel);
    cx.run_until_parked();
    assert!(
        active_popup(&mut cx).is_none(),
        "cancel should close the popup"
    );
    assert!(
        is_focused(&origin_editor, &mut cx),
        "cancel should return focus to the origin editor"
    );
    assert_eq!(popup_state(&popup, &mut cx).query, "");

    cx.dispatch_action(ShowUsages::default());
    cx.run_until_parked();
    let popup = active_popup(&mut cx).expect("Show Usages should open the popup again");
    assert_eq!(popup_state(&popup, &mut cx).selected, Some(1));
    cx.dispatch_action(menu::SelectNext);
    cx.run_until_parked();
    let state = popup_state(&popup, &mut cx);
    assert_eq!(state.selected, Some(2));
    assert_eq!(
        state
            .rows
            .get(2)
            .map(|row| (row.file_path.to_string(), row.row)),
        Some(("dir/other.rs".to_string(), 1))
    );

    cx.dispatch_action(menu::Confirm);
    cx.run_until_parked();

    assert!(
        active_popup(&mut cx).is_none(),
        "confirming a usage should close the popup"
    );
    let usage_editor = active_editor(&mut cx).expect("the usage should open in an editor");
    assert_ne!(usage_editor, origin_editor);
    assert!(is_focused(&usage_editor, &mut cx));
    let mut usage_cx = EditorTestContext::for_editor_in(usage_editor, &mut cx).await;
    usage_cx.assert_editor_state(indoc! {"
        fn first() -> u32 {
            «helperˇ»()
        }

        fn second() -> u32 {
            helper() + 1
        }
    "});
}

#[gpui::test]
async fn test_show_usages_write_access_filter_hides_write_usages(cx: &mut TestAppContext) {
    let mut cx = show_usages_context(
        lsp::ServerCapabilities {
            references_provider: Some(lsp::OneOf::Left(true)),
            document_highlight_provider: Some(lsp::OneOf::Left(true)),
            ..lsp::ServerCapabilities::default()
        },
        cx,
    )
    .await;
    cx.set_state(COUNTER_SOURCE);
    let uri = cx.buffer_lsp_url.clone();
    let usages = vec![
        location(&uri, 1, 12, 19),
        location(&uri, 2, 4, 11),
        location(&uri, 3, 16, 23),
        location(&uri, 4, 17, 24),
    ];
    let highlights = usages
        .iter()
        .zip([
            lsp::DocumentHighlightKind::WRITE,
            lsp::DocumentHighlightKind::WRITE,
            lsp::DocumentHighlightKind::READ,
            lsp::DocumentHighlightKind::READ,
        ])
        .map(|(usage, kind)| lsp::DocumentHighlight {
            range: usage.range,
            kind: Some(kind),
        })
        .collect::<Vec<_>>();
    handle_references(&cx, usages);
    cx.lsp
        .set_request_handler::<lsp::request::DocumentHighlightRequest, _, _>(move |_, _| {
            let highlights = highlights.clone();
            async move { Ok(Some(highlights)) }
        });

    cx.dispatch_action(ShowUsages::default());
    cx.run_until_parked();
    let popup = active_popup(&mut cx).expect("several usages should open the popup");
    let state = popup_state(&popup, &mut cx);
    assert_eq!(
        state.rows.iter().map(|row| row.access).collect::<Vec<_>>(),
        vec![
            UsageAccess::Write,
            UsageAccess::Write,
            UsageAccess::Read,
            UsageAccess::Read,
        ],
        "document highlights should classify every usage"
    );
    assert_eq!(state.display_rows, usage_display_rows(4));
    assert_eq!(state.status.as_deref(), Some("4 usages"));

    cx.dispatch_action(ToggleWriteAccess);
    cx.run_until_parked();
    let state = popup_state(&popup, &mut cx);
    assert!(!state.filters.show_write_access);
    assert_eq!(
        state.display_rows,
        vec![
            DisplayRow::Usage(2),
            DisplayRow::Usage(3),
            DisplayRow::FilteredOut(2),
        ],
        "hiding write access should leave the read usages and a filtered-out row"
    );
    assert_eq!(state.status.as_deref(), Some("2 of 4 usages"));

    cx.dispatch_action(menu::SelectLast);
    cx.run_until_parked();
    assert_eq!(popup_state(&popup, &mut cx).selected, Some(2));
    cx.dispatch_action(menu::Confirm);
    cx.run_until_parked();

    let restarted =
        active_popup(&mut cx).expect("confirming the filtered-out row should search again");
    assert_ne!(restarted.entity_id(), popup.entity_id());
    let state = popup_state(&restarted, &mut cx);
    assert_eq!(state.filters, UsageFilters::default());
    assert_eq!(state.display_rows, usage_display_rows(4));
    let saved_filters = cx.update(|_, cx| crate::saved_state(cx).filters);
    assert_eq!(saved_filters, UsageFilters::default());
}

#[gpui::test]
async fn test_show_usages_scopes_hide_and_reveal_usages(cx: &mut TestAppContext) {
    let mut cx = show_usages_context(references_capabilities(), cx).await;
    insert_tree(
        &mut cx,
        path!("/root/dir"),
        json!({ "other.rs": SCOPED_OTHER_SOURCE }),
    )
    .await;
    insert_tree(
        &mut cx,
        path!("/outside"),
        json!({ "lib.rs": OUTSIDE_SOURCE }),
    )
    .await;
    cx.set_state(SCOPED_MAIN_SOURCE);
    let main_uri = cx.buffer_lsp_url.clone();
    let other_uri = lsp::Uri::from_file_path(path!("/root/dir/other.rs")).unwrap();
    let outside_uri = lsp::Uri::from_file_path(path!("/outside/lib.rs")).unwrap();
    handle_references(
        &cx,
        vec![
            location(&outside_uri, 1, 4, 10),
            location(&main_uri, 5, 16, 22),
            location(&other_uri, 1, 4, 10),
            location(&main_uri, 6, 17, 23),
        ],
    );

    cx.dispatch_action(ShowUsages::default());
    cx.run_until_parked();
    let project_files =
        active_popup(&mut cx).expect("usages in project files should open the popup");
    let state = popup_state(&project_files, &mut cx);
    assert_eq!(state.scope, UsageScope::ProjectFiles);
    assert_eq!(
        state.display_rows,
        vec![
            DisplayRow::Usage(0),
            DisplayRow::Usage(1),
            DisplayRow::Usage(2),
            DisplayRow::OutOfScope(1),
        ],
        "the usage outside the worktree should be summarized as out of scope"
    );
    assert_eq!(
        row_positions(&state.rows),
        vec![
            position("dir/file.rs", 5, 16),
            position("dir/file.rs", 6, 17),
            position("dir/other.rs", 1, 4),
        ]
    );
    assert_eq!(state.status.as_deref(), Some("3+ usages"));

    cx.dispatch_action(menu::SelectLast);
    cx.run_until_parked();
    assert_eq!(popup_state(&project_files, &mut cx).selected, Some(3));
    cx.dispatch_action(menu::Confirm);
    cx.run_until_parked();

    let all_places = active_popup(&mut cx).expect("the out-of-scope row should search all places");
    assert_ne!(all_places.entity_id(), project_files.entity_id());
    let state = popup_state(&all_places, &mut cx);
    assert_eq!(state.scope, UsageScope::AllPlaces);
    assert_eq!(state.display_rows, usage_display_rows(4));
    let outside_row = state
        .rows
        .last()
        .expect("all places should list the outside usage");
    assert_eq!(outside_row.file_path, path!("/outside/lib.rs"));
    assert_eq!(outside_row.file_name, "lib.rs");
    assert_eq!(outside_row.row, 1);
    assert!(!outside_row.in_project);
    assert_eq!(state.status.as_deref(), Some("4 usages"));

    cx.update(|window, cx| {
        all_places.update(cx, |popup, cx| {
            popup.choose_scope_for_test(UsageScope::CurrentFile, window, cx)
        });
    });
    cx.run_until_parked();

    let current_file = active_popup(&mut cx).expect("choosing a scope should search again");
    assert_ne!(current_file.entity_id(), all_places.entity_id());
    let state = popup_state(&current_file, &mut cx);
    assert_eq!(state.scope, UsageScope::CurrentFile);
    assert_eq!(
        state.display_rows,
        vec![
            DisplayRow::Usage(0),
            DisplayRow::Usage(1),
            DisplayRow::OutOfScope(2),
        ]
    );
    assert_eq!(
        row_positions(&state.rows),
        vec![
            position("dir/file.rs", 5, 16),
            position("dir/file.rs", 6, 17),
        ],
        "the current file scope should only list usages from the current file"
    );
    let saved_scope = cx.update(|_, cx| crate::saved_state(cx).scope);
    assert_eq!(saved_scope, UsageScope::CurrentFile);
}

#[gpui::test]
async fn test_show_usages_code_lens_click_opens_lens_locations(cx: &mut TestAppContext) {
    init_show_usages(cx);
    let mut cx = EditorLspTestContext::new_typescript(
        lsp::ServerCapabilities {
            code_lens_provider: Some(lsp::CodeLensOptions {
                resolve_provider: None,
            }),
            references_provider: Some(lsp::OneOf::Left(true)),
            ..lsp::ServerCapabilities::default()
        },
        cx,
    )
    .await;
    let uri = cx.buffer_lsp_url.clone();
    let lens_locations = vec![location(&uri, 0, 9, 14), location(&uri, 10, 0, 5)];
    let lens_arguments = vec![
        serde_json::to_value(&uri).unwrap(),
        serde_json::to_value(lsp::Position::new(0, 9)).unwrap(),
        serde_json::to_value(&lens_locations).unwrap(),
    ];
    let reference_requests = handle_references(&cx, Vec::new());
    let mut code_lens_requests = cx
        .lsp
        .set_request_handler::<lsp::request::CodeLensRequest, _, _>(move |_, _| {
            let lens_arguments = lens_arguments.clone();
            async move {
                Ok(Some(vec![lsp::CodeLens {
                    range: lsp::Range::new(lsp::Position::new(0, 9), lsp::Position::new(0, 14)),
                    command: Some(lsp::Command {
                        title: "2 references".to_owned(),
                        command: "editor.action.showReferences".to_owned(),
                        arguments: Some(lens_arguments),
                    }),
                    data: None,
                }]))
            }
        });
    cx.update(|_, cx| {
        cx.update_global::<SettingsStore, _>(|store, cx| {
            store.update_user_settings(cx, |settings| {
                settings.editor.code_lens = Some(CodeLens::On);
            });
        });
    });
    cx.set_state(LENS_SOURCE);
    assert!(
        code_lens_requests.next().await.is_some(),
        "the editor should request code lenses"
    );
    cx.run_until_parked();

    let click_position = lens_click_position(&mut cx);
    cx.simulate_mouse_move(click_position, None, Modifiers::none());
    cx.simulate_click(click_position, Modifiers::none());
    cx.run_until_parked();

    let popup = active_popup(&mut cx).expect("clicking a references lens should open the popup");
    let state = popup_state(&popup, &mut cx);
    assert_eq!(state.title, "Usages of 'hello'");
    assert_eq!(state.kind, UsagesKind::References);
    assert_eq!(
        state.anchor,
        Some(click_position),
        "the popup should open where the lens was clicked"
    );
    assert_eq!(
        row_positions(&state.rows),
        vec![
            position("dir/file.ts", 0, 9),
            position("dir/file.ts", 10, 0)
        ],
        "the popup should list exactly the lens locations"
    );
    let lens_target_count = cx.update(|_, cx| match &popup.read(cx).session().target {
        UsagesTarget::Locations { locations } => Some(locations.len()),
        UsagesTarget::Symbol { .. } => None,
    });
    assert_eq!(lens_target_count, Some(2));
    assert_eq!(
        reference_requests.load(Ordering::SeqCst),
        0,
        "the lens locations should be used without a references request"
    );
    let editor_count =
        cx.update_workspace(|workspace, _, cx| workspace.items_of_type::<Editor>(cx).count());
    assert_eq!(editor_count, 1);
}

#[gpui::test]
async fn test_show_usages_cmd_click_on_declaration_opens_popup(cx: &mut TestAppContext) {
    let mut cx = show_usages_context(references_and_definition_capabilities(), cx).await;
    cx.set_state(indoc! {"
        fn helperˇ() {}
        fn main() { helper(); }
        fn other() { helper(); }
    "});
    let uri = cx.buffer_lsp_url.clone();
    handle_definition(&cx, location(&uri, 0, 3, 9));
    let reference_requests = handle_references(
        &cx,
        vec![
            location(&uri, 0, 3, 9),
            location(&uri, 1, 12, 18),
            location(&uri, 2, 13, 19),
        ],
    );
    cx.run_until_parked();

    let click_position = cx.pixel_position(indoc! {"
        fn helˇper() {}
        fn main() { helper(); }
        fn other() { helper(); }
    "});
    cx.simulate_mouse_move(click_position, None, Modifiers::secondary_key());
    cx.run_until_parked();
    cx.simulate_click(click_position, Modifiers::secondary_key());
    cx.run_until_parked();

    let popup = active_popup(&mut cx)
        .expect("cmd-click on a declaration should open the Show Usages popup");
    let state = popup_state(&popup, &mut cx);
    assert_eq!(state.title, "Usages of 'helper'");
    assert_eq!(
        state.anchor,
        Some(click_position),
        "the popup should open at the mouse position"
    );
    assert_eq!(
        row_positions(&state.rows),
        vec![
            position("dir/file.rs", 1, 12),
            position("dir/file.rs", 2, 13)
        ]
    );
    assert_eq!(reference_requests.load(Ordering::SeqCst), 1);
    let editor_count =
        cx.update_workspace(|workspace, _, cx| workspace.items_of_type::<Editor>(cx).count());
    assert_eq!(
        editor_count, 1,
        "no references multibuffer should open when Show Usages handles the click"
    );
}

#[gpui::test]
async fn test_show_usages_open_in_find_window_opens_references_tab(cx: &mut TestAppContext) {
    let (mut cx, main_uri, other_uri) = two_file_context(cx).await;
    handle_references(&cx, two_file_usages(&main_uri, &other_uri));

    cx.dispatch_action(ShowUsages::default());
    cx.run_until_parked();
    assert!(
        active_popup(&mut cx).is_some(),
        "several usages should open the popup"
    );

    cx.dispatch_action(OpenInFindWindow);
    cx.run_until_parked();

    assert!(
        active_popup(&mut cx).is_none(),
        "opening the find window should close the popup"
    );
    let references_editor =
        active_editor(&mut cx).expect("the usages should open in a references tab");
    assert_ne!(references_editor, cx.editor.clone());
    let (title, text, buffer_count) = cx.update(|_, cx| {
        let editor = references_editor.read(cx);
        let multi_buffer = editor.buffer().read(cx);
        (
            multi_buffer.title(cx).into_owned(),
            editor.text(cx),
            multi_buffer.all_buffers().len(),
        )
    });
    assert_eq!(title, "References to helper");
    assert_eq!(buffer_count, 2);
    for usage_line in [
        "let total = helper() + helper();",
        "let other = helper();",
        "helper() + 1",
    ] {
        assert!(
            text.contains(usage_line),
            "the references tab should contain {usage_line:?}, got {text:?}"
        );
    }
}

#[gpui::test]
async fn test_show_usages_more_usages_row_shows_the_rest(cx: &mut TestAppContext) {
    let mut cx = show_usages_context(references_capabilities(), cx).await;
    let usage_lines = 2..152_u32;
    let usage_count = usage_lines.len();
    let mut source = String::from("fn heˇlper() {}\nfn main() {\n");
    for _ in usage_lines.clone() {
        source.push_str("    helper();\n");
    }
    source.push_str("}\n");
    cx.set_state(&source);
    let uri = cx.buffer_lsp_url.clone();
    handle_references(
        &cx,
        usage_lines.map(|row| location(&uri, row, 4, 10)).collect(),
    );

    cx.dispatch_action(ShowUsages::default());
    cx.run_until_parked();
    let popup = active_popup(&mut cx).expect("many usages should open the popup");
    let state = popup_state(&popup, &mut cx);
    assert_eq!(state.rows.len(), USAGES_PAGE_SIZE);
    assert_eq!(state.display_rows.len(), USAGES_PAGE_SIZE + 1);
    assert_eq!(state.display_rows.last(), Some(&DisplayRow::MoreUsages));
    let page_status = format!("{USAGES_PAGE_SIZE}+ usages");
    assert_eq!(state.status.as_deref(), Some(page_status.as_str()));

    cx.dispatch_action(menu::SelectLast);
    cx.run_until_parked();
    assert_eq!(
        popup_state(&popup, &mut cx).selected,
        Some(USAGES_PAGE_SIZE)
    );
    cx.dispatch_action(menu::Confirm);
    cx.run_until_parked();

    assert_eq!(
        active_popup(&mut cx).map(|active| active.entity_id()),
        Some(popup.entity_id()),
        "showing more usages should keep the popup open"
    );
    let state = popup_state(&popup, &mut cx);
    assert_eq!(state.rows.len(), usage_count);
    assert_eq!(state.display_rows, usage_display_rows(usage_count));
    let full_status = format!("{usage_count} usages");
    assert_eq!(state.status.as_deref(), Some(full_status.as_str()));
}

#[gpui::test]
async fn test_show_usages_speed_search_selects_match_and_escape_clears_query(
    cx: &mut TestAppContext,
) {
    let mut cx = show_usages_context(references_capabilities(), cx).await;
    cx.update(|_, cx| {
        cx.bind_keys([KeyBinding::new("escape", menu::Cancel, None)]);
    });
    cx.set_state(SPEED_SEARCH_SOURCE);
    let uri = cx.buffer_lsp_url.clone();
    handle_references(
        &cx,
        vec![
            location(&uri, 5, 16, 22),
            location(&uri, 6, 15, 21),
            location(&uri, 7, 16, 22),
            location(&uri, 8, 30, 36),
        ],
    );
    let origin_editor = cx.editor.clone();

    cx.dispatch_action(ShowUsages::default());
    cx.run_until_parked();
    let popup = active_popup(&mut cx).expect("several usages should open the popup");
    assert_eq!(popup_state(&popup, &mut cx).selected, Some(0));

    cx.simulate_keystrokes("g a m m a");
    let state = popup_state(&popup, &mut cx);
    assert_eq!(state.query, "gamma");
    assert_eq!(
        state.selected,
        Some(2),
        "typing should select the first row matching the query"
    );

    cx.simulate_keystrokes("escape");
    assert_eq!(
        active_popup(&mut cx).map(|active| active.entity_id()),
        Some(popup.entity_id()),
        "the first escape should only clear the query"
    );
    let state = popup_state(&popup, &mut cx);
    assert_eq!(state.query, "");
    assert_eq!(state.selected, Some(2));

    cx.simulate_keystrokes("escape");
    assert!(
        active_popup(&mut cx).is_none(),
        "the second escape should close the popup"
    );
    assert!(is_focused(&origin_editor, &mut cx));
}
