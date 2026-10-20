use gpui::{Pixels, px, rems};
use theme::UiDensity;

use crate::{ButtonSize, IconSize};

const TOOLBAR_BUTTON_SIDE: Pixels = px(22.);
const NAVBAR_BUTTON_SIDE: Pixels = px(20.);
const COMPACT_BUTTON_HEIGHT: Pixels = px(18.);
const LARGE_BUTTON_SIDE: Pixels = px(32.);
const HUGGING_BUTTON_HEIGHT: Pixels = px(16.);
const HUGGING_BUTTON_HORIZONTAL_PADDING: Pixels = px(1.);
const FORM_BUTTON_HEIGHT: Pixels = px(28.);
const COMPACT_FORM_BUTTON_HEIGHT: Pixels = px(24.);
const FORM_BUTTON_MINIMUM_WIDTH: Pixels = px(72.);
const FORM_BUTTON_HORIZONTAL_PADDING: Pixels = px(14.);
const FORM_BUTTON_ICON_GAP: Pixels = px(4.);
const TOOLBAR_BUTTON_HORIZONTAL_PADDING: Pixels = px(4.);
const TOOLBAR_BUTTON_ICON_GAP: Pixels = px(2.);

pub const TOOLBAR_ICON_SIDE: Pixels = px(16.);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ButtonMetrics {
    pub height: Pixels,
    pub icon_button_side: Pixels,
    pub horizontal_padding: Option<Pixels>,
    pub icon_gap: Option<Pixels>,
    pub minimum_width: Option<Pixels>,
}

impl ButtonMetrics {
    pub fn for_size(size: ButtonSize, density: UiDensity) -> Self {
        let form_height = if density == UiDensity::Compact {
            COMPACT_FORM_BUTTON_HEIGHT
        } else {
            FORM_BUTTON_HEIGHT
        };
        match size {
            ButtonSize::Default => Self {
                height: TOOLBAR_BUTTON_SIDE,
                icon_button_side: TOOLBAR_BUTTON_SIDE,
                horizontal_padding: Some(TOOLBAR_BUTTON_HORIZONTAL_PADDING),
                icon_gap: Some(TOOLBAR_BUTTON_ICON_GAP),
                minimum_width: None,
            },
            ButtonSize::Compact => Self {
                height: COMPACT_BUTTON_HEIGHT,
                icon_button_side: NAVBAR_BUTTON_SIDE,
                horizontal_padding: Some(TOOLBAR_BUTTON_HORIZONTAL_PADDING),
                icon_gap: Some(TOOLBAR_BUTTON_ICON_GAP),
                minimum_width: None,
            },
            ButtonSize::Medium => Self {
                height: form_height,
                icon_button_side: form_height,
                horizontal_padding: Some(FORM_BUTTON_HORIZONTAL_PADDING),
                icon_gap: Some(FORM_BUTTON_ICON_GAP),
                minimum_width: Some(FORM_BUTTON_MINIMUM_WIDTH),
            },
            ButtonSize::Large => Self {
                height: LARGE_BUTTON_SIDE,
                icon_button_side: LARGE_BUTTON_SIDE,
                horizontal_padding: None,
                icon_gap: None,
                minimum_width: None,
            },
            ButtonSize::None => Self {
                height: HUGGING_BUTTON_HEIGHT,
                icon_button_side: HUGGING_BUTTON_HEIGHT,
                horizontal_padding: Some(HUGGING_BUTTON_HORIZONTAL_PADDING),
                icon_gap: None,
                minimum_width: None,
            },
        }
    }
}

pub fn icon_button_side(size: ButtonSize, density: UiDensity, icon_side: Pixels) -> Pixels {
    ButtonMetrics::for_size(size, density)
        .icon_button_side
        .max(icon_side)
}

pub fn exact_icon_size(side: Pixels, rem_size: Pixels) -> IconSize {
    IconSize::Custom(rems(f32::from(side) / f32::from(rem_size)))
}

pub fn icon_button_icon_size(requested: IconSize, rem_size: Pixels) -> IconSize {
    match requested {
        IconSize::Small | IconSize::Medium => exact_icon_size(TOOLBAR_ICON_SIDE, rem_size),
        other => other,
    }
}

pub fn icon_button_icon_side(requested: IconSize, rem_size: Pixels) -> Pixels {
    match requested {
        IconSize::Small | IconSize::Medium => TOOLBAR_ICON_SIDE,
        other => other.rems() * rem_size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IconButtonShape, prelude::*};
    use gpui::{Context, Render, TestAppContext, VisualTestContext};

    const DENSITIES: [UiDensity; 3] = [
        UiDensity::Compact,
        UiDensity::Default,
        UiDensity::Comfortable,
    ];
    const REM_SIZES: [f32; 4] = [13., 15., 16., 20.];

    fn side(size: ButtonSize, density: UiDensity) -> Pixels {
        icon_button_side(size, density, TOOLBAR_ICON_SIDE)
    }

    #[test]
    fn icon_button_chrome_size_is_the_toolbar_action_button_in_every_density() {
        for density in DENSITIES {
            assert_eq!(side(ButtonSize::Default, density), px(22.));
            assert_eq!(side(ButtonSize::Compact, density), px(20.));
        }
    }

    #[test]
    fn icon_button_chrome_size_of_medium_follows_the_form_control_height() {
        assert_eq!(side(ButtonSize::Medium, UiDensity::Default), px(28.));
        assert_eq!(side(ButtonSize::Medium, UiDensity::Comfortable), px(28.));
        assert_eq!(side(ButtonSize::Medium, UiDensity::Compact), px(24.));
    }

    #[test]
    fn icon_button_chrome_size_of_large_is_the_same_in_every_density() {
        for density in DENSITIES {
            assert_eq!(side(ButtonSize::Large, density), px(32.));
        }
    }

    #[test]
    fn icon_button_chrome_size_grows_with_an_icon_larger_than_the_box() {
        for density in DENSITIES {
            assert_eq!(
                icon_button_side(ButtonSize::Default, density, px(24.)),
                px(24.)
            );
            assert_eq!(
                icon_button_side(ButtonSize::Default, density, px(22.)),
                px(22.)
            );
        }
    }

    #[test]
    fn icon_button_chrome_size_text_buttons_follow_the_darcula_button() {
        let default = ButtonMetrics::for_size(ButtonSize::Medium, UiDensity::Default);
        let compact = ButtonMetrics::for_size(ButtonSize::Medium, UiDensity::Compact);
        assert_eq!(default.height, px(28.));
        assert_eq!(compact.height, px(24.));
        for metrics in [default, compact] {
            assert_eq!(metrics.minimum_width, Some(px(72.)));
            assert_eq!(metrics.horizontal_padding, Some(px(14.)));
            assert_eq!(metrics.icon_gap, Some(px(4.)));
        }
    }

    #[test]
    fn icon_button_chrome_size_toolbar_text_buttons_ignore_the_density() {
        for density in DENSITIES {
            let metrics = ButtonMetrics::for_size(ButtonSize::Default, density);
            assert_eq!(metrics.height, px(22.));
            assert_eq!(metrics.horizontal_padding, Some(px(4.)));
            assert_eq!(metrics.icon_gap, Some(px(2.)));
            assert_eq!(metrics.minimum_width, None);
        }
    }

    #[test]
    fn icon_button_chrome_size_icons_are_sixteen_pixels_at_every_ui_font_size() {
        for rem_size in REM_SIZES {
            for requested in [IconSize::Small, IconSize::Medium] {
                let icon_side = icon_button_icon_side(requested, px(rem_size));
                assert_eq!(icon_side, px(16.));
                let size = icon_button_icon_size(requested, px(rem_size));
                let rendered = size.rems() * px(rem_size);
                assert!((f32::from(rendered) - 16.).abs() < 1e-4, "{rendered:?}");
            }
        }
    }

    #[test]
    fn icon_button_chrome_size_keeps_the_other_icon_sizes() {
        for rem_size in REM_SIZES {
            for requested in [IconSize::XSmall, IconSize::Indicator, IconSize::XLarge] {
                assert!(icon_button_icon_size(requested, px(rem_size)) == requested);
                assert_eq!(
                    icon_button_icon_side(requested, px(rem_size)),
                    requested.rems() * px(rem_size)
                );
            }
        }
    }

    struct SizeProbe;

    impl Render for SizeProbe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            h_flex()
                .child(IconButton::new("default", IconName::Check))
                .child(
                    IconButton::new("compact", IconName::Plus)
                        .size(ButtonSize::Compact)
                        .icon_size(IconSize::Small),
                )
                .child(IconButton::new("medium", IconName::Dash).size(ButtonSize::Medium))
                .child(
                    IconButton::new("square", IconName::ChevronDown)
                        .shape(IconButtonShape::Square)
                        .icon_size(IconSize::Small),
                )
        }
    }

    fn init_theme(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
    }

    fn set_ui_density(density: settings::UiDensity, cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.update_global::<settings::SettingsStore, _>(|store, cx| {
                store.update_user_settings(cx, |settings| {
                    settings.theme.ui_density = Some(density);
                });
            });
        });
    }

    #[track_caller]
    fn assert_box(cx: &mut VisualTestContext, selector: &'static str, expected: f32) {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is painted"));
        for side in [bounds.size.width, bounds.size.height] {
            assert!(
                (f32::from(side) - expected).abs() < 1e-3,
                "{selector} is {bounds:?}, expected {expected} px square"
            );
        }
    }

    #[gpui::test]
    fn icon_button_chrome_size_renders_the_toolbar_box_in_default_density(cx: &mut TestAppContext) {
        init_theme(cx);
        let (_, cx) = cx.add_window_view(|_, _| SizeProbe);
        assert_box(cx, "ICON-Check", 22.);
        assert_box(cx, "ICON-Plus", 20.);
        assert_box(cx, "ICON-Dash", 28.);
        assert_box(cx, "ICON-ChevronDown", 22.);
    }

    #[gpui::test]
    fn icon_button_chrome_size_keeps_the_toolbar_box_in_compact_density(cx: &mut TestAppContext) {
        init_theme(cx);
        set_ui_density(settings::UiDensity::Compact, cx);
        let (_, cx) = cx.add_window_view(|_, _| SizeProbe);
        assert_box(cx, "ICON-Check", 22.);
        assert_box(cx, "ICON-Plus", 20.);
        assert_box(cx, "ICON-Dash", 24.);
        assert_box(cx, "ICON-ChevronDown", 22.);
    }
}
