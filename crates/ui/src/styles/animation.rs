use crate::prelude::*;
use gpui::{AnimationElement, AnimationExt, Global, Point, Styled, point, transition};
use std::time::{Duration, Instant};

use gpui::ease_out_quint;

/// Whether the application lets surfaces move. It turns motion on once it follows the user's
/// motion preference; until then, as in the tests of single components, every surface appears
/// where it settles.
struct SurfaceMotionSwitch(bool);

impl Global for SurfaceMotionSwitch {}

pub fn enable_surface_motion(enabled: bool, cx: &mut App) {
    cx.set_global(SurfaceMotionSwitch(enabled));
}

/// Whether popups, dialogs, balloons and tool windows move as they come and go right now.
pub fn surfaces_move(cx: &App) -> bool {
    cx.try_global::<SurfaceMotionSwitch>()
        .is_some_and(|switch| switch.0)
        && !cx.reduce_motion()
}

/// How a kind of surface comes and goes. Every presenter of a kind — the modal layer for dialogs
/// and pickers, context menus, notification balloons, docks — takes the motion from here, so all
/// surfaces of one kind move alike and one place tunes them.
///
/// Surfaces only fade and travel a few pixels with a decelerating curve: Apple keeps motion brief
/// and purposeful and lets people act before it ends, so a surface takes input from its first
/// frame, and a dismissed one lets the mouse through while it fades out. Their layout and glass
/// never change while they move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceMotion {
    pub enter: Duration,
    /// Zero removes the surface at once.
    pub exit: Duration,
    /// Where the surface starts, relative to where it settles.
    pub offset: Point<Pixels>,
}

impl SurfaceMotion {
    /// Menus open under the pointer many times an hour, and motion in an interaction that
    /// frequent only slows it down, so they fade in briefly and stay where they open.
    pub const MENU: Self = Self {
        enter: Duration::from_millis(110),
        exit: Duration::ZERO,
        offset: point(px(0.), px(0.)),
    };

    /// Dialogs, palettes and pickers drop in a little from the title bar, like a macOS sheet, and
    /// fade out when they are dismissed.
    pub const DIALOG: Self = Self {
        enter: Duration::from_millis(220),
        exit: Duration::from_millis(140),
        offset: point(px(0.), px(-10.)),
    };

    /// Notification balloons slide in from the window edge they stack against.
    pub const BALLOON: Self = Self {
        enter: Duration::from_millis(260),
        exit: Duration::ZERO,
        offset: point(px(24.), px(0.)),
    };

    pub const TOOLTIP: Self = Self {
        enter: Duration::from_millis(120),
        exit: Duration::ZERO,
        offset: point(px(0.), px(0.)),
    };

    /// A tool window slides in from the edge of the window its dock sits at.
    pub const fn tool_window(from: AnimationDirection) -> Self {
        let distance = 12.;
        Self {
            enter: Duration::from_millis(220),
            exit: Duration::ZERO,
            offset: match from {
                AnimationDirection::FromLeft => point(px(-distance), px(0.)),
                AnimationDirection::FromRight => point(px(distance), px(0.)),
                AnimationDirection::FromBottom => point(px(0.), px(distance)),
                AnimationDirection::FromTop => point(px(0.), px(-distance)),
            },
        }
    }

    /// `content` coming in with this motion. `id` must be new each time the surface opens, since
    /// the motion runs once per id.
    pub fn present(
        self,
        id: impl Into<ElementId>,
        content: impl IntoElement,
        cx: &App,
    ) -> AnyElement {
        self.present_when_ready(id, content, true, None, cx)
    }

    /// Like [`Self::present`], for a surface that may still be loading (`ready`, see
    /// [`ReadyGate`](crate::ReadyGate)) or already leaving (`leaving_since`): it appears with the
    /// whole motion from the frame its content is ready, and fades out from the moment it was
    /// dismissed.
    pub fn present_when_ready(
        self,
        id: impl Into<ElementId>,
        content: impl IntoElement,
        ready: bool,
        leaving_since: Option<Instant>,
        cx: &App,
    ) -> AnyElement {
        if !surfaces_move(cx) {
            return content.into_any_element();
        }
        transition(id, content)
            .enter(self.enter)
            .exit(self.exit)
            .offset(self.offset)
            .ready(ready)
            .leaving_since(leaving_since)
            .into_any_element()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationDuration {
    Instant = 50,
    Fast = 150,
    Slow = 300,
}

impl AnimationDuration {
    pub fn duration(&self) -> Duration {
        Duration::from_millis(*self as u64)
    }
}

impl Into<std::time::Duration> for AnimationDuration {
    fn into(self) -> Duration {
        self.duration()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationDirection {
    FromBottom,
    FromLeft,
    FromRight,
    FromTop,
}

pub trait DefaultAnimations: Styled + Sized + Element {
    fn animate_in(
        self,
        animation_type: AnimationDirection,
        fade_in: bool,
    ) -> AnimationElement<Self> {
        let animation_name = match animation_type {
            AnimationDirection::FromBottom => "animate_from_bottom",
            AnimationDirection::FromLeft => "animate_from_left",
            AnimationDirection::FromRight => "animate_from_right",
            AnimationDirection::FromTop => "animate_from_top",
        };

        let animation_id = self.id().map_or_else(
            || ElementId::from(animation_name),
            |id| (id, animation_name).into(),
        );

        self.with_animation(
            animation_id,
            gpui::Animation::new(AnimationDuration::Fast.into()).with_easing(ease_out_quint()),
            move |mut this, delta| {
                let start_opacity = 0.4;
                let start_pos = 0.0;
                let end_pos = 40.0;

                if fade_in {
                    this = this.opacity(start_opacity + delta * (1.0 - start_opacity));
                }

                match animation_type {
                    AnimationDirection::FromBottom => {
                        this.bottom(px(start_pos + delta * (end_pos - start_pos)))
                    }
                    AnimationDirection::FromLeft => {
                        this.left(px(start_pos + delta * (end_pos - start_pos)))
                    }
                    AnimationDirection::FromRight => {
                        this.right(px(start_pos + delta * (end_pos - start_pos)))
                    }
                    AnimationDirection::FromTop => {
                        this.top(px(start_pos + delta * (end_pos - start_pos)))
                    }
                }
            },
        )
    }

    fn animate_in_from_bottom(self, fade: bool) -> AnimationElement<Self> {
        self.animate_in(AnimationDirection::FromBottom, fade)
    }

    fn animate_in_from_left(self, fade: bool) -> AnimationElement<Self> {
        self.animate_in(AnimationDirection::FromLeft, fade)
    }

    fn animate_in_from_right(self, fade: bool) -> AnimationElement<Self> {
        self.animate_in(AnimationDirection::FromRight, fade)
    }

    fn animate_in_from_top(self, fade: bool) -> AnimationElement<Self> {
        self.animate_in(AnimationDirection::FromTop, fade)
    }
}

impl<E: Styled + Element> DefaultAnimations for E {}

// Don't use this directly, it only exists to show animation previews
#[derive(RegisterComponent)]
struct Animation {}

impl Component for Animation {
    fn scope() -> ComponentScope {
        ComponentScope::Utilities
    }

    fn description() -> &'static str {
        "Demonstrates various animation patterns and transitions available in the UI system."
    }

    fn preview(_window: &mut Window, cx: &mut App) -> AnyElement {
        let container_size = 128.0;
        let element_size = 32.0;
        let offset = container_size / 2.0 - element_size / 2.0;

        let container = || {
            h_flex()
                .relative()
                .justify_center()
                .bg(cx.theme().colors().text.opacity(0.05))
                .border_1()
                .border_color(cx.theme().colors().border)
                .rounded_sm()
        };

        v_flex()
            .gap_6()
            .children(vec![
                example_group_with_title(
                    "Animate In",
                    vec![
                        single_example(
                            "From Bottom",
                            container()
                                .size(px(container_size))
                                .child(
                                    div()
                                        .id("animate-in-from-bottom")
                                        .absolute()
                                        .size(px(element_size))
                                        .left(px(offset))
                                        .rounded_md()
                                        .bg(gpui::red())
                                        .animate_in_from_bottom(false),
                                )
                                .into_any_element(),
                        ),
                        single_example(
                            "From Top",
                            container()
                                .size(px(container_size))
                                .child(
                                    div()
                                        .id("animate-in-from-top")
                                        .absolute()
                                        .size(px(element_size))
                                        .left(px(offset))
                                        .rounded_md()
                                        .bg(gpui::blue())
                                        .animate_in_from_top(false),
                                )
                                .into_any_element(),
                        ),
                        single_example(
                            "From Left",
                            container()
                                .size(px(container_size))
                                .child(
                                    div()
                                        .id("animate-in-from-left")
                                        .absolute()
                                        .size(px(element_size))
                                        .top(px(offset))
                                        .rounded_md()
                                        .bg(gpui::green())
                                        .animate_in_from_left(false),
                                )
                                .into_any_element(),
                        ),
                        single_example(
                            "From Right",
                            container()
                                .size(px(container_size))
                                .child(
                                    div()
                                        .id("animate-in-from-right")
                                        .absolute()
                                        .size(px(element_size))
                                        .top(px(offset))
                                        .rounded_md()
                                        .bg(gpui::yellow())
                                        .animate_in_from_right(false),
                                )
                                .into_any_element(),
                        ),
                    ],
                )
                .grow(),
                example_group_with_title(
                    "Fade and Animate In",
                    vec![
                        single_example(
                            "From Bottom",
                            container()
                                .size(px(container_size))
                                .child(
                                    div()
                                        .id("fade-animate-in-from-bottom")
                                        .absolute()
                                        .size(px(element_size))
                                        .left(px(offset))
                                        .rounded_md()
                                        .bg(gpui::red())
                                        .animate_in_from_bottom(true),
                                )
                                .into_any_element(),
                        ),
                        single_example(
                            "From Top",
                            container()
                                .size(px(container_size))
                                .child(
                                    div()
                                        .id("fade-animate-in-from-top")
                                        .absolute()
                                        .size(px(element_size))
                                        .left(px(offset))
                                        .rounded_md()
                                        .bg(gpui::blue())
                                        .animate_in_from_top(true),
                                )
                                .into_any_element(),
                        ),
                        single_example(
                            "From Left",
                            container()
                                .size(px(container_size))
                                .child(
                                    div()
                                        .id("fade-animate-in-from-left")
                                        .absolute()
                                        .size(px(element_size))
                                        .top(px(offset))
                                        .rounded_md()
                                        .bg(gpui::green())
                                        .animate_in_from_left(true),
                                )
                                .into_any_element(),
                        ),
                        single_example(
                            "From Right",
                            container()
                                .size(px(container_size))
                                .child(
                                    div()
                                        .id("fade-animate-in-from-right")
                                        .absolute()
                                        .size(px(element_size))
                                        .top(px(offset))
                                        .rounded_md()
                                        .bg(gpui::yellow())
                                        .animate_in_from_right(true),
                                )
                                .into_any_element(),
                        ),
                    ],
                )
                .grow(),
            ])
            .into_any_element()
    }
}
