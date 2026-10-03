use gpui::{App, Pixels, Window, px, rems};

use crate::{ButtonSize, DynamicSpacing, IconSize};

const DEFAULT_ICON_PX: f32 = 14.;
const BUTTON_ICON_PADDING: Pixels = px(8.);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChromeRegion {
    TitleBar,
    TabBar,
    Toolbar,
    StatusBar,
}

pub fn chrome_height(region: ChromeRegion, cx: &App) -> Option<Pixels> {
    let sizes = theme::theme_settings(cx).chrome_sizes(cx);
    match region {
        ChromeRegion::TitleBar => sizes.title_bar_height,
        ChromeRegion::TabBar => sizes.tab_bar_height,
        ChromeRegion::Toolbar => sizes.toolbar_height,
        ChromeRegion::StatusBar => sizes.status_bar_height,
    }
}

pub fn chrome_icon_size(region: ChromeRegion, default: IconSize, cx: &App) -> IconSize {
    match configured_icon_size(region, cx) {
        Some(icon_size) => scaled_icon_size(default, icon_size, current_ui_font_size(cx)),
        None => default,
    }
}

pub fn chrome_icon_scale(region: ChromeRegion, cx: &App) -> Option<f32> {
    configured_icon_size(region, cx).map(icon_scale)
}

pub fn chrome_button_height(region: ChromeRegion, size: ButtonSize, cx: &App) -> Option<Pixels> {
    let icon_size = configured_icon_size(region, cx)?;
    Some(button_height(size, icon_size, current_ui_font_size(cx)))
}

pub(crate) fn chrome_fit_height(region: ChromeRegion, size: ButtonSize, cx: &App) -> Pixels {
    let ui_font_size = current_ui_font_size(cx);
    match configured_icon_size(region, cx) {
        Some(icon_size) => button_height(size, icon_size, ui_font_size),
        None => size.rems().to_pixels(ui_font_size),
    }
}

pub(crate) fn chrome_square_side(
    region: ChromeRegion,
    default: IconSize,
    window: &Window,
    cx: &App,
) -> Option<Pixels> {
    let icon_size = configured_icon_size(region, cx)?;
    let scaled_icon = scaled_icon_size(default, icon_size, current_ui_font_size(cx));
    let icon_px = scaled_icon.rems().to_pixels(window.rem_size());
    let padding = DynamicSpacing::Base02.px(cx);
    Some(square_side(icon_size, icon_px, padding))
}

fn configured_icon_size(region: ChromeRegion, cx: &App) -> Option<Pixels> {
    let sizes = theme::theme_settings(cx).chrome_sizes(cx);
    match region {
        ChromeRegion::TitleBar => sizes.title_bar_icon_size,
        ChromeRegion::TabBar => sizes.tab_bar_icon_size,
        ChromeRegion::Toolbar => sizes.toolbar_icon_size,
        ChromeRegion::StatusBar => sizes.status_bar_icon_size,
    }
}

fn current_ui_font_size(cx: &App) -> Pixels {
    theme::theme_settings(cx).ui_font_size(cx)
}

fn icon_scale(icon_size: Pixels) -> f32 {
    f32::from(icon_size) / DEFAULT_ICON_PX
}

fn scaled_icon_px(default: IconSize, icon_size: Pixels) -> Pixels {
    let default_px = default.rems().0 * crate::BASE_REM_SIZE_IN_PX;
    px(default_px * f32::from(icon_size) / DEFAULT_ICON_PX)
}

fn icon_size_from_px(size: Pixels, ui_font_size: Pixels) -> IconSize {
    IconSize::Custom(rems(f32::from(size) / f32::from(ui_font_size)))
}

fn scaled_icon_size(default: IconSize, icon_size: Pixels, ui_font_size: Pixels) -> IconSize {
    icon_size_from_px(scaled_icon_px(default, icon_size), ui_font_size)
}

fn button_height(size: ButtonSize, icon_size: Pixels, ui_font_size: Pixels) -> Pixels {
    let default_height = size.rems().to_pixels(ui_font_size);
    default_height.max(icon_size + BUTTON_ICON_PADDING)
}

fn square_side(icon_size: Pixels, icon_px: Pixels, padding: Pixels) -> Pixels {
    (icon_size + BUTTON_ICON_PADDING).max(icon_px + padding * 2.)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UI_FONT_SIZES: [f32; 5] = [10., 14., 16., 20., 24.];
    const ICON_SIZES: [f32; 5] = [10., 14., 18., 20., 32.];

    #[track_caller]
    fn assert_px(actual: f32, expected: f32) {
        let difference = (actual - expected).abs();
        assert!(difference < 1e-4, "{actual} != {expected}");
    }

    fn rendered_px(icon_size: IconSize, ui_font_size: f32) -> f32 {
        icon_size.rems().0 * ui_font_size
    }

    #[test]
    fn small_icons_become_exactly_the_configured_size() {
        for icon_size in ICON_SIZES {
            let scaled = scaled_icon_px(IconSize::Small, px(icon_size));
            assert_eq!(scaled, px(icon_size));
            for ui_font_size in UI_FONT_SIZES {
                let icon = scaled_icon_size(IconSize::Small, px(icon_size), px(ui_font_size));
                assert_px(rendered_px(icon, ui_font_size), icon_size);
            }
        }
    }

    #[test]
    fn small_icon_set_to_twenty_renders_twenty_pixels_at_every_ui_font_size() {
        for ui_font_size in UI_FONT_SIZES {
            let icon = scaled_icon_size(IconSize::Small, px(20.), px(ui_font_size));
            assert_px(rendered_px(icon, ui_font_size), 20.);
        }
    }

    #[test]
    fn other_icon_sizes_scale_in_proportion_to_the_small_icon() {
        let defaults = [
            (IconSize::Indicator, 10.),
            (IconSize::XSmall, 12.),
            (IconSize::Small, 14.),
            (IconSize::Medium, 16.),
            (IconSize::Custom(rems(1.25)), 20.),
            (IconSize::Custom(rems(0.5)), 8.),
        ];
        for (default, default_px) in defaults {
            for icon_size in ICON_SIZES {
                let expected = default_px * icon_size / 14.;
                let scaled = scaled_icon_px(default, px(icon_size));
                assert_px(f32::from(scaled), expected);
                for ui_font_size in UI_FONT_SIZES {
                    let icon = scaled_icon_size(default, px(icon_size), px(ui_font_size));
                    assert_px(rendered_px(icon, ui_font_size), expected);
                }
            }
        }
    }

    #[test]
    fn icon_scale_is_relative_to_the_small_icon() {
        assert_px(icon_scale(px(14.)), 1.);
        assert_px(icon_scale(px(21.)), 1.5);
        assert_px(icon_scale(px(28.)), 2.);
        assert_px(icon_scale(px(10.)), 10. / 14.);
    }

    #[test]
    fn buttons_grow_to_fit_the_icon_and_never_shrink_below_their_size() {
        let cases = [
            (ButtonSize::Default, 20., 16., 28.),
            (ButtonSize::Default, 10., 16., 22.),
            (ButtonSize::Default, 14., 16., 22.),
            (ButtonSize::Default, 14., 20., 27.5),
            (ButtonSize::Default, 24., 20., 32.),
            (ButtonSize::Large, 20., 16., 32.),
            (ButtonSize::Large, 32., 16., 40.),
            (ButtonSize::Compact, 10., 16., 18.),
            (ButtonSize::None, 10., 16., 18.),
            (ButtonSize::None, 32., 16., 40.),
        ];
        for (size, icon_size, ui_font_size, expected) in cases {
            let height = button_height(size, px(icon_size), px(ui_font_size));
            assert_px(f32::from(height), expected);
        }
    }

    #[test]
    fn square_buttons_match_the_bar_and_keep_padding_around_larger_icons() {
        let padding = 2.;
        let cases = [
            (IconSize::XSmall, 20., 28.),
            (IconSize::Small, 20., 28.),
            (IconSize::Medium, 20., 28.),
            (IconSize::Small, 10., 18.),
            (IconSize::Medium, 32., 16. * 32. / 14. + 2. * padding),
            (IconSize::XLarge, 20., 48. * 20. / 14. + 2. * padding),
        ];
        for (default, icon_size, expected) in cases {
            let icon_px = scaled_icon_px(default, px(icon_size));
            let side = square_side(px(icon_size), icon_px, px(padding));
            assert_px(f32::from(side), expected);
        }
    }
}
