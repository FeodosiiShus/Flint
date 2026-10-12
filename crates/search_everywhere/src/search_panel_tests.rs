use editor::Editor;
use gpui::{
    Action as _, AppContext as _, Entity, Focusable, TestAppContext, UpdateGlobal as _,
    VisualContext, VisualTestContext, px,
};
use picker::{Picker, TogglePreview};
use project::Project;
use serde_json::json;
use settings::SettingsStore;
use ui::IconName;
use util::path;
use workspace::{
    DeploySearch, Workspace,
    dock::{DockPosition, Panel},
};
use zed_actions::{
    search::NewSearchInDirectory, search_everywhere::Tab, search_panel::ToggleFocus,
};

use crate::{
    SearchPanel,
    delegate::{SearchEverywhereDelegate, activate_tab},
    search_everywhere_tests::{
        build_workspace, entries, init_test, selected_entry, settle, type_query,
    },
};

const EXPECTED_BOTTOM_DOCK_PRIORITY: u32 = 5;

fn add_search_panel(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Entity<SearchPanel> {
    workspace.update_in(cx, |workspace, window, cx| {
        let panel = cx.new(|cx| SearchPanel::new(workspace, window, cx));
        workspace.add_panel(panel.clone(), window, cx);
        panel
    })
}

fn panel_picker(
    panel: &Entity<SearchPanel>,
    cx: &mut VisualTestContext,
) -> Entity<Picker<SearchEverywhereDelegate>> {
    panel.read_with(cx, |panel, cx| panel.search.read(cx).picker.clone())
}

fn assert_panel_visible(
    workspace: &Entity<Workspace>,
    panel: &Entity<SearchPanel>,
    cx: &mut VisualTestContext,
) {
    workspace.read_with(cx, |workspace, cx| {
        let dock = workspace.bottom_dock().read(cx);
        assert!(dock.is_open(), "the bottom dock is open");
        assert_eq!(
            dock.visible_panel().map(|visible| visible.panel_id()),
            Some(panel.entity_id()),
            "the search panel is the visible panel of the bottom dock"
        );
    });
}

fn dock_search_panel(position: DockPosition, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        SettingsStore::update_global(cx, |store, cx| {
            store.update_user_settings(cx, |settings| {
                settings.search_panel.get_or_insert_default().dock = Some(position.into());
            });
        });
    });
    cx.run_until_parked();
}

fn add_focused_editor(
    text: &str,
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Entity<Editor> {
    let editor = cx.new_window_entity(|window, cx| {
        let mut editor = Editor::single_line(window, cx);
        editor.set_text(text, window, cx);
        editor
    });
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.add_item_to_active_pane(Box::new(editor.clone()), None, true, window, cx);
        editor.update(cx, |editor, cx| window.focus(&editor.focus_handle(cx), cx));
    });
    editor
}

#[gpui::test]
async fn test_panel_lives_in_bottom_dock_with_search_tool_window_button(cx: &mut TestAppContext) {
    let app_state = init_test(cx);
    let project = Project::test(app_state.fs.clone(), [], cx).await;
    let (workspace, cx) = build_workspace(project, cx);
    let panel = add_search_panel(&workspace, cx);

    workspace.read_with(cx, |workspace, cx| {
        assert!(
            workspace
                .bottom_dock()
                .read(cx)
                .panel::<SearchPanel>()
                .is_some(),
            "the search panel is added to the bottom dock"
        );
        assert!(
            workspace
                .left_dock()
                .read(cx)
                .panel::<SearchPanel>()
                .is_none()
        );
        assert!(
            workspace
                .right_dock()
                .read(cx)
                .panel::<SearchPanel>()
                .is_none()
        );
    });
    cx.update(|window, cx| {
        let panel = panel.read(cx);
        assert_eq!(panel.position(window, cx), DockPosition::Bottom);
        assert_eq!(panel.icon(window, cx), Some(IconName::MagnifyingGlass));
        assert_eq!(panel.icon_tooltip(window, cx), Some("Search"));
        assert_eq!(panel.activation_priority(), EXPECTED_BOTTOM_DOCK_PRIORITY);
        assert_eq!(
            panel.toggle_action().name(),
            ToggleFocus.name(),
            "the tool window button toggles the search panel"
        );
        assert_eq!(SearchPanel::persistent_name(), "SearchPanel");
    });
}

#[gpui::test]
async fn test_panel_default_size_is_the_height_at_the_bottom_and_the_width_at_the_sides(
    cx: &mut TestAppContext,
) {
    let app_state = init_test(cx);
    let project = Project::test(app_state.fs.clone(), [], cx).await;
    let (workspace, cx) = build_workspace(project, cx);
    let panel = add_search_panel(&workspace, cx);

    let default_size = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| panel.read(cx).default_size(window, cx))
    };
    assert_eq!(
        default_size(cx),
        px(320.),
        "the bottom dock uses search_panel.default_height"
    );
    for position in [DockPosition::Left, DockPosition::Right] {
        dock_search_panel(position, cx);
        assert_eq!(
            default_size(cx),
            px(360.),
            "the {position:?} dock uses search_panel.default_width"
        );
    }
}

#[gpui::test]
async fn test_panel_preview_sits_right_of_the_results_in_the_bottom_dock_and_below_them_at_the_sides(
    cx: &mut TestAppContext,
) {
    let app_state = init_test(cx);
    app_state
        .fs
        .as_fake()
        .insert_tree(path!("/dir"), json!({ "qzalpha.txt": "alpha" }))
        .await;
    let project = Project::test(app_state.fs.clone(), [path!("/dir").as_ref()], cx).await;
    let (workspace, cx) = build_workspace(project, cx);
    let panel = add_search_panel(&workspace, cx);

    cx.dispatch_action(ToggleFocus);
    cx.run_until_parked();
    assert_panel_visible(&workspace, &panel, cx);
    assert_eq!(
        cx.debug_bounds("picker_panel_preview"),
        None,
        "the panel keeps the preview hidden until it is toggled"
    );

    cx.dispatch_action(TogglePreview);
    cx.run_until_parked();
    let results = cx
        .debug_bounds("picker_panel_results")
        .expect("the panel shows its results next to the preview");
    let preview = cx
        .debug_bounds("picker_panel_preview")
        .expect("toggling the preview shows it");
    assert_eq!(
        preview.left(),
        results.right(),
        "in the bottom dock the preview sits to the right of the results, like IntelliJ's Find tool window"
    );
    assert_eq!(
        (preview.top(), preview.bottom()),
        (results.top(), results.bottom()),
        "the preview spans the height of the results"
    );
    assert!(
        (preview.size.width - results.size.width).abs() <= px(1.),
        "the results and the preview split the width in half, IntelliJ's default preview splitter proportion: results {results:?}, preview {preview:?}"
    );

    dock_search_panel(DockPosition::Left, cx);
    let results = cx
        .debug_bounds("picker_panel_results")
        .expect("the panel moved to the left dock keeps its results");
    let preview = cx
        .debug_bounds("picker_panel_preview")
        .expect("the panel moved to the left dock keeps the preview open");
    assert_eq!(
        preview.top(),
        results.bottom(),
        "in a side dock the preview sits below the results"
    );
    assert_eq!(
        (preview.left(), preview.right()),
        (results.left(), results.right()),
        "the preview spans the width of the results"
    );
}

#[gpui::test]
async fn test_confirming_a_result_in_the_panel_keeps_the_panel_and_its_results(
    cx: &mut TestAppContext,
) {
    let app_state = init_test(cx);
    app_state
        .fs
        .as_fake()
        .insert_tree(
            path!("/dir"),
            json!({ "qzalpha.txt": "alpha", "other.txt": "other" }),
        )
        .await;
    let project = Project::test(app_state.fs.clone(), [path!("/dir").as_ref()], cx).await;
    let (workspace, cx) = build_workspace(project, cx);
    let panel = add_search_panel(&workspace, cx);

    cx.dispatch_action(ToggleFocus);
    cx.run_until_parked();
    assert_panel_visible(&workspace, &panel, cx);

    let picker = panel_picker(&panel, cx);
    type_query(cx, "qzalpha");
    assert_eq!(entries(&picker, cx), vec!["# Files", "qzalpha.txt"]);
    assert_eq!(selected_entry(&picker, cx), "qzalpha.txt");

    cx.dispatch_action(menu::Confirm);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();

    let editor = cx.update(|_, cx| workspace.read(cx).active_item_as::<Editor>(cx));
    let editor = editor.expect("confirming a file opens it in an editor");
    editor.update(cx, |editor, cx| assert_eq!(editor.title(cx), "qzalpha.txt"));
    assert_panel_visible(&workspace, &panel, cx);
    picker.read_with(cx, |picker, cx| {
        assert_eq!(picker.query(cx), "qzalpha", "the query is kept");
    });
    assert_eq!(
        entries(&picker, cx),
        vec!["# Files", "qzalpha.txt"],
        "the results are kept"
    );
}

#[gpui::test]
async fn test_escape_in_the_panel_returns_focus_to_the_editor(cx: &mut TestAppContext) {
    let app_state = init_test(cx);
    app_state
        .fs
        .as_fake()
        .insert_tree(path!("/dir"), json!({ "qzalpha.txt": "alpha" }))
        .await;
    let project = Project::test(app_state.fs.clone(), [path!("/dir").as_ref()], cx).await;
    let (workspace, cx) = build_workspace(project, cx);
    let editor = add_focused_editor("abc", &workspace, cx);
    let panel = add_search_panel(&workspace, cx);

    cx.dispatch_action(ToggleFocus);
    cx.run_until_parked();
    let picker = panel_picker(&panel, cx);
    type_query(cx, "qzalpha");
    cx.update(|window, cx| {
        assert!(panel.read(cx).focus_handle(cx).contains_focused(window, cx));
        assert!(!editor.focus_handle(cx).is_focused(window));
    });

    cx.dispatch_action(menu::Cancel);
    cx.run_until_parked();

    cx.update(|window, cx| {
        assert!(!panel.read(cx).focus_handle(cx).contains_focused(window, cx));
        assert!(editor.focus_handle(cx).is_focused(window));
    });
    assert_panel_visible(&workspace, &panel, cx);
    assert_eq!(
        entries(&picker, cx),
        vec!["# Files", "qzalpha.txt"],
        "dismissing keeps the results"
    );
}

#[gpui::test]
async fn test_actions_in_the_panel_run_on_the_previously_focused_editor(cx: &mut TestAppContext) {
    let app_state = init_test(cx);
    let project = Project::test(app_state.fs.clone(), [], cx).await;
    let (workspace, cx) = build_workspace(project, cx);
    let panel = add_search_panel(&workspace, cx);
    let editor = add_focused_editor("abc", &workspace, cx);

    cx.dispatch_action(ToggleFocus);
    cx.run_until_parked();
    let picker = panel_picker(&panel, cx);
    picker.update_in(cx, |picker, window, cx| {
        activate_tab(picker, Tab::Actions, window, cx);
    });
    type_query(cx, "bcksp");
    assert_eq!(selected_entry(&picker, cx), "editor: backspace");

    cx.dispatch_action(menu::Confirm);
    cx.run_until_parked();

    editor.update(cx, |editor, cx| assert_eq!(editor.text(cx), "ab"));
    assert_panel_visible(&workspace, &panel, cx);
}

#[gpui::test]
async fn test_deploy_search_opens_the_panel_on_the_text_tab_seeded_from_the_editor(
    cx: &mut TestAppContext,
) {
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
    add_focused_editor("qzneedle", &workspace, cx);
    let panel = add_search_panel(&workspace, cx);

    cx.dispatch_action(DeploySearch::default());
    settle(cx);

    assert_panel_visible(&workspace, &panel, cx);
    let picker = panel_picker(&panel, cx);
    picker.read_with(cx, |picker, cx| {
        assert_eq!(picker.delegate.tab(), Tab::Text);
        assert_eq!(
            picker.query(cx),
            "qzneedle",
            "the query is seeded from the editor"
        );
    });
    assert_eq!(
        entries(&picker, cx),
        vec!["notes.md:2", "open in text finder"]
    );
    cx.update(|window, cx| {
        assert!(
            panel_picker_focus(&panel, cx).is_focused(window),
            "the query editor of the panel is focused"
        );
    });
}

#[gpui::test]
async fn test_deploy_search_query_takes_precedence_over_the_editor_seed(cx: &mut TestAppContext) {
    let app_state = init_test(cx);
    let project = Project::test(app_state.fs.clone(), [], cx).await;
    let (workspace, cx) = build_workspace(project, cx);
    add_focused_editor("qzneedle", &workspace, cx);
    let panel = add_search_panel(&workspace, cx);

    cx.dispatch_action(DeploySearch {
        query: Some("explicit".into()),
        ..Default::default()
    });
    settle(cx);

    let picker = panel_picker(&panel, cx);
    picker.read_with(cx, |picker, cx| {
        assert_eq!(picker.delegate.tab(), Tab::Text);
        assert_eq!(picker.query(cx), "explicit");
    });
}

#[gpui::test]
async fn test_search_in_directory_scopes_the_text_tab_to_that_directory(cx: &mut TestAppContext) {
    let app_state = init_test(cx);
    app_state
        .fs
        .as_fake()
        .insert_tree(
            path!("/dir"),
            json!({
                "inside": { "hit.txt": "qzneedle" },
                "outside": { "hit.txt": "qzneedle" },
            }),
        )
        .await;
    let project = Project::test(app_state.fs.clone(), [path!("/dir").as_ref()], cx).await;
    let (workspace, cx) = build_workspace(project, cx);
    let panel = add_search_panel(&workspace, cx);

    cx.dispatch_action(NewSearchInDirectory {
        directory: "inside".into(),
    });
    cx.run_until_parked();
    assert_panel_visible(&workspace, &panel, cx);
    let picker = panel_picker(&panel, cx);
    picker.read_with(cx, |picker, _| {
        assert_eq!(picker.delegate.tab(), Tab::Text);
        assert!(picker.delegate.text_scope.is_some());
    });

    type_query(cx, "qzneedle");
    assert_eq!(
        entries(&picker, cx),
        vec!["inside/hit.txt:1", "open in text finder"],
        "only files inside the directory are searched"
    );

    picker.update_in(cx, |picker, window, cx| {
        picker.delegate.set_text_scope(None);
        picker.refresh(window, cx);
    });
    settle(cx);
    assert_eq!(
        entries(&picker, cx).len(),
        3,
        "clearing the scope searches the whole project"
    );
}

fn panel_picker_focus(panel: &Entity<SearchPanel>, cx: &gpui::App) -> gpui::FocusHandle {
    panel.read(cx).search.focus_handle(cx)
}
