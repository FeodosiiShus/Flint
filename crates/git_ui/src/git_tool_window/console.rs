use std::ops::Range;

use editor::{Editor, HighlightKey, MultiBufferOffset, actions::MoveToEnd};
use git::console::{GitConsoleContentType, GitConsoleLine};
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, HighlightStyle, Hsla, Pixels, Task, Window, px,
    rgb,
};
use language::language_settings::SoftWrap;
use project::git_store::GitStore;
use ui::{IconButton, IconButtonShape, Tooltip, prelude::*};

use super::persistence;
use super::{ClearConsole, ScrollConsoleToEnd, ToggleConsoleSoftWraps};

const CONSOLE_KEY_CONTEXT: &str = "GitConsole";
const TOOLBAR_WIDTH: Pixels = px(36.);
const TOOLBAR_BUTTON_SIZE: Pixels = px(22.);
const TOOLBAR_VERTICAL_INSET: Pixels = px(5.);
const SOFT_WRAP_TEXT: &str = "Soft-Wrap";
const SOFT_WRAP_DESCRIPTION: &str = "Toggle using soft wraps in the current editor";
const SCROLL_TO_END_TEXT: &str = "Scroll to End";
const CLEAR_ALL_TEXT: &str = "Clear All";
const CLEAR_ALL_DESCRIPTION: &str = "Clear the contents of the console";
const CONTENT_TYPES: [GitConsoleContentType; 3] = [
    GitConsoleContentType::NormalOutput,
    GitConsoleContentType::ErrorOutput,
    GitConsoleContentType::SystemOutput,
];

fn content_type_index(content_type: GitConsoleContentType) -> usize {
    match content_type {
        GitConsoleContentType::NormalOutput => 0,
        GitConsoleContentType::ErrorOutput => 1,
        GitConsoleContentType::SystemOutput => 2,
    }
}

pub(crate) fn console_color(content_type: GitConsoleContentType, light: bool) -> Hsla {
    let color = match (content_type, light) {
        (GitConsoleContentType::ErrorOutput, false) => rgb(0xF75464),
        (GitConsoleContentType::NormalOutput | GitConsoleContentType::SystemOutput, false) => {
            rgb(0xBCBEC4)
        }
        (GitConsoleContentType::ErrorOutput, true) => rgb(0x7F0000),
        (GitConsoleContentType::NormalOutput, true) => rgb(0x000000),
        (GitConsoleContentType::SystemOutput, true) => rgb(0x00007F),
    };
    color.into()
}

pub(crate) fn line_length(line: &GitConsoleLine) -> usize {
    line.chunks
        .iter()
        .map(|chunk| chunk.text.len())
        .sum::<usize>()
        + 1
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ConsoleDocument {
    pub text: String,
    pub ranges: [Vec<Range<usize>>; 3],
}

pub(crate) fn console_document(lines: &[GitConsoleLine]) -> ConsoleDocument {
    let mut document = ConsoleDocument::default();
    for line in lines {
        let mut last_type = GitConsoleContentType::NormalOutput;
        for chunk in &line.chunks {
            let start = document.text.len();
            document.text.push_str(&chunk.text);
            let end = document.text.len();
            push_range(&mut document, chunk.content_type, start..end);
            last_type = chunk.content_type;
        }
        let start = document.text.len();
        document.text.push('\n');
        let end = document.text.len();
        push_range(&mut document, last_type, start..end);
    }
    document
}

fn push_range(
    document: &mut ConsoleDocument,
    content_type: GitConsoleContentType,
    range: Range<usize>,
) {
    if range.is_empty() {
        return;
    }
    if let Some(ranges) = document.ranges.get_mut(content_type_index(content_type)) {
        match ranges.last_mut() {
            Some(last) if last.end == range.start => last.end = range.end,
            _ => ranges.push(range),
        }
    }
}

pub(crate) fn kept_lines_start(shown: &[GitConsoleLine], lines: &[GitConsoleLine]) -> usize {
    (0..shown.len())
        .find(|start| {
            shown
                .get(*start..)
                .is_some_and(|kept| lines.get(..kept.len()) == Some(kept))
        })
        .unwrap_or(shown.len())
}

pub(crate) struct GitConsoleView {
    editor: Entity<Editor>,
    git_store: Entity<GitStore>,
    shown_lines: Vec<GitConsoleLine>,
    soft_wraps: bool,
    pending_soft_wraps_save: Task<()>,
}

impl GitConsoleView {
    pub(crate) fn new(
        git_store: Entity<GitStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let soft_wraps = persistence::console_soft_wraps(cx);
        let editor = cx.new(|cx| {
            let mut editor = Editor::multi_line(window, cx);
            editor.set_read_only(true);
            editor.set_input_enabled(false);
            editor.set_use_modal_editing(false);
            editor.set_show_gutter(false, cx);
            editor.set_show_indent_guides(false, cx);
            editor.set_show_wrap_guides(false, cx);
            editor.set_soft_wrap_mode(soft_wrap_mode(soft_wraps), cx);
            editor
        });
        let mut this = Self {
            editor,
            git_store,
            shown_lines: Vec::new(),
            soft_wraps,
            pending_soft_wraps_save: Task::ready(()),
        };
        this.sync(window, cx);
        this.scroll_to_end(window, cx);
        this
    }

    #[cfg(test)]
    pub(crate) fn text(&self, cx: &App) -> String {
        self.editor.read(cx).text(cx)
    }

    pub(crate) fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lines = self.git_store.read(cx).console_lines().to_vec();
        if lines == self.shown_lines {
            return;
        }
        let kept_start = kept_lines_start(&self.shown_lines, &lines);
        let removed_length: usize = self
            .shown_lines
            .get(..kept_start)
            .map(|removed| removed.iter().map(line_length).sum())
            .unwrap_or(0);
        let kept_count = self.shown_lines.len() - kept_start;
        let appended = console_document(lines.get(kept_count..).unwrap_or_default()).text;
        let document = console_document(&lines);
        let light = cx.theme().appearance().is_light();

        self.editor.update(cx, |editor, cx| {
            let display_snapshot = editor.display_snapshot(cx);
            let end = editor.buffer().read(cx).len(cx);
            let newest = editor
                .selections
                .newest::<MultiBufferOffset>(&display_snapshot);
            let caret_on_last_line = newest.head() >= end;
            let last_row = f64::from(display_snapshot.max_point().row().0);
            let scroll_top = editor.scroll_position(cx).y;
            let scrolled_to_bottom = editor
                .visible_line_count()
                .is_some_and(|visible_lines| scroll_top + visible_lines >= last_row);
            let stick_to_end = newest.is_empty() && (caret_on_last_line || scrolled_to_bottom);

            let mut edits: Vec<(Range<MultiBufferOffset>, String)> = Vec::new();
            if removed_length > 0 {
                edits.push((
                    MultiBufferOffset(0)..MultiBufferOffset(removed_length),
                    String::new(),
                ));
            }
            if !appended.is_empty() {
                edits.push((end..end, appended));
            }
            editor
                .buffer()
                .update(cx, |buffer, cx| buffer.edit(edits, None, cx));

            let snapshot = editor.buffer().read(cx).snapshot(cx);
            for content_type in CONTENT_TYPES {
                let index = content_type_index(content_type);
                let key = HighlightKey::ConsoleAnsiHighlight(index);
                let ranges = document
                    .ranges
                    .get(index)
                    .map(|ranges| {
                        ranges
                            .iter()
                            .map(|range| {
                                snapshot.anchor_after(MultiBufferOffset(range.start))
                                    ..snapshot.anchor_before(MultiBufferOffset(range.end))
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if ranges.is_empty() {
                    editor.clear_highlights(key, cx);
                } else {
                    editor.highlight_text(
                        key,
                        ranges,
                        HighlightStyle {
                            color: Some(console_color(content_type, light)),
                            ..HighlightStyle::default()
                        },
                        cx,
                    );
                }
            }
            if stick_to_end {
                editor.move_to_end(&MoveToEnd, window, cx);
            }
        });
        self.shown_lines = lines;
        cx.notify();
    }

    pub(crate) fn scroll_to_end(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            editor.move_to_end(&MoveToEnd, window, cx);
        });
    }

    pub(crate) fn toggle_soft_wraps(&mut self, cx: &mut Context<Self>) {
        self.soft_wraps = !self.soft_wraps;
        let mode = soft_wrap_mode(self.soft_wraps);
        self.editor
            .update(cx, |editor, cx| editor.set_soft_wrap_mode(mode, cx));
        self.pending_soft_wraps_save = persistence::save_console_soft_wraps(self.soft_wraps, cx);
        cx.notify();
    }

    pub(crate) fn clear(&mut self, cx: &mut Context<Self>) {
        self.git_store
            .update(cx, |git_store, cx| git_store.clear_console(cx));
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_content = !self.shown_lines.is_empty();
        v_flex()
            .flex_none()
            .w(TOOLBAR_WIDTH)
            .h_full()
            .py(TOOLBAR_VERTICAL_INSET)
            .items_center()
            .child(toolbar_slot(
                IconButton::new("git-console-soft-wrap", IconName::GeneralSoftWrap)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Medium)
                    .toggle_state(self.soft_wraps)
                    .tooltip(move |_, cx| {
                        Tooltip::with_meta(SOFT_WRAP_TEXT, None, SOFT_WRAP_DESCRIPTION, cx)
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_soft_wraps(cx))),
            ))
            .child(toolbar_slot(
                IconButton::new("git-console-scroll-to-end", IconName::GeneralScrollDown)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Medium)
                    .tooltip(Tooltip::text(SCROLL_TO_END_TEXT))
                    .on_click(cx.listener(|this, _, window, cx| this.scroll_to_end(window, cx))),
            ))
            .child(toolbar_slot(
                IconButton::new("git-console-clear-all", IconName::GeneralDelete)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Medium)
                    .disabled(!has_content)
                    .tooltip(move |_, cx| {
                        Tooltip::with_meta(CLEAR_ALL_TEXT, None, CLEAR_ALL_DESCRIPTION, cx)
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.clear(cx))),
            ))
    }
}

fn soft_wrap_mode(soft_wraps: bool) -> SoftWrap {
    if soft_wraps {
        SoftWrap::EditorWidth
    } else {
        SoftWrap::None
    }
}

fn toolbar_slot(button: IconButton) -> impl IntoElement {
    h_flex()
        .flex_none()
        .size(TOOLBAR_BUTTON_SIZE)
        .justify_center()
        .child(button)
}

impl Focusable for GitConsoleView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for GitConsoleView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .key_context(CONSOLE_KEY_CONTEXT)
            .size_full()
            .on_action(
                cx.listener(|this, _: &ToggleConsoleSoftWraps, _, cx| this.toggle_soft_wraps(cx)),
            )
            .on_action(cx.listener(|this, _: &ScrollConsoleToEnd, window, cx| {
                this.scroll_to_end(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ClearConsole, _, cx| this.clear(cx)))
            .child(self.render_toolbar(cx))
            .child(div().flex_1().min_w_0().h_full().child(self.editor.clone()))
    }
}
