use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    sync::Arc,
};

use gpui::{
    Action as _, AppContext as _, Entity, Focusable as _, SharedString, TestAppContext,
    VisualTestContext,
};
use language::{BinaryStatus, ServerHealth};
use lsp::{LanguageServerId, LanguageServerName};
use project::{FakeFs, Project};
use serde_json::json;
use ui::{Color, IconName};
use util::path;
use workspace::{
    AppState, MultiWorkspace, Workspace,
    dock::{DockPosition, Panel},
};

use crate::{
    language_services_panel::{LanguageServicesPanel, PanelRow, ServerRow},
    lsp_button::{
        LanguageServerBinaryStatus, LanguageServerHealthStatus, ServersForPath, ToggleFocus,
    },
};

const EXPECTED_LEFT_DOCK_PRIORITY: u32 = 6;

struct ServerFixture {
    id: usize,
    name: &'static str,
    health: ServerHealth,
    message: Option<&'static str>,
}

fn healthy(id: usize, name: &'static str) -> ServerFixture {
    ServerFixture {
        id,
        name,
        health: ServerHealth::Ok,
        message: None,
    }
}

fn warning(id: usize, name: &'static str, message: &'static str) -> ServerFixture {
    ServerFixture {
        id,
        name,
        health: ServerHealth::Warning,
        message: Some(message),
    }
}

fn failing(id: usize, name: &'static str, message: &'static str) -> ServerFixture {
    ServerFixture {
        id,
        name,
        health: ServerHealth::Error,
        message: Some(message),
    }
}

fn init_test(cx: &mut TestAppContext) -> Arc<AppState> {
    cx.update(|cx| {
        let app_state = AppState::test(cx);
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        release_channel::init(semver::Version::new(0, 0, 0), cx);
        editor::init(cx);
        crate::language_services_panel::init(cx);
        app_state
    })
}

async fn build_panel(
    cx: &mut TestAppContext,
) -> (
    Entity<Workspace>,
    Entity<LanguageServicesPanel>,
    &mut VisualTestContext,
) {
    init_test(cx);
    let fs = FakeFs::new(cx.background_executor.clone());
    fs.insert_tree(path!("/project"), json!({ "main.rs": "" }))
        .await;
    let project = Project::test(fs, [path!("/project").as_ref()], cx).await;
    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project, window, cx));
    let workspace =
        multi_workspace.read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone());
    let panel = workspace.update_in(cx, |workspace, window, cx| {
        let panel = cx.new(|cx| LanguageServicesPanel::new(workspace, window, cx));
        workspace.add_panel(panel.clone(), window, cx);
        panel
    });
    (workspace, panel, cx)
}

fn seed_servers(
    workspace: &Entity<Workspace>,
    panel: &Entity<LanguageServicesPanel>,
    servers: &[ServerFixture],
    cx: &mut VisualTestContext,
) {
    let worktree = workspace.read_with(cx, |workspace, cx| {
        workspace
            .project()
            .read(cx)
            .worktrees(cx)
            .next()
            .expect("the test project has a worktree")
            .downgrade()
    });
    let buffer_path = PathBuf::from(path!("/project/main.rs"));
    panel.update(cx, |panel, cx| {
        panel.server_state.update(cx, |state, cx| {
            for server in servers {
                let id = LanguageServerId(server.id);
                let name = LanguageServerName(server.name.into());
                state.language_servers.health_statuses.insert(
                    id,
                    LanguageServerHealthStatus {
                        name: name.clone(),
                        health: Some((server.message.map(SharedString::from), server.health)),
                    },
                );
                state
                    .language_servers
                    .servers_per_buffer_abs_path
                    .entry(buffer_path.clone())
                    .or_insert_with(|| ServersForPath {
                        servers: HashMap::default(),
                        worktree: Some(worktree.clone()),
                    })
                    .servers
                    .insert(id, Some(name));
            }
            state.regenerate_items(cx);
        });
    });
}

fn set_binary_status(
    panel: &Entity<LanguageServicesPanel>,
    name: &'static str,
    status: BinaryStatus,
    cx: &mut VisualTestContext,
) {
    panel.update(cx, |panel, cx| {
        panel.server_state.update(cx, |state, cx| {
            state.language_servers.binary_statuses.insert(
                LanguageServerName(name.into()),
                LanguageServerBinaryStatus {
                    status,
                    message: None,
                },
            );
            state.regenerate_items(cx);
        });
    });
}

fn rows_of(panel: &Entity<LanguageServicesPanel>, cx: &mut VisualTestContext) -> Vec<PanelRow> {
    panel.read_with(cx, |panel, cx| panel.rows(cx))
}

fn server_rows(rows: &[PanelRow]) -> Vec<&ServerRow> {
    rows.iter()
        .filter_map(|row| match row {
            PanelRow::Server(server_row) => Some(server_row),
            _ => None,
        })
        .collect()
}

fn server_row<'a>(rows: &'a [PanelRow], name: &str) -> &'a ServerRow {
    server_rows(rows)
        .into_iter()
        .find(|row| row.info.name.to_string() == name)
        .unwrap_or_else(|| panic!("no row for server {name}"))
}

fn icon_label(panel: &Entity<LanguageServicesPanel>, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|window, cx| panel.read(cx).icon_label(window, cx))
}

#[gpui::test]
async fn test_language_services_panel_lives_in_left_dock_with_bolt_button(cx: &mut TestAppContext) {
    let (workspace, panel, cx) = build_panel(cx).await;

    workspace.read_with(cx, |workspace, cx| {
        assert!(
            workspace
                .left_dock()
                .read(cx)
                .panel::<LanguageServicesPanel>()
                .is_some(),
            "the language services panel is added to the left dock"
        );
        assert!(
            workspace
                .right_dock()
                .read(cx)
                .panel::<LanguageServicesPanel>()
                .is_none()
        );
        assert!(
            workspace
                .bottom_dock()
                .read(cx)
                .panel::<LanguageServicesPanel>()
                .is_none()
        );
    });
    cx.update(|window, cx| {
        let panel = panel.read(cx);
        assert_eq!(panel.position(window, cx), DockPosition::Left);
        assert_eq!(panel.icon(window, cx), Some(IconName::BoltOutlined));
        assert_eq!(
            panel.icon_tooltip(window, cx),
            Some("Language Services Panel")
        );
        assert_eq!(panel.activation_priority(), EXPECTED_LEFT_DOCK_PRIORITY);
        assert_eq!(
            panel.toggle_action().name(),
            ToggleFocus.name(),
            "the tool window button toggles the language services panel"
        );
        for position in [
            DockPosition::Left,
            DockPosition::Right,
            DockPosition::Bottom,
        ] {
            assert!(panel.position_is_valid(position));
        }
        assert_eq!(
            LanguageServicesPanel::persistent_name(),
            "LanguageServicesPanel"
        );
        assert_eq!(LanguageServicesPanel::panel_key(), "LanguageServicesPanel");
    });
}

#[gpui::test]
async fn test_language_services_panel_without_servers_has_no_rows_and_no_badge(
    cx: &mut TestAppContext,
) {
    let (_workspace, panel, cx) = build_panel(cx).await;

    assert!(rows_of(&panel, cx).is_empty());
    assert_eq!(icon_label(&panel, cx), None);
}

#[gpui::test]
async fn test_language_services_panel_rows_group_servers_under_worktree_header(
    cx: &mut TestAppContext,
) {
    let (workspace, panel, cx) = build_panel(cx).await;
    seed_servers(
        &workspace,
        &panel,
        &[
            healthy(1, "rust-analyzer"),
            warning(2, "typos-lsp", "slow indexing"),
            failing(3, "eslint", "crashed"),
        ],
        cx,
    );

    let rows = rows_of(&panel, cx);

    assert!(
        matches!(rows.first(), Some(PanelRow::Header(name)) if name.to_string() == "project"),
        "the first row is the worktree header, got {rows:?}"
    );
    assert_eq!(
        server_rows(&rows)
            .into_iter()
            .map(|row| row.info.name.to_string())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "eslint".to_string(),
            "rust-analyzer".to_string(),
            "typos-lsp".to_string()
        ]),
    );
    let [
        ..,
        PanelRow::ToggleAllServers { restart: true },
        PanelRow::ToggleAllServers { restart: false },
    ] = rows.as_slice()
    else {
        panic!("the group ends with Restart All and Stop All rows, got {rows:?}");
    };
    assert_eq!(rows.len(), 1 + 3 + 2);
}

#[gpui::test]
async fn test_language_services_panel_server_rows_describe_health_and_message(
    cx: &mut TestAppContext,
) {
    let (workspace, panel, cx) = build_panel(cx).await;
    seed_servers(
        &workspace,
        &panel,
        &[
            healthy(1, "rust-analyzer"),
            warning(2, "typos-lsp", "slow indexing"),
            failing(3, "eslint", "crashed"),
        ],
        cx,
    );

    let rows = rows_of(&panel, cx);

    let running = server_row(&rows, "rust-analyzer");
    assert_eq!(running.status_label, "Running");
    assert_eq!(running.status_color, Color::Success);
    assert_eq!(running.message, None);
    assert!(running.info.can_stop());

    let warned = server_row(&rows, "typos-lsp");
    assert_eq!(warned.status_label, "Warning");
    assert_eq!(warned.status_color, Color::Warning);
    assert_eq!(warned.message.as_deref(), Some("slow indexing"));

    let failed = server_row(&rows, "eslint");
    assert_eq!(failed.status_label, "Error");
    assert_eq!(failed.status_color, Color::Error);
    assert_eq!(failed.message.as_deref(), Some("crashed"));
}

#[gpui::test]
async fn test_language_services_panel_binary_status_overrides_health_in_server_rows(
    cx: &mut TestAppContext,
) {
    let (workspace, panel, cx) = build_panel(cx).await;
    seed_servers(
        &workspace,
        &panel,
        &[healthy(1, "rust-analyzer"), healthy(2, "typos-lsp")],
        cx,
    );
    set_binary_status(&panel, "rust-analyzer", BinaryStatus::Starting, cx);
    set_binary_status(&panel, "typos-lsp", BinaryStatus::Stopped, cx);

    let rows = rows_of(&panel, cx);

    let starting = server_row(&rows, "rust-analyzer");
    assert_eq!(starting.status_label, "Starting…");
    assert_eq!(starting.status_color, Color::Modified);
    assert!(starting.info.can_stop());

    let stopped = server_row(&rows, "typos-lsp");
    assert_eq!(stopped.status_label, "Stopped");
    assert_eq!(stopped.status_color, Color::Disabled);
    assert!(
        !stopped.info.can_stop(),
        "a stopped server offers no Stop button"
    );
}

#[gpui::test]
async fn test_language_services_panel_icon_label_counts_only_error_and_warning_servers(
    cx: &mut TestAppContext,
) {
    let (workspace, panel, cx) = build_panel(cx).await;
    seed_servers(
        &workspace,
        &panel,
        &[healthy(1, "rust-analyzer"), healthy(2, "typos-lsp")],
        cx,
    );
    assert_eq!(
        icon_label(&panel, cx),
        None,
        "healthy servers show no badge"
    );

    seed_servers(
        &workspace,
        &panel,
        &[
            warning(2, "typos-lsp", "slow indexing"),
            failing(3, "eslint", "crashed"),
        ],
        cx,
    );
    assert_eq!(icon_label(&panel, cx), Some("2".to_string()));
}

#[gpui::test]
async fn test_language_services_panel_toggle_focus_action_toggles_the_panel(
    cx: &mut TestAppContext,
) {
    let (workspace, panel, cx) = build_panel(cx).await;

    cx.dispatch_action(ToggleFocus);
    cx.run_until_parked();

    workspace.read_with(cx, |workspace, cx| {
        let dock = workspace.left_dock().read(cx);
        assert!(dock.is_open(), "the left dock opens");
        assert_eq!(
            dock.visible_panel().map(|visible| visible.panel_id()),
            Some(panel.entity_id()),
            "the language services panel is the visible panel of the left dock"
        );
    });
    cx.update(|window, cx| {
        assert!(panel.focus_handle(cx).contains_focused(window, cx));
    });

    cx.dispatch_action(ToggleFocus);
    cx.run_until_parked();

    cx.update(|window, cx| {
        assert!(
            !panel.focus_handle(cx).contains_focused(window, cx),
            "the second toggle returns focus to the center"
        );
    });
}
