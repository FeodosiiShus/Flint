use std::borrow::Cow;
use std::cell::RefCell;
use std::cmp::min;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use editor::display_map::DisplayRow;
use editor::items::open_resolved_target;
use editor::{
    Bias, DisplayPoint, Editor, EditorEvent, EditorSettingsScrollbarProxy, MultiBufferOffset,
    PreviewLayout,
};
use gpui::{
    App, AsyncWindowContext, ClipboardItem, Context, Entity, FocusHandle, Focusable, ImageSource,
    InteractiveElement, IntoElement, IsZero, Pixels, Render, Resource, RetainAllImageCache,
    ScrollHandle, SharedString, SharedUri, Subscription, Task, WeakEntity, Window, point, px,
};
use markdown::{
    CodeBlockRenderer, CopyButtonVisibility, Markdown, MarkdownElement, MarkdownFont,
    MarkdownOptions, MarkdownStyle, WrapButtonVisibility,
};
use project::{Project, ProjectPath, image_store};
use settings::{Settings, update_settings_file};
use theme_settings::ThemeSettings;
use ui::utils::WithRemSize;
use ui::{
    ContextMenu, LinkPreview, ScrollAxes, Scrollbars, WithScrollbar, prelude::*, right_click_menu,
};
use util::{
    ResultExt,
    markdown::{source_position_from_fragment, split_local_url_fragment},
    paths::{PathStyle, PathWithPosition},
    rel_path::RelPath,
};
use workspace::notifications::NotifyTaskExt;
use workspace::path_link::{PathMatching, resolve_open_target};
use workspace::{OpenOptions, OpenVisible, Workspace};
use zed_actions::{DecreaseBufferFontSize, IncreaseBufferFontSize, ResetBufferFontSize};

use crate::{
    ScrollDown, ScrollDownByItem, ScrollPageDown, ScrollPageUp, ScrollToBottom, ScrollToTop,
    ScrollUp, ScrollUpByItem,
};

const REPARSE_DEBOUNCE: Duration = Duration::from_millis(200);

fn reset_persisted_font_size(settings: &mut settings::SettingsContent) {
    if let Some(markdown_preview) = settings.markdown_preview.as_mut() {
        markdown_preview.font_size = None;
    }
}

pub struct MarkdownPreview {
    editor: WeakEntity<Editor>,
    focus_handle: FocusHandle,
    markdown: Entity<Markdown>,
    scroll_handle: ScrollHandle,
    image_cache: Entity<RetainAllImageCache>,
    project: Option<Entity<Project>>,
    source_path: Option<PathBuf>,
    source_project_path: Option<ProjectPath>,
    pending_update_task: Option<Task<Result<()>>>,
    is_stale: bool,
    last_synced_source_index: Option<usize>,
    hovered_url: Option<SharedString>,
    _editor_subscription: Subscription,
}

impl MarkdownPreview {
    pub fn new(editor: &Editor, window: &mut Window, cx: &mut Context<Editor>) -> Entity<Self> {
        let editor_entity = cx.entity();
        let project = editor.project().cloned();
        let language_registry = project
            .as_ref()
            .map(|project| project.read(cx).languages().clone());
        let source_path = source_path(editor, cx);
        let source_project_path = source_project_path(editor, cx);
        cx.new(|cx| {
            let markdown = cx.new(|cx| {
                Markdown::new_with_options(
                    SharedString::default(),
                    language_registry,
                    None,
                    MarkdownOptions {
                        parse_html: true,
                        parse_heading_slugs: true,
                        render_metadata_blocks: true,
                        ..Default::default()
                    },
                    cx,
                )
            });
            let mut this = Self {
                editor: editor_entity.downgrade(),
                focus_handle: cx.focus_handle(),
                markdown,
                scroll_handle: ScrollHandle::new(),
                image_cache: RetainAllImageCache::new(cx),
                project,
                source_path,
                source_project_path,
                pending_update_task: None,
                is_stale: false,
                last_synced_source_index: None,
                hovered_url: None,
                _editor_subscription: cx.subscribe_in(
                    &editor_entity,
                    window,
                    Self::handle_editor_event,
                ),
            };
            this.update_markdown(false, true, window, cx);
            this
        })
    }

    fn handle_editor_event(
        &mut self,
        editor: &Entity<Editor>,
        event: &EditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            EditorEvent::Edited { .. } | EditorEvent::BufferEdited | EditorEvent::DirtyChanged => {
                self.update_markdown(true, false, window, cx);
            }
            EditorEvent::FileHandleChanged => {
                let editor = editor.read(cx);
                self.source_path = source_path(editor, cx);
                self.source_project_path = source_project_path(editor, cx);
                self.update_markdown(false, false, window, cx);
            }
            EditorEvent::ScrollPositionChanged { .. } => {
                if is_preview_visible(editor, cx) {
                    self.follow_editor_scroll(editor, cx);
                }
            }
            EditorEvent::PreviewLayoutChanged => {
                if !is_preview_visible(editor, cx) {
                    return;
                }
                if self.is_stale {
                    self.update_markdown(false, true, window, cx);
                } else {
                    self.scroll_to_editor_top(editor, cx);
                }
            }
            _ => {}
        }
    }

    fn update_markdown(
        &mut self,
        wait_for_debounce: bool,
        align_to_editor_top: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if wait_for_debounce && self.pending_update_task.is_some() {
            return;
        }
        self.pending_update_task = Some(cx.spawn_in(window, async move |this, cx| {
            if wait_for_debounce {
                cx.background_executor().timer(REPARSE_DEBOUNCE).await;
            }
            this.update(cx, |this, cx| {
                this.refresh_markdown(align_to_editor_top, cx);
                this.pending_update_task = None;
            })
        }));
    }

    fn refresh_markdown(&mut self, align_to_editor_top: bool, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        if !is_preview_visible(&editor, cx) {
            self.is_stale = true;
            return;
        }
        let Some(buffer) = editor.read(cx).buffer().read(cx).as_singleton() else {
            return;
        };
        let contents = SharedString::from(buffer.read(cx).as_rope().to_string());
        self.is_stale = false;
        self.hovered_url = None;
        self.markdown.update(cx, |markdown, cx| {
            markdown.reset(contents, cx);
        });
        if align_to_editor_top {
            self.scroll_to_editor_top(&editor, cx);
        }
        cx.notify();
    }

    fn follow_editor_scroll(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let Some(source_index) = editor_top_source_index(editor, cx) else {
            return;
        };
        if self.last_synced_source_index != Some(source_index) {
            self.scroll_preview_to_source_index(source_index, cx);
        }
    }

    fn scroll_to_editor_top(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let Some(source_index) = editor_top_source_index(editor, cx) else {
            return;
        };
        self.scroll_preview_to_source_index(source_index, cx);
    }

    fn scroll_preview_to_source_index(&mut self, source_index: usize, cx: &mut Context<Self>) {
        self.last_synced_source_index = Some(source_index);
        self.markdown.update(cx, |markdown, cx| {
            markdown.request_scroll_to_source_index_at_top(source_index, cx);
        });
        cx.notify();
    }

    fn base_directory(&self) -> Option<PathBuf> {
        self.source_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }

    fn workspace(&self, cx: &App) -> Option<Entity<Workspace>> {
        self.editor.upgrade()?.read(cx).workspace()
    }

    fn scroll_to_heading_in_source(
        &mut self,
        path: &Path,
        heading: SharedString,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.source_path.as_deref() != Some(path) {
            return false;
        }
        self.scroll_to_heading(heading, cx);
        true
    }

    fn scroll_to_heading(&mut self, heading: SharedString, cx: &mut Context<Self>) {
        self.markdown.update(cx, |markdown, cx| {
            markdown.scroll_to_heading_when_parsed(heading, cx);
        });
        cx.notify();
    }

    fn line_scroll_amount(&self, cx: &App) -> Pixels {
        let settings = ThemeSettings::get_global(cx);
        settings.markdown_preview_font_size(cx) * settings.buffer_line_height.value()
    }

    fn increase_font_size(
        &mut self,
        action: &IncreaseBufferFontSize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.adjust_font_size(action.persist, px(1.0), cx);
    }

    fn decrease_font_size(
        &mut self,
        action: &DecreaseBufferFontSize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.adjust_font_size(action.persist, px(-1.0), cx);
    }

    fn adjust_font_size(&mut self, persist: bool, delta: Pixels, cx: &mut Context<Self>) {
        if persist {
            let Some(workspace) = self.workspace(cx) else {
                return;
            };
            let fs = workspace.read(cx).app_state().fs.clone();
            update_settings_file(fs, cx, move |settings, cx| {
                let size = ThemeSettings::get_global(cx).markdown_preview_font_size(cx) + delta;
                settings.markdown_preview.get_or_insert_default().font_size =
                    Some(f32::from(theme_settings::clamp_font_size(size)).into());
            });
        } else {
            theme_settings::adjust_markdown_preview_font_size(cx, |size| size + delta);
        }
    }

    fn reset_font_size(
        &mut self,
        action: &ResetBufferFontSize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if action.persist {
            let Some(workspace) = self.workspace(cx) else {
                return;
            };
            let fs = workspace.read(cx).app_state().fs.clone();
            update_settings_file(fs, cx, move |settings, _| {
                reset_persisted_font_size(settings);
            });
        } else {
            theme_settings::reset_markdown_preview_font_size(cx);
        }
    }

    fn scroll_by_amount(&self, distance: Pixels) {
        let offset = self.scroll_handle.offset();
        self.scroll_handle
            .set_offset(point(offset.x, offset.y - distance));
    }

    fn scroll_page_up(&mut self, _: &ScrollPageUp, _window: &mut Window, cx: &mut Context<Self>) {
        let viewport_height = self.scroll_handle.bounds().size.height;
        if viewport_height.is_zero() {
            return;
        }

        self.scroll_by_amount(-viewport_height);
        cx.notify();
    }

    fn scroll_page_down(
        &mut self,
        _: &ScrollPageDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let viewport_height = self.scroll_handle.bounds().size.height;
        if viewport_height.is_zero() {
            return;
        }

        self.scroll_by_amount(viewport_height);
        cx.notify();
    }

    fn scroll_up(&mut self, _: &ScrollUp, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(bounds) = self
            .scroll_handle
            .bounds_for_item(self.scroll_handle.top_item())
        {
            let item_height = bounds.size.height;
            let max_height = window.rem_size() * 2;
            let scroll_height = min(item_height, max_height);
            self.scroll_by_amount(-scroll_height);
        } else {
            let scroll_height = self.line_scroll_amount(cx);
            if !scroll_height.is_zero() {
                self.scroll_by_amount(-scroll_height);
            }
        }
        cx.notify();
    }

    fn scroll_down(&mut self, _: &ScrollDown, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(bounds) = self
            .scroll_handle
            .bounds_for_item(self.scroll_handle.top_item())
        {
            let item_height = bounds.size.height;
            let max_height = window.rem_size() * 2;
            let scroll_height = min(item_height, max_height);
            self.scroll_by_amount(scroll_height);
        } else {
            let scroll_height = self.line_scroll_amount(cx);
            if !scroll_height.is_zero() {
                self.scroll_by_amount(scroll_height);
            }
        }
        cx.notify();
    }

    fn scroll_up_by_item(
        &mut self,
        _: &ScrollUpByItem,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(bounds) = self
            .scroll_handle
            .bounds_for_item(self.scroll_handle.top_item())
        {
            self.scroll_by_amount(-bounds.size.height);
        }
        cx.notify();
    }

    fn scroll_down_by_item(
        &mut self,
        _: &ScrollDownByItem,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(bounds) = self
            .scroll_handle
            .bounds_for_item(self.scroll_handle.top_item())
        {
            self.scroll_by_amount(bounds.size.height);
        }
        cx.notify();
    }

    fn scroll_to_top(&mut self, _: &ScrollToTop, _window: &mut Window, cx: &mut Context<Self>) {
        self.scroll_handle.scroll_to_item(0);
        cx.notify();
    }

    fn scroll_to_bottom(
        &mut self,
        _: &ScrollToBottom,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.scroll_handle.scroll_to_bottom();
        cx.notify();
    }

    fn render_markdown_element(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> MarkdownElement {
        let workspace_directory = self.project.as_ref().and_then(|project| {
            project
                .read(cx)
                .worktrees(cx)
                .next()
                .map(|tree| tree.read(cx).abs_path().to_path_buf())
        });
        let base_directory = self.base_directory();
        let project = self.project.clone();
        let source_project_path = self.source_project_path.clone();
        let preview = cx.entity().downgrade();

        MarkdownElement::new(
            self.markdown.clone(),
            MarkdownStyle::themed(MarkdownFont::Preview, window, cx),
        )
        .input_focus_handle(self.focus_handle.clone())
        .code_block_renderer(CodeBlockRenderer::Default {
            copy_button_visibility: CopyButtonVisibility::VisibleOnHover,
            wrap_button_visibility: WrapButtonVisibility::Hidden,
            border: false,
        })
        .scroll_handle(self.scroll_handle.clone())
        .image_resolver(move |dest_url, cx| {
            resolve_preview_image(
                dest_url,
                base_directory.as_deref(),
                workspace_directory.as_deref(),
                project.as_ref(),
                source_project_path.as_ref(),
                PathStyle::local(),
                cx,
            )
        })
        .on_url_click({
            let preview = preview.clone();
            move |url, window, cx| {
                handle_url_click(url, &preview, window, cx);
            }
        })
        .on_url_hover({
            let preview = preview.clone();
            move |hovered_url, _window, cx| {
                preview
                    .update(cx, |preview, cx| {
                        if preview.hovered_url != hovered_url {
                            preview.hovered_url = hovered_url;
                            cx.notify();
                        }
                    })
                    .log_err();
            }
        })
        .on_checkbox_toggle(move |source_range, checked, window, cx| {
            toggle_task_checkbox(&preview, source_range, checked, window, cx);
        })
    }
}

fn is_preview_visible(editor: &Entity<Editor>, cx: &App) -> bool {
    editor
        .read(cx)
        .preview_layout()
        .is_some_and(|layout| layout != PreviewLayout::EditorOnly)
}

fn editor_top_source_index(editor: &Entity<Editor>, cx: &mut App) -> Option<usize> {
    editor.update(cx, |editor, cx| {
        let scroll_top = editor.scroll_position(cx).y.max(0.);
        let display_snapshot = editor.display_snapshot(cx);
        let max_point = display_snapshot.max_point();
        let top_row = DisplayRow((scroll_top.floor() as u32).min(max_point.row().0));
        let row_fraction = if top_row < max_point.row() {
            scroll_top.fract()
        } else {
            0.
        };
        let row_start = DisplayPoint::new(top_row, 0).to_offset(&display_snapshot, Bias::Left);
        let next_row_start = if top_row < max_point.row() {
            DisplayPoint::new(DisplayRow(top_row.0 + 1), 0).to_offset(&display_snapshot, Bias::Left)
        } else {
            row_start
        };
        let row_length = next_row_start.0.saturating_sub(row_start.0);
        let buffer_snapshot = display_snapshot.buffer_snapshot();
        let top_offset = buffer_snapshot.clip_offset(
            MultiBufferOffset(row_start.0 + (row_fraction * row_length as f64) as usize),
            Bias::Left,
        );
        let (_, buffer_offset) = buffer_snapshot.point_to_buffer_offset(top_offset)?;
        Some(buffer_offset.0)
    })
}

fn source_path(editor: &Editor, cx: &App) -> Option<PathBuf> {
    let file = editor.file_at(MultiBufferOffset(0), cx)?;
    let project_path = ProjectPath::from_file(file.as_ref(), cx);
    editor
        .project()
        .and_then(|project| project.read(cx).absolute_path(&project_path, cx))
        .or_else(|| file.as_local().map(|file| file.abs_path(cx)))
}

fn source_project_path(editor: &Editor, cx: &App) -> Option<ProjectPath> {
    let file = editor.file_at(MultiBufferOffset(0), cx)?;
    Some(ProjectPath::from_file(file.as_ref(), cx))
}

fn toggle_task_checkbox(
    preview: &WeakEntity<MarkdownPreview>,
    source_range: Range<usize>,
    checked: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(preview) = preview.upgrade() else {
        return;
    };
    let Some(editor) = preview.read(cx).editor.upgrade() else {
        return;
    };
    apply_checkbox_toggle_to_editor(&editor, source_range, checked, cx);
    if preview.read(cx).focus_handle.contains_focused(window, cx) {
        preview.update(cx, |preview, cx| {
            preview.update_markdown(false, false, window, cx);
        });
    }
}

fn apply_checkbox_toggle_to_editor(
    editor: &Entity<Editor>,
    source_range: Range<usize>,
    checked: bool,
    cx: &mut App,
) {
    let task_marker = if checked { "[x]" } else { "[ ]" };
    let expected_existing_marker = if checked { "[ ]" } else { "[x]" };

    editor.update(cx, |editor, cx| {
        let marker_range =
            MultiBufferOffset(source_range.start)..MultiBufferOffset(source_range.end);
        let existing_marker: String = editor
            .buffer()
            .read(cx)
            .snapshot(cx)
            .text_for_range(marker_range.clone())
            .collect();

        if !existing_marker.eq_ignore_ascii_case(expected_existing_marker) {
            return;
        }

        editor.edit([(marker_range, task_marker)], cx);
    });
}

fn handle_url_click(
    url: SharedString,
    preview: &WeakEntity<MarkdownPreview>,
    window: &mut Window,
    cx: &mut App,
) {
    let (path_part, fragment) = split_local_url_fragment(url.as_ref());

    if path_part.is_empty() {
        if let Some(fragment) = fragment {
            let preview = preview.clone();
            let slug = SharedString::from(fragment.to_string());
            window.defer(cx, move |_, cx| {
                if let Some(preview) = preview.upgrade() {
                    let markdown = preview.read(cx).markdown.clone();
                    markdown.update(cx, |markdown, cx| {
                        markdown.scroll_to_heading(&slug, cx);
                    });
                }
            });
        }
        return;
    }

    let Some(preview_entity) = preview.upgrade() else {
        return;
    };
    let base_directory = preview_entity.read(cx).base_directory();
    let workspace = preview_entity.read(cx).workspace(cx);
    open_preview_url(
        SharedString::from(path_part.to_string()),
        fragment.map(|fragment| SharedString::from(fragment.to_string())),
        base_directory,
        preview.clone(),
        workspace,
        window,
        cx,
    );
}

fn open_preview_url(
    url: SharedString,
    fragment: Option<SharedString>,
    base_directory: Option<PathBuf>,
    preview: WeakEntity<MarkdownPreview>,
    workspace: Option<Entity<Workspace>>,
    window: &mut Window,
    cx: &mut App,
) {
    let decoded_path = urlencoding::decode(&url).unwrap_or_else(|_| Cow::Borrowed(&url));
    let Some(workspace) = workspace else {
        cx.open_url(url.as_ref());
        return;
    };

    if !decoded_path.contains("://") {
        let parsed_path = PathWithPosition::parse_str(&decoded_path);
        let fragment_position = fragment
            .as_deref()
            .and_then(|fragment| fragment.strip_prefix('L'))
            .and_then(source_position_from_fragment);
        let path_with_position = if parsed_path.row.is_some() {
            Some(decoded_path.to_string())
        } else {
            fragment_position.map(|(row, column)| {
                if column == 0 {
                    format!("{decoded_path}:{}", row + 1)
                } else {
                    format!("{decoded_path}:{}:{}", row + 1, column + 1)
                }
            })
        };

        if let Some(path_with_position) = path_with_position {
            open_local_link(
                path_with_position,
                None,
                decoded_path.into_owned(),
                base_directory,
                preview,
                workspace.downgrade(),
                window,
                cx,
            );
            return;
        }

        if let Some(heading) = fragment {
            open_local_link(
                decoded_path.to_string(),
                Some(heading),
                decoded_path.into_owned(),
                base_directory,
                preview,
                workspace.downgrade(),
                window,
                cx,
            );
            return;
        }
    }

    workspace.update(cx, |workspace, cx| {
        workspace.open_url_or_file(&decoded_path, base_directory.as_deref(), window, cx);
    });
}

fn open_local_link(
    target: String,
    heading: Option<SharedString>,
    fallback_path: String,
    base_directory: Option<PathBuf>,
    preview: WeakEntity<MarkdownPreview>,
    workspace: WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    let open_target = resolve_open_target(
        &workspace,
        PathMatching::Exact,
        &target,
        base_directory.as_deref(),
        cx,
    );
    let workspace_for_error = workspace.clone();
    let task = window.spawn(cx, async move |cx| {
        let opened = match open_target.await {
            Some(open_target) => match heading {
                Some(heading) if open_target.is_file() => {
                    let target_path = open_target.path().path.clone();
                    let scrolled_in_source = preview
                        .update(cx, |preview, cx| {
                            preview.scroll_to_heading_in_source(&target_path, heading.clone(), cx)
                        })
                        .unwrap_or(false);
                    scrolled_in_source
                        || open_file_at_heading(&workspace, target_path, heading, cx).await?
                }
                _ => open_resolved_target(&workspace, &open_target, cx)
                    .await
                    .context("opening Markdown preview link")?,
            },
            None => false,
        };

        if !opened {
            workspace.update_in(cx, |workspace, window, cx| {
                workspace.open_url_or_file(&fallback_path, base_directory.as_deref(), window, cx);
            })?;
        }

        anyhow::Ok(())
    });
    task.detach_and_notify_err(workspace_for_error, window, cx);
}

async fn open_file_at_heading(
    workspace: &WeakEntity<Workspace>,
    path: PathBuf,
    heading: SharedString,
    cx: &mut AsyncWindowContext,
) -> Result<bool> {
    let opened_item = workspace
        .update_in(cx, |workspace, window, cx| {
            workspace.open_paths(
                vec![path],
                OpenOptions {
                    visible: Some(OpenVisible::OnlyDirectories),
                    ..Default::default()
                },
                None,
                window,
                cx,
            )
        })?
        .await
        .pop()
        .flatten();
    let Some(opened_item) = opened_item else {
        return Ok(false);
    };
    let opened_item = opened_item.context("opening Markdown preview link")?;
    if let Some(editor) = opened_item.downcast::<Editor>() {
        cx.update(|_, cx| scroll_markdown_preview_to_heading(&editor, heading, cx))?;
    }
    Ok(true)
}

fn scroll_markdown_preview_to_heading(
    editor: &Entity<Editor>,
    heading: SharedString,
    cx: &mut App,
) {
    if let Some(preview) = attached_markdown_preview(editor, cx) {
        preview.update(cx, |preview, cx| preview.scroll_to_heading(heading, cx));
        return;
    }
    let pending_subscription: Rc<RefCell<Option<Subscription>>> = Rc::default();
    let subscription = cx.subscribe(editor, {
        let pending_subscription = pending_subscription.clone();
        move |editor, event: &EditorEvent, cx| {
            if !matches!(event, EditorEvent::PreviewLayoutChanged) {
                return;
            }
            let Some(preview) = attached_markdown_preview(&editor, cx) else {
                return;
            };
            preview.update(cx, |preview, cx| {
                preview.scroll_to_heading(heading.clone(), cx)
            });
            *pending_subscription.borrow_mut() = None;
        }
    });
    *pending_subscription.borrow_mut() = Some(subscription);
}

fn attached_markdown_preview(editor: &Entity<Editor>, cx: &App) -> Option<Entity<MarkdownPreview>> {
    editor
        .read(cx)
        .preview_view()?
        .clone()
        .downcast::<MarkdownPreview>()
        .ok()
}

fn resolve_preview_image(
    dest_url: &str,
    base_directory: Option<&Path>,
    workspace_directory: Option<&Path>,
    project: Option<&Entity<Project>>,
    source_project_path: Option<&ProjectPath>,
    client_path_style: PathStyle,
    cx: &App,
) -> Option<ImageSource> {
    if dest_url.starts_with("data:") {
        return None;
    }

    if dest_url.starts_with("http://") || dest_url.starts_with("https://") {
        return Some(ImageSource::Resource(Resource::Uri(SharedUri::from(
            dest_url.to_string(),
        ))));
    }

    let decoded = urlencoding::decode(dest_url)
        .map(|decoded| decoded.into_owned())
        .unwrap_or_else(|_| dest_url.to_string());
    let workspace_relative_path = ['/', '\\']
        .iter()
        .find_map(|prefix| decoded.strip_prefix(*prefix));

    if let (Some(project), Some(source_project_path)) = (project, source_project_path) {
        let project_path = resolve_project_path_for_preview_image(
            &decoded,
            workspace_relative_path,
            source_project_path,
            project,
            cx,
        );
        if let Some(project_path) = project_path {
            let is_remote = project
                .read(cx)
                .worktree_for_id(project_path.worktree_id, cx)
                .is_some_and(|worktree| !worktree.read(cx).is_local());
            if is_remote {
                return Some(image_store::project_image_source(
                    project.downgrade(),
                    project_path,
                ));
            }

            let path = project.read(cx).absolute_path(&project_path, cx)?;
            return path
                .exists()
                .then(|| ImageSource::Resource(Resource::Path(Arc::from(path.as_path()))));
        } else if project.read(cx).is_remote() {
            return None;
        }
    }

    let path = if let (Some(stripped), Some(root)) = (workspace_relative_path, workspace_directory)
    {
        client_path_style.join_path(root, stripped).ok()?
    } else if client_path_style.is_absolute(&decoded) {
        PathBuf::from(decoded)
    } else {
        client_path_style.join_path(base_directory?, decoded).ok()?
    };

    path.exists()
        .then(|| ImageSource::Resource(Resource::Path(Arc::from(path.as_path()))))
}

fn resolve_project_path_for_preview_image(
    decoded_path: &str,
    workspace_relative_path: Option<&str>,
    source_project_path: &ProjectPath,
    project: &Entity<Project>,
    cx: &App,
) -> Option<ProjectPath> {
    let worktree = project
        .read(cx)
        .worktree_for_id(source_project_path.worktree_id, cx)?;
    let path_style = worktree.read(cx).path_style();

    if workspace_relative_path.is_none() && path_style.is_absolute(decoded_path) {
        return project
            .read(cx)
            .project_path_for_absolute_path(Path::new(decoded_path), cx);
    }

    let source_directory = source_project_path.path.parent()?;
    let path = if let Some(workspace_relative_path) = workspace_relative_path {
        RelPath::new(Path::new(workspace_relative_path), path_style)
            .ok()?
            .into_owned()
    } else {
        let joined_path = path_style
            .join_path(source_directory.as_std_path(), decoded_path)
            .ok()?;
        RelPath::new(&joined_path, path_style).ok()?.into_owned()
    };

    Some(ProjectPath {
        worktree_id: source_project_path.worktree_id,
        path: path.into(),
    })
}

impl Focusable for MarkdownPreview {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MarkdownPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background_color = cx.theme().colors().editor_background;
        let preview_font_size = ThemeSettings::get_global(cx).markdown_preview_font_size(cx);
        let hovered_url = self.hovered_url.clone();
        div()
            .image_cache(self.image_cache.clone())
            .id("MarkdownPreview")
            .key_context("MarkdownPreview")
            .track_focus(&self.focus_handle(cx))
            .on_hover(cx.listener(|preview, hovered, _window, cx| {
                if !hovered && preview.hovered_url.take().is_some() {
                    cx.notify();
                }
            }))
            .on_action(cx.listener(Self::scroll_page_up))
            .on_action(cx.listener(Self::scroll_page_down))
            .on_action(cx.listener(Self::scroll_up))
            .on_action(cx.listener(Self::scroll_down))
            .on_action(cx.listener(Self::scroll_up_by_item))
            .on_action(cx.listener(Self::scroll_down_by_item))
            .on_action(cx.listener(Self::scroll_to_top))
            .on_action(cx.listener(Self::scroll_to_bottom))
            .on_action(cx.listener(Self::increase_font_size))
            .on_action(cx.listener(Self::decrease_font_size))
            .on_action(cx.listener(Self::reset_font_size))
            .size_full()
            .min_h_0()
            .relative()
            .bg(background_color)
            .child(ui::background_image_layer(
                ui::BackgroundImageTarget::EditorAndTools,
                ui::BackgroundImageArea::Window,
                background_color,
                true,
                gpui::Corners::default(),
            ))
            .child(
                WithRemSize::new(preview_font_size).size_full().child(
                    div()
                        .id("markdown-preview-scroll-container")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll_handle)
                        .restrict_scroll_to_axis()
                        .p_4()
                        .child({
                            let markdown_element = self.render_markdown_element(window, cx);
                            let markdown = self.markdown.clone();
                            right_click_menu("markdown-preview-context-menu")
                                .trigger(move |_, _, _| markdown_element)
                                .maybe_menu(move |window, cx| {
                                    let focus = window.focused(cx);
                                    let markdown = markdown.read(cx);
                                    let context_menu_link = markdown.context_menu_link().cloned();
                                    let selected_text =
                                        markdown.context_menu_selected_text().cloned();
                                    let selected_markdown =
                                        markdown.context_menu_selected_markdown().cloned();
                                    if context_menu_link.is_none()
                                        && selected_text.is_none()
                                        && selected_markdown.is_none()
                                    {
                                        return None;
                                    }
                                    Some(ContextMenu::build(window, cx, move |menu, _, _cx| {
                                        menu.when_some(focus, |menu, focus| menu.context(focus))
                                            .when_some(selected_text, |menu, text| {
                                                menu.entry(
                                                    "Copy",
                                                    Some(Box::new(markdown::Copy)),
                                                    move |_, cx| {
                                                        cx.write_to_clipboard(
                                                            ClipboardItem::new_string(
                                                                text.to_string(),
                                                            ),
                                                        );
                                                    },
                                                )
                                            })
                                            .when_some(selected_markdown, |menu, text| {
                                                menu.entry(
                                                    "Copy as Markdown",
                                                    Some(Box::new(markdown::CopyAsMarkdown)),
                                                    move |_, cx| {
                                                        cx.write_to_clipboard(
                                                            ClipboardItem::new_string(
                                                                text.to_string(),
                                                            ),
                                                        );
                                                    },
                                                )
                                            })
                                            .when_some(context_menu_link, |menu, url| {
                                                menu.entry("Copy Link", None, move |_, cx| {
                                                    cx.write_to_clipboard(
                                                        ClipboardItem::new_string(url.to_string()),
                                                    );
                                                })
                                            })
                                    }))
                                })
                        }),
                ),
            )
            .custom_scrollbars(
                Scrollbars::for_settings::<EditorSettingsScrollbarProxy>()
                    .show_along(ScrollAxes::Vertical)
                    .tracked_scroll_handle(&self.scroll_handle),
                window,
                cx,
            )
            .when_some(hovered_url, |this, hovered_url| {
                this.child(
                    div()
                        .absolute()
                        .bottom_2()
                        .left_0()
                        .max_w_full()
                        .overflow_hidden()
                        .child(LinkPreview::new(hovered_url.as_ref(), cx)),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buffer_diff::BufferDiff;
    use fs::FakeFs;
    use gpui::{AppContext as _, TestAppContext, WindowHandle};
    use language::{Buffer, DiskState, Point};
    use serde_json::json;
    use util::path;
    use util::rel_path::rel_path;
    use util::test::TempTree;
    use workspace::{AppState, MultiWorkspace, SaveIntent, open_paths};

    #[gpui::test]
    async fn attaches_preview_only_to_markdown_editors(cx: &mut TestAppContext) {
        let (project, _, multi_workspace) = markdown_workspace(
            cx,
            json!({
                "note.md": "# Note\n",
                "notes.txt": "plain text\n",
            }),
        )
        .await;

        let markdown_editor = open_project_editor(cx, &project, &multi_workspace, "note.md").await;
        let text_editor = open_project_editor(cx, &project, &multi_workspace, "notes.txt").await;

        assert!(
            markdown_editor.read_with(cx, |editor, _| editor.has_preview()),
            "a Markdown editor should host the preview in the same tab"
        );
        assert!(
            !text_editor.read_with(cx, |editor, _| editor.has_preview()),
            "a plain text editor must not get a Markdown preview"
        );
    }

    #[gpui::test]
    async fn attaches_preview_when_markdown_language_is_assigned_later(cx: &mut TestAppContext) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let buffer = cx.new(|cx| Buffer::local("# Title\n", cx));
        let (editor, cx) = cx.add_window_view({
            let buffer = buffer.clone();
            move |window, cx| Editor::for_buffer(buffer, Some(project), window, cx)
        });
        cx.run_until_parked();
        assert!(
            !editor.read_with(cx, |editor, _| editor.has_preview()),
            "an editor without the Markdown language must not get a preview"
        );

        let markdown_language = Arc::new(language::Language::new(markdown_language_config(), None));
        buffer.update(cx, |buffer, cx| {
            buffer.set_language(Some(markdown_language), cx)
        });
        cx.run_until_parked();
        assert!(
            editor.read_with(cx, |editor, _| editor.has_preview()),
            "assigning the Markdown language after creation should attach the preview"
        );

        buffer.update(cx, |buffer, cx| buffer.set_language(None, cx));
        cx.run_until_parked();
        assert!(
            !editor.read_with(cx, |editor, _| editor.has_preview()),
            "the preview should be detached once the buffer is no longer Markdown"
        );
    }

    #[gpui::test]
    async fn diff_view_editors_do_not_get_a_preview(cx: &mut TestAppContext) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let markdown_language = Arc::new(language::Language::new(markdown_language_config(), None));
        let buffer = cx.new(|cx| {
            let mut buffer = Buffer::local("# Title\n", cx);
            buffer.set_language(Some(markdown_language), cx);
            buffer
        });
        let (editor, cx) = cx.add_window_view(move |window, cx| {
            let multibuffer = cx.new(|cx| {
                let mut multibuffer = editor::MultiBuffer::singleton(buffer, cx);
                multibuffer.set_all_diff_hunks_expanded(cx);
                multibuffer
            });
            Editor::for_multibuffer(multibuffer, Some(project), window, cx)
        });
        cx.run_until_parked();

        assert!(
            !editor.read_with(cx, |editor, _| editor.has_preview()),
            "a diff view showing a Markdown file must keep the plain diff editor"
        );
    }

    #[gpui::test]
    async fn edits_reach_the_preview_after_the_reparse_debounce(cx: &mut TestAppContext) {
        let previews = capture_previews(cx);
        let (project, _, multi_workspace) =
            markdown_workspace(cx, json!({ "note.md": "# Note\n" })).await;
        let editor = open_project_editor(cx, &project, &multi_workspace, "note.md").await;
        let preview = single_preview(&previews);
        assert_eq!(preview_source(cx, &preview), "# Note\n");

        editor.update(cx, |editor, cx| {
            editor.edit([(MultiBufferOffset(7)..MultiBufferOffset(7), "Body\n")], cx);
        });
        cx.executor().advance_clock(REPARSE_DEBOUNCE / 2);
        cx.run_until_parked();
        assert_eq!(
            preview_source(cx, &preview),
            "# Note\n",
            "the preview should wait for the typing debounce before reparsing"
        );

        cx.executor().advance_clock(REPARSE_DEBOUNCE);
        cx.run_until_parked();
        assert_eq!(preview_source(cx, &preview), "# Note\nBody\n");
    }

    #[gpui::test]
    async fn editor_top_source_index_interpolates_within_the_top_row(cx: &mut TestAppContext) {
        init_test(cx);
        let text = (0..100)
            .map(|row| format!("line {row:02}\n"))
            .collect::<String>();
        let buffer = cx.new(|cx| Buffer::local(text, cx));
        let (editor, cx) =
            cx.add_window_view(|window, cx| Editor::for_buffer(buffer, None, window, cx));
        cx.run_until_parked();

        editor.update_in(cx, |editor, window, cx| {
            editor.set_scroll_position(point(0., 10.5), window, cx);
        });
        let source_index = cx.update(|_, cx| editor_top_source_index(&editor, cx));

        assert_eq!(
            source_index,
            Some(10 * "line 00\n".len() + "line".len()),
            "half a row down from row 10 should map halfway into that row's source"
        );
    }

    #[gpui::test]
    async fn toggling_a_task_checkbox_edits_and_saves_the_source(cx: &mut TestAppContext) {
        let previews = capture_previews(cx);
        let app_state = init_test(cx);
        app_state.languages.add(Arc::new(language::Language::new(
            markdown_language_config(),
            None,
        )));
        app_state
            .fs
            .as_fake()
            .insert_tree(path!("/dir"), json!({ "todo.md": "- [ ] Finish work\n" }))
            .await;
        cx.update(|cx| {
            open_paths(
                &[PathBuf::from(path!("/dir/todo.md"))],
                app_state.clone(),
                workspace::OpenOptions::default(),
                cx,
            )
        })
        .await
        .unwrap();
        cx.run_until_parked();

        let multi_workspace = cx.update(|cx| cx.windows()[0].downcast::<MultiWorkspace>().unwrap());
        let preview = single_preview(&previews);
        let save_task = multi_workspace
            .update(cx, |multi_workspace, window, cx| {
                toggle_task_checkbox(&preview.downgrade(), 2..5, true, window, cx);
                multi_workspace.workspace().update(cx, |workspace, cx| {
                    workspace.save_active_item(SaveIntent::Save, window, cx)
                })
            })
            .unwrap();
        save_task.await.unwrap();
        cx.run_until_parked();

        assert_eq!(
            app_state
                .fs
                .load(path!("/dir/todo.md").as_ref())
                .await
                .unwrap(),
            "- [x] Finish work\n"
        );
    }

    #[gpui::test]
    async fn preview_uses_buffer_contents_instead_of_diff_contents(cx: &mut TestAppContext) {
        let previews = capture_previews(cx);
        let (project, _, multi_workspace) =
            markdown_workspace(cx, json!({ "note.md": "new\n" })).await;
        let editor = open_project_editor(cx, &project, &multi_workspace, "note.md").await;
        let preview = single_preview(&previews);

        editor.update(cx, |editor, cx| {
            let buffer = editor.buffer().read(cx).as_singleton().unwrap();
            let diff = cx.new(|cx| {
                BufferDiff::new_with_base_text("old\n", &buffer.read(cx).text_snapshot(), cx)
            });
            editor.buffer().update(cx, |multibuffer, cx| {
                multibuffer.add_diff(diff, cx);
                multibuffer.set_all_diff_hunks_expanded(cx);
            });
            let end = editor.buffer().read(cx).snapshot(cx).len();
            editor.edit([(end..end, "more\n")], cx);
        });
        let diff_text = editor.read_with(cx, |editor, cx| {
            editor.buffer().read(cx).snapshot(cx).text()
        });
        assert!(diff_text.contains("old"));
        cx.executor().advance_clock(REPARSE_DEBOUNCE * 2);
        cx.run_until_parked();

        assert_eq!(preview_source(cx, &preview), "new\nmore\n");
    }

    #[gpui::test]
    fn resetting_persisted_font_size_preserves_unrelated_settings(cx: &mut App) {
        let settings_store = settings::SettingsStore::test(cx);
        cx.set_global(settings_store);
        let without_markdown_preview = "{\n  \"buffer_font_size\": 14\n}\n".to_string();
        assert_eq!(
            cx.global::<settings::SettingsStore>()
                .new_text_for_update(without_markdown_preview.clone(), reset_persisted_font_size)
                .unwrap(),
            without_markdown_preview
        );

        let updated = cx
            .global::<settings::SettingsStore>()
            .new_text_for_update(
                r#"{"markdown_preview":{"font_size":18,"code_font_family":"Menlo"}}"#.into(),
                reset_persisted_font_size,
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&updated).unwrap(),
            json!({"markdown_preview": {"code_font_family": "Menlo"}})
        );
    }

    #[gpui::test]
    fn resolves_workspace_absolute_preview_image_path_and_rejects_missing(cx: &mut App) {
        let tree = TempTree::new(json!({
            "docs": {},
            "test_image.png": "mock data"
        }));
        let workspace_directory = tree.path();
        let base_directory = tree.path().join("docs");
        let image_file = workspace_directory.join("test_image.png");

        for workspace_root_relative_path in ["/test_image.png", "\\test_image.png"] {
            let resolved = resolve_preview_image(
                workspace_root_relative_path,
                Some(&base_directory),
                Some(workspace_directory),
                None,
                None,
                PathStyle::local(),
                cx,
            );
            assert_resolved_preview_image_path(resolved, image_file.as_path());
        }

        let missing = resolve_preview_image(
            "/missing_image.png",
            Some(&base_directory),
            Some(workspace_directory),
            None,
            None,
            PathStyle::local(),
            cx,
        );
        assert!(missing.is_none());
    }

    #[gpui::test]
    async fn resolves_remote_preview_image_through_project(cx: &mut TestAppContext) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let worktree = project.update(cx, |project, cx| {
            project.add_test_remote_worktree("/remote/project", cx)
        });
        let source_project_path = ProjectPath {
            worktree_id: worktree.read_with(cx, |worktree, _cx| worktree.id()),
            path: rel_path("docs/readme.md").into(),
        };

        let resolved = cx.update(|cx| {
            resolve_preview_image(
                "images/example.png",
                Some(Path::new("/remote/project/docs")),
                Some(Path::new("/remote/project")),
                Some(&project),
                Some(&source_project_path),
                PathStyle::local(),
                cx,
            )
        });
        assert!(matches!(resolved, Some(ImageSource::Custom(_))));

        let outside_worktree = cx.update(|cx| {
            resolve_preview_image(
                "../../outside/example.png",
                Some(Path::new("/remote/project/docs")),
                Some(Path::new("/remote/project")),
                Some(&project),
                Some(&source_project_path),
                PathStyle::local(),
                cx,
            )
        });
        assert!(outside_worktree.is_none());
    }

    #[gpui::test]
    async fn resolves_remote_preview_image_with_different_client_path_style(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let worktree = project.update(cx, |project, cx| {
            project.add_test_remote_worktree("/remote/project", cx)
        });
        let source_project_path = ProjectPath {
            worktree_id: worktree.read_with(cx, |worktree, _cx| worktree.id()),
            path: rel_path("docs/readme.md").into(),
        };

        let resolved = cx.update(|cx| {
            resolve_preview_image(
                "images/example.png",
                Some(Path::new("/remote/project/docs")),
                Some(Path::new("/remote/project")),
                Some(&project),
                Some(&source_project_path),
                PathStyle::Windows,
                cx,
            )
        });

        assert!(matches!(resolved, Some(ImageSource::Custom(_))));
        let project_path = cx.update(|cx| {
            resolve_project_path_for_preview_image(
                "images/example.png",
                None,
                &source_project_path,
                &project,
                cx,
            )
        });
        assert_eq!(
            project_path
                .expect("image should resolve within the remote worktree")
                .path,
            rel_path("docs/images/example.png").into()
        );
    }

    #[gpui::test]
    async fn resolves_workspace_relative_remote_image_in_source_worktree(cx: &mut TestAppContext) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let source_worktree = project.update(cx, |project, cx| {
            project.add_test_remote_worktree("/remote/other", cx);
            project.add_test_remote_worktree("/remote/project", cx)
        });
        let source_project_path = ProjectPath {
            worktree_id: source_worktree.read_with(cx, |worktree, _cx| worktree.id()),
            path: rel_path("docs/readme.md").into(),
        };

        let resolved = cx.update(|cx| {
            resolve_preview_image(
                "/images/root.png",
                Some(Path::new("/remote/project/docs")),
                Some(Path::new("/remote/other")),
                Some(&project),
                Some(&source_project_path),
                PathStyle::Windows,
                cx,
            )
        });
        assert!(matches!(resolved, Some(ImageSource::Custom(_))));

        let project_path = cx.update(|cx| {
            resolve_project_path_for_preview_image(
                "/images/root.png",
                Some("images/root.png"),
                &source_project_path,
                &project,
                cx,
            )
        });
        assert_eq!(
            project_path,
            Some(ProjectPath {
                worktree_id: source_project_path.worktree_id,
                path: rel_path("images/root.png").into(),
            })
        );
    }

    #[gpui::test]
    async fn derives_remote_preview_source_path(cx: &mut TestAppContext) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let worktree = project.update(cx, |project, cx| {
            project.add_test_remote_worktree("/remote/project", cx)
        });
        let file: Arc<dyn language::File> = Arc::new(project::File {
            worktree,
            path: rel_path("docs/readme.md").into(),
            disk_state: DiskState::New,
            entry_id: None,
            is_local: false,
            is_private: false,
        });
        let buffer = cx.new(|cx| {
            let mut buffer = Buffer::local("# readme\n", cx);
            buffer.file_updated(file, cx);
            buffer
        });
        let (editor, cx) =
            cx.add_window_view(|window, cx| Editor::for_buffer(buffer, Some(project), window, cx));

        let path = editor.read_with(cx, |editor, cx| source_path(editor, cx));
        assert_eq!(path, Some(PathBuf::from("/remote/project/docs/readme.md")));
    }

    #[gpui::test]
    async fn opens_preview_file_links_at_positions(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            path!("/project"),
            json!({
                "docs": {"guide.txt": "alpha\nbeta\ngamma\n"},
                "src": {"main.rs": "first\nsecond\nthird\n"}
            }),
        )
        .await;
        let project = Project::test(fs, [path!("/project").as_ref()], cx).await;
        register_markdown_language(&project, cx);
        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project, window, cx));
        let workspace =
            multi_workspace.read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone());
        let open_link = |url: &str, fragment: Option<&str>, cx: &mut gpui::VisualTestContext| {
            let url = SharedString::from(url.to_string());
            let fragment = fragment.map(|fragment| SharedString::from(fragment.to_string()));
            let workspace = workspace.clone();
            multi_workspace.update_in(cx, |_, window, cx| {
                open_preview_url(
                    url,
                    fragment,
                    Some(PathBuf::from(path!("/project/docs"))),
                    WeakEntity::new_invalid(),
                    Some(workspace),
                    window,
                    cx,
                );
            });
            cx.run_until_parked();
        };

        open_link("../src/main.rs:2:4", None, cx);
        let editor = workspace.read_with(cx, |workspace, cx| {
            workspace
                .active_item(cx)
                .and_then(|item| item.downcast::<Editor>())
                .expect("file should be open in an editor")
        });
        editor.update_in(cx, |editor, window, cx| {
            let snapshot = editor.snapshot(window, cx);
            assert_eq!(
                editor.selections.newest::<Point>(&snapshot).head(),
                Point::new(1, 3)
            );
        });

        open_link("../src/main.rs", Some("L3"), cx);
        editor.update_in(cx, |editor, window, cx| {
            let snapshot = editor.snapshot(window, cx);
            assert_eq!(
                editor.selections.newest::<Point>(&snapshot).head(),
                Point::new(2, 0)
            );
        });

        open_link("guide.txt:2", None, cx);
        let guide_editor = workspace.read_with(cx, |workspace, cx| {
            workspace
                .active_item(cx)
                .and_then(|item| item.downcast::<Editor>())
                .expect("guide.txt should be open in an editor")
        });
        guide_editor.update_in(cx, |editor, window, cx| {
            let file_path = editor
                .buffer()
                .read(cx)
                .as_singleton()
                .and_then(|buffer| buffer.read(cx).file().map(|file| file.path().clone()))
                .expect("opened editor should have a file");
            assert_eq!(file_path.as_ref(), rel_path("docs/guide.txt"));
            let snapshot = editor.snapshot(window, cx);
            assert_eq!(
                editor.selections.newest::<Point>(&snapshot).head(),
                Point::new(1, 0)
            );
        });

        open_link("main.rs:2", None, cx);
        assert_eq!(cx.opened_url().as_deref(), Some("main.rs:2"));
        editor.update_in(cx, |editor, window, cx| {
            let snapshot = editor.snapshot(window, cx);
            assert_eq!(
                editor.selections.newest::<Point>(&snapshot).head(),
                Point::new(2, 0),
                "broken link must not move the cursor in a suffix-matching file"
            );
        });

        open_link("tel:123", None, cx);
        assert_eq!(cx.opened_url().as_deref(), Some("tel:123"));
    }

    #[gpui::test]
    async fn opens_link_to_another_markdown_file_at_its_heading(cx: &mut TestAppContext) {
        let previews = capture_previews(cx);
        let guide = format!(
            "# Guide\n\n{}## Target\n\nDetails\n",
            (0..200)
                .map(|index| format!("Paragraph {index}\n\n"))
                .collect::<String>()
        );
        let (project, workspace, multi_workspace) = markdown_workspace(
            cx,
            json!({
                "index.md": "[Guide](guide.md#target)\n",
                "guide.md": guide,
            }),
        )
        .await;
        open_project_editor(cx, &project, &multi_workspace, "index.md").await;
        let index_preview = single_preview(&previews);

        multi_workspace
            .update(cx, |_, window, cx| {
                handle_url_click(
                    SharedString::from("guide.md#target"),
                    &index_preview.downgrade(),
                    window,
                    cx,
                );
            })
            .unwrap();
        cx.run_until_parked();

        let guide_editor = workspace.read_with(cx, |workspace, cx| {
            workspace
                .active_item(cx)
                .and_then(|item| item.downcast::<Editor>())
                .expect("the linked Markdown file should open in an editor")
        });
        let guide_file = guide_editor.read_with(cx, |editor, cx| {
            editor
                .buffer()
                .read(cx)
                .as_singleton()
                .and_then(|buffer| buffer.read(cx).file().map(|file| file.path().clone()))
        });
        assert_eq!(guide_file.as_deref(), Some(rel_path("guide.md")));
        let guide_preview = guide_editor
            .read_with(cx, |editor, _| editor.preview_view().cloned())
            .and_then(|view| view.downcast::<MarkdownPreview>().ok())
            .expect("the linked Markdown file should host a preview");

        multi_workspace
            .update(cx, |_, window, _| window.refresh())
            .unwrap();
        cx.run_until_parked();

        assert!(
            guide_preview.read_with(cx, |preview, _| preview.scroll_handle.offset().y) < px(0.),
            "the linked file's preview should scroll down to the heading named by the fragment"
        );
    }

    fn init_test(cx: &mut TestAppContext) -> Arc<AppState> {
        cx.update(|cx| {
            cx.set_global(db::AppDatabase::test_new());
            let state = AppState::test(cx);
            editor::init(cx);
            crate::init(cx);
            state
        })
    }

    fn capture_previews(cx: &mut TestAppContext) -> Rc<RefCell<Vec<Entity<MarkdownPreview>>>> {
        let previews = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let previews = previews.clone();
            cx.observe_new(move |_: &mut MarkdownPreview, _, cx| {
                previews.borrow_mut().push(cx.entity());
            })
            .detach();
        });
        previews
    }

    #[track_caller]
    fn single_preview(
        previews: &Rc<RefCell<Vec<Entity<MarkdownPreview>>>>,
    ) -> Entity<MarkdownPreview> {
        let previews = previews.borrow();
        assert_eq!(
            previews.len(),
            1,
            "exactly one Markdown preview should exist"
        );
        previews[0].clone()
    }

    fn preview_source(cx: &mut TestAppContext, preview: &Entity<MarkdownPreview>) -> String {
        preview.read_with(cx, |preview, cx| {
            preview.markdown.read(cx).source().to_string()
        })
    }

    async fn markdown_workspace(
        cx: &mut TestAppContext,
        files: serde_json::Value,
    ) -> (
        Entity<Project>,
        Entity<Workspace>,
        WindowHandle<MultiWorkspace>,
    ) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(path!("/project"), files).await;
        let project = Project::test(fs, [path!("/project").as_ref()], cx).await;
        register_markdown_language(&project, cx);
        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace
            .update(cx, |multi_workspace, _, _| {
                multi_workspace.workspace().clone()
            })
            .unwrap();
        (project, workspace, multi_workspace)
    }

    async fn open_project_editor(
        cx: &mut TestAppContext,
        project: &Entity<Project>,
        multi_workspace: &WindowHandle<MultiWorkspace>,
        path: &str,
    ) -> Entity<Editor> {
        let open = multi_workspace
            .update(cx, |multi_workspace, window, cx| {
                let worktree_id = project.read(cx).worktrees(cx).next().unwrap().read(cx).id();
                multi_workspace.workspace().update(cx, |workspace, cx| {
                    workspace.open_path(
                        ProjectPath {
                            worktree_id,
                            path: rel_path(path).into(),
                        },
                        None,
                        true,
                        window,
                        cx,
                    )
                })
            })
            .unwrap();
        let item = open.await.unwrap();
        cx.run_until_parked();
        item.downcast::<Editor>()
            .expect("project files should open in an editor")
    }

    fn markdown_language_config() -> language::LanguageConfig {
        language::LanguageConfig {
            name: "Markdown".into(),
            matcher: Arc::new(language::LanguageMatcher {
                path_suffixes: vec!["md".to_string(), "markdown".to_string()],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn register_markdown_language(project: &Entity<Project>, cx: &mut TestAppContext) {
        let languages = project.read_with(cx, |project, _| project.languages().clone());
        languages.add(Arc::new(language::Language::new(
            markdown_language_config(),
            None,
        )));
    }

    #[track_caller]
    fn assert_resolved_preview_image_path(resolved: Option<ImageSource>, expected_path: &Path) {
        match resolved {
            Some(ImageSource::Resource(Resource::Path(path))) => {
                assert_eq!(path.as_ref(), expected_path);
            }
            _ => panic!("Expected preview image to resolve to a local path"),
        }
    }
}
