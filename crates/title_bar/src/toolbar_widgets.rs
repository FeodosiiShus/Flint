use std::rc::Rc;

use gpui::{
    Action, AnyView, Background, ClickEvent, CursorStyle, Hsla, KeyDownEvent, MouseButton, Role,
    Toggled, hsla, linear_color_stop, linear_gradient, rgb,
};
use ui::{
    COMPONENT_CORNER_RADII, ChromeRegion, CommonAnimationExt, KeyBinding, Tooltip, prelude::*,
};

const REST: f32 = 0.;
const HOVER_DARK: f32 = 0x17 as f32 / 255.;
const HOVER_LIGHT: f32 = 0x12 as f32 / 255.;
const PRESSED_DARK: f32 = 0x29 as f32 / 255.;
const PRESSED_LIGHT: f32 = 0x20 as f32 / 255.;

pub(crate) const PROJECT_AVATAR_CHIP_DEBUG_SELECTOR: &str = "title_bar_project_avatar_chip";
pub(crate) const TOOLBAR_WIDGET_ICON_DEBUG_SELECTOR: &str = "title_bar_widget_icon";
pub(crate) const TOOLBAR_WIDGET_CHEVRON_DEBUG_SELECTOR: &str = "title_bar_widget_chevron";
const PROJECT_AVATAR_SIDE: Pixels = px(16.);
const PROJECT_AVATAR_ARC_RATIO: f32 = 0.4;
const PROJECT_AVATAR_FONT_SIZE: Pixels = px(10.);
// IntelliJ draws the initials in JetBrains Mono whatever the UI font
// (`AvatarUtils.getNewUiFont`).
const PROJECT_AVATAR_FONT_FAMILY: &str = "JetBrains Mono";
// `RecentProject.Color7` `Avatar.End`, the bottom-left colour of the avatar gradient.
const PROJECT_AVATAR_GRADIENT_END: u32 = 0xA84DE0;
const TOOLBAR_WIDGET_ICON_SIDE: Pixels = px(16.);
const TOOLBAR_WIDGET_ARROW_SIDE: Pixels = px(12.);
// The focus ring is a permanent `border_2` that is transparent until focused, so the padding
// shrinks by its width to keep IntelliJ's 10 px and 6 px insets.
const TOOLBAR_WIDGET_FOCUS_RING_WIDTH: Pixels = px(2.);
const TOOLBAR_WIDGET_PADDING_LEFT: Pixels = px(10.);
const TOOLBAR_WIDGET_PADDING_RIGHT: Pixels = px(6.);
const TOOLBAR_WIDGET_ICON_TEXT_GAP: Pixels = px(6.);
const TOOLBAR_WIDGET_CHEVRON_GAP: Pixels = px(2.);
pub const SEARCH_FIELD_DEBUG_SELECTOR: &str = "title_bar_search_field";
const TOOLBAR_SEARCH_FIELD_WIDTH: Pixels = px(260.);
const TOOLBAR_SEARCH_FIELD_PADDING: Pixels = px(8.);

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ToolbarWidgetColors {
    pub(crate) rest: Hsla,
    pub(crate) hover: Hsla,
    pub(crate) open: Hsla,
}

impl ToolbarWidgetColors {
    pub(crate) fn current(cx: &App) -> Self {
        if cx.theme().appearance().is_light() {
            Self::with_tint(0., [REST, HOVER_LIGHT, PRESSED_LIGHT])
        } else {
            Self::with_tint(1., [REST, HOVER_DARK, PRESSED_DARK])
        }
    }

    fn with_tint(lightness: f32, [rest, hover, open]: [f32; 3]) -> Self {
        Self {
            rest: hsla(0., 0., lightness, rest),
            hover: hsla(0., 0., lightness, hover),
            open: hsla(0., 0., lightness, open),
        }
    }
}

/// The colour of the widget icons and the chevron: IntelliJ recolours the `#CED0D6` stroke of
/// the expui icons to `icon-default-stroke`, which is the theme's `icon` colour.
pub(crate) fn toolbar_widget_icon_color(cx: &App) -> Color {
    Color::Custom(cx.theme().colors().icon)
}

/// IntelliJ's generated project avatar: a diagonal gradient from `Avatar.End` at the bottom-left
/// to `Avatar.Start` (`color`) at the top-right (`AvatarUtils.generateColoredAvatar`).
pub(crate) fn project_avatar_background(color: Hsla) -> Background {
    linear_gradient(
        45.,
        linear_color_stop(rgb(PROJECT_AVATAR_GRADIENT_END), 0.),
        linear_color_stop(color, 1.),
    )
}

#[derive(IntoElement)]
pub(crate) struct ToolbarWidget {
    id: ElementId,
    icon: Option<Icon>,
    avatar: Option<(SharedString, Hsla)>,
    label: SharedString,
    upstream: Option<UpstreamTracking>,
    loading: bool,
    toggled: bool,
    tab_index: Option<isize>,
    cursor_style: CursorStyle,
    tooltip: Option<Box<dyn Fn(&mut Window, &mut App) -> AnyView>>,
    on_click: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
}

impl ToolbarWidget {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        icon: Icon,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            icon: Some(icon),
            ..Self::text(id, label)
        }
    }

    /// A widget with only its text and the chevron, like IntelliJ's VCS widget for a project
    /// without version control.
    pub(crate) fn text(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: None,
            avatar: None,
            label: label.into(),
            upstream: None,
            loading: false,
            toggled: false,
            tab_index: None,
            cursor_style: CursorStyle::PointingHand,
            tooltip: None,
            on_click: None,
        }
    }

    pub(crate) fn avatar(id: impl Into<ElementId>, initials: SharedString, color: Hsla) -> Self {
        Self {
            avatar: Some((initials, color)),
            ..Self::text(id, SharedString::new_static(""))
        }
    }

    pub(crate) fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = label.into();
        self
    }

    pub(crate) fn upstream(mut self, upstream: Option<UpstreamTracking>) -> Self {
        self.upstream = upstream;
        self
    }

    pub(crate) fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub(crate) fn tab_index(mut self, tab_index: isize) -> Self {
        self.tab_index = Some(tab_index);
        self
    }

    pub(crate) fn tooltip(
        mut self,
        tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        self.tooltip = Some(Box::new(tooltip));
        self
    }
}

impl Clickable for ToolbarWidget {
    fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    fn cursor_style(mut self, cursor_style: CursorStyle) -> Self {
        self.cursor_style = cursor_style;
        self
    }
}

impl Toggleable for ToolbarWidget {
    fn toggle_state(mut self, selected: bool) -> Self {
        self.toggled = selected;
        self
    }
}

impl RenderOnce for ToolbarWidget {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = ToolbarWidgetColors::current(cx);
        let toggled = self.toggled;
        let focus_color = cx.theme().colors().border_focused;
        let icon_color = toolbar_widget_icon_color(cx);
        let rem_size = window.rem_size();
        // The sizes are logical pixels like IntelliJ's, scaled only by `title_bar.icon_size`,
        // never by `ui_font_size`.
        let icon_scale = ui::chrome_icon_scale(ChromeRegion::TitleBar, cx).unwrap_or(1.);
        let icon_size = ui::exact_icon_size(TOOLBAR_WIDGET_ICON_SIDE * icon_scale, rem_size);
        let arrow_size = ui::exact_icon_size(TOOLBAR_WIDGET_ARROW_SIDE * icon_scale, rem_size);
        let avatar_side = PROJECT_AVATAR_SIDE * icon_scale;
        let avatar_font_size = PROJECT_AVATAR_FONT_SIZE * icon_scale;
        let leading_icon = if self.loading {
            Some(
                Icon::new(IconName::LoadCircle)
                    .size(icon_size)
                    .color(Color::Muted)
                    .with_keyed_rotate_animation((self.id.clone(), "loading"), 2)
                    .into_any_element(),
            )
        } else {
            self.icon
                .map(|icon| icon.size(icon_size).into_any_element())
        };
        let leading_icon = leading_icon.map(|icon| {
            div()
                .debug_selector(|| TOOLBAR_WIDGET_ICON_DEBUG_SELECTOR.into())
                .flex_none()
                .child(icon)
                .into_any_element()
        });
        let avatar = self.avatar.map(|(initials, color)| {
            div()
                .debug_selector(|| PROJECT_AVATAR_CHIP_DEBUG_SELECTOR.into())
                .flex_none()
                .size(avatar_side)
                .rounded(avatar_side * (PROJECT_AVATAR_ARC_RATIO / 2.))
                .bg(project_avatar_background(color))
                .font_family(PROJECT_AVATAR_FONT_FAMILY)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Label::new(initials)
                        .size(LabelSize::Custom(rems(
                            f32::from(avatar_font_size) / f32::from(rem_size),
                        )))
                        .weight(gpui::FontWeight::SEMIBOLD)
                        .color(Color::Custom(gpui::white())),
                )
                .into_any_element()
        });
        let debug_label = self.label.clone();

        h_flex()
            .id(self.id)
            .debug_selector(move || format!("TOOLBAR_WIDGET-{debug_label}"))
            .role(Role::Button)
            .aria_label(self.label.clone())
            .aria_toggled(if toggled {
                Toggled::True
            } else {
                Toggled::False
            })
            .when_some(self.tab_index, |this, tab_index| this.tab_index(tab_index))
            .font_ui(cx)
            .flex_none()
            .h(ui::utils::title_bar_widget_height(cx))
            .border_2()
            .border_color(gpui::transparent_black())
            .pl(TOOLBAR_WIDGET_PADDING_LEFT - TOOLBAR_WIDGET_FOCUS_RING_WIDTH)
            .pr(TOOLBAR_WIDGET_PADDING_RIGHT - TOOLBAR_WIDGET_FOCUS_RING_WIDTH)
            .gap(TOOLBAR_WIDGET_ICON_TEXT_GAP)
            .rounded(COMPONENT_CORNER_RADII.toolbar_widget)
            // IntelliJ's combo paints its hover colour while the popup is shown and has no
            // pressed state (`ToolbarComboButtonUI.paintBackground`).
            .bg(if toggled { colors.hover } else { colors.rest })
            .cursor(self.cursor_style)
            .when(!toggled, |this| {
                this.hover(move |style| style.bg(colors.hover))
            })
            .focus_visible(move |style| style.border_color(focus_color))
            .when_some(self.on_click, |this, on_click| {
                let open_on_down_key = on_click.clone();
                this.on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(move |event, window, cx| {
                        cx.stop_propagation();
                        on_click(event, window, cx)
                    })
                    // GPUI already turns Enter and Space into a click; IntelliJ also opens the
                    // popup on Down (`ToolbarComboButtonUI.installKeyboardActions`).
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "down" && !event.keystroke.modifiers.modified() {
                            cx.stop_propagation();
                            open_on_down_key(&ClickEvent::default(), window, cx)
                        }
                    })
            })
            .when_some(self.tooltip.filter(|_| !toggled), |this, tooltip| {
                this.tooltip(move |window, cx| tooltip(window, cx))
            })
            .when_some(leading_icon.or(avatar), |this, leading| this.child(leading))
            .when(!self.label.is_empty(), |this| {
                this.child(
                    h_flex()
                        .gap(TOOLBAR_WIDGET_CHEVRON_GAP)
                        .child(Label::new(self.label))
                        .when_some(self.upstream, |this, upstream| {
                            this.child(upstream_tracking_icons(upstream, arrow_size))
                        })
                        .child(
                            div()
                                .debug_selector(|| TOOLBAR_WIDGET_CHEVRON_DEBUG_SELECTOR.into())
                                .flex_none()
                                .child(
                                    Icon::new(IconName::ChevronDown)
                                        .size(icon_size)
                                        .color(icon_color),
                                ),
                        ),
                )
            })
    }
}

/// A field-shaped button that opens Search Everywhere, like the main toolbar search field of
/// `SearchEverywhereRiderMainToolbarAction`: the magnifier, the muted text and the shortcut.
#[derive(IntoElement)]
pub(crate) struct ToolbarSearchField {
    id: ElementId,
    placeholder: SharedString,
    action: Box<dyn Action>,
}

impl ToolbarSearchField {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        placeholder: impl Into<SharedString>,
        action: Box<dyn Action>,
    ) -> Self {
        Self {
            id: id.into(),
            placeholder: placeholder.into(),
            action,
        }
    }
}

impl RenderOnce for ToolbarSearchField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = ToolbarWidgetColors::current(cx);
        let focus_color = cx.theme().colors().border_focused;
        let icon_scale = ui::chrome_icon_scale(ChromeRegion::TitleBar, cx).unwrap_or(1.);
        let icon_size =
            ui::exact_icon_size(TOOLBAR_WIDGET_ICON_SIDE * icon_scale, window.rem_size());
        let tooltip = Tooltip::for_action_title(self.placeholder.clone(), self.action.as_ref());
        let shortcut = KeyBinding::for_action(self.action.as_ref(), cx);
        let action = self.action;

        h_flex()
            .id(self.id)
            .debug_selector(|| SEARCH_FIELD_DEBUG_SELECTOR.into())
            .role(Role::Button)
            .aria_label(self.placeholder.clone())
            .tab_index(0)
            .font_ui(cx)
            .flex_none()
            .w(TOOLBAR_SEARCH_FIELD_WIDTH * icon_scale)
            .h(ui::utils::title_bar_widget_height(cx))
            .px(TOOLBAR_SEARCH_FIELD_PADDING)
            .gap(TOOLBAR_WIDGET_ICON_TEXT_GAP)
            .rounded(COMPONENT_CORNER_RADII.toolbar_widget)
            .border_1()
            .border_color(colors.open)
            .bg(colors.hover)
            .cursor(CursorStyle::IBeam)
            .hover(move |style| style.bg(colors.open))
            .focus_visible(move |style| style.border_color(focus_color))
            // Keep the keyboard focus where it is, so Search Everywhere opens for the focused
            // editor just like Double Shift does.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                window.dispatch_action(action.boxed_clone(), cx)
            })
            .tooltip(tooltip)
            .child(
                div().flex_none().child(
                    Icon::new(IconName::MagnifyingGlass)
                        .size(icon_size)
                        .color(toolbar_widget_icon_color(cx)),
                ),
            )
            .child(
                div().flex_1().min_w_0().child(
                    Label::new(self.placeholder)
                        .color(Color::Placeholder)
                        .truncate(),
                ),
            )
            .child(div().flex_none().child(shortcut.color(Color::Placeholder)))
    }
}

/// The incoming and outgoing commit arrows of IntelliJ's VCS widget
/// (`GitToolbarWidgetAction.getInAndOutIcons`): a 12x12 blue incoming arrow and a green
/// outgoing arrow, drawn edge to edge as one `RowIcon` for every direction that has commits,
/// incoming first, 6 px after the branch name (`iconTextGap`) and 2 px before the chevron
/// (`BEFORE_CHEVRON_GAP`).
fn upstream_tracking_icons(upstream: UpstreamTracking, arrow_size: IconSize) -> impl IntoElement {
    h_flex()
        .ml(px(4.))
        .when(upstream.incoming, |this| {
            this.child(
                div()
                    .debug_selector(|| "title_bar_incoming_commits".into())
                    .child(
                        Icon::new(IconName::IncomingCommits)
                            .size(arrow_size)
                            .color(Color::Info),
                    ),
            )
        })
        .when(upstream.outgoing, |this| {
            this.child(
                div()
                    .debug_selector(|| "title_bar_outgoing_commits".into())
                    .child(
                        Icon::new(IconName::OutgoingCommits)
                            .size(arrow_size)
                            .color(Color::Success),
                    ),
            )
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UpstreamTracking {
    pub(crate) incoming: bool,
    pub(crate) outgoing: bool,
}

/// IntelliJ's `AvatarUtils.initials`: the camel-case initials when they give two letters,
/// otherwise the first letters of the first and last words.
pub(crate) fn avatar_initials(text: &str) -> String {
    let filtered: String = text
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    let filtered = filtered.trim();

    let camel_case_initials: String = filtered
        .trim_start_matches(|character: char| !character.is_alphabetic())
        .chars()
        .take_while(|character| character.is_alphanumeric())
        .enumerate()
        .filter(|(index, character)| *index == 0 || character.is_uppercase())
        .map(|(_, character)| character)
        .take(2)
        .collect::<String>()
        .to_uppercase();
    if camel_case_initials.chars().count() == 2 {
        return camel_case_initials;
    }

    for delimiters in [
        &[' '][..],
        &[','][..],
        &['-'][..],
        &['_'][..],
        &['.'][..],
        &['`', '\'', '"'][..],
    ] {
        let words: Vec<String> = filtered
            .split(delimiters)
            .map(|word| {
                word.chars()
                    .filter(|character| character.is_alphanumeric())
                    .collect::<String>()
            })
            .filter(|word| !word.is_empty())
            .collect();
        if let (Some(first_word), Some(last_word)) = (words.first(), words.last())
            && words.len() >= 2
        {
            return [first_word.chars().next(), last_word.chars().next()]
                .into_iter()
                .flatten()
                .collect::<String>()
                .to_uppercase();
        }
    }

    camel_case_initials
}

/// The arrows shown for a branch that is behind or ahead of its upstream; only the non-zero
/// directions are drawn.
pub(crate) fn upstream_tracking(ahead: u32, behind: u32) -> Option<UpstreamTracking> {
    match (behind > 0, ahead > 0) {
        (false, false) => None,
        (incoming, outgoing) => Some(UpstreamTracking { incoming, outgoing }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};
    use settings::SettingsStore;
    use std::cell::Cell;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
    }

    #[test]
    fn test_upstream_tracking_is_hidden_when_in_sync() {
        assert_eq!(upstream_tracking(0, 0), None);
    }

    #[test]
    fn test_upstream_tracking_shows_only_nonzero_directions() {
        assert_eq!(
            upstream_tracking(0, 2),
            Some(UpstreamTracking {
                incoming: true,
                outgoing: false
            })
        );
        assert_eq!(
            upstream_tracking(3, 0),
            Some(UpstreamTracking {
                incoming: false,
                outgoing: true
            })
        );
    }

    #[test]
    fn test_upstream_tracking_shows_both_directions_together() {
        assert_eq!(
            upstream_tracking(3, 2),
            Some(UpstreamTracking {
                incoming: true,
                outgoing: true
            })
        );
        assert_eq!(
            upstream_tracking(u32::MAX, 1),
            Some(UpstreamTracking {
                incoming: true,
                outgoing: true
            })
        );
    }

    struct UpstreamWidgetView {
        upstream: Option<UpstreamTracking>,
    }

    impl Render for UpstreamWidgetView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            ToolbarWidget::new(
                "upstream_test_widget",
                Icon::new(IconName::GitBranch).color(Color::Muted),
                "main",
            )
            .upstream(self.upstream)
        }
    }

    #[gpui::test]
    async fn test_branch_widget_draws_the_incoming_then_outgoing_arrows(cx: &mut TestAppContext) {
        init_test(cx);
        let (_view, cx) = cx.add_window_view(|_window, _cx| UpstreamWidgetView {
            upstream: Some(UpstreamTracking {
                incoming: true,
                outgoing: true,
            }),
        });
        cx.run_until_parked();

        let incoming = cx
            .debug_bounds("title_bar_incoming_commits")
            .expect("an incoming branch draws the blue arrow");
        let outgoing = cx
            .debug_bounds("title_bar_outgoing_commits")
            .expect("an outgoing branch draws the green arrow");
        assert!(
            incoming.right() <= outgoing.left(),
            "the incoming arrow is drawn first, like the RowIcon of the IntelliJ widget: {incoming:?}, {outgoing:?}"
        );
        assert_eq!(
            incoming.size, outgoing.size,
            "both arrows are drawn at the widget icon size"
        );
    }

    #[gpui::test]
    async fn test_branch_widget_draws_only_the_nonzero_direction(cx: &mut TestAppContext) {
        init_test(cx);
        let (_view, cx) = cx.add_window_view(|_window, _cx| UpstreamWidgetView {
            upstream: Some(UpstreamTracking {
                incoming: false,
                outgoing: true,
            }),
        });
        cx.run_until_parked();

        assert_eq!(cx.debug_bounds("title_bar_incoming_commits"), None);
        assert!(cx.debug_bounds("title_bar_outgoing_commits").is_some());
    }

    struct KeyboardWidgetView {
        clicks: Rc<Cell<usize>>,
    }

    impl Render for KeyboardWidgetView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let clicks = self.clicks.clone();
            ToolbarWidget::new(
                "keyboard_test_widget",
                Icon::new(IconName::GitBranch),
                "main",
            )
            .tab_index(0)
            .on_click(move |_, _, _| clicks.set(clicks.get() + 1))
        }
    }

    #[gpui::test]
    async fn test_focused_widget_opens_on_down_and_draws_a_focus_ring(cx: &mut TestAppContext) {
        init_test(cx);
        let clicks = Rc::new(Cell::new(0));
        let (_view, cx) = cx.add_window_view({
            let clicks = clicks.clone();
            move |_window, _cx| KeyboardWidgetView { clicks }
        });
        cx.run_until_parked();
        let focus_color = cx.update(|_, cx| cx.theme().colors().border_focused);
        let has_focus_ring = |cx: &mut VisualTestContext| {
            cx.update(|window, _| window.painted_quads())
                .iter()
                .any(|quad| quad.border_color == focus_color)
        };

        cx.update(|window, cx| window.focus_next(cx));
        cx.run_until_parked();
        assert!(
            !has_focus_ring(cx),
            "a widget focused without the keyboard draws no ring"
        );

        cx.simulate_keystrokes("down");
        cx.run_until_parked();
        assert_eq!(
            clicks.get(),
            1,
            "Down opens the widget popup, like ToolbarComboButtonUI"
        );
        assert!(
            has_focus_ring(cx),
            "keyboard focus draws IntelliJ's focus ring around the pill"
        );

        cx.simulate_keystrokes("shift-down");
        assert_eq!(
            clicks.get(),
            1,
            "a modified Down is left to the key bindings"
        );
    }

    #[gpui::test]
    fn test_widget_icons_use_the_theme_icon_colour(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            let icon_color = toolbar_widget_icon_color(cx);
            assert_ne!(
                icon_color,
                Color::Muted,
                "IntelliJ recolours the widget icons to icon-default-stroke, not the muted text colour"
            );
            assert_eq!(icon_color.color(cx), cx.theme().colors().icon);
        });
    }

    #[gpui::test]
    async fn test_branch_widget_draws_no_arrows_in_sync(cx: &mut TestAppContext) {
        init_test(cx);
        let (_view, cx) = cx.add_window_view(|_window, _cx| UpstreamWidgetView { upstream: None });
        cx.run_until_parked();

        assert_eq!(cx.debug_bounds("title_bar_incoming_commits"), None);
        assert_eq!(cx.debug_bounds("title_bar_outgoing_commits"), None);
    }
}
