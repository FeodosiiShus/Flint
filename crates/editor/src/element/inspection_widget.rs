use gpui::{
    AnyElement, App, AvailableSpace, Context, Div, Edges, Entity, Focusable as _, Hitbox, Pixels,
    Render, SharedString, Size, Stateful, WeakEntity, Window, point, px,
};
use project::ProjectItem as _;
use settings::Settings as _;
use smallvec::SmallVec;
use ui::{SurfaceKind, TOOLBAR_ICON_SIDE, Tooltip, concentric_radius, exact_icon_size, prelude::*};
use util::ResultExt as _;

use super::{EditorElement, EditorLayout};
use crate::{
    DiagnosticCounts, Editor, EditorSettings, HighlightLevel,
    actions::{GoToNextHighlightedError, GoToPreviousHighlightedError},
};

const STATUS_ICON_CELL_SIZE: Pixels = px(18.);
const TOOLBAR_BORDER: Pixels = px(2.);
const CORNER_RADIUS: Pixels = px(4.);
const BUTTON_INSET: Pixels = px(2.);
const NAVIGATION_BUTTON_SIDE_INSET: Pixels = px(1.);
const STATUS_EDGE_PADDING: Pixels = px(5.);
const SEVERITY_GROUP_GAP: Pixels = px(6.);
const ICON_COUNT_GAP: Pixels = px(1.);
const COUNT_FONT_SIZE_REDUCTION: Pixels = px(2.);
const POPUP_MIN_WIDTH: Pixels = px(296.);
const POPUP_INSET: Pixels = px(10.);
const POPUP_DETAILS_RIGHT_INSET: Pixels = px(6.);
const POPUP_LOWER_PANEL_VERTICAL_PADDING: Pixels = px(4.);

const STATUS_OFF: &str = "OFF";
const NO_PROBLEMS_FOUND: &str = "No problems found";
const NO_ANALYSIS_PERFORMED: &str = "No analysis has been performed";
const HIGHLIGHTING_DISABLED: &str = "Highlighting disabled";
const HIGHLIGHT_LABEL: &str = "Highlight:";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SeverityGroup {
    Errors,
    Warnings,
    WeakWarnings,
}

impl SeverityGroup {
    fn icon(self, full_highlighting: bool) -> IconName {
        match self {
            Self::Errors if full_highlighting => IconName::InspectionsError,
            Self::Errors => IconName::InspectionsErrorEmpty,
            Self::Warnings | Self::WeakWarnings => IconName::InspectionsWarning,
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Errors => Color::Error,
            Self::Warnings => Color::Warning,
            Self::WeakWarnings => Color::Info,
        }
    }

    fn count_message(self, count: usize) -> String {
        let noun = match self {
            Self::Errors => "error",
            Self::Warnings => "warning",
            Self::WeakWarnings => "weak warning",
        };
        if count == 1 {
            format!("{count} {noun}")
        } else {
            format!("{count} {noun}s")
        }
    }
}

fn severity_groups(counts: DiagnosticCounts) -> impl Iterator<Item = (SeverityGroup, usize)> {
    [
        (SeverityGroup::Errors, counts.errors),
        (SeverityGroup::Warnings, counts.warnings),
        (SeverityGroup::WeakWarnings, counts.weak_warnings),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
}

#[derive(Clone, Debug, PartialEq)]
struct SeverityItem {
    icon: IconName,
    color: Color,
    count: usize,
}

#[derive(Clone, Debug, PartialEq)]
enum StatusContent {
    Text(SharedString),
    Icon { icon: IconName, color: Color },
    Severities(SmallVec<[SeverityItem; 3]>),
}

#[derive(Clone, Debug, PartialEq)]
struct InspectionWidgetModel {
    status: StatusContent,
    shows_navigation: bool,
}

fn inspection_widget_model(
    level: HighlightLevel,
    counts: DiagnosticCounts,
) -> InspectionWidgetModel {
    if level == HighlightLevel::None {
        return InspectionWidgetModel {
            status: StatusContent::Text(SharedString::new_static(STATUS_OFF)),
            shows_navigation: false,
        };
    }

    let full_highlighting = level == HighlightLevel::AllProblems;
    let severities = severity_groups(counts)
        .map(|(group, count)| SeverityItem {
            icon: group.icon(full_highlighting),
            color: group.color(),
            count,
        })
        .collect::<SmallVec<[SeverityItem; 3]>>();

    if severities.is_empty() {
        let icon = if full_highlighting {
            IconName::InspectionsOk
        } else {
            IconName::InspectionsOkEmpty
        };
        InspectionWidgetModel {
            status: StatusContent::Icon {
                icon,
                color: Color::Success,
            },
            shows_navigation: false,
        }
    } else {
        InspectionWidgetModel {
            status: StatusContent::Severities(severities),
            shows_navigation: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InspectionPopupText {
    title: Option<SharedString>,
    details: Option<SharedString>,
}

fn inspection_popup_text(level: HighlightLevel, counts: DiagnosticCounts) -> InspectionPopupText {
    if level == HighlightLevel::None {
        return InspectionPopupText {
            title: Some(SharedString::new_static(NO_ANALYSIS_PERFORMED)),
            details: Some(SharedString::new_static(HIGHLIGHTING_DISABLED)),
        };
    }

    let details = severity_groups(counts)
        .map(|(group, count)| group.count_message(count))
        .collect::<Vec<_>>();
    if details.is_empty() {
        InspectionPopupText {
            title: Some(SharedString::new_static(NO_PROBLEMS_FOUND)),
            details: None,
        }
    } else {
        InspectionPopupText {
            title: None,
            details: Some(details.join(", ").into()),
        }
    }
}

fn highlight_link_text(level: HighlightLevel) -> String {
    format!("{HIGHLIGHT_LABEL} {}", level.name())
}

struct InspectionPopup {
    editor: WeakEntity<Editor>,
    levels_expanded: bool,
    dismissed: bool,
}

impl InspectionPopup {
    fn new(editor: WeakEntity<Editor>) -> Self {
        Self {
            editor,
            levels_expanded: false,
            dismissed: false,
        }
    }

    fn reopen(&mut self) {
        self.levels_expanded = false;
        self.dismissed = false;
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.dismissed = true;
        cx.notify();
    }

    fn toggle_levels(&mut self, cx: &mut Context<Self>) {
        self.levels_expanded = !self.levels_expanded;
        cx.notify();
    }

    fn apply_level(&mut self, level: HighlightLevel, window: &mut Window, cx: &mut Context<Self>) {
        self.levels_expanded = false;
        self.editor
            .update(cx, |editor, cx| {
                editor.set_highlight_level(level, window, cx)
            })
            .log_err();
        cx.notify();
    }

    fn render_level_choices(
        &self,
        current_level: HighlightLevel,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let hover_background = cx.theme().colors().ghost_element_hover;
        v_flex()
            .pt_1()
            .children(
                HighlightLevel::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(index, level)| {
                        h_flex()
                            .id(("inspection-popup-level", index))
                            .gap_1()
                            .px_1()
                            .py_0p5()
                            .rounded(CORNER_RADIUS)
                            .cursor_pointer()
                            .hover(|style| style.bg(hover_background))
                            .child(h_flex().w_4().flex_none().when(
                                level == current_level,
                                |this| {
                                    this.child(
                                        Icon::new(IconName::Check)
                                            .size(IconSize::Small)
                                            .color(Color::Accent),
                                    )
                                },
                            ))
                            .child(
                                v_flex().child(Label::new(level.name())).child(
                                    Label::new(level.description())
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                ),
                            )
                            .on_click(cx.listener(move |popup, _, window, cx| {
                                popup.apply_level(level, window, cx);
                            }))
                    }),
            )
    }
}

impl Render for InspectionPopup {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.dismissed {
            return div().into_any_element();
        }
        let Some(editor) = self.editor.upgrade() else {
            return div().into_any_element();
        };
        let (level, counts) = {
            let editor = editor.read(cx);
            (editor.highlight_level(), editor.diagnostic_counts(cx))
        };
        let InspectionPopupText { title, details } = inspection_popup_text(level, counts);
        let lower_panel_background = cx.theme().colors().surface_background;
        let popup = SurfaceKind::Popup.standard();

        let first_row_text = title.clone().or_else(|| details.clone());
        let details_row_text = title.and(details);

        v_flex()
            .id("inspection-popup")
            .min_w(POPUP_MIN_WIDTH)
            .popup_surface(cx)
            .font_ui(cx)
            .when_some(first_row_text, |this, text| {
                this.child(div().p(POPUP_INSET).child(Label::new(text)))
            })
            .when_some(details_row_text, |this, text| {
                this.child(
                    div()
                        .pl(POPUP_INSET)
                        .pr(POPUP_DETAILS_RIGHT_INSET)
                        .pb(POPUP_INSET)
                        .child(Label::new(text)),
                )
            })
            .child(
                v_flex()
                    .px(POPUP_INSET)
                    .py(POPUP_LOWER_PANEL_VERTICAL_PADDING)
                    .rounded_b(concentric_radius(popup.corner_radius, popup.border_width))
                    .bg(lower_panel_background)
                    .child(
                        h_flex()
                            .id("inspection-popup-highlight-level")
                            .gap_0p5()
                            .cursor_pointer()
                            .child(Label::new(highlight_link_text(level)).color(Color::Accent))
                            .child(
                                Icon::new(IconName::ChevronDown)
                                    .size(IconSize::XSmall)
                                    .color(Color::Accent),
                            )
                            .on_click(cx.listener(|popup, _, _, cx| popup.toggle_levels(cx))),
                    )
                    .when(self.levels_expanded, |this| {
                        this.child(self.render_level_choices(level, cx))
                    }),
            )
            .into_any_element()
    }
}

fn toolbar_button(
    id: &'static str,
    insets: Edges<Pixels>,
    content: impl IntoElement,
    cx: &App,
) -> Stateful<Div> {
    let colors = cx.theme().colors();
    let hover_background = colors.ghost_element_hover;
    let active_background = colors.ghost_element_active;
    div()
        .id(id)
        .group(id)
        .flex()
        .pt(insets.top)
        .pr(insets.right)
        .pb(insets.bottom)
        .pl(insets.left)
        .child(
            h_flex()
                .id("background")
                .min_w(STATUS_ICON_CELL_SIZE)
                .min_h(STATUS_ICON_CELL_SIZE)
                .justify_center()
                .rounded(CORNER_RADIUS)
                .group_hover(id, |style| style.bg(hover_background))
                .group_active(id, |style| style.bg(active_background))
                .child(content),
        )
}

fn icon_cell(icon: IconName, color: Color) -> impl IntoElement {
    h_flex()
        .flex_none()
        .size(STATUS_ICON_CELL_SIZE)
        .justify_center()
        .child(Icon::new(icon).size(IconSize::Medium).color(color))
}

fn status_count_font_size(cx: &App) -> Pixels {
    TextSize::Default.pixels(cx) - COUNT_FONT_SIZE_REDUCTION
}

fn render_status_content(
    status: &StatusContent,
    count_font_size: Pixels,
    rem_size: Pixels,
    cx: &App,
) -> AnyElement {
    let count_color = Color::Muted.color(cx);
    match status {
        StatusContent::Text(text) => div()
            .px(STATUS_EDGE_PADDING)
            .text_size(count_font_size)
            .text_color(count_color)
            .child(text.clone())
            .into_any_element(),
        StatusContent::Icon { icon, color } => Icon::new(*icon)
            .size(IconSize::Medium)
            .color(*color)
            .into_any_element(),
        StatusContent::Severities(severities) => h_flex()
            .px(STATUS_EDGE_PADDING)
            .gap(SEVERITY_GROUP_GAP)
            .children(severities.iter().map(|severity| {
                h_flex()
                    .gap(ICON_COUNT_GAP)
                    .child(
                        Icon::new(severity.icon)
                            .size(exact_icon_size(TOOLBAR_ICON_SIDE, rem_size))
                            .color(severity.color),
                    )
                    .child(
                        div()
                            .text_size(count_font_size)
                            .text_color(count_color)
                            .child(severity.count.to_string()),
                    )
            }))
            .into_any_element(),
    }
}

fn render_inspection_widget(
    model: &InspectionWidgetModel,
    editor: &Entity<Editor>,
    popup: &Entity<InspectionPopup>,
    count_font_size: Pixels,
    rem_size: Pixels,
    cx: &App,
) -> AnyElement {
    let focus_handle = editor.focus_handle(cx);

    let status_insets = Edges {
        top: BUTTON_INSET,
        right: if model.shows_navigation {
            Pixels::ZERO
        } else {
            BUTTON_INSET
        },
        bottom: BUTTON_INSET,
        left: BUTTON_INSET,
    };
    let status_button = toolbar_button(
        "inspection-widget-status",
        status_insets,
        render_status_content(&model.status, count_font_size, rem_size, cx),
        cx,
    )
    .hoverable_tooltip({
        let popup = popup.clone();
        move |_window, cx| {
            popup.update(cx, |popup, _| popup.reopen());
            popup.clone().into()
        }
    })
    .on_click({
        let editor = editor.clone();
        let popup = popup.clone();
        move |_event, window, cx| {
            popup.update(cx, |popup, cx| popup.dismiss(cx));
            window.focus(&editor.focus_handle(cx), cx);
            window.dispatch_action(Box::new(zed_actions::diagnostics::ToggleFileProblems), cx);
        }
    });

    h_flex()
        .id("inspection-widget")
        .p(TOOLBAR_BORDER)
        .font_ui(cx)
        .block_mouse_except_scroll()
        .child(
            h_flex()
                .items_stretch()
                .rounded(CORNER_RADIUS)
                .bg(cx.theme().colors().editor_background)
                .child(status_button)
                .when(model.shows_navigation, |this| {
                    this.child(
                        toolbar_button(
                            "inspection-widget-previous-error",
                            Edges {
                                top: BUTTON_INSET,
                                right: BUTTON_INSET,
                                bottom: BUTTON_INSET,
                                left: NAVIGATION_BUTTON_SIDE_INSET,
                            },
                            icon_cell(IconName::FindAndShowPrevMatches, Color::Default),
                            cx,
                        )
                        .tooltip({
                            let focus_handle = focus_handle.clone();
                            move |_window, cx| {
                                Tooltip::for_action_in(
                                    "Previous Highlighted Error",
                                    &GoToPreviousHighlightedError,
                                    &focus_handle,
                                    cx,
                                )
                            }
                        })
                        .on_click({
                            let editor = editor.clone();
                            move |_event, window, cx| {
                                window.focus(&editor.focus_handle(cx), cx);
                                editor.update(cx, |editor, cx| {
                                    editor.go_to_previous_highlighted_error(
                                        &GoToPreviousHighlightedError,
                                        window,
                                        cx,
                                    );
                                });
                            }
                        }),
                    )
                    .child(
                        toolbar_button(
                            "inspection-widget-next-error",
                            Edges {
                                top: BUTTON_INSET,
                                right: NAVIGATION_BUTTON_SIDE_INSET,
                                bottom: BUTTON_INSET,
                                left: NAVIGATION_BUTTON_SIDE_INSET,
                            },
                            icon_cell(IconName::FindAndShowNextMatches, Color::Default),
                            cx,
                        )
                        .tooltip({
                            let focus_handle = focus_handle.clone();
                            move |_window, cx| {
                                Tooltip::for_action_in(
                                    "Next Highlighted Error",
                                    &GoToNextHighlightedError,
                                    &focus_handle,
                                    cx,
                                )
                            }
                        })
                        .on_click({
                            let editor = editor.clone();
                            move |_event, window, cx| {
                                window.focus(&editor.focus_handle(cx), cx);
                                editor.update(cx, |editor, cx| {
                                    editor.go_to_next_highlighted_error(
                                        &GoToNextHighlightedError,
                                        window,
                                        cx,
                                    );
                                });
                            }
                        }),
                    )
                }),
        )
        .into_any_element()
}

pub(super) struct InspectionWidgetLayout {
    element: AnyElement,
    pub(super) size: Size<Pixels>,
}

impl EditorElement {
    pub(super) fn layout_inspection_widget(
        &self,
        text_hitbox: &Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<InspectionWidgetLayout> {
        if !EditorSettings::get_global(cx).show_inspection_widget {
            return None;
        }

        let editor = self.editor.read(cx);
        if !editor.mode().is_full() {
            return None;
        }
        editor.project()?;
        let buffer = editor.buffer().read(cx).as_singleton()?;
        buffer.read(cx).project_path(cx)?;
        let model = inspection_widget_model(editor.highlight_level(), editor.diagnostic_counts(cx));

        window.with_element_namespace("inspection_widget", |window| {
            let editor_handle = self.editor.downgrade();
            let popup = window.use_keyed_state("inspection-widget-popup", cx, |_, _| {
                InspectionPopup::new(editor_handle)
            });
            let rem_size = window.rem_size();
            let count_font_size = status_count_font_size(cx);
            let mut element = render_inspection_widget(
                &model,
                &self.editor,
                &popup,
                count_font_size,
                rem_size,
                cx,
            );
            let size = element.layout_as_root(AvailableSpace::min_size(), window, cx);
            (size.width <= text_hitbox.size.width)
                .then_some(InspectionWidgetLayout { element, size })
        })
    }

    pub(super) fn prepaint_inspection_widget(
        &self,
        layout: InspectionWidgetLayout,
        text_hitbox: &Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let InspectionWidgetLayout { mut element, size } = layout;
        let origin = point(
            text_hitbox.bounds.right() - size.width,
            text_hitbox.bounds.top(),
        );
        window.with_element_namespace("inspection_widget", |window| {
            element.prepaint_at(origin, window, cx);
        });
        element
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
    use crate::{
        actions::{GoToDiagnostic, ToggleDiagnostics},
        editor_tests::init_test,
        test::editor_test_context::EditorTestContext,
    };
    use gpui::{TestAppContext, UpdateGlobal as _};
    use indoc::indoc;
    use language::DiagnosticSourceKind;
    use lsp::LanguageServerId;
    use project::project_settings::DiagnosticSeverity;
    use settings::{DiagnosticSeverityContent, KeymapFile, SettingsStore};
    use util::path;

    fn counts(errors: usize, warnings: usize, weak_warnings: usize) -> DiagnosticCounts {
        DiagnosticCounts {
            errors,
            warnings,
            weak_warnings,
        }
    }

    fn severity(icon: IconName, color: Color, count: usize) -> SeverityItem {
        SeverityItem { icon, color, count }
    }

    fn lsp_diagnostic(
        line: u32,
        severity: lsp::DiagnosticSeverity,
        message: &'static str,
    ) -> lsp::Diagnostic {
        lsp::Diagnostic {
            range: lsp::Range::new(lsp::Position::new(line, 0), lsp::Position::new(line, 2)),
            severity: Some(severity),
            message: lsp::DiagnosticMessage::from(message),
            ..Default::default()
        }
    }

    fn publish_diagnostics(cx: &mut EditorTestContext, diagnostics: Vec<lsp::Diagnostic>) {
        let lsp_store =
            cx.update_editor(|editor, _, cx| editor.project().unwrap().read(cx).lsp_store());
        cx.update(|_, cx| {
            lsp_store.update(cx, |lsp_store, cx| {
                lsp_store
                    .update_diagnostics(
                        LanguageServerId(0),
                        lsp::PublishDiagnosticsParams {
                            uri: lsp::Uri::from_file_path(path!("/root/file")).unwrap(),
                            version: None,
                            diagnostics,
                        },
                        None,
                        DiagnosticSourceKind::Pushed,
                        &[],
                        cx,
                    )
                    .unwrap()
            });
        });
        cx.run_until_parked();
    }

    async fn editor_with_every_severity(cx: &mut TestAppContext) -> EditorTestContext {
        init_test(cx, |_| {});
        let mut cx = EditorTestContext::new(cx).await;
        cx.set_state(indoc! {"
            ˇone
            two
            three
            four
            five
        "});
        publish_diagnostics(
            &mut cx,
            vec![
                lsp_diagnostic(0, lsp::DiagnosticSeverity::ERROR, "first error"),
                lsp_diagnostic(1, lsp::DiagnosticSeverity::ERROR, "second error"),
                lsp_diagnostic(2, lsp::DiagnosticSeverity::WARNING, "a warning"),
                lsp_diagnostic(3, lsp::DiagnosticSeverity::INFORMATION, "an information"),
                lsp_diagnostic(4, lsp::DiagnosticSeverity::HINT, "a hint"),
            ],
        );
        cx
    }

    #[test]
    fn inspection_widget_lists_severities_errors_first_with_full_highlighting_icons() {
        let model = inspection_widget_model(HighlightLevel::AllProblems, counts(2, 3, 4));

        assert_eq!(
            model.status,
            StatusContent::Severities(SmallVec::from_vec(vec![
                severity(IconName::InspectionsError, Color::Error, 2),
                severity(IconName::InspectionsWarning, Color::Warning, 3),
                severity(IconName::InspectionsWarning, Color::Info, 4),
            ])),
            "errors, warnings and weak warnings are shown in IntelliJ order, each tinted with its severity colour"
        );
        assert!(model.shows_navigation);
    }

    #[test]
    fn inspection_widget_omits_severities_without_problems() {
        let model = inspection_widget_model(HighlightLevel::AllProblems, counts(0, 1, 0));

        assert_eq!(
            model.status,
            StatusContent::Severities(SmallVec::from_vec(vec![severity(
                IconName::InspectionsWarning,
                Color::Warning,
                1
            )])),
            "only severities with a positive count get a group"
        );
    }

    #[test]
    fn inspection_widget_without_problems_shows_a_single_ok_icon() {
        let model = inspection_widget_model(HighlightLevel::AllProblems, counts(0, 0, 0));

        assert_eq!(
            model.status,
            StatusContent::Icon {
                icon: IconName::InspectionsOk,
                color: Color::Success,
            }
        );
        assert!(
            !model.shows_navigation,
            "there is nothing to navigate to without problems"
        );
    }

    #[test]
    fn inspection_widget_at_syntax_level_uses_outline_icons() {
        let clean = inspection_widget_model(HighlightLevel::Syntax, counts(0, 0, 0));
        assert_eq!(
            clean.status,
            StatusContent::Icon {
                icon: IconName::InspectionsOkEmpty,
                color: Color::Success,
            },
            "below full highlighting the clean state uses the outline icon"
        );

        let with_errors = inspection_widget_model(HighlightLevel::Syntax, counts(1, 0, 0));
        assert_eq!(
            with_errors.status,
            StatusContent::Severities(SmallVec::from_vec(vec![severity(
                IconName::InspectionsErrorEmpty,
                Color::Error,
                1
            )])),
            "below full highlighting errors use the outline icon"
        );
        assert!(with_errors.shows_navigation);
    }

    #[test]
    fn inspection_widget_at_level_none_shows_off_without_navigation() {
        let model = inspection_widget_model(HighlightLevel::None, counts(3, 2, 1));

        assert_eq!(
            model.status,
            StatusContent::Text(SharedString::new_static("OFF")),
            "highlighting disabled shows the OFF text instead of counts"
        );
        assert!(!model.shows_navigation);
    }

    #[test]
    fn inspection_widget_popup_texts_follow_the_status() {
        assert_eq!(
            inspection_popup_text(HighlightLevel::AllProblems, counts(0, 0, 0)),
            InspectionPopupText {
                title: Some("No problems found".into()),
                details: None,
            }
        );
        assert_eq!(
            inspection_popup_text(HighlightLevel::AllProblems, counts(1, 2, 0)),
            InspectionPopupText {
                title: None,
                details: Some("1 error, 2 warnings".into()),
            }
        );
        assert_eq!(
            inspection_popup_text(HighlightLevel::AllProblems, counts(2, 1, 1)),
            InspectionPopupText {
                title: None,
                details: Some("2 errors, 1 warning, 1 weak warning".into()),
            }
        );
        assert_eq!(
            inspection_popup_text(HighlightLevel::AllProblems, counts(0, 0, 3)),
            InspectionPopupText {
                title: None,
                details: Some("3 weak warnings".into()),
            }
        );
        assert_eq!(
            inspection_popup_text(HighlightLevel::None, counts(5, 0, 0)),
            InspectionPopupText {
                title: Some("No analysis has been performed".into()),
                details: Some("Highlighting disabled".into()),
            }
        );
    }

    #[test]
    fn inspection_widget_popup_link_names_the_level() {
        assert_eq!(
            highlight_link_text(HighlightLevel::AllProblems),
            "Highlight: All Problems"
        );
        assert_eq!(
            highlight_link_text(HighlightLevel::Syntax),
            "Highlight: Syntax"
        );
        assert_eq!(highlight_link_text(HighlightLevel::None), "Highlight: None");
    }

    fn assert_active_diagnostic(
        cx: &mut EditorTestContext,
        expected: Option<&str>,
        explanation: &str,
    ) {
        cx.update_editor(|editor, _, _| {
            assert_eq!(
                editor.active_diagnostic_message(),
                expected,
                "{explanation}"
            );
        });
    }

    #[gpui::test]
    async fn inspection_widget_arrows_go_to_errors_first(cx: &mut TestAppContext) {
        let mut cx = editor_with_every_severity(cx).await;

        cx.dispatch_action(GoToNextHighlightedError);
        assert_active_diagnostic(
            &mut cx,
            Some("first error"),
            "the error at the caret comes first",
        );
        cx.dispatch_action(GoToNextHighlightedError);
        assert_active_diagnostic(&mut cx, Some("second error"), "the next error follows");
        cx.dispatch_action(GoToNextHighlightedError);
        assert_active_diagnostic(
            &mut cx,
            Some("first error"),
            "with errors in the file the warning is skipped and navigation wraps around",
        );
        cx.dispatch_action(GoToPreviousHighlightedError);
        assert_active_diagnostic(
            &mut cx,
            Some("second error"),
            "going back from the first error wraps to the last error",
        );

        publish_diagnostics(
            &mut cx,
            vec![
                lsp_diagnostic(2, lsp::DiagnosticSeverity::WARNING, "a warning"),
                lsp_diagnostic(3, lsp::DiagnosticSeverity::INFORMATION, "an information"),
                lsp_diagnostic(4, lsp::DiagnosticSeverity::HINT, "a hint"),
            ],
        );
        cx.dispatch_action(GoToNextHighlightedError);
        assert_active_diagnostic(
            &mut cx,
            Some("a warning"),
            "without errors warnings come first",
        );
        cx.dispatch_action(GoToNextHighlightedError);
        assert_active_diagnostic(
            &mut cx,
            Some("a warning"),
            "while the file has warnings the weak warning is skipped",
        );

        publish_diagnostics(
            &mut cx,
            vec![
                lsp_diagnostic(3, lsp::DiagnosticSeverity::INFORMATION, "an information"),
                lsp_diagnostic(4, lsp::DiagnosticSeverity::HINT, "a hint"),
            ],
        );
        cx.dispatch_action(GoToNextHighlightedError);
        assert_active_diagnostic(
            &mut cx,
            Some("an information"),
            "without errors and warnings weak warnings are visited",
        );
        cx.dispatch_action(GoToNextHighlightedError);
        assert_active_diagnostic(&mut cx, Some("an information"), "hints are never visited");

        publish_diagnostics(
            &mut cx,
            vec![lsp_diagnostic(4, lsp::DiagnosticSeverity::HINT, "a hint")],
        );
        cx.dispatch_action(GoToNextHighlightedError);
        cx.dispatch_action(GoToPreviousHighlightedError);
        cx.assert_editor_state(indoc! {"
            one
            two
            three
            ˇfour
            five
        "});
    }

    #[gpui::test]
    async fn jetbrains_f2_goes_to_the_next_highlighted_error(cx: &mut TestAppContext) {
        let mut cx = editor_with_every_severity(cx).await;
        cx.update(|_, cx| {
            for asset_path in ["keymaps/default-macos.json", "keymaps/macos/jetbrains.json"] {
                cx.bind_keys(KeymapFile::load_asset_allow_partial_failure(asset_path, cx).unwrap());
            }
        });

        cx.simulate_keystroke("f2");
        assert_active_diagnostic(
            &mut cx,
            Some("first error"),
            "F2 goes to the error at the caret",
        );
        cx.simulate_keystroke("f2");
        assert_active_diagnostic(&mut cx, Some("second error"), "F2 goes to the next error");
        cx.simulate_keystroke("f2");
        assert_active_diagnostic(
            &mut cx,
            Some("first error"),
            "F2 goes to errors first, like the widget's arrows, and skips the warning",
        );
        cx.simulate_keystroke("shift-f2");
        assert_active_diagnostic(
            &mut cx,
            Some("second error"),
            "Shift-F2 goes to the previous error",
        );
    }

    #[gpui::test]
    async fn inspection_widget_arrows_stay_within_the_highlight_level(cx: &mut TestAppContext) {
        let mut cx = editor_with_every_severity(cx).await;
        cx.update_editor(|editor, window, cx| {
            editor.set_highlight_level(HighlightLevel::Syntax, window, cx);
        });
        publish_diagnostics(
            &mut cx,
            vec![lsp_diagnostic(
                2,
                lsp::DiagnosticSeverity::WARNING,
                "a warning",
            )],
        );

        cx.dispatch_action(GoToNextHighlightedError);
        assert_active_diagnostic(
            &mut cx,
            None,
            "the Syntax level highlights errors only, so there is no warning to go to",
        );
        cx.assert_editor_state(indoc! {"
            ˇone
            two
            three
            four
            five
        "});
    }

    fn status_width(status: StatusContent, window: &mut Window, cx: &mut App) -> f32 {
        let rem_size = window.rem_size();
        let count_font_size = status_count_font_size(cx);
        let mut element = render_status_content(&status, count_font_size, rem_size, cx);
        f32::from(
            element
                .layout_as_root(AvailableSpace::min_size(), window, cx)
                .width,
        )
    }

    #[gpui::test]
    fn inspection_widget_severity_icons_are_16_px_without_padding(cx: &mut TestAppContext) {
        init_test(cx, |_| {});
        let cx = cx.add_empty_window();
        let error = severity(IconName::InspectionsError, Color::Error, 1);
        let warning = severity(IconName::InspectionsWarning, Color::Warning, 1);
        let mut widths = None;
        cx.draw(
            point(px(0.), px(0.)),
            AvailableSpace::min_size(),
            |window, cx| {
                // At a 15 px rem, IconSize::Medium would be 15 px instead of IntelliJ's 16 px.
                window.set_rem_size(px(15.));
                widths = Some((
                    status_width(
                        StatusContent::Text(SharedString::new_static("1")),
                        window,
                        cx,
                    ),
                    status_width(
                        StatusContent::Severities(SmallVec::from_vec(vec![error.clone()])),
                        window,
                        cx,
                    ),
                    status_width(
                        StatusContent::Severities(SmallVec::from_vec(vec![error, warning])),
                        window,
                        cx,
                    ),
                ));
                div()
            },
        );
        let (count_only, one_group, two_groups) =
            widths.expect("the status content is laid out while drawing");
        let count_width = count_only - 2. * f32::from(STATUS_EDGE_PADDING);

        assert!(
            (one_group - count_only - 17.).abs() < 0.5,
            "a group adds IntelliJ's 16 px icon and a 1 px gap to the count, got {}",
            one_group - count_only
        );
        assert!(
            (two_groups - one_group - count_width - 23.).abs() < 0.5,
            "a second group adds the 6 px group gap, the 16 px icon and the 1 px gap, got {}",
            two_groups - one_group - count_width
        );
    }

    #[gpui::test]
    async fn inspection_widget_counts_primary_diagnostics_and_ignores_hints(
        cx: &mut TestAppContext,
    ) {
        let mut cx = editor_with_every_severity(cx).await;

        cx.update_editor(|editor, _, cx| {
            assert_eq!(
                editor.diagnostic_counts(cx),
                counts(2, 1, 1),
                "information counts as a weak warning and hints are never counted"
            );
        });

        publish_diagnostics(
            &mut cx,
            vec![lsp_diagnostic(
                2,
                lsp::DiagnosticSeverity::WARNING,
                "the only warning",
            )],
        );
        cx.update_editor(|editor, _, cx| {
            assert_eq!(
                editor.diagnostic_counts(cx),
                counts(0, 1, 0),
                "counts follow a new diagnostics publication"
            );
        });
    }

    #[gpui::test]
    async fn inspection_widget_syntax_level_counts_errors_only_and_survives_settings_changes(
        cx: &mut TestAppContext,
    ) {
        let mut cx = editor_with_every_severity(cx).await;

        cx.update_editor(|editor, window, cx| {
            editor.set_highlight_level(HighlightLevel::Syntax, window, cx);
        });
        cx.update_editor(|editor, _, cx| {
            assert_eq!(editor.highlight_level(), HighlightLevel::Syntax);
            assert!(editor.diagnostics_enabled());
            assert_eq!(editor.diagnostics_max_severity, DiagnosticSeverity::Error);
            assert_eq!(
                editor.diagnostic_counts(cx),
                counts(2, 0, 0),
                "the syntax level only highlights errors"
            );
        });

        cx.update(|_, cx| {
            SettingsStore::update_global(cx, |store, cx| {
                store.update_user_settings(cx, |settings| {
                    settings.editor.diagnostics_max_severity =
                        Some(DiagnosticSeverityContent::Warning);
                });
            });
        });
        cx.run_until_parked();
        cx.update_editor(|editor, _, _| {
            assert_eq!(
                editor.highlight_level(),
                HighlightLevel::Syntax,
                "a settings change keeps the level chosen for this editor"
            );
            assert_eq!(editor.diagnostics_max_severity, DiagnosticSeverity::Error);
        });

        cx.update_editor(|editor, window, cx| {
            editor.set_highlight_level(HighlightLevel::AllProblems, window, cx);
        });
        cx.update_editor(|editor, _, cx| {
            assert_eq!(editor.highlight_level(), HighlightLevel::AllProblems);
            assert_eq!(
                editor.diagnostics_max_severity,
                DiagnosticSeverity::Warning,
                "all problems uses the configured maximum severity"
            );
            assert_eq!(editor.diagnostic_counts(cx), counts(2, 1, 0));
        });
    }

    #[gpui::test]
    async fn inspection_widget_level_none_disables_diagnostics_and_survives_settings_changes(
        cx: &mut TestAppContext,
    ) {
        let mut cx = editor_with_every_severity(cx).await;

        cx.update_editor(|editor, window, cx| {
            editor.set_highlight_level(HighlightLevel::None, window, cx);
        });
        cx.update_editor(|editor, _, _| {
            assert_eq!(editor.highlight_level(), HighlightLevel::None);
            assert!(!editor.diagnostics_enabled());
            assert_eq!(editor.diagnostics_max_severity, DiagnosticSeverity::Off);
        });

        cx.update(|_, cx| {
            SettingsStore::update_global(cx, |store, cx| {
                store.update_user_settings(cx, |settings| {
                    settings.editor.diagnostics_max_severity =
                        Some(DiagnosticSeverityContent::Hint);
                });
            });
        });
        cx.run_until_parked();
        cx.update_editor(|editor, _, _| {
            assert_eq!(
                editor.highlight_level(),
                HighlightLevel::None,
                "a settings change does not re-enable highlighting"
            );
            assert_eq!(editor.diagnostics_max_severity, DiagnosticSeverity::Off);
        });
    }

    #[gpui::test]
    async fn inspection_widget_level_stays_consistent_with_toggle_diagnostics(
        cx: &mut TestAppContext,
    ) {
        let mut cx = editor_with_every_severity(cx).await;

        cx.update_editor(|editor, window, cx| {
            editor.set_highlight_level(HighlightLevel::Syntax, window, cx);
        });
        cx.dispatch_action(ToggleDiagnostics);
        cx.update_editor(|editor, _, _| {
            assert_eq!(
                editor.highlight_level(),
                HighlightLevel::None,
                "toggling diagnostics off is the None level"
            );
            assert!(!editor.diagnostics_enabled());
        });

        cx.dispatch_action(ToggleDiagnostics);
        cx.update_editor(|editor, _, cx| {
            assert_eq!(
                editor.highlight_level(),
                HighlightLevel::AllProblems,
                "toggling diagnostics on restores All Problems"
            );
            assert!(editor.diagnostics_enabled());
            assert_eq!(editor.diagnostic_counts(cx), counts(2, 1, 1));
        });
    }

    #[gpui::test]
    async fn inspection_widget_level_none_dismisses_the_expanded_diagnostic(
        cx: &mut TestAppContext,
    ) {
        let mut cx = editor_with_every_severity(cx).await;

        cx.update_editor(|editor, window, cx| {
            editor.go_to_diagnostic(&GoToDiagnostic::default(), window, cx);
        });
        cx.update_editor(|editor, _, _| {
            assert_eq!(
                editor.active_diagnostic_message(),
                Some("first error"),
                "going to a diagnostic expands the one under the cursor"
            );
        });

        cx.update_editor(|editor, window, cx| {
            editor.set_highlight_level(HighlightLevel::None, window, cx);
        });
        cx.update_editor(|editor, _, _| {
            assert_eq!(
                editor.active_diagnostic_message(),
                None,
                "disabling highlighting must not leave the expanded diagnostic on screen"
            );
        });
    }

    #[gpui::test]
    async fn inspection_widget_chosen_all_problems_survives_settings_changes_when_diagnostics_are_off(
        cx: &mut TestAppContext,
    ) {
        let mut cx = editor_with_every_severity(cx).await;

        cx.update(|_, cx| {
            SettingsStore::update_global(cx, |store, cx| {
                store.update_user_settings(cx, |settings| {
                    settings.editor.diagnostics_max_severity = Some(DiagnosticSeverityContent::Off);
                });
            });
        });
        cx.run_until_parked();
        cx.update_editor(|editor, _, _| {
            assert_eq!(
                editor.highlight_level(),
                HighlightLevel::None,
                "an editor without a chosen level follows the configured maximum severity"
            );
        });

        cx.update_editor(|editor, window, cx| {
            editor.set_highlight_level(HighlightLevel::AllProblems, window, cx);
        });
        cx.update_editor(|editor, _, _| {
            assert_eq!(editor.highlight_level(), HighlightLevel::AllProblems);
            assert_eq!(editor.diagnostics_max_severity, DiagnosticSeverity::Hint);
        });

        cx.update(|_, cx| {
            SettingsStore::update_global(cx, |store, cx| {
                store.update_user_settings(cx, |settings| {
                    settings.editor.cursor_blink = Some(false);
                });
            });
        });
        cx.run_until_parked();
        cx.update_editor(|editor, _, _| {
            assert_eq!(
                editor.highlight_level(),
                HighlightLevel::AllProblems,
                "an unrelated settings change keeps the explicitly chosen All Problems level"
            );
            assert_eq!(editor.diagnostics_max_severity, DiagnosticSeverity::Hint);
        });
    }
}
