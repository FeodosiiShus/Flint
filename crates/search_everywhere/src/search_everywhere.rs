mod actions;
mod delegate;
mod files;
mod symbols;
mod text;

#[cfg(test)]
mod search_everywhere_tests;

use file_finder::FoundPath;
use gpui::{
    App, AppContext, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, ParentElement, Render, Task, WeakEntity, Window,
};
use picker::{Direction, Picker};
use project::Project;
use ui::{IntoElement, v_flex};
use workspace::{ModalView, Workspace};
use zed_actions::search_everywhere::{
    NextSection, NextTab, PreviousSection, PreviousTab, Tab, Toggle, ToggleNonProjectItems,
};

use delegate::{SearchEverywhereDelegate, activate_tab, adjacent_tab};

pub fn init(cx: &mut App) {
    cx.observe_new(SearchEverywhere::register).detach();
}

pub struct SearchEverywhere {
    picker: Entity<Picker<SearchEverywhereDelegate>>,
}

impl SearchEverywhere {
    fn register(
        workspace: &mut Workspace,
        _window: Option<&mut Window>,
        _: &mut Context<Workspace>,
    ) {
        workspace.register_action(|workspace, action: &Toggle, window, cx| {
            if let Some(search_everywhere) = workspace.active_modal::<Self>(cx) {
                search_everywhere.update(cx, |search_everywhere, cx| {
                    search_everywhere
                        .picker
                        .update(cx, |picker, cx| match action.tab {
                            Some(tab) if tab != picker.delegate.tab() => {
                                activate_tab(picker, tab, window, cx);
                            }
                            _ => {
                                picker.delegate.toggle_non_project_items();
                                picker.refresh(window, cx);
                            }
                        });
                });
                return;
            }
            Self::open(workspace, action.tab.unwrap_or_default(), window, cx).detach();
        });
    }

    fn open(
        workspace: &mut Workspace,
        tab: Tab,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Task<()> {
        let recent_files = file_finder::history_items(workspace, cx);
        cx.spawn_in(window, async move |workspace, cx| {
            let recent_files = recent_files.await;
            workspace
                .update_in(cx, |workspace, window, cx| {
                    let previous_focus_handle = window
                        .focused(cx)
                        .unwrap_or_else(|| workspace.focus_handle(cx));
                    let project = workspace.project().clone();
                    let weak_workspace = cx.entity().downgrade();
                    workspace.toggle_modal(window, cx, |window, cx| {
                        SearchEverywhere::new(
                            weak_workspace,
                            project,
                            previous_focus_handle,
                            recent_files,
                            tab,
                            window,
                            cx,
                        )
                    });
                })
                .ok();
        })
    }

    fn new(
        workspace: WeakEntity<Workspace>,
        project: Entity<Project>,
        previous_focus_handle: FocusHandle,
        recent_files: Vec<FoundPath>,
        tab: Tab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let commands = command_palette::palette_actions(window, cx);
        let delegate = SearchEverywhereDelegate::new(
            cx.entity().downgrade(),
            workspace,
            project.clone(),
            previous_focus_handle,
            recent_files,
            commands,
            tab,
            cx.focus_handle(),
        );
        let preview = picker_preview::editor_preview(project, window, cx);
        let picker = cx.new(|cx| Picker::uniform_list_with_preview(delegate, preview, window, cx));
        let picker_focus_handle = picker.focus_handle(cx);
        picker.update(cx, |picker, _| {
            picker.delegate.set_focus_handle(picker_focus_handle);
        });
        Self { picker }
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab(true, window, cx);
    }

    fn previous_tab(&mut self, _: &PreviousTab, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab(false, window, cx);
    }

    fn cycle_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.picker.update(cx, |picker, cx| {
            let tab = adjacent_tab(picker.delegate.tab(), forward);
            activate_tab(picker, tab, window, cx);
        });
    }

    fn next_section(&mut self, _: &NextSection, window: &mut Window, cx: &mut Context<Self>) {
        self.jump_to_section(Direction::Down, window, cx);
    }

    fn previous_section(
        &mut self,
        _: &PreviousSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.jump_to_section(Direction::Up, window, cx);
    }

    fn jump_to_section(
        &mut self,
        direction: Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.picker.update(cx, |picker, cx| {
            if let Some(index) = picker.delegate.section_start(direction) {
                picker.set_selected_index(index, Some(Direction::Down), true, window, cx);
                cx.notify();
            }
        });
    }

    fn toggle_non_project_items(
        &mut self,
        _: &ToggleNonProjectItems,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.picker.update(cx, |picker, cx| {
            picker.delegate.toggle_non_project_items();
            picker.refresh(window, cx);
        });
    }
}

impl ModalView for SearchEverywhere {}

impl EventEmitter<DismissEvent> for SearchEverywhere {}

impl Focusable for SearchEverywhere {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.focus_handle(cx)
    }
}

impl Render for SearchEverywhere {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .key_context("SearchEverywhere")
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .on_action(cx.listener(Self::next_section))
            .on_action(cx.listener(Self::previous_section))
            .on_action(cx.listener(Self::toggle_non_project_items))
            .child(self.picker.clone())
    }
}
