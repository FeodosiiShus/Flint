use std::sync::Arc;

use anyhow::Result;
use fs::Fs;
use gpui::{
    Action, AsyncWindowContext, Entity, EventEmitter, FocusHandle, Focusable, Subscription, Task,
    WeakEntity,
};
use project::Project;
use settings::{IntoGpui, RegisterSetting, Settings};
use ui::prelude::*;
use util::ResultExt;
use workspace::{
    HideStatusItem, Workspace,
    dock::{DockPosition, Panel, PanelEvent, PanelHeaderAction},
    item::Item,
};

use crate::{
    DIAGNOSTICS_SUMMARY_UPDATE_DEBOUNCE, Deploy, IncludeWarnings, ProjectDiagnosticsEditor,
    ToggleDiagnosticsRefresh, ToggleWarnings,
};

const PROJECT_DIAGNOSTICS_PANEL_KEY: &str = "ProjectDiagnosticsPanel";
const PROJECT_DIAGNOSTICS_PANEL_ACTIVATION_PRIORITY: u32 = 5;
const DEFAULT_BOTTOM_DOCK_HEIGHT: Pixels = px(320.);

#[derive(Debug, Clone, PartialEq, RegisterSetting)]
pub struct ProjectDiagnosticsPanelSettings {
    pub button: bool,
    pub dock: DockPosition,
    pub default_width: Pixels,
}

impl Settings for ProjectDiagnosticsPanelSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let project_diagnostics_panel = content.project_diagnostics_panel.clone().unwrap();
        Self {
            button: project_diagnostics_panel.button.unwrap(),
            dock: project_diagnostics_panel.dock.unwrap().into(),
            default_width: project_diagnostics_panel.default_width.unwrap().into_gpui(),
        }
    }
}

pub(crate) fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _window, _cx| {
        workspace.register_action(|workspace, _: &Deploy, window, cx| {
            workspace.toggle_panel_focus::<ProjectDiagnosticsPanel>(window, cx);
        });
        workspace.register_action(toggle_warnings);
        workspace.register_action(toggle_diagnostics_refresh);
    })
    .detach();
}

fn toggle_warnings(
    workspace: &mut Workspace,
    _: &ToggleWarnings,
    _window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    if workspace.panel::<ProjectDiagnosticsPanel>(cx).is_none() {
        cx.propagate();
        return;
    }
    let include_warnings = IncludeWarnings::current(cx);
    cx.set_global(IncludeWarnings(!include_warnings));
}

fn toggle_diagnostics_refresh(
    workspace: &mut Workspace,
    _: &ToggleDiagnosticsRefresh,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(panel) = workspace.panel::<ProjectDiagnosticsPanel>(cx) else {
        cx.propagate();
        return;
    };
    panel.update(cx, |panel, cx| panel.toggle_refresh(window, cx));
}

pub struct ProjectDiagnosticsPanel {
    focus_handle: FocusHandle,
    workspace: WeakEntity<Workspace>,
    project: Entity<Project>,
    fs: Arc<dyn Fs>,
    pub(crate) view: Option<Entity<ProjectDiagnosticsEditor>>,
    error_count: usize,
    error_count_update: Task<()>,
    _view_observation: Option<Subscription>,
    _project_subscription: Subscription,
}

impl ProjectDiagnosticsPanel {
    pub async fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        workspace.update_in(&mut cx, |workspace, _window, cx| {
            cx.new(|cx| Self::new(workspace, cx))
        })
    }

    pub(crate) fn new(workspace: &Workspace, cx: &mut Context<Self>) -> Self {
        let project = workspace.project().clone();
        let project_subscription =
            cx.subscribe(&project, |this, _project, event, cx| match event {
                project::Event::DiskBasedDiagnosticsFinished { .. }
                | project::Event::LanguageServerRemoved(_) => {
                    this.refresh_error_count(cx);
                }
                project::Event::DiagnosticsUpdated { .. } => {
                    this.error_count_update = cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(DIAGNOSTICS_SUMMARY_UPDATE_DEBOUNCE)
                            .await;
                        this.update(cx, |this, cx| {
                            this.refresh_error_count(cx);
                        })
                        .log_err();
                    });
                }
                _ => {}
            });
        let error_count = project.read(cx).diagnostic_summary(false, cx).error_count;
        Self {
            focus_handle: cx.focus_handle(),
            workspace: workspace.weak_handle(),
            project,
            fs: workspace.app_state().fs.clone(),
            view: None,
            error_count,
            error_count_update: Task::ready(()),
            _view_observation: None,
            _project_subscription: project_subscription,
        }
    }

    fn refresh_error_count(&mut self, cx: &mut Context<Self>) {
        let error_count = self
            .project
            .read(cx)
            .diagnostic_summary(false, cx)
            .error_count;
        if error_count != self.error_count {
            self.error_count = error_count;
            cx.notify();
        }
    }

    fn ensure_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.view.is_some() {
            return;
        }
        let project = self.project.clone();
        let workspace = self.workspace.clone();
        let include_warnings = IncludeWarnings::current(cx);
        let view = cx.new(|cx| {
            ProjectDiagnosticsEditor::new(include_warnings, project, workspace, window, cx)
        });
        self._view_observation = Some(cx.observe(&view, |_, _, cx| cx.notify()));
        if self.focus_handle.is_focused(window) {
            view.focus_handle(cx).focus(window, cx);
        }
        self.view = Some(view.clone());
        cx.notify();

        let workspace = self.workspace.clone();
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |workspace, cx| {
                    view.update(cx, |view, cx| {
                        view.added_to_workspace(workspace, window, cx);
                    });
                })
                .log_err();
        });
    }

    fn toggle_refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| {
                view.toggle_diagnostics_refresh(&ToggleDiagnosticsRefresh, window, cx);
            });
        }
    }
}

impl EventEmitter<PanelEvent> for ProjectDiagnosticsPanel {}

impl Focusable for ProjectDiagnosticsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ProjectDiagnosticsPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let content = match &self.view {
            Some(view) => view.clone().into_any_element(),
            None => v_flex()
                .size_full()
                .p_2()
                .child(Label::new("Checking diagnostics…").color(Color::Muted))
                .into_any_element(),
        };
        div()
            .key_context("ProjectDiagnosticsPanel")
            .track_focus(&self.focus_handle)
            .size_full()
            .child(content)
    }
}

impl Panel for ProjectDiagnosticsPanel {
    fn persistent_name() -> &'static str {
        PROJECT_DIAGNOSTICS_PANEL_KEY
    }

    fn panel_key() -> &'static str {
        PROJECT_DIAGNOSTICS_PANEL_KEY
    }

    fn activation_focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.view {
            Some(view) => view.focus_handle(cx),
            None => self.focus_handle.clone(),
        }
    }

    fn position(&self, _: &Window, cx: &App) -> DockPosition {
        ProjectDiagnosticsPanelSettings::get_global(cx).dock
    }

    fn position_is_valid(&self, _: DockPosition) -> bool {
        true
    }

    fn set_position(&mut self, position: DockPosition, _: &mut Window, cx: &mut Context<Self>) {
        settings::update_settings_file(self.fs.clone(), cx, move |settings, _| {
            settings
                .project_diagnostics_panel
                .get_or_insert_default()
                .dock = Some(position.into())
        });
    }

    fn default_size(&self, window: &Window, cx: &App) -> Pixels {
        match self.position(window, cx) {
            DockPosition::Left | DockPosition::Right => {
                ProjectDiagnosticsPanelSettings::get_global(cx).default_width
            }
            DockPosition::Bottom => DEFAULT_BOTTOM_DOCK_HEIGHT,
        }
    }

    fn icon(&self, _: &Window, cx: &App) -> Option<IconName> {
        ProjectDiagnosticsPanelSettings::get_global(cx)
            .button
            .then_some(IconName::Warning)
    }

    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<&'static str> {
        Some("Project Diagnostics Panel")
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(Deploy)
    }

    fn icon_label(&self, _: &Window, _: &App) -> Option<String> {
        (self.error_count > 0).then(|| self.error_count.to_string())
    }

    fn activation_priority(&self) -> u32 {
        PROJECT_DIAGNOSTICS_PANEL_ACTIVATION_PRIORITY
    }

    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if active {
            cx.defer_in(window, |this, window, cx| {
                this.ensure_view(window, cx);
            });
        }
    }

    fn hide_button_setting(&self, _: &App) -> Option<HideStatusItem> {
        Some(HideStatusItem::new(|settings| {
            settings
                .project_diagnostics_panel
                .get_or_insert_default()
                .button = Some(false);
        }))
    }

    fn header_actions(&self, _: &Window, cx: &App) -> Vec<PanelHeaderAction> {
        let mut actions = Vec::new();
        if let Some(view) = &self.view {
            let is_updating = view.read(cx).update_excerpts_task.is_some();
            actions.push(if is_updating {
                PanelHeaderAction::new(
                    "stop-diagnostics-update",
                    IconName::Stop,
                    "Stop Diagnostics Update",
                    Box::new(ToggleDiagnosticsRefresh),
                )
            } else {
                PanelHeaderAction::new(
                    "refresh-diagnostics",
                    IconName::ArrowCircle,
                    "Refresh Diagnostics",
                    Box::new(ToggleDiagnosticsRefresh),
                )
            });
        }
        actions.push(PanelHeaderAction::new(
            "toggle-warnings",
            IconName::Warning,
            "Toggle Warnings",
            Box::new(ToggleWarnings),
        ));
        actions
    }
}
