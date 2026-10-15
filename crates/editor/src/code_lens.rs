use std::{
    iter,
    ops::{Range, RangeInclusive},
    sync::Arc,
};

use ::git::status::{FileStatus, StatusCode, TrackedStatus};
use buffer_diff::BufferDiffSnapshot;
use collections::HashMap;
use futures::{StreamExt as _, future::join_all, stream::FuturesUnordered};
use gpui::{Entity, Focusable as _, Font, Hsla, MouseButton, Task, TaskExt, WeakEntity, svg};
use itertools::Itertools;
use language::{BufferId, BufferSnapshot, ClientCommand, Node, point_from_lsp};
use multi_buffer::{
    Anchor, MultiBuffer, MultiBufferRow, MultiBufferSnapshot, RowInfo, ToPoint as _,
};
use project::{CodeAction, LspAction, Project, lsp_store::code_lens::CodeLensActions};
use serde::Deserialize;
use settings::Settings as _;
use text::{Bias, Point, ToOffset as _};
use theme_settings::ThemeSettings;
use ui::prelude::*;

use crate::{
    DisplayPoint, DisplayRow, Editor, EditorStyle, GitBlame, LSP_REQUEST_DEBOUNCE_TIMEOUT,
    actions::ToggleCodeLens,
    display_map::{DisplaySnapshot, ToDisplayPoint as _},
    element::inlay_hint_pill::inlay_hint_font_size,
    hover_links::{HoverLink, ShowUsagesOfLocations, UsageLocations, UsageLocationsKind},
};

pub(crate) const CODE_VISION_GAP: Pixels = px(5.);
const CODE_AUTHOR_ICON_GAP: Pixels = px(2.);
const CODE_AUTHOR_GROUP: &str = "code-vision-author";
const NEW_CODE_TEXT: &str = "new *";
const IMPLEMENTATIONS_TITLE_MARKER: &[u8] = b"implementation";
const REFERENCES_TITLE_MARKER: &[u8] = b"reference";
const MEMBER_DECLARATIONS_WITHOUT_CODE_VISION: [&str; 4] = [
    "property_signature",
    "public_field_definition",
    "field_definition",
    "enum_assignment",
];
const ENUM_BODY_KIND: &str = "enum_body";
const DECLARATIONS_WITH_CODE_AUTHOR: &[&str] = &[
    "class_declaration",
    "abstract_class_declaration",
    "class",
    "interface_declaration",
    "type_alias_declaration",
    "enum_declaration",
    "function_declaration",
    "generator_function_declaration",
    "method_definition",
    "method_signature",
    "abstract_method_signature",
    "variable_declarator",
    "lexical_declaration",
    "function_item",
    "function_signature_item",
    "struct_item",
    "enum_item",
    "trait_item",
    "impl_item",
    "type_item",
    "const_item",
    "static_item",
    "mod_item",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CodeVisionKind {
    Usages(usize),
    Implementations(usize),
    Other,
}

impl CodeVisionKind {
    fn for_title(title: &str) -> Self {
        let Some(count) = leading_count(title) else {
            return Self::Other;
        };
        if title_mentions(title, IMPLEMENTATIONS_TITLE_MARKER) {
            Self::Implementations(count)
        } else if title_mentions(title, REFERENCES_TITLE_MARKER) {
            Self::Usages(count)
        } else {
            Self::Other
        }
    }

    fn text(self, title: &str) -> Option<SharedString> {
        match self {
            Self::Usages(0) => Some(SharedString::new_static("no usages")),
            Self::Usages(1) => Some(SharedString::new_static("1 usage")),
            Self::Usages(count) => Some(format!("{count} usages").into()),
            Self::Implementations(0) => None,
            Self::Implementations(1) => Some(SharedString::new_static("1 implementation")),
            Self::Implementations(count) => Some(format!("{count} implementations").into()),
            Self::Other => Some(SharedString::from(title)),
        }
    }

    fn row_order(self) -> u8 {
        match self {
            Self::Usages(_) => 0,
            Self::Implementations(_) => 1,
            Self::Other => 2,
        }
    }
}

#[derive(Clone, Debug)]
struct CodeVisionEntry {
    text: SharedString,
    kind: CodeVisionKind,
    action: CodeAction,
}

#[derive(Clone, Debug)]
pub(crate) struct CodeVisionLine {
    position: Anchor,
    entries: Arc<[CodeVisionEntry]>,
    declaration: Option<Range<text::Anchor>>,
    authors: Option<CodeAuthorsCache>,
}

#[derive(Clone, Debug)]
struct CodeAuthorsCache {
    blame_epoch: usize,
    rows: RangeInclusive<u32>,
    authors: Option<CodeAuthors>,
    label: Option<CachedCodeAuthorLabel>,
}

#[derive(Clone, Debug)]
struct CachedCodeAuthorLabel {
    modified: bool,
    label: CodeAuthorLabel,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CodeAuthors {
    main_author: Option<SharedString>,
    other_authors: usize,
    has_uncommitted_rows: bool,
}

#[derive(Clone, Copy, Debug)]
struct DeclarationRow<'a> {
    blank: bool,
    author: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CodeAuthorLabel {
    text: SharedString,
    show_icon: bool,
}

pub(crate) struct VisibleCodeVisionLine {
    entries: Arc<[CodeVisionEntry]>,
    author: Option<CodeAuthorLabel>,
}

struct CodeAuthorContext {
    blame: Entity<GitBlame>,
    project: Entity<Project>,
    multi_buffer: Entity<MultiBuffer>,
    blame_epoch: usize,
}

pub(crate) struct CodeVisionStyle {
    font: Font,
    font_size: Pixels,
    line_height: Pixels,
    text_color: Hsla,
    hover_color: Hsla,
}

impl CodeVisionStyle {
    pub(crate) fn new(
        editor_style: &EditorStyle,
        line_height: Pixels,
        window: &Window,
        cx: &App,
    ) -> Self {
        let colors = cx.theme().colors();
        Self {
            font: ThemeSettings::get_global(cx).ui_font.clone(),
            font_size: inlay_hint_font_size(
                editor_style.text.font_size.to_pixels(window.rem_size()),
            ),
            line_height,
            text_color: colors
                .editor_code_lens_foreground
                .unwrap_or(colors.text_muted),
            hover_color: colors.link_text_hover,
        }
    }
}

pub(super) struct CodeLensState {
    lines: HashMap<BufferId, Vec<CodeVisionLine>>,
    actions: HashMap<BufferId, CodeLensActions>,
    resolve_task: Task<()>,
    blame_epoch: usize,
}

impl Default for CodeLensState {
    fn default() -> Self {
        Self {
            lines: HashMap::default(),
            actions: HashMap::default(),
            resolve_task: Task::ready(()),
            blame_epoch: 0,
        }
    }
}

impl CodeAuthorContext {
    fn new(
        blame: Entity<GitBlame>,
        project: Entity<Project>,
        multi_buffer: Entity<MultiBuffer>,
        blame_epoch: usize,
    ) -> Self {
        Self {
            blame,
            project,
            multi_buffer,
            blame_epoch,
        }
    }

    fn label(
        &self,
        line: &mut CodeVisionLine,
        buffer: &BufferSnapshot,
        multi_buffer_snapshot: &MultiBufferSnapshot,
        cx: &mut App,
    ) -> Option<CodeAuthorLabel> {
        let rows = declaration_rows(buffer, line.declaration.as_ref()?);
        let cache_is_current = line
            .authors
            .as_ref()
            .is_some_and(|cache| cache.blame_epoch == self.blame_epoch && cache.rows == rows);
        if !cache_is_current {
            line.authors = Some(CodeAuthorsCache {
                blame_epoch: self.blame_epoch,
                rows: rows.clone(),
                authors: self.authors(buffer, rows.clone(), cx),
                label: None,
            });
        }
        let cache = line.authors.as_mut()?;
        let authors = cache.authors.as_ref()?;
        let modified = authors.main_author.is_some()
            && self.is_modified(buffer, multi_buffer_snapshot, &rows, authors, cx);
        if cache
            .label
            .as_ref()
            .is_none_or(|cached| cached.modified != modified)
        {
            cache.label = Some(CachedCodeAuthorLabel {
                modified,
                label: code_author_label(authors, modified),
            });
        }
        cache.label.as_ref().map(|cached| cached.label.clone())
    }

    fn authors(
        &self,
        buffer: &BufferSnapshot,
        rows: RangeInclusive<u32>,
        cx: &mut App,
    ) -> Option<CodeAuthors> {
        let buffer_id = buffer.remote_id();
        let project = self.project.read(cx);
        let ignored = buffer.file().is_some_and(|file| {
            project
                .worktree_for_id(file.worktree_id(cx), cx)
                .is_some_and(|worktree| worktree.read(cx).is_path_ignored(file.path()))
        });
        if ignored {
            return None;
        }
        let git_store = project.git_store().read(cx);
        git_store.repository_and_path_for_buffer_id(buffer_id, cx)?;
        match git_store.status_for_buffer_id(buffer_id, cx) {
            Some(FileStatus::Untracked | FileStatus::Ignored) => return None,
            Some(FileStatus::Tracked(TrackedStatus {
                index_status: StatusCode::Added,
                ..
            })) => return Some(CodeAuthors::default()),
            _ => {}
        }
        if !self.blame.read(cx).has_generated_entries() {
            return None;
        }

        let row_infos = rows
            .clone()
            .map(|row| RowInfo {
                buffer_id: Some(buffer_id),
                buffer_row: Some(row),
                ..RowInfo::default()
            })
            .collect::<Vec<_>>();
        let row_authors = self.blame.update(cx, |blame, cx| {
            blame
                .blame_for_rows(&row_infos, cx)
                .map(|entry| entry.map(|(_, entry)| entry.author.unwrap_or_default()))
                .collect::<Vec<_>>()
        });
        Some(code_authors(rows.zip(&row_authors).map(|(row, author)| {
            DeclarationRow {
                blank: buffer.is_line_blank(row),
                author: author.as_deref(),
            }
        })))
    }

    fn is_modified(
        &self,
        buffer: &BufferSnapshot,
        multi_buffer_snapshot: &MultiBufferSnapshot,
        rows: &RangeInclusive<u32>,
        authors: &CodeAuthors,
        cx: &App,
    ) -> bool {
        match self.uncommitted_diff(buffer.remote_id(), multi_buffer_snapshot, cx) {
            Some(diff) => has_uncommitted_changes(diff, buffer, rows),
            None => authors.has_uncommitted_rows,
        }
    }

    fn uncommitted_diff<'snapshot>(
        &self,
        buffer_id: BufferId,
        multi_buffer_snapshot: &'snapshot MultiBufferSnapshot,
        cx: &App,
    ) -> Option<&'snapshot BufferDiffSnapshot> {
        let displayed_diff = self.multi_buffer.read(cx).diff_for(buffer_id)?;
        let uncommitted_diff = self
            .project
            .read(cx)
            .git_store()
            .read(cx)
            .get_uncommitted_diff(buffer_id, cx)?;
        if displayed_diff != uncommitted_diff {
            return None;
        }
        multi_buffer_snapshot.diff_for_buffer_id(buffer_id)
    }
}

fn declaration_rows(
    buffer: &BufferSnapshot,
    declaration: &Range<text::Anchor>,
) -> RangeInclusive<u32> {
    let start_row = buffer.summary_for_anchor::<Point>(&declaration.start).row;
    let end_row = buffer.summary_for_anchor::<Point>(&declaration.end).row;
    start_row..=end_row.max(start_row)
}

fn code_authors<'a>(rows: impl IntoIterator<Item = DeclarationRow<'a>>) -> CodeAuthors {
    let mut rows_by_author = HashMap::<&str, usize>::default();
    let mut has_uncommitted_rows = false;
    for row in rows {
        if row.blank {
            continue;
        }
        match row.author {
            None => has_uncommitted_rows = true,
            Some("") => {}
            Some(author) => *rows_by_author.entry(author).or_default() += 1,
        }
    }
    let main_author = rows_by_author
        .iter()
        .max_by(|(author, author_rows), (other_author, other_author_rows)| {
            author_rows
                .cmp(other_author_rows)
                .then_with(|| other_author.cmp(author))
        })
        .map(|(author, _)| SharedString::from(author.to_string()));
    CodeAuthors {
        main_author,
        other_authors: rows_by_author.len().saturating_sub(1),
        has_uncommitted_rows,
    }
}

fn code_author_label(authors: &CodeAuthors, modified: bool) -> CodeAuthorLabel {
    let Some(main_author) = authors.main_author.as_ref() else {
        return CodeAuthorLabel {
            text: SharedString::new_static(NEW_CODE_TEXT),
            show_icon: false,
        };
    };
    let text = match (authors.other_authors, modified) {
        (0, false) => main_author.clone(),
        (0, true) => format!("{main_author} *").into(),
        (other_authors, false) => format!("{main_author} +{other_authors}").into(),
        (other_authors, true) => format!("{main_author} +{other_authors} *").into(),
    };
    CodeAuthorLabel {
        text,
        show_icon: true,
    }
}

fn has_uncommitted_changes(
    diff: &BufferDiffSnapshot,
    buffer: &BufferSnapshot,
    rows: &RangeInclusive<u32>,
) -> bool {
    let start = buffer.anchor_before(Point::new(*rows.start(), 0));
    let end = buffer.anchor_after(Point::new(*rows.end(), buffer.line_len(*rows.end())));
    diff.raw_hunks_intersecting_range(start..end, buffer)
        .any(|hunk| hunk_changes_rows(&hunk.range, rows))
}

fn hunk_changes_rows(hunk: &Range<Point>, rows: &RangeInclusive<u32>) -> bool {
    if hunk.start == hunk.end {
        return *rows.start() < hunk.start.row && hunk.start.row <= *rows.end();
    }
    let end_row = if hunk.end.column == 0 {
        hunk.end.row
    } else {
        hunk.end.row.saturating_add(1)
    };
    hunk.start.row <= *rows.end() && *rows.start() < end_row
}

pub(super) fn try_handle_client_command(
    action: &CodeAction,
    editor: &mut Editor,
    workspace: &gpui::Entity<workspace::Workspace>,
    window: &mut Window,
    cx: &mut Context<Editor>,
) -> bool {
    try_handle_client_command_at(action, None, editor, workspace, window, cx)
}

fn try_handle_client_command_at(
    action: &CodeAction,
    click_position: Option<gpui::Point<Pixels>>,
    editor: &mut Editor,
    workspace: &gpui::Entity<workspace::Workspace>,
    window: &mut Window,
    cx: &mut Context<Editor>,
) -> bool {
    let Some(command) = action.lsp_action.command() else {
        return false;
    };

    let arguments = command.arguments.as_deref().unwrap_or_default();
    let project = workspace.read(cx).project().clone();
    let client_command = project
        .read(cx)
        .lsp_store()
        .read(cx)
        .language_server_adapter_for_id(action.server_id)
        .and_then(|adapter| adapter.adapter.client_command(&command.command, arguments))
        .or_else(|| match command.command.as_str() {
            "editor.action.showReferences"
            | "editor.action.goToLocations"
            | "editor.action.peekLocations" => Some(ClientCommand::ShowLocations),
            _ => None,
        });

    match client_command {
        Some(ClientCommand::ShowLocations) => try_show_references(
            &command.title,
            arguments,
            action,
            click_position,
            editor,
            window,
            cx,
        ),
        None => false,
    }
}

fn try_show_references(
    title: &str,
    arguments: &[serde_json::Value],
    action: &CodeAction,
    click_position: Option<gpui::Point<Pixels>>,
    editor: &mut Editor,
    window: &mut Window,
    cx: &mut Context<Editor>,
) -> bool {
    let Some(locations) = arguments
        .get(2)
        .and_then(|locations| serde_json::from_value::<Vec<lsp::Location>>(locations.clone()).ok())
    else {
        return false;
    };

    let usage_locations = UsageLocations {
        editor: cx.weak_entity(),
        kind: usage_locations_kind(title),
        symbol_position: lens_symbol_position(arguments, action, editor, cx),
        server_id: action.server_id,
        locations,
        popup_position: click_position
            .or_else(|| editor.popup_origin_below_cursor(window, cx))
            .unwrap_or_else(|| window.mouse_position()),
    };
    let Some(locations) = deliver_to_show_usages(usage_locations, editor, window, cx) else {
        return true;
    };
    if locations.is_empty() {
        return false;
    }

    let server_id = action.server_id;
    let nav_entry = editor.navigation_entry(editor.selections.newest_anchor().head(), cx);
    let links = locations
        .into_iter()
        .map(|location| HoverLink::LspLocation(location, server_id))
        .collect();
    editor
        .navigate_to_hover_links(None, links, nav_entry, false, window, cx)
        .detach_and_log_err(cx);

    true
}

fn deliver_to_show_usages(
    usage_locations: UsageLocations,
    editor: &Editor,
    window: &mut Window,
    cx: &mut Context<Editor>,
) -> Option<Vec<lsp::Location>> {
    let show_usages = ShowUsagesOfLocations(Arc::new(usage_locations));
    let focus_handle = editor.focus_handle(cx);
    if window.is_action_available_in(&show_usages, &focus_handle) {
        window.defer(cx, move |window, cx| {
            focus_handle.dispatch_action(&show_usages, window, cx);
        });
        return None;
    }
    Some(match Arc::try_unwrap(show_usages.0) {
        Ok(usage_locations) => usage_locations.locations,
        Err(usage_locations) => usage_locations.locations.clone(),
    })
}

fn usage_locations_kind(title: &str) -> UsageLocationsKind {
    if title_mentions(title, IMPLEMENTATIONS_TITLE_MARKER) {
        UsageLocationsKind::Implementations
    } else {
        UsageLocationsKind::References
    }
}

fn title_mentions(title: &str, marker: &[u8]) -> bool {
    title
        .as_bytes()
        .windows(marker.len())
        .any(|candidate| candidate.eq_ignore_ascii_case(marker))
}

fn leading_count(title: &str) -> Option<usize> {
    let title = title.trim_start();
    let digits_end = title
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(title.len());
    title.get(..digits_end)?.parse().ok()
}

fn lens_symbol_position(
    arguments: &[serde_json::Value],
    action: &CodeAction,
    editor: &Editor,
    cx: &App,
) -> Anchor {
    let multi_buffer = editor.buffer().read(cx);
    let snapshot = multi_buffer.snapshot(cx);
    arguments
        .get(1)
        .and_then(|argument| lsp::Position::deserialize(argument).ok())
        .and_then(|position| {
            let buffer_snapshot = multi_buffer
                .buffer(action.range.start.buffer_id)?
                .read(cx)
                .snapshot();
            let point = buffer_snapshot.clip_point_utf16(point_from_lsp(position), Bias::Left);
            snapshot.anchor_in_excerpt(buffer_snapshot.anchor_before(point))
        })
        .or_else(|| snapshot.anchor_in_excerpt(action.range.start))
        .unwrap_or_else(|| editor.selections.newest_anchor().head())
}

fn lens_title(action: &CodeAction) -> Option<&str> {
    match &action.lsp_action {
        LspAction::CodeLens(lens) => lens
            .command
            .as_ref()
            .map(|command| command.title.as_str())
            .filter(|title| !title.is_empty()),
        LspAction::Action(_) | LspAction::Command(_) => None,
    }
}

fn is_pending(action: &CodeAction) -> bool {
    lens_title(action).is_none() && !action.resolved
}

fn code_vision_entry(action: &CodeAction) -> Option<CodeVisionEntry> {
    let title = lens_title(action)?;
    let kind = CodeVisionKind::for_title(title);
    Some(CodeVisionEntry {
        text: kind.text(title)?,
        kind,
        action: action.clone(),
    })
}

fn is_declaration_lens(action: &CodeAction) -> bool {
    match lens_title(action) {
        Some(title) => matches!(
            CodeVisionKind::for_title(title),
            CodeVisionKind::Usages(_) | CodeVisionKind::Implementations(_)
        ),
        None => !action.resolved,
    }
}

fn lens_syntax_node<'a>(buffer: &'a BufferSnapshot, action: &CodeAction) -> Option<Node<'a>> {
    let offset = action.range.start.to_offset(buffer);
    buffer
        .syntax_layer_at(offset)
        .and_then(|layer| layer.node().named_descendant_for_byte_range(offset, offset))
}

fn is_member_without_code_vision(buffer: &BufferSnapshot, action: &CodeAction) -> bool {
    let Some(node) = lens_syntax_node(buffer, action) else {
        return false;
    };
    let declares_member_without_code_vision =
        |kind: &str| MEMBER_DECLARATIONS_WITHOUT_CODE_VISION.contains(&kind);
    declares_member_without_code_vision(node.kind())
        || node.parent().is_some_and(|parent| {
            parent.kind() == ENUM_BODY_KIND || declares_member_without_code_vision(parent.kind())
        })
}

fn declaration_range(buffer: &BufferSnapshot, action: &CodeAction) -> Range<text::Anchor> {
    lens_syntax_node(buffer, action)
        .and_then(|node| {
            iter::successors(Some(node), |node| node.parent())
                .find(|node| DECLARATIONS_WITH_CODE_AUTHOR.contains(&node.kind()))
        })
        .map(|declaration| {
            let start = buffer.clip_offset(declaration.start_byte(), Bias::Left);
            let end = buffer.clip_offset(declaration.end_byte(), Bias::Left);
            buffer.anchor_after(start)..buffer.anchor_before(end)
        })
        .unwrap_or_else(|| action.range.clone())
}

fn lens_rows<'a>(
    actions: &'a CodeLensActions,
    snapshot: &MultiBufferSnapshot,
) -> Vec<(MultiBufferRow, Anchor, Vec<&'a CodeAction>)> {
    actions
        .iter()
        .sorted_by_key(|(id, _)| **id)
        .filter(|(_, action)| matches!(action.lsp_action, LspAction::CodeLens(_)))
        .filter_map(|(_, action)| Some((snapshot.anchor_in_excerpt(action.range.start)?, action)))
        .into_group_map_by(|(position, _)| MultiBufferRow(position.to_point(snapshot).row))
        .into_iter()
        .sorted_by_key(|(row, _)| *row)
        .filter_map(|(row, row_lenses)| {
            let position = row_lenses.first()?.0;
            let row_actions = row_lenses.into_iter().map(|(_, action)| action).collect();
            Some((row, position, row_actions))
        })
        .collect()
}

pub(crate) fn render_code_vision_line(
    line: &VisibleCodeVisionLine,
    display_row: DisplayRow,
    style: &CodeVisionStyle,
    editor: &WeakEntity<Editor>,
) -> AnyElement {
    let hover_color = style.hover_color;
    h_flex()
        .id(("code-vision", display_row.0))
        .block_mouse_except_scroll()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .h(style.line_height)
        .line_height(style.line_height)
        .gap(style.font_size)
        .font(style.font.clone())
        .text_size(style.font_size)
        .text_color(style.text_color)
        .children(line.entries.iter().enumerate().map(|(index, entry)| {
            let entries = line.entries.clone();
            let editor = editor.clone();
            div()
                .id(index)
                .cursor_pointer()
                .hover(move |hovered| hovered.text_color(hover_color).underline())
                .on_click(move |event, window, cx| {
                    let (Some(entry), Some(editor)) = (entries.get(index), editor.upgrade()) else {
                        return;
                    };
                    editor.update(cx, |editor, cx| {
                        editor.activate_code_vision_entry(entry, event.position(), window, cx);
                    });
                })
                .child(entry.text.clone())
        }))
        .children(
            line.author
                .as_ref()
                .map(|author| render_code_author(author, display_row, style, editor)),
        )
        .into_any_element()
}

fn render_code_author(
    author: &CodeAuthorLabel,
    display_row: DisplayRow,
    style: &CodeVisionStyle,
    editor: &WeakEntity<Editor>,
) -> impl IntoElement {
    let hover_color = style.hover_color;
    let editor = editor.clone();
    h_flex()
        .id("code-author")
        .debug_selector(move || format!("code_vision_author_{}", display_row.0))
        .group(CODE_AUTHOR_GROUP)
        .gap(CODE_AUTHOR_ICON_GAP)
        .cursor_pointer()
        .hover(move |hovered| hovered.text_color(hover_color).underline())
        .on_click(move |_, window, cx| {
            let Some(editor) = editor.upgrade() else {
                return;
            };
            let focus_handle = editor.read(cx).focus_handle(cx);
            window.defer(cx, move |window, cx| {
                focus_handle.dispatch_action(&::git::Blame, window, cx);
            });
        })
        .when(author.show_icon, |entry| {
            entry.child(
                svg()
                    .path(IconName::VcsAuthor.path())
                    .flex_none()
                    .size(style.font_size)
                    .text_color(style.text_color)
                    .group_hover(CODE_AUTHOR_GROUP, move |icon| icon.text_color(hover_color)),
            )
        })
        .child(author.text.clone())
}

impl Editor {
    pub(super) fn refresh_code_lenses(
        &mut self,
        for_buffer: Option<BufferId>,
        _window: &Window,
        cx: &mut Context<Self>,
    ) {
        if !self.lsp_data_enabled() || self.code_lens.is_none() {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };

        let buffers_to_query = self
            .visible_buffers(cx)
            .into_iter()
            .filter(|buffer| self.is_lsp_relevant(buffer.read(cx).file(), cx))
            .chain(for_buffer.and_then(|buffer_id| self.buffer.read(cx).buffer(buffer_id)))
            .filter(|editor_buffer| {
                let editor_buffer_id = editor_buffer.read(cx).remote_id();
                for_buffer.is_none_or(|buffer_id| buffer_id == editor_buffer_id)
                    && self.registered_buffers.contains_key(&editor_buffer_id)
            })
            .unique_by(|buffer| buffer.read(cx).remote_id())
            .collect::<Vec<_>>();

        if buffers_to_query.is_empty() {
            return;
        }

        let project = project.downgrade();
        self.refresh_code_lens_task = cx.spawn(async move |editor, cx| {
            cx.background_executor()
                .timer(LSP_REQUEST_DEBOUNCE_TIMEOUT)
                .await;

            let Some(tasks_per_buffer) = project
                .update(cx, |project, cx| {
                    project.lsp_store().update(cx, |lsp_store, cx| {
                        buffers_to_query
                            .into_iter()
                            .map(|buffer| {
                                let buffer_id = buffer.read(cx).remote_id();
                                let task = lsp_store.code_lens_actions(&buffer, cx);
                                async move { (buffer_id, task.await) }
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .ok()
            else {
                return;
            };

            let code_lens_per_buffer = join_all(tasks_per_buffer).await;
            if code_lens_per_buffer.is_empty() {
                return;
            }

            editor
                .update(cx, |editor, cx| {
                    let snapshot = editor.buffer().read(cx).snapshot(cx);
                    for (buffer_id, result) in code_lens_per_buffer {
                        let actions = match result {
                            Ok(Some(actions)) => actions,
                            Ok(None) => continue,
                            Err(e) => {
                                log::error!(
                                    "Failed to fetch code lenses for buffer {buffer_id:?}: {e:#}"
                                );
                                continue;
                            }
                        };
                        editor.apply_lens_actions_for_buffer(buffer_id, actions, &snapshot, cx);
                    }
                    editor.resolve_visible_code_lenses(cx);
                })
                .ok();
            editor
                .update_in(cx, |editor, window, cx| {
                    if editor.has_code_vision_declarations() {
                        editor.ensure_git_blame_for_code_vision(window, cx);
                    }
                })
                .ok();
        });
    }

    fn apply_lens_actions_for_buffer(
        &mut self,
        buffer_id: BufferId,
        mut actions: CodeLensActions,
        snapshot: &MultiBufferSnapshot,
        cx: &mut Context<Self>,
    ) {
        if let Some(buffer_snapshot) = snapshot.buffer_for_id(buffer_id) {
            actions.retain(|_, action| !is_member_without_code_vision(buffer_snapshot, action));
        }

        let code_lens = self.code_lens.get_or_insert_with(CodeLensState::default);
        let mut previous_lines_by_row = code_lens
            .lines
            .remove(&buffer_id)
            .unwrap_or_default()
            .into_iter()
            .map(|line| (MultiBufferRow(line.position.to_point(snapshot).row), line))
            .collect::<HashMap<_, _>>();

        let buffer_snapshot = snapshot.buffer_for_id(buffer_id);
        let mut lines = Vec::new();
        for (row, position, row_actions) in lens_rows(&actions, snapshot) {
            let mut previous_line = previous_lines_by_row.remove(&row);
            if row_actions.iter().all(|action| is_pending(action))
                && let Some(previous_line) = previous_line.take()
            {
                lines.push(previous_line);
                continue;
            }
            let declaration = row_actions
                .iter()
                .find(|action| is_declaration_lens(action))
                .zip(buffer_snapshot)
                .map(|(action, buffer)| declaration_range(buffer, action));
            let mut entries = row_actions
                .into_iter()
                .filter_map(code_vision_entry)
                .collect::<Vec<_>>();
            if entries.is_empty() && declaration.is_none() {
                continue;
            }
            entries.sort_by_key(|entry| entry.kind.row_order());
            lines.push(CodeVisionLine {
                position,
                entries: entries.into(),
                declaration,
                authors: previous_line.and_then(|line| line.authors),
            });
        }

        if lines.is_empty() {
            code_lens.lines.remove(&buffer_id);
        } else {
            code_lens.lines.insert(buffer_id, lines);
        }
        if actions.is_empty() {
            code_lens.actions.remove(&buffer_id);
        } else {
            code_lens.actions.insert(buffer_id, actions);
        }
        cx.notify();
    }

    pub(crate) fn code_vision_lines_in_rows(
        &mut self,
        display_snapshot: &DisplaySnapshot,
        rows: Range<DisplayRow>,
        cx: &mut Context<Self>,
    ) -> Vec<(DisplayRow, VisibleCodeVisionLine)> {
        let blame = self.blame.clone();
        let project = self.project.clone();
        let multi_buffer = self.buffer.clone();
        let Some(code_lens) = self.code_lens.as_mut() else {
            return Vec::new();
        };
        if rows.is_empty() || code_lens.lines.is_empty() {
            return Vec::new();
        }

        let buffer_snapshot = display_snapshot.buffer_snapshot();
        let last_display_row = rows.end.min(display_snapshot.max_point().row());
        let first_buffer_row = DisplayPoint::new(rows.start, 0)
            .to_point(display_snapshot)
            .row;
        let last_buffer_row = DisplayPoint::new(last_display_row, 0)
            .to_point(display_snapshot)
            .row;

        let mut lines_by_display_row = HashMap::default();
        for (buffer_id, lines) in &code_lens.lines {
            let first_candidate = lines.partition_point(|line| {
                line.position.to_point(buffer_snapshot).row < first_buffer_row
            });
            for (index, line) in lines.iter().enumerate().skip(first_candidate) {
                let point = line.position.to_point(buffer_snapshot);
                if point.row > last_buffer_row {
                    break;
                }
                let hidden_by_fold = point
                    .to_display_point(display_snapshot)
                    .to_point(display_snapshot)
                    != point;
                if hidden_by_fold {
                    continue;
                }
                let display_row = display_snapshot.next_line_boundary(point).1.row();
                if rows.contains(&display_row) {
                    lines_by_display_row
                        .entry(display_row)
                        .or_insert((*buffer_id, index));
                }
            }
        }

        let author_context = blame.zip(project).map(|(blame, project)| {
            CodeAuthorContext::new(blame, project, multi_buffer, code_lens.blame_epoch)
        });
        lines_by_display_row
            .into_iter()
            .filter_map(|(display_row, (buffer_id, index))| {
                let line = code_lens.lines.get_mut(&buffer_id)?.get_mut(index)?;
                let author = author_context
                    .as_ref()
                    .zip(buffer_snapshot.buffer_for_id(buffer_id))
                    .and_then(|(author_context, buffer)| {
                        author_context.label(line, buffer, buffer_snapshot, cx)
                    });
                if line.entries.is_empty() && author.is_none() {
                    return None;
                }
                Some((
                    display_row,
                    VisibleCodeVisionLine {
                        entries: line.entries.clone(),
                        author,
                    },
                ))
            })
            .collect()
    }

    fn has_code_vision_declarations(&self) -> bool {
        self.code_lens.as_ref().is_some_and(|code_lens| {
            code_lens
                .lines
                .values()
                .flatten()
                .any(|line| line.declaration.is_some())
        })
    }

    pub(super) fn invalidate_code_vision_authors(&mut self) {
        if let Some(code_lens) = self.code_lens.as_mut() {
            code_lens.blame_epoch = code_lens.blame_epoch.wrapping_add(1);
        }
    }

    fn activate_code_vision_entry(
        &mut self,
        entry: &CodeVisionEntry,
        click_position: gpui::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if entry.kind == CodeVisionKind::Usages(0) {
            let no_usages = UsageLocations {
                editor: cx.weak_entity(),
                kind: UsageLocationsKind::References,
                symbol_position: lens_symbol_position(&[], &entry.action, self, cx),
                server_id: entry.action.server_id,
                locations: Vec::new(),
                popup_position: click_position,
            };
            deliver_to_show_usages(no_usages, self, window, cx);
            return;
        }

        let Some(workspace) = self.workspace() else {
            return;
        };
        if try_handle_client_command_at(
            &entry.action,
            Some(click_position),
            self,
            &workspace,
            window,
            cx,
        ) {
            return;
        }

        let Some(buffer) = self
            .buffer()
            .read(cx)
            .buffer(entry.action.range.start.buffer_id)
        else {
            return;
        };
        let project = workspace.read(cx).project().clone();
        let action = entry.action.clone();
        project
            .update(cx, |project, cx| {
                project.apply_code_action(buffer, action, true, cx)
            })
            .detach_and_log_err(cx);
    }

    pub fn supports_code_lens(&self, cx: &ui::App) -> bool {
        let Some(project) = self.project.as_ref() else {
            return false;
        };
        let lsp_store = project.read(cx).lsp_store().read(cx);
        lsp_store
            .lsp_server_capabilities
            .values()
            .any(|caps| caps.code_lens_provider.is_some())
    }

    pub fn code_lens_enabled(&self) -> bool {
        self.code_lens.is_some()
    }

    pub fn toggle_code_lens_action(
        &mut self,
        _: &ToggleCodeLens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let currently_enabled = self.code_lens.is_some();
        self.toggle_code_lens(!currently_enabled, window, cx);
    }

    pub(super) fn toggle_code_lens(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if enabled {
            self.code_lens.get_or_insert_with(CodeLensState::default);
            self.refresh_code_lenses(None, window, cx);
        } else {
            self.clear_code_lenses(cx);
        }
    }

    pub(super) fn resolve_visible_code_lenses(&mut self, cx: &mut Context<Self>) {
        if !self.lsp_data_enabled() || self.code_lens.is_none() {
            return;
        }
        let Some(project) = self.project.clone() else {
            return;
        };

        let lsp_store = project.read(cx).lsp_store();

        let mut pending_resolves = Vec::new();
        for (buffer_snapshot, visible_range, _) in self.visible_buffer_ranges(cx) {
            let buffer_id = buffer_snapshot.remote_id();
            let Some(buffer) = self.buffer.read(cx).buffer(buffer_id) else {
                continue;
            };
            let Some(actions) = self
                .code_lens
                .as_ref()
                .and_then(|state| state.actions.get(&buffer_id))
            else {
                continue;
            };
            for (lens_id, action) in actions {
                if action.resolved {
                    continue;
                }
                if let project::LspAction::CodeLens(lens) = &action.lsp_action {
                    if lens.command.is_some() {
                        continue;
                    }
                }
                let action_offset = action.range.start.to_offset(&buffer_snapshot);
                if action_offset < visible_range.start.0 || action_offset > visible_range.end.0 {
                    continue;
                }
                let resolve_task = lsp_store.update(cx, |lsp_store, cx| {
                    lsp_store.resolve_code_lens(&buffer, action.server_id, *lens_id, cx)
                });
                pending_resolves.push((buffer_id, resolve_task));
            }
        }
        if pending_resolves.is_empty() {
            return;
        }

        let code_lens = self.code_lens.get_or_insert_with(CodeLensState::default);
        code_lens.resolve_task = cx.spawn(async move |editor, cx| {
            let mut resolves_in_progress = pending_resolves
                .into_iter()
                .map(|(buffer_id, task)| async move { (buffer_id, task.await) })
                .collect::<FuturesUnordered<_>>();
            while let Some((buffer_id, resolve_result)) = resolves_in_progress.next().await {
                let Some((resolved_id, resolved)) = resolve_result else {
                    continue;
                };
                editor
                    .update(cx, |editor, cx| {
                        let snapshot = editor.buffer().read(cx).snapshot(cx);
                        let Some(mut actions) = editor
                            .code_lens
                            .as_ref()
                            .and_then(|state| state.actions.get(&buffer_id))
                            .cloned()
                        else {
                            return;
                        };
                        if let Some(slot) = actions.get_mut(&resolved_id) {
                            *slot = resolved;
                        }
                        editor.apply_lens_actions_for_buffer(buffer_id, actions, &snapshot, cx);
                    })
                    .ok();
            }
        });
    }

    pub(super) fn clear_code_lenses(&mut self, cx: &mut Context<Self>) {
        if self.code_lens.take().is_some() {
            cx.notify();
        }
        self.refresh_code_lens_task = Task::ready(());
    }

    pub(super) fn remove_code_lenses_for_buffers(
        &mut self,
        buffer_ids: &[BufferId],
        cx: &mut Context<Self>,
    ) {
        let Some(code_lens) = self.code_lens.as_mut() else {
            return;
        };
        for buffer_id in buffer_ids {
            code_lens.lines.remove(buffer_id);
            code_lens.actions.remove(buffer_id);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use std::{ops::Range, sync::Arc, time::Duration};

    use ::git::{
        blame::{Blame, BlameEntry},
        repository::repo_path,
    };
    use collections::{HashMap, HashSet};
    use futures::{StreamExt, channel::mpsc::UnboundedReceiver};
    use gpui::{
        Entity, Modifiers, Pixels, TestAppContext, UpdateGlobal as _, VisualTestContext, px,
    };
    use indoc::indoc;
    use parking_lot::Mutex;
    use settings::{CodeLens, SettingsStore};
    use util::path;

    use multi_buffer::{MultiBufferRow, ToPoint as _};
    use text::Point;

    use super::{
        CODE_VISION_GAP, CodeVisionKind, DeclarationRow, code_author_label, code_authors,
        hunk_changes_rows,
    };
    use crate::{
        DisplayPoint, DisplayRow, Editor, LSP_REQUEST_DEBOUNCE_TIMEOUT,
        display_map::ToDisplayPoint as _,
        editor_tests::{init_test, update_test_editor_settings},
        hover_links::{ShowUsagesOfLocations, UsageLocations, UsageLocationsKind},
        test::editor_lsp_test_context::EditorLspTestContext,
    };

    const SHOW_USAGES_LENS_SOURCE: &str = indoc! {"
        ˇfunction hello() {}
        const first = 1;
        const second = 2;
        const third = 3;
        const fourth = 4;
        const fifth = 5;
        const sixth = 6;
        const seventh = 7;
        const eighth = 8;
        const ninth = 9;
        hello();
    "};

    const MEMBER_LENS_SOURCE: &str = indoc! {"
        ˇinterface QaseReporterOptions {
          readonly qaseCode: string;
          qaseUrl?: string;
        }
        class QaseReporter {
          private options: QaseReporterOptions;
          constructor(options: QaseReporterOptions) {
            this.options = options;
          }
          report(): void {}
        }
        enum Status {
          Passed,
          Failed = 1,
        }
    "};

    const MEMBER_LENSES: [(u32, u32, &str, &str); 11] = [
        (0, 10, "references", "1 reference"),
        (0, 10, "implementations", "0 implementations"),
        (1, 11, "references", "3 references"),
        (2, 2, "references", "1 reference"),
        (4, 6, "references", "0 references"),
        (5, 10, "references", "2 references"),
        (6, 2, "references", "0 references"),
        (9, 2, "references", "2 references"),
        (11, 5, "references", "1 reference"),
        (12, 2, "references", "4 references"),
        (13, 2, "references", "1 reference"),
    ];

    const CODE_AUTHOR_SOURCE: &str = indoc! {"
        ˇinterface QaseReporterOptions {
          readonly qaseCode: string;
        }

        class QaseReporter {
          constructor(private readonly options: QaseReporterOptions) {

            this.options.qaseCode;
          }
        }
    "};

    const CODE_AUTHOR_LENSES: [(u32, u32, &str); 4] = [
        (0, 10, "1 reference"),
        (1, 11, "3 references"),
        (4, 6, "0 references"),
        (5, 2, "0 references"),
    ];

    const MAIN_AUTHOR: &str = "Oleksandr Kreminskyi";

    async fn show_usages_lens_context(
        cx: &mut TestAppContext,
    ) -> (EditorLspTestContext, Vec<lsp::Location>) {
        init_test(cx, |_| {});
        update_test_editor_settings(cx, &|settings| {
            settings.code_lens = Some(CodeLens::On);
        });

        let mut cx = EditorLspTestContext::new_typescript(
            lsp::ServerCapabilities {
                code_lens_provider: Some(lsp::CodeLensOptions {
                    resolve_provider: None,
                }),
                ..lsp::ServerCapabilities::default()
            },
            cx,
        )
        .await;

        let uri = cx.buffer_lsp_url.clone();
        let usage_locations = vec![
            lsp::Location {
                uri: uri.clone(),
                range: lsp::Range::new(lsp::Position::new(0, 9), lsp::Position::new(0, 14)),
            },
            lsp::Location {
                uri: uri.clone(),
                range: lsp::Range::new(lsp::Position::new(10, 0), lsp::Position::new(10, 5)),
            },
        ];
        let lens_arguments = vec![
            serde_json::to_value(&uri).unwrap(),
            serde_json::to_value(lsp::Position::new(0, 9)).unwrap(),
            serde_json::to_value(&usage_locations).unwrap(),
        ];
        let mut code_lens_request =
            cx.set_request_handler::<lsp::request::CodeLensRequest, _, _>(move |_, _, _| {
                let lens_arguments = lens_arguments.clone();
                async move {
                    Ok(Some(vec![lsp::CodeLens {
                        range: lsp::Range::new(lsp::Position::new(0, 9), lsp::Position::new(0, 14)),
                        command: Some(lsp::Command {
                            title: "2 references".to_owned(),
                            command: "editor.action.showReferences".to_owned(),
                            arguments: Some(lens_arguments),
                        }),
                        data: None,
                    }]))
                }
            });

        cx.set_state(SHOW_USAGES_LENS_SOURCE);
        assert!(
            code_lens_request.next().await.is_some(),
            "should have received a code lens request"
        );
        cx.run_until_parked();
        cx.editor(|editor, _, cx| {
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                SHOW_USAGES_LENS_SOURCE.replace("ˇ", "").replacen(
                    "function hello() {}",
                    "function hello() {}  [2 usages]",
                    1
                ),
                "the references lens should render as a usages hint after the declaration line"
            );
        });
        (cx, usage_locations)
    }

    fn record_show_usages(cx: &mut EditorLspTestContext) -> Arc<Mutex<Vec<Arc<UsageLocations>>>> {
        let dispatched: Arc<Mutex<Vec<Arc<UsageLocations>>>> = Arc::default();
        cx.update_editor(|editor, _, cx| {
            let dispatched = dispatched.clone();
            editor
                .register_action(move |action: &ShowUsagesOfLocations, _, _| {
                    dispatched.lock().push(action.0.clone());
                })
                .detach();
            cx.notify();
        });
        cx.run_until_parked();
        dispatched
    }

    fn code_vision_click_position(cx: &mut EditorLspTestContext, row: u32) -> gpui::Point<Pixels> {
        let line_end = cx.update_editor(|editor, window, cx| {
            let snapshot = editor.snapshot(window, cx);
            let line_len = snapshot.buffer_snapshot().line_len(MultiBufferRow(row));
            Point::new(row, line_len).to_display_point(&snapshot)
        });
        let line_end_position = cx.pixel_position_for(line_end);
        gpui::point(
            line_end_position.x + CODE_VISION_GAP + px(10.),
            line_end_position.y,
        )
    }

    fn click_code_vision(cx: &mut EditorLspTestContext, row: u32) -> gpui::Point<Pixels> {
        let click_position = code_vision_click_position(cx, row);
        cx.simulate_mouse_move(click_position, None, Modifiers::none());
        cx.simulate_click(click_position, Modifiers::none());
        cx.run_until_parked();
        click_position
    }

    #[test]
    fn test_code_vision_texts_follow_intellij_usages_telescope() {
        let expectations = [
            ("0 references", Some("no usages")),
            ("1 reference", Some("1 usage")),
            ("2 references", Some("2 usages")),
            ("17 references", Some("17 usages")),
            ("0 implementations", None),
            ("1 implementation", Some("1 implementation")),
            ("3 implementations", Some("3 implementations")),
            ("▶ Run", Some("▶ Run")),
            ("Debug", Some("Debug")),
            ("2 tests", Some("2 tests")),
        ];
        for (title, expected_text) in expectations {
            let text = CodeVisionKind::for_title(title).text(title);
            assert_eq!(
                text.as_ref().map(|text| text.as_str()),
                expected_text,
                "unexpected code vision text for lens title {title:?}"
            );
        }
    }

    fn committed(author: &str) -> DeclarationRow<'_> {
        DeclarationRow {
            blank: false,
            author: Some(author),
        }
    }

    fn uncommitted() -> DeclarationRow<'static> {
        DeclarationRow {
            blank: false,
            author: None,
        }
    }

    fn blank(author: Option<&str>) -> DeclarationRow<'_> {
        DeclarationRow {
            blank: true,
            author,
        }
    }

    fn code_author_text(rows: &[DeclarationRow<'_>], modified: bool) -> (String, bool) {
        let label = code_author_label(&code_authors(rows.iter().copied()), modified);
        (label.text.to_string(), label.show_icon)
    }

    #[test]
    fn test_code_vision_author_shows_single_author() {
        assert_eq!(
            code_author_text(&[committed(MAIN_AUTHOR), committed(MAIN_AUTHOR)], false),
            (MAIN_AUTHOR.to_string(), true),
            "code written by one author should show the full blame name after the person icon"
        );
    }

    #[test]
    fn test_code_vision_author_most_frequent_author_wins() {
        assert_eq!(
            code_author_text(
                &[
                    committed("Alice Example"),
                    committed(MAIN_AUTHOR),
                    committed(MAIN_AUTHOR),
                ],
                false
            ),
            (format!("{MAIN_AUTHOR} +1"), true),
            "the author of most rows should be the main author"
        );
    }

    #[test]
    fn test_code_vision_author_tie_breaks_alphabetically() {
        assert_eq!(
            code_author_text(
                &[committed("Bob Example"), committed("Alice Example")],
                false
            ),
            ("Alice Example +1".to_string(), true),
            "authors with the same row count should resolve to the alphabetically smallest name"
        );
        assert_eq!(
            code_author_text(
                &[
                    committed("Bob Example"),
                    committed("Bob Example"),
                    committed("Aaron Example"),
                    committed("Carol Example"),
                    committed("Carol Example"),
                ],
                false
            ),
            ("Bob Example +2".to_string(), true),
            "the tie break should only choose among the most frequent authors"
        );
    }

    #[test]
    fn test_code_vision_author_counts_other_authors() {
        assert_eq!(
            code_author_text(
                &[
                    committed(MAIN_AUTHOR),
                    committed(MAIN_AUTHOR),
                    committed(MAIN_AUTHOR),
                    committed("Alice Example"),
                    committed("Bob Example"),
                    committed("Alice Example"),
                ],
                false
            ),
            (format!("{MAIN_AUTHOR} +2"), true),
            "the count after the main author should be the number of distinct other authors"
        );
    }

    #[test]
    fn test_code_vision_author_marks_modified_code() {
        assert_eq!(
            code_author_text(&[committed(MAIN_AUTHOR)], true),
            (format!("{MAIN_AUTHOR} *"), true),
            "modified code by one author should end with an asterisk"
        );
        assert_eq!(
            code_author_text(
                &[
                    committed(MAIN_AUTHOR),
                    committed(MAIN_AUTHOR),
                    committed("Alice Example"),
                ],
                true
            ),
            (format!("{MAIN_AUTHOR} +1 *"), true),
            "modified code by several authors should end with an asterisk after the count"
        );
    }

    #[test]
    fn test_code_vision_author_new_code_without_authors() {
        let cases = [
            (Vec::new(), "a declaration without rows"),
            (
                vec![uncommitted(), uncommitted()],
                "a declaration made only of uncommitted rows",
            ),
            (
                vec![committed("")],
                "a declaration whose blame has no author name",
            ),
        ];
        for (rows, description) in cases {
            for modified in [false, true] {
                assert_eq!(
                    code_author_text(&rows, modified),
                    ("new *".to_string(), false),
                    "{description} should read as new code without the person icon"
                );
            }
        }
        assert!(
            code_authors([uncommitted(), committed(MAIN_AUTHOR)]).has_uncommitted_rows,
            "rows without blame should mark the declaration as changed when no diff is loaded"
        );
    }

    #[test]
    fn test_code_vision_author_ignores_blank_rows() {
        let rows = [
            committed(MAIN_AUTHOR),
            blank(Some("Alice Example")),
            blank(None),
            committed(MAIN_AUTHOR),
        ];
        assert_eq!(
            code_author_text(&rows, false),
            (MAIN_AUTHOR.to_string(), true),
            "blank rows should neither choose the main author nor count as other authors"
        );
        assert!(
            !code_authors(rows).has_uncommitted_rows,
            "a blank row without blame should not mark the declaration as changed"
        );
    }

    #[test]
    fn test_code_vision_author_modified_only_by_hunks_inside_declaration() {
        let declaration = 4..=9;
        let cases = [
            (
                Point::new(6, 0)..Point::new(7, 0),
                true,
                "a changed row inside",
            ),
            (
                Point::new(4, 0)..Point::new(5, 0),
                true,
                "a changed first row",
            ),
            (
                Point::new(9, 0)..Point::new(10, 0),
                true,
                "a changed last row",
            ),
            (
                Point::new(9, 0)..Point::new(9, 1),
                true,
                "a changed last row without a trailing newline",
            ),
            (
                Point::new(2, 0)..Point::new(4, 0),
                false,
                "rows changed right above",
            ),
            (
                Point::new(10, 0)..Point::new(12, 0),
                false,
                "rows changed right below",
            ),
            (
                Point::new(6, 0)..Point::new(6, 0),
                true,
                "rows deleted inside",
            ),
            (
                Point::new(4, 0)..Point::new(4, 0),
                false,
                "rows deleted right above",
            ),
            (
                Point::new(10, 0)..Point::new(10, 0),
                false,
                "rows deleted right below",
            ),
        ];
        for (hunk, expected, description) in cases {
            assert_eq!(
                hunk_changes_rows(&hunk, &declaration),
                expected,
                "{description} the declaration rows {declaration:?}"
            );
        }
    }

    fn authored_blame_entry(sha: &str, rows: Range<u32>, author: &str) -> BlameEntry {
        BlameEntry {
            sha: sha.parse().unwrap(),
            range: rows,
            original_line_number: 0,
            author: Some(author.to_string()),
            author_mail: None,
            author_time: None,
            author_tz: None,
            committer_name: None,
            committer_email: None,
            committer_time: None,
            committer_tz: None,
            summary: None,
            previous: None,
            filename: "dir/file.ts".to_string(),
            boundary: false,
        }
    }

    async fn code_author_context(
        cx: &mut TestAppContext,
    ) -> (EditorLspTestContext, UnboundedReceiver<()>) {
        init_test(cx, |_| {});
        update_test_editor_settings(cx, &|settings| {
            settings.code_lens = Some(CodeLens::On);
        });
        cx.update(|cx| {
            SettingsStore::update_global(cx, |store, cx| {
                store.update_user_settings(cx, |settings| {
                    settings
                        .git
                        .get_or_insert_default()
                        .inline_blame
                        .get_or_insert_default()
                        .enabled = Some(false);
                });
            });
        });

        let mut cx = EditorLspTestContext::new_typescript(
            lsp::ServerCapabilities {
                code_lens_provider: Some(lsp::CodeLensOptions {
                    resolve_provider: None,
                }),
                ..lsp::ServerCapabilities::default()
            },
            cx,
        )
        .await;

        let fs = cx.update_editor(|editor, _, cx| {
            editor
                .project()
                .expect("the code author test editor should belong to a project")
                .read(cx)
                .fs()
                .as_fake()
        });
        fs.set_blame_for_repo(
            &EditorLspTestContext::root_path().join(".git"),
            vec![(
                repo_path("dir/file.ts"),
                Blame {
                    entries: vec![
                        authored_blame_entry("1b1b1b", 0..6, MAIN_AUTHOR),
                        authored_blame_entry("2c2c2c", 6..7, "Alice Example"),
                        authored_blame_entry("1b1b1b", 7..11, MAIN_AUTHOR),
                    ],
                    ..Blame::default()
                },
            )],
        );

        let mut code_lens_request =
            cx.set_request_handler::<lsp::request::CodeLensRequest, _, _>(|_, _, _| async {
                Ok(Some(
                    CODE_AUTHOR_LENSES
                        .iter()
                        .map(|(row, column, title)| lsp::CodeLens {
                            range: lsp::Range::new(
                                lsp::Position::new(*row, *column),
                                lsp::Position::new(*row, *column),
                            ),
                            command: Some(lsp::Command {
                                title: title.to_string(),
                                command: String::new(),
                                arguments: None,
                            }),
                            data: None,
                        })
                        .collect(),
                ))
            });

        cx.set_state(CODE_AUTHOR_SOURCE);
        cx.set_head_text(&CODE_AUTHOR_SOURCE.replace("ˇ", ""));
        assert!(
            code_lens_request.next().await.is_some(),
            "should have received a code lens request"
        );
        cx.run_until_parked();
        (cx, code_lens_request)
    }

    fn code_vision_with_authors_text(cx: &mut EditorLspTestContext) -> String {
        cx.update_editor(|editor, window, cx| {
            let snapshot = editor.snapshot(window, cx);
            let display_snapshot = &snapshot.display_snapshot;
            let all_rows = DisplayRow(0)..DisplayRow(display_snapshot.max_point().row().0 + 1);
            let hints_by_row = editor
                .code_vision_lines_in_rows(display_snapshot, all_rows, cx)
                .into_iter()
                .map(|(display_row, line)| {
                    let hints = line
                        .entries
                        .iter()
                        .map(|entry| format!("[{}]", entry.text))
                        .chain(line.author.iter().map(|author| {
                            if author.show_icon {
                                format!("[author: {}]", author.text)
                            } else {
                                format!("[{}]", author.text)
                            }
                        }))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let buffer_row = DisplayPoint::new(display_row, 0)
                        .to_point(display_snapshot)
                        .row;
                    (buffer_row, hints)
                })
                .collect::<HashMap<_, _>>();
            editor
                .buffer()
                .read(cx)
                .snapshot(cx)
                .text()
                .split('\n')
                .enumerate()
                .map(|(row, line_text)| match hints_by_row.get(&(row as u32)) {
                    Some(hints) => format!("{line_text}  {hints}"),
                    None => line_text.to_string(),
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    #[gpui::test]
    async fn test_code_vision_shows_code_author_after_usages(cx: &mut TestAppContext) {
        let (mut cx, mut code_lens_request) = code_author_context(cx).await;
        cx.update_editor(|editor, _, _| {
            assert!(
                editor.blame().is_some(),
                "code vision should start git blame for its code author hints"
            );
            assert!(
                !editor.show_git_blame_gutter(),
                "starting blame for code vision must not show the blame gutter"
            );
            assert!(
                !editor.show_git_blame_inline,
                "starting blame for code vision must not show inline blame"
            );
        });
        assert_eq!(
            code_vision_with_authors_text(&mut cx),
            indoc! {"
                interface QaseReporterOptions {  [1 usage] [author: Oleksandr Kreminskyi]
                  readonly qaseCode: string;
                }

                class QaseReporter {  [no usages] [author: Oleksandr Kreminskyi]
                  constructor(private readonly options: QaseReporterOptions) {  [no usages] [author: Oleksandr Kreminskyi]

                    this.options.qaseCode;
                  }
                }
            "},
            "declarations should end with their code author after the usages, while the interface \
             property and the blank row blamed on another author should add nothing"
        );

        cx.set_selections_state(indoc! {"
            interface QaseReporterOptions {
              readonly qaseCode: «stringˇ»;
            }

            class QaseReporter {
              constructor(private readonly options: QaseReporterOptions) {

                this.options.qaseCode;
              }
            }
        "});
        cx.update_editor(|editor, window, cx| editor.handle_input("number", window, cx));
        cx.executor()
            .advance_clock(LSP_REQUEST_DEBOUNCE_TIMEOUT + Duration::from_millis(50));
        assert!(
            code_lens_request.next().await.is_some(),
            "should have received another code lens request after the edit"
        );
        cx.run_until_parked();
        assert_eq!(
            code_vision_with_authors_text(&mut cx),
            indoc! {"
                interface QaseReporterOptions {  [1 usage] [author: Oleksandr Kreminskyi *]
                  readonly qaseCode: number;
                }

                class QaseReporter {  [no usages] [author: Oleksandr Kreminskyi]
                  constructor(private readonly options: QaseReporterOptions) {  [no usages] [author: Oleksandr Kreminskyi]

                    this.options.qaseCode;
                  }
                }
            "},
            "an uncommitted change inside the interface should mark only its author hint as modified"
        );
    }

    #[gpui::test]
    async fn test_code_vision_author_click_shows_git_blame_gutter(cx: &mut TestAppContext) {
        let (mut cx, _code_lens_request) = code_author_context(cx).await;
        cx.update_editor(|_, _, cx| cx.notify());
        cx.run_until_parked();

        let author_bounds = cx
            .debug_bounds("code_vision_author_0")
            .expect("the interface row should render its code author hint");
        cx.simulate_mouse_move(author_bounds.center(), None, Modifiers::none());
        cx.simulate_click(author_bounds.center(), Modifiers::none());
        cx.run_until_parked();

        cx.update_editor(|editor, _, _| {
            assert!(
                editor.show_git_blame_gutter(),
                "clicking the code author hint should annotate the file with the git blame gutter"
            );
            assert!(
                !editor.show_git_blame_inline,
                "clicking the code author hint must not turn on inline blame"
            );
        });
        cx.assert_editor_state(CODE_AUTHOR_SOURCE);
    }

    #[gpui::test]
    async fn test_code_vision_click_dispatches_lens_locations(cx: &mut TestAppContext) {
        let (mut cx, usage_locations) = show_usages_lens_context(cx).await;
        let dispatched = record_show_usages(&mut cx);

        let click_position = click_code_vision(&mut cx, 0);

        let dispatched = dispatched.lock().drain(..).collect::<Vec<_>>();
        assert_eq!(
            dispatched.len(),
            1,
            "clicking a usages hint should hand its locations to Show Usages exactly once"
        );
        let usages = &dispatched[0];
        assert_eq!(usages.locations, usage_locations);
        assert_eq!(usages.kind, UsageLocationsKind::References);
        assert_eq!(
            usages.popup_position, click_position,
            "the popup should open where the hint was clicked"
        );
        assert_eq!(usages.server_id, cx.lsp.server.server_id());
        assert_eq!(usages.editor, cx.editor.downgrade());
        let symbol_point = cx.editor(|editor, _, cx| {
            usages
                .symbol_position
                .to_point(&editor.buffer().read(cx).snapshot(cx))
        });
        assert_eq!(
            symbol_point,
            Point::new(0, 9),
            "the symbol position should come from the lens command's position argument"
        );
        let editor_count =
            cx.update_workspace(|workspace, _, cx| workspace.items_of_type::<Editor>(cx).count());
        assert_eq!(
            editor_count, 1,
            "the references multibuffer must not open when Show Usages handles the hint"
        );
        cx.assert_editor_state(SHOW_USAGES_LENS_SOURCE);
    }

    #[gpui::test]
    async fn test_code_vision_click_without_handler_opens_multibuffer(cx: &mut TestAppContext) {
        let (mut cx, _) = show_usages_lens_context(cx).await;

        click_code_vision(&mut cx, 0);

        let original_editor = cx.editor.clone();
        let opened_titles = cx.update_workspace(|workspace, _, cx| {
            workspace
                .items_of_type::<Editor>(cx)
                .filter(|editor| *editor != original_editor)
                .map(|editor| editor.read(cx).buffer().read(cx).title(cx).into_owned())
                .collect::<Vec<_>>()
        });
        assert_eq!(
            opened_titles,
            vec!["Definitions for hello".to_string()],
            "without a Show Usages handler the hint should keep opening the locations multibuffer"
        );
    }

    #[gpui::test]
    async fn test_code_vision_no_usages_click_dispatches_empty_locations(cx: &mut TestAppContext) {
        init_test(cx, |_| {});
        update_test_editor_settings(cx, &|settings| {
            settings.code_lens = Some(CodeLens::On);
        });

        let mut cx = EditorLspTestContext::new_typescript(
            lsp::ServerCapabilities {
                code_lens_provider: Some(lsp::CodeLensOptions {
                    resolve_provider: None,
                }),
                ..lsp::ServerCapabilities::default()
            },
            cx,
        )
        .await;

        let mut code_lens_request =
            cx.set_request_handler::<lsp::request::CodeLensRequest, _, _>(move |_, _, _| async {
                Ok(Some(vec![lsp::CodeLens {
                    range: lsp::Range::new(lsp::Position::new(0, 9), lsp::Position::new(0, 14)),
                    command: Some(lsp::Command {
                        title: "0 references".to_owned(),
                        command: String::new(),
                        arguments: None,
                    }),
                    data: None,
                }]))
            });

        cx.set_state("ˇfunction hello() {}\nfunction world() {}");
        assert!(
            code_lens_request.next().await.is_some(),
            "should have received a code lens request"
        );
        cx.run_until_parked();
        cx.editor(|editor, _, cx| {
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                "function hello() {}  [no usages]\nfunction world() {}",
                "a references lens without usages should render the IntelliJ no usages hint"
            );
        });

        let dispatched = record_show_usages(&mut cx);
        let click_position = click_code_vision(&mut cx, 0);

        let dispatched = dispatched.lock().drain(..).collect::<Vec<_>>();
        assert_eq!(
            dispatched.len(),
            1,
            "clicking a no usages hint should still ask Show Usages for its empty result"
        );
        let usages = &dispatched[0];
        assert!(
            usages.locations.is_empty(),
            "a no usages hint has no locations to hand over"
        );
        assert_eq!(usages.kind, UsageLocationsKind::References);
        assert_eq!(
            usages.popup_position, click_position,
            "the no usages hint should appear where the hint was clicked"
        );
        assert_eq!(usages.server_id, cx.lsp.server.server_id());
        let symbol_point = cx.editor(|editor, _, cx| {
            usages
                .symbol_position
                .to_point(&editor.buffer().read(cx).snapshot(cx))
        });
        assert_eq!(
            symbol_point,
            Point::new(0, 9),
            "the empty search should target the lens range start"
        );
        let editor_count =
            cx.update_workspace(|workspace, _, cx| workspace.items_of_type::<Editor>(cx).count());
        assert_eq!(editor_count, 1, "no multibuffer should open for no usages");
        cx.assert_editor_state("ˇfunction hello() {}\nfunction world() {}");
    }

    #[gpui::test]
    async fn test_code_vision_renders_at_line_end_without_extra_rows(cx: &mut TestAppContext) {
        init_test(cx, |_| {});
        update_test_editor_settings(cx, &|settings| {
            settings.code_lens = Some(CodeLens::On);
        });

        let mut cx = EditorLspTestContext::new_typescript(
            lsp::ServerCapabilities {
                code_lens_provider: Some(lsp::CodeLensOptions {
                    resolve_provider: None,
                }),
                execute_command_provider: Some(lsp::ExecuteCommandOptions {
                    commands: vec!["lens_cmd".to_string()],
                    ..lsp::ExecuteCommandOptions::default()
                }),
                ..lsp::ServerCapabilities::default()
            },
            cx,
        )
        .await;

        let mut code_lens_request =
            cx.set_request_handler::<lsp::request::CodeLensRequest, _, _>(move |_, _, _| async {
                Ok(Some(vec![
                    lsp::CodeLens {
                        range: lsp::Range::new(lsp::Position::new(0, 0), lsp::Position::new(0, 19)),
                        command: Some(lsp::Command {
                            title: "2 references".to_owned(),
                            command: "lens_cmd".to_owned(),
                            arguments: None,
                        }),
                        data: None,
                    },
                    lsp::CodeLens {
                        range: lsp::Range::new(lsp::Position::new(1, 0), lsp::Position::new(1, 19)),
                        command: Some(lsp::Command {
                            title: "0 references".to_owned(),
                            command: "lens_cmd".to_owned(),
                            arguments: None,
                        }),
                        data: None,
                    },
                ]))
            });

        cx.set_state("ˇfunction hello() {}\nfunction world() {}");

        assert!(
            code_lens_request.next().await.is_some(),
            "should have received a code lens request"
        );
        cx.run_until_parked();

        cx.update_editor(|editor, _, cx| {
            assert_eq!(
                editor.code_lens_enabled(),
                true,
                "code lens should be enabled"
            );
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                "function hello() {}  [2 usages]\nfunction world() {}  [no usages]",
                "both lenses should render as usages hints at the end of their lines"
            );
            assert_eq!(
                editor.display_text(cx),
                "function hello() {}\nfunction world() {}",
                "code vision hints must not add display rows or display text"
            );
            assert_eq!(
                editor.display_snapshot(cx).max_point().row(),
                DisplayRow(1),
                "code vision hints must not add display rows"
            );
        });
    }

    #[gpui::test]
    async fn test_code_vision_skips_property_field_and_enum_member_lenses(cx: &mut TestAppContext) {
        init_test(cx, |_| {});
        update_test_editor_settings(cx, &|settings| {
            settings.code_lens = Some(CodeLens::On);
        });

        let mut cx = EditorLspTestContext::new_typescript(
            lsp::ServerCapabilities {
                code_lens_provider: Some(lsp::CodeLensOptions {
                    resolve_provider: Some(true),
                }),
                ..lsp::ServerCapabilities::default()
            },
            cx,
        )
        .await;

        let mut code_lens_request =
            cx.set_request_handler::<lsp::request::CodeLensRequest, _, _>(move |_, _, _| async {
                Ok(Some(
                    MEMBER_LENSES
                        .iter()
                        .map(|(row, column, kind, _)| lsp::CodeLens {
                            range: lsp::Range::new(
                                lsp::Position::new(*row, *column),
                                lsp::Position::new(*row, *column),
                            ),
                            command: None,
                            data: Some(serde_json::json!({ "row": row, "kind": kind })),
                        })
                        .collect(),
                ))
            });

        let resolved_rows = Arc::new(Mutex::new(HashSet::<u32>::default()));
        cx.lsp
            .set_request_handler::<lsp::request::CodeLensResolve, _, _>({
                let resolved_rows = resolved_rows.clone();
                move |lens, _| {
                    let resolved_rows = resolved_rows.clone();
                    async move {
                        let data = lens.data.clone().unwrap_or_default();
                        let row = data
                            .get("row")
                            .and_then(|row| row.as_u64())
                            .unwrap_or_default() as u32;
                        let kind = data
                            .get("kind")
                            .and_then(|kind| kind.as_str())
                            .unwrap_or_default()
                            .to_owned();
                        resolved_rows.lock().insert(row);
                        let title = MEMBER_LENSES
                            .iter()
                            .find(|(lens_row, _, lens_kind, _)| {
                                *lens_row == row && *lens_kind == kind
                            })
                            .map(|(_, _, _, title)| title.to_string())
                            .unwrap_or_default();
                        Ok(lsp::CodeLens {
                            command: Some(lsp::Command {
                                title,
                                command: String::new(),
                                arguments: None,
                            }),
                            ..lens
                        })
                    }
                }
            });

        cx.set_state(MEMBER_LENS_SOURCE);
        assert!(
            code_lens_request.next().await.is_some(),
            "should have received a code lens request"
        );
        cx.run_until_parked();

        cx.update_editor(|editor, _, cx| {
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                indoc! {"
                    interface QaseReporterOptions {  [1 usage]
                      readonly qaseCode: string;
                      qaseUrl?: string;
                    }
                    class QaseReporter {  [no usages]
                      private options: QaseReporterOptions;
                      constructor(options: QaseReporterOptions) {  [no usages]
                        this.options = options;
                      }
                      report(): void {}  [2 usages]
                    }
                    enum Status {  [1 usage]
                      Passed,
                      Failed = 1,
                    }
                "},
                "interface properties, class fields and enum members should get no hints, \
                 zero implementations should stay hidden"
            );
            assert_eq!(
                editor.display_text(cx),
                MEMBER_LENS_SOURCE.replace("ˇ", ""),
                "code vision hints must not change the displayed text"
            );
        });
        assert_eq!(
            *resolved_rows.lock(),
            HashSet::from_iter([0, 4, 6, 9, 11]),
            "lenses on members without code vision should never be resolved"
        );
    }

    #[gpui::test]
    async fn test_code_vision_refresh_requeries_open_document(cx: &mut TestAppContext) {
        init_test(cx, |_| {});
        update_test_editor_settings(cx, &|settings| {
            settings.code_lens = Some(CodeLens::On);
        });

        let mut cx = EditorLspTestContext::new_typescript(
            lsp::ServerCapabilities {
                code_lens_provider: Some(lsp::CodeLensOptions {
                    resolve_provider: None,
                }),
                execute_command_provider: Some(lsp::ExecuteCommandOptions {
                    commands: vec!["lens_cmd".to_string()],
                    ..lsp::ExecuteCommandOptions::default()
                }),
                ..lsp::ServerCapabilities::default()
            },
            cx,
        )
        .await;

        let lens_title = Arc::new(Mutex::new("Initial lens".to_string()));
        let mut code_lens_request =
            cx.set_request_handler::<lsp::request::CodeLensRequest, _, _>({
                let lens_title = lens_title.clone();
                move |_, _, _| {
                    let lens_title = lens_title.clone();
                    async move {
                        let title = lens_title.lock().clone();
                        Ok(Some(vec![lsp::CodeLens {
                            range: lsp::Range::new(
                                lsp::Position::new(0, 0),
                                lsp::Position::new(0, 19),
                            ),
                            command: Some(lsp::Command {
                                title,
                                command: "lens_cmd".to_owned(),
                                arguments: None,
                            }),
                            data: None,
                        }]))
                    }
                }
            });

        cx.set_state("ˇfunction hello() {}\nfunction world() {}");
        assert!(
            code_lens_request.next().await.is_some(),
            "should have received the initial code lens request"
        );
        cx.run_until_parked();
        cx.editor(|editor, _, cx| {
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                "function hello() {}  [Initial lens]\nfunction world() {}",
                "initial fetch should render the server title"
            );
        });

        *lens_title.lock() = "Refreshed lens".to_string();
        cx.lsp
            .request::<lsp::request::CodeLensRefresh>((), lsp::DEFAULT_LSP_REQUEST_TIMEOUT)
            .await
            .into_response()
            .expect("code lens refresh request failed");
        cx.executor()
            .advance_clock(LSP_REQUEST_DEBOUNCE_TIMEOUT * 2);
        cx.run_until_parked();
        cx.editor(|editor, _, cx| {
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                "function hello() {}  [Refreshed lens]\nfunction world() {}",
                "refresh should update the displayed hint to the new server title"
            );
        });
    }

    #[gpui::test]
    async fn test_code_vision_dynamic_registration_requeries_open_document(
        cx: &mut TestAppContext,
    ) {
        init_test(cx, |_| {});
        update_test_editor_settings(cx, &|settings| {
            settings.code_lens = Some(CodeLens::On);
        });

        // The server advertises no code lens capability up front; it registers
        // `textDocument/codeLens` dynamically only after the document is open.
        let mut cx = EditorLspTestContext::new_typescript(
            lsp::ServerCapabilities {
                execute_command_provider: Some(lsp::ExecuteCommandOptions {
                    commands: vec!["lens_cmd".to_string()],
                    ..lsp::ExecuteCommandOptions::default()
                }),
                ..lsp::ServerCapabilities::default()
            },
            cx,
        )
        .await;

        let _code_lens_request =
            cx.set_request_handler::<lsp::request::CodeLensRequest, _, _>(move |_, _, _| async {
                Ok(Some(vec![lsp::CodeLens {
                    range: lsp::Range::new(lsp::Position::new(0, 0), lsp::Position::new(0, 19)),
                    command: Some(lsp::Command {
                        title: "Dynamic lens".to_owned(),
                        command: "lens_cmd".to_owned(),
                        arguments: None,
                    }),
                    data: None,
                }]))
            });

        cx.set_state("ˇfunction hello() {}\nfunction world() {}");
        // Drain any debounced refresh scheduled before the capability exists, so
        // the post-registration re-query can only come from the dynamic
        // registration handling itself.
        cx.executor()
            .advance_clock(LSP_REQUEST_DEBOUNCE_TIMEOUT * 2);
        cx.run_until_parked();
        cx.editor(|editor, _, cx| {
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                "function hello() {}\nfunction world() {}",
                "no hints should render before the capability is registered"
            );
        });

        cx.lsp
            .request::<lsp::request::RegisterCapability>(
                lsp::RegistrationParams {
                    registrations: vec![lsp::Registration {
                        id: "code-lens".to_string(),
                        method: "textDocument/codeLens".to_string(),
                        register_options: Some(
                            serde_json::to_value(lsp::CodeLensOptions {
                                resolve_provider: None,
                            })
                            .unwrap(),
                        ),
                    }],
                },
                lsp::DEFAULT_LSP_REQUEST_TIMEOUT,
            )
            .await
            .into_response()
            .expect("register capability request failed");
        cx.executor()
            .advance_clock(LSP_REQUEST_DEBOUNCE_TIMEOUT * 2);
        cx.run_until_parked();
        cx.editor(|editor, _, cx| {
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                "function hello() {}  [Dynamic lens]\nfunction world() {}",
                "dynamic textDocument/codeLens registration should re-query and display hints for the open document"
            );
        });
    }

    #[gpui::test]
    async fn test_code_vision_kept_across_refresh(cx: &mut TestAppContext) {
        init_test(cx, |_| {});
        update_test_editor_settings(cx, &|settings| {
            settings.code_lens = Some(CodeLens::On);
        });

        let mut cx = EditorLspTestContext::new_typescript(
            lsp::ServerCapabilities {
                code_lens_provider: Some(lsp::CodeLensOptions {
                    resolve_provider: None,
                }),
                execute_command_provider: Some(lsp::ExecuteCommandOptions {
                    commands: vec!["lens_cmd".to_string()],
                    ..lsp::ExecuteCommandOptions::default()
                }),
                ..lsp::ServerCapabilities::default()
            },
            cx,
        )
        .await;

        let mut code_lens_request =
            cx.set_request_handler::<lsp::request::CodeLensRequest, _, _>(move |_, _, _| async {
                Ok(Some(vec![lsp::CodeLens {
                    range: lsp::Range::new(lsp::Position::new(0, 0), lsp::Position::new(0, 19)),
                    command: Some(lsp::Command {
                        title: "1 reference".to_owned(),
                        command: "lens_cmd".to_owned(),
                        arguments: None,
                    }),
                    data: None,
                }]))
            });

        cx.set_state("ˇfunction hello() {}\nfunction world() {}");

        assert!(
            code_lens_request.next().await.is_some(),
            "should have received the initial code lens request"
        );
        cx.run_until_parked();

        cx.editor(|editor, _, cx| {
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                "function hello() {}  [1 usage]\nfunction world() {}",
                "initial fetch should render the usages hint"
            );
        });

        cx.update_editor(|editor, window, cx| {
            editor.move_to_end(&crate::actions::MoveToEnd, window, cx);
            editor.handle_input("\n// trailing comment", window, cx);
        });
        cx.executor()
            .advance_clock(LSP_REQUEST_DEBOUNCE_TIMEOUT + Duration::from_millis(50));
        assert!(
            code_lens_request.next().await.is_some(),
            "should have received another code lens request after edit"
        );
        cx.run_until_parked();

        cx.editor(|editor, _, cx| {
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                "function hello() {}  [1 usage]\nfunction world() {}\n// trailing comment",
                "the refreshed lens should keep rendering the same hint"
            );
        });
    }

    #[gpui::test]
    async fn test_code_vision_kept_while_refetched_lenses_resolve(cx: &mut TestAppContext) {
        init_test(cx, |_| {});
        update_test_editor_settings(cx, &|settings| {
            settings.code_lens = Some(CodeLens::On);
        });

        let mut cx = EditorLspTestContext::new_typescript(
            lsp::ServerCapabilities {
                code_lens_provider: Some(lsp::CodeLensOptions {
                    resolve_provider: Some(true),
                }),
                ..lsp::ServerCapabilities::default()
            },
            cx,
        )
        .await;

        // The LSP returns shallow code lenses on every fetch; only `resolve`
        // populates the command/title. This is the realistic flow with
        // servers like rust-analyzer and exercises the path where each
        // post-edit refresh comes back unresolved before the resolve catches
        // up.
        let mut code_lens_request =
            cx.set_request_handler::<lsp::request::CodeLensRequest, _, _>(move |_, _, _| async {
                Ok(Some(vec![lsp::CodeLens {
                    range: lsp::Range::new(lsp::Position::new(0, 0), lsp::Position::new(0, 19)),
                    command: None,
                    data: Some(serde_json::json!({"id": "lens_1"})),
                }]))
            });

        let resolve_gate: Arc<Mutex<Option<futures::channel::oneshot::Receiver<()>>>> =
            Arc::default();
        cx.lsp
            .set_request_handler::<lsp::request::CodeLensResolve, _, _>({
                let resolve_gate = resolve_gate.clone();
                move |lens, _| {
                    let gate = resolve_gate.lock().take();
                    async move {
                        if let Some(gate) = gate {
                            gate.await.ok();
                        }
                        Ok(lsp::CodeLens {
                            command: Some(lsp::Command {
                                title: "1 reference".to_owned(),
                                command: "resolved_cmd".to_owned(),
                                arguments: None,
                            }),
                            ..lens
                        })
                    }
                }
            });

        cx.set_state("ˇfunction hello() {}\nfunction world() {}");

        assert!(
            code_lens_request.next().await.is_some(),
            "should have received the initial code lens request"
        );
        cx.run_until_parked();

        cx.editor(|editor, _, cx| {
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                "function hello() {}  [1 usage]\nfunction world() {}",
                "resolve should fill the hint with the server count"
            );
        });

        let (release_resolve, gate) = futures::channel::oneshot::channel::<()>();
        *resolve_gate.lock() = Some(gate);
        cx.update_editor(|editor, window, cx| {
            editor.move_to_end(&crate::actions::MoveToEnd, window, cx);
            editor.handle_input(" ", window, cx);
        });
        cx.executor()
            .advance_clock(LSP_REQUEST_DEBOUNCE_TIMEOUT + Duration::from_millis(50));
        assert!(
            code_lens_request.next().await.is_some(),
            "should have received another (shallow) code lens request after edit"
        );
        cx.run_until_parked();

        cx.editor(|editor, _, cx| {
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                "function hello() {}  [1 usage]\nfunction world() {} ",
                "the previous hint should stay while the refetched lens is still resolving"
            );
        });

        release_resolve.send(()).ok();
        cx.run_until_parked();

        cx.editor(|editor, _, cx| {
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                "function hello() {}  [1 usage]\nfunction world() {} ",
                "the resolved lens should keep rendering the same hint"
            );
        });
    }

    #[gpui::test]
    async fn test_code_vision_renders_nothing_before_resolve(cx: &mut TestAppContext) {
        init_test(cx, |_| {});
        update_test_editor_settings(cx, &|settings| {
            settings.code_lens = Some(CodeLens::On);
        });

        let mut cx = EditorLspTestContext::new_typescript(
            lsp::ServerCapabilities {
                code_lens_provider: Some(lsp::CodeLensOptions {
                    resolve_provider: Some(true),
                }),
                ..lsp::ServerCapabilities::default()
            },
            cx,
        )
        .await;

        let mut code_lens_request =
            cx.set_request_handler::<lsp::request::CodeLensRequest, _, _>(move |_, _, _| async {
                let mut lenses = Vec::new();
                lenses.push(lsp::CodeLens {
                    range: lsp::Range::new(lsp::Position::new(0, 0), lsp::Position::new(0, 19)),
                    command: None,
                    data: Some(serde_json::json!({"id": "lens_1"})),
                });
                Ok(Some(lenses))
            });

        let (resolve_tx, resolve_rx) = futures::channel::oneshot::channel::<()>();
        let resolve_rx = Mutex::new(Some(resolve_rx));
        cx.lsp
            .set_request_handler::<lsp::request::CodeLensResolve, _, _>(move |lens, _| {
                let rx = resolve_rx.lock().take();
                async move {
                    if let Some(rx) = rx {
                        rx.await.ok();
                    }
                    Ok(lsp::CodeLens {
                        command: Some(lsp::Command {
                            title: "1 reference".to_owned(),
                            command: "resolved_cmd".to_owned(),
                            arguments: None,
                        }),
                        ..lens
                    })
                }
            });

        cx.set_state("ˇfunction hello() {}");

        assert!(
            code_lens_request.next().await.is_some(),
            "should have received the initial code lens request"
        );
        cx.run_until_parked();

        cx.update_editor(|editor, _, cx| {
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                "function hello() {}",
                "an unresolved lens should render nothing"
            );
            assert_eq!(
                editor.display_text(cx),
                "function hello() {}",
                "no row should be reserved for an unresolved lens"
            );
        });

        resolve_tx.send(()).ok();
        cx.run_until_parked();

        cx.editor(|editor, _, cx| {
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                "function hello() {}  [1 usage]",
                "after resolve the hint should display the usages count"
            );
        });
    }

    #[gpui::test]
    async fn test_code_vision_hides_lens_resolved_with_empty_title(cx: &mut TestAppContext) {
        init_test(cx, |_| {});
        update_test_editor_settings(cx, &|settings| {
            settings.code_lens = Some(CodeLens::On);
        });

        let mut cx = EditorLspTestContext::new_typescript(
            lsp::ServerCapabilities {
                code_lens_provider: Some(lsp::CodeLensOptions {
                    resolve_provider: Some(true),
                }),
                ..lsp::ServerCapabilities::default()
            },
            cx,
        )
        .await;

        let mut code_lens_request =
            cx.set_request_handler::<lsp::request::CodeLensRequest, _, _>(move |_, _, _| async {
                let mut lenses = Vec::new();
                lenses.push(lsp::CodeLens {
                    range: lsp::Range::new(lsp::Position::new(0, 0), lsp::Position::new(0, 19)),
                    command: None,
                    data: Some(serde_json::json!({"id": "lens_1"})),
                });
                Ok(Some(lenses))
            });

        cx.lsp
            .set_request_handler::<lsp::request::CodeLensResolve, _, _>(|lens, _| async move {
                Ok(lsp::CodeLens {
                    command: Some(lsp::Command {
                        title: String::new(),
                        command: "noop".to_owned(),
                        arguments: None,
                    }),
                    ..lens
                })
            });

        cx.set_state("ˇfunction hello() {}");

        assert!(
            code_lens_request.next().await.is_some(),
            "should have received the initial code lens request"
        );
        cx.run_until_parked();

        cx.editor(|editor, _, cx| {
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                "function hello() {}",
                "a lens resolved to an empty title should render nothing"
            );
            assert_eq!(code_vision_line_count(editor), 0);
        });
    }

    #[gpui::test]
    async fn test_code_vision_same_range_lenses_resolve_independently(cx: &mut TestAppContext) {
        init_test(cx, |_| {});
        update_test_editor_settings(cx, &|settings| {
            settings.code_lens = Some(CodeLens::On);
        });

        let mut cx = EditorLspTestContext::new_typescript(
            lsp::ServerCapabilities {
                code_lens_provider: Some(lsp::CodeLensOptions {
                    resolve_provider: Some(true),
                }),
                ..lsp::ServerCapabilities::default()
            },
            cx,
        )
        .await;

        // Two shallow lenses on the same range, distinguished only by `data`
        // — exactly the shape vtsls/TypeScript-LS uses for the
        // "references" + "implementations" pair on the same line.
        let mut code_lens_request =
            cx.set_request_handler::<lsp::request::CodeLensRequest, _, _>(move |_, _, _| async {
                Ok(Some(vec![
                    lsp::CodeLens {
                        range: lsp::Range::new(lsp::Position::new(0, 0), lsp::Position::new(0, 19)),
                        command: None,
                        data: Some(serde_json::json!({"kind": "implementations"})),
                    },
                    lsp::CodeLens {
                        range: lsp::Range::new(lsp::Position::new(0, 0), lsp::Position::new(0, 19)),
                        command: None,
                        data: Some(serde_json::json!({"kind": "references"})),
                    },
                ]))
            });

        let resolve_calls = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        cx.lsp
            .set_request_handler::<lsp::request::CodeLensResolve, _, _>({
                let resolve_calls = resolve_calls.clone();
                move |lens, _| {
                    let resolve_calls = resolve_calls.clone();
                    async move {
                        let kind = lens
                            .data
                            .as_ref()
                            .and_then(|d| d.get("kind"))
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        resolve_calls.lock().push(kind.clone());
                        let title = match kind.as_str() {
                            Some("references") => "2 references",
                            Some("implementations") => "1 implementation",
                            _ => "",
                        };
                        Ok(lsp::CodeLens {
                            command: Some(lsp::Command {
                                title: title.to_owned(),
                                command: "noop".to_owned(),
                                arguments: None,
                            }),
                            ..lens
                        })
                    }
                }
            });

        cx.set_state("ˇfunction hello() {}");

        assert!(
            code_lens_request.next().await.is_some(),
            "should have received the initial code lens request"
        );
        cx.run_until_parked();

        let calls = resolve_calls.lock().clone();
        assert_eq!(
            calls.len(),
            2,
            "both same-range lenses should be resolved independently, got {calls:?}"
        );
        let kinds: Vec<&str> = calls.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(kinds.contains(&"references"), true);
        assert_eq!(kinds.contains(&"implementations"), true);

        cx.editor(|editor, _, cx| {
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                "function hello() {}  [2 usages] [1 implementation]",
                "both same-range lenses should render, usages before implementations"
            );
        });
    }

    #[gpui::test]
    async fn test_code_vision_hides_lens_resolved_without_command(cx: &mut TestAppContext) {
        init_test(cx, |_| {});
        update_test_editor_settings(cx, &|settings| {
            settings.code_lens = Some(CodeLens::On);
        });

        let mut cx = EditorLspTestContext::new_typescript(
            lsp::ServerCapabilities {
                code_lens_provider: Some(lsp::CodeLensOptions {
                    resolve_provider: Some(true),
                }),
                ..lsp::ServerCapabilities::default()
            },
            cx,
        )
        .await;

        let mut code_lens_request =
            cx.set_request_handler::<lsp::request::CodeLensRequest, _, _>(move |_, _, _| async {
                Ok(Some(vec![lsp::CodeLens {
                    range: lsp::Range::new(lsp::Position::new(0, 0), lsp::Position::new(0, 19)),
                    command: None,
                    data: Some(serde_json::json!({"id": "lens_1"})),
                }]))
            });

        cx.lsp
            .set_request_handler::<lsp::request::CodeLensResolve, _, _>(|lens, _| async move {
                Ok(lsp::CodeLens {
                    command: None,
                    ..lens
                })
            });

        cx.set_state("ˇfunction hello() {}");

        assert!(
            code_lens_request.next().await.is_some(),
            "should have received the initial code lens request"
        );
        cx.run_until_parked();

        cx.editor(|editor, _, cx| {
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                "function hello() {}",
                "a lens resolved without a command should render nothing"
            );
            assert_eq!(code_vision_line_count(editor), 0);
        });
    }

    #[gpui::test]
    async fn test_code_lens_disabled_by_default(cx: &mut TestAppContext) {
        init_test(cx, |_| {});

        let mut cx = EditorLspTestContext::new_typescript(
            lsp::ServerCapabilities {
                code_lens_provider: Some(lsp::CodeLensOptions {
                    resolve_provider: None,
                }),
                execute_command_provider: Some(lsp::ExecuteCommandOptions {
                    commands: vec!["lens_cmd".to_string()],
                    ..lsp::ExecuteCommandOptions::default()
                }),
                ..lsp::ServerCapabilities::default()
            },
            cx,
        )
        .await;

        cx.lsp
            .set_request_handler::<lsp::request::CodeLensRequest, _, _>(|_, _| async move {
                panic!("Should not request code lenses when disabled");
            });

        cx.set_state("ˇfunction hello() {}");
        cx.run_until_parked();

        cx.editor(|editor, _, _cx| {
            assert_eq!(
                editor.code_lens_enabled(),
                false,
                "code lens should not be enabled when setting is off"
            );
        });
    }

    #[gpui::test]
    async fn test_code_vision_toggling(cx: &mut TestAppContext) {
        init_test(cx, |_| {});
        update_test_editor_settings(cx, &|settings| {
            settings.code_lens = Some(CodeLens::On);
        });

        let mut cx = EditorLspTestContext::new_typescript(
            lsp::ServerCapabilities {
                code_lens_provider: Some(lsp::CodeLensOptions {
                    resolve_provider: None,
                }),
                execute_command_provider: Some(lsp::ExecuteCommandOptions {
                    commands: vec!["lens_cmd".to_string()],
                    ..lsp::ExecuteCommandOptions::default()
                }),
                ..lsp::ServerCapabilities::default()
            },
            cx,
        )
        .await;

        let mut code_lens_request =
            cx.set_request_handler::<lsp::request::CodeLensRequest, _, _>(move |_, _, _| async {
                Ok(Some(vec![lsp::CodeLens {
                    range: lsp::Range::new(lsp::Position::new(0, 0), lsp::Position::new(0, 19)),
                    command: Some(lsp::Command {
                        title: "1 reference".to_owned(),
                        command: "lens_cmd".to_owned(),
                        arguments: None,
                    }),
                    data: None,
                }]))
            });

        cx.set_state("ˇfunction hello() {}");

        assert!(
            code_lens_request.next().await.is_some(),
            "should have received a code lens request"
        );
        cx.run_until_parked();

        cx.editor(|editor, _, _cx| {
            assert_eq!(
                editor.code_lens_enabled(),
                true,
                "code lens should be enabled"
            );
            assert_eq!(
                code_vision_line_count(editor),
                1,
                "Should have one code vision line"
            );
        });

        cx.update_editor(|editor, _window, cx| {
            editor.clear_code_lenses(cx);
        });

        cx.editor(|editor, _, _cx| {
            assert_eq!(
                editor.code_lens_enabled(),
                false,
                "code lens should be disabled after clearing"
            );
            assert_eq!(code_vision_line_count(editor), 0);
        });
    }

    #[gpui::test]
    async fn test_code_vision_resolves_unresolved_lenses(cx: &mut TestAppContext) {
        init_test(cx, |_| {});
        update_test_editor_settings(cx, &|settings| {
            settings.code_lens = Some(CodeLens::On);
        });

        let mut cx = EditorLspTestContext::new_typescript(
            lsp::ServerCapabilities {
                code_lens_provider: Some(lsp::CodeLensOptions {
                    resolve_provider: Some(true),
                }),
                ..lsp::ServerCapabilities::default()
            },
            cx,
        )
        .await;

        let mut code_lens_request =
            cx.set_request_handler::<lsp::request::CodeLensRequest, _, _>(move |_, _, _| async {
                Ok(Some(vec![
                    lsp::CodeLens {
                        range: lsp::Range::new(lsp::Position::new(0, 0), lsp::Position::new(0, 19)),
                        command: None,
                        data: Some(serde_json::json!({"id": "lens_1"})),
                    },
                    lsp::CodeLens {
                        range: lsp::Range::new(lsp::Position::new(1, 0), lsp::Position::new(1, 19)),
                        command: None,
                        data: Some(serde_json::json!({"id": "lens_2"})),
                    },
                ]))
            });

        cx.lsp
            .set_request_handler::<lsp::request::CodeLensResolve, _, _>(|lens, _| async move {
                let id = lens
                    .data
                    .as_ref()
                    .and_then(|d| d.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let title = match id {
                    "lens_1" => "3 references",
                    "lens_2" => "1 implementation",
                    _ => "unknown",
                };
                Ok(lsp::CodeLens {
                    command: Some(lsp::Command {
                        title: title.to_owned(),
                        command: format!("resolved_{id}"),
                        arguments: None,
                    }),
                    ..lens
                })
            });

        cx.set_state("ˇfunction hello() {}\nfunction world() {}");

        assert!(
            code_lens_request.next().await.is_some(),
            "should have received a code lens request"
        );
        cx.run_until_parked();

        cx.editor(|editor, _, cx| {
            assert_eq!(
                code_vision_assertion_text(editor, cx),
                "function hello() {}  [3 usages]\nfunction world() {}  [1 implementation]",
                "Unresolved lenses should have been resolved and displayed"
            );
        });
    }

    #[gpui::test]
    async fn test_code_vision_resolves_only_visible_lenses(cx: &mut TestAppContext) {
        init_test(cx, |_| {});
        update_test_editor_settings(cx, &|settings| {
            settings.code_lens = Some(CodeLens::On);
        });

        let line_count: u32 = 100;
        let lens_every: u32 = 10;
        let lines = (0..line_count)
            .map(|i| format!("function func_{i}() {{}}"))
            .collect::<Vec<_>>()
            .join("\n");

        let lens_lines = (0..line_count)
            .filter(|i| i % lens_every == 0)
            .collect::<Vec<_>>();

        let resolved_lines = Arc::new(Mutex::new(Vec::<u32>::new()));

        let fs = project::FakeFs::new(cx.executor());
        fs.insert_tree(path!("/dir"), serde_json::json!({ "main.ts": lines }))
            .await;

        let project = project::Project::test(fs, [path!("/dir").as_ref()], cx).await;
        let (multi_workspace, cx) = cx.add_window_view(|window, cx| {
            workspace::MultiWorkspace::test_new(project.clone(), window, cx)
        });
        let workspace = multi_workspace.read_with(cx, |mw, _| mw.workspace().clone());

        let language_registry = project.read_with(cx, |project, _| project.languages().clone());
        language_registry.add(Arc::new(language::Language::new(
            language::LanguageConfig {
                name: "TypeScript".into(),
                matcher: (language::LanguageMatcher {
                    path_suffixes: vec!["ts".to_string()],
                    ..language::LanguageMatcher::default()
                })
                .into(),
                ..language::LanguageConfig::default()
            },
            Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        )));

        let mut fake_servers = language_registry.register_fake_lsp(
            "TypeScript",
            language::FakeLspAdapter {
                capabilities: lsp::ServerCapabilities {
                    code_lens_provider: Some(lsp::CodeLensOptions {
                        resolve_provider: Some(true),
                    }),
                    ..lsp::ServerCapabilities::default()
                },
                ..language::FakeLspAdapter::default()
            },
        );

        let editor = workspace
            .update_in(cx, |workspace, window, cx| {
                workspace.open_abs_path(
                    std::path::PathBuf::from(path!("/dir/main.ts")),
                    workspace::OpenOptions::default(),
                    window,
                    cx,
                )
            })
            .await
            .unwrap()
            .downcast::<Editor>()
            .unwrap();
        let fake_server = fake_servers.next().await.unwrap();

        let lens_lines_for_handler = lens_lines.clone();
        fake_server.set_request_handler::<lsp::request::CodeLensRequest, _, _>(move |_, _| {
            let lens_lines = lens_lines_for_handler.clone();
            async move {
                Ok(Some(
                    lens_lines
                        .iter()
                        .map(|&line| lsp::CodeLens {
                            range: lsp::Range::new(
                                lsp::Position::new(line, 0),
                                lsp::Position::new(line, 10),
                            ),
                            command: None,
                            data: Some(serde_json::json!({ "line": line })),
                        })
                        .collect(),
                ))
            }
        });

        {
            let resolved_lines = resolved_lines.clone();
            fake_server.set_request_handler::<lsp::request::CodeLensResolve, _, _>(
                move |lens, _| {
                    let resolved_lines = resolved_lines.clone();
                    async move {
                        let line = lens
                            .data
                            .as_ref()
                            .and_then(|d| d.get("line"))
                            .and_then(|v| v.as_u64())
                            .unwrap() as u32;
                        resolved_lines.lock().push(line);
                        Ok(lsp::CodeLens {
                            command: Some(lsp::Command {
                                title: format!("{line} references"),
                                command: format!("show_refs_{line}"),
                                arguments: None,
                            }),
                            ..lens
                        })
                    }
                },
            );
        }

        cx.executor().advance_clock(Duration::from_millis(500));
        cx.run_until_parked();

        let initial_resolved = resolved_lines.lock().drain(..).collect::<HashSet<_>>();
        let initially_visible = visible_lens_lines(&editor, &lens_lines, cx);
        assert!(
            initially_visible.contains(&0) && !initially_visible.contains(&90),
            "the viewport should start at the top of the buffer, got {initially_visible:?}"
        );
        assert_eq!(
            initial_resolved, initially_visible,
            "Only lenses visible at the top should be resolved"
        );

        editor.update_in(cx, |editor, window, cx| {
            editor.move_to_end(&crate::actions::MoveToEnd, window, cx);
        });
        cx.executor().advance_clock(Duration::from_millis(500));
        cx.run_until_parked();

        let after_scroll_resolved = resolved_lines.lock().drain(..).collect::<HashSet<_>>();
        let visible_after_scroll = visible_lens_lines(&editor, &lens_lines, cx);
        assert!(
            visible_after_scroll.contains(&90) && !visible_after_scroll.contains(&0),
            "the viewport should end at the bottom of the buffer, got {visible_after_scroll:?}"
        );
        let newly_visible = visible_after_scroll
            .difference(&initially_visible)
            .copied()
            .collect::<HashSet<_>>();
        assert_eq!(
            after_scroll_resolved, newly_visible,
            "Only newly visible lenses at the bottom should be resolved, not middle ones"
        );
    }

    fn visible_lens_lines(
        editor: &Entity<Editor>,
        lens_lines: &[u32],
        cx: &mut VisualTestContext,
    ) -> HashSet<u32> {
        editor.update(cx, |editor, cx| {
            editor
                .visible_buffer_ranges(cx)
                .into_iter()
                .flat_map(|(buffer_snapshot, visible_range, _)| {
                    lens_lines
                        .iter()
                        .copied()
                        .filter(|line| {
                            let offset = buffer_snapshot.point_to_offset(Point::new(*line, 0));
                            visible_range.start.0 <= offset && offset <= visible_range.end.0
                        })
                        .collect::<Vec<_>>()
                })
                .collect()
        })
    }

    fn code_vision_line_count(editor: &Editor) -> usize {
        editor
            .code_lens
            .as_ref()
            .map_or(0, |state| state.lines.values().map(Vec::len).sum())
    }

    fn code_vision_assertion_text(editor: &Editor, cx: &ui::App) -> String {
        let snapshot = editor.buffer().read(cx).snapshot(cx);
        let hints_by_row = editor
            .code_lens
            .as_ref()
            .map(|state| {
                state
                    .lines
                    .values()
                    .flatten()
                    .filter(|line| !line.entries.is_empty())
                    .map(|line| {
                        let hints = line
                            .entries
                            .iter()
                            .map(|entry| format!("[{}]", entry.text))
                            .collect::<Vec<_>>()
                            .join(" ");
                        (line.position.to_point(&snapshot).row, hints)
                    })
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        snapshot
            .text()
            .split('\n')
            .enumerate()
            .map(|(row, line_text)| match hints_by_row.get(&(row as u32)) {
                Some(hints) => format!("{line_text}  {hints}"),
                None => line_text.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
