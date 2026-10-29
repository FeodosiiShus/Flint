use std::time::Instant;

use collections::HashSet;
use gpui::{App, AsyncWindowContext, Context, Entity, SharedString, Task, WeakEntity, Window};
use settings::Settings as _;
use util::ResultExt as _;

use crate::{
    Workspace,
    item::{ItemHandle, SaveOptions},
    pane::Pane,
    workspace_error::{ErrorAction, ErrorSeverity, WorkspaceError},
    workspace_settings::{AutosaveSetting, WorkspaceSettings},
};

/// What prompts saving every modified file of the workspace without asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutosaveTrigger {
    /// The user switches to another application or to the built-in terminal, where a build or
    /// another tool may read the files from disk. Saves when `autosave` is `on_window_change` or
    /// `on_focus_change`.
    SwitchAway,
    /// Nothing has been typed or clicked for `autosave_when_idle.seconds`.
    Idle,
}

impl AutosaveTrigger {
    fn is_enabled(self, settings: &WorkspaceSettings) -> bool {
        match self {
            Self::SwitchAway => matches!(
                settings.autosave,
                AutosaveSetting::OnWindowChange | AutosaveSetting::OnFocusChange
            ),
            Self::Idle => settings.autosave_when_idle.is_some(),
        }
    }
}

pub(crate) struct IdleAutosave {
    last_activity: Instant,
    timer: Option<Task<()>>,
}

impl IdleAutosave {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            last_activity: now,
            timer: None,
        }
    }
}

pub(crate) struct AutosaveError {
    failures: Vec<(SharedString, anyhow::Error)>,
}

impl WorkspaceError for AutosaveError {
    fn primary_message(&self) -> SharedString {
        match self.failures.as_slice() {
            [(file_name, _)] => format!("Could not save {file_name}").into(),
            failures => format!("Could not save {} files", failures.len()).into(),
        }
    }

    fn secondary_message(&self) -> Option<SharedString> {
        let details = self
            .failures
            .iter()
            .map(|(file_name, error)| format!("{file_name}: {error:#}"))
            .collect::<Vec<_>>();
        Some(details.join("\n").into())
    }

    fn primary_action(&self) -> ErrorAction {
        ErrorAction::dismiss()
    }

    fn severity(&self) -> ErrorSeverity {
        ErrorSeverity::Error
    }
}

impl Workspace {
    /// Saves every modified file of the workspace, in the center and in the docks, like
    /// IntelliJ's `saveAllDocuments(isExplicit = false)`. Files without a path, with a conflict,
    /// deleted on disk or read only stay unsaved, and a failed save is shown to the user.
    pub fn autosave_dirty_items(
        &mut self,
        trigger: AutosaveTrigger,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let items = self.items_to_autosave(trigger, cx);
        if items.is_empty() {
            return Task::ready(());
        }

        let saves = items
            .into_iter()
            .map(|item| {
                let file_name = item.tab_content_text(0, cx);
                let save = item.save(
                    SaveOptions {
                        format: true,
                        force_format: false,
                        autosave: true,
                    },
                    self.project.clone(),
                    window,
                    cx,
                );
                (file_name, save)
            })
            .collect::<Vec<_>>();
        cx.spawn_in(window, async move |workspace, cx| {
            let mut failures = Vec::new();
            for (file_name, save) in saves {
                if let Err(error) = save.await {
                    log::error!("Failed to autosave {file_name}: {error:#}");
                    failures.push((file_name, error));
                }
            }
            if !failures.is_empty() {
                workspace
                    .update(cx, |workspace, cx| {
                        workspace.show_error(AutosaveError { failures }, cx)
                    })
                    .log_err();
            }
        })
    }

    /// Restarts the countdown to the idle autosave. Key presses, mouse clicks, focus changes,
    /// window activation and edits count as activity, like in IntelliJ's idle tracker; moving or
    /// scrolling the mouse doesn't.
    pub(crate) fn reset_idle_autosave_timer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if WorkspaceSettings::get_global(cx)
            .autosave_when_idle
            .is_none()
        {
            self.idle_autosave.timer = None;
            return;
        }
        self.idle_autosave.last_activity = cx.background_executor().now();
        // A running countdown sleeps again for the rest of the delay when it wakes up, which
        // spares starting a timer on every key press.
        if self.idle_autosave.timer.is_none() {
            self.idle_autosave.timer = Some(cx.spawn_in(window, async move |workspace, cx| {
                Self::run_idle_autosave_countdown(workspace, cx).await;
            }));
        }
    }

    async fn run_idle_autosave_countdown(workspace: WeakEntity<Self>, cx: &mut AsyncWindowContext) {
        loop {
            let Ok(remaining) = workspace.update(cx, |workspace, cx| {
                let delay = WorkspaceSettings::get_global(cx).autosave_when_idle?;
                let idle_for = cx
                    .background_executor()
                    .now()
                    .saturating_duration_since(workspace.idle_autosave.last_activity);
                Some(delay.saturating_sub(idle_for))
            }) else {
                return;
            };
            match remaining {
                Some(remaining) if remaining.is_zero() => break,
                Some(remaining) => cx.background_executor().timer(remaining).await,
                None => {
                    workspace
                        .update(cx, |workspace, _| workspace.idle_autosave.timer = None)
                        .log_err();
                    return;
                }
            }
        }

        workspace
            .update_in(cx, |workspace, window, cx| {
                workspace.idle_autosave.timer = None;
                // Like IntelliJ, which saves on idle only outside modal dialogs, wait for the next
                // idle period while one is open.
                if !workspace.has_active_modal(window, cx) {
                    workspace
                        .autosave_dirty_items(AutosaveTrigger::Idle, window, cx)
                        .detach();
                }
            })
            .log_err();
    }

    fn items_to_autosave(&self, trigger: AutosaveTrigger, cx: &App) -> Vec<Box<dyn ItemHandle>> {
        let mut items = Vec::new();
        let mut vetoed_buffers = HashSet::default();
        for pane in self.panes_including_docks(cx) {
            for item in pane.read(cx).items() {
                if !trigger.is_enabled(item.workspace_settings(cx)) || !item.can_autosave(cx) {
                    continue;
                }
                // Switching away saves an item anyway: the tool the user switches to reads the
                // file from disk, and an editor closes its completion menu when it loses focus.
                if trigger == AutosaveTrigger::Idle && item.vetoes_implicit_save(cx) {
                    vetoed_buffers.extend(item.project_item_model_ids(cx));
                } else {
                    items.push(item.boxed_clone());
                }
            }
        }

        let mut saved_buffers = HashSet::default();
        items.retain(|item| {
            let buffers = item.project_item_model_ids(cx);
            if buffers.iter().any(|buffer| vetoed_buffers.contains(buffer)) {
                return false;
            }
            // Several editors can show one buffer, and saving one of them writes it.
            let mut saves_another_buffer = buffers.is_empty();
            for buffer in buffers {
                saves_another_buffer |= saved_buffers.insert(buffer);
            }
            saves_another_buffer
        });
        items
    }

    fn panes_including_docks(&self, cx: &App) -> Vec<Entity<Pane>> {
        let mut panes = self.panes.clone();
        let dock_panes = self
            .panes_by_item
            .values()
            .filter_map(|pane| pane.upgrade())
            .chain(
                self.all_docks()
                    .into_iter()
                    .filter_map(|dock| dock.read(cx).active_panel()?.pane(cx)),
            )
            .collect::<Vec<_>>();
        for pane in dock_panes {
            if !panes.contains(&pane) {
                panes.push(pane);
            }
        }
        panes
    }
}
