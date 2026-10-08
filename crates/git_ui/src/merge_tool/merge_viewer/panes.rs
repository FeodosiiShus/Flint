use std::rc::Rc;

use editor::actions::{Redo, Undo};
use gpui::{
    AnyElement, App, Context, FontWeight, Hsla, Img, IntoElement, MouseButton, Pixels,
    ScrollWheelEvent, Stateful, Window, img, point, px,
};
use merge_diff::{Side, ThreeSide};
use ui::{Tooltip, prelude::*};
use util::ResultExt as _;

use super::{
    MergeViewer,
    actions::{
        AcceptLeftSide, AcceptRightSide, ApplyNonConflictingAll, ApplyNonConflictingLeft,
        ApplyNonConflictingRight, FocusOppositePane, FocusOppositePaneAndScroll, IgnoreLeftSide,
        IgnoreRightSide, IgnoreSelectedChanges, NextConflict, NextDifference, PreviousConflict,
        PreviousDifference, ResolveSimpleConflict, ResolveSimpleConflicts, ResolveUsingLeft,
        ResolveUsingRight, RevertConflictResolution, ToggleCollapseUnchangedFragments,
        ToggleSynchronizeScrolling,
    },
    operations::PopupAction,
    scroll::PaneRows,
};
use crate::merge_tool::{
    conflict_resolution::rich_text::{RichSegment, RichStyle},
    merge_gutter::{
        MergeGutter,
        borders::{FoldInteraction, MergeEditorOverlay},
        gutter_width, max_icons_per_line,
    },
    merge_model::line_separator::{LineSeparator, separators_to_display},
    merge_ribbons::{DIVIDER_WIDTH, DividerInteraction, MergeRibbons},
    merge_window::{
        MergePaneTitle,
        chrome::{ThemedImage, themed_image},
        layout::{ComponentWidths, calc_component_widths},
    },
    palette::{current_palette, editor_background, hex_color},
};

const TITLE_INSET: f32 = 6.0;
const TITLE_DETAIL_GAP: f32 = 8.0;
const TITLE_STATUS_GAP: f32 = 4.0;
const READ_ONLY_ICON_SIZE: f32 = 16.0;
const BALLOON_ICON_SIZE: f32 = 16.0;
const SUCCESS_IMAGE_DARK: &str = "images/merge_success_dark.svg";
const SUCCESS_IMAGE_LIGHT: &str = "images/merge_success_light.svg";
pub(crate) const NOTIFICATION_HORIZONTAL_INSET: f32 = 12.0;
pub(crate) const NOTIFICATION_VERTICAL_INSET: f32 = 10.0;
pub(crate) const NOTIFICATION_ICON_SIZE: f32 = 16.0;
pub(crate) const NOTIFICATION_ICON_GAP: f32 = 8.0;
pub(crate) const NOTIFICATION_TEXT_TRAILING_GAP: f32 = 20.0;
pub(crate) const NOTIFICATION_BACKGROUND_DARK: u32 = 0x44321D;
pub(crate) const NOTIFICATION_BORDER_DARK: u32 = 0x694820;
pub(crate) const NOTIFICATION_FOREGROUND_DARK: u32 = 0xDFE1E5;
pub(crate) const NOTIFICATION_BACKGROUND_LIGHT: u32 = 0xFFF6E9;
pub(crate) const NOTIFICATION_BORDER_LIGHT: u32 = 0xF4CD9A;
pub(crate) const NOTIFICATION_FOREGROUND_LIGHT: u32 = 0x000000;
const BALLOON_TOP_OFFSET: f32 = 5.0;
const BALLOON_TITLE: &str = "All changes have been processed";
const BALLOON_LINK: &str = "Apply Changes";
const SHOW_DETAILS_LABEL: &str = "Show Details";
const HIDE_NOTIFICATION_LABEL: &str = "Hide";
const HIDE_NOTIFICATION_TOOLTIP: &str = "Hide this notification";

fn success_image(cx: &App) -> Img {
    let path = if current_palette(cx).kind.is_dark() {
        SUCCESS_IMAGE_DARK
    } else {
        SUCCESS_IMAGE_LIGHT
    };
    img(path)
        .w(px(BALLOON_ICON_SIZE))
        .h(px(BALLOON_ICON_SIZE))
        .flex_none()
}

fn icon_namespace(side: ThreeSide) -> &'static str {
    match side {
        ThreeSide::Left => "merge-gutter-icon-left",
        ThreeSide::Base => "merge-gutter-icon-result",
        ThreeSide::Right => "merge-gutter-icon-right",
    }
}

impl MergeViewer {
    pub(super) fn window_width(window: &Window) -> i32 {
        f32::from(window.viewport_size().width).floor() as i32
    }

    fn drag_divider(
        &mut self,
        side: Side,
        pointer_x: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let total_width = f32::from(window.viewport_size().width);
        self.proportions = self
            .proportions
            .dragged_to(side, f32::from(pointer_x), total_width);
        cx.notify();
    }

    fn toggle_dividers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.proportions = self.proportions.toggled(Self::window_width(window));
        cx.notify();
    }

    fn divider_interaction(&self, side: Side, cx: &mut Context<Self>) -> DividerInteraction {
        let drag_viewer = cx.weak_entity();
        let toggle_viewer = cx.weak_entity();
        DividerInteraction {
            on_drag: Rc::new(
                move |pointer_x: Pixels, window: &mut Window, cx: &mut App| {
                    drag_viewer
                        .update(cx, |viewer, cx| {
                            viewer.drag_divider(side, pointer_x, window, cx)
                        })
                        .log_err();
                },
            ),
            on_double_click: Rc::new(move |window: &mut Window, cx: &mut App| {
                toggle_viewer
                    .update(cx, |viewer, cx| viewer.toggle_dividers(window, cx))
                    .log_err();
            }),
        }
    }

    fn fold_interaction(&self, side: ThreeSide, cx: &mut Context<Self>) -> FoldInteraction {
        let hover_viewer = cx.weak_entity();
        let expand_viewer = cx.weak_entity();
        FoldInteraction {
            on_hover: Rc::new(
                move |group: Option<usize>, _window: &mut Window, cx: &mut App| {
                    hover_viewer
                        .update(cx, |viewer, cx| match group {
                            Some(group) => viewer.hover_fold(group, side, cx),
                            None => viewer.unhover_fold(side, cx),
                        })
                        .log_err();
                },
            ),
            on_expand: Rc::new(
                move |group: usize, block: usize, window: &mut Window, cx: &mut App| {
                    expand_viewer
                        .update(cx, |viewer, cx| {
                            viewer.expand_fold_block(group, block, window, cx)
                        })
                        .log_err();
                },
            ),
        }
    }

    fn undo_in_result(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.result_editor()
            .clone()
            .update(cx, |editor, cx| editor.undo(&Undo, window, cx));
    }

    fn redo_in_result(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.result_editor()
            .clone()
            .update(cx, |editor, cx| editor.redo(&Redo, window, cx));
    }

    fn render_title(&self, index: usize, width: i32, cx: &mut Context<Self>) -> AnyElement {
        let title: &MergePaneTitle = &self.titles[index];
        let read_only = index != ThreeSide::Base.index();
        let accent = cx.theme().colors().text_accent;
        let show_details = title.show_details.clone();
        let segments = h_flex().min_w_0().overflow_hidden().children(
            title
                .label
                .segments()
                .iter()
                .filter_map(|segment| match segment {
                    RichSegment::Text { text, style } => {
                        let label = Label::new(text.clone());
                        Some(match style {
                            RichStyle::Plain => label,
                            RichStyle::Bold => label.weight(FontWeight::BOLD),
                            RichStyle::Code => label.buffer_font(cx),
                        })
                    }
                    RichSegment::LineBreak => None,
                })
                .collect::<Vec<_>>(),
        );
        let secondary = title.detail.clone().map(|detail| {
            let tooltip = title.detail_tooltip.clone();
            div()
                .id(("merge-title-path", index))
                .min_w_0()
                .overflow_hidden()
                .when_some(tooltip, |element, tooltip| {
                    element.tooltip(Tooltip::text(tooltip))
                })
                .child(Label::new(detail).color(Color::Muted).truncate())
        });
        let read_only_icon =
            read_only.then(|| themed_image(ThemedImage::ReadOnly, READ_ONLY_ICON_SIZE, cx));
        let dark = current_palette(cx).kind.is_dark();
        let separators = separators_to_display(self.model.read(cx).line_separators());
        let separator_label = separators[index].map(|separator| {
            Label::new(separator.label())
                .color(Color::Custom(Self::separator_color(separator, dark)))
        });
        let center = h_flex()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .items_center()
            .when_some(read_only_icon, |row, icon| row.child(icon))
            .child(segments)
            .when_some(secondary, |row, secondary| {
                row.child(secondary.ml(px(TITLE_DETAIL_GAP)))
            })
            .when_some(show_details, |row, show_details| {
                row.child(
                    div()
                        .id(("merge-title-show-details", index))
                        .ml_auto()
                        .flex_none()
                        .cursor_pointer()
                        .text_color(accent)
                        .child(SHOW_DETAILS_LABEL)
                        .on_click(move |_, window, cx| show_details(window, cx)),
                )
            });
        h_flex()
            .w(px(width as f32))
            .flex_none()
            .overflow_hidden()
            .px(px(TITLE_INSET))
            .py(px(TITLE_INSET))
            .items_center()
            .bg(editor_background(cx))
            .child(center)
            .when_some(separator_label, |row, label| {
                row.child(div().flex_none().ml(px(TITLE_STATUS_GAP)).child(label))
            })
            .into_any_element()
    }

    fn separator_color(separator: LineSeparator, dark: bool) -> Hsla {
        hex_color(match (separator, dark) {
            (LineSeparator::Crlf, false) => 0xFF0000,
            (LineSeparator::Crlf, true) => 0xFF6464,
            (LineSeparator::Lf, false) => 0x0000FF,
            (LineSeparator::Lf, true) => 0x589DF6,
            (LineSeparator::Cr, false) => 0xFF00FF,
            (LineSeparator::Cr, true) => 0x9776A9,
        })
    }

    fn render_title_row(&self, widths: &ComponentWidths, cx: &mut Context<Self>) -> AnyElement {
        let palette = current_palette(cx);
        h_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(hex_color(palette.tearline))
            .bg(editor_background(cx))
            .child(self.render_title(0, widths.left, cx))
            .child(div().w(px(widths.left_divider as f32)).flex_none())
            .child(self.render_title(1, widths.middle, cx))
            .child(div().w(px(widths.right_divider as f32)).flex_none())
            .child(self.render_title(2, widths.right, cx))
            .into_any_element()
    }

    fn render_pane(
        &mut self,
        side: ThreeSide,
        width: i32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let index = side.index();
        let editor = self.editor(side).clone();
        let mirrored = side == ThreeSide::Left;
        let line_count = PaneRows::capture(&editor, cx).line_count().max(1) as u32;
        let icons = self.gutter_icons(side, cx);
        let changes = self.pane_changes(side, cx);
        let folded_lines = self.folded_lines(side, cx);
        let show_line_numbers = self.appearance.line_numbers;
        let gutter_pixels = gutter_width(
            window,
            cx,
            line_count,
            max_icons_per_line(&icons),
            show_line_numbers,
        );
        let fold_interaction = self.fold_interaction(side, cx);
        let gutter = MergeGutter::new(
            ("merge-gutter", index),
            icon_namespace(side),
            editor.clone(),
            mirrored,
            line_count,
            changes.clone(),
            icons,
        )
        .line_numbers_visible(show_line_numbers)
        .fold_expand(fold_interaction.on_expand.clone())
        .folded_lines(folded_lines.clone());
        let overlay = MergeEditorOverlay::new(
            ("merge-overlay", index),
            editor.clone(),
            mirrored,
            gutter_pixels,
            changes,
        )
        .folded_lines(folded_lines)
        .fold_interaction(fold_interaction);
        let balloon =
            (side == ThreeSide::Base && self.balloon_visible).then(|| self.render_balloon(cx));
        let editor_area = div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(editor)
            .child(overlay)
            .when_some(balloon, |area, balloon| area.child(balloon));
        let pane = h_flex()
            .w(px(width as f32))
            .flex_none()
            .h_full()
            .overflow_hidden()
            .bg(editor_background(cx));
        let pane = if mirrored {
            pane.child(editor_area).child(gutter)
        } else {
            pane.child(gutter).child(editor_area)
        };
        if side == ThreeSide::Base {
            return pane.into_any_element();
        }
        pane.capture_action(cx.listener(|viewer, _: &Undo, window, cx| {
            viewer.undo_in_result(window, cx);
            cx.stop_propagation();
        }))
        .capture_action(cx.listener(|viewer, _: &Redo, window, cx| {
            viewer.redo_in_result(window, cx);
            cx.stop_propagation();
        }))
        .into_any_element()
    }

    fn render_balloon(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = current_palette(cx);
        let accent = cx.theme().colors().text_accent;
        div()
            .absolute()
            .top(px(BALLOON_TOP_OFFSET))
            .left_0()
            .w_full()
            .flex()
            .justify_center()
            .child(
                v_flex()
                    .id("merge-balloon")
                    .gap(px(6.0))
                    .pt(px(12.0))
                    .pb(px(12.0))
                    .pl(px(13.0))
                    .pr(px(21.0))
                    .rounded_md()
                    .border_1()
                    .border_color(hex_color(palette.success_balloon_border))
                    .bg(hex_color(palette.success_balloon_fill))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .gap(px(6.0))
                            .items_center()
                            .child(success_image(cx))
                            .child(Label::new(BALLOON_TITLE).weight(FontWeight::BOLD)),
                    )
                    .child(
                        h_flex().pl(px(22.0)).child(
                            div()
                                .id("merge-balloon-apply")
                                .cursor_pointer()
                                .text_color(accent)
                                .child(BALLOON_LINK)
                                .on_click(cx.listener(|viewer, _, _, cx| {
                                    viewer.request_finish_from_balloon(cx);
                                })),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn render_notification(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let notification = self.visible_notification()?;
        let dark = current_palette(cx).kind.is_dark();
        let (background, border, foreground) = if dark {
            (
                NOTIFICATION_BACKGROUND_DARK,
                NOTIFICATION_BORDER_DARK,
                NOTIFICATION_FOREGROUND_DARK,
            )
        } else {
            (
                NOTIFICATION_BACKGROUND_LIGHT,
                NOTIFICATION_BORDER_LIGHT,
                NOTIFICATION_FOREGROUND_LIGHT,
            )
        };
        Some(
            h_flex()
                .w_full()
                .flex_none()
                .items_center()
                .px(px(NOTIFICATION_HORIZONTAL_INSET))
                .py(px(NOTIFICATION_VERTICAL_INSET))
                .bg(hex_color(background))
                .border_t_1()
                .border_b_1()
                .border_color(hex_color(border))
                .text_color(hex_color(foreground))
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(NOTIFICATION_ICON_GAP))
                        .items_center()
                        .child(themed_image(
                            ThemedImage::BannerWarning,
                            NOTIFICATION_ICON_SIZE,
                            cx,
                        ))
                        .child(
                            Label::new(notification.text())
                                .color(Color::Custom(hex_color(foreground))),
                        ),
                )
                .child(
                    div()
                        .id("merge-notification-hide")
                        .flex_none()
                        .ml(px(NOTIFICATION_TEXT_TRAILING_GAP))
                        .tooltip(Tooltip::text(HIDE_NOTIFICATION_TOOLTIP))
                        .cursor_pointer()
                        .text_color(cx.theme().colors().text_accent)
                        .child(HIDE_NOTIFICATION_LABEL)
                        .on_click(cx.listener(|viewer, _, _, cx| viewer.hide_notification(cx))),
                )
                .into_any_element(),
        )
    }

    fn render_panes(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let total_width = Self::window_width(window);
        let widths = calc_component_widths(total_width, self.proportions);
        let left_ribbons = self.render_ribbons(Side::Left, cx);
        let right_ribbons = self.render_ribbons(Side::Right, cx);
        let title_row = self.render_title_row(&widths, cx);
        let left_pane = self.render_pane(ThreeSide::Left, widths.left, window, cx);
        let middle_pane = self.render_pane(ThreeSide::Base, widths.middle, window, cx);
        let right_pane = self.render_pane(ThreeSide::Right, widths.right, window, cx);
        v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(title_row)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(left_pane)
                    .child(left_ribbons)
                    .child(middle_pane)
                    .child(right_ribbons)
                    .child(right_pane),
            )
            .into_any_element()
    }

    fn render_ribbons(&mut self, divider: Side, cx: &mut Context<Self>) -> AnyElement {
        let (left_side, right_side, element_id) = match divider {
            Side::Left => (ThreeSide::Left, ThreeSide::Base, "merge-ribbons-left"),
            Side::Right => (ThreeSide::Base, ThreeSide::Right, "merge-ribbons-right"),
        };
        let changes = self.ribbon_changes(divider, cx);
        let connectors = self.fold_connectors(divider, cx);
        let interaction = self.divider_interaction(divider, cx);
        let ribbons = MergeRibbons::new(
            element_id,
            self.editor(left_side).clone(),
            self.editor(right_side).clone(),
            changes,
        )
        .connectors(connectors)
        .interaction(interaction);
        div()
            .flex()
            .flex_row()
            .h_full()
            .w(px(DIVIDER_WIDTH))
            .flex_none()
            .on_scroll_wheel(cx.listener(|viewer, event: &ScrollWheelEvent, window, cx| {
                viewer.scroll_result_from_divider(event, window, cx);
                cx.stop_propagation();
            }))
            .child(ribbons)
            .into_any_element()
    }

    fn scroll_result_from_divider(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result_editor = self.result_editor().clone();
        result_editor.update(cx, |editor, cx| {
            let Some(metrics) = editor.last_text_metrics() else {
                return;
            };
            let line_height = metrics.line_height;
            if f32::from(line_height) <= 0.0 {
                return;
            }
            let pixels = event.delta.pixel_delta(line_height);
            editor.apply_scroll_delta(point(0.0, -(pixels.y / line_height)), window, cx);
        });
    }

    fn registered_viewer_actions(
        &self,
        root: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        root.on_action(cx.listener(|viewer, _: &NextDifference, window, cx| {
            viewer.navigate(false, true, window, cx);
        }))
        .on_action(cx.listener(|viewer, _: &PreviousDifference, window, cx| {
            viewer.navigate(false, false, window, cx);
        }))
        .on_action(cx.listener(|viewer, _: &NextConflict, window, cx| {
            viewer.navigate(true, true, window, cx);
        }))
        .on_action(cx.listener(|viewer, _: &PreviousConflict, window, cx| {
            viewer.navigate(true, false, window, cx);
        }))
        .on_action(cx.listener(|viewer, _: &AcceptLeftSide, _, cx| {
            viewer.perform_popup_action(PopupAction::Accept(Side::Left), viewer.current_side, cx);
        }))
        .on_action(cx.listener(|viewer, _: &AcceptRightSide, _, cx| {
            viewer.perform_popup_action(PopupAction::Accept(Side::Right), viewer.current_side, cx);
        }))
        .on_action(cx.listener(|viewer, _: &IgnoreLeftSide, _, cx| {
            viewer.perform_popup_action(
                PopupAction::IgnoreSide(Side::Left),
                viewer.current_side,
                cx,
            );
        }))
        .on_action(cx.listener(|viewer, _: &IgnoreRightSide, _, cx| {
            viewer.perform_popup_action(
                PopupAction::IgnoreSide(Side::Right),
                viewer.current_side,
                cx,
            );
        }))
        .on_action(cx.listener(|viewer, _: &ResolveUsingLeft, _, cx| {
            viewer.perform_popup_action(
                PopupAction::ResolveUsing(Side::Left),
                viewer.current_side,
                cx,
            );
        }))
        .on_action(cx.listener(|viewer, _: &ResolveUsingRight, _, cx| {
            viewer.perform_popup_action(
                PopupAction::ResolveUsing(Side::Right),
                viewer.current_side,
                cx,
            );
        }))
        .on_action(cx.listener(|viewer, _: &ResolveSimpleConflict, _, cx| {
            viewer.perform_popup_action(PopupAction::ResolveAutomatically, viewer.current_side, cx);
        }))
        .on_action(cx.listener(|viewer, _: &IgnoreSelectedChanges, _, cx| {
            viewer.perform_popup_action(PopupAction::IgnoreAll, viewer.current_side, cx);
        }))
        .on_action(
            cx.listener(|viewer, _: &ApplyNonConflictingLeft, window, cx| {
                viewer.apply_non_conflicting(ThreeSide::Left, window, cx);
            }),
        )
        .on_action(
            cx.listener(|viewer, _: &ApplyNonConflictingRight, window, cx| {
                viewer.apply_non_conflicting(ThreeSide::Right, window, cx);
            }),
        )
        .on_action(
            cx.listener(|viewer, _: &ApplyNonConflictingAll, window, cx| {
                viewer.apply_non_conflicting(ThreeSide::Base, window, cx);
            }),
        )
        .on_action(
            cx.listener(|viewer, _: &ResolveSimpleConflicts, window, cx| {
                viewer.resolve_simple_conflicts(window, cx);
            }),
        )
        .on_action(cx.listener(|viewer, _: &RevertConflictResolution, _, cx| {
            viewer.request_revert_conflict_resolution(cx);
        }))
        .on_action(
            cx.listener(|viewer, _: &ToggleSynchronizeScrolling, _, cx| {
                viewer.toggle_synchronize_scrolling(cx);
            }),
        )
        .on_action(
            cx.listener(|viewer, _: &ToggleCollapseUnchangedFragments, window, cx| {
                let collapsed = viewer.collapse_unchanged_selected();
                viewer.set_collapse_unchanged(!collapsed, window, cx);
            }),
        )
        .on_action(cx.listener(|viewer, _: &FocusOppositePane, window, cx| {
            viewer.focus_opposite_pane(false, window, cx);
        }))
        .on_action(
            cx.listener(|viewer, _: &FocusOppositePaneAndScroll, window, cx| {
                viewer.focus_opposite_pane(true, window, cx);
            }),
        )
    }
}

impl Render for MergeViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.render_header(window, cx).into_any_element();
        let notification = self.render_notification(cx);
        let body = if self.has_error_content() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(Label::new("Error"))
                .into_any_element()
        } else {
            self.render_panes(window, cx)
        };
        let root = v_flex()
            .id("merge-viewer")
            .flex_1()
            .min_h_0()
            .w_full()
            .bg(editor_background(cx))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|viewer, _, _, cx| viewer.dismiss_balloon(cx)),
            );
        self.registered_viewer_actions(root, cx)
            .child(header)
            .when_some(notification, |root, notification| root.child(notification))
            .child(body)
    }
}
