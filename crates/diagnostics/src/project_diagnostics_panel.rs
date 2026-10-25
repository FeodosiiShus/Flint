use std::sync::Arc;

use anyhow::Result;
use editor::Editor;
use fs::Fs;
use gpui::{
    Action, AnyView, AsyncWindowContext, Entity, EntityId, EventEmitter, FocusHandle, Focusable,
    Subscription, Task, WeakEntity,
};
use language::Buffer;
use project::{DiagnosticSummary, Project, ProjectPath};
use settings::{IntoGpui, RegisterSetting, Settings};
use ui::prelude::*;
use util::ResultExt;
use workspace::{
    HideButtonSetting, Workspace, WorkspaceSettings,
    dock::{DockPosition, Panel, PanelEvent, PanelHeaderAction},
    item::{Item, ItemHandle},
};
use zed_actions::diagnostics::ToggleFileProblems;

use crate::{
    DIAGNOSTICS_SUMMARY_UPDATE_DEBOUNCE, Deploy, IncludeWarnings, ProjectDiagnosticsEditor,
    ToggleDiagnosticsRefresh, ToggleWarnings, buffer_diagnostics::BufferDiagnosticsEditor,
};

const PROJECT_DIAGNOSTICS_PANEL_KEY: &str = "ProjectDiagnosticsPanel";
const PROJECT_DIAGNOSTICS_PANEL_ACTIVATION_PRIORITY: u32 = 3;
const TAB_PROBLEM_COUNT_GAP: Pixels = px(8.);
const TAB_HORIZONTAL_PADDING: Pixels = px(11.);
const TAB_UNDERLINE_HEIGHT: Pixels = px(3.);
const NO_SELECTED_FILE_TEXT: &str = "Open file in editor to see problems";
const PROBLEMS_FILTERED_OUT_TEXT: &str = "Problems could be filtered out";

#[derive(Debug, Clone, PartialEq, RegisterSetting)]
pub struct ProjectDiagnosticsPanelSettings {
    pub button: bool,
    pub dock: DockPosition,
    pub default_width: Pixels,
    pub default_height: Pixels,
}

impl Settings for ProjectDiagnosticsPanelSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let project_diagnostics_panel = content.project_diagnostics_panel.clone().unwrap();
        Self {
            button: project_diagnostics_panel.button.unwrap(),
            dock: project_diagnostics_panel.dock.unwrap().into(),
            default_width: project_diagnostics_panel.default_width.unwrap().into_gpui(),
            default_height: project_diagnostics_panel
                .default_height
                .unwrap()
                .into_gpui(),
        }
    }
}

pub(crate) fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _window, _cx| {
        workspace.register_action(|workspace, _: &Deploy, window, cx| {
            workspace.toggle_panel_focus::<ProjectDiagnosticsPanel>(window, cx);
        });
        workspace.register_action(toggle_file_problems);
        workspace.register_action(toggle_warnings);
        workspace.register_action(toggle_diagnostics_refresh);
    })
    .detach();
}

fn toggle_file_problems(
    workspace: &mut Workspace,
    _: &ToggleFileProblems,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(panel) = workspace.panel::<ProjectDiagnosticsPanel>(cx) else {
        cx.propagate();
        return;
    };
    let active_file = match focused_editor_item(workspace, window, cx) {
        Some((item, file)) => {
            workspace.activate_item(item.as_ref(), true, false, window, cx);
            Some(file)
        }
        None => active_editor_file(workspace, cx),
    };
    let is_visible = panel_is_visible(workspace, panel.entity_id(), cx);
    let (selected_tab, shows_active_file) = {
        let panel = panel.read(cx);
        let shown_path = panel.file_path();
        let shows_active_file = active_file
            .as_ref()
            .is_some_and(|active_file| shown_path == Some(&active_file.project_path));
        (panel.selected_tab, shows_active_file)
    };

    if !is_visible || selected_tab != ProblemsTab::File || active_file.is_none() {
        panel.update(cx, |panel, cx| {
            panel.selected_tab = ProblemsTab::File;
            panel.show_file(active_file, window, cx);
            cx.notify();
        });
        workspace.focus_panel::<ProjectDiagnosticsPanel>(window, cx);
    } else if shows_active_file {
        let panel_had_focus = panel.read(cx).focus_handle.contains_focused(window, cx);
        workspace.close_panel::<ProjectDiagnosticsPanel>(window, cx);
        if panel_had_focus {
            workspace.focus_center_pane(window, cx);
        }
    } else {
        panel.update(cx, |panel, cx| panel.show_file(active_file, window, cx));
        workspace.focus_panel::<ProjectDiagnosticsPanel>(window, cx);
    }
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

fn active_editor_file(workspace: &Workspace, cx: &App) -> Option<ActiveFile> {
    let editor = workspace.active_item_as::<Editor>(cx)?;
    editor_file(&editor, cx)
}

fn focused_editor_item(
    workspace: &Workspace,
    window: &Window,
    cx: &App,
) -> Option<(Box<dyn ItemHandle>, ActiveFile)> {
    workspace.panes().iter().find_map(|pane| {
        let item = pane.read(cx).active_item()?;
        let editor = item.act_as::<Editor>(cx)?;
        if !editor.focus_handle(cx).contains_focused(window, cx) {
            return None;
        }
        let file = editor_file(&editor, cx)?;
        Some((item, file))
    })
}

fn editor_file(editor: &Entity<Editor>, cx: &App) -> Option<ActiveFile> {
    let editor = editor.read(cx);
    let buffer = editor.buffer().read(cx).as_singleton()?;
    let project_path = editor.active_project_path(cx)?;
    Some(ActiveFile {
        project_path,
        buffer,
    })
}

fn panel_is_visible(workspace: &Workspace, panel_id: EntityId, cx: &App) -> bool {
    workspace.all_docks().into_iter().any(|dock| {
        dock.read(cx)
            .visible_panel()
            .is_some_and(|panel| panel.panel_id() == panel_id)
    })
}

fn visible_problem_count(summary: DiagnosticSummary, include_warnings: bool) -> usize {
    let warning_count = if include_warnings {
        summary.warning_count
    } else {
        0
    };
    summary.error_count + warning_count
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProblemsTab {
    File,
    Project,
}

impl ProblemsTab {
    const ALL: [Self; 2] = [Self::File, Self::Project];

    fn title(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Project => "Project",
        }
    }

    fn element_id(self) -> &'static str {
        match self {
            Self::File => "problems-tab-file",
            Self::Project => "problems-tab-project",
        }
    }
}

pub(crate) struct ActiveFile {
    pub(crate) project_path: ProjectPath,
    pub(crate) buffer: Entity<Buffer>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EmptyState {
    pub(crate) title: SharedString,
    pub(crate) detail: Option<&'static str>,
}

impl EmptyState {
    fn no_selected_file() -> Self {
        Self {
            title: NO_SELECTED_FILE_TEXT.into(),
            detail: None,
        }
    }
}

struct FileProblems {
    project_path: ProjectPath,
    view: Entity<BufferDiagnosticsEditor>,
    _view_observation: Subscription,
}

pub struct ProjectDiagnosticsPanel {
    focus_handle: FocusHandle,
    workspace: WeakEntity<Workspace>,
    project: Entity<Project>,
    fs: Arc<dyn Fs>,
    pub(crate) selected_tab: ProblemsTab,
    pub(crate) project_view: Option<Entity<ProjectDiagnosticsEditor>>,
    file_problems: Option<FileProblems>,
    header_tabs: Entity<ProblemsHeaderTabs>,
    error_count: usize,
    problem_count_update: Task<()>,
    _project_view_observation: Option<Subscription>,
    _project_subscription: Subscription,
    _workspace_subscription: Option<Subscription>,
    _include_warnings_subscription: Subscription,
}

impl ProjectDiagnosticsPanel {
    pub async fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        workspace.update_in(&mut cx, |workspace, window, cx| {
            cx.new(|cx| Self::new(workspace, window, cx))
        })
    }

    pub(crate) fn new(workspace: &Workspace, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let project = workspace.project().clone();
        let project_subscription =
            cx.subscribe(&project, |this, _project, event, cx| match event {
                project::Event::DiskBasedDiagnosticsFinished { .. }
                | project::Event::LanguageServerRemoved(_) => {
                    this.refresh_problem_counts(cx);
                }
                project::Event::DiagnosticsUpdated { .. } => {
                    this.problem_count_update = cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(DIAGNOSTICS_SUMMARY_UPDATE_DEBOUNCE)
                            .await;
                        this.update(cx, |this, cx| {
                            this.refresh_problem_counts(cx);
                        })
                        .log_err();
                    });
                }
                _ => {}
            });
        let workspace_subscription = workspace.weak_handle().upgrade().map(|workspace_entity| {
            cx.subscribe_in(
                &workspace_entity,
                window,
                |this, _workspace, event: &workspace::Event, window, cx| {
                    if matches!(event, workspace::Event::ActiveItemChanged) {
                        this.follow_active_editor(window, cx);
                    }
                },
            )
        });
        let include_warnings_subscription =
            cx.observe_global_in::<IncludeWarnings>(window, |this, window, cx| {
                this.sync_file_view_warnings(window, cx);
            });
        let error_count = project.read(cx).diagnostic_summary(false, cx).error_count;
        let header_tabs = cx.new({
            let panel = cx.weak_entity();
            |_| ProblemsHeaderTabs { panel }
        });
        Self {
            focus_handle: cx.focus_handle(),
            workspace: workspace.weak_handle(),
            project,
            fs: workspace.app_state().fs.clone(),
            selected_tab: ProblemsTab::Project,
            project_view: None,
            file_problems: None,
            header_tabs,
            error_count,
            problem_count_update: Task::ready(()),
            _project_view_observation: None,
            _project_subscription: project_subscription,
            _workspace_subscription: workspace_subscription,
            _include_warnings_subscription: include_warnings_subscription,
        }
    }

    pub(crate) fn file_path(&self) -> Option<&ProjectPath> {
        self.file_problems
            .as_ref()
            .map(|file_problems| &file_problems.project_path)
    }

    pub(crate) fn problem_count(&self, tab: ProblemsTab, cx: &App) -> usize {
        let project = self.project.read(cx);
        let summary = match tab {
            ProblemsTab::Project => project.diagnostic_summary(false, cx),
            ProblemsTab::File => match &self.file_problems {
                Some(file_problems) => {
                    project.diagnostic_summary_for_path(&file_problems.project_path, cx)
                }
                None => return 0,
            },
        };
        visible_problem_count(summary, IncludeWarnings::current(cx))
    }

    #[cfg(test)]
    pub(crate) fn file_tab_empty_state(&self, cx: &App) -> Option<EmptyState> {
        match &self.file_problems {
            Some(file_problems) => self.no_problems_state(file_problems, cx),
            None => Some(EmptyState::no_selected_file()),
        }
    }

    pub(crate) fn select_tab(
        &mut self,
        tab: ProblemsTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_tab != tab {
            self.selected_tab = tab;
            self.sync_selected_tab(window, cx);
            cx.notify();
        }
        self.activation_focus_handle(cx).focus(window, cx);
    }

    pub(crate) fn show_file(
        &mut self,
        active_file: Option<ActiveFile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active_path = active_file
            .as_ref()
            .map(|active_file| &active_file.project_path);
        if self.file_path() == active_path {
            return;
        }
        let panel_had_focus = self.focus_handle.contains_focused(window, cx);
        self.file_problems =
            active_file.map(|active_file| self.build_file_problems(active_file, window, cx));
        if panel_had_focus {
            self.activation_focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    fn build_file_problems(
        &self,
        active_file: ActiveFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> FileProblems {
        let ActiveFile {
            project_path,
            buffer,
        } = active_file;
        let project = self.project.clone();
        let include_warnings = IncludeWarnings::current(cx);
        let view_path = project_path.clone();
        let view = cx.new(|cx| {
            BufferDiagnosticsEditor::new(
                view_path,
                project,
                Some(buffer),
                include_warnings,
                window,
                cx,
            )
        });
        self.attach_to_workspace(view.clone(), window, cx);
        FileProblems {
            project_path,
            _view_observation: cx.observe(&view, |_, _, cx| cx.notify()),
            view,
        }
    }

    fn no_problems_state(&self, file_problems: &FileProblems, cx: &App) -> Option<EmptyState> {
        let include_warnings = IncludeWarnings::current(cx);
        let project = self.project.read(cx);
        let summary = project.diagnostic_summary_for_path(&file_problems.project_path, cx);
        if visible_problem_count(summary, include_warnings) > 0 {
            return None;
        }
        let path = &file_problems.project_path.path;
        let file_name = match path.file_name() {
            Some(file_name) => file_name.to_string(),
            None => path.display(project.path_style(cx)).to_string(),
        };
        Some(EmptyState {
            title: format!("No problems in {file_name}").into(),
            detail: (!include_warnings).then_some(PROBLEMS_FILTERED_OUT_TEXT),
        })
    }

    fn refresh_problem_counts(&mut self, cx: &mut Context<Self>) {
        self.error_count = self
            .project
            .read(cx)
            .diagnostic_summary(false, cx)
            .error_count;
        cx.notify();
    }

    fn sync_selected_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected_tab {
            ProblemsTab::Project => self.ensure_project_view(window, cx),
            ProblemsTab::File => self.follow_active_editor(window, cx),
        }
    }

    fn follow_active_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_tab != ProblemsTab::File {
            return;
        }
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let workspace = workspace.read(cx);
        if !panel_is_visible(workspace, cx.entity_id(), cx) {
            return;
        }
        let active_file = active_editor_file(workspace, cx);
        self.show_file(active_file, window, cx);
    }

    fn sync_file_view_warnings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(file_problems) = &self.file_problems {
            let include_warnings = IncludeWarnings::current(cx);
            file_problems.view.update(cx, |view, cx| {
                if view.include_warnings != include_warnings {
                    view.toggle_warnings(&ToggleWarnings, window, cx);
                }
            });
        }
        cx.notify();
    }

    fn ensure_project_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.project_view.is_some() {
            return;
        }
        let project = self.project.clone();
        let workspace = self.workspace.clone();
        let include_warnings = IncludeWarnings::current(cx);
        let view = cx.new(|cx| {
            ProjectDiagnosticsEditor::new(include_warnings, project, workspace, window, cx)
        });
        self._project_view_observation = Some(cx.observe(&view, |_, _, cx| cx.notify()));
        if self.focus_handle.is_focused(window) {
            view.focus_handle(cx).focus(window, cx);
        }
        self.project_view = Some(view.clone());
        cx.notify();
        self.attach_to_workspace(view, window, cx);
    }

    fn attach_to_workspace<V: Item>(
        &self,
        view: Entity<V>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        match self.selected_tab {
            ProblemsTab::Project => {
                if let Some(view) = &self.project_view {
                    view.update(cx, |view, cx| {
                        view.toggle_diagnostics_refresh(&ToggleDiagnosticsRefresh, window, cx);
                    });
                }
            }
            ProblemsTab::File => {
                if let Some(file_problems) = &self.file_problems {
                    file_problems.view.update(cx, |view, cx| {
                        if view.update_excerpts_task.take().is_none() {
                            view.update_all_excerpts(window, cx);
                        }
                        cx.notify();
                    });
                }
            }
        }
    }

    fn selected_view_is_updating(&self, cx: &App) -> Option<bool> {
        match self.selected_tab {
            ProblemsTab::Project => self
                .project_view
                .as_ref()
                .map(|view| view.read(cx).update_excerpts_task.is_some()),
            ProblemsTab::File => self
                .file_problems
                .as_ref()
                .map(|file_problems| file_problems.view.read(cx).update_excerpts_task.is_some()),
        }
    }

    /// Drawn like the Git tool window's header tabs: the selected tab is an island pill, or
    /// underlined without islands.
    fn render_tab(
        &self,
        tab: ProblemsTab,
        panel: WeakEntity<Self>,
        focused: bool,
        cx: &App,
    ) -> impl IntoElement + use<> {
        let problem_count = self.problem_count(tab, cx);
        let selected = self.selected_tab == tab;
        let colors = cx.theme().colors();
        let metrics = ui::island_tab_metrics(cx);
        let islands = WorkspaceSettings::get_global(cx).islands.enabled;
        let pill = ui::island_tab_pill_colors(focused, colors);
        let hover_background = ui::island_tab_hover_background(selected, focused, colors);
        h_flex()
            .id(tab.element_id())
            .debug_selector(move || tab.element_id().into())
            .relative()
            .h(metrics.pill_height)
            .px(TAB_HORIZONTAL_PADDING)
            .gap(TAB_PROBLEM_COUNT_GAP)
            .rounded(metrics.pill_radius)
            .border_1()
            .border_color(colors.border_transparent)
            .cursor_pointer()
            .when(islands && selected, |this| {
                this.bg(pill.background).border_color(pill.border)
            })
            .when(!islands && selected, |this| {
                this.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(TAB_UNDERLINE_HEIGHT)
                        .bg(if focused {
                            colors.border_selected
                        } else {
                            colors.border
                        }),
                )
            })
            .when_some(hover_background, |this, hover_background| {
                this.hover(move |style| style.bg(hover_background))
            })
            .child(Label::new(tab.title()))
            .when(problem_count > 0, |this| {
                this.child(Label::new(problem_count.to_string()).color(Color::Muted))
            })
            .on_click(move |_, window, cx| {
                panel
                    .update(cx, |panel, cx| panel.select_tab(tab, window, cx))
                    .log_err();
            })
    }

    fn render_project_content(&self) -> AnyElement {
        match &self.project_view {
            Some(view) => view.clone().into_any_element(),
            None => v_flex()
                .size_full()
                .p_2()
                .child(Label::new("Checking diagnostics…").color(Color::Muted))
                .into_any_element(),
        }
    }

    fn render_file_content(&self, cx: &App) -> AnyElement {
        let Some(file_problems) = &self.file_problems else {
            return render_empty_state(EmptyState::no_selected_file()).into_any_element();
        };
        match self.no_problems_state(file_problems, cx) {
            Some(empty_state) => render_empty_state(empty_state)
                .track_focus(&file_problems.view.focus_handle(cx))
                .into_any_element(),
            None => file_problems.view.clone().into_any_element(),
        }
    }
}

fn render_empty_state(empty_state: EmptyState) -> Div {
    v_flex()
        .size_full()
        .p_2()
        .gap_1()
        .items_center()
        .justify_center()
        .text_center()
        .child(Label::new(empty_state.title).color(Color::Muted))
        .when_some(empty_state.detail, |this, detail| {
            this.child(Label::new(detail).color(Color::Muted))
        })
}

impl EventEmitter<PanelEvent> for ProjectDiagnosticsPanel {}

impl Focusable for ProjectDiagnosticsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ProjectDiagnosticsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.selected_tab {
            ProblemsTab::Project => self.render_project_content(),
            ProblemsTab::File => self.render_file_content(cx),
        };
        v_flex()
            .key_context("ProjectDiagnosticsPanel")
            .track_focus(&self.focus_handle)
            .size_full()
            .child(div().flex_1().min_h_0().w_full().child(content))
    }
}

/// The File and Project tabs, which the tool window header shows after the panel name.
struct ProblemsHeaderTabs {
    panel: WeakEntity<ProjectDiagnosticsPanel>,
}

impl Render for ProblemsHeaderTabs {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(panel) = self.panel.upgrade() else {
            return div();
        };
        let panel = panel.read(cx);
        let focused = panel.focus_handle.contains_focused(window, cx);
        h_flex().h_full().gap_0p5().children(
            ProblemsTab::ALL.map(|tab| panel.render_tab(tab, self.panel.clone(), focused, cx)),
        )
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
        let selected_view_focus_handle = match self.selected_tab {
            ProblemsTab::Project => self.project_view.as_ref().map(|view| view.focus_handle(cx)),
            ProblemsTab::File => self
                .file_problems
                .as_ref()
                .map(|file_problems| file_problems.view.focus_handle(cx)),
        };
        selected_view_focus_handle.unwrap_or_else(|| self.focus_handle.clone())
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
        let settings = ProjectDiagnosticsPanelSettings::get_global(cx);
        match self.position(window, cx) {
            DockPosition::Left | DockPosition::Right => settings.default_width,
            DockPosition::Bottom => settings.default_height,
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
                this.sync_selected_tab(window, cx);
            });
        }
    }

    fn hide_button_setting(&self, _: &App) -> Option<HideButtonSetting> {
        Some(HideButtonSetting::new(|settings| {
            settings
                .project_diagnostics_panel
                .get_or_insert_default()
                .button = Some(false);
        }))
    }

    fn header_content(&self, _: &Window, _: &App) -> Option<AnyView> {
        Some(self.header_tabs.clone().into())
    }

    fn header_actions(&self, _: &Window, cx: &App) -> Vec<PanelHeaderAction> {
        let mut actions = Vec::new();
        if let Some(is_updating) = self.selected_view_is_updating(cx) {
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
