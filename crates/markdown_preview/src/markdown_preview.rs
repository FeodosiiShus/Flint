mod markdown_preview_view;

use editor::{Editor, EditorEvent, EditorPreview};
use gpui::{App, Context, Focusable as _, Window, actions};

use crate::markdown_preview_view::MarkdownPreview;

actions!(
    markdown,
    [
        #[action(deprecated_aliases = ["markdown::MovePageUp"])]
        ScrollPageUp,
        #[action(deprecated_aliases = ["markdown::MovePageDown"])]
        ScrollPageDown,
        ScrollUp,
        ScrollDown,
        ScrollUpByItem,
        ScrollDownByItem,
        ScrollToTop,
        ScrollToBottom,
    ]
);

pub fn init(cx: &mut App) {
    cx.observe_new(|editor: &mut Editor, window, cx| {
        let Some(window) = window else {
            return;
        };
        if !can_host_markdown_preview(editor, cx) {
            return;
        }
        let editor_entity = cx.entity();
        cx.subscribe_in(
            &editor_entity,
            window,
            |editor, _, event: &EditorEvent, window, cx| {
                if matches!(event, EditorEvent::Reparsed(_)) {
                    sync_markdown_preview(editor, window, cx);
                }
            },
        )
        .detach();
        sync_markdown_preview(editor, window, cx);
    })
    .detach();
}

fn can_host_markdown_preview(editor: &Editor, cx: &App) -> bool {
    let buffer = editor.buffer().read(cx);
    editor.mode().is_full()
        && editor.project().is_some()
        && buffer.as_singleton().is_some()
        && !buffer.all_diff_hunks_expanded()
}

fn is_markdown_editor(editor: &Editor, cx: &App) -> bool {
    editor
        .buffer()
        .read(cx)
        .as_singleton()
        .is_some_and(|buffer| {
            buffer
                .read(cx)
                .language()
                .is_some_and(|language| language.name() == "Markdown")
        })
}

fn sync_markdown_preview(editor: &mut Editor, window: &mut Window, cx: &mut Context<Editor>) {
    let is_markdown = is_markdown_editor(editor, cx);
    if is_markdown == editor.has_preview() {
        return;
    }
    if is_markdown {
        let preview = MarkdownPreview::new(editor, window, cx);
        let focus_handle = preview.focus_handle(cx);
        editor.set_preview(
            Some(EditorPreview {
                view: preview.into(),
                focus_handle,
            }),
            window,
            cx,
        );
    } else {
        editor.set_preview(None, window, cx);
    }
}
