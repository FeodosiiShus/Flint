use std::time::Duration;

use gpui::{Action as _, AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext};
use language::DiagnosticSourceKind;
use lsp::LanguageServerId;
use project::{FakeFs, Project};
use serde_json::json;
use settings::SettingsStore;
use ui::IconName;
use util::path;
use workspace::{
    MultiWorkspace, Workspace,
    dock::{DockPosition, Panel},
};

use crate::{
    DIAGNOSTICS_UPDATE_DEBOUNCE, Deploy, IncludeWarnings, ProjectDiagnosticsPanel,
    ToggleDiagnosticsRefresh, ToggleWarnings,
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
        let panel = cx.new(|cx| ProjectDiagnosticsPanel::new(workspace, cx));
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
    panel.read_with(cx, |panel, _| panel.view.is_some())
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
