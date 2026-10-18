use gpui::{
    AnyElement, AnyView, ClickEvent, ElementId, Hsla, IntoElement, KeybindingKeystroke, Keystroke,
    Role, Styled, Toggled, Window, black, div, hsla, prelude::*, transparent_black, white,
};
use std::{rc::Rc, sync::Arc};

use crate::utils::is_light;
use crate::{COMPONENT_CORNER_RADII, Color, Icon, IconName, ToggleState, Tooltip};
use crate::{ElevationIndex, KeyBinding, prelude::*};

const CHECKBOX_BOX_SIZE: Pixels = px(16.);
const CHECKBOX_FOCUS_RING_WIDTH: Pixels = px(2.);
const CHECKBOX_LABEL_GAP: Pixels = px(4.);
const CHECKBOX_PLACEHOLDER_DOT_SIZE: Pixels = px(4.);
const CHECKBOX_MARK_SIZE: f32 = 14.;
const SWITCH_WIDTH: Pixels = px(32.);
const SWITCH_HEIGHT: Pixels = px(22.);
const SWITCH_FOCUS_RING_WIDTH: Pixels = px(2.);
const SWITCH_TRACK_INSET: Pixels = px(3.);
const SWITCH_TRACK_WIDTH: Pixels = px(26.);
const SWITCH_TRACK_HEIGHT: Pixels = px(16.);
const SWITCH_ON_NOTCH_DIAMETER: Pixels = px(10.);
const SWITCH_ON_NOTCH_LEFT: Pixels = px(16.);
const SWITCH_ON_NOTCH_TOP: Pixels = px(6.);
const SWITCH_OFF_NOTCH_DIAMETER: Pixels = px(8.);
const SWITCH_OFF_NOTCH_LEFT: Pixels = px(7.);
const SWITCH_OFF_NOTCH_TOP: Pixels = px(7.);
const SWITCH_OFF_TRACK_ALPHA_DARK: f32 = 59. / 255.;
const SWITCH_OFF_TRACK_ALPHA_LIGHT: f32 = 48. / 255.;

// TODO: Checkbox, CheckboxWithLabel, and Switch could all be
// restructured to use a ToggleLike, similar to Button/Buttonlike, Label/Labellike

/// Creates a new checkbox.
pub fn checkbox(id: impl Into<ElementId>, toggle_state: ToggleState) -> Checkbox {
    Checkbox::new(id, toggle_state)
}

/// Creates a new switch.
pub fn switch(id: impl Into<ElementId>, toggle_state: ToggleState) -> Switch {
    Switch::new(id, toggle_state)
}

/// The visual style of a toggle.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum ToggleStyle {
    /// Toggle has a transparent background
    #[default]
    Ghost,
    /// Toggle has a filled background based on the
    /// elevation index of the parent container
    ElevationBased(ElevationIndex),
    /// A custom style using a color to tint the toggle
    Custom(Hsla),
}

/// # Checkbox
///
/// Checkboxes are used for multiple choices, not for mutually exclusive choices.
/// Each checkbox works independently from other checkboxes in the list,
/// therefore checking an additional box does not affect any other selections.
#[derive(IntoElement, RegisterComponent)]
pub struct Checkbox {
    id: ElementId,
    toggle_state: ToggleState,
    style: ToggleStyle,
    disabled: bool,
    placeholder: bool,
    filled: bool,
    visualization: bool,
    label: Option<SharedString>,
    label_size: LabelSize,
    label_color: Color,
    tooltip: Option<Box<dyn Fn(&mut Window, &mut App) -> AnyView>>,
    on_click: Option<Box<dyn Fn(&ToggleState, &ClickEvent, &mut Window, &mut App) + 'static>>,
    tab_index: Option<isize>,
}

impl Checkbox {
    /// Creates a new [`Checkbox`].
    pub fn new(id: impl Into<ElementId>, checked: ToggleState) -> Self {
        Self {
            id: id.into(),
            toggle_state: checked,
            style: ToggleStyle::default(),
            disabled: false,
            placeholder: false,
            filled: false,
            visualization: false,
            label: None,
            label_size: LabelSize::Default,
            label_color: Color::Muted,
            tooltip: None,
            on_click: None,
            tab_index: None,
        }
    }

    /// Sets the disabled state of the [`Checkbox`].
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Sets the disabled state of the [`Checkbox`].
    pub fn placeholder(mut self, placeholder: bool) -> Self {
        self.placeholder = placeholder;
        self
    }

    /// Binds a handler to the [`Checkbox`] that will be called when clicked.
    pub fn on_click(
        mut self,
        handler: impl Fn(&ToggleState, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(move |state, _, window, cx| {
            handler(state, window, cx)
        }));
        self
    }

    pub fn on_click_ext(
        mut self,
        handler: impl Fn(&ToggleState, &ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Sets the `fill` setting of the checkbox, indicating whether it should be filled.
    pub fn fill(mut self) -> Self {
        self.filled = true;
        self
    }

    /// Makes the checkbox look enabled but without pointer cursor and hover styles.
    /// Primarily used for uninteractive markdown previews.
    pub fn visualization_only(mut self, visualization: bool) -> Self {
        self.visualization = visualization;
        self
    }

    /// Sets the style of the checkbox using the specified [`ToggleStyle`].
    pub fn style(mut self, style: ToggleStyle) -> Self {
        self.style = style;
        self
    }

    /// Match the style of the checkbox to the current elevation using [`ToggleStyle::ElevationBased`].
    pub fn elevation(mut self, elevation: ElevationIndex) -> Self {
        self.style = ToggleStyle::ElevationBased(elevation);
        self
    }

    /// Sets the tooltip for the checkbox.
    pub fn tooltip(mut self, tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.tooltip = Some(Box::new(tooltip));
        self
    }

    /// Set the label for the checkbox.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn label_size(mut self, size: LabelSize) -> Self {
        self.label_size = size;
        self
    }

    pub fn label_color(mut self, color: Color) -> Self {
        self.label_color = color;
        self
    }

    pub fn tab_index(mut self, tab_index: impl Into<isize>) -> Self {
        self.tab_index = Some(tab_index.into());
        self
    }
}

struct CheckboxColors {
    background: Hsla,
    border: Hsla,
    mark: Color,
}

impl Checkbox {
    fn unchecked_background(&self, cx: &App) -> Hsla {
        match (self.style.clone(), self.filled) {
            (ToggleStyle::Ghost, true) => cx.theme().colors().element_background,
            (ToggleStyle::ElevationBased(elevation), true) => elevation.darker_bg(cx),
            (ToggleStyle::Custom(color), true) => color.opacity(0.2),
            (_, false) => cx.theme().colors().editor_background,
        }
    }

    fn accent_color(&self, cx: &App) -> Hsla {
        match self.style.clone() {
            ToggleStyle::Custom(color) => color,
            ToggleStyle::Ghost | ToggleStyle::ElevationBased(_) => cx.theme().colors().text_accent,
        }
    }

    fn is_checked(&self) -> bool {
        match self.toggle_state {
            ToggleState::Selected => !self.placeholder,
            ToggleState::Indeterminate => true,
            ToggleState::Unselected => false,
        }
    }

    fn colors(&self, cx: &App) -> CheckboxColors {
        let theme_colors = cx.theme().colors();
        if self.disabled {
            CheckboxColors {
                background: theme_colors.element_disabled,
                border: theme_colors.border_disabled,
                mark: Color::Disabled,
            }
        } else if self.is_checked() {
            let accent = self.accent_color(cx);
            CheckboxColors {
                background: accent,
                border: accent,
                mark: Color::Custom(white()),
            }
        } else {
            CheckboxColors {
                background: self.unchecked_background(cx),
                border: theme_colors.border,
                mark: Color::Default,
            }
        }
    }

    pub fn container_size() -> Pixels {
        px(20.0)
    }
}

impl RenderOnce for Checkbox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = self.colors(cx);
        let is_interactive = !self.disabled && !self.visualization;
        let next_state = self.toggle_state.inverse();
        let placeholder_color = if self.disabled {
            Color::Disabled
        } else {
            Color::Selected
        }
        .color(cx)
        .alpha(0.5);
        let focus_ring_color = cx.theme().colors().border_focused;

        let mark_size = IconSize::Custom(rems(CHECKBOX_MARK_SIZE / f32::from(window.rem_size())));
        let mark = match self.toggle_state {
            ToggleState::Selected if !self.placeholder => Some(IconName::CheckBoxMark),
            ToggleState::Indeterminate => Some(IconName::CheckBoxIndeterminateMark),
            ToggleState::Selected | ToggleState::Unselected => None,
        }
        .map(|icon_name| Icon::new(icon_name).size(mark_size).color(colors.mark));

        let on_click: Option<Rc<dyn Fn(&ToggleState, &ClickEvent, &mut Window, &mut App)>> =
            self.on_click.filter(|_| !self.disabled).map(Rc::from);

        let checkbox = h_flex()
            .id(self.id.clone())
            .flex_none()
            .size(Self::container_size())
            .justify_center()
            .items_center()
            .rounded(COMPONENT_CORNER_RADII.checkbox + CHECKBOX_FOCUS_RING_WIDTH)
            .border(CHECKBOX_FOCUS_RING_WIDTH)
            .border_color(cx.theme().colors().border_transparent)
            .when_some(
                self.tab_index.filter(|_| is_interactive),
                |this, tab_index| {
                    this.tab_index(tab_index)
                        .focus_visible(move |mut style| {
                            style.border_color = Some(focus_ring_color);
                            style
                        })
                        .when_some(on_click.clone(), |this, on_click| {
                            this.on_click(move |click, window, cx| {
                                cx.stop_propagation();
                                on_click(&next_state, click, window, cx)
                            })
                        })
                },
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .justify_center()
                    .items_center()
                    .size(CHECKBOX_BOX_SIZE)
                    .rounded(COMPONENT_CORNER_RADII.checkbox)
                    .bg(colors.background)
                    .border_1()
                    .border_color(colors.border)
                    .when(self.placeholder, |this| {
                        this.child(
                            div()
                                .flex_none()
                                .rounded_full()
                                .bg(placeholder_color)
                                .size(CHECKBOX_PLACEHOLDER_DOT_SIZE),
                        )
                    })
                    .children(mark),
            );

        h_flex()
            .id(self.id)
            .map(|this| {
                if self.disabled {
                    this.cursor_not_allowed()
                } else if self.visualization {
                    this.cursor_default()
                } else {
                    this.cursor_pointer()
                }
            })
            .gap(CHECKBOX_LABEL_GAP)
            .child(checkbox)
            .when_some(self.label, |this, label| {
                this.child(
                    Label::new(label)
                        .color(self.label_color)
                        .size(self.label_size),
                )
            })
            .when_some(self.tooltip, |this, tooltip| {
                this.tooltip(move |window, cx| tooltip(window, cx))
            })
            .when_some(on_click, |this, on_click| {
                this.on_click(move |click, window, cx| on_click(&next_state, click, window, cx))
            })
    }
}

/// Defines the color for a switch component.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Default)]
pub enum SwitchColor {
    #[default]
    Accent,
    Custom(Hsla),
}

impl SwitchColor {
    fn track_color(&self, cx: &App) -> Hsla {
        match self {
            SwitchColor::Accent => cx.theme().colors().text_accent,
            SwitchColor::Custom(color) => *color,
        }
    }
}

impl From<SwitchColor> for Color {
    fn from(color: SwitchColor) -> Self {
        match color {
            SwitchColor::Accent => Color::Accent,
            SwitchColor::Custom(_) => Color::Default,
        }
    }
}

/// Defines the color for a switch component.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Default)]
pub enum SwitchLabelPosition {
    Start,
    #[default]
    End,
}

/// # Switch
///
/// Switches are used to represent opposite states, such as enabled or disabled.
#[derive(IntoElement, RegisterComponent)]
pub struct Switch {
    id: ElementId,
    toggle_state: ToggleState,
    disabled: bool,
    on_click: Option<Rc<dyn Fn(&ToggleState, &mut Window, &mut App) + 'static>>,
    label: Option<SharedString>,
    label_position: Option<SwitchLabelPosition>,
    label_size: LabelSize,
    label_color: Color,
    full_width: bool,
    key_binding: Option<KeyBinding>,
    color: SwitchColor,
    tab_index: Option<isize>,
    aria_label: Option<SharedString>,
    aria_description: Option<SharedString>,
}

impl Switch {
    /// Creates a new [`Switch`].
    pub fn new(id: impl Into<ElementId>, state: ToggleState) -> Self {
        Self {
            id: id.into(),
            toggle_state: state,
            disabled: false,
            on_click: None,
            label: None,
            label_position: None,
            label_size: LabelSize::Small,
            label_color: Color::Default,
            full_width: false,
            key_binding: None,
            color: SwitchColor::default(),
            tab_index: None,
            aria_label: None,
            aria_description: None,
        }
    }

    /// Sets the color of the switch using the specified [`SwitchColor`].
    pub fn color(mut self, color: SwitchColor) -> Self {
        self.color = color;
        self
    }

    /// Sets the disabled state of the [`Switch`].
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Binds a handler to the [`Switch`] that will be called when clicked.
    pub fn on_click(
        mut self,
        handler: impl Fn(&ToggleState, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    /// Sets the label of the [`Switch`].
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn label_position(
        mut self,
        label_position: impl Into<Option<SwitchLabelPosition>>,
    ) -> Self {
        self.label_position = label_position.into();
        self
    }

    pub fn label_size(mut self, size: LabelSize) -> Self {
        self.label_size = size;
        self
    }

    pub fn label_color(mut self, color: Color) -> Self {
        self.label_color = color;
        self
    }

    pub fn full_width(mut self, full_width: bool) -> Self {
        self.full_width = full_width;
        self
    }

    /// Display the keybinding that triggers the switch action.
    pub fn key_binding(mut self, key_binding: impl Into<Option<KeyBinding>>) -> Self {
        self.key_binding = key_binding.into();
        self
    }

    pub fn tab_index(mut self, tab_index: impl Into<isize>) -> Self {
        self.tab_index = Some(tab_index.into());
        self
    }

    /// Sets the label announced by assistive technology.
    /// Defaults to the switch's visible label, if any.
    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());
        self
    }

    /// Sets the supplementary description announced by assistive technology
    /// after the switch's name, role, and state.
    pub fn aria_description(mut self, description: impl Into<SharedString>) -> Self {
        self.aria_description = Some(description.into());
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_on = self.toggle_state == ToggleState::Selected;
        let is_light_theme = is_light(cx);
        let theme_colors = cx.theme().colors();

        let (track_background, track_border, notch_color) = if self.disabled {
            (
                transparent_black(),
                theme_colors.border_disabled,
                theme_colors.icon_disabled,
            )
        } else if is_on {
            let on_color = self.color.track_color(cx);
            (on_color, on_color, white())
        } else if is_light_theme {
            (
                black().opacity(SWITCH_OFF_TRACK_ALPHA_LIGHT),
                transparent_black(),
                white(),
            )
        } else {
            (
                white().opacity(SWITCH_OFF_TRACK_ALPHA_DARK),
                transparent_black(),
                theme_colors.text,
            )
        };
        let focus_ring_color = theme_colors.border_focused;

        let label = self.label;
        let aria_label = self.aria_label.or_else(|| label.clone());
        let aria_description = self.aria_description;
        let aria_keyshortcuts = self
            .key_binding
            .as_ref()
            .and_then(|key_binding| key_binding.keyboard_shortcut_text(window, cx));

        let track = div()
            .absolute()
            .left(SWITCH_TRACK_INSET - SWITCH_FOCUS_RING_WIDTH)
            .top(SWITCH_TRACK_INSET - SWITCH_FOCUS_RING_WIDTH)
            .w(SWITCH_TRACK_WIDTH)
            .h(SWITCH_TRACK_HEIGHT)
            .rounded_full()
            .bg(track_background)
            .border_1()
            .border_color(track_border);

        let notch = if is_on {
            div()
                .absolute()
                .left(SWITCH_ON_NOTCH_LEFT - SWITCH_FOCUS_RING_WIDTH)
                .top(SWITCH_ON_NOTCH_TOP - SWITCH_FOCUS_RING_WIDTH)
                .size(SWITCH_ON_NOTCH_DIAMETER)
                .rounded_full()
                .bg(notch_color)
        } else {
            div()
                .absolute()
                .left(SWITCH_OFF_NOTCH_LEFT - SWITCH_FOCUS_RING_WIDTH)
                .top(SWITCH_OFF_NOTCH_TOP - SWITCH_FOCUS_RING_WIDTH)
                .size(SWITCH_OFF_NOTCH_DIAMETER)
                .rounded_full()
                .border_2()
                .border_color(notch_color)
        };

        let switch = div()
            .id((self.id.clone(), "switch"))
            .role(Role::Switch)
            .when_some(aria_label, |this, label| this.aria_label(label))
            .when_some(aria_keyshortcuts, |this, keyshortcuts| {
                this.aria_keyshortcuts(keyshortcuts)
            })
            .when_some(aria_description, |this, description| {
                this.aria_description(description)
            })
            .aria_toggled(match self.toggle_state {
                ToggleState::Selected => Toggled::True,
                ToggleState::Indeterminate => Toggled::Mixed,
                ToggleState::Unselected => Toggled::False,
            })
            .flex_none()
            .w(SWITCH_WIDTH)
            .h(SWITCH_HEIGHT)
            .border(SWITCH_FOCUS_RING_WIDTH)
            .border_color(transparent_black())
            .rounded(SWITCH_HEIGHT * 0.5)
            .when_some(
                self.tab_index.filter(|_| !self.disabled),
                |this, tab_index| {
                    this.tab_index(tab_index)
                        .focus_visible(move |mut style| {
                            style.border_color = Some(focus_ring_color);
                            style
                        })
                        .when_some(self.on_click.clone(), |this, on_click| {
                            this.on_click(move |_, window, cx| {
                                on_click(&self.toggle_state.inverse(), window, cx)
                            })
                        })
                },
            )
            .child(div().relative().size_full().child(track).child(notch));

        h_flex()
            .id(self.id)
            .cursor_pointer()
            .gap(DynamicSpacing::Base06.rems(cx))
            .when(self.full_width, |this| this.w_full().justify_between())
            .when(
                self.label_position == Some(SwitchLabelPosition::Start),
                |this| {
                    this.when_some(label.clone(), |this, label| {
                        this.child(
                            Label::new(label)
                                .size(self.label_size)
                                .color(self.label_color),
                        )
                    })
                },
            )
            .child(switch)
            .when(
                self.label_position == Some(SwitchLabelPosition::End),
                |this| {
                    this.when_some(label, |this, label| {
                        this.child(
                            Label::new(label)
                                .size(self.label_size)
                                .color(self.label_color),
                        )
                    })
                },
            )
            .children(self.key_binding)
            .when_some(
                self.on_click.filter(|_| !self.disabled),
                |this, on_click| {
                    this.on_click(move |_, window, cx| {
                        on_click(&self.toggle_state.inverse(), window, cx)
                    })
                },
            )
    }
}

/// # SwitchField
///
/// A field component that combines a label, description, and switch into one reusable component.
///
/// # Examples
///
/// ```
/// use ui::prelude::*;
/// use ui::{SwitchField, ToggleState};
///
/// let switch_field = SwitchField::new(
///     "feature-toggle",
///     Some("Enable feature"),
///     Some("This feature adds new functionality to the app.".into()),
///     ToggleState::Unselected,
///     |state, window, cx| {
///         // Logic here
///     }
/// );
/// ```
#[derive(IntoElement, RegisterComponent)]
pub struct SwitchField {
    id: ElementId,
    label: Option<SharedString>,
    description: Option<SharedString>,
    toggle_state: ToggleState,
    on_click: Arc<dyn Fn(&ToggleState, &mut Window, &mut App) + 'static>,
    disabled: bool,
    color: SwitchColor,
    tooltip: Option<Rc<dyn Fn(&mut Window, &mut App) -> AnyView>>,
    tab_index: Option<isize>,
}

impl SwitchField {
    pub fn new(
        id: impl Into<ElementId>,
        label: Option<impl Into<SharedString>>,
        description: Option<SharedString>,
        toggle_state: impl Into<ToggleState>,
        on_click: impl Fn(&ToggleState, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.map(Into::into),
            description,
            toggle_state: toggle_state.into(),
            on_click: Arc::new(on_click),
            disabled: false,
            color: SwitchColor::Accent,
            tooltip: None,
            tab_index: None,
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Sets the color of the switch using the specified [`SwitchColor`].
    /// This changes the color scheme of the switch when it's in the "on" state.
    pub fn color(mut self, color: SwitchColor) -> Self {
        self.color = color;
        self
    }

    pub fn tooltip(mut self, tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.tooltip = Some(Rc::new(tooltip));
        self
    }

    pub fn tab_index(mut self, tab_index: isize) -> Self {
        self.tab_index = Some(tab_index);
        self
    }
}

impl RenderOnce for SwitchField {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tooltip = self
            .tooltip
            .zip(self.label.clone())
            .map(|(tooltip_fn, label)| {
                h_flex().gap_0p5().child(Label::new(label)).child(
                    IconButton::new("tooltip_button", IconName::Info)
                        .icon_size(IconSize::XSmall)
                        .icon_color(Color::Muted)
                        .shape(crate::IconButtonShape::Square)
                        .style(ButtonStyle::Transparent)
                        .tooltip({
                            let tooltip = tooltip_fn.clone();
                            move |window, cx| tooltip(window, cx)
                        })
                        .on_click(|_, _, _| {}), // Intentional empty on click handler so that clicking on the info tooltip icon doesn't trigger the switch toggle
                )
            });

        h_flex()
            .id((self.id.clone(), "container"))
            .when(!self.disabled, |this| {
                this.hover(|this| this.cursor_pointer())
            })
            .w_full()
            .gap_4()
            .justify_between()
            .flex_wrap()
            .child(match (&self.description, tooltip) {
                (Some(description), Some(tooltip)) => v_flex()
                    .gap_0p5()
                    .max_w_5_6()
                    .child(tooltip)
                    .child(Label::new(description.clone()).color(Color::Muted))
                    .into_any_element(),
                (Some(description), None) => v_flex()
                    .gap_0p5()
                    .max_w_5_6()
                    .when_some(self.label, |this, label| this.child(Label::new(label)))
                    .child(Label::new(description.clone()).color(Color::Muted))
                    .into_any_element(),
                (None, Some(tooltip)) => tooltip.into_any_element(),
                (None, None) => {
                    if let Some(label) = self.label.clone() {
                        Label::new(label).into_any_element()
                    } else {
                        gpui::Empty.into_any_element()
                    }
                }
            })
            .child(
                Switch::new((self.id.clone(), "switch"), self.toggle_state)
                    .color(self.color)
                    .disabled(self.disabled)
                    .when_some(
                        self.tab_index.filter(|_| !self.disabled),
                        |this, tab_index| this.tab_index(tab_index),
                    )
                    .on_click({
                        let on_click = self.on_click.clone();
                        move |state, window, cx| {
                            (on_click)(state, window, cx);
                        }
                    }),
            )
            .when(!self.disabled, |this| {
                this.on_click({
                    let on_click = self.on_click.clone();
                    let toggle_state = self.toggle_state;
                    move |_click, window, cx| {
                        (on_click)(&toggle_state.inverse(), window, cx);
                    }
                })
            })
    }
}

impl Component for SwitchField {
    fn scope() -> ComponentScope {
        ComponentScope::Input
    }

    fn description() -> &'static str {
        "A field component that combines a label, description, and switch"
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> AnyElement {
        v_flex()
            .gap_6()
            .children(vec![
                example_group_with_title(
                    "States",
                    vec![
                        single_example(
                            "Unselected",
                            SwitchField::new(
                                "switch_field_unselected",
                                Some("Enable notifications"),
                                Some(SharedString::new_static(
                                    "Receive notifications when new messages arrive.",
                                )),
                                ToggleState::Unselected,
                                |_, _, _| {},
                            )
                            .into_any_element(),
                        ),
                        single_example(
                            "Selected",
                            SwitchField::new(
                                "switch_field_selected",
                                Some("Enable notifications"),
                                Some(SharedString::new_static(
                                    "Receive notifications when new messages arrive.",
                                )),
                                ToggleState::Selected,
                                |_, _, _| {},
                            )
                            .into_any_element(),
                        ),
                    ],
                ),
                example_group_with_title(
                    "Colors",
                    vec![
                        single_example(
                            "Default",
                            SwitchField::new(
                                "switch_field_default",
                                Some("Default color"),
                                Some(SharedString::new_static(
                                    "This uses the default switch color.",
                                )),
                                ToggleState::Selected,
                                |_, _, _| {},
                            )
                            .into_any_element(),
                        ),
                        single_example(
                            "Accent",
                            SwitchField::new(
                                "switch_field_accent",
                                Some("Accent color"),
                                Some(SharedString::new_static(
                                    "This uses the accent color scheme.",
                                )),
                                ToggleState::Selected,
                                |_, _, _| {},
                            )
                            .color(SwitchColor::Accent)
                            .into_any_element(),
                        ),
                    ],
                ),
                example_group_with_title(
                    "Disabled",
                    vec![single_example(
                        "Disabled",
                        SwitchField::new(
                            "switch_field_disabled",
                            Some("Disabled field"),
                            Some(SharedString::new_static(
                                "This field is disabled and cannot be toggled.",
                            )),
                            ToggleState::Selected,
                            |_, _, _| {},
                        )
                        .disabled(true)
                        .into_any_element(),
                    )],
                ),
                example_group_with_title(
                    "No Description",
                    vec![single_example(
                        "No Description",
                        SwitchField::new(
                            "switch_field_disabled",
                            Some("Disabled field"),
                            None,
                            ToggleState::Selected,
                            |_, _, _| {},
                        )
                        .into_any_element(),
                    )],
                ),
                example_group_with_title(
                    "With Tooltip",
                    vec![
                        single_example(
                            "Tooltip with Description",
                            SwitchField::new(
                                "switch_field_tooltip_with_desc",
                                Some("Nice Feature"),
                                Some(SharedString::new_static(
                                    "Enable advanced configuration options.",
                                )),
                                ToggleState::Unselected,
                                |_, _, _| {},
                            )
                            .tooltip(Tooltip::text("This is content for this tooltip!"))
                            .into_any_element(),
                        ),
                        single_example(
                            "Tooltip without Description",
                            SwitchField::new(
                                "switch_field_tooltip_no_desc",
                                Some("Nice Feature"),
                                None,
                                ToggleState::Selected,
                                |_, _, _| {},
                            )
                            .tooltip(Tooltip::text("This is content for this tooltip!"))
                            .into_any_element(),
                        ),
                    ],
                ),
            ])
            .into_any_element()
    }
}

impl Component for Checkbox {
    fn scope() -> ComponentScope {
        ComponentScope::Input
    }

    fn description() -> &'static str {
        "A checkbox component that can be used for multiple choice selections"
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> AnyElement {
        v_flex()
            .gap_6()
            .children(vec![
                example_group_with_title(
                    "States",
                    vec![
                        single_example(
                            "Unselected",
                            Checkbox::new("checkbox_unselected", ToggleState::Unselected)
                                .into_any_element(),
                        ),
                        single_example(
                            "Placeholder",
                            Checkbox::new("checkbox_indeterminate", ToggleState::Selected)
                                .placeholder(true)
                                .into_any_element(),
                        ),
                        single_example(
                            "Indeterminate",
                            Checkbox::new("checkbox_indeterminate", ToggleState::Indeterminate)
                                .into_any_element(),
                        ),
                        single_example(
                            "Selected",
                            Checkbox::new("checkbox_selected", ToggleState::Selected)
                                .into_any_element(),
                        ),
                    ],
                ),
                example_group_with_title(
                    "Styles",
                    vec![
                        single_example(
                            "Default",
                            Checkbox::new("checkbox_default", ToggleState::Selected)
                                .into_any_element(),
                        ),
                        single_example(
                            "Filled",
                            Checkbox::new("checkbox_filled", ToggleState::Selected)
                                .fill()
                                .into_any_element(),
                        ),
                        single_example(
                            "ElevationBased",
                            Checkbox::new("checkbox_elevation", ToggleState::Selected)
                                .style(ToggleStyle::ElevationBased(ElevationIndex::EditorSurface))
                                .into_any_element(),
                        ),
                        single_example(
                            "Custom Color",
                            Checkbox::new("checkbox_custom", ToggleState::Selected)
                                .style(ToggleStyle::Custom(hsla(142.0 / 360., 0.68, 0.45, 0.7)))
                                .into_any_element(),
                        ),
                    ],
                ),
                example_group_with_title(
                    "Disabled",
                    vec![
                        single_example(
                            "Unselected",
                            Checkbox::new("checkbox_disabled_unselected", ToggleState::Unselected)
                                .disabled(true)
                                .into_any_element(),
                        ),
                        single_example(
                            "Selected",
                            Checkbox::new("checkbox_disabled_selected", ToggleState::Selected)
                                .disabled(true)
                                .into_any_element(),
                        ),
                    ],
                ),
                example_group_with_title(
                    "With Label",
                    vec![single_example(
                        "Default",
                        Checkbox::new("checkbox_with_label", ToggleState::Selected)
                            .label("Always save on quit")
                            .into_any_element(),
                    )],
                ),
                example_group_with_title(
                    "Extra",
                    vec![single_example(
                        "Visualization-Only",
                        Checkbox::new("viz_only", ToggleState::Selected)
                            .visualization_only(true)
                            .into_any_element(),
                    )],
                ),
            ])
            .into_any_element()
    }
}

impl Component for Switch {
    fn scope() -> ComponentScope {
        ComponentScope::Input
    }

    fn description() -> &'static str {
        "A switch component that represents binary states like on/off"
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> AnyElement {
        v_flex()
            .gap_6()
            .children(vec![
                example_group_with_title(
                    "States",
                    vec![
                        single_example(
                            "Off",
                            Switch::new("switch_off", ToggleState::Unselected)
                                .on_click(|_, _, _cx| {})
                                .into_any_element(),
                        ),
                        single_example(
                            "On",
                            Switch::new("switch_on", ToggleState::Selected)
                                .on_click(|_, _, _cx| {})
                                .into_any_element(),
                        ),
                    ],
                ),
                example_group_with_title(
                    "Colors",
                    vec![
                        single_example(
                            "Accent (Default)",
                            Switch::new("switch_accent_style", ToggleState::Selected)
                                .on_click(|_, _, _cx| {})
                                .into_any_element(),
                        ),
                        single_example(
                            "Custom",
                            Switch::new("switch_custom_style", ToggleState::Selected)
                                .color(SwitchColor::Custom(hsla(300.0 / 360.0, 0.6, 0.6, 1.0)))
                                .on_click(|_, _, _cx| {})
                                .into_any_element(),
                        ),
                    ],
                ),
                example_group_with_title(
                    "Disabled",
                    vec![
                        single_example(
                            "Off",
                            Switch::new("switch_disabled_off", ToggleState::Unselected)
                                .disabled(true)
                                .into_any_element(),
                        ),
                        single_example(
                            "On",
                            Switch::new("switch_disabled_on", ToggleState::Selected)
                                .disabled(true)
                                .into_any_element(),
                        ),
                    ],
                ),
                example_group_with_title(
                    "With Label",
                    vec![
                        single_example(
                            "Start Label",
                            Switch::new("switch_with_label_start", ToggleState::Selected)
                                .label("Always save on quit")
                                .label_position(SwitchLabelPosition::Start)
                                .into_any_element(),
                        ),
                        single_example(
                            "End Label",
                            Switch::new("switch_with_label_end", ToggleState::Selected)
                                .label("Always save on quit")
                                .label_position(SwitchLabelPosition::End)
                                .into_any_element(),
                        ),
                        single_example(
                            "Default Size Label",
                            Switch::new("switch_with_label_default_size", ToggleState::Selected)
                                .label("Always save on quit")
                                .label_size(LabelSize::Default)
                                .into_any_element(),
                        ),
                        single_example(
                            "Small Size Label",
                            Switch::new("switch_with_label_small_size", ToggleState::Selected)
                                .label("Always save on quit")
                                .label_size(LabelSize::Small)
                                .into_any_element(),
                        ),
                    ],
                ),
                example_group_with_title(
                    "With Keybinding",
                    vec![single_example(
                        "Keybinding",
                        Switch::new("switch_with_keybinding", ToggleState::Selected)
                            .key_binding(Some(KeyBinding::from_keystrokes(
                                vec![KeybindingKeystroke::from_keystroke(
                                    Keystroke::parse("cmd-s").unwrap(),
                                )]
                                .into(),
                                false,
                            )))
                            .into_any_element(),
                    )],
                ),
            ])
            .into_any_element()
    }
}
