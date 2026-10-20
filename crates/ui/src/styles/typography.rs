use crate::prelude::*;
use gpui::{
    AnyElement, App, IntoElement, ParentElement, Rems, RenderOnce, SharedString, Styled, Window,
    div, rems,
};
use theme::ActiveTheme;

use crate::Color;

/// Extends [`gpui::Styled`] with typography-related styling methods.
pub trait StyledTypography: Styled + Sized {
    /// Sets the font family to the buffer font.
    fn font_buffer(self, cx: &App) -> Self {
        let settings = theme::theme_settings(cx);
        let buffer_font_family = settings.buffer_font(cx).family.clone();

        self.font_family(buffer_font_family)
    }

    /// Sets the font family to the UI font.
    fn font_ui(self, cx: &App) -> Self {
        let settings = theme::theme_settings(cx);
        let ui_font_family = settings.ui_font(cx).family.clone();

        self.font_family(ui_font_family)
    }

    /// Sets the text size using a [`TextSize`].
    fn text_ui_size(self, size: TextSize, cx: &App) -> Self {
        self.text_size(size.pixels(cx))
    }

    /// The large size for UI text, 14 px by default (IntelliJ's `JBFont.h4()` size).
    fn text_ui_lg(self, cx: &App) -> Self {
        self.text_size(TextSize::Large.pixels(cx))
    }

    /// The default size for UI text, 13 px by default (IntelliJ's label font).
    fn text_ui(self, cx: &App) -> Self {
        self.text_size(TextSize::default().pixels(cx))
    }

    /// The small size for UI text, 12 px by default (IntelliJ's `JBFont.medium()`).
    fn text_ui_sm(self, cx: &App) -> Self {
        self.text_size(TextSize::Small.pixels(cx))
    }

    /// The extra small size for UI text, 11 px by default (IntelliJ's `JBFont.small()`).
    fn text_ui_xs(self, cx: &App) -> Self {
        self.text_size(TextSize::XSmall.pixels(cx))
    }

    /// The font size for buffer text.
    ///
    /// Retrieves the default font size, or the user's custom font size if set.
    ///
    /// This should only be used for text that is displayed in a buffer,
    /// or other places that text needs to match the user's buffer font size.
    fn text_buffer(self, cx: &App) -> Self {
        let settings = theme::theme_settings(cx);
        self.text_size(settings.buffer_font_size(cx))
    }
}

impl<E: Styled> StyledTypography for E {}

/// A utility for getting the size of various semantic text sizes.
///
/// The sizes are whole pixels, like IntelliJ's macOS type scale (label 13, medium 12, small 11
/// at a 1:1 UI scale), and they are the same inside editors, whose rem follows the buffer font.
/// Fractional sizes such as `14/16` of a 15 px rem made the same label 13.125 px in one place
/// and 12.8 px in another.
#[derive(Debug, Default, Clone, Copy)]
pub enum TextSize {
    /// The default size for UI text: 13 px at the default `ui_font_size` of 15.
    #[default]
    Default,
    /// The large size for UI text: 14 px at the default `ui_font_size`.
    Large,
    /// The small size for UI text: 12 px at the default `ui_font_size`.
    Small,
    /// The extra small size for UI text: 11 px at the default `ui_font_size`.
    XSmall,
    /// The `ui_font_size` set by the user.
    Ui,
    /// The `buffer_font_size` set by the user.
    Editor,
    // TODO: The terminal settings will need to be passed to
    // ThemeSettings before we can enable this.
    //// The `terminal.font_size` set by the user.
    // Terminal,
}

/// How far IntelliJ's label font (13 pt) sits below Flint's default `ui_font_size` (15), which
/// also sets the rem that spacing is measured in.
const LABEL_SIZE_BELOW_UI_FONT_SIZE: f32 = 2.;
/// Keeps the extra small size readable when the UI font size is turned far down.
const MIN_LABEL_SIZE: f32 = 10.;

impl TextSize {
    /// The UI label size for a `ui_font_size`: two pixels below it, rounded to a whole pixel.
    pub fn label_size_for(ui_font_size: Pixels) -> Pixels {
        px((f32::from(ui_font_size) - LABEL_SIZE_BELOW_UI_FONT_SIZE)
            .round()
            .max(MIN_LABEL_SIZE))
    }

    pub fn pixels(self, cx: &App) -> Pixels {
        let settings = theme::theme_settings(cx);
        let label = Self::label_size_for(settings.ui_font_size(cx));
        match self {
            Self::Large => label + px(1.),
            Self::Default => label,
            Self::Small => label - px(1.),
            Self::XSmall => label - px(2.),
            Self::Ui => settings.ui_font_size(cx),
            Self::Editor => settings.buffer_font_size(cx),
        }
    }
}

/// The size of a [`Headline`] element, in whole pixels (see [`HeadlineSize::pixels`]).
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Default)]
pub enum HeadlineSize {
    /// 13 px at the default `ui_font_size`.
    XSmall,
    /// 14 px at the default `ui_font_size`.
    Small,
    #[default]
    /// 16 px at the default `ui_font_size`.
    Medium,
    /// 18 px at the default `ui_font_size`.
    Large,
    /// 22 px at the default `ui_font_size`.
    XLarge,
}

impl HeadlineSize {
    /// The headline size in whole pixels, on IntelliJ's `JBFont` heading sizes above the label
    /// font: 13, then `h4` 14, `h3` 16, `h2` 18 and `h1` 22 px at the default `ui_font_size`.
    pub fn pixels(self, cx: &App) -> Pixels {
        let label = TextSize::Default.pixels(cx);
        match self {
            Self::XSmall => label,
            Self::Small => label + px(1.),
            Self::Medium => label + px(3.),
            Self::Large => label + px(5.),
            Self::XLarge => label + px(9.),
        }
    }

    /// Returns the line height for the headline size.
    pub fn line_height(self) -> Rems {
        match self {
            Self::XSmall => rems(1.6),
            Self::Small => rems(1.6),
            Self::Medium => rems(1.6),
            Self::Large => rems(1.6),
            Self::XLarge => rems(1.6),
        }
    }
}

/// A headline element, used to emphasize some text and
/// create a visual hierarchy.
#[derive(IntoElement, RegisterComponent)]
pub struct Headline {
    size: HeadlineSize,
    text: SharedString,
    color: Color,
}

impl RenderOnce for Headline {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ui_font = theme::theme_settings(cx).ui_font(cx).clone();

        div()
            .font(ui_font)
            .line_height(self.size.line_height())
            .text_size(self.size.pixels(cx))
            .text_color(cx.theme().colors().text)
            .child(self.text)
    }
}

impl Headline {
    /// Create a new headline element.
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            size: HeadlineSize::default(),
            text: text.into(),
            color: Color::default(),
        }
    }

    /// Set the size of the headline.
    pub fn size(mut self, size: HeadlineSize) -> Self {
        self.size = size;
        self
    }

    /// Set the color of the headline.
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }
}

impl Component for Headline {
    fn scope() -> ComponentScope {
        ComponentScope::Typography
    }

    fn description() -> &'static str {
        "A headline element used to emphasize text and create visual hierarchy in the UI."
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> AnyElement {
        v_flex()
            .gap_1()
            .children(vec![
                single_example(
                    "XLarge",
                    Headline::new("XLarge Headline")
                        .size(HeadlineSize::XLarge)
                        .into_any_element(),
                ),
                single_example(
                    "Large",
                    Headline::new("Large Headline")
                        .size(HeadlineSize::Large)
                        .into_any_element(),
                ),
                single_example(
                    "Medium (Default)",
                    Headline::new("Medium Headline").into_any_element(),
                ),
                single_example(
                    "Small",
                    Headline::new("Small Headline")
                        .size(HeadlineSize::Small)
                        .into_any_element(),
                ),
                single_example(
                    "XSmall",
                    Headline::new("XSmall Headline")
                        .size(HeadlineSize::XSmall)
                        .into_any_element(),
                ),
            ])
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[test]
    fn ui_text_sizes_are_whole_pixels_for_any_ui_font_size() {
        for tenths in 80..=320 {
            let ui_font_size = px(tenths as f32 / 10.);
            let label = TextSize::label_size_for(ui_font_size);
            assert_eq!(label, label.round(), "ui_font_size {ui_font_size:?}");
        }
        assert_eq!(TextSize::label_size_for(px(15.)), px(13.));
        assert_eq!(TextSize::label_size_for(px(16.)), px(14.));
        assert_eq!(TextSize::label_size_for(px(14.6)), px(13.));
        assert_eq!(
            TextSize::label_size_for(px(8.)),
            px(MIN_LABEL_SIZE),
            "the extra small size stays readable"
        );
    }

    #[gpui::test]
    fn the_default_ui_text_follows_intellijs_macos_type_scale(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let mut settings_store = settings::SettingsStore::test(cx);
            // The test settings use 14 px fonts; check Flint's own defaults.
            let defaults = settings::parse_json_with_comments::<settings::SettingsContent>(
                &settings::default_settings(),
            )
            .expect("the default settings parse");
            settings_store.update_user_settings(cx, |settings| {
                settings.theme.ui_font_size = defaults.theme.ui_font_size;
                settings.theme.buffer_font_size = defaults.theme.buffer_font_size;
            });
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);

            assert_eq!(TextSize::Default.pixels(cx), px(13.), "the label font");
            assert_eq!(TextSize::Small.pixels(cx), px(12.), "JBFont.medium()");
            assert_eq!(TextSize::XSmall.pixels(cx), px(11.), "JBFont.small()");
            assert_eq!(TextSize::Large.pixels(cx), px(14.), "JBFont.h4() size");
            assert_eq!(TextSize::Ui.pixels(cx), px(15.), "the ui_font_size itself");
            assert_eq!(TextSize::Editor.pixels(cx), px(13.), "JetBrains Mono 13");
            assert_eq!(
                [
                    HeadlineSize::XSmall,
                    HeadlineSize::Small,
                    HeadlineSize::Medium,
                    HeadlineSize::Large,
                    HeadlineSize::XLarge,
                ]
                .map(|size| size.pixels(cx)),
                [px(13.), px(14.), px(16.), px(18.), px(22.)]
            );
        });
    }
}
