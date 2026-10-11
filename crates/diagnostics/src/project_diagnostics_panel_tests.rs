use std::time::Duration;

use gpui::{Action as _, AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext};
use language::DiagnosticSourceKind;
use lsp::LanguageServerId;
use project::{FakeFs, Project, ProjectPath};
use serde_json::json;
use settings::SettingsStore;
use ui::IconName;
use util::{path, rel_path::rel_path};
use workspace::{
    MultiWorkspace, SplitDirection, Workspace,
    dock::{DockPosition, Panel},
};
use zed_actions::diagnostics::ToggleFileProblems;

use crate::{
    DIAGNOSTICS_UPDATE_DEBOUNCE, Deploy, IncludeWarnings, ProjectDiagnosticsPanel,
    ToggleDiagnosticsRefresh, ToggleWarnings,
    project_diagnostics_panel::{ActiveFile, ProblemsTab},
};

const EXPECTED_LEFT_DOCK_PRIORITY: u32 = 5;
const SETTLE_DURATION: Duration = Duration::from_millis(100);

fn init_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let settings = SettingsStore::test(cx);
        cx.set_global(settings);
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        crate::init(cx);
        editor::init(cx);
    });
}

async fn build_project(cx: &mut TestAppContext) -> Entity<Project> {
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree(
        path!("/test"),
        json!({
            "main.rs": "fn main() {}\nfn other() {}\nfn third() {}\n",
            "other.rs": "fn helper() {}\n",
        }),
    )
    .await;
    Project::test(fs, [path!("/test").as_ref()], cx).await
}

fn build_workspace(
    project: Entity<Project>,
    cx: &mut TestAppContext,
) -> (Entity<Workspace>, &mut VisualTestContext) {
    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project, window, cx));
    let workspace =
        multi_workspace.read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone());
    (workspace, cx)
}

fn add_project_diagnostics_panel(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Entity<ProjectDiagnosticsPanel> {
    workspace.update_in(cx, |workspace, window, cx| {
        let panel = cx.new(|cx| ProjectDiagnosticsPanel::new(workspace, window, cx));
        workspace.add_panel(panel.clone(), window, cx);
        panel
    })
}

fn focus_workspace(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    workspace.update_in(cx, |workspace, window, cx| {
        window.focus(&workspace.focus_handle(cx), cx);
    });
}

fn publish_errors(project: &Entity<Project>, error_count: u32, cx: &mut VisualTestContext) {
    let lsp_store = project.read_with(cx, |project, _| project.lsp_store());
    let diagnostics: Vec<lsp::Diagnostic> = (0..error_count)
        .map(|row| lsp::Diagnostic {
            range: lsp::Range::new(lsp::Position::new(row, 0), lsp::Position::new(row, 2)),
            severity: Some(lsp::DiagnosticSeverity::ERROR),
            message: lsp::DiagnosticMessage::from(format!("error on row {row}")),
            ..Default::default()
        })
        .collect();
    lsp_store.update(cx, |lsp_store, cx| {
        lsp_store
            .update_diagnostics(
                LanguageServerId(0),
                lsp::PublishDiagnosticsParams {
                    uri: lsp::Uri::from_file_path(path!("/test/main.rs")).unwrap(),
                    diagnostics,
                    version: None,
                },
                None,
                DiagnosticSourceKind::Pushed,
                &[],
                cx,
            )
            .unwrap();
    });
    settle(cx);
}

fn settle(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(SETTLE_DURATION);
    cx.run_until_parked();
}

fn header_action_ids(
    panel: &Entity<ProjectDiagnosticsPanel>,
    cx: &mut VisualTestContext,
) -> Vec<&'static str> {
    cx.update(|window, cx| {
        panel
            .read(cx)
            .header_actions(window, cx)
            .into_iter()
            .map(|action| action.id)
            .collect()
    })
}

fn panel_contains_focus(
    panel: &Entity<ProjectDiagnosticsPanel>,
    cx: &mut VisualTestContext,
) -> bool {
    cx.update(|window, cx| panel.read(cx).focus_handle(cx).contains_focused(window, cx))
}

fn has_hosted_view(panel: &Entity<ProjectDiagnosticsPanel>, cx: &mut VisualTestContext) -> bool {
    panel.read_with(cx, |panel, _| panel.project_view.is_some())
}

fn project_path(
    project: &Entity<Project>,
    relative_path: &str,
    cx: &mut VisualTestContext,
) -> ProjectPath {
    ProjectPath {
        worktree_id: project.read_with(cx, |project, cx| {
            project.worktrees(cx).next().unwrap().read(cx).id()
        }),
        path: rel_path(relative_path).into(),
    }
}

async fn open_file(
    workspace: &Entity<Workspace>,
    project_path: ProjectPath,
    cx: &mut VisualTestContext,
) {
    workspace
        .update_in(cx, |workspace, window, cx| {
            workspace.open_path(project_path, None, true, window, cx)
        })
        .await
        .unwrap();
    cx.run_until_parked();
}

fn toggle_file_problems(cx: &mut VisualTestContext) {
    cx.dispatch_action(ToggleFileProblems);
    cx.run_until_parked();
}

fn shown_file(
    panel: &Entity<ProjectDiagnosticsPanel>,
    cx: &mut VisualTestContext,
) -> Option<ProjectPath> {
    panel.read_with(cx, |panel, _| panel.file_path().cloned())
}

fn selected_tab(
    panel: &Entity<ProjectDiagnosticsPanel>,
    cx: &mut VisualTestContext,
) -> ProblemsTab {
    panel.read_with(cx, |panel, _| panel.selected_tab)
}

fn panel_is_shown(
    workspace: &Entity<Workspace>,
    panel: &Entity<ProjectDiagnosticsPanel>,
    cx: &mut VisualTestContext,
) -> bool {
    workspace.read_with(cx, |workspace, cx| {
        let dock = workspace.left_dock().read(cx);
        dock.is_open()
            && dock.visible_panel().map(|visible| visible.panel_id()) == Some(panel.entity_id())
    })
}

fn file_tab_empty_title(
    panel: &Entity<ProjectDiagnosticsPanel>,
    cx: &mut VisualTestContext,
) -> Option<String> {
    cx.update(|_, cx| {
        panel
            .read(cx)
            .file_tab_empty_state(cx)
            .map(|empty_state| empty_state.title.to_string())
    })
}

fn problem_count(
    panel: &Entity<ProjectDiagnosticsPanel>,
    tab: ProblemsTab,
    cx: &mut VisualTestContext,
) -> usize {
    cx.update(|_, cx| panel.read(cx).problem_count(tab, cx))
}

#[gpui::test]
async fn test_project_diagnostics_panel_lives_in_left_dock_with_tool_window_button(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = build_project(cx).await;
    let (workspace, cx) = build_workspace(project, cx);
    let panel = add_project_diagnostics_panel(&workspace, cx);

    workspace.read_with(cx, |workspace, cx| {
        assert!(
            workspace
                .left_dock()
                .read(cx)
                .panel::<ProjectDiagnosticsPanel>()
                .is_some(),
            "the project diagnostics panel is added to the left dock"
        );
        assert!(
            workspace
                .right_dock()
                .read(cx)
                .panel::<ProjectDiagnosticsPanel>()
                .is_none()
        );
        assert!(
            workspace
                .bottom_dock()
                .read(cx)
                .panel::<ProjectDiagnosticsPanel>()
                .is_none()
        );
    });
    cx.update(|window, cx| {
        let panel = panel.read(cx);
        assert_eq!(panel.position(window, cx), DockPosition::Left);
        assert_eq!(panel.icon(window, cx), Some(IconName::Warning));
        assert_eq!(
            panel.icon_tooltip(window, cx),
            Some("Project Diagnostics Panel")
        );
        assert_eq!(panel.activation_priority(), EXPECTED_LEFT_DOCK_PRIORITY);
        assert_eq!(
            panel.toggle_action().name(),
            Deploy.name(),
            "the tool window button toggles the panel through the Deploy action"
        );
        assert_eq!(panel.icon_label(window, cx), None);
        for position in [
            DockPosition::Left,
            DockPosition::Right,
            DockPosition::Bottom,
        ] {
            assert!(panel.position_is_valid(position));
        }
        assert_eq!(
            ProjectDiagnosticsPanel::persistent_name(),
            "ProjectDiagnosticsPanel"
        );
        assert_eq!(
            ProjectDiagnosticsPanel::panel_key(),
            "ProjectDiagnosticsPanel"
        );
    });
}

#[gpui::test]
async fn test_project_diagnostics_panel_creates_hosted_view_on_first_activation(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = build_project(cx).await;
    let (workspace, cx) = build_workspace(project.clone(), cx);
    let panel = add_project_diagnostics_panel(&workspace, cx);
    publish_errors(&project, 2, cx);

    assert!(
        !has_hosted_view(&panel, cx),
        "adding the panel and publishing diagnostics must not open any diagnostics buffers"
    );

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.toggle_panel_focus::<ProjectDiagnosticsPanel>(window, cx);
    });
    cx.run_until_parked();

    assert!(
        has_hosted_view(&panel, cx),
        "the hosted view exists once the panel is activated"
    );
    workspace.read_with(cx, |workspace, cx| {
        let dock = workspace.left_dock().read(cx);
        assert!(dock.is_open());
        assert_eq!(
            dock.visible_panel().map(|visible| visible.panel_id()),
            Some(panel.entity_id())
        );
    });
    assert!(
        panel_contains_focus(&panel, cx),
        "focus moves into the hosted view"
    );
}

#[gpui::test]
async fn test_project_diagnostics_panel_icon_label_reflects_error_count(cx: &mut TestAppContext) {
    init_test(cx);
    let project = build_project(cx).await;
    let (workspace, cx) = build_workspace(project.clone(), cx);
    let panel = add_project_diagnostics_panel(&workspace, cx);

    let icon_label =
        |cx: &mut VisualTestContext| cx.update(|window, cx| panel.read(cx).icon_label(window, cx));

    assert_eq!(icon_label(cx), None);

    publish_errors(&project, 2, cx);
    assert_eq!(icon_label(cx), Some("2".to_string()));

    publish_errors(&project, 3, cx);
    assert_eq!(icon_label(cx), Some("3".to_string()));

    publish_errors(&project, 0, cx);
    assert_eq!(icon_label(cx), None);
}

#[gpui::test]
async fn test_project_diagnostics_panel_deploy_action_toggles_panel_focus(cx: &mut TestAppContext) {
    init_test(cx);
    let project = build_project(cx).await;
    let (workspace, cx) = build_workspace(project, cx);
    let panel = add_project_diagnostics_panel(&workspace, cx);
    focus_workspace(&workspace, cx);

    cx.dispatch_action(Deploy);
    cx.run_until_parked();

    workspace.read_with(cx, |workspace, cx| {
        let dock = workspace.left_dock().read(cx);
        assert!(dock.is_open(), "Deploy opens the left dock");
        assert_eq!(
            dock.visible_panel().map(|visible| visible.panel_id()),
            Some(panel.entity_id())
        );
    });
    assert!(panel_contains_focus(&panel, cx), "Deploy focuses the panel");

    cx.dispatch_action(Deploy);
    cx.run_until_parked();

    assert!(
        !panel_contains_focus(&panel, cx),
        "a second Deploy hands focus back to the center"
    );
}

#[gpui::test]
async fn test_project_diagnostics_panel_header_actions_follow_hosted_view(cx: &mut TestAppContext) {
    init_test(cx);
    let project = build_project(cx).await;
    let (workspace, cx) = build_workspace(project, cx);
    let panel = add_project_diagnostics_panel(&workspace, cx);

    assert_eq!(header_action_ids(&panel, cx), ["toggle-warnings"]);

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.toggle_panel_focus::<ProjectDiagnosticsPanel>(window, cx);
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(DIAGNOSTICS_UPDATE_DEBOUNCE + SETTLE_DURATION);
    cx.run_until_parked();

    assert_eq!(
        header_action_ids(&panel, cx),
        ["refresh-diagnostics", "toggle-warnings"]
    );

    focus_workspace(&workspace, cx);
    cx.dispatch_action(ToggleDiagnosticsRefresh);
    assert_eq!(
        header_action_ids(&panel, cx),
        ["stop-diagnostics-update", "toggle-warnings"],
        "refreshing shows the stop button"
    );

    cx.dispatch_action(ToggleDiagnosticsRefresh);
    assert_eq!(
        header_action_ids(&panel, cx),
        ["refresh-diagnostics", "toggle-warnings"],
        "stopping shows the refresh button again"
    );
}

#[gpui::test]
async fn test_project_diagnostics_panel_toggle_warnings_action_flips_include_warnings(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = build_project(cx).await;
    let (workspace, cx) = build_workspace(project, cx);
    let _panel = add_project_diagnostics_panel(&workspace, cx);
    focus_workspace(&workspace, cx);

    let include_warnings =
        |cx: &mut VisualTestContext| cx.update(|_, cx| IncludeWarnings::current(cx));
    let initial = include_warnings(cx);

    cx.dispatch_action(ToggleWarnings);
    assert_eq!(include_warnings(cx), !initial);

    cx.dispatch_action(ToggleWarnings);
    assert_eq!(include_warnings(cx), initial);
}

#[gpui::test]
async fn test_project_diagnostics_panel_toggle_file_problems_opens_file_tab_for_active_file(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = build_project(cx).await;
    let (workspace, cx) = build_workspace(project.clone(), cx);
    let panel = add_project_diagnostics_panel(&workspace, cx);
    let main_path = project_path(&project, "main.rs", cx);
    open_file(&workspace, main_path.clone(), cx).await;

    assert_eq!(
        selected_tab(&panel, cx),
        ProblemsTab::Project,
        "the panel starts on the Project tab"
    );

    toggle_file_problems(cx);

    assert!(
        panel_is_shown(&workspace, &panel, cx),
        "ToggleFileProblems opens the panel"
    );
    assert_eq!(selected_tab(&panel, cx), ProblemsTab::File);
    assert_eq!(
        shown_file(&panel, cx),
        Some(main_path),
        "the File tab shows the active editor's file"
    );
    assert!(
        panel_contains_focus(&panel, cx),
        "ToggleFileProblems focuses the panel"
    );
    assert_eq!(
        file_tab_empty_title(&panel, cx).as_deref(),
        Some("No problems in main.rs")
    );

    publish_errors(&project, 2, cx);

    assert_eq!(
        file_tab_empty_title(&panel, cx),
        None,
        "a file with problems shows its problems instead of the empty state"
    );
    assert_eq!(problem_count(&panel, ProblemsTab::File, cx), 2);
}

#[gpui::test]
async fn test_project_diagnostics_panel_toggle_file_problems_without_editor_shows_empty_file_tab(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = build_project(cx).await;
    let (workspace, cx) = build_workspace(project, cx);
    let panel = add_project_diagnostics_panel(&workspace, cx);
    focus_workspace(&workspace, cx);

    toggle_file_problems(cx);

    assert!(panel_is_shown(&workspace, &panel, cx));
    assert_eq!(selected_tab(&panel, cx), ProblemsTab::File);
    assert_eq!(shown_file(&panel, cx), None);
    assert_eq!(
        file_tab_empty_title(&panel, cx).as_deref(),
        Some("Open file in editor to see problems")
    );
}

#[gpui::test]
async fn test_project_diagnostics_panel_toggle_file_problems_again_for_same_file_hides_panel(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = build_project(cx).await;
    let (workspace, cx) = build_workspace(project.clone(), cx);
    let panel = add_project_diagnostics_panel(&workspace, cx);
    let main_path = project_path(&project, "main.rs", cx);
    open_file(&workspace, main_path, cx).await;

    toggle_file_problems(cx);
    assert!(panel_is_shown(&workspace, &panel, cx));

    toggle_file_problems(cx);

    assert!(
        !panel_is_shown(&workspace, &panel, cx),
        "toggling the file that the File tab already shows hides the panel"
    );
    assert!(
        !panel_contains_focus(&panel, cx),
        "hiding the panel returns focus to the editor"
    );
    assert_eq!(
        selected_tab(&panel, cx),
        ProblemsTab::File,
        "the File tab stays selected after hiding"
    );
}

#[gpui::test]
async fn test_project_diagnostics_panel_toggle_file_problems_switches_to_other_active_file(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = build_project(cx).await;
    let (workspace, cx) = build_workspace(project.clone(), cx);
    let panel = add_project_diagnostics_panel(&workspace, cx);
    let main_path = project_path(&project, "main.rs", cx);
    let other_path = project_path(&project, "other.rs", cx);
    open_file(&workspace, main_path.clone(), cx).await;
    toggle_file_problems(cx);

    let other_buffer = project
        .update(cx, |project, cx| {
            project.open_buffer(other_path.clone(), cx)
        })
        .await
        .unwrap();
    panel.update_in(cx, |panel, window, cx| {
        panel.show_file(
            Some(ActiveFile {
                project_path: other_path.clone(),
                buffer: other_buffer,
            }),
            window,
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(shown_file(&panel, cx), Some(other_path));

    toggle_file_problems(cx);

    assert!(
        panel_is_shown(&workspace, &panel, cx),
        "switching to another file keeps the panel open"
    );
    assert_eq!(
        shown_file(&panel, cx),
        Some(main_path),
        "the File tab switches to the active editor's file"
    );
    assert!(panel_contains_focus(&panel, cx));
}

#[gpui::test]
async fn test_project_diagnostics_panel_toggle_file_problems_uses_the_focused_editor_of_an_inactive_pane(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = build_project(cx).await;
    let (workspace, cx) = build_workspace(project.clone(), cx);
    let panel = add_project_diagnostics_panel(&workspace, cx);
    let main_path = project_path(&project, "main.rs", cx);
    let other_path = project_path(&project, "other.rs", cx);
    open_file(&workspace, main_path.clone(), cx).await;
    let main_editor = workspace.read_with(cx, |workspace, cx| {
        workspace
            .active_item_as::<editor::Editor>(cx)
            .expect("main.rs opens in an editor")
    });
    let right_pane = workspace.update_in(cx, |workspace, window, cx| {
        let left_pane = workspace.active_pane().clone();
        workspace.split_pane(left_pane, SplitDirection::Right, window, cx)
    });
    workspace
        .update_in(cx, |workspace, window, cx| {
            workspace.open_path(other_path, Some(right_pane.downgrade()), true, window, cx)
        })
        .await
        .unwrap();
    cx.run_until_parked();

    cx.update(|window, cx| {
        window.focus(&main_editor.focus_handle(cx), cx);
        window.dispatch_action(Box::new(ToggleFileProblems), cx);
    });
    cx.run_until_parked();

    assert_eq!(
        shown_file(&panel, cx),
        Some(main_path),
        "the inspection widget focuses its editor before dispatching, so the File tab must show the clicked editor's file even though its pane was not active yet"
    );
}

#[gpui::test]
async fn test_project_diagnostics_panel_file_tab_follows_active_editor(cx: &mut TestAppContext) {
    init_test(cx);
    let project = build_project(cx).await;
    let (workspace, cx) = build_workspace(project.clone(), cx);
    let panel = add_project_diagnostics_panel(&workspace, cx);
    let main_path = project_path(&project, "main.rs", cx);
    let other_path = project_path(&project, "other.rs", cx);
    open_file(&workspace, main_path.clone(), cx).await;
    publish_errors(&project, 2, cx);
    toggle_file_problems(cx);
    assert_eq!(shown_file(&panel, cx), Some(main_path.clone()));
    assert_eq!(problem_count(&panel, ProblemsTab::File, cx), 2);

    open_file(&workspace, other_path.clone(), cx).await;

    assert!(panel_is_shown(&workspace, &panel, cx));
    assert_eq!(
        shown_file(&panel, cx),
        Some(other_path),
        "the File tab follows the newly active editor"
    );
    assert_eq!(problem_count(&panel, ProblemsTab::File, cx), 0);
    assert_eq!(
        problem_count(&panel, ProblemsTab::Project, cx),
        2,
        "the Project tab still counts every file's problems"
    );
    assert_eq!(
        file_tab_empty_title(&panel, cx).as_deref(),
        Some("No problems in other.rs")
    );

    open_file(&workspace, main_path.clone(), cx).await;

    assert_eq!(shown_file(&panel, cx), Some(main_path));
}

#[gpui::test]
async fn test_project_diagnostics_panel_deploy_opens_selected_tab(cx: &mut TestAppContext) {
    init_test(cx);
    let project = build_project(cx).await;
    let (workspace, cx) = build_workspace(project.clone(), cx);
    let panel = add_project_diagnostics_panel(&workspace, cx);
    let main_path = project_path(&project, "main.rs", cx);
    open_file(&workspace, main_path, cx).await;

    cx.dispatch_action(Deploy);
    cx.run_until_parked();

    assert!(panel_is_shown(&workspace, &panel, cx));
    assert_eq!(
        selected_tab(&panel, cx),
        ProblemsTab::Project,
        "Deploy opens the Project tab by default"
    );
    assert!(has_hosted_view(&panel, cx));
    assert!(panel_contains_focus(&panel, cx));

    toggle_file_problems(cx);
    assert_eq!(selected_tab(&panel, cx), ProblemsTab::File);
    toggle_file_problems(cx);
    assert!(!panel_is_shown(&workspace, &panel, cx));

    cx.dispatch_action(Deploy);
    cx.run_until_parked();

    assert!(panel_is_shown(&workspace, &panel, cx));
    assert_eq!(
        selected_tab(&panel, cx),
        ProblemsTab::File,
        "Deploy reopens the panel on the last selected tab"
    );
    assert!(panel_contains_focus(&panel, cx));
}
