use gpui::{Pixels, px};

const MENU_VERTICAL_PADDING: f32 = 7.;
// IntelliJ menu rows are `List.rowHeight` plus the 1 px `PopupMenu.Selection.outerInsets` above
// and below the pill, inside the 6 px `PopupMenu.borderInsets`, so the first pill still starts
// 7 px below the border.
const MENU_ROW_INSET_Y: f32 = 1.;

pub const POPOVER_Y_PADDING: Pixels = px(MENU_VERTICAL_PADDING * 2.);
pub const POPOVER_VERTICAL_PADDING: Pixels = px(MENU_VERTICAL_PADDING);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PopupSurfaceMetrics {
    pub corner_radius: Pixels,
    pub border_width: Pixels,
}

pub const POPUP_SURFACE_METRICS: PopupSurfaceMetrics = PopupSurfaceMetrics {
    corner_radius: crate::SurfaceKind::Popup.standard().corner_radius,
    border_width: crate::SurfaceKind::Popup.standard().border_width,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PopupRowStyle {
    #[default]
    List,
    Menu,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PopupRowMetrics {
    pub pill_height: Pixels,
    pub pill_inset_x: Pixels,
    pub pill_inset_y: Pixels,
    pub pill_radius: Pixels,
    pub content_padding_x: Pixels,
}

impl PopupRowMetrics {
    pub const fn for_style(style: PopupRowStyle) -> Self {
        match style {
            PopupRowStyle::List => Self {
                pill_height: px(24.),
                pill_inset_x: px(8.),
                pill_inset_y: px(0.),
                pill_radius: px(4.),
                content_padding_x: px(8.),
            },
            PopupRowStyle::Menu => Self {
                pill_height: px(24.),
                pill_inset_x: px(7.),
                pill_inset_y: px(MENU_ROW_INSET_Y),
                // Concentric with the 12 px popup, like macOS 26's menu selection.
                pill_radius: px(5.),
                content_padding_x: px(6.),
            },
        }
    }

    pub fn row_height(&self) -> Pixels {
        self.pill_height + self.pill_inset_y * 2.
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PopupMenuMetrics {
    pub vertical_padding: Pixels,
    pub icon_column_width: Pixels,
    pub icon_column_gap: Pixels,
    pub separator_height: Pixels,
    pub separator_line_offset: Pixels,
    pub separator_line_thickness: Pixels,
    pub separator_inset: Pixels,
}

pub const POPUP_MENU_METRICS: PopupMenuMetrics = PopupMenuMetrics {
    vertical_padding: px(MENU_VERTICAL_PADDING - MENU_ROW_INSET_Y),
    icon_column_width: px(18.),
    icon_column_gap: px(4.),
    separator_height: px(9.),
    separator_line_offset: px(4.),
    separator_line_thickness: px(1.),
    separator_inset: px(10.),
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PopupListMetrics {
    pub vertical_padding: Pixels,
}

pub const POPUP_LIST_METRICS: PopupListMetrics = PopupListMetrics {
    vertical_padding: px(8.),
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TooltipMetrics {
    pub corner_radius: Pixels,
    pub padding_top: Pixels,
    pub padding_x: Pixels,
    pub padding_bottom: Pixels,
    pub max_text_width: Pixels,
}

pub const TOOLTIP_METRICS: TooltipMetrics = TooltipMetrics {
    corner_radius: px(6.),
    padding_top: px(8.),
    padding_x: px(12.),
    padding_bottom: px(9.),
    max_text_width: px(250.),
};
