use gpui::{App, Pixels, Window, px};
#[cfg(not(target_os = "windows"))]
use theme::UiDensity;

pub const MACOS_SDK_26_OR_LATER: bool = cfg!(macos_sdk_26_or_later);

// Use pixels here instead of a rem-based size because the macOS traffic
// lights are a static size, and don't scale with the rest of the UI.
//
// Magic number: There is one extra pixel of padding on the left side due to
// the 1px border around the window on macOS apps.
pub const TRAFFIC_LIGHT_PADDING: f32 = if MACOS_SDK_26_OR_LATER { 78. } else { 71. };

const TITLE_BAR_BUTTON_MARGIN: Pixels = px(8.);
#[cfg(not(target_os = "windows"))]
const MAIN_TOOLBAR_HEIGHT: Pixels = px(40.);
#[cfg(not(target_os = "windows"))]
const MAIN_TOOLBAR_COMPACT_HEIGHT: Pixels = px(32.);

/// Returns the platform-appropriate title bar height.
///
/// On Windows, this returns a fixed height of 32px.
/// On other platforms, it scales with the window's rem size (1.75x) with a minimum of 34px.
#[cfg(not(target_os = "windows"))]
pub fn platform_title_bar_height(_window: &Window, cx: &App) -> Pixels {
    let default = match theme::theme_settings(cx).ui_density(cx) {
        UiDensity::Compact => MAIN_TOOLBAR_COMPACT_HEIGHT,
        UiDensity::Default | UiDensity::Comfortable => MAIN_TOOLBAR_HEIGHT,
    };
    configured_title_bar_height(default, cx)
}

#[cfg(target_os = "windows")]
pub fn platform_title_bar_height(_window: &Window, cx: &App) -> Pixels {
    // todo(windows) instead of hard coded size report the actual size to the Windows platform API
    configured_title_bar_height(px(32.), cx)
}

fn configured_title_bar_height(default: Pixels, cx: &App) -> Pixels {
    let region = crate::ChromeRegion::TitleBar;
    let size = crate::ButtonSize::Default;
    match crate::chrome_height(region, cx) {
        Some(height) => height.max(crate::chrome_fit_height(region, size, cx)),
        None => match crate::chrome_button_height(region, size, cx) {
            Some(button_height) => default.max(button_height + TITLE_BAR_BUTTON_MARGIN),
            None => default,
        },
    }
}
