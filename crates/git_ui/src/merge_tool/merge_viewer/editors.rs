use editor::{CurrentLineHighlight, Editor};
use gpui::{AppContext as _, Context, Entity, Window};
use language::language_settings::{ShowWhitespaceSetting, SoftWrap};
use merge_diff::ThreeSide;

use super::{Appearance, MergeViewer, viewer_settings};
use crate::merge_tool::merge_model::MergeConflictModel;

pub(super) fn create_pane_editors(
    model: &Entity<MergeConflictModel>,
    appearance: Appearance,
    window: &mut Window,
    cx: &mut Context<MergeViewer>,
) -> [Entity<Editor>; 3] {
    let buffers = model.read(cx).buffers().clone();
    let left = create_pane_editor(buffers.left, true, appearance, window, cx);
    let result = create_pane_editor(buffers.result, false, appearance, window, cx);
    let right = create_pane_editor(buffers.right, true, appearance, window, cx);
    [left, result, right]
}

fn create_pane_editor(
    buffer: Entity<language::Buffer>,
    read_only: bool,
    appearance: Appearance,
    window: &mut Window,
    cx: &mut Context<MergeViewer>,
) -> Entity<Editor> {
    cx.new(|cx| {
        let mut editor = Editor::for_buffer(buffer, None, window, cx);
        configure_pane_editor(&mut editor, read_only, appearance, cx);
        editor
    })
}

fn configure_pane_editor(
    editor: &mut Editor,
    read_only: bool,
    appearance: Appearance,
    cx: &mut Context<Editor>,
) {
    editor.set_read_only(read_only);
    editor.set_input_enabled(!read_only);
    editor.set_should_serialize(false, cx);
    editor.set_show_vertical_scrollbar(true, cx);
    editor.set_show_horizontal_scrollbar(true, cx);
    editor.disable_inline_diagnostics();
    editor.disable_diagnostics(cx);
    editor.disable_expand_excerpt_buttons(cx);
    editor.disable_mouse_wheel_zoom();
    editor.set_show_gutter(false, cx);
    editor.set_show_line_numbers(false, cx);
    editor.set_show_git_diff_gutter(false, cx);
    editor.set_show_code_actions(false, cx);
    editor.set_show_breakpoints(false, cx);
    editor.set_show_bookmarks(false, cx);
    editor.set_show_wrap_guides(false, cx);
    editor.set_show_cursor_when_unfocused(true, cx);
    editor.set_current_line_highlight(Some(CurrentLineHighlight::None));
    apply_appearance_to_editor(editor, appearance, cx);
}

fn apply_appearance_to_editor(
    editor: &mut Editor,
    appearance: Appearance,
    cx: &mut Context<Editor>,
) {
    let soft_wrap = if appearance.soft_wrap {
        SoftWrap::EditorWidth
    } else {
        SoftWrap::None
    };
    editor.set_soft_wrap_mode(soft_wrap, cx);
    editor.set_show_indent_guides(appearance.indent_guides, cx);
    editor.set_show_whitespaces_override(Some(whitespace_setting(appearance.show_whitespaces)), cx);
}

fn whitespace_setting(shown: bool) -> ShowWhitespaceSetting {
    if shown {
        ShowWhitespaceSetting::All
    } else {
        ShowWhitespaceSetting::None
    }
}

impl MergeViewer {
    pub(super) fn set_result_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        self.editor(ThreeSide::Base).update(cx, |editor, _| {
            editor.set_read_only(!editable);
            editor.set_input_enabled(editable);
        });
    }

    pub(crate) fn apply_appearance(&mut self, appearance: Appearance, cx: &mut Context<Self>) {
        self.appearance = appearance;
        viewer_settings::store_appearance(appearance, cx);
        for side in ThreeSide::ALL {
            self.editor(side).update(cx, |editor, cx| {
                apply_appearance_to_editor(editor, appearance, cx);
            });
        }
        cx.notify();
    }
}
