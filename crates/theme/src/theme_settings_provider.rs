use gpui::{App, Font, Global, Pixels};

use crate::UiDensity;

#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChromeSizes {
    pub title_bar_height: Option<Pixels>,
    pub title_bar_icon_size: Option<Pixels>,
    pub tab_bar_height: Option<Pixels>,
    pub tab_bar_icon_size: Option<Pixels>,
    pub toolbar_height: Option<Pixels>,
    pub toolbar_icon_size: Option<Pixels>,
    pub status_bar_height: Option<Pixels>,
    pub status_bar_icon_size: Option<Pixels>,
    pub panel_height: Option<Pixels>,
    pub panel_icon_size: Option<Pixels>,
}

/// Trait for providing theme-related settings (fonts, font sizes, UI density)
/// without coupling to the concrete settings infrastructure.
///
/// A concrete implementation is registered as a global by the `theme_settings` crate.
pub trait ThemeSettingsProvider: Send + Sync + 'static {
    /// Returns the font used for UI elements.
    fn ui_font<'a>(&'a self, cx: &'a App) -> &'a Font;

    /// Returns the font used for buffers and the terminal.
    fn buffer_font<'a>(&'a self, cx: &'a App) -> &'a Font;

    /// Returns the UI font size in pixels.
    fn ui_font_size(&self, cx: &App) -> Pixels;

    /// Returns the buffer font size in pixels.
    fn buffer_font_size(&self, cx: &App) -> Pixels;

    /// Returns the current UI density setting.
    fn ui_density(&self, cx: &App) -> UiDensity;

    #[allow(missing_docs)]
    fn chrome_sizes(&self, cx: &App) -> ChromeSizes;
}

struct GlobalThemeSettingsProvider(Box<dyn ThemeSettingsProvider>);

impl Global for GlobalThemeSettingsProvider {}

/// Registers the global [`ThemeSettingsProvider`] implementation.
///
/// This should be called during application initialization by the crate
/// that owns the concrete theme settings (e.g. `theme_settings`).
pub fn set_theme_settings_provider(provider: Box<dyn ThemeSettingsProvider>, cx: &mut App) {
    cx.set_global(GlobalThemeSettingsProvider(provider));
}

/// Returns the global [`ThemeSettingsProvider`].
///
/// Panics if no provider has been registered via [`set_theme_settings_provider`].
pub fn theme_settings(cx: &App) -> &dyn ThemeSettingsProvider {
    &*cx.global::<GlobalThemeSettingsProvider>().0
}
