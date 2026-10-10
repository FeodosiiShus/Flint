use std::cmp::Ordering;

use gpui::{AnyElement, Hsla, IntoElement, Rgba, Stateful};
use smallvec::SmallVec;
use theme::{ThemeColors, UiDensity};

use crate::prelude::*;

const START_TAB_SLOT_SIZE: Pixels = px(12.);
const END_TAB_SLOT_SIZE: Pixels = px(14.);
const TAB_BORDER_WIDTH: Pixels = px(1.);
const ISLAND_TAB_UNSELECTED_TEXT_WEIGHT: f32 = 0.7;
const ISLAND_TAB_UNSELECTED_ICON_OPACITY: f32 = 0.75;
const ISLAND_TAB_FIRST_TAB_OFFSET: Pixels = px(3.);
const ISLAND_TAB_CLOSE_SLOT_SIZE: Pixels = px(16.);
const ISLAND_TAB_SLOT_GAP: Pixels = px(2.);
const ISLAND_TAB_CONTENT_START_PADDING: Pixels = px(8.);
const ISLAND_TAB_MINIMUM_CONTENT_WIDTH: Pixels = px(50.);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IslandTabMetrics {
    pub strip_height: Pixels,
    pub pill_height: Pixels,
    pub pill_radius: Pixels,
    pub pill_inset: Pixels,
    pub first_tab_offset: Pixels,
    pub close_slot_size: Pixels,
    pub content_start_padding: Pixels,
    pub content_end_padding: Pixels,
    pub slot_gap: Pixels,
    pub minimum_content_width: Pixels,
}

impl IslandTabMetrics {
    pub fn for_density(density: UiDensity) -> Self {
        let compact = density == UiDensity::Compact;
        let pick =
            |default: f32, compact_value: f32| px(if compact { compact_value } else { default });
        Self {
            strip_height: pick(41., 31.),
            pill_height: pick(28., 24.),
            pill_radius: pick(6., 4.),
            pill_inset: pick(4., 2.),
            first_tab_offset: ISLAND_TAB_FIRST_TAB_OFFSET,
            close_slot_size: ISLAND_TAB_CLOSE_SLOT_SIZE,
            content_start_padding: ISLAND_TAB_CONTENT_START_PADDING,
            content_end_padding: pick(4., 2.),
            slot_gap: ISLAND_TAB_SLOT_GAP,
            minimum_content_width: ISLAND_TAB_MINIMUM_CONTENT_WIDTH,
        }
    }

    pub fn visible_gap_between_pills(&self) -> Pixels {
        self.pill_inset * 2. - TAB_BORDER_WIDTH
    }

    pub fn first_pill_offset(&self) -> Pixels {
        self.first_tab_offset + self.pill_inset
    }
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

pub fn island_tab_label_color(selected: bool, hovered: bool, colors: &ThemeColors) -> Hsla {
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
        let default_height = IslandTabMetrics::for_density(current_ui_density(cx)).strip_height;
        let height = crate::chrome_height(region, cx);
        let button_height = crate::chrome_button_height(region, ButtonSize::Default, cx);
        if height.is_none() && button_height.is_none() {
            return default_height;
        }
        let height = height.unwrap_or(default_height);
        let button_height = crate::chrome_fit_height(region, ButtonSize::Default, cx);
        height.max(button_height + TAB_BORDER_WIDTH)
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
    fn render(self, _: &mut Window, cx: &mut App) -> Stateful<Div> {
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
            let hover_background = island_tab_hover_background(self.selected, self.focused, colors);
            let transparent_border = colors.border_transparent;
            let container_height = Tab::container_height(cx);
            let pill_height = metrics.pill_height.min(container_height);
            let first_tab_offset = if self.position == TabPosition::First {
                metrics.first_tab_offset
            } else {
                px(0.)
            };
            let close_slot_size = match slot_scale {
                Some(scale) => metrics.close_slot_size * scale,
                None => metrics.close_slot_size,
            };
            let (start_padding, end_padding) = match self.close_side {
                TabCloseSide::End => (metrics.content_start_padding, metrics.content_end_padding),
                TabCloseSide::Start => (metrics.content_end_padding, metrics.content_start_padding),
            };
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
                .pl(start_padding - TAB_BORDER_WIDTH)
                .pr(end_padding - TAB_BORDER_WIDTH)
                .gap(metrics.slot_gap)
                .text_color(text_color)
                .rounded(metrics.pill_radius)
                .border_1()
                .children(self.start_slot)
                .children(leading_slot)
                .children(self.children)
                .children(trailing_slot)
                .map(|this| {
                    if self.selected {
                        this.border_color(pill.border)
                            .bg(pill.background)
                            .when_some(hover_background, |this, hover_background| {
                                this.hover(move |style| style.bg(hover_background))
                            })
                    } else {
                        this.border_color(transparent_border).when_some(
                            hover_background,
                            |this, hover_background| {
                                this.hover(move |style| style.bg(hover_background))
                            },
                        )
                    }
                });

            return self
                .div
                .h(container_height)
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
    fn island_tab_strip_height_is_forty_one_and_thirty_one_when_compact() {
        assert_eq!(
            IslandTabMetrics::for_density(UiDensity::Default).strip_height,
            px(41.)
        );
        assert_eq!(
            IslandTabMetrics::for_density(UiDensity::Comfortable).strip_height,
            px(41.)
        );
        assert_eq!(
            IslandTabMetrics::for_density(UiDensity::Compact).strip_height,
            px(31.)
        );
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
    fn island_tab_content_slots_follow_the_intellij_layout() {
        let metrics = IslandTabMetrics::for_density(UiDensity::Default);
        assert_eq!(metrics.content_start_padding, px(8.));
        assert_eq!(metrics.close_slot_size, px(16.));
        assert_eq!(metrics.slot_gap, px(2.));
        assert_eq!(metrics.content_end_padding, px(4.));
        assert_eq!(metrics.minimum_content_width, px(50.));
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
        let blended = eight_bit(island_tab_label_color(false, false, &colors));
        let expected = [0xAF, 0xB1, 0xB6];
        for (actual, expected) in blended.into_iter().zip(expected) {
            assert!(
                actual.abs_diff(expected) <= EIGHT_BIT_TOLERANCE,
                "blend {blended:?} should match {expected:?}"
            );
        }
    }

    #[test]
    fn island_tab_selected_and_hovered_labels_use_the_full_text_color() {
        let colors = ThemeColors::dark();
        assert_eq!(island_tab_label_color(true, false, &colors), colors.text);
        assert_eq!(island_tab_label_color(false, true, &colors), colors.text);
        assert_ne!(island_tab_label_color(false, false, &colors), colors.text);
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
}
