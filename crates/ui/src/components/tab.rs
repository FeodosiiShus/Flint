use std::cmp::Ordering;

use gpui::{AnyElement, Hsla, IntoElement, Rgba, Stateful, transparent_black};
use smallvec::SmallVec;
use theme::{Appearance, ThemeColors, UiDensity};

use crate::prelude::*;

const START_TAB_SLOT_SIZE: Pixels = px(12.);
const END_TAB_SLOT_SIZE: Pixels = px(14.);
const TAB_BORDER_WIDTH: Pixels = px(1.);
const ISLAND_TAB_UNSELECTED_TEXT_WEIGHT: f32 = 0.7;
const ISLAND_TAB_UNSELECTED_ICON_OPACITY: f32 = 0.75;
const ISLAND_TAB_FIRST_TAB_OFFSET: Pixels = px(3.);
const ISLAND_TAB_CLOSE_SLOT_SIZE: Pixels = px(16.);
const ISLAND_TAB_MINIMUM_TAB_WIDTH: Pixels = px(50.);
const ISLAND_TAB_FILE_COLOR_ALPHA: f32 = 0.4;
/// IntelliJ's island tab pill is opaque because it sits on an opaque island; on glass it keeps
/// the material visible, like the pills macOS draws over its own glass.
const ISLAND_TAB_PILL_ON_GLASS_OPACITY: f32 = 0.5;
const ISLAND_TAB_FILE_COLOR_ARC_SHRINK: Pixels = px(2.);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IslandTabMetrics {
    pub strip_height: Pixels,
    pub header_height: Pixels,
    pub bottom_border_width: Pixels,
    pub pill_height: Pixels,
    pub pill_radius: Pixels,
    pub pill_inset: Pixels,
    pub first_tab_offset: Pixels,
    pub close_slot_size: Pixels,
    pub content_start_padding: Pixels,
    pub content_end_padding: Pixels,
    pub slot_gap: Pixels,
    pub leading_close_start_padding: Pixels,
    pub leading_close_end_padding: Pixels,
    pub leading_close_gap: Pixels,
    pub minimum_tab_width: Pixels,
    pub label_font_size_offset: Pixels,
    pub file_color_inset: Pixels,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IslandTabPadding {
    pub start: Pixels,
    pub gap: Pixels,
    pub end: Pixels,
}

impl IslandTabMetrics {
    pub fn for_density(density: UiDensity) -> Self {
        let compact = density == UiDensity::Compact;
        let pick =
            |default: f32, compact_value: f32| px(if compact { compact_value } else { default });
        Self {
            strip_height: pick(42., 32.),
            header_height: pick(41., 31.),
            bottom_border_width: TAB_BORDER_WIDTH,
            pill_height: pick(28., 24.),
            pill_radius: pick(6., 4.),
            pill_inset: pick(4., 2.),
            first_tab_offset: ISLAND_TAB_FIRST_TAB_OFFSET,
            close_slot_size: ISLAND_TAB_CLOSE_SLOT_SIZE,
            content_start_padding: pick(8., 4.),
            content_end_padding: pick(4., 2.),
            slot_gap: pick(2., 0.),
            leading_close_start_padding: pick(4., 4.),
            leading_close_end_padding: pick(4., 2.),
            leading_close_gap: pick(6., 0.),
            minimum_tab_width: ISLAND_TAB_MINIMUM_TAB_WIDTH,
            label_font_size_offset: pick(0., -1.),
            file_color_inset: pick(3., 2.),
        }
    }

    pub fn visible_gap_between_pills(&self) -> Pixels {
        self.pill_inset * 2. - TAB_BORDER_WIDTH
    }

    pub fn first_pill_offset(&self) -> Pixels {
        self.first_tab_offset + self.pill_inset
    }

    pub fn minimum_pill_width(&self) -> Pixels {
        self.minimum_tab_width - self.pill_inset * 2.
    }

    pub fn file_color_inner_radius(&self) -> Pixels {
        (self.pill_radius * 2. - self.file_color_inset - ISLAND_TAB_FILE_COLOR_ARC_SHRINK)
            .max(Pixels::ZERO)
            * 0.5
    }

    pub fn padding(&self, close_side: TabCloseSide, has_close_slot: bool) -> IslandTabPadding {
        match (close_side, has_close_slot) {
            (TabCloseSide::Start, true) => IslandTabPadding {
                start: self.leading_close_start_padding,
                gap: self.leading_close_gap,
                end: self.leading_close_end_padding,
            },
            _ => IslandTabPadding {
                start: self.content_start_padding,
                gap: self.slot_gap,
                end: self.content_end_padding,
            },
        }
    }
}

pub fn island_tab_metrics(cx: &App) -> IslandTabMetrics {
    IslandTabMetrics::for_density(current_ui_density(cx))
}

pub fn island_tab_label_size(density: UiDensity, rem_size: Pixels) -> LabelSize {
    let offset = IslandTabMetrics::for_density(density).label_font_size_offset;
    let font_size = (TextSize::label_size_for(rem_size) + offset).max(px(1.));
    LabelSize::Custom(rems(font_size / rem_size))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IslandTabPillColors {
    pub background: Hsla,
    pub border: Hsla,
}

pub fn island_tab_pill_colors(pane_focused: bool, colors: &ThemeColors) -> IslandTabPillColors {
    if pane_focused {
        IslandTabPillColors {
            background: colors.tab_active_background,
            border: colors.border_selected,
        }
    } else {
        IslandTabPillColors {
            background: colors.background,
            border: colors.border,
        }
    }
}

pub fn island_tab_hover_background(
    selected: bool,
    pane_focused: bool,
    colors: &ThemeColors,
) -> Option<Hsla> {
    (!selected || !pane_focused).then_some(colors.ghost_element_hover)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IslandTabFills {
    pub pill: Option<Hsla>,
    pub hover: Option<Hsla>,
    pub file_color_inner: Option<Hsla>,
}

pub fn island_tab_fills(
    selected: bool,
    pane_focused: bool,
    file_color: Option<Hsla>,
    colors: &ThemeColors,
) -> IslandTabFills {
    let hover = island_tab_hover_background(selected, pane_focused, colors);
    match (selected, file_color) {
        (true, Some(file_color)) => IslandTabFills {
            pill: Some(colors.editor_background),
            hover: None,
            file_color_inner: Some(if pane_focused {
                file_color
            } else {
                file_color.alpha(ISLAND_TAB_FILE_COLOR_ALPHA)
            }),
        },
        (true, None) => IslandTabFills {
            pill: Some(island_tab_pill_colors(pane_focused, colors).background),
            hover,
            file_color_inner: None,
        },
        (false, Some(file_color)) => {
            let fill = file_color.alpha(ISLAND_TAB_FILE_COLOR_ALPHA);
            IslandTabFills {
                pill: Some(fill),
                hover: hover.map(|hover| alpha_blending(hover, fill)),
                file_color_inner: None,
            }
        }
        (false, None) => IslandTabFills {
            pill: None,
            hover,
            file_color_inner: None,
        },
    }
}

fn alpha_blending(foreground: Hsla, background: Hsla) -> Hsla {
    let foreground = Rgba::from(foreground);
    let background = Rgba::from(background);
    let alpha = foreground.a + background.a * (1. - foreground.a);
    if alpha <= 0. {
        return transparent_black();
    }
    let blend_channel = |foreground_channel: f32, background_channel: f32| {
        (foreground_channel * foreground.a
            + background_channel * background.a * (1. - foreground.a))
            / alpha
    };
    Hsla::from(Rgba {
        r: blend_channel(foreground.r, background.r),
        g: blend_channel(foreground.g, background.g),
        b: blend_channel(foreground.b, background.b),
        a: alpha,
    })
}

pub fn island_tab_label_color(
    selected: bool,
    pane_focused: bool,
    hovered: bool,
    colors: &ThemeColors,
    appearance: Appearance,
) -> Hsla {
    if selected && !pane_focused {
        return match appearance {
            Appearance::Dark => gpui::white(),
            Appearance::Light => gpui::black(),
        };
    }
    if selected || hovered {
        colors.text
    } else {
        blend_toward_background(
            colors.text,
            colors.editor_background,
            ISLAND_TAB_UNSELECTED_TEXT_WEIGHT,
        )
    }
}

pub fn island_tab_icon_opacity(selected: bool, hovered: bool) -> f32 {
    if selected || hovered {
        1.
    } else {
        ISLAND_TAB_UNSELECTED_ICON_OPACITY
    }
}

fn blend_toward_background(foreground: Hsla, background: Hsla, foreground_weight: f32) -> Hsla {
    let foreground = Rgba::from(foreground);
    let background = Rgba::from(background);
    let blend_channel = |foreground: f32, background: f32| {
        (foreground * foreground * foreground_weight
            + background * background * (1. - foreground_weight))
            .sqrt()
    };
    Hsla::from(Rgba {
        r: blend_channel(foreground.r, background.r),
        g: blend_channel(foreground.g, background.g),
        b: blend_channel(foreground.b, background.b),
        a: foreground.a,
    })
}

fn current_ui_density(cx: &App) -> UiDensity {
    theme::theme_settings(cx).ui_density(cx)
}

/// The position of a [`Tab`] within a list of tabs.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum TabPosition {
    /// The tab is first in the list.
    First,

    /// The tab is in the middle of the list (i.e., it is not the first or last tab).
    ///
    /// The [`Ordering`] is where this tab is positioned with respect to the selected tab.
    Middle(Ordering),

    /// The tab is last in the list.
    Last,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum TabCloseSide {
    Start,
    End,
}

#[derive(IntoElement, RegisterComponent)]
pub struct Tab {
    div: Stateful<Div>,
    selected: bool,
    focused: bool,
    islands: bool,
    file_color: Option<Hsla>,
    position: TabPosition,
    close_side: TabCloseSide,
    start_slot: Option<AnyElement>,
    end_slot: Option<AnyElement>,
    children: SmallVec<[AnyElement; 2]>,
}

impl Tab {
    pub fn new(id: impl Into<ElementId>) -> Self {
        let id = id.into();
        Self {
            div: div()
                .id(id.clone())
                .debug_selector(|| format!("TAB-{}", id)),
            selected: false,
            focused: false,
            islands: false,
            file_color: None,
            position: TabPosition::First,
            close_side: TabCloseSide::End,
            start_slot: None,
            end_slot: None,
            children: SmallVec::new(),
        }
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn file_color(mut self, file_color: Option<Hsla>) -> Self {
        self.file_color = file_color;
        self
    }

    pub fn islands(mut self, islands: bool) -> Self {
        self.islands = islands;
        self
    }

    pub fn position(mut self, position: TabPosition) -> Self {
        self.position = position;
        self
    }

    pub fn close_side(mut self, close_side: TabCloseSide) -> Self {
        self.close_side = close_side;
        self
    }

    pub fn start_slot<E: IntoElement>(mut self, element: impl Into<Option<E>>) -> Self {
        self.start_slot = element.into().map(IntoElement::into_any_element);
        self
    }

    pub fn end_slot<E: IntoElement>(mut self, element: impl Into<Option<E>>) -> Self {
        self.end_slot = element.into().map(IntoElement::into_any_element);
        self
    }

    pub fn content_height(cx: &App) -> Pixels {
        Self::container_height(cx) - TAB_BORDER_WIDTH
    }

    pub fn container_height(cx: &App) -> Pixels {
        let region = crate::ChromeRegion::TabBar;
        let default_height = island_tab_metrics(cx).header_height;
        let height = crate::chrome_height(region, cx);
        let button_height = crate::chrome_button_height(region, ButtonSize::Default, cx);
        if height.is_none() && button_height.is_none() {
            return default_height;
        }
        let height = height.unwrap_or(default_height);
        let button_height = crate::chrome_fit_height(region, ButtonSize::Default, cx);
        height.max(button_height + TAB_BORDER_WIDTH)
    }

    pub fn island_strip_height(cx: &App) -> Pixels {
        let metrics = island_tab_metrics(cx);
        let container_height = Self::container_height(cx);
        if crate::chrome_height(crate::ChromeRegion::TabBar, cx).is_some() {
            container_height
        } else {
            container_height + (metrics.strip_height - metrics.header_height)
        }
    }
}

impl InteractiveElement for Tab {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.div.interactivity()
    }
}

impl StatefulInteractiveElement for Tab {}

impl Toggleable for Tab {
    fn toggle_state(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
}

impl ParentElement for Tab {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements)
    }
}

impl RenderOnce for Tab {
    #[allow(refining_impl_trait)]
    fn render(self, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        let (text_color, tab_bg, _tab_hover_bg, _tab_active_bg) = match self.selected {
            false => (
                cx.theme().colors().text_muted,
                cx.theme().colors().tab_inactive_background,
                cx.theme().colors().ghost_element_hover,
                cx.theme().colors().ghost_element_active,
            ),
            true => (
                cx.theme().colors().text,
                cx.theme().colors().tab_active_background,
                cx.theme().colors().element_hover,
                cx.theme().colors().element_active,
            ),
        };

        let slot_scale = crate::chrome_icon_scale(crate::ChromeRegion::TabBar, cx);

        if self.islands {
            let colors = cx.theme().colors();
            let metrics = IslandTabMetrics::for_density(current_ui_density(cx));
            let pill = island_tab_pill_colors(self.focused, colors);
            let mut fills = island_tab_fills(self.selected, self.focused, self.file_color, colors);
            if window.on_glass_surface() {
                // A pill on glass keeps its colour but not its opacity: an opaque pill would
                // punch a flat hole through the material.
                fills.pill = fills
                    .pill
                    .map(|pill| pill.opacity(ISLAND_TAB_PILL_ON_GLASS_OPACITY));
                fills.hover = fills
                    .hover
                    .map(|hover| hover.opacity(ISLAND_TAB_PILL_ON_GLASS_OPACITY));
            }
            let transparent_border = colors.border_transparent;
            let border = if self.selected {
                pill.border
            } else {
                transparent_border
            };
            let file_color_offset = metrics.file_color_inset - TAB_BORDER_WIDTH;
            let file_color_inner = fills.file_color_inner.map(|file_color| {
                div()
                    .debug_selector(|| "TAB_FILE_COLOR".into())
                    .absolute()
                    .top(file_color_offset)
                    .left(file_color_offset)
                    .right(file_color_offset)
                    .bottom(file_color_offset)
                    .rounded(metrics.file_color_inner_radius())
                    .bg(file_color)
            });
            let strip_height = Tab::island_strip_height(cx);
            let pill_height = metrics.pill_height.min(strip_height);
            let first_tab_offset = if self.position == TabPosition::First {
                metrics.first_tab_offset
            } else {
                px(0.)
            };
            let close_slot_size = match slot_scale {
                Some(scale) => metrics.close_slot_size * scale,
                None => metrics.close_slot_size,
            };
            let padding = metrics.padding(self.close_side, self.end_slot.is_some());
            let close_slot = self.end_slot.map(|end_slot| {
                h_flex()
                    .size(close_slot_size)
                    .flex_none()
                    .justify_center()
                    .child(end_slot)
            });
            let (leading_slot, trailing_slot) = match self.close_side {
                TabCloseSide::End => (None, close_slot),
                TabCloseSide::Start => (close_slot, None),
            };

            let content = h_flex()
                .group("")
                .relative()
                .h(pill_height)
                .min_w(metrics.minimum_pill_width())
                .pl(padding.start - TAB_BORDER_WIDTH)
                .pr(padding.end - TAB_BORDER_WIDTH)
                .gap(padding.gap)
                .text_color(text_color)
                .rounded(metrics.pill_radius)
                .border_1()
                .border_color(border)
                .children(file_color_inner)
                .children(self.start_slot)
                .children(leading_slot)
                .children(self.children)
                .children(trailing_slot)
                .when_some(fills.pill, |this, pill_background| this.bg(pill_background))
                .when_some(fills.hover, |this, hover_background| {
                    this.hover(move |style| style.bg(hover_background))
                });

            return self
                .div
                .h(strip_height)
                .flex()
                .items_center()
                .pl(first_tab_offset + metrics.pill_inset)
                .pr(metrics.pill_inset - TAB_BORDER_WIDTH)
                .cursor_pointer()
                .child(content);
        }

        let (start_slot_size, end_slot_size) = match slot_scale {
            Some(scale) => (START_TAB_SLOT_SIZE * scale, END_TAB_SLOT_SIZE * scale),
            None => (START_TAB_SLOT_SIZE, END_TAB_SLOT_SIZE),
        };

        let (start_slot, end_slot) = {
            let start_slot = h_flex()
                .size(start_slot_size)
                .justify_center()
                .children(self.start_slot);

            let end_slot = h_flex()
                .size(end_slot_size)
                .justify_center()
                .children(self.end_slot);

            match self.close_side {
                TabCloseSide::End => (start_slot, end_slot),
                TabCloseSide::Start => (end_slot, start_slot),
            }
        };

        let content = h_flex()
            .group("")
            .relative()
            .px(DynamicSpacing::Base04.px(cx))
            .gap(DynamicSpacing::Base04.rems(cx))
            .text_color(text_color)
            .child(start_slot)
            .children(self.children)
            .child(end_slot);

        self.div
            .h(Tab::container_height(cx))
            .bg(tab_bg)
            .child(crate::background_image_layer(
                crate::BackgroundImageTarget::EditorAndTools,
                crate::BackgroundImageArea::Window,
                tab_bg,
                true,
                gpui::Corners::default(),
            ))
            .border_color(cx.theme().colors().border)
            .map(|this| match self.position {
                TabPosition::First => {
                    if self.selected {
                        this.pl_px().border_r_1().pb_px()
                    } else {
                        this.pl_px().pr_px().border_b_1()
                    }
                }
                TabPosition::Last => {
                    if self.selected {
                        this.border_l_1().border_r_1().pb_px()
                    } else {
                        this.pl_px().border_b_1().border_r_1()
                    }
                }
                TabPosition::Middle(Ordering::Equal) => this.border_l_1().border_r_1().pb_px(),
                TabPosition::Middle(Ordering::Less) => this.border_l_1().pr_px().border_b_1(),
                TabPosition::Middle(Ordering::Greater) => this.border_r_1().pl_px().border_b_1(),
            })
            .cursor_pointer()
            .child(content.h(Tab::content_height(cx)))
    }
}

impl Component for Tab {
    fn scope() -> ComponentScope {
        ComponentScope::Navigation
    }

    fn description() -> &'static str {
        "A tab component that can be used in a tabbed interface, \
        supporting different positions and states."
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> AnyElement {
        v_flex()
            .gap_6()
            .children(vec![example_group_with_title(
                "Variations",
                vec![
                    single_example(
                        "Default",
                        Tab::new("default").child("Default Tab").into_any_element(),
                    ),
                    single_example(
                        "Selected",
                        Tab::new("selected")
                            .toggle_state(true)
                            .child("Selected Tab")
                            .into_any_element(),
                    ),
                    single_example(
                        "First",
                        Tab::new("first")
                            .position(TabPosition::First)
                            .child("First Tab")
                            .into_any_element(),
                    ),
                    single_example(
                        "Middle",
                        Tab::new("middle")
                            .position(TabPosition::Middle(Ordering::Equal))
                            .child("Middle Tab")
                            .into_any_element(),
                    ),
                    single_example(
                        "Last",
                        Tab::new("last")
                            .position(TabPosition::Last)
                            .child("Last Tab")
                            .into_any_element(),
                    ),
                ],
            )])
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Render, TestAppContext, rgb};

    const LABEL_WIDTH: f32 = 48.;
    const EIGHT_BIT_TOLERANCE: u8 = 1;

    struct IslandTabRow;

    impl Render for IslandTabRow {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            h_flex()
                .child(
                    Tab::new("visible_close")
                        .islands(true)
                        .position(TabPosition::Middle(Ordering::Equal))
                        .child(div().w(px(LABEL_WIDTH)))
                        .end_slot(div().size_full().bg(gpui::red())),
                )
                .child(
                    Tab::new("hidden_close")
                        .islands(true)
                        .position(TabPosition::Middle(Ordering::Equal))
                        .child(div().w(px(LABEL_WIDTH)))
                        .end_slot(div().size_full().invisible()),
                )
                .child(
                    Tab::new("without_close")
                        .islands(true)
                        .position(TabPosition::Middle(Ordering::Equal))
                        .child(div().w(px(LABEL_WIDTH))),
                )
        }
    }

    fn eight_bit(color: Hsla) -> [u8; 3] {
        let rgba = Rgba::from(color);
        [rgba.r, rgba.g, rgba.b].map(|channel| (channel * 255.).round() as u8)
    }

    #[test]
    fn island_tab_strip_is_the_header_height_plus_the_one_pixel_top_inset() {
        for (density, header, strip) in [
            (UiDensity::Default, 41., 42.),
            (UiDensity::Comfortable, 41., 42.),
            (UiDensity::Compact, 31., 32.),
        ] {
            let metrics = IslandTabMetrics::for_density(density);
            assert_eq!(metrics.header_height, px(header));
            assert_eq!(metrics.strip_height, px(strip));
            assert_eq!(
                metrics.strip_height - metrics.header_height,
                metrics.bottom_border_width
            );
        }
    }

    #[test]
    fn island_tab_pill_height_and_radius_follow_density() {
        let default = IslandTabMetrics::for_density(UiDensity::Default);
        let compact = IslandTabMetrics::for_density(UiDensity::Compact);
        assert_eq!(
            (default.pill_height, default.pill_radius),
            (px(28.), px(6.))
        );
        assert_eq!(
            (compact.pill_height, compact.pill_radius),
            (px(24.), px(4.))
        );
        assert!(default.pill_height < default.strip_height);
        assert!(compact.pill_height < compact.strip_height);
    }

    #[test]
    fn island_tab_pills_are_seven_apart_and_start_seven_from_the_strip_edge() {
        let default = IslandTabMetrics::for_density(UiDensity::Default);
        let compact = IslandTabMetrics::for_density(UiDensity::Compact);
        assert_eq!(default.visible_gap_between_pills(), px(7.));
        assert_eq!(default.first_pill_offset(), px(7.));
        assert_eq!(compact.visible_gap_between_pills(), px(3.));
        assert_eq!(compact.first_pill_offset(), px(5.));
    }

    #[test]
    fn island_tab_pill_sits_seven_and_four_pixels_below_the_strip_top() {
        let default = IslandTabMetrics::for_density(UiDensity::Default);
        let compact = IslandTabMetrics::for_density(UiDensity::Compact);
        assert_eq!((default.strip_height - default.pill_height) / 2., px(7.));
        assert_eq!((compact.strip_height - compact.pill_height) / 2., px(4.));
    }

    #[test]
    fn island_tab_content_paddings_follow_the_intellij_tab_insets() {
        let default = IslandTabMetrics::for_density(UiDensity::Default);
        let compact = IslandTabMetrics::for_density(UiDensity::Compact);
        let padding = |start: f32, gap: f32, end: f32| IslandTabPadding {
            start: px(start),
            gap: px(gap),
            end: px(end),
        };
        assert_eq!(default.close_slot_size, px(16.));
        assert_eq!(compact.close_slot_size, px(16.));
        assert_eq!(
            default.padding(TabCloseSide::End, true),
            padding(8., 2., 4.)
        );
        assert_eq!(
            compact.padding(TabCloseSide::End, true),
            padding(4., 0., 2.)
        );
        assert_eq!(
            default.padding(TabCloseSide::Start, true),
            padding(4., 6., 4.)
        );
        assert_eq!(
            compact.padding(TabCloseSide::Start, true),
            padding(4., 0., 2.)
        );
        for metrics in [default, compact] {
            assert_eq!(
                metrics.padding(TabCloseSide::Start, false),
                metrics.padding(TabCloseSide::End, false),
                "without a close slot both sides share the right-hand insets"
            );
        }
    }

    #[test]
    fn island_tab_minimum_width_is_fifty_counting_the_pill_insets() {
        for density in [UiDensity::Default, UiDensity::Compact] {
            let metrics = IslandTabMetrics::for_density(density);
            assert_eq!(metrics.minimum_tab_width, px(50.));
            assert_eq!(
                metrics.minimum_pill_width() + metrics.pill_inset * 2.,
                px(50.)
            );
        }
    }

    #[test]
    fn island_tab_focused_pill_uses_the_active_tab_fill_and_selected_border() {
        for colors in [ThemeColors::dark(), ThemeColors::light()] {
            let pill = island_tab_pill_colors(true, &colors);
            assert_eq!(pill.background, colors.tab_active_background);
            assert_eq!(pill.border, colors.border_selected);
        }
    }

    #[test]
    fn island_tab_unfocused_selected_pill_uses_the_frame_fill_and_border() {
        for colors in [ThemeColors::dark(), ThemeColors::light()] {
            let pill = island_tab_pill_colors(false, &colors);
            assert_eq!(pill.background, colors.background);
            assert_eq!(pill.border, colors.border);
            assert_ne!(
                pill.border,
                island_tab_pill_colors(true, &colors).border,
                "focus must be visible from the pill border alone"
            );
        }
    }

    #[test]
    fn island_tab_hover_fill_applies_except_to_the_selected_tab_of_a_focused_pane() {
        let colors = ThemeColors::dark();
        assert_eq!(
            island_tab_hover_background(false, true, &colors),
            Some(colors.ghost_element_hover)
        );
        assert_eq!(
            island_tab_hover_background(false, false, &colors),
            Some(colors.ghost_element_hover)
        );
        assert_eq!(
            island_tab_hover_background(true, false, &colors),
            Some(colors.ghost_element_hover)
        );
        assert_eq!(island_tab_hover_background(true, true, &colors), None);
    }

    #[test]
    fn island_tab_unselected_label_is_the_text_blended_toward_the_strip_background() {
        let mut colors = ThemeColors::dark();
        colors.text = Hsla::from(rgb(0xD1D3D9));
        colors.editor_background = Hsla::from(rgb(0x191A1C));
        let blended = eight_bit(island_tab_label_color(
            false,
            true,
            false,
            &colors,
            Appearance::Dark,
        ));
        let expected = [0xAF, 0xB1, 0xB6];
        for (actual, expected) in blended.into_iter().zip(expected) {
            assert!(
                actual.abs_diff(expected) <= EIGHT_BIT_TOLERANCE,
                "blend {blended:?} should match {expected:?}"
            );
        }
    }

    #[test]
    fn island_tab_selected_and_hovered_labels_use_the_full_text_color_in_a_focused_pane() {
        let colors = ThemeColors::dark();
        let label = |selected, hovered| {
            island_tab_label_color(selected, true, hovered, &colors, Appearance::Dark)
        };
        assert_eq!(label(true, false), colors.text);
        assert_eq!(label(false, true), colors.text);
        assert_ne!(label(false, false), colors.text);
    }

    #[test]
    fn island_tab_selected_label_of_an_unfocused_pane_is_pure_white_or_black() {
        let colors = ThemeColors::dark();
        for hovered in [false, true] {
            assert_eq!(
                island_tab_label_color(true, false, hovered, &colors, Appearance::Dark),
                gpui::white()
            );
            assert_eq!(
                island_tab_label_color(true, false, hovered, &colors, Appearance::Light),
                gpui::black()
            );
        }
        assert_eq!(
            island_tab_label_color(false, false, true, &colors, Appearance::Dark),
            colors.text
        );
        assert_ne!(
            island_tab_label_color(false, false, false, &colors, Appearance::Dark),
            colors.text
        );
    }

    #[test]
    fn island_tab_unselected_icon_is_dimmed_until_selected_or_hovered() {
        assert_eq!(island_tab_icon_opacity(false, false), 0.75);
        assert_eq!(island_tab_icon_opacity(true, false), 1.);
        assert_eq!(island_tab_icon_opacity(false, true), 1.);
    }

    #[gpui::test]
    fn island_tab_reserves_the_close_slot_whether_or_not_its_content_is_visible(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
        let (_, cx) = cx.add_window_view(|_, _| IslandTabRow);
        let visible = cx
            .debug_bounds("TAB-visible_close")
            .expect("tab is painted");
        let hidden = cx.debug_bounds("TAB-hidden_close").expect("tab is painted");
        let without = cx
            .debug_bounds("TAB-without_close")
            .expect("tab is painted");
        let metrics = IslandTabMetrics::for_density(UiDensity::Default);
        assert_eq!(visible.size.width, hidden.size.width);
        assert_eq!(
            visible.size.width - without.size.width,
            metrics.close_slot_size + metrics.slot_gap
        );
        assert_eq!(
            visible.size.width,
            metrics.pill_inset * 2. - TAB_BORDER_WIDTH
                + metrics.content_start_padding
                + px(LABEL_WIDTH)
                + metrics.slot_gap
                + metrics.close_slot_size
                + metrics.content_end_padding
        );
        assert_eq!(visible.size.height, metrics.strip_height);
    }

    #[gpui::test]
    fn island_tab_draws_no_divider_element(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
        let (_, cx) = cx.add_window_view(|_, _| IslandTabRow);
        assert!(cx.debug_bounds("TAB-visible_close").is_some());
        for (name, selector) in [
            ("visible_close", "TAB_DIVIDER-visible_close"),
            ("hidden_close", "TAB_DIVIDER-hidden_close"),
            ("without_close", "TAB_DIVIDER-without_close"),
        ] {
            assert!(
                cx.debug_bounds(selector).is_none(),
                "tab {name} must not paint a divider"
            );
        }
    }

    struct IslandFileColorTab {
        selected: bool,
    }

    impl Render for IslandFileColorTab {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            h_flex().child(
                Tab::new("file_color")
                    .islands(true)
                    .focused(true)
                    .toggle_state(self.selected)
                    .file_color(Some(gpui::green()))
                    .position(TabPosition::Middle(Ordering::Equal))
                    .child(div().w(px(LABEL_WIDTH))),
            )
        }
    }

    fn init_island_tab_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
    }

    #[test]
    fn island_tab_label_is_two_pixels_below_the_ui_font_and_one_smaller_when_compact() {
        for (density, rem_size, expected) in [
            (UiDensity::Default, 15., 13.),
            (UiDensity::Comfortable, 15., 13.),
            (UiDensity::Compact, 15., 12.),
            (UiDensity::Default, 16., 14.),
            (UiDensity::Compact, 16., 13.),
        ] {
            let LabelSize::Custom(size) = island_tab_label_size(density, px(rem_size)) else {
                panic!("island tabs always draw a custom label size");
            };
            let actual = f32::from(size.to_pixels(px(rem_size)));
            assert!(
                (actual - expected).abs() < 0.001,
                "{density:?} at a {rem_size} px UI font should be {expected} px, got {actual}"
            );
        }
    }

    #[test]
    fn island_tab_file_color_inset_and_inner_radius_follow_density() {
        let default = IslandTabMetrics::for_density(UiDensity::Default);
        let compact = IslandTabMetrics::for_density(UiDensity::Compact);
        assert_eq!(default.file_color_inset, px(3.));
        assert_eq!(compact.file_color_inset, px(2.));
        assert_eq!(default.file_color_inner_radius(), px(3.5));
        assert_eq!(compact.file_color_inner_radius(), px(2.));
    }

    #[test]
    fn island_tab_selected_file_colored_tab_fills_the_inner_pill_with_the_file_color() {
        let colors = ThemeColors::dark();
        let file_color = Hsla::from(rgb(0x1C261C));
        let focused = island_tab_fills(true, true, Some(file_color), &colors);
        assert_eq!(focused.pill, Some(colors.editor_background));
        assert_eq!(focused.file_color_inner, Some(file_color));
        assert_eq!(focused.hover, None);
        let unfocused = island_tab_fills(true, false, Some(file_color), &colors);
        assert_eq!(unfocused.pill, Some(colors.editor_background));
        assert_eq!(unfocused.file_color_inner, Some(file_color.alpha(0.4)));
        assert_eq!(unfocused.hover, None);
    }

    #[test]
    fn island_tab_unselected_file_colored_tab_is_tinted_at_forty_percent() {
        let colors = ThemeColors::dark();
        let file_color = Hsla::from(rgb(0x1C261C));
        let tinted = file_color.alpha(0.4);
        let fills = island_tab_fills(false, true, Some(file_color), &colors);
        assert_eq!(fills.pill, Some(tinted));
        assert_eq!(fills.file_color_inner, None);
        let hover = fills.hover.expect("an unselected tab has a hover fill");
        assert_eq!(hover, alpha_blending(colors.ghost_element_hover, tinted));
        assert!(
            hover.a >= tinted.a,
            "the hover fill is composed over the tint, not instead of it"
        );
    }

    #[test]
    fn island_tab_without_file_color_keeps_the_plain_fills() {
        for colors in [ThemeColors::dark(), ThemeColors::light()] {
            for pane_focused in [true, false] {
                let selected = island_tab_fills(true, pane_focused, None, &colors);
                assert_eq!(
                    selected.pill,
                    Some(island_tab_pill_colors(pane_focused, &colors).background)
                );
                assert_eq!(
                    selected.hover,
                    island_tab_hover_background(true, pane_focused, &colors)
                );
                assert_eq!(selected.file_color_inner, None);
                let unselected = island_tab_fills(false, pane_focused, None, &colors);
                assert_eq!(unselected.pill, None);
                assert_eq!(unselected.hover, Some(colors.ghost_element_hover));
                assert_eq!(unselected.file_color_inner, None);
            }
        }
    }

    #[test]
    fn island_tab_alpha_blending_matches_intellij_color_util() {
        let white = Hsla::from(rgb(0xFFFFFF));
        let black = Hsla::from(rgb(0x000000));
        let blended = alpha_blending(white.alpha(0.5), black);
        assert_eq!(blended.a, 1.);
        for channel in eight_bit(blended) {
            assert!(
                channel.abs_diff(0x80) <= EIGHT_BIT_TOLERANCE,
                "half white over black must be mid grey, got {channel}"
            );
        }
        let translucent = alpha_blending(white.alpha(0.5), black.alpha(0.5));
        assert!((translucent.a - 0.75).abs() < 0.001);
        let opaque = alpha_blending(white, black.alpha(0.5));
        assert_eq!(opaque.a, 1.);
        assert_eq!(eight_bit(opaque), [0xFF, 0xFF, 0xFF]);
    }

    #[gpui::test]
    fn island_tab_selected_file_color_is_inset_inside_the_pill(cx: &mut TestAppContext) {
        init_island_tab_test(cx);
        let (_, cx) = cx.add_window_view(|_, _| IslandFileColorTab { selected: true });
        let tab = cx.debug_bounds("TAB-file_color").expect("tab is painted");
        let inner = cx
            .debug_bounds("TAB_FILE_COLOR")
            .expect("the selected file-colored tab paints an inner fill");
        let metrics = IslandTabMetrics::for_density(UiDensity::Default);
        let pill_width = tab.size.width - (metrics.pill_inset * 2. - TAB_BORDER_WIDTH);
        assert_eq!(
            inner.origin.x - tab.origin.x,
            metrics.pill_inset + metrics.file_color_inset
        );
        assert_eq!(inner.size.width, pill_width - metrics.file_color_inset * 2.);
        assert_eq!(
            inner.size.height,
            metrics.pill_height - metrics.file_color_inset * 2.
        );
    }

    #[gpui::test]
    fn island_tab_unselected_file_color_paints_no_inner_fill(cx: &mut TestAppContext) {
        init_island_tab_test(cx);
        let (_, cx) = cx.add_window_view(|_, _| IslandFileColorTab { selected: false });
        assert!(cx.debug_bounds("TAB-file_color").is_some());
        assert!(cx.debug_bounds("TAB_FILE_COLOR").is_none());
    }

    struct IslandTabSides {
        label_width: f32,
    }

    impl Render for IslandTabSides {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            h_flex()
                .child(
                    Tab::new("close_end")
                        .islands(true)
                        .position(TabPosition::Middle(Ordering::Equal))
                        .close_side(TabCloseSide::End)
                        .child(div().w(px(self.label_width)))
                        .end_slot(div().size_full().debug_selector(|| "CLOSE-end".into())),
                )
                .child(
                    Tab::new("close_start")
                        .islands(true)
                        .position(TabPosition::Middle(Ordering::Equal))
                        .close_side(TabCloseSide::Start)
                        .child(div().w(px(self.label_width)))
                        .end_slot(div().size_full().debug_selector(|| "CLOSE-start".into())),
                )
        }
    }

    #[gpui::test]
    fn island_tab_close_slot_sits_at_the_intellij_inset_on_either_side(cx: &mut TestAppContext) {
        init_island_tab_test(cx);
        let (_, cx) = cx.add_window_view(|_, _| IslandTabSides {
            label_width: LABEL_WIDTH,
        });
        let metrics = IslandTabMetrics::for_density(UiDensity::Default);
        let trailing_tab = cx.debug_bounds("TAB-close_end").expect("tab is painted");
        let trailing_close = cx.debug_bounds("CLOSE-end").expect("close slot is painted");
        let trailing_padding = metrics.padding(TabCloseSide::End, true);
        assert_eq!(trailing_close.size.width, metrics.close_slot_size);
        assert_eq!(
            trailing_tab.right() - trailing_close.right(),
            trailing_padding.end + metrics.pill_inset - TAB_BORDER_WIDTH
        );
        let leading_tab = cx.debug_bounds("TAB-close_start").expect("tab is painted");
        let leading_close = cx
            .debug_bounds("CLOSE-start")
            .expect("close slot is painted");
        let leading_padding = metrics.padding(TabCloseSide::Start, true);
        assert_eq!(
            leading_close.left() - leading_tab.left(),
            metrics.pill_inset + leading_padding.start
        );
        assert_eq!(
            leading_tab.size.width,
            metrics.pill_inset * 2. - TAB_BORDER_WIDTH
                + leading_padding.start
                + metrics.close_slot_size
                + leading_padding.gap
                + px(LABEL_WIDTH)
                + leading_padding.end
        );
    }

    #[gpui::test]
    fn island_tab_without_content_is_as_wide_as_the_intellij_minimum_tab(cx: &mut TestAppContext) {
        init_island_tab_test(cx);
        let (_, cx) = cx.add_window_view(|_, _| IslandTabSides { label_width: 1. });
        let metrics = IslandTabMetrics::for_density(UiDensity::Default);
        for selector in ["TAB-close_end", "TAB-close_start"] {
            let tab = cx.debug_bounds(selector).expect("tab is painted");
            assert_eq!(
                tab.size.width,
                metrics.minimum_tab_width - TAB_BORDER_WIDTH,
                "{selector} keeps the 50 px label minimum less the 1 px tab overlap"
            );
            assert_eq!(tab.size.height, metrics.strip_height);
        }
    }
}
