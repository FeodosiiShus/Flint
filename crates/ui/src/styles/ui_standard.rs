use gpui::{Pixels, Rems, Size, px, rems, size};

use crate::{BALLOON_METRICS, ElevationIndex, PopupRowMetrics, PopupRowStyle, TOOLTIP_METRICS};

/// Every surface that floats above the editor and the tool windows is one of these kinds, and
/// each kind is drawn from one [`SurfaceStandard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SurfaceKind {
    /// Popup menus, context menus, pickers, popovers and the branches popup.
    Popup,
    /// Modal dialogs.
    Dialog,
    /// Notification balloons.
    Balloon,
    /// Tooltips.
    Tooltip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceMaterial {
    /// macOS 26's "regular" Liquid Glass: the content behind the surface is blurred and the
    /// surface colour is laid over it.
    Glass,
    /// macOS 26 keeps its own tooltips off glass, and so does Flint.
    Opaque,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceStandard {
    pub corner_radius: Pixels,
    pub border_width: Pixels,
    /// The inset of the controls drawn inside the surface. Their corners are concentric with the
    /// surface: inner radius = [`Self::corner_radius`] − inset.
    pub content_inset: Pixels,
    /// The smallest gap kept between the surface and the window edges.
    pub viewport_margin: Pixels,
    pub material: SurfaceMaterial,
    pub elevation: ElevationIndex,
}

/// macOS 26 context menus are 12 px glass with concentric selection pills; IntelliJ's row insets
/// (8 px for lists, 7 px for menus) then give its 4 px list pill.
const POPUP_RADIUS: Pixels = px(12.);
/// The radius of a titled window on macOS 26, concentric with IntelliJ's 12 px dialog insets and
/// its 4 px fields and buttons.
const DIALOG_RADIUS: Pixels = px(16.);
const DIALOG_CONTENT_INSET: Pixels = px(12.);
const POPUP_VIEWPORT_MARGIN: Pixels = px(8.);
const DIALOG_VIEWPORT_MARGIN: Pixels = px(32.);
const SURFACE_BORDER_WIDTH: Pixels = px(1.);

impl SurfaceKind {
    pub const ALL: [SurfaceKind; 4] = [
        SurfaceKind::Popup,
        SurfaceKind::Dialog,
        SurfaceKind::Balloon,
        SurfaceKind::Tooltip,
    ];

    pub const fn standard(self) -> SurfaceStandard {
        match self {
            SurfaceKind::Popup => SurfaceStandard {
                corner_radius: POPUP_RADIUS,
                border_width: SURFACE_BORDER_WIDTH,
                content_inset: PopupRowMetrics::for_style(PopupRowStyle::List).pill_inset_x,
                viewport_margin: POPUP_VIEWPORT_MARGIN,
                material: SurfaceMaterial::Glass,
                elevation: ElevationIndex::ElevatedSurface,
            },
            SurfaceKind::Dialog => SurfaceStandard {
                corner_radius: DIALOG_RADIUS,
                border_width: SURFACE_BORDER_WIDTH,
                content_inset: DIALOG_CONTENT_INSET,
                viewport_margin: DIALOG_VIEWPORT_MARGIN,
                material: SurfaceMaterial::Glass,
                elevation: ElevationIndex::ModalSurface,
            },
            SurfaceKind::Balloon => SurfaceStandard {
                corner_radius: BALLOON_METRICS.corner_radius,
                border_width: BALLOON_METRICS.border_stroke_width,
                content_inset: BALLOON_METRICS.window_inset,
                viewport_margin: BALLOON_METRICS.window_inset,
                material: SurfaceMaterial::Glass,
                elevation: ElevationIndex::ElevatedSurface,
            },
            SurfaceKind::Tooltip => SurfaceStandard {
                corner_radius: TOOLTIP_METRICS.corner_radius,
                border_width: SURFACE_BORDER_WIDTH,
                content_inset: TOOLTIP_METRICS.padding_x,
                viewport_margin: POPUP_VIEWPORT_MARGIN,
                material: SurfaceMaterial::Opaque,
                elevation: ElevationIndex::ElevatedSurface,
            },
        }
    }

    pub fn debug_selector(self) -> &'static str {
        match self {
            SurfaceKind::Popup => "SURFACE-Popup",
            SurfaceKind::Dialog => "SURFACE-Dialog",
            SurfaceKind::Balloon => "SURFACE-Balloon",
            SurfaceKind::Tooltip => "SURFACE-Tooltip",
        }
    }

    /// Shrinks `preferred` so the surface keeps [`SurfaceStandard::viewport_margin`] from every
    /// window edge. A surface never grows here and never gets a negative size.
    pub fn fit_to_viewport(self, preferred: Size<Pixels>, viewport: Size<Pixels>) -> Size<Pixels> {
        let margin = self.standard().viewport_margin * 2.;
        size(
            preferred
                .width
                .min((viewport.width - margin).max(px(0.)))
                .round(),
            preferred
                .height
                .min((viewport.height - margin).max(px(0.)))
                .round(),
        )
    }
}

/// The Liquid Glass material of [`SurfaceMaterial::Glass`] surfaces is [`GlassSettings`]
/// ([`theme_settings::GlassSettings`]): the theme's elevated surface colour over a blurred
/// backdrop. On macOS 26 AppKit draws it with its own `NSGlassEffectView` — the window draws
/// everything below a surface into its layer, places the glass view, and draws the surface's
/// content into a transparent layer above it — and the theme colour reaches that view as its
/// tint. Elsewhere, and in headless tests, the renderer emulates it with a blur.

/// The widest a content-sized popup (a menu, the branches popup) may grow before its rows elide.
pub const POPUP_MAX_CONTENT_WIDTH: Pixels = px(560.);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CloseButtonSize {
    /// Close buttons in the header of a dialog, popup or balloon.
    #[default]
    Header,
    /// Close buttons inside a tab, a chip or a field.
    Inline,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CloseButtonMetrics {
    pub box_side: Pixels,
    pub icon_side: Pixels,
    /// The hover fill is always a circle, like IntelliJ's `closeSmallHovered` and macOS 26's
    /// close buttons.
    pub hover_radius: Pixels,
}

/// IntelliJ's `closeSmallHovered` circle is #F0F1F2 at 13% on dark and #313547 at 10% on light;
/// Flint lays the theme's text colour over the surface at the dark-theme strength.
pub const CLOSE_BUTTON_HOVER_ALPHA: f32 = 0.13;

const CLOSE_ICON_SIDE: Pixels = px(16.);

impl CloseButtonSize {
    pub const fn metrics(self) -> CloseButtonMetrics {
        match self {
            CloseButtonSize::Header => CloseButtonMetrics {
                box_side: px(20.),
                icon_side: CLOSE_ICON_SIDE,
                hover_radius: px(10.),
            },
            CloseButtonSize::Inline => CloseButtonMetrics {
                box_side: px(16.),
                icon_side: CLOSE_ICON_SIDE,
                hover_radius: px(8.),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogWidth {
    Small,
    Medium,
    Large,
}

impl DialogWidth {
    pub const fn rems(self) -> Rems {
        match self {
            DialogWidth::Small => rems(30.),
            DialogWidth::Medium => rems(36.),
            DialogWidth::Large => rems(48.),
        }
    }

    /// Whole pixels, so the dialog's edges and its centred origin land on device pixels.
    pub fn pixels(self, rem_size: Pixels) -> Pixels {
        self.rems().to_pixels(rem_size).round()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DialogMetrics {
    pub padding_x: Pixels,
    pub header_padding_top: Pixels,
    pub header_padding_bottom: Pixels,
    pub section_gap: Pixels,
    pub footer_padding_top: Pixels,
    pub footer_padding_bottom: Pixels,
    /// IntelliJ's `DialogWrapper` keeps 12 px between the visible edges of its buttons.
    pub button_gap: Pixels,
    pub button_group_gap: Pixels,
    pub title_icon_gap: Pixels,
}

/// The dialog's 1 px border sits inside its bounds, so content padded by these values starts
/// [`DIALOG_CONTENT_INSET`] from the dialog's outer edge.
const DIALOG_EDGE_PADDING: Pixels = px(11.);

pub const DIALOG_METRICS: DialogMetrics = DialogMetrics {
    padding_x: DIALOG_EDGE_PADDING,
    header_padding_top: DIALOG_EDGE_PADDING,
    header_padding_bottom: px(8.),
    section_gap: px(8.),
    footer_padding_top: px(12.),
    footer_padding_bottom: DIALOG_EDGE_PADDING,
    button_gap: px(12.),
    button_group_gap: px(20.),
    title_icon_gap: px(8.),
};

/// The gap between the pointer and a tooltip on macOS (IntelliJ's `HelpTooltip` cursor offset),
/// so a tooltip never sits on the control it describes.
pub const TOOLTIP_CURSOR_OFFSET: Pixels = px(17.);

/// The radius a control nested in a surface needs to stay concentric with it.
pub fn concentric_radius(outer_radius: Pixels, inset: Pixels) -> Pixels {
    (outer_radius - inset).max(px(0.))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ButtonMetrics, ButtonSize, COMPONENT_CORNER_RADII};
    use theme::UiDensity;

    const SCALE_FACTORS: [f32; 4] = [1., 1.5, 2., 3.];
    const REM_SIZES: [f32; 7] = [10., 12., 13., 14., 15., 18., 24.];

    #[test]
    fn popup_rows_are_concentric_with_the_popup_surface() {
        let popup = SurfaceKind::Popup.standard();
        for style in [PopupRowStyle::List, PopupRowStyle::Menu] {
            let row = PopupRowMetrics::for_style(style);
            assert_eq!(
                row.pill_radius,
                concentric_radius(popup.corner_radius, row.pill_inset_x),
                "{style:?} selection pill inside the {:?} popup",
                popup.corner_radius
            );
        }
    }

    #[test]
    fn dialog_fields_and_buttons_are_concentric_with_the_dialog_surface() {
        let dialog = SurfaceKind::Dialog.standard();
        let inner = concentric_radius(dialog.corner_radius, dialog.content_inset);
        assert_eq!(COMPONENT_CORNER_RADII.input, inner, "text fields");
        assert_eq!(COMPONENT_CORNER_RADII.button, inner, "dialog buttons");
        for (name, padding) in [
            ("side", DIALOG_METRICS.padding_x),
            ("top", DIALOG_METRICS.header_padding_top),
            ("bottom", DIALOG_METRICS.footer_padding_bottom),
        ] {
            assert_eq!(
                padding + dialog.border_width,
                dialog.content_inset,
                "the {name} content edge is the content inset from the outer edge"
            );
        }
    }

    #[test]
    fn every_surface_has_a_single_pixel_border_and_whole_pixel_geometry() {
        for kind in SurfaceKind::ALL {
            let standard = kind.standard();
            assert_eq!(standard.border_width, px(1.), "{kind:?} border");
            for (name, value) in [
                ("corner radius", standard.corner_radius),
                ("content inset", standard.content_inset),
                ("viewport margin", standard.viewport_margin),
            ] {
                assert_eq!(value, value.round(), "{kind:?} {name} must be whole pixels");
            }
        }
    }

    #[test]
    fn surfaces_follow_intellij_and_macos_radii() {
        assert_eq!(SurfaceKind::Popup.standard().corner_radius, px(12.));
        assert_eq!(SurfaceKind::Dialog.standard().corner_radius, px(16.));
        assert_eq!(SurfaceKind::Balloon.standard().corner_radius, px(12.));
        assert_eq!(SurfaceKind::Tooltip.standard().corner_radius, px(6.));
    }

    #[test]
    fn only_tooltips_stay_off_glass() {
        for kind in SurfaceKind::ALL {
            let expected = if kind == SurfaceKind::Tooltip {
                SurfaceMaterial::Opaque
            } else {
                SurfaceMaterial::Glass
            };
            assert_eq!(kind.standard().material, expected, "{kind:?}");
        }
    }

    #[test]
    fn whole_pixel_geometry_lands_on_device_pixels_at_every_mac_scale() {
        let mut values = vec![
            CloseButtonSize::Header.metrics().box_side,
            CloseButtonSize::Inline.metrics().box_side,
            CLOSE_ICON_SIDE,
            DIALOG_METRICS.padding_x,
            DIALOG_METRICS.header_padding_top,
            DIALOG_METRICS.header_padding_bottom,
            DIALOG_METRICS.footer_padding_top,
            DIALOG_METRICS.footer_padding_bottom,
            DIALOG_METRICS.button_gap,
            POPUP_MAX_CONTENT_WIDTH,
        ];
        for kind in SurfaceKind::ALL {
            let standard = kind.standard();
            values.extend([
                standard.corner_radius,
                standard.border_width,
                standard.content_inset,
            ]);
        }
        for rem in REM_SIZES {
            for width in [DialogWidth::Small, DialogWidth::Medium, DialogWidth::Large] {
                values.push(width.pixels(px(rem)));
            }
        }
        for scale in SCALE_FACTORS {
            for value in &values {
                let device = f32::from(*value) * scale;
                if scale.fract() == 0. {
                    assert_eq!(device, device.round(), "{value:?} at {scale}x");
                } else {
                    // At 1.5x macOS rounds odd values; even values still land on a pixel.
                    assert!(
                        (device - device.round()).abs() <= 0.5,
                        "{value:?} at {scale}x"
                    );
                }
            }
        }
    }

    #[test]
    fn dialog_widths_grow_with_the_ui_font_and_stay_whole_pixels() {
        for width in [DialogWidth::Small, DialogWidth::Medium, DialogWidth::Large] {
            let mut previous = px(0.);
            for rem in REM_SIZES {
                let pixels = width.pixels(px(rem));
                assert_eq!(pixels, pixels.round(), "{width:?} at rem {rem}");
                assert!(pixels > previous, "{width:?} grows with the font");
                previous = pixels;
            }
        }
        assert_eq!(DialogWidth::Medium.pixels(px(15.)), px(540.));
    }

    #[test]
    fn close_buttons_share_one_glyph_and_a_circular_hover() {
        for size in [CloseButtonSize::Header, CloseButtonSize::Inline] {
            let metrics = size.metrics();
            assert_eq!(metrics.icon_side, px(16.), "{size:?} glyph");
            assert_eq!(
                metrics.hover_radius * 2.,
                metrics.box_side,
                "{size:?} circle"
            );
        }
        assert_eq!(CloseButtonSize::Header.metrics().box_side, px(20.));
        assert_eq!(CloseButtonSize::Inline.metrics().box_side, px(16.));
    }

    #[test]
    fn dialog_buttons_use_the_intellij_form_button() {
        for density in [UiDensity::Default, UiDensity::Compact] {
            let button = ButtonMetrics::for_size(ButtonSize::Medium, density);
            assert_eq!(button.minimum_width, Some(px(72.)), "{density:?}");
        }
        assert_eq!(DIALOG_METRICS.button_gap, px(12.));
        assert_eq!(DIALOG_METRICS.button_group_gap, px(20.));
    }

    #[test]
    fn surfaces_shrink_to_keep_their_margin_from_the_window_edges() {
        let viewport = size(px(800.), px(600.));
        for kind in SurfaceKind::ALL {
            let margin = kind.standard().viewport_margin;
            let fitted = kind.fit_to_viewport(size(px(2000.), px(2000.)), viewport);
            assert_eq!(fitted.width, viewport.width - margin * 2., "{kind:?}");
            assert_eq!(fitted.height, viewport.height - margin * 2., "{kind:?}");

            let small = size(px(200.), px(100.));
            assert_eq!(kind.fit_to_viewport(small, viewport), small, "{kind:?}");

            let tiny_window = size(px(10.), px(10.));
            let fitted = kind.fit_to_viewport(small, tiny_window);
            assert!(
                fitted.width >= px(0.) && fitted.height >= px(0.),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn fitted_surfaces_have_whole_pixel_sizes() {
        let fitted = SurfaceKind::Popup
            .fit_to_viewport(size(px(1000.25), px(700.5)), size(px(640.75), px(480.25)));
        assert_eq!(fitted.width, fitted.width.round());
        assert_eq!(fitted.height, fitted.height.round());
    }
}
