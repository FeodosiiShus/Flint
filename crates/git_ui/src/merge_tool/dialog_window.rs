mod blocker;
mod bounds;
#[cfg(test)]
mod tests;

use std::rc::Rc;

use anyhow::{Context as _, Result, anyhow};
use gpui::{
    AnyView, AnyWindowHandle, Bounds, Entity, FocusHandle, Focusable, Global, Size, Subscription,
    TitlebarOptions, WeakEntity, WindowBounds, WindowHandle, WindowId, WindowKind, WindowOptions,
    point, size,
};
use release_channel::ReleaseChannel;
use ui::prelude::*;
use util::ResultExt as _;
use workspace::{CloseWindow, Workspace};

use self::blocker::{block_workspace, release_workspace};
use self::bounds::{
    FrameRect, FrameSize, OwnerPlacement, PlannedBounds, ScreenSet, TrackedBounds,
    load_saved_bounds, plan_bounds,
};

pub(crate) const DIALOG_WINDOW_KEY_CONTEXT: &str = "DialogWindow";

const ESTIMATED_TITLE_BAR_HEIGHT: f32 = 28.;
const SIZE_TOLERANCE: f32 = 0.5;

pub(crate) struct DialogWindowSpec {
    pub title: SharedString,
    pub initial_size: Size<Pixels>,
    pub min_size: Size<Pixels>,
    pub bounds_key: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseRequest {
    TitleBarButton,
    Escape,
    CloseShortcut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseDecision {
    Close,
    Keep,
}

pub(crate) trait DialogWindowContent: Render + Focusable + Sized {
    fn close_requested(
        &mut self,
        request: CloseRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> CloseDecision;
}

#[derive(Clone)]
pub(crate) enum DialogOwner {
    Workspace {
        workspace: WeakEntity<Workspace>,
        window: AnyWindowHandle,
    },
    DialogWindow {
        window: AnyWindowHandle,
    },
}

#[derive(Clone, Copy)]
enum BlockingChange {
    Add,
    Remove,
}

impl DialogOwner {
    pub(crate) fn workspace(workspace: WeakEntity<Workspace>, window: AnyWindowHandle) -> Self {
        Self::Workspace { workspace, window }
    }

    pub(crate) fn dialog_window(window: AnyWindowHandle) -> Self {
        Self::DialogWindow { window }
    }

    fn window(&self) -> AnyWindowHandle {
        match self {
            Self::Workspace { window, .. } | Self::DialogWindow { window } => *window,
        }
    }

    fn placement(&self, cx: &mut App) -> Result<OwnerPlacement> {
        self.window().update(cx, |_, window, cx| OwnerPlacement {
            frame: FrameRect::from_bounds(window.bounds()),
            display: window.display(cx).map(|display| display.id()),
        })
    }

    fn project_scope(&self, cx: &App) -> Option<String> {
        match self {
            Self::Workspace { workspace, .. } => workspace
                .read_with(cx, |workspace, _| {
                    workspace.database_id().map(|id| i64::from(id).to_string())
                })
                .log_err()
                .flatten(),
            Self::DialogWindow { window } => window
                .read(cx, |shell: Entity<DialogWindowShell>, cx| {
                    shell.read(cx).tracked.project_scope().map(str::to_owned)
                })
                .log_err()
                .flatten(),
        }
    }

    fn block(&self, cx: &mut App) -> Result<()> {
        match self {
            Self::Workspace { workspace, window } => block_workspace(workspace, *window, cx),
            Self::DialogWindow { window } => {
                change_blocking_children(*window, BlockingChange::Add, cx)
            }
        }
    }

    fn unblock(&self, cx: &mut App) -> Result<()> {
        match self {
            Self::Workspace { workspace, window } => release_workspace(workspace, *window, cx),
            Self::DialogWindow { window } => {
                change_blocking_children(*window, BlockingChange::Remove, cx)
            }
        }
    }

    fn activate(&self, cx: &mut App) -> Result<()> {
        self.window()
            .update(cx, |_, window, _| window.activate_window())
    }
}

fn change_blocking_children(
    window: AnyWindowHandle,
    change: BlockingChange,
    cx: &mut App,
) -> Result<()> {
    window.update(cx, |root, _, cx| {
        let shell = root
            .downcast::<DialogWindowShell>()
            .map_err(|_| anyhow!("the owner window is not a dialog window"))?;
        shell.update(cx, |shell, cx| {
            shell.blocking_children = match change {
                BlockingChange::Add => shell.blocking_children + 1,
                BlockingChange::Remove => shell.blocking_children.saturating_sub(1),
            };
            cx.notify();
        });
        anyhow::Ok(())
    })?
}

struct OwnerActivationWatcher {
    _subscription: Subscription,
}

fn watch_owner_activation(
    owner: AnyWindowHandle,
    dialog: AnyWindowHandle,
    cx: &mut App,
) -> Option<Entity<OwnerActivationWatcher>> {
    let watcher = owner.update(cx, |_, owner_window, cx| {
        cx.new(|cx: &mut Context<OwnerActivationWatcher>| {
            let subscription =
                cx.observe_window_activation(owner_window, move |_, owner_window, cx| {
                    if owner_window.is_window_active() {
                        ignore_closed_window(
                            dialog
                                .update(cx, |_, dialog_window, _| dialog_window.activate_window()),
                            "re-activating a dialog window",
                        );
                    }
                });
            OwnerActivationWatcher {
                _subscription: subscription,
            }
        })
    });
    match watcher {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            log::warn!("could not watch the owner window of a dialog window: {error:#}");
            None
        }
    }
}

pub(crate) struct DialogWindow<V> {
    pub window: WindowHandle<DialogWindowShell>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub content: Entity<V>,
}

pub(crate) fn open_dialog_window<V: DialogWindowContent>(
    spec: DialogWindowSpec,
    owner: DialogOwner,
    cx: &mut App,
    build_content: impl FnOnce(&mut Window, &mut App) -> Entity<V> + 'static,
    on_opened: impl FnOnce(Result<DialogWindow<V>>, &mut App) + 'static,
) {
    cx.defer(move |cx| {
        let opened = open_dialog_window_now(spec, owner, cx, build_content);
        on_opened(opened, cx);
    });
}

fn open_dialog_window_now<V: DialogWindowContent>(
    spec: DialogWindowSpec,
    owner: DialogOwner,
    cx: &mut App,
    build_content: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Result<DialogWindow<V>> {
    let screens = ScreenSet::capture(cx);
    let owner_placement = owner
        .placement(cx)
        .context("the owner window of the dialog window is not available")?;
    let project_scope = owner.project_scope(cx);
    let saved = load_saved_bounds(spec.bounds_key, project_scope.as_deref(), cx);
    let title_bar_height = title_bar_height(cx);
    let minimum = FrameSize {
        width: spec.min_size.width.as_f32(),
        height: spec.min_size.height.as_f32() + title_bar_height,
    };
    let planned = plan_bounds(
        FrameSize::from_size(spec.initial_size),
        minimum,
        &saved,
        &owner_placement,
        &screens,
    );
    let options = window_options(&spec, &planned, title_bar_height, cx);
    let tracked = TrackedBounds::new(spec.bounds_key, project_scope, &planned);

    owner
        .block(cx)
        .context("could not block the owner window of the dialog window")?;

    let mut content_slot = None;
    let opened = cx.open_window(options, |window, cx| {
        let content = build_content(window, cx);
        content_slot = Some(content.clone());
        cx.new(|cx| {
            DialogWindowShell::new(content, owner.clone(), tracked, planned.frame, window, cx)
        })
    });

    match opened {
        Ok(window) => {
            let content = content_slot.context("the dialog window content was not built")?;
            Ok(DialogWindow { window, content })
        }
        Err(error) => {
            ignore_closed_window(owner.unblock(cx), "releasing the owner of a failed dialog");
            Err(error)
        }
    }
}

fn window_options(
    spec: &DialogWindowSpec,
    planned: &PlannedBounds,
    title_bar_height: f32,
    cx: &App,
) -> WindowOptions {
    let content_size = size(
        px(planned.frame.width),
        px((planned.frame.height - title_bar_height).max(1.)),
    );
    WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(spec.title.clone()),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(planned.frame.x), px(planned.frame.y)),
            content_size,
        ))),
        focus: false,
        show: false,
        kind: WindowKind::Normal,
        is_movable: true,
        is_resizable: true,
        is_minimizable: false,
        display_id: planned.display,
        window_background: cx.theme().window_background_appearance(),
        app_id: ReleaseChannel::try_global(cx).map(|channel| channel.app_id().to_owned()),
        window_min_size: Some(spec.min_size),
        ..Default::default()
    }
}

struct MeasuredTitleBar {
    height: f32,
}

impl Global for MeasuredTitleBar {}

fn title_bar_height(cx: &App) -> f32 {
    cx.try_global::<MeasuredTitleBar>()
        .map_or(ESTIMATED_TITLE_BAR_HEIGHT, |measured| measured.height)
}

fn align_frame_size(window: &mut Window, desired: FrameSize, cx: &mut App) -> bool {
    if window.display(cx).is_none() {
        return false;
    }
    let frame = window.bounds().size;
    let content = window.viewport_size();
    let chrome_width = (frame.width - content.width).as_f32().max(0.);
    let chrome_height = (frame.height - content.height).as_f32().max(0.);
    cx.set_global(MeasuredTitleBar {
        height: chrome_height,
    });
    let target_width = (desired.width - chrome_width).max(1.);
    let target_height = (desired.height - chrome_height).max(1.);
    let needs_resize = (content.width.as_f32() - target_width).abs() >= SIZE_TOLERANCE
        || (content.height.as_f32() - target_height).abs() >= SIZE_TOLERANCE;
    if needs_resize {
        window.resize(size(px(target_width), px(target_height)));
    }
    true
}

pub(crate) fn close_dialog_window(window: &mut Window) {
    window.remove_window();
}

fn ignore_closed_window(result: Result<()>, action: &str) {
    if let Err(error) = result {
        log::debug!("{action}: {error:#}");
    }
}

type CloseHandler = Rc<dyn Fn(CloseRequest, &mut Window, &mut App) -> CloseDecision>;

pub(crate) struct DialogWindowShell {
    content: AnyView,
    content_focus: FocusHandle,
    close_handler: CloseHandler,
    owner: Option<DialogOwner>,
    blocking_children: usize,
    tracked: TrackedBounds,
    pending_alignment: Option<FrameSize>,
    _subscriptions: Vec<Subscription>,
    _owner_activation: Option<Entity<OwnerActivationWatcher>>,
}

impl DialogWindowShell {
    fn new<V: DialogWindowContent>(
        content: Entity<V>,
        owner: DialogOwner,
        mut tracked: TrackedBounds,
        frame: FrameRect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let desired_size = frame.size();
        let pending_alignment =
            (!align_frame_size(window, desired_size, cx)).then_some(desired_size);
        tracked.remember(window, cx);
        let content_focus = content.focus_handle(cx);

        let weak_content = content.downgrade();
        let close_handler: CloseHandler = Rc::new(
            move |request: CloseRequest, window: &mut Window, cx: &mut App| {
                weak_content
                    .update(cx, |content, cx| {
                        content.close_requested(request, window, cx)
                    })
                    .unwrap_or(CloseDecision::Close)
            },
        );

        let weak_shell = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            weak_shell
                .update(cx, |shell, cx| {
                    shell.decide(CloseRequest::TitleBarButton, window, cx) == CloseDecision::Close
                })
                .unwrap_or(true)
        });

        let own_window = window.window_handle();
        let owner_window = owner.window();
        let subscriptions = vec![
            cx.observe_window_bounds(window, |shell, window, cx| {
                shell.align_pending_frame(window, cx);
                shell.tracked.remember(window, cx);
            }),
            cx.on_window_closed(move |cx: &mut App, closed: WindowId| {
                if closed == owner_window.window_id() {
                    cx.defer(move |cx| {
                        ignore_closed_window(
                            own_window.update(cx, |_, window, _| window.remove_window()),
                            "closing a dialog window whose owner window closed",
                        );
                    });
                }
            }),
            cx.on_focus_lost(window, |shell, window, cx| {
                shell.focus_content(window, cx);
            }),
        ];
        let owner_activation = watch_owner_activation(owner_window, own_window, cx);

        cx.on_release(|shell, cx| shell.finish(cx)).detach();

        window.focus(&content_focus, cx);
        window.activate_window();

        Self {
            content: content.into(),
            content_focus,
            close_handler,
            owner: Some(owner),
            blocking_children: 0,
            tracked,
            pending_alignment,
            _subscriptions: subscriptions,
            _owner_activation: owner_activation,
        }
    }

    fn focus_content(&mut self, window: &mut Window, cx: &mut App) {
        window.focus(&self.content_focus, cx);
    }

    fn align_pending_frame(&mut self, window: &mut Window, cx: &mut App) {
        if let Some(desired) = self.pending_alignment
            && align_frame_size(window, desired, cx)
        {
            self.pending_alignment = None;
        }
    }

    fn decide(
        &mut self,
        request: CloseRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> CloseDecision {
        if self.blocking_children > 0 {
            return CloseDecision::Keep;
        }
        let app: &mut App = cx;
        (self.close_handler)(request, window, app)
    }

    pub(crate) fn close_from_shortcut(
        &mut self,
        request: CloseRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.decide(request, window, cx) == CloseDecision::Close {
            close_dialog_window(window);
        }
    }

    fn finish(&mut self, cx: &mut App) {
        self.tracked.save(cx);
        if let Some(owner) = self.owner.take() {
            cx.defer(move |cx| {
                ignore_closed_window(owner.unblock(cx), "releasing the owner of a dialog window");
                ignore_closed_window(
                    owner.activate(cx),
                    "returning activation to the owner of a dialog window",
                );
            });
        }
    }
}

impl Render for DialogWindowShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui_font = theme_settings::setup_ui_font(window, cx);
        div()
            .flex()
            .flex_col()
            .relative()
            .size_full()
            .key_context(DIALOG_WINDOW_KEY_CONTEXT)
            .on_action(cx.listener(|shell, _: &menu::Cancel, window, cx| {
                shell.close_from_shortcut(CloseRequest::Escape, window, cx);
            }))
            .on_action(cx.listener(|shell, _: &CloseWindow, window, cx| {
                shell.close_from_shortcut(CloseRequest::CloseShortcut, window, cx);
            }))
            .font(ui_font)
            .text_color(cx.theme().colors().text)
            .bg(cx.theme().colors().elevated_surface_background)
            .child(self.content.clone())
            .when(self.blocking_children > 0, |this| {
                this.child(div().absolute().top_0().left_0().size_full().occlude())
            })
    }
}
