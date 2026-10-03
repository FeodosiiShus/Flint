use std::cmp::Ordering;

use gpui::{AnyElement, Hsla, IntoElement, Stateful};
use smallvec::SmallVec;
use theme::ThemeColors;

use crate::prelude::*;

const START_TAB_SLOT_SIZE: Pixels = px(12.);
const END_TAB_SLOT_SIZE: Pixels = px(14.);
const TAB_BORDER_WIDTH: Pixels = px(1.);
const ISLAND_TAB_PILL_RADIUS: Pixels = px(6.);
const ISLAND_TAB_PILL_VERTICAL_INSET: Pixels = px(4.);
const ISLAND_TAB_ACCENT_FILL_OPACITY: f32 = 0.15;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IslandTabPillColors {
    pub background: Hsla,
    pub border: Hsla,
}

pub fn island_tab_pill_colors(pane_focused: bool, colors: &ThemeColors) -> IslandTabPillColors {
    if pane_focused {
        IslandTabPillColors {
            background: colors.text_accent.opacity(ISLAND_TAB_ACCENT_FILL_OPACITY),
            border: colors.text_accent,
        }
    } else {
        IslandTabPillColors {
            background: gpui::transparent_black(),
            border: colors.border,
        }
    }
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
        let height = crate::chrome_height(region, cx);
        let button_height = crate::chrome_button_height(region, ButtonSize::Default, cx);
        if height.is_none() && button_height.is_none() {
            return DynamicSpacing::Base32.px(cx);
        }
        let height = height.unwrap_or_else(|| DynamicSpacing::Base32.px(cx));
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

        if self.islands {
            let colors = cx.theme().colors();
            let pill = island_tab_pill_colors(self.focused, colors);
            let hover_background = colors.ghost_element_hover;
            let transparent_border = colors.border_transparent;
            return self
                .div
                .h(Tab::container_height(cx))
                .py(ISLAND_TAB_PILL_VERTICAL_INSET)
                .px_px()
                .cursor_pointer()
                .child(
                    content
                        .h_full()
                        .rounded(ISLAND_TAB_PILL_RADIUS)
                        .border_1()
                        .map(|this| {
                            if self.selected {
                                this.border_color(pill.border).bg(pill.background)
                            } else {
                                this.border_color(transparent_border)
                                    .hover(move |style| style.bg(hover_background))
                            }
                        }),
                );
        }

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

    #[test]
    fn focused_pane_pill_has_accent_border_and_tinted_fill() {
        for colors in [ThemeColors::dark(), ThemeColors::light()] {
            let pill = island_tab_pill_colors(true, &colors);
            assert_eq!(pill.border, colors.text_accent);
            assert_eq!(
                (pill.background.h, pill.background.s, pill.background.l),
                (
                    colors.text_accent.h,
                    colors.text_accent.s,
                    colors.text_accent.l
                ),
                "the fill is tinted with the accent hue"
            );
            assert!(
                (pill.background.a - colors.text_accent.a * 0.15).abs() < f32::EPSILON,
                "the fill keeps 15% of the accent alpha, got {}",
                pill.background.a
            );
        }
    }

    #[test]
    fn unfocused_pane_pill_has_border_color_and_no_fill() {
        for colors in [ThemeColors::dark(), ThemeColors::light()] {
            let pill = island_tab_pill_colors(false, &colors);
            assert_eq!(pill.border, colors.border);
            assert!(pill.background.is_transparent());
            assert_ne!(
                pill.border,
                island_tab_pill_colors(true, &colors).border,
                "focus must be visible from the pill border alone"
            );
        }
    }
}
