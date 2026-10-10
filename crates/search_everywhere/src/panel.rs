use std::sync::Arc;

use anyhow::Result;
use editor::Editor;
use file_finder::FoundPath;
use fs::Fs;
use gpui::{
    Action, App, AppContext as _, AsyncWindowContext, Context, DismissEvent, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement, ParentElement, Pixels, Render, SharedString,
    Styled, Subscription, Task, WeakEntity, Window, div,
};
use search::project_search::{ProjectSearchView, buffer_search_query};
use settings::{IntoGpui, RegisterSetting, Settings};
use ui::{IconName, IntoElement};
use util::{ResultExt, paths::PathMatcher};
use workspace::{
    DeploySearch, HideButtonSetting, Workspace,
    dock::{DockPosition, Panel, PanelEvent},
    searchable::SearchableItemHandle,
};
use zed_actions::{
    search::NewSearchInDirectory, search_everywhere::Tab, search_panel::ToggleFocus,
};

use crate::{
    SearchEverywhere,
    delegate::{Presentation, TextScope},
};

const SEARCH_PANEL_KEY: &str = "SearchPanel";
const SEARCH_PANEL_ACTIVATION_PRIORITY: u32 = 4;

#[derive(Debug, Clone, PartialEq, RegisterSetting)]
pub struct SearchPanelSettings {
    pub button: bool,
    pub dock: DockPosition,
    pub default_width: Pixels,
}

impl Settings for SearchPanelSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let search_panel = content.search_panel.clone().unwrap();
        Self {
            button: search_panel.button.unwrap(),
            dock: search_panel.dock.unwrap().into(),
            default_width: search_panel.default_width.unwrap().into_gpui(),
        }
    }
}

pub(crate) fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _window, _cx| {
        workspace.register_action(|workspace, _: &ToggleFocus, window, cx| {
            capture_context_into_panel(workspace, window, cx);
            workspace.toggle_panel_focus::<SearchPanel>(window, cx);
        });
        workspace.register_action(deploy_search);
        workspace.register_action(search_in_directory);
    })
    .detach();
}

struct ContextSnapshot {
    previous_focus_handle: FocusHandle,
    recent_files: Task<Vec<FoundPath>>,
    actions: Option<Vec<(SharedString, Box<dyn Action>)>>,
}

impl ContextSnapshot {
    fn capture(
        workspace: &Workspace,
        panel_focus_handle: &FocusHandle,
        search_focus_handle: &FocusHandle,
        window: &Window,
        cx: &App,
    ) -> Self {
        let previous_focus_handle = workspace
            .active_item(cx)
            .map(|item| item.item_focus_handle(cx))
            .unwrap_or_else(|| workspace.focus_handle(cx));
        let panel_has_focus = panel_focus_handle.contains_focused(window, cx)
            || search_focus_handle.is_focused(window);
        let actions = (!panel_has_focus).then(|| command_palette::palette_actions(window, cx));
        Self {
            previous_focus_handle,
            recent_files: file_finder::history_items(workspace, cx),
            actions,
        }
    }
}

pub struct SearchPanel {
    focus_handle: FocusHandle,
    workspace: WeakEntity<Workspace>,
    fs: Arc<dyn Fs>,
    pub(crate) search: Entity<SearchEverywhere>,
    _recent_files_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl SearchPanel {
    pub async fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        workspace.update_in(&mut cx, |workspace, window, cx| {
            cx.new(|cx| Self::new(workspace, window, cx))
        })
    }

    pub(crate) fn new(workspace: &Workspace, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let search = cx.new(|cx| {
            SearchEverywhere::new(
                workspace.weak_handle(),
                workspace.project().clone(),
                workspace.focus_handle(cx),
                Vec::new(),
                Tab::All,
                Presentation::Panel,
                window,
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe_in(&search, window, Self::search_dismissed),
            cx.on_focus_in(&focus_handle, window, |_, window, cx| {
                cx.defer_in(window, |this, window, cx| {
                    this.refresh_context(window, cx);
                });
            }),
        ];
        Self {
            focus_handle,
            workspace: workspace.weak_handle(),
            fs: workspace.app_state().fs.clone(),
            search,
            _recent_files_task: Task::ready(()),
            _subscriptions: subscriptions,
        }
    }

    fn search_dismissed(
        &mut self,
        _: &Entity<SearchEverywhere>,
        _: &DismissEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus_handle.contains_focused(window, cx) {
            return;
        }
        let workspace = self.workspace.clone();
        let panel_focus_handle = self.focus_handle.clone();
        window.defer(cx, move |window, cx| {
            if !panel_focus_handle.contains_focused(window, cx) {
                return;
            }
            workspace
                .update(cx, |workspace, cx| {
                    workspace
                        .fallback_focus_handle(window, cx)
                        .focus(window, cx);
                })
                .log_err();
        });
    }

    fn refresh_context(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let search_focus_handle = self.search.focus_handle(cx);
        let snapshot = ContextSnapshot::capture(
            workspace.read(cx),
            &self.focus_handle,
            &search_focus_handle,
            window,
            cx,
        );
        self.apply_context(snapshot, window, cx);
    }

    fn apply_context(
        &mut self,
        snapshot: ContextSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ContextSnapshot {
            previous_focus_handle,
            recent_files,
            actions,
        } = snapshot;
        self.search.update(cx, |search, cx| {
            search.set_context(previous_focus_handle, actions, window, cx);
        });
        self._recent_files_task = cx.spawn_in(window, async move |this, cx| {
            let recent_files = recent_files.await;
            this.update_in(cx, |this, window, cx| {
                this.search.update(cx, |search, cx| {
                    search.set_recent_files(recent_files, window, cx);
                });
            })
            .ok();
        });
    }

    fn show_text_search(
        &mut self,
        query: Option<String>,
        scope: Option<TextScope>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search.update(cx, |search, cx| {
            search.show_text_search(query, scope, window, cx);
        });
    }
}

fn capture_context_into_panel(
    workspace: &Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Option<Entity<SearchPanel>> {
    let panel = workspace.panel::<SearchPanel>(cx)?;
    let (panel_focus_handle, search_focus_handle) = {
        let panel = panel.read(cx);
        (panel.focus_handle.clone(), panel.search.focus_handle(cx))
    };
    let snapshot = ContextSnapshot::capture(
        workspace,
        &panel_focus_handle,
        &search_focus_handle,
        window,
        cx,
    );
    panel.update(cx, |panel, cx| panel.apply_context(snapshot, window, cx));
    Some(panel)
}

fn open_text_search(
    workspace: &mut Workspace,
    query: Option<String>,
    scope: Option<TextScope>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(panel) = capture_context_into_panel(workspace, window, cx) else {
        return;
    };
    panel.update(cx, |panel, cx| {
        panel.show_text_search(query, scope, window, cx);
    });
    workspace.focus_panel::<SearchPanel>(window, cx);
    cx.notify();
}

fn query_seed(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Option<String> {
    let item = workspace.active_item(cx)?;
    if let Some(buffer_search_query) = buffer_search_query(workspace, item.as_ref(), cx) {
        return Some(buffer_search_query);
    }
    let editor = item.act_as::<Editor>(cx)?;
    let query = editor.query_suggestion(None, window, cx);
    (!query.is_empty()).then_some(query)
}

fn needs_project_search_view(action: &DeploySearch) -> bool {
    action.replace_enabled
        || action.included_files.is_some()
        || action.excluded_files.is_some()
        || action.regex.is_some()
        || action.case_sensitive.is_some()
        || action.whole_word.is_some()
        || action.include_ignored.is_some()
}

fn deploy_search(
    workspace: &mut Workspace,
    action: &DeploySearch,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    if workspace.has_active_modal(window, cx) && !workspace.hide_modal(window, cx) {
        cx.propagate();
        return;
    }
    if needs_project_search_view(action) || workspace.panel::<SearchPanel>(cx).is_none() {
        ProjectSearchView::deploy_search(workspace, action, window, cx);
        cx.notify();
        return;
    }
    let query = action
        .query
        .clone()
        .filter(|query| !query.is_empty())
        .or_else(|| query_seed(workspace, window, cx));
    open_text_search(workspace, query, None, window, cx);
}

fn search_in_directory(
    workspace: &mut Workspace,
    action: &NewSearchInDirectory,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    if workspace.panel::<SearchPanel>(cx).is_none() {
        ProjectSearchView::new_search_with_filter(workspace, action.directory.clone(), window, cx);
        cx.notify();
        return;
    }
    let path_style = workspace.project().read(cx).path_style(cx);
    let scope = (!action.directory.is_empty())
        .then(|| PathMatcher::new([action.directory.as_str()], path_style).log_err())
        .flatten()
        .map(|matcher| TextScope {
            directory: action.directory.clone().into(),
            matcher,
        });
    let query = query_seed(workspace, window, cx);
    open_text_search(workspace, query, scope, window, cx);
}

impl EventEmitter<PanelEvent> for SearchPanel {}

impl Focusable for SearchPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SearchPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("SearchPanel")
            .track_focus(&self.focus_handle)
            .size_full()
            .child(self.search.clone())
    }
}

impl Panel for SearchPanel {
    fn persistent_name() -> &'static str {
        "SearchPanel"
    }

    fn panel_key() -> &'static str {
        SEARCH_PANEL_KEY
    }

    fn activation_focus_handle(&self, cx: &App) -> FocusHandle {
        self.search.focus_handle(cx)
    }

    fn position(&self, _: &Window, cx: &App) -> DockPosition {
        SearchPanelSettings::get_global(cx).dock
    }

    fn position_is_valid(&self, _: DockPosition) -> bool {
        true
    }

    fn set_position(&mut self, position: DockPosition, _: &mut Window, cx: &mut Context<Self>) {
        settings::update_settings_file(self.fs.clone(), cx, move |settings, _| {
            settings.search_panel.get_or_insert_default().dock = Some(position.into())
        });
    }

    fn default_size(&self, _: &Window, cx: &App) -> Pixels {
        SearchPanelSettings::get_global(cx).default_width
    }

    fn icon(&self, _: &Window, cx: &App) -> Option<IconName> {
        SearchPanelSettings::get_global(cx)
            .button
            .then_some(IconName::MagnifyingGlass)
    }

    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<&'static str> {
        Some("Search")
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(ToggleFocus)
    }

    fn activation_priority(&self) -> u32 {
        SEARCH_PANEL_ACTIVATION_PRIORITY
    }

    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if active {
            cx.defer_in(window, |this, window, cx| {
                this.refresh_context(window, cx);
            });
        }
    }

    fn hide_button_setting(&self, _: &App) -> Option<HideButtonSetting> {
        Some(HideButtonSetting::new(|settings| {
            settings.search_panel.get_or_insert_default().button = Some(false);
        }))
    }
}
