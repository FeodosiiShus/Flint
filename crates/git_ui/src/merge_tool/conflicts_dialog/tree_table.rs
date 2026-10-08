use std::ops::Range;
use std::path::Path;

use file_icons::FileIcons;
use gpui::{ClickEvent, FontWeight, Hsla, MouseButton, MouseDownEvent, rgb, rgba, uniform_list};
use merge_diff::Side;
use ui::{Tooltip, WithScrollbar, prelude::*};

use super::ConflictsDialog;
use super::messages;
use super::state::{BadgeView, GroupKind, RowView, RowViewKind};
use crate::branch_operations::opaque_elevated_surface;

pub(super) const ROW_HEIGHT: f32 = 28.;
const HEADER_HEIGHT: f32 = 24.;
const STATUS_COLUMN_WIDTH: f32 = 160.;
const STATUS_LEFT_PADDING: f32 = 6.;
const STATUS_BUTTON_GAP: f32 = 16.;
const ACCEPT_BUTTON_WIDTH: f32 = 72.;
const TREE_LEFT_PADDING: f32 = 4.;
const INDENT_WIDTH: f32 = 16.;
const DISCLOSURE_WIDTH: f32 = 16.;
const BADGE_VERTICAL_OFFSET: f32 = 6.;
const BADGE_HEIGHT: f32 = ROW_HEIGHT - 2. * BADGE_VERTICAL_OFFSET;
const DISABLED_OVERLAY_ALPHA: f32 = 0.4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BadgePalette {
    foreground: u32,
    background: u32,
}

fn badge_palette(modified: bool, light: bool) -> BadgePalette {
    match (modified, light) {
        (false, false) => BadgePalette {
            foreground: 0xB4B8BFFF,
            background: 0xB4B8BF33,
        },
        (false, true) => BadgePalette {
            foreground: 0x5A5D6BFF,
            background: 0x5A5D6B28,
        },
        (true, false) => BadgePalette {
            foreground: 0xD1E0FFFF,
            background: 0x35538FCC,
        },
        (true, true) => BadgePalette {
            foreground: 0x2E55A3FF,
            background: 0x3574F028,
        },
    }
}

fn group_icon_color(group: GroupKind, light: bool) -> u32 {
    match (group, light) {
        (GroupKind::Resolved, false) => 0x57965C,
        (GroupKind::Resolved, true) => 0x369650,
        (GroupKind::Unresolved, false) => 0xDB5C5C,
        (GroupKind::Unresolved, true) => 0xDB3B4B,
    }
}

fn indentation_width(depth: usize) -> f32 {
    depth as f32 * INDENT_WIDTH
}

fn badge_label(value: &str) -> String {
    format!("  {value}  ")
}

fn thin_spaced(text: &str) -> String {
    format!("{}{text}", messages::THIN_SPACE_SEPARATOR)
}

fn opaque_over_surface(color: Hsla, cx: &App) -> Hsla {
    opaque_elevated_surface(cx).blend(color).alpha(1.)
}

fn icon_slot(icon: Icon) -> AnyElement {
    h_flex().flex_none().pr_1().child(icon).into_any_element()
}

fn badge_capsule(badge: &BadgeView, light: bool) -> AnyElement {
    let palette = badge_palette(badge.modified, light);
    h_flex()
        .flex_none()
        .h(px(BADGE_HEIGHT))
        .rounded_full()
        .bg(Hsla::from(rgba(palette.background)))
        .child(
            Label::new(badge_label(&badge.value))
                .color(Color::Custom(Hsla::from(rgba(palette.foreground))))
                .single_line(),
        )
        .into_any_element()
}

fn header_cell(text: SharedString) -> AnyElement {
    h_flex()
        .w(px(STATUS_COLUMN_WIDTH))
        .flex_none()
        .h_full()
        .pl(px(STATUS_LEFT_PADDING))
        .overflow_hidden()
        .child(Label::new(text).color(Color::Muted).truncate())
        .into_any_element()
}

impl ConflictsDialog {
    pub(super) fn render_tree_table(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let scroll_handle = self.table_scroll.clone();
        let item_count = self.state.visible_indices().len();
        let enabled = self.table_enabled();
        let overlay_color = cx
            .theme()
            .status()
            .info_background
            .alpha(DISABLED_OVERLAY_ALPHA);
        let header = self.render_tree_header(cx);
        let rows = uniform_list(
            "conflicts-tree-rows",
            item_count,
            cx.processor(move |this, range: Range<usize>, _window, cx| {
                let row_views = this.state.row_views();
                range
                    .filter_map(|index| {
                        row_views
                            .get(index)
                            .map(|row| this.render_tree_row(index, row, cx))
                    })
                    .collect::<Vec<AnyElement>>()
            }),
        )
        .size_full()
        .track_scroll(&scroll_handle);
        let body =
            v_flex()
                .size_full()
                .child(rows)
                .vertical_scrollbar_for(&scroll_handle, window, cx);
        v_flex()
            .size_full()
            .overflow_hidden()
            .child(header)
            .child(
                div()
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .child(body)
                    .when(!enabled, |this| {
                        this.child(
                            div()
                                .id("conflicts-table-disabled-overlay")
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full()
                                .occlude()
                                .bg(overlay_color),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_tree_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let border_color = cx.theme().colors().border;
        h_flex()
            .w_full()
            .h(px(HEADER_HEIGHT))
            .flex_none()
            .border_b_1()
            .border_color(border_color)
            .child(div().flex_1().min_w_0().h_full())
            .child(header_cell(self.texts.yours_column.clone()))
            .child(header_cell(self.texts.theirs_column.clone()))
            .into_any_element()
    }

    fn render_tree_row(&self, index: usize, row: &RowView, cx: &mut Context<Self>) -> AnyElement {
        let enabled = self.table_enabled();
        let hovered = enabled && self.hovered_row.as_ref() == Some(&row.id);
        let (selected_color, hover_color) = {
            let colors = cx.theme().colors();
            (colors.element_selected, colors.element_hover)
        };
        let background = if row.selected {
            Some(opaque_over_surface(selected_color, cx))
        } else if hovered {
            Some(opaque_over_surface(hover_color, cx))
        } else {
            None
        };
        let (yours_status, theirs_status, tooltip) = match &row.kind {
            RowViewKind::File {
                yours_status,
                theirs_status,
                tooltip,
                ..
            } => (Some(*yours_status), Some(*theirs_status), tooltip.clone()),
            RowViewKind::Group { .. } | RowViewKind::Directory { .. } => (None, None, None),
        };
        let show_accept = self
            .state
            .accept_button_visible(&row.id, self.hovered_row.as_ref());
        let left_click_id = row.id.clone();
        let hover_id = row.id.clone();
        h_flex()
            .id(("conflicts-row", index))
            .w_full()
            .h(px(ROW_HEIGHT))
            .flex_none()
            .when_some(background, |this, color| this.bg(color))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.handle_row_mouse_down(
                        left_click_id.clone(),
                        event.modifiers,
                        event.click_count,
                        window,
                        cx,
                    );
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.handle_row_context_menu(event.position, window, cx);
                }),
            )
            .on_hover(cx.listener(move |this, is_hovered: &bool, _window, cx| {
                if *is_hovered {
                    this.set_hovered_row(Some(hover_id.clone()), cx);
                } else if this.hovered_row.as_ref() == Some(&hover_id) {
                    this.set_hovered_row(None, cx);
                }
            }))
            .when_some(tooltip, |this, text| this.tooltip(Tooltip::text(text)))
            .child(self.render_tree_cell(row, cx))
            .child(self.render_status_cell(index, yours_status, Side::Left, show_accept, cx))
            .child(self.render_status_cell(index, theirs_status, Side::Right, show_accept, cx))
            .into_any_element()
    }

    fn render_tree_cell(&self, row: &RowView, cx: &mut Context<Self>) -> AnyElement {
        let light = cx.theme().appearance().is_light();
        let content = match &row.kind {
            RowViewKind::Group {
                group,
                title,
                count_text,
            } => {
                let icon_name = match group {
                    GroupKind::Resolved => IconName::GreenCheckmark,
                    GroupKind::Unresolved => IconName::VcsRemove,
                };
                let icon_color = Hsla::from(rgb(group_icon_color(*group, light)));
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .child(
                        Icon::new(icon_name)
                            .size(IconSize::Small)
                            .color(Color::Custom(icon_color)),
                    )
                    .child(
                        Label::new(thin_spaced(title))
                            .weight(FontWeight::BOLD)
                            .single_line(),
                    )
                    .child(
                        Label::new(thin_spaced(count_text))
                            .color(Color::Muted)
                            .single_line(),
                    )
                    .into_any_element()
            }
            RowViewKind::Directory { label, count_text } => {
                let icon =
                    FileIcons::get_folder_icon(row.expanded.unwrap_or(true), Path::new(label), cx)
                        .map(Icon::from_path)
                        .unwrap_or_else(|| Icon::new(IconName::Folder))
                        .size(IconSize::Small)
                        .color(Color::Muted);
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .child(icon_slot(icon))
                    .child(Label::new(label.clone()).truncate())
                    .child(
                        Label::new(thin_spaced(count_text))
                            .color(Color::Muted)
                            .single_line(),
                    )
                    .into_any_element()
            }
            RowViewKind::File {
                path,
                file_name,
                parent_path,
                badge,
                ..
            } => {
                let icon = FileIcons::get_icon(path.as_std_path(), cx)
                    .map(Icon::from_path)
                    .unwrap_or_else(|| Icon::new(IconName::File))
                    .size(IconSize::Small)
                    .color(Color::Muted);
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .child(icon_slot(icon))
                    .child(Label::new(file_name.clone()).single_line())
                    .when_some(badge.as_ref(), |this, badge| {
                        this.child(Label::new(messages::THIN_SPACE_SEPARATOR).single_line())
                            .child(badge_capsule(badge, light))
                    })
                    .when_some(parent_path.as_ref(), |this, parent| {
                        this.child(
                            Label::new(thin_spaced(parent))
                                .color(Color::Muted)
                                .truncate(),
                        )
                    })
                    .into_any_element()
            }
        };
        h_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .pl(px(TREE_LEFT_PADDING))
            .child(
                div()
                    .flex_none()
                    .w(px(indentation_width(row.depth)))
                    .h_full(),
            )
            .child(self.render_disclosure(row, cx))
            .child(content)
            .into_any_element()
    }

    fn render_disclosure(&self, row: &RowView, cx: &mut Context<Self>) -> AnyElement {
        let slot = h_flex()
            .flex_none()
            .w(px(DISCLOSURE_WIDTH))
            .h_full()
            .justify_center();
        let Some(expanded) = row.expanded else {
            return slot.into_any_element();
        };
        let icon_name = if expanded {
            IconName::GeneralChevronDown
        } else {
            IconName::GeneralChevronRight
        };
        let toggle_id = row.id.clone();
        slot.cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                    this.toggle_row_expanded(toggle_id.clone(), cx);
                }),
            )
            .child(
                Icon::new(icon_name)
                    .size(IconSize::XSmall)
                    .color(Color::Muted),
            )
            .into_any_element()
    }

    fn render_status_cell(
        &self,
        index: usize,
        status: Option<&'static str>,
        side: Side,
        show_accept: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accept_id = match side {
            Side::Left => ("conflicts-accept-yours", index),
            Side::Right => ("conflicts-accept-theirs", index),
        };
        let enabled = self.table_enabled();
        h_flex()
            .w(px(STATUS_COLUMN_WIDTH))
            .flex_none()
            .h_full()
            .pl(px(STATUS_LEFT_PADDING))
            .gap(px(STATUS_BUTTON_GAP))
            .overflow_hidden()
            .when_some(status, |this, status| {
                this.child(Label::new(status).single_line())
            })
            .when(show_accept, |this| {
                this.child(
                    Button::new(accept_id, messages::ACCEPT)
                        .label_size(LabelSize::Small)
                        .width(px(ACCEPT_BUTTON_WIDTH))
                        .disabled(!enabled)
                        .on_click(cx.listener(move |this, _event: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            this.handle_inline_accept(side, window, cx);
                        })),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_tool_badge_palette_unmodified_dark_uses_grey_with_twenty_percent_background() {
        assert_eq!(
            badge_palette(false, false),
            BadgePalette {
                foreground: 0xB4B8BFFF,
                background: 0xB4B8BF33,
            }
        );
    }

    #[test]
    fn merge_tool_badge_palette_unmodified_light_uses_slate_with_sixteen_percent_background() {
        assert_eq!(
            badge_palette(false, true),
            BadgePalette {
                foreground: 0x5A5D6BFF,
                background: 0x5A5D6B28,
            }
        );
    }

    #[test]
    fn merge_tool_badge_palette_modified_dark_uses_blue_with_eighty_percent_background() {
        assert_eq!(
            badge_palette(true, false),
            BadgePalette {
                foreground: 0xD1E0FFFF,
                background: 0x35538FCC,
            }
        );
    }

    #[test]
    fn merge_tool_badge_palette_modified_light_uses_blue_with_sixteen_percent_background() {
        assert_eq!(
            badge_palette(true, true),
            BadgePalette {
                foreground: 0x2E55A3FF,
                background: 0x3574F028,
            }
        );
    }

    #[test]
    fn merge_tool_group_icon_color_distinguishes_group_and_theme() {
        assert_eq!(group_icon_color(GroupKind::Resolved, false), 0x57965C);
        assert_eq!(group_icon_color(GroupKind::Resolved, true), 0x369650);
        assert_eq!(group_icon_color(GroupKind::Unresolved, false), 0xDB5C5C);
        assert_eq!(group_icon_color(GroupKind::Unresolved, true), 0xDB3B4B);
    }

    #[test]
    fn merge_tool_badge_label_pads_value_with_two_spaces_on_each_side() {
        assert_eq!(badge_label("2/5"), "  2/5  ");
    }

    #[test]
    fn merge_tool_thin_spaced_prefixes_space_and_thin_space() {
        assert_eq!(thin_spaced("Unresolved"), " \u{2009}Unresolved");
    }
}
