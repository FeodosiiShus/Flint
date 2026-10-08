use gpui::{Action, Context, Entity, Global, VisualTestContext, Window, WindowHandle};
use merge_diff::Side;

use super::ConflictsDialog;
use crate::merge_tool::dialog_window::DialogWindowShell;

pub(super) struct OpenedDialog {
    pub(super) window: WindowHandle<DialogWindowShell>,
    pub(super) content: Entity<ConflictsDialog>,
}

impl Global for OpenedDialog {}

pub(crate) struct OpenConflictsDialog {
    pub(crate) window: WindowHandle<DialogWindowShell>,
    dialog: Entity<ConflictsDialog>,
    cx: VisualTestContext,
}

pub(crate) fn find_open_dialog(cx: &mut VisualTestContext) -> Option<OpenConflictsDialog> {
    let (window, dialog) = cx.update(|_, cx| {
        cx.try_global::<OpenedDialog>()
            .map(|opened| (opened.window, opened.content.clone()))
    })?;
    let dialog_cx = VisualTestContext::from_window(window.into(), cx);
    Some(OpenConflictsDialog {
        window,
        dialog,
        cx: dialog_cx,
    })
}

impl OpenConflictsDialog {
    pub(super) fn read<R>(&self, reader: impl FnOnce(&ConflictsDialog) -> R) -> R {
        self.dialog.read_with(&self.cx, |dialog, _| reader(dialog))
    }

    pub(super) fn update_in<R>(
        &mut self,
        update: impl FnOnce(&mut ConflictsDialog, &mut Window, &mut Context<ConflictsDialog>) -> R,
    ) -> R {
        let result = self.dialog.update_in(&mut self.cx, update);
        self.cx.run_until_parked();
        result
    }

    pub(super) fn dispatch(&mut self, action: impl Action) {
        self.cx.dispatch_action(action);
        self.cx.run_until_parked();
    }

    pub(crate) fn accept(&mut self, side: Side) {
        self.update_in(|dialog, window, cx| dialog.accept_for_resolution(side, window, cx));
    }

    pub(crate) fn close(&mut self) {
        self.update_in(|dialog, window, cx| dialog.close_button_pressed(window, cx));
    }

    pub(crate) fn accept_and_finish(&mut self) {
        self.update_in(|dialog, window, cx| dialog.accept_and_finish(window, cx));
    }

    pub(crate) fn resolve_all_simple_conflicts(&mut self) {
        self.update_in(|dialog, window, cx| dialog.resolve_all_simple_conflicts(window, cx));
    }

    pub(crate) fn review(&mut self) {
        self.update_in(|dialog, window, cx| dialog.review_button_pressed(window, cx));
    }

    pub(crate) fn unresolved_files(&self) -> Vec<String> {
        self.read(|dialog| {
            dialog
                .state
                .unresolved_group_files()
                .iter()
                .map(|file| file.as_unix_str().to_string())
                .collect()
        })
    }

    pub(crate) fn resolved_files(&self) -> Vec<String> {
        self.read(|dialog| {
            dialog
                .state
                .resolved_file_paths()
                .iter()
                .map(|file| file.as_unix_str().to_string())
                .collect()
        })
    }

    pub(crate) fn column_titles(&self) -> (String, String) {
        self.read(|dialog| {
            (
                dialog.texts.yours_column.to_string(),
                dialog.texts.theirs_column.to_string(),
            )
        })
    }

    pub(crate) fn auto_resolve_status(&self) -> Option<String> {
        self.read(|dialog| dialog.auto_resolve_status.clone())
    }
}
