use anyhow::{Result, ensure};
use gpui::{AnyWindowHandle, DismissEvent, EventEmitter, FocusHandle, Focusable, WeakEntity};
use ui::prelude::*;
use workspace::{DismissDecision, ModalView, Workspace};

pub(super) struct WorkspaceBlocker {
    focus_handle: FocusHandle,
    holders: usize,
    released: bool,
}

impl WorkspaceBlocker {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            holders: 1,
            released: false,
        }
    }

    fn release_one(&mut self) -> bool {
        self.holders = self.holders.saturating_sub(1);
        self.released = self.holders == 0;
        self.released
    }
}

impl EventEmitter<DismissEvent> for WorkspaceBlocker {}

impl Focusable for WorkspaceBlocker {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ModalView for WorkspaceBlocker {
    fn on_before_dismiss(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> DismissDecision {
        DismissDecision::Dismiss(self.released)
    }
}

impl Render for WorkspaceBlocker {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus_handle)
    }
}

pub(super) fn block_workspace(
    workspace: &WeakEntity<Workspace>,
    window: AnyWindowHandle,
    cx: &mut App,
) -> Result<()> {
    window.update(cx, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            if let Some(blocker) = workspace.active_modal::<WorkspaceBlocker>(cx) {
                blocker.update(cx, |blocker, _| blocker.holders += 1);
                return Ok(());
            }
            workspace.toggle_modal(window, cx, |_, cx| WorkspaceBlocker::new(cx));
            ensure!(
                workspace.active_modal::<WorkspaceBlocker>(cx).is_some(),
                "another modal of the workspace refused to close"
            );
            Ok(())
        })
    })???;
    Ok(())
}

pub(super) fn release_workspace(
    workspace: &WeakEntity<Workspace>,
    window: AnyWindowHandle,
    cx: &mut App,
) -> Result<()> {
    window.update(cx, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            let Some(blocker) = workspace.active_modal::<WorkspaceBlocker>(cx) else {
                return;
            };
            let fully_released = blocker.update(cx, |blocker, _| blocker.release_one());
            if fully_released {
                workspace.hide_modal(window, cx);
            }
        })
    })??;
    Ok(())
}
