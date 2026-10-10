use gpui::{
    AnyElement, App, AvailableSpace, Entity, Focusable as _, Hitbox, Pixels, SharedString, Window,
    point, px, size,
};
use project::ProjectItem as _;
use settings::Settings as _;
use smallvec::SmallVec;
use ui::{IconButtonShape, Tooltip, prelude::*};

use super::{EditorElement, EditorLayout};
use crate::{
    Editor, EditorSettings,
    actions::{GoToDiagnostic, GoToPreviousDiagnostic},
};

const INSPECTION_WIDGET_TOP_OFFSET: Pixels = px(4.);
const INSPECTION_WIDGET_SCROLLBAR_GAP: Pixels = px(4.);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InspectionIndicator {
    Errors(usize),
    Warnings(usize),
    NoProblems,
}

impl InspectionIndicator {
    fn icon(self) -> IconName {
        match self {
            Self::Errors(_) => IconName::XCircle,
            Self::Warnings(_) => IconName::Warning,
            Self::NoProblems => IconName::Check,
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Errors(_) => Color::Error,
            Self::Warnings(_) => Color::Warning,
            Self::NoProblems => Color::Success,
        }
    }

    fn count(self) -> Option<usize> {
        match self {
            Self::Errors(count) | Self::Warnings(count) => Some(count),
            Self::NoProblems => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct InspectionWidgetModel {
    indicators: SmallVec<[InspectionIndicator; 2]>,
    shows_navigation: bool,
    tooltip: SharedString,
}

fn inspection_widget_model(error_count: usize, warning_count: usize) -> InspectionWidgetModel {
    let mut indicators: SmallVec<[InspectionIndicator; 2]> = SmallVec::new();
    if error_count > 0 {
        indicators.push(InspectionIndicator::Errors(error_count));
    }
    if warning_count > 0 {
        indicators.push(InspectionIndicator::Warnings(warning_count));
    }
    let shows_navigation = !indicators.is_empty();
    if indicators.is_empty() {
        indicators.push(InspectionIndicator::NoProblems);
    }

    InspectionWidgetModel {
        indicators,
        shows_navigation,
        tooltip: inspection_tooltip(error_count, warning_count),
    }
}

fn inspection_tooltip(error_count: usize, warning_count: usize) -> SharedString {
    match (error_count, warning_count) {
        (0, 0) => SharedString::new_static("No problems found"),
        (errors, 0) => counted_noun(errors, "error").into(),
        (0, warnings) => counted_noun(warnings, "warning").into(),
        (errors, warnings) => format!(
            "{}, {}",
            counted_noun(errors, "error"),
            counted_noun(warnings, "warning")
        )
        .into(),
    }
}

fn counted_noun(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

fn render_inspection_widget(
    model: &InspectionWidgetModel,
    editor: &Entity<Editor>,
    cx: &App,
) -> AnyElement {
    let focus_handle = editor.focus_handle(cx);

    let indicators = h_flex()
        .id("inspection-widget-indicators")
        .gap_1p5()
        .px_1()
        .children(model.indicators.iter().map(|indicator| {
            h_flex()
                .gap_0p5()
                .child(
                    Icon::new(indicator.icon())
                        .size(IconSize::XSmall)
                        .color(indicator.color()),
                )
                .when_some(indicator.count(), |this, count| {
                    this.child(Label::new(count.to_string()).size(LabelSize::Small))
                })
        }))
        .tooltip(Tooltip::text(model.tooltip.clone()));

    h_flex()
        .id("inspection-widget")
        .font_ui(cx)
        .gap_0p5()
        .block_mouse_except_scroll()
        .child(indicators)
        .when(model.shows_navigation, |this| {
            this.child(
                IconButton::new("inspection-widget-previous-error", IconName::ChevronUp)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::XSmall)
                    .style(ButtonStyle::Subtle)
                    .tooltip({
                        let focus_handle = focus_handle.clone();
                        move |_window, cx| {
                            Tooltip::for_action_in(
                                "Previous Highlighted Error",
                                &GoToPreviousDiagnostic::default(),
                                &focus_handle,
                                cx,
                            )
                        }
                    })
                    .on_click({
                        let editor = editor.clone();
                        move |_event, window, cx| {
                            editor.update(cx, |editor, cx| {
                                editor.go_to_prev_diagnostic(
                                    &GoToPreviousDiagnostic::default(),
                                    window,
                                    cx,
                                );
                            });
                        }
                    }),
            )
            .child(
                IconButton::new("inspection-widget-next-error", IconName::ChevronDown)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::XSmall)
                    .style(ButtonStyle::Subtle)
                    .tooltip({
                        let focus_handle = focus_handle.clone();
                        move |_window, cx| {
                            Tooltip::for_action_in(
                                "Next Highlighted Error",
                                &GoToDiagnostic::default(),
                                &focus_handle,
                                cx,
                            )
                        }
                    })
                    .on_click({
                        let editor = editor.clone();
                        move |_event, window, cx| {
                            editor.update(cx, |editor, cx| {
                                editor.go_to_diagnostic(&GoToDiagnostic::default(), window, cx);
                            });
                        }
                    }),
            )
        })
        .into_any_element()
}

impl EditorElement {
    pub(super) fn layout_inspection_widget(
        &self,
        text_hitbox: &Hitbox,
        right_margin: Pixels,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        if !EditorSettings::get_global(cx).show_inspection_widget {
            return None;
        }

        let editor = self.editor.read(cx);
        if !editor.mode().is_full() {
            return None;
        }
        let project = editor.project()?;
        let buffer = editor.buffer().read(cx).as_singleton()?;
        let project_path = buffer.read(cx).project_path(cx)?;
        let summary = project
            .read(cx)
            .diagnostic_summary_for_path(&project_path, cx);

        let model = inspection_widget_model(summary.error_count, summary.warning_count);
        let mut widget = render_inspection_widget(&model, &self.editor, cx);
        let available_space = size(AvailableSpace::MinContent, AvailableSpace::MinContent);
        let widget_size = widget.layout_as_root(available_space, window, cx);
        let origin = point(
            text_hitbox.bounds.right()
                - right_margin
                - INSPECTION_WIDGET_SCROLLBAR_GAP
                - widget_size.width,
            text_hitbox.bounds.top() + INSPECTION_WIDGET_TOP_OFFSET,
        );
        if origin.x < text_hitbox.bounds.left() {
            return None;
        }

        widget.prepaint_as_root(origin, available_space, window, cx);
        Some(widget)
    }

    pub(super) fn paint_inspection_widget(
        &self,
        layout: &mut EditorLayout,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(widget) = layout.inspection_widget.as_mut() {
            window.with_element_namespace("inspection_widget", |window| {
                widget.paint(window, cx);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_widget_without_problems_shows_a_single_check() {
        let model = inspection_widget_model(0, 0);

        assert_eq!(
            model.indicators.as_slice(),
            &[InspectionIndicator::NoProblems]
        );
        assert!(
            !model.shows_navigation,
            "there is nothing to navigate to without problems"
        );
        assert_eq!(model.tooltip, "No problems found");
        assert_eq!(InspectionIndicator::NoProblems.icon(), IconName::Check);
        assert_eq!(InspectionIndicator::NoProblems.color(), Color::Success);
        assert_eq!(InspectionIndicator::NoProblems.count(), None);
    }

    #[test]
    fn inspection_widget_with_errors_only_shows_the_error_count() {
        let model = inspection_widget_model(3, 0);

        assert_eq!(
            model.indicators.as_slice(),
            &[InspectionIndicator::Errors(3)]
        );
        assert!(model.shows_navigation);
        assert_eq!(model.tooltip, "3 errors");
        assert_eq!(InspectionIndicator::Errors(3).icon(), IconName::XCircle);
        assert_eq!(InspectionIndicator::Errors(3).color(), Color::Error);
        assert_eq!(InspectionIndicator::Errors(3).count(), Some(3));
    }

    #[test]
    fn inspection_widget_with_warnings_only_shows_the_warning_count() {
        let model = inspection_widget_model(0, 1);

        assert_eq!(
            model.indicators.as_slice(),
            &[InspectionIndicator::Warnings(1)]
        );
        assert!(model.shows_navigation);
        assert_eq!(model.tooltip, "1 warning");
        assert_eq!(InspectionIndicator::Warnings(1).icon(), IconName::Warning);
        assert_eq!(InspectionIndicator::Warnings(1).color(), Color::Warning);
        assert_eq!(InspectionIndicator::Warnings(1).count(), Some(1));
    }

    #[test]
    fn inspection_widget_with_errors_and_warnings_lists_errors_first() {
        let model = inspection_widget_model(2, 1);

        assert_eq!(
            model.indicators.as_slice(),
            &[
                InspectionIndicator::Errors(2),
                InspectionIndicator::Warnings(1)
            ]
        );
        assert!(model.shows_navigation);
        assert_eq!(model.tooltip, "2 errors, 1 warning");
    }

    #[test]
    fn inspection_widget_shows_navigation_only_with_problems() {
        assert!(!inspection_widget_model(0, 0).shows_navigation);
        assert!(inspection_widget_model(1, 0).shows_navigation);
        assert!(inspection_widget_model(0, 1).shows_navigation);
        assert!(inspection_widget_model(1, 1).shows_navigation);
    }
}
