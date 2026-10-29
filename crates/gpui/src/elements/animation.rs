use scheduler::Instant;
use std::{cell::Cell, rc::Rc, time::Duration};

use crate::{
    AnyElement, App, Element, ElementId, Empty, GlobalElementId, InspectorElementId, IntoElement,
    ParentElement, Pixels, Point, SpringAnimation, SpringConfig, SpringPlayback, SpringState,
    SpringTarget, Window,
};

pub use easing::*;
use smallvec::SmallVec;

/// An animation that can be applied to an element.
#[derive(Clone)]
pub struct Animation {
    /// The amount of time for which this animation should run
    pub duration: Duration,
    /// Whether to repeat this animation when it finishes
    pub oneshot: bool,
    /// Whether to derive the phase from a shared clock. See [`Animation::repeat_synced`].
    pub synced: bool,
    /// A function that maps normalized time to an animated value.
    /// The result may exceed 0..1 for easing functions that overshoot.
    pub easing: Rc<dyn Fn(f32) -> f32>,
    /// The maximum number of times per second this animation re-renders.
    /// When `None`, the animation re-renders on every frame.
    pub max_fps: Option<f32>,
}

impl Animation {
    /// Create a new animation with the given duration.
    /// By default the animation will only run once and will use a linear easing function.
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            oneshot: true,
            synced: false,
            easing: Rc::new(linear),
            max_fps: None,
        }
    }

    /// Set the animation to loop when it finishes.
    pub fn repeat(mut self) -> Self {
        self.oneshot = false;
        self
    }

    /// Set the animation to loop when it finishes, phase-locked to a clock shared by the whole [`App`].
    pub fn repeat_synced(mut self) -> Self {
        self.oneshot = false;
        self.synced = true;
        self
    }

    /// Sets the easing function used to map normalized time to an animated value.
    ///
    /// The output is not clamped, allowing physical easing functions such as
    /// springs to overshoot.
    pub fn with_easing(mut self, easing: impl Fn(f32) -> f32 + 'static) -> Self {
        self.easing = Rc::new(easing);
        self
    }

    /// Limit how often this animation re-renders. Instead of re-rendering on
    /// every frame, the animation schedules its next render `1 / max_fps`
    /// seconds after the current one. Values that are not finite and positive
    /// are ignored.
    pub fn with_max_fps(mut self, max_fps: f32) -> Self {
        self.max_fps = Some(max_fps);
        self
    }
}

/// An extension trait for adding the animation wrapper to both Elements and Components
///
/// Animations rendered through this trait automatically respect
/// [`App::reduce_motion`](crate::App::reduce_motion): when it is set,
/// the element is rendered in a static state (the end state for oneshot
/// animations, the start state for repeating ones) and no animation frames are
/// scheduled.
pub trait AnimationExt {
    /// Render this component or element with an animation
    fn with_animation(
        self,
        id: impl Into<ElementId>,
        animation: Animation,
        animator: impl Fn(Self, f32) -> Self + 'static,
    ) -> AnimationElement<Self>
    where
        Self: Sized,
    {
        AnimationElement {
            id: id.into(),
            element: Some(self),
            animator: Box::new(move |this, _, value| animator(this, value)),
            animations: smallvec::smallvec![animation],
        }
    }

    /// Render this component or element with a chain of animations
    fn with_animations(
        self,
        id: impl Into<ElementId>,
        animations: Vec<Animation>,
        animator: impl Fn(Self, usize, f32) -> Self + 'static,
    ) -> AnimationElement<Self>
    where
        Self: Sized,
    {
        AnimationElement {
            id: id.into(),
            element: Some(self),
            animator: Box::new(animator),
            animations: animations.into(),
        }
    }

    /// Renders this component or element at the value produced by a spring.
    ///
    /// The element ID preserves position and velocity across target changes.
    /// A newly mounted spring starts at its target unless configured with
    /// [`SpringAnimation::from`].
    fn with_spring<T>(
        self,
        id: impl Into<ElementId>,
        animation: SpringAnimation<T>,
        animator: impl FnOnce(Self, T::Output) -> Self + 'static,
    ) -> SpringAnimationElement<Self>
    where
        Self: Sized,
        T: SpringTarget,
        T::Output: 'static,
    {
        let SpringAnimation {
            config,
            target,
            epsilon,
            initial,
            playback,
        } = animation;
        let scalar_target = target.target();
        SpringAnimationElement {
            id: id.into(),
            element: Some(self),
            config,
            target: scalar_target,
            epsilon,
            initial,
            playback,
            animator: Some(Box::new(move |this, value| {
                animator(this, target.resolve(value))
            })),
        }
    }
}

impl<E: IntoElement + 'static> AnimationExt for E {}

/// A GPUI element that applies an animation to another element
pub struct AnimationElement<E> {
    id: ElementId,
    element: Option<E>,
    animations: SmallVec<[Animation; 1]>,
    animator: Box<dyn Fn(E, usize, f32) -> E + 'static>,
}

/// A GPUI element driven by a stateful spring.
pub struct SpringAnimationElement<E> {
    id: ElementId,
    element: Option<E>,
    config: SpringConfig,
    target: f32,
    epsilon: f32,
    initial: Option<f32>,
    playback: SpringPlayback,
    animator: Option<Box<dyn FnOnce(E, f32) -> E + 'static>>,
}

impl<E: ParentElement> ParentElement for SpringAnimationElement<E> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        let Some(element) = &mut self.element else {
            return;
        };

        element.extend(elements);
    }
}

impl<E> SpringAnimationElement<E> {
    /// Returns a new [`SpringAnimationElement<E>`] after applying the given function
    /// to the element being animated.
    pub fn map_element(mut self, f: impl FnOnce(E) -> E) -> SpringAnimationElement<E> {
        self.element = self.element.map(f);
        self
    }
}

impl<E: IntoElement + 'static> IntoElement for SpringAnimationElement<E> {
    type Element = SpringAnimationElement<E>;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl<E: ParentElement> ParentElement for AnimationElement<E> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        let Some(element) = &mut self.element else {
            return;
        };

        element.extend(elements);
    }
}

impl<E> AnimationElement<E> {
    /// Returns a new [`AnimationElement<E>`] after applying the given function
    /// to the element being animated.
    pub fn map_element(mut self, f: impl FnOnce(E) -> E) -> AnimationElement<E> {
        self.element = self.element.map(f);
        self
    }
}

impl<E: IntoElement + 'static> IntoElement for AnimationElement<E> {
    type Element = AnimationElement<E>;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct AnimationState {
    start: Instant,
    animation_ix: usize,
    /// Whether a throttled re-render (see [`Animation::with_max_fps`]) is
    /// already scheduled, so overlapping renders don't stack extra timers.
    delayed_frame_pending: Rc<Cell<bool>>,
}

struct SpringElementState {
    spring: SpringState,
    target: f32,
    config: SpringConfig,
    initial: f32,
    playback: SpringPlayback,
    updated_at: Instant,
}

impl<E: IntoElement + 'static> Element for SpringAnimationElement<E> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (crate::LayoutId, Self::RequestLayoutState) {
        window.with_element_state(global_id.unwrap(), |state, window| {
            let now = cx.background_executor().now();
            let initial = self.initial.unwrap_or(self.target);
            let mut state = state.unwrap_or_else(|| SpringElementState {
                spring: SpringState {
                    position: initial,
                    velocity: 0.0,
                },
                target: self.target,
                config: self.config,
                initial,
                playback: self.playback,
                updated_at: now,
            });

            let elapsed = now.duration_since(state.updated_at).as_secs_f32();
            match state.playback {
                SpringPlayback::Running => {
                    state.spring = state.config.step(state.spring, state.target, elapsed);
                }
                SpringPlayback::Paused
                | SpringPlayback::Stopped
                | SpringPlayback::Completed
                | SpringPlayback::Cancelled => {}
            }

            state.config = self.config;
            state.target = self.target;

            let done = match self.playback {
                SpringPlayback::Running => {
                    if cx.reduce_motion() {
                        state.spring = SpringState {
                            position: state.target,
                            velocity: 0.0,
                        };
                        true
                    } else {
                        let done =
                            state
                                .config
                                .is_settled(state.spring, state.target, self.epsilon);
                        if done {
                            state.spring = SpringState {
                                position: state.target,
                                velocity: 0.0,
                            };
                        }
                        done
                    }
                }
                SpringPlayback::Paused => true,
                SpringPlayback::Stopped => {
                    state.spring.velocity = 0.0;
                    true
                }
                SpringPlayback::Completed => {
                    state.spring = SpringState {
                        position: state.target,
                        velocity: 0.0,
                    };
                    true
                }
                SpringPlayback::Cancelled => {
                    state.spring = SpringState {
                        position: state.initial,
                        velocity: 0.0,
                    };
                    true
                }
            };
            state.playback = self.playback;
            state.updated_at = now;

            let element = self.element.take().expect("should only be called once");
            let animator = self.animator.take().expect("should only be called once");
            let mut element = animator(element, state.spring.position).into_any_element();

            if !done {
                window.request_animation_frame();
            }

            ((element.request_layout(window, cx), element), state)
        })
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: crate::Bounds<crate::Pixels>,
        element: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        element.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: crate::Bounds<crate::Pixels>,
        element: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        element.paint(window, cx);
    }
}

impl<E: IntoElement + 'static> Element for AnimationElement<E> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (crate::LayoutId, Self::RequestLayoutState) {
        window.with_element_state(global_id.unwrap(), |state, window| {
            let mut state = state.unwrap_or_else(|| AnimationState {
                start: Instant::now(),
                animation_ix: 0,
                delayed_frame_pending: Rc::new(Cell::new(false)),
            });
            let (animation_ix, delta, done) = if cx.reduce_motion() {
                let animation_ix = self.animations.len() - 1;
                let delta = if self.animations[animation_ix].oneshot {
                    1.0
                } else {
                    0.0
                };
                (animation_ix, delta, true)
            } else {
                let animation_ix = state.animation_ix;
                let duration = self.animations[animation_ix].duration;

                let elapsed = if self.animations[animation_ix].synced && !duration.is_zero() {
                    let elapsed = cx.background_executor().now() - cx.synced_animation_epoch;
                    // Reduce modulo the duration before f32 conversion, which loses sub-second precision at scale.
                    Duration::from_nanos((elapsed.as_nanos() % duration.as_nanos()) as u64)
                } else {
                    state.start.elapsed()
                };
                let mut delta = elapsed.as_secs_f32() / duration.as_secs_f32();

                let mut done = false;
                if delta > 1.0 {
                    if self.animations[animation_ix].oneshot {
                        if animation_ix >= self.animations.len() - 1 {
                            done = true;
                        } else {
                            state.start = Instant::now();
                            state.animation_ix += 1;
                        }
                        delta = 1.0;
                    } else {
                        delta %= 1.0;
                    }
                }
                (animation_ix, delta, done)
            };
            let delta = (self.animations[animation_ix].easing)(delta);

            debug_assert!(delta.is_finite(), "animated value should be finite");

            let element = self.element.take().expect("should only be called once");
            let mut element = (self.animator)(element, animation_ix, delta).into_any_element();

            if !done {
                match self.animations[animation_ix].max_fps {
                    Some(max_fps) if max_fps.is_finite() && max_fps > 0.0 => {
                        if !state.delayed_frame_pending.get() {
                            state.delayed_frame_pending.set(true);
                            let delayed_frame_pending = state.delayed_frame_pending.clone();
                            let view = window.current_view();
                            let interval = Duration::from_secs_f32(1.0 / max_fps);
                            window
                                .spawn(cx, async move |cx| {
                                    cx.background_executor().timer(interval).await;
                                    delayed_frame_pending.set(false);
                                    cx.update(move |_, cx| cx.notify(view)).ok();
                                })
                                .detach();
                        }
                    }
                    _ => window.request_animation_frame(),
                }
            }

            ((element.request_layout(window, cx), element), state)
        })
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: crate::Bounds<crate::Pixels>,
        element: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        element.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: crate::Bounds<crate::Pixels>,
        element: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        element.paint(window, cx);
    }
}

/// How a surface comes and goes, owned by whatever presents it: it fades in while it travels from
/// `offset` into place, starting at the first frame its content is ready, and fades out in place,
/// inert, once the presenter says it is leaving. Only opacity and position change, so its layout
/// and its glass stay the same throughout.
///
/// [`App::reduce_motion`](crate::App::reduce_motion) shows it settled at once.
pub fn transition(id: impl Into<ElementId>, child: impl IntoElement) -> Transition {
    Transition {
        id: id.into(),
        child: Some(child.into_any_element()),
        enter: Duration::ZERO,
        exit: Duration::ZERO,
        easing: Rc::new(ease_out_quint()),
        offset: Point::default(),
        ready: true,
        leaving_since: None,
    }
}

/// See [`transition`].
pub struct Transition {
    id: ElementId,
    child: Option<AnyElement>,
    enter: Duration,
    exit: Duration,
    easing: Rc<dyn Fn(f32) -> f32>,
    offset: Point<Pixels>,
    ready: bool,
    leaving_since: Option<Instant>,
}

impl Transition {
    /// How long the surface takes to appear.
    pub fn enter(mut self, duration: Duration) -> Self {
        self.enter = duration;
        self
    }

    /// How long the surface takes to fade out once it is leaving.
    pub fn exit(mut self, duration: Duration) -> Self {
        self.exit = duration;
        self
    }

    /// Maps the share of the enter time that has passed to how far the surface has arrived.
    pub fn easing(mut self, easing: impl Fn(f32) -> f32 + 'static) -> Self {
        self.easing = Rc::new(easing);
        self
    }

    /// Where the surface starts, relative to where it settles.
    pub fn offset(mut self, offset: Point<Pixels>) -> Self {
        self.offset = offset;
        self
    }

    /// Whether the content is ready to be seen. Until it is, the surface is laid out and takes
    /// input but draws nothing, and its enter time has not started, so content that loads after
    /// the surface opens appears in its final form with the whole transition instead of popping
    /// in halfway through it.
    pub fn ready(mut self, ready: bool) -> Self {
        self.ready = ready;
        self
    }

    /// The moment the presenter dismissed the surface, on the executor's clock
    /// ([`BackgroundExecutor::now`](crate::BackgroundExecutor::now)): from then on it fades out
    /// and lets the mouse through to the window beneath it.
    pub fn leaving_since(mut self, since: Option<Instant>) -> Self {
        self.leaving_since = since;
        self
    }

    /// Where the surface is in its motion, and whether that still changes.
    fn moment(&self, shown_since: Option<Instant>, cx: &App) -> (TransitionMoment, bool) {
        let Some(shown_since) = shown_since else {
            return (
                TransitionMoment {
                    arrival: 0.,
                    opacity: 0.,
                },
                false,
            );
        };
        if cx.reduce_motion() {
            let opacity = if self.leaving_since.is_some() { 0. } else { 1. };
            return (
                TransitionMoment {
                    arrival: 1.,
                    opacity,
                },
                false,
            );
        }
        let now = cx.background_executor().now();
        let arrived = progress(now, shown_since, self.enter);
        let arrival = (self.easing)(arrived).clamp(0., 1.);
        match self.leaving_since {
            // It fades out from wherever it got to, without moving.
            Some(leaving_since) => {
                let left = progress(now, leaving_since, self.exit);
                (
                    TransitionMoment {
                        arrival,
                        opacity: arrival.min(1. - left),
                    },
                    left < 1.,
                )
            }
            None => (
                TransitionMoment {
                    arrival,
                    opacity: arrival,
                },
                arrived < 1.,
            ),
        }
    }
}

#[derive(Clone, Copy)]
struct TransitionMoment {
    /// How far the surface has travelled from its offset, from 0 to 1.
    arrival: f32,
    opacity: f32,
}

fn progress(now: Instant, since: Instant, duration: Duration) -> f32 {
    if duration.is_zero() {
        return 1.;
    }
    (now.saturating_duration_since(since).as_secs_f32() / duration.as_secs_f32()).clamp(0., 1.)
}

#[derive(Default)]
struct TransitionState {
    /// The first frame the content was ready in.
    shown_since: Option<Instant>,
}

#[doc(hidden)]
pub struct TransitionFrame {
    child: AnyElement,
    moment: TransitionMoment,
}

impl IntoElement for Transition {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Transition {
    type RequestLayoutState = TransitionFrame;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (crate::LayoutId, Self::RequestLayoutState) {
        let Some(global_id) = global_id else {
            let mut child = self
                .child
                .take()
                .unwrap_or_else(|| Empty.into_any_element());
            return (
                child.request_layout(window, cx),
                TransitionFrame {
                    child,
                    moment: TransitionMoment {
                        arrival: 1.,
                        opacity: 1.,
                    },
                },
            );
        };
        window.with_element_state(global_id, |state: Option<TransitionState>, window| {
            let mut state = state.unwrap_or_default();
            if self.ready && state.shown_since.is_none() {
                state.shown_since = Some(cx.background_executor().now());
            }
            let (moment, changing) = self.moment(state.shown_since, cx);
            if changing {
                window.request_animation_frame();
            }
            let mut child = self
                .child
                .take()
                .unwrap_or_else(|| Empty.into_any_element());
            let layout_id = child.request_layout(window, cx);
            ((layout_id, TransitionFrame { child, moment }), state)
        })
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: crate::Bounds<Pixels>,
        frame: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let travel = self.offset * (1. - frame.moment.arrival);
        window.with_inert(self.leaving_since.is_some(), |window| {
            window.with_element_offset(travel, |window| {
                window.with_element_opacity(Some(frame.moment.opacity), |window| {
                    frame.child.prepaint(window, cx)
                })
            })
        });
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: crate::Bounds<Pixels>,
        frame: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_element_opacity(Some(frame.moment.opacity), |window| {
            frame.child.paint(window, cx)
        });
    }
}

mod easing {
    use std::f32::consts::PI;

    /// The linear easing function, or delta itself
    pub fn linear(delta: f32) -> f32 {
        delta
    }

    /// The quadratic easing function, delta * delta
    pub fn quadratic(delta: f32) -> f32 {
        delta * delta
    }

    /// The quadratic ease-in-out function, which starts and ends slowly but speeds up in the middle
    pub fn ease_in_out(delta: f32) -> f32 {
        if delta < 0.5 {
            2.0 * delta * delta
        } else {
            let x = -2.0 * delta + 2.0;
            1.0 - x * x / 2.0
        }
    }

    /// The Quint ease-out function, which starts quickly and decelerates to a stop
    pub fn ease_out_quint() -> impl Fn(f32) -> f32 {
        move |delta| 1.0 - (1.0 - delta).powi(5)
    }

    /// Apply the given easing function, first in the forward direction and then in the reverse direction
    pub fn bounce(easing: impl Fn(f32) -> f32) -> impl Fn(f32) -> f32 {
        move |delta| {
            if delta < 0.5 {
                easing(delta * 2.0)
            } else {
                easing((1.0 - delta) * 2.0)
            }
        }
    }

    /// A custom easing function for pulsating alpha that slows down as it approaches 0.1
    pub fn pulsating_between(min: f32, max: f32) -> impl Fn(f32) -> f32 {
        let range = max - min;

        move |delta| {
            // Use a combination of sine and cubic functions for a more natural breathing rhythm
            let t = (delta * 2.0 * PI).sin();
            let breath = (t * t * t + t) / 2.0;

            // Map the breath to our desired alpha range
            let normalized_alpha = (breath + 1.0) / 2.0;

            min + (normalized_alpha * range)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc, time::Duration};

    use crate::{
        Animation, Context, InteractiveElement, Pixels, Render, SpringAnimation, SpringConfig,
        TestAppContext, WindowHandle, div, prelude::*, px, size,
    };

    use super::*;

    struct AnimationTestView {
        rendered_deltas: Rc<RefCell<Vec<f32>>>,
        max_fps: Option<f32>,
    }

    struct SyncedAnimationTestView {
        show_second: bool,
        first_deltas: Rc<RefCell<Vec<f32>>>,
        second_deltas: Rc<RefCell<Vec<f32>>>,
    }

    struct SpringAnimationTestView {
        target: Pixels,
        initial: Option<Pixels>,
        playback: SpringPlayback,
        rendered_values: Rc<RefCell<Vec<Pixels>>>,
    }

    impl Render for SpringAnimationTestView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let rendered_values = self.rendered_values.clone();
            let mut animation = SpringAnimation::new(SpringConfig::new(100.0, 2.0, 1.0))
                .to(self.target)
                .with_epsilon(0.01)
                .playback(self.playback);
            if let Some(initial) = self.initial {
                animation = animation.from(initial);
            }
            div().with_spring("spring-animation", animation, move |this, value| {
                rendered_values.borrow_mut().push(value);
                this.left(value)
            })
        }
    }

    impl Render for SyncedAnimationTestView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let record_deltas = |deltas: Rc<RefCell<Vec<f32>>>| {
                move |this, delta| {
                    deltas.borrow_mut().push(delta);
                    this
                }
            };
            div()
                .size_full()
                .child(div().with_animation(
                    "first-synced-animation",
                    Animation::new(Duration::from_secs(1)).repeat_synced(),
                    record_deltas(self.first_deltas.clone()),
                ))
                .when(self.show_second, |this| {
                    this.child(div().with_animation(
                        "second-synced-animation",
                        Animation::new(Duration::from_secs(1)).repeat_synced(),
                        record_deltas(self.second_deltas.clone()),
                    ))
                })
        }
    }

    impl Render for AnimationTestView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let rendered_deltas = self.rendered_deltas.clone();
            // The throttled variant syncs to the shared clock so the deltas
            // follow the test scheduler's clock rather than wall time.
            let mut animation = Animation::new(Duration::from_secs(1));
            if let Some(max_fps) = self.max_fps {
                animation = animation.repeat_synced().with_max_fps(max_fps);
            } else {
                animation = animation.repeat();
            }
            div().size_full().child(div().with_animation(
                "repeating-animation",
                animation,
                move |this, delta| {
                    rendered_deltas.borrow_mut().push(delta);
                    this
                },
            ))
        }
    }

    fn open_test_window(
        cx: &mut TestAppContext,
    ) -> (Rc<RefCell<Vec<f32>>>, WindowHandle<AnimationTestView>) {
        open_test_window_with_max_fps(cx, None)
    }

    fn open_test_window_with_max_fps(
        cx: &mut TestAppContext,
        max_fps: Option<f32>,
    ) -> (Rc<RefCell<Vec<f32>>>, WindowHandle<AnimationTestView>) {
        let rendered_deltas = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.), px(100.)), {
            let rendered_deltas = rendered_deltas.clone();
            move |_, _| AnimationTestView {
                rendered_deltas,
                max_fps,
            }
        });
        cx.run_until_parked();
        (rendered_deltas, window)
    }

    fn simulate_next_frame<V: Render>(window: &WindowHandle<V>, cx: &mut TestAppContext) -> usize {
        let callback_count = window
            .update(cx, |_, window, cx| window.simulate_next_frame(cx))
            .unwrap();
        cx.run_until_parked();
        callback_count
    }
    // Before parent-animation-element, using .with_animation
    // would not allow chaining .parent after. This is just a
    // build check that we can call div().id().with_animation().child()
    #[test]
    fn test_animation_parent() {
        div()
            .id("id")
            //
            .with_animation(
                "animation",
                Animation::new(Duration::from_secs(1)),
                |el, _t| {
                    //
                    el
                },
            )
            .child(
                //
                div(),
            );
    }

    #[test]
    fn test_spring_animation_parent() {
        div()
            .id("id")
            .with_spring(
                "spring-animation",
                SpringAnimation::new(SpringConfig::new(100.0, 10.0, 1.0))
                    .to(px(10.0))
                    .from(px(0.0)),
                |element, value| element.left(value),
            )
            .child(div());
    }

    #[gpui::test]
    fn test_spring_animation_preserves_velocity_when_retargeted(cx: &mut TestAppContext) {
        let rendered_values = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let rendered_values = rendered_values.clone();
            move |_, _| SpringAnimationTestView {
                target: px(0.0),
                initial: None,
                playback: SpringPlayback::Running,
                rendered_values,
            }
        });
        cx.run_until_parked();
        assert_eq!(*rendered_values.borrow(), vec![px(0.0)]);

        window
            .update(cx, |view, _, cx| {
                view.target = px(100.0);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();

        cx.executor().advance_clock(Duration::from_millis(50));
        assert!(simulate_next_frame(&window, cx) > 0);
        let value_before_retargeting = *rendered_values.borrow().last().unwrap();
        assert!(value_before_retargeting > px(0.0));
        assert!(value_before_retargeting < px(100.0));

        window
            .update(cx, |view, _, cx| {
                view.target = px(0.0);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();

        cx.executor().advance_clock(Duration::from_millis(5));
        assert!(simulate_next_frame(&window, cx) > 0);
        let value_after_retargeting = *rendered_values.borrow().last().unwrap();
        assert!(value_after_retargeting > value_before_retargeting);
    }

    #[gpui::test]
    fn test_paused_spring_resumes_with_its_velocity(cx: &mut TestAppContext) {
        let rendered_values = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let rendered_values = rendered_values.clone();
            move |_, _| SpringAnimationTestView {
                target: px(0.0),
                initial: None,
                playback: SpringPlayback::Running,
                rendered_values,
            }
        });
        cx.run_until_parked();

        window
            .update(cx, |view, _, cx| {
                view.target = px(100.0);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(50));
        assert!(simulate_next_frame(&window, cx) > 0);

        window
            .update(cx, |view, _, cx| {
                view.target = px(0.0);
                view.playback = SpringPlayback::Paused;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        let paused_value = *rendered_values.borrow().last().unwrap();

        cx.executor().advance_clock(Duration::from_millis(500));
        assert!(simulate_next_frame(&window, cx) > 0);
        assert_eq!(*rendered_values.borrow().last().unwrap(), paused_value);
        assert_eq!(simulate_next_frame(&window, cx), 0);

        window
            .update(cx, |view, _, cx| {
                view.playback = SpringPlayback::Running;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(5));
        assert!(simulate_next_frame(&window, cx) > 0);
        assert!(*rendered_values.borrow().last().unwrap() > paused_value);
    }

    #[gpui::test]
    fn test_stopped_spring_resumes_without_velocity(cx: &mut TestAppContext) {
        let rendered_values = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let rendered_values = rendered_values.clone();
            move |_, _| SpringAnimationTestView {
                target: px(0.0),
                initial: None,
                playback: SpringPlayback::Running,
                rendered_values,
            }
        });
        cx.run_until_parked();

        window
            .update(cx, |view, _, cx| {
                view.target = px(1_000_000.0);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(50));
        assert!(simulate_next_frame(&window, cx) > 0);

        window
            .update(cx, |view, _, cx| {
                view.target = px(0.0);
                view.playback = SpringPlayback::Stopped;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        let stopped_value = *rendered_values.borrow().last().unwrap();

        cx.executor().advance_clock(Duration::from_millis(500));
        assert!(simulate_next_frame(&window, cx) > 0);
        assert_eq!(*rendered_values.borrow().last().unwrap(), stopped_value);
        assert_eq!(simulate_next_frame(&window, cx), 0);

        window
            .update(cx, |view, _, cx| {
                view.target = stopped_value;
                view.playback = SpringPlayback::Running;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(*rendered_values.borrow().last().unwrap(), stopped_value);
        assert_eq!(simulate_next_frame(&window, cx), 0);
    }

    #[gpui::test]
    fn test_cancelled_and_completed_springs_resolve_their_endpoints(cx: &mut TestAppContext) {
        let rendered_values = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let rendered_values = rendered_values.clone();
            move |_, _| SpringAnimationTestView {
                target: px(100.0),
                initial: Some(px(20.0)),
                playback: SpringPlayback::Running,
                rendered_values,
            }
        });
        cx.run_until_parked();
        assert_eq!(*rendered_values.borrow(), vec![px(20.0)]);

        cx.executor().advance_clock(Duration::from_millis(50));
        assert!(simulate_next_frame(&window, cx) > 0);
        assert!(*rendered_values.borrow().last().unwrap() > px(20.0));

        window
            .update(cx, |view, _, cx| {
                view.playback = SpringPlayback::Cancelled;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(*rendered_values.borrow().last().unwrap(), px(20.0));
        assert!(simulate_next_frame(&window, cx) > 0);
        assert_eq!(simulate_next_frame(&window, cx), 0);

        window
            .update(cx, |view, _, cx| {
                view.playback = SpringPlayback::Completed;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(*rendered_values.borrow().last().unwrap(), px(100.0));
        assert_eq!(simulate_next_frame(&window, cx), 0);
    }

    #[gpui::test]
    fn test_spring_animation_respects_reduced_motion(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let rendered_values = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.0), px(100.0)), {
            let rendered_values = rendered_values.clone();
            move |_, _| SpringAnimationTestView {
                target: px(100.0),
                initial: None,
                playback: SpringPlayback::Running,
                rendered_values,
            }
        });
        cx.run_until_parked();

        assert_eq!(*rendered_values.borrow(), vec![px(100.0)]);
        assert_eq!(simulate_next_frame(&window, cx), 0);
    }

    #[gpui::test]
    fn test_repeating_animation_schedules_animation_frames(cx: &mut TestAppContext) {
        let (rendered_deltas, window) = open_test_window(cx);

        assert_eq!(rendered_deltas.borrow().len(), 1);

        for expected_frames in 2..=3 {
            assert_eq!(simulate_next_frame(&window, cx), 1);
            assert_eq!(rendered_deltas.borrow().len(), expected_frames);
        }
    }

    #[gpui::test]
    fn test_max_fps_schedules_timer_driven_frames(cx: &mut TestAppContext) {
        let (rendered_deltas, window) = open_test_window_with_max_fps(cx, Some(10.0));

        // The test scheduler's clock jitters forward slightly on each poll,
        // so compare against expectations loosely.
        let assert_deltas_approx_eq = |expected: &[f32]| {
            let actual = rendered_deltas.borrow();
            assert_eq!(actual.len(), expected.len(), "deltas: {actual:?}");
            for (actual, expected) in actual.iter().zip(expected) {
                assert!(
                    (actual - expected).abs() < 1e-2,
                    "expected {expected}, got {actual}"
                );
            }
        };

        assert_deltas_approx_eq(&[0.0]);

        // No per-frame callback is scheduled; re-renders are timer-driven.
        assert_eq!(simulate_next_frame(&window, cx), 0);
        assert_deltas_approx_eq(&[0.0]);

        cx.executor().advance_clock(Duration::from_millis(105));
        cx.run_until_parked();
        assert_deltas_approx_eq(&[0.0, 0.105]);

        cx.executor().advance_clock(Duration::from_millis(105));
        cx.run_until_parked();
        assert_deltas_approx_eq(&[0.0, 0.105, 0.21]);
    }

    #[gpui::test]
    fn test_synced_animations_share_phase_across_elements(cx: &mut TestAppContext) {
        let first_deltas = Rc::new(RefCell::new(Vec::new()));
        let second_deltas = Rc::new(RefCell::new(Vec::new()));
        let window = cx.open_window(size(px(100.), px(100.)), {
            let first_deltas = first_deltas.clone();
            let second_deltas = second_deltas.clone();
            move |_, _| SyncedAnimationTestView {
                show_second: false,
                first_deltas,
                second_deltas,
            }
        });
        cx.run_until_parked();

        assert_eq!(*first_deltas.borrow(), vec![0.0]);

        cx.executor().advance_clock(Duration::from_millis(250));
        simulate_next_frame(&window, cx);
        assert_eq!(*first_deltas.borrow(), vec![0.0, 0.25]);

        // The second element mounts a quarter through the cycle, yet renders
        // the shared phase rather than starting at zero.
        window
            .update(cx, |view, _, cx| {
                view.show_second = true;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(250));
        simulate_next_frame(&window, cx);

        assert_eq!(*second_deltas.borrow().last().unwrap(), 0.5);
        assert_eq!(
            *first_deltas.borrow().last().unwrap(),
            *second_deltas.borrow().last().unwrap()
        );
        assert!(second_deltas.borrow().iter().all(|delta| *delta > 0.0));

        // The phase wraps around each full cycle.
        cx.executor().advance_clock(Duration::from_millis(2250));
        simulate_next_frame(&window, cx);
        assert_eq!(*first_deltas.borrow().last().unwrap(), 0.75);

        // Sub-second precision survives months of uptime: converting the raw
        // elapsed time to f32 would round 0.25 away entirely.
        cx.executor()
            .advance_clock(Duration::from_secs(300 * 24 * 60 * 60) + Duration::from_millis(500));
        simulate_next_frame(&window, cx);
        assert_eq!(*first_deltas.borrow().last().unwrap(), 0.25);
    }

    #[gpui::test]
    fn test_reduce_motion_renders_single_static_frame(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (rendered_deltas, window) = open_test_window(cx);

        assert_eq!(*rendered_deltas.borrow(), vec![0.0]);

        assert_eq!(simulate_next_frame(&window, cx), 0);
        assert_eq!(*rendered_deltas.borrow(), vec![0.0]);
    }

    struct TransitionTestView {
        ready: bool,
        leaving_since: Option<Instant>,
        clicks_beneath: Rc<RefCell<usize>>,
    }

    impl Render for TransitionTestView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let clicks_beneath = self.clicks_beneath.clone();
            div()
                .relative()
                .size_full()
                .child(
                    div()
                        .id("beneath")
                        .absolute()
                        .top(px(0.))
                        .left(px(0.))
                        .size(px(80.))
                        .on_click(move |_, _, _| *clicks_beneath.borrow_mut() += 1),
                )
                .child(
                    div().absolute().top(px(20.)).left(px(20.)).child(
                        transition(
                            "surface",
                            div().id("surface").occlude().size(px(40.)).bg(crate::red()),
                        )
                        .enter(Duration::from_millis(100))
                        .exit(Duration::from_millis(100))
                        .offset(crate::point(px(0.), px(-10.)))
                        .ready(self.ready)
                        .leaving_since(self.leaving_since),
                    ),
                )
        }
    }

    /// The red surface as painted: its top edge and its opacity.
    fn painted_surface(cx: &mut crate::VisualTestContext) -> Option<(f32, f32)> {
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
            window
                .painted_quads()
                .into_iter()
                .find(|quad| {
                    quad.background
                        .as_solid()
                        .is_some_and(|color| color.h == crate::red().h && color.s > 0.5)
                })
                .map(|quad| {
                    let color = quad.background.as_solid().unwrap_or_default();
                    (quad.bounds.origin.y.0 / window.scale_factor(), color.a)
                })
        })
    }

    fn open_transition(
        cx: &mut TestAppContext,
        ready: bool,
    ) -> (
        crate::Entity<TransitionTestView>,
        &mut crate::VisualTestContext,
        Rc<RefCell<usize>>,
    ) {
        let clicks_beneath = Rc::new(RefCell::new(0));
        let (view, cx) = cx.add_window_view({
            let clicks_beneath = clicks_beneath.clone();
            move |_, _| TransitionTestView {
                ready,
                leaving_since: None,
                clicks_beneath,
            }
        });
        (view, cx, clicks_beneath)
    }

    #[gpui::test]
    fn test_transition_fades_in_while_it_travels_into_place(cx: &mut TestAppContext) {
        let (_, cx, _) = open_transition(cx, true);
        assert!(
            painted_surface(cx).is_none_or(|(_, opacity)| opacity < 0.05),
            "the first frame starts transparent"
        );

        cx.executor().advance_clock(Duration::from_millis(30));
        let (top, opacity) = painted_surface(cx).expect("the surface is on its way in");
        assert!(
            opacity > 0.3 && opacity < 1. && top > 10. && top < 20.,
            "halfway it is partly there: top {top}, opacity {opacity}"
        );

        cx.executor().advance_clock(Duration::from_millis(100));
        assert_eq!(
            painted_surface(cx),
            Some((20., 1.)),
            "once the enter time has passed it is settled"
        );
    }

    #[gpui::test]
    fn test_transition_waits_until_its_content_is_ready(cx: &mut TestAppContext) {
        let (view, cx, _) = open_transition(cx, false);
        cx.executor().advance_clock(Duration::from_millis(500));
        assert!(
            painted_surface(cx).is_none_or(|(_, opacity)| opacity == 0.),
            "content that is still loading is not drawn"
        );

        view.update(cx, |view, cx| {
            view.ready = true;
            cx.notify();
        });
        let (_, opacity) = painted_surface(cx).unwrap_or((0., 0.));
        assert!(
            opacity < 0.05,
            "the time it waited does not count: it comes in from the start, not halfway: {opacity}"
        );
        cx.executor().advance_clock(Duration::from_millis(150));
        assert_eq!(painted_surface(cx), Some((20., 1.)));
    }

    #[gpui::test]
    fn test_a_leaving_transition_fades_out_and_lets_the_mouse_through(cx: &mut TestAppContext) {
        let (view, cx, clicks_beneath) = open_transition(cx, true);
        cx.executor().advance_clock(Duration::from_millis(200));
        assert_eq!(painted_surface(cx), Some((20., 1.)));

        cx.simulate_click(crate::point(px(30.), px(30.)), crate::Modifiers::none());
        assert_eq!(
            *clicks_beneath.borrow(),
            0,
            "a settled surface takes the clicks on it"
        );

        let now = cx.executor().now();
        view.update(cx, |view, cx| {
            view.leaving_since = Some(now);
            cx.notify();
        });
        cx.executor().advance_clock(Duration::from_millis(50));
        let (top, opacity) = painted_surface(cx).expect("the surface is fading out");
        assert!(
            opacity > 0. && opacity < 1. && top == 20.,
            "it fades out where it is: top {top}, opacity {opacity}"
        );
        cx.simulate_click(crate::point(px(30.), px(30.)), crate::Modifiers::none());
        assert_eq!(
            *clicks_beneath.borrow(),
            1,
            "a click on a surface that is going away reaches what lies beneath it"
        );

        cx.executor().advance_clock(Duration::from_millis(100));
        assert!(
            painted_surface(cx).is_none_or(|(_, opacity)| opacity == 0.),
            "once the exit time has passed nothing of it is drawn"
        );
    }

    #[gpui::test]
    fn test_transition_with_reduced_motion_appears_settled(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (_, cx, _) = open_transition(cx, true);
        assert_eq!(painted_surface(cx), Some((20., 1.)));
    }

    struct RedSquare;

    impl Render for RedSquare {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size(px(40.)).bg(crate::red())
        }
    }

    struct CachedContentView {
        content: crate::Entity<RedSquare>,
    }

    impl Render for CachedContentView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().absolute().top(px(20.)).left(px(20.)).child(
                transition(
                    "surface",
                    crate::AnyView::from(self.content.clone())
                        .cached(crate::StyleRefinement::default().size(px(40.))),
                )
                .enter(Duration::from_millis(100)),
            )
        }
    }

    #[gpui::test]
    fn test_a_cached_view_fades_in_with_its_surface(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, cx| CachedContentView {
            content: cx.new(|_| RedSquare),
        });
        let mut opacities = Vec::new();
        for _ in 0..3 {
            cx.executor().advance_clock(Duration::from_millis(25));
            opacities.push(
                cx.update(|window, cx| {
                    window.draw(cx).clear(cx);
                    window.painted_quads()
                })
                .into_iter()
                .find_map(|quad| {
                    quad.background
                        .as_solid()
                        .filter(|color| color.s > 0.5)
                        .map(|color| color.a)
                })
                .unwrap_or(0.),
            );
        }
        assert!(
            opacities.windows(2).all(|pair| pair[0] < pair[1]),
            "a view whose primitives were cached in an earlier frame still follows the fade: {opacities:?}"
        );
    }
}
