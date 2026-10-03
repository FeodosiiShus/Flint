use std::ops::Range;

use editor::{Anchor, Editor, SelectionEffects, scroll::Autoscroll};
use gpui::{Entity, Focusable, Subscription, TextRun, WeakEntity};
use project::{Project, ProjectPath};
use settings::Settings;
use ui::{Tooltip, prelude::*};
use workspace::{
    HideStatusItem, ItemHandle, StatusBarSettings, StatusItemView, Workspace, item::ItemEvent,
};

const SEGMENT_SEPARATOR: &str = "›";
const TRUNCATION_MARKER: &str = "…";
const MIN_VISIBLE_SEGMENTS: usize = 2;
const MAX_WINDOW_WIDTH_FRACTION: f32 = 0.5;
const SEGMENT_HORIZONTAL_PADDING: Pixels = px(8.);
const SEGMENT_GAP: Pixels = px(2.);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavigationTarget {
    Entry(ProjectPath),
    Symbol(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationSegment {
    pub label: SharedString,
    pub target: NavigationTarget,
}

pub fn build_segments(
    entry: Option<(&ProjectPath, &str)>,
    symbol_labels: impl IntoIterator<Item = SharedString>,
) -> Vec<NavigationSegment> {
    let mut segments = Vec::new();
    if let Some((project_path, root_name)) = entry {
        let ancestors = project_path.path.ancestors().collect::<Vec<_>>();
        segments.extend(ancestors.into_iter().rev().map(|ancestor| {
            let label = ancestor.file_name().unwrap_or(root_name);
            NavigationSegment {
                label: SharedString::from(label.to_string()),
                target: NavigationTarget::Entry(ProjectPath {
                    worktree_id: project_path.worktree_id,
                    path: ancestor.into_arc(),
                }),
            }
        }));
    }
    segments.extend(symbol_labels.into_iter().enumerate().map(|(index, label)| {
        NavigationSegment {
            label,
            target: NavigationTarget::Symbol(index),
        }
    }));
    segments
}

pub fn hidden_middle_range(
    segment_widths: &[Pixels],
    separator_width: Pixels,
    truncation_marker_width: Pixels,
    available_width: Pixels,
) -> Option<Range<usize>> {
    if row_width(segment_widths.iter().copied(), separator_width) <= available_width {
        return None;
    }
    let segment_count = segment_widths.len();
    if segment_count <= MIN_VISIBLE_SEGMENTS {
        return None;
    }
    let max_hidden_count = segment_count - MIN_VISIBLE_SEGMENTS;
    (1..=max_hidden_count)
        .map(|hidden_count| middle_range(segment_count, hidden_count))
        .find(|hidden| {
            let visible_widths = segment_widths
                .iter()
                .enumerate()
                .filter(|(index, _)| !hidden.contains(index))
                .map(|(_, width)| *width)
                .chain(Some(truncation_marker_width));
            row_width(visible_widths, separator_width) <= available_width
        })
        .or_else(|| Some(middle_range(segment_count, max_hidden_count)))
}

fn middle_range(segment_count: usize, hidden_count: usize) -> Range<usize> {
    let visible_count = segment_count - hidden_count;
    let tail_count = visible_count / 2;
    let head_count = visible_count - tail_count;
    head_count..head_count + hidden_count
}

fn row_width(widths: impl Iterator<Item = Pixels>, separator_width: Pixels) -> Pixels {
    let mut item_count: usize = 0;
    let mut total = Pixels::ZERO;
    for width in widths {
        total += width;
        item_count += 1;
    }
    total + separator_width * item_count.saturating_sub(1) as f32
}

pub struct NavigationBar {
    project: Entity<Project>,
    active_item: Option<Box<dyn ItemHandle>>,
    active_editor: Option<WeakEntity<Editor>>,
    segments: Vec<NavigationSegment>,
    symbol_anchors: Vec<Anchor>,
    _item_events: Option<Subscription>,
}

impl NavigationBar {
    pub fn new(workspace: &Workspace) -> Self {
        Self {
            project: workspace.project().clone(),
            active_item: None,
            active_editor: None,
            segments: Vec::new(),
            symbol_anchors: Vec::new(),
            _item_events: None,
        }
    }

    pub fn segments(&self) -> &[NavigationSegment] {
        &self.segments
    }

    pub fn navigate(
        &mut self,
        target: &NavigationTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            NavigationTarget::Entry(project_path) => {
                let entry_id = self
                    .project
                    .read(cx)
                    .entry_for_path(project_path, cx)
                    .map(|entry| entry.id);
                if let Some(entry_id) = entry_id {
                    self.project.update(cx, |_, cx| {
                        cx.emit(project::Event::RevealInProjectPanel(entry_id));
                    });
                }
            }
            NavigationTarget::Symbol(index) => {
                let Some(anchor) = self.symbol_anchors.get(*index).copied() else {
                    return;
                };
                let Some(editor) = self
                    .active_editor
                    .as_ref()
                    .and_then(|editor| editor.upgrade())
                else {
                    return;
                };
                editor.update(cx, |editor, cx| {
                    editor.change_selections(
                        SelectionEffects::scroll(Autoscroll::center()),
                        window,
                        cx,
                        |selections| selections.select_anchor_ranges([anchor..anchor]),
                    );
                    window.focus(&editor.focus_handle(cx), cx);
                });
            }
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.segments.clear();
        self.symbol_anchors.clear();
        self.active_editor = None;
        if let Some(item) = self.active_item.as_ref() {
            let project_path = item.project_path(cx);
            let root_name = project_path
                .as_ref()
                .and_then(|project_path| {
                    self.project
                        .read(cx)
                        .worktree_for_id(project_path.worktree_id, cx)
                })
                .map(|worktree| worktree.read(cx).root_name_str().to_string());
            let entry = project_path.as_ref().zip(root_name.as_deref());

            let mut symbol_labels = Vec::new();
            if let Some(editor) = item.act_as::<Editor>(cx) {
                for symbol in editor.read(cx).outline_symbols_at_cursor(cx) {
                    symbol_labels.push(SharedString::from(symbol.text.replace('\n', " ")));
                    self.symbol_anchors.push(symbol.selection_range.start);
                }
                self.active_editor = Some(editor.downgrade());
            }
            self.segments = build_segments(entry, symbol_labels);
        }
        cx.notify();
    }

    fn hidden_range(&self, window: &Window, cx: &App) -> Option<Range<usize>> {
        let font_size = TextSize::Small.rems(cx).to_pixels(window.rem_size());
        let font = theme::theme_settings(cx).ui_font(cx).clone();
        let measure = |text: &str| -> Pixels {
            let run = TextRun {
                len: text.len(),
                font: font.clone(),
                ..TextRun::default()
            };
            window
                .text_system()
                .shape_line(
                    SharedString::from(text.to_string()),
                    font_size,
                    &[run],
                    None,
                )
                .width()
        };
        let segment_widths = self
            .segments
            .iter()
            .map(|segment| measure(segment.label.as_ref()) + SEGMENT_HORIZONTAL_PADDING)
            .collect::<Vec<_>>();
        let separator_width = measure(SEGMENT_SEPARATOR) + SEGMENT_GAP * 2.;
        let truncation_marker_width = measure(TRUNCATION_MARKER);
        let available_width = window.viewport_size().width * MAX_WINDOW_WIDTH_FRACTION;
        hidden_middle_range(
            &segment_widths,
            separator_width,
            truncation_marker_width,
            available_width,
        )
    }

    fn render_segment(
        &self,
        index: usize,
        segment: &NavigationSegment,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tooltip = match segment.target {
            NavigationTarget::Entry(_) => "Reveal in Project Panel",
            NavigationTarget::Symbol(_) => "Go to Symbol",
        };
        let target = segment.target.clone();
        Button::new(("navigation-bar-segment", index), segment.label.clone())
            .label_size(LabelSize::Small)
            .chrome_region(ui::ChromeRegion::StatusBar)
            .tab_index(0isize)
            .tooltip(Tooltip::text(tooltip))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.navigate(&target, window, cx);
            }))
            .into_any_element()
    }
}

impl Render for NavigationBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let container = h_flex()
            .id("navigation-bar")
            .min_w_0()
            .overflow_x_hidden()
            .gap(SEGMENT_GAP);
        if !StatusBarSettings::get_global(cx).navigation_bar || self.segments.is_empty() {
            return container;
        }

        let hidden = self.hidden_range(window, cx);
        let mut children: Vec<AnyElement> = Vec::new();
        for (index, segment) in self.segments.iter().enumerate() {
            let is_hidden = hidden
                .as_ref()
                .is_some_and(|hidden| hidden.contains(&index));
            let starts_hidden_range = hidden.as_ref().is_some_and(|hidden| hidden.start == index);
            if is_hidden && !starts_hidden_range {
                continue;
            }
            if !children.is_empty() {
                children.push(
                    Label::new(SEGMENT_SEPARATOR)
                        .size(LabelSize::Small)
                        .color(Color::Placeholder)
                        .into_any_element(),
                );
            }
            if is_hidden {
                children.push(
                    Label::new(TRUNCATION_MARKER)
                        .size(LabelSize::Small)
                        .color(Color::Muted)
                        .into_any_element(),
                );
            } else {
                children.push(self.render_segment(index, segment, cx));
            }
        }
        container.children(children)
    }
}

impl StatusItemView for NavigationBar {
    fn set_active_pane_item(
        &mut self,
        active_pane_item: Option<&dyn ItemHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_item = active_pane_item.map(|item| item.boxed_clone());
        self._item_events = active_pane_item.map(|item| {
            let this = cx.entity().downgrade();
            item.subscribe_to_item_events(
                window,
                cx,
                Box::new(move |event, _, cx| {
                    if matches!(event, ItemEvent::UpdateBreadcrumbs | ItemEvent::UpdateTab) {
                        this.update(cx, |this, cx| this.refresh(cx)).ok();
                    }
                }),
            )
        });
        self.refresh(cx);
    }

    fn hide_setting(&self, _: &App) -> Option<HideStatusItem> {
        Some(HideStatusItem::new(|settings| {
            settings.status_bar.get_or_insert_default().navigation_bar = Some(false);
        }))
    }
}
