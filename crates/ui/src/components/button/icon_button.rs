use gpui::{AnyView, DefiniteLength, Hsla};

use super::button_like::{ButtonCommon, ButtonLike, ButtonSize, ButtonStyle};
use crate::{
    ElevationIndex, Icon, IconWithIndicator, Indicator, SelectableButton, TintColor, Tooltip,
    icon_button_icon_size, icon_button_side, prelude::*, ui_density,
};
use crate::{IconName, IconSize};

/// The shape of an [`IconButton`].
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy)]
pub enum IconButtonShape {
    Square,
    Wide,
}

#[derive(IntoElement, RegisterComponent)]
pub struct IconButton {
    base: ButtonLike,
    shape: IconButtonShape,
    icon: IconName,
    icon_size: IconSize,
    icon_color: Color,
    selected_icon: Option<IconName>,
    selected_icon_color: Option<Color>,
    selected_style: Option<ButtonStyle>,
    indicator: Option<Indicator>,
    indicator_border_color: Option<Hsla>,
    alpha: Option<f32>,
    box_side: Option<Pixels>,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: IconName) -> Self {
        let mut this = Self {
            base: ButtonLike::new(id),
            shape: IconButtonShape::Wide,
            icon,
            icon_size: IconSize::default(),
            icon_color: Color::Default,
            selected_icon: None,
            selected_icon_color: None,
            selected_style: None,
            indicator: None,
            indicator_border_color: None,
            alpha: None,
            box_side: None,
        };
        this.base.base = this.base.base.debug_selector(|| format!("ICON-{:?}", icon));
        this
    }

    pub fn shape(mut self, shape: IconButtonShape) -> Self {
        self.shape = shape;
        self
    }

    /// Sets the label announced by assistive technology.
    ///
    /// Icon buttons have no visible text, so they should always set this for
    /// screen reader users. Often this matches the tooltip text.
    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.base = self.base.aria_label(label);
        self
    }

    /// Sets the expanded state reported to assistive technology, for buttons
    /// that control a popup (e.g. dropdown or disclosure triggers).
    pub fn aria_expanded(mut self, expanded: bool) -> Self {
        self.base = self.base.aria_expanded(expanded);
        self
    }

    /// Registers a handler for an accessibility action (e.g.
    /// [`gpui::accesskit::Action::Expand`]) dispatched by assistive technology.
    pub fn on_a11y_action(
        mut self,
        action: gpui::accesskit::Action,
        listener: impl FnMut(Option<&gpui::accesskit::ActionData>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.base = self.base.on_a11y_action(action, listener);
        self
    }

    pub fn icon_size(mut self, icon_size: IconSize) -> Self {
        self.icon_size = icon_size;
        self
    }

    pub fn box_size(mut self, side: Pixels) -> Self {
        self.box_side = Some(side);
        self
    }

    pub fn corner_radius(mut self, radius: Pixels) -> Self {
        self.base.corner_radius = Some(radius);
        self
    }

    pub fn chrome_region(mut self, region: crate::ChromeRegion) -> Self {
        self.base = self.base.chrome_region(region);
        self
    }

    pub fn active_background(mut self, background: Hsla) -> Self {
        self.base.active_background = Some(background);
        self
    }

    pub fn hover_background(mut self, background: Hsla) -> Self {
        self.base.hover_background = Some(background);
        self
    }

    pub fn icon_color(mut self, icon_color: Color) -> Self {
        self.icon_color = icon_color;
        self
    }

    pub fn alpha(mut self, alpha: f32) -> Self {
        self.alpha = Some(alpha);
        self
    }

    pub fn selected_icon(mut self, icon: impl Into<Option<IconName>>) -> Self {
        self.selected_icon = icon.into();
        self
    }

    pub fn on_right_click(
        mut self,
        handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.base = self.base.on_right_click(handler);
        self
    }

    /// Sets the icon color used when the button is in a selected state.
    pub fn selected_icon_color(mut self, color: impl Into<Option<Color>>) -> Self {
        self.selected_icon_color = color.into();
        self
    }

    pub fn indicator(mut self, indicator: Indicator) -> Self {
        self.indicator = Some(indicator);
        self
    }

    pub fn indicator_border_color(mut self, color: Option<Hsla>) -> Self {
        self.indicator_border_color = color;

        self
    }

    /// Use the given callback to construct a new tooltip view when the mouse hovers over this
    /// button. The tooltip itself is also hoverable and won't disappear when the user moves the
    /// mouse into the tooltip, allowing it to contain interactive elements like links or buttons.
    pub fn hoverable_tooltip(
        mut self,
        tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        self.base = self.base.hoverable_tooltip(tooltip);
        self
    }
}

impl Disableable for IconButton {
    fn disabled(mut self, disabled: bool) -> Self {
        self.base = self.base.disabled(disabled);
        self
    }
}

impl Toggleable for IconButton {
    fn toggle_state(mut self, selected: bool) -> Self {
        self.base = self.base.toggle_state(selected);
        self
    }
}

impl SelectableButton for IconButton {
    fn selected_style(mut self, style: ButtonStyle) -> Self {
        self.selected_style = Some(style);
        self.base = self.base.selected_style(style);
        self
    }
}

impl Clickable for IconButton {
    #[inline(always)]
    fn on_click(
        mut self,
        handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.base = self.base.on_click(handler);
        self
    }

    fn cursor_style(mut self, cursor_style: gpui::CursorStyle) -> Self {
        self.base = self.base.cursor_style(cursor_style);
        self
    }
}

impl FixedWidth for IconButton {
    fn width(mut self, width: impl Into<DefiniteLength>) -> Self {
        self.base = self.base.width(width);
        self
    }

    fn full_width(mut self) -> Self {
        self.base = self.base.full_width();
        self
    }
}

impl ButtonCommon for IconButton {
    fn id(&self) -> &ElementId {
        self.base.id()
    }

    fn style(mut self, style: ButtonStyle) -> Self {
        self.base = self.base.style(style);
        self
    }

    fn size(mut self, size: ButtonSize) -> Self {
        self.base = self.base.size(size);
        self
    }

    #[inline(always)]
    fn tooltip(mut self, tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.base = self.base.tooltip(tooltip);
        self
    }

    fn tab_index(mut self, tab_index: impl Into<isize>) -> Self {
        self.base = self.base.tab_index(tab_index);
        self
    }

    fn layer(mut self, elevation: ElevationIndex) -> Self {
        self.base = self.base.layer(elevation);
        self
    }

    fn track_focus(mut self, focus_handle: &gpui::FocusHandle) -> Self {
        self.base = self.base.track_focus(focus_handle);
        self
    }
}

impl VisibleOnHover for IconButton {
    fn visible_on_hover(mut self, group_name: impl Into<SharedString>) -> Self {
        self.base = self.base.visible_on_hover(group_name);
        self
    }
}

impl RenderOnce for IconButton {
    #[allow(refining_impl_trait)]
    fn render(self, window: &mut Window, cx: &mut App) -> ButtonLike {
        let is_disabled = self.base.disabled;
        let is_selected = self.base.selected;
        let pressed_fill =
            is_selected && self.selected_style.is_none() && self.base.style == ButtonStyle::Subtle;

        let icon = self
            .selected_icon
            .filter(|_| is_selected)
            .unwrap_or(self.icon);

        let icon_color = if is_disabled {
            Color::Custom(cx.theme().colors().icon_disabled)
        } else if let Some(selected_style) = self.selected_style.filter(|_| is_selected) {
            Color::Custom(icon_foreground(selected_style.into(), cx))
        } else if is_selected && !pressed_fill {
            let selected_color = self.selected_icon_color.unwrap_or(Color::Selected);
            Color::Custom(icon_foreground(selected_color, cx))
        } else {
            let base_color = self
                .selected_icon_color
                .filter(|_| is_selected)
                .unwrap_or(self.icon_color);
            Color::Custom(icon_foreground(base_color, cx).opacity(self.alpha.unwrap_or(1.0)))
        };

        let chrome_region = self.base.chrome_region;
        let region_configured =
            chrome_region.is_some_and(|region| crate::chrome_icon_scale(region, cx).is_some());
        let size = self.base.size;
        let hugging = size == ButtonSize::None;
        let rem_size = window.rem_size();
        let icon_size = match chrome_region {
            Some(region) if region_configured => {
                crate::chrome_icon_size(region, self.icon_size, cx)
            }
            _ if hugging => self.icon_size,
            _ => icon_button_icon_size(self.icon_size, rem_size),
        };
        let chrome_side = chrome_region
            .filter(|_| self.shape == IconButtonShape::Square)
            .and_then(|region| crate::chrome_square_side(region, self.icon_size, window, cx));
        let icon_element = Icon::new(icon).size(icon_size).color(icon_color);

        let fixed_side = match self.box_side {
            Some(side) => Some(side),
            None => match self.shape {
                IconButtonShape::Square => match hugging || region_configured {
                    true => {
                        let side = self.icon_size.square(window, cx);
                        Some(chrome_side.map_or(side, |chrome| side.max(chrome)))
                    }
                    false => Some(icon_button_side(
                        size,
                        ui_density(cx),
                        icon_size.rems() * rem_size,
                    )),
                },
                IconButtonShape::Wide if hugging => None,
                IconButtonShape::Wide => Some(icon_button_side(
                    size,
                    ui_density(cx),
                    icon_size.rems() * rem_size,
                )),
            },
        };

        let mut base = self.base;
        base.selected_fill_pressed = pressed_fill;
        base.fixed_side = fixed_side;
        base.child(match self.indicator {
            Some(indicator) => IconWithIndicator::new(icon_element, Some(indicator))
                .indicator_border_color(self.indicator_border_color)
                .into_any_element(),
            None => icon_element.into_any_element(),
        })
    }
}

fn icon_foreground(color: Color, cx: &App) -> Hsla {
    let colors = cx.theme().colors();
    match color {
        Color::Default => colors.icon,
        Color::Muted => colors.icon_muted,
        Color::Disabled => colors.icon_disabled,
        Color::Placeholder => colors.icon_placeholder,
        Color::Accent => colors.icon_accent,
        other => other.color(cx),
    }
}

impl Component for IconButton {
    fn scope() -> ComponentScope {
        ComponentScope::Input
    }

    fn sort_name() -> &'static str {
        "ButtonB"
    }

    fn description() -> &'static str {
        "A compact button that displays a single icon with an optional tooltip.\
        The most frequently used button in the Zed codebase."
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> AnyElement {
        v_flex()
            .gap_6()
            .children(vec![
                example_group_with_title(
                    "Icon Button Styles",
                    vec![
                        single_example(
                            "Default",
                            IconButton::new("default", IconName::Check)
                                .layer(ElevationIndex::Background)
                                .into_any_element(),
                        ),
                        single_example(
                            "Filled",
                            IconButton::new("filled", IconName::Check)
                                .layer(ElevationIndex::Background)
                                .style(ButtonStyle::Filled)
                                .into_any_element(),
                        ),
                        single_example(
                            "Subtle",
                            IconButton::new("subtle", IconName::Check)
                                .layer(ElevationIndex::Background)
                                .style(ButtonStyle::Subtle)
                                .into_any_element(),
                        ),
                        single_example(
                            "Tinted",
                            IconButton::new("tinted", IconName::Check)
                                .layer(ElevationIndex::Background)
                                .style(ButtonStyle::Tinted(TintColor::Accent))
                                .into_any_element(),
                        ),
                        single_example(
                            "Transparent",
                            IconButton::new("transparent", IconName::Check)
                                .layer(ElevationIndex::Background)
                                .style(ButtonStyle::Transparent)
                                .into_any_element(),
                        ),
                    ],
                ),
                example_group_with_title(
                    "Icon Button Shapes",
                    vec![
                        single_example(
                            "Square",
                            IconButton::new("square", IconName::Check)
                                .shape(IconButtonShape::Square)
                                .style(ButtonStyle::Filled)
                                .layer(ElevationIndex::Background)
                                .into_any_element(),
                        ),
                        single_example(
                            "Wide",
                            IconButton::new("wide", IconName::Check)
                                .shape(IconButtonShape::Wide)
                                .style(ButtonStyle::Filled)
                                .layer(ElevationIndex::Background)
                                .into_any_element(),
                        ),
                    ],
                ),
                example_group_with_title(
                    "Icon Button Sizes",
                    vec![
                        single_example(
                            "XSmall",
                            IconButton::new("xsmall", IconName::Check)
                                .icon_size(IconSize::XSmall)
                                .style(ButtonStyle::Filled)
                                .layer(ElevationIndex::Background)
                                .into_any_element(),
                        ),
                        single_example(
                            "Small",
                            IconButton::new("small", IconName::Check)
                                .icon_size(IconSize::Small)
                                .style(ButtonStyle::Filled)
                                .layer(ElevationIndex::Background)
                                .into_any_element(),
                        ),
                        single_example(
                            "Medium",
                            IconButton::new("medium", IconName::Check)
                                .icon_size(IconSize::Medium)
                                .style(ButtonStyle::Filled)
                                .layer(ElevationIndex::Background)
                                .into_any_element(),
                        ),
                        single_example(
                            "XLarge",
                            IconButton::new("xlarge", IconName::Check)
                                .icon_size(IconSize::XLarge)
                                .style(ButtonStyle::Filled)
                                .layer(ElevationIndex::Background)
                                .into_any_element(),
                        ),
                    ],
                ),
                example_group_with_title(
                    "Special States",
                    vec![
                        single_example(
                            "Disabled",
                            IconButton::new("disabled", IconName::Check)
                                .disabled(true)
                                .style(ButtonStyle::Filled)
                                .layer(ElevationIndex::Background)
                                .into_any_element(),
                        ),
                        single_example(
                            "Selected",
                            IconButton::new("selected", IconName::Check)
                                .toggle_state(true)
                                .style(ButtonStyle::Filled)
                                .layer(ElevationIndex::Background)
                                .into_any_element(),
                        ),
                        single_example(
                            "With Indicator",
                            IconButton::new("indicator", IconName::Check)
                                .indicator(Indicator::dot().color(Color::Success))
                                .style(ButtonStyle::Filled)
                                .layer(ElevationIndex::Background)
                                .into_any_element(),
                        ),
                        single_example(
                            "With Tooltip",
                            IconButton::new("tooltip", IconName::Check)
                                .style(ButtonStyle::Filled)
                                .layer(ElevationIndex::Background)
                                .tooltip(Tooltip::text("As mentioned - with a tooltip"))
                                .into_any_element(),
                        ),
                    ],
                ),
                example_group_with_title(
                    "Custom Colors",
                    vec![
                        single_example(
                            "Custom Icon Color",
                            IconButton::new("custom_color", IconName::Check)
                                .icon_color(Color::Accent)
                                .style(ButtonStyle::Filled)
                                .layer(ElevationIndex::Background)
                                .into_any_element(),
                        ),
                        single_example(
                            "With Alpha",
                            IconButton::new("alpha", IconName::Check)
                                .alpha(0.5)
                                .style(ButtonStyle::Filled)
                                .layer(ElevationIndex::Background)
                                .into_any_element(),
                        ),
                    ],
                ),
            ])
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Render, TestAppContext, VisualTestContext};

    struct SizeProbe;

    impl Render for SizeProbe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            h_flex()
                .child(IconButton::new("wide", IconName::Check))
                .child(IconButton::new("square", IconName::Close).shape(IconButtonShape::Square))
                .child(
                    IconButton::new("wide-box", IconName::Dash)
                        .shape(IconButtonShape::Wide)
                        .box_size(px(30.)),
                )
                .child(
                    IconButton::new("square-box", IconName::ChevronDown)
                        .shape(IconButtonShape::Square)
                        .box_size(px(30.)),
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

    fn bounds(cx: &mut VisualTestContext, selector: &'static str) -> gpui::Bounds<Pixels> {
        cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is painted"))
    }

    #[track_caller]
    fn assert_box(cx: &mut VisualTestContext, selector: &'static str, expected: f32) {
        let bounds = bounds(cx, selector);
        for side in [bounds.size.width, bounds.size.height] {
            assert!(
                (f32::from(side) - expected).abs() < 1e-3,
                "{selector} is {bounds:?}, expected {expected} px square"
            );
        }
    }

    #[gpui::test]
    fn icon_button_chrome_size_wide_default_is_the_toolbar_box(cx: &mut TestAppContext) {
        init_theme(cx);
        let (_, cx) = cx.add_window_view(|_, _| SizeProbe);
        assert_box(cx, "ICON-Check", 22.);
        assert_box(cx, "ICON-Close", 22.);
    }

    #[gpui::test]
    fn icon_button_chrome_size_box_size_overrides_shape(cx: &mut TestAppContext) {
        init_theme(cx);
        let (_, cx) = cx.add_window_view(|_, _| SizeProbe);
        assert_box(cx, "ICON-Dash", 30.);
        assert_box(cx, "ICON-ChevronDown", 30.);
    }
}
