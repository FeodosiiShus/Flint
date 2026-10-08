use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    Action, AnyWindowHandle, Bounds, DismissEvent, DisplayId, Entity, EventEmitter, FocusHandle,
    Focusable, TestAppContext, VisualTestContext, WindowHandle, WindowKind, point, px, size,
};
use project::{FakeFs, Project};
use settings::SettingsStore;
use ui::prelude::*;
use workspace::{CloseWindow, DismissDecision, ModalView, MultiWorkspace, Workspace};

use super::blocker::WorkspaceBlocker;
use super::bounds::{
    BoundsToSave, FrameRect, FrameSize, OwnerPlacement, PlannedBounds, SavedBounds, SavedLocation,
    SavedSize, ScreenGeometry, ScreenSet, bounds_to_save, fit_to_screen, load_saved_bounds,
    plan_bounds, save_bounds,
};
use super::{
    CloseDecision, CloseRequest, DialogOwner, DialogWindow, DialogWindowContent, DialogWindowShell,
    DialogWindowSpec, open_dialog_window, window_options,
};

const INITIAL: FrameSize = FrameSize {
    width: 874.,
    height: 505.,
};
const MINIMUM: FrameSize = FrameSize {
    width: 574.,
    height: 268.,
};

fn frame(x: f32, y: f32, width: f32, height: f32) -> FrameRect {
    FrameRect {
        x,
        y,
        width,
        height,
    }
}

fn screen(id: u64, uuid: &str, visible: FrameRect) -> ScreenGeometry {
    ScreenGeometry {
        id: DisplayId::new(id),
        uuid: Some(uuid.to_string()),
        visible,
    }
}

fn single_screen(visible: FrameRect) -> ScreenSet {
    ScreenSet {
        screens: vec![screen(1, "main", visible)],
        primary: Some(0),
    }
}

fn two_screens() -> ScreenSet {
    ScreenSet {
        screens: vec![
            screen(1, "first", frame(0., 0., 1920., 1080.)),
            screen(2, "second", frame(0., 0., 1440., 900.)),
        ],
        primary: Some(0),
    }
}

fn owner_on(display: u64, owner_frame: FrameRect) -> OwnerPlacement {
    OwnerPlacement {
        frame: owner_frame,
        display: Some(DisplayId::new(display)),
    }
}

fn saved_at(x: i32, y: i32, display: &str) -> SavedLocation {
    SavedLocation {
        x,
        y,
        display: Some(display.to_string()),
    }
}

#[test]
fn fit_to_screen_moves_an_overflowing_frame_back_inside_before_cropping_it() {
    let screen_bounds = frame(0., 25., 1512., 924.);

    assert_eq!(
        fit_to_screen(frame(1500., 100., 600., 400.), screen_bounds),
        frame(912., 100., 600., 400.)
    );
    assert_eq!(
        fit_to_screen(frame(-50., -20., 2000., 1200.), screen_bounds),
        frame(0., 25., 1512., 924.)
    );
    assert_eq!(
        fit_to_screen(frame(300., 200., 600., 400.), screen_bounds),
        frame(300., 200., 600., 400.)
    );
}

#[test]
fn a_dialog_without_saved_bounds_is_centered_on_its_owner() {
    let screens = single_screen(frame(0., 0., 1512., 982.));
    let owner = owner_on(1, frame(0., 33., 1512., 949.));

    let planned = plan_bounds(INITIAL, MINIMUM, &SavedBounds::default(), &owner, &screens);

    assert_eq!(planned.frame, frame(319., 255., 874., 505.));
    assert_eq!(planned.display, Some(DisplayId::new(1)));
    assert_eq!(
        planned.size_reference,
        SavedSize {
            width: 874,
            height: 505
        }
    );
}

#[test]
fn a_dialog_larger_than_its_owner_is_centered_by_truncating_the_half_difference_toward_zero() {
    let screens = single_screen(frame(0., 0., 1512., 982.));
    let conflicts_dialog = owner_on(1, frame(319., 255., 874., 505.));
    let merge_window = FrameSize {
        width: 1000.,
        height: 700.,
    };

    let planned = plan_bounds(
        merge_window,
        MINIMUM,
        &SavedBounds::default(),
        &conflicts_dialog,
        &screens,
    );

    assert_eq!(planned.frame, frame(256., 158., 1000., 700.));
}

#[test]
fn a_dialog_centered_on_its_owner_is_pushed_back_inside_the_screen() {
    let screens = single_screen(frame(0., 25., 1440., 875.));
    let owner = owner_on(1, frame(0., 25., 1440., 875.));
    let merge_window = FrameSize {
        width: 1000.,
        height: 900.,
    };

    let planned = plan_bounds(
        merge_window,
        MINIMUM,
        &SavedBounds::default(),
        &owner,
        &screens,
    );

    assert_eq!(planned.frame, frame(220., 25., 1000., 875.));
}

#[test]
fn saved_location_and_size_win_over_the_owner_and_the_initial_size() {
    let screens = single_screen(frame(0., 0., 1920., 1080.));
    let owner = owner_on(1, frame(0., 0., 1920., 1080.));
    let saved = SavedBounds {
        location: Some(saved_at(100, 120, "main")),
        size: Some(SavedSize {
            width: 900,
            height: 600,
        }),
    };

    let planned = plan_bounds(INITIAL, MINIMUM, &saved, &owner, &screens);

    assert_eq!(planned.frame, frame(100., 120., 900., 600.));
    assert_eq!(
        planned.size_reference,
        SavedSize {
            width: 900,
            height: 600
        }
    );
}

#[test]
fn a_saved_location_alone_keeps_the_initial_size() {
    let screens = single_screen(frame(0., 0., 1920., 1080.));
    let owner = owner_on(1, frame(0., 0., 1920., 1080.));
    let saved = SavedBounds {
        location: Some(saved_at(40, 60, "main")),
        size: None,
    };

    let planned = plan_bounds(INITIAL, MINIMUM, &saved, &owner, &screens);

    assert_eq!(planned.frame, frame(40., 60., 874., 505.));
}

#[test]
fn a_saved_location_on_a_missing_display_falls_back_to_the_owner_display_and_is_clamped() {
    let screens = two_screens();
    let owner = owner_on(2, frame(0., 0., 1440., 900.));
    let saved = SavedBounds {
        location: Some(saved_at(3000, 100, "gone")),
        size: None,
    };

    let planned = plan_bounds(INITIAL, MINIMUM, &saved, &owner, &screens);

    assert_eq!(planned.frame, frame(566., 100., 874., 505.));
    assert_eq!(planned.display, Some(DisplayId::new(2)));
}

#[test]
fn the_saved_display_wins_over_the_owner_display() {
    let screens = two_screens();
    let owner = owner_on(1, frame(0., 0., 1920., 1080.));
    let saved = SavedBounds {
        location: Some(saved_at(1500, 100, "second")),
        size: None,
    };

    let planned = plan_bounds(INITIAL, MINIMUM, &saved, &owner, &screens);

    assert_eq!(planned.frame, frame(566., 100., 874., 505.));
    assert_eq!(planned.display, Some(DisplayId::new(2)));
}

#[test]
fn a_saved_size_below_the_minimum_is_raised_to_the_minimum() {
    let screens = single_screen(frame(0., 0., 1920., 1080.));
    let owner = owner_on(1, frame(0., 0., 1920., 1080.));
    let saved = SavedBounds {
        location: None,
        size: Some(SavedSize {
            width: 100,
            height: 100,
        }),
    };

    let planned = plan_bounds(INITIAL, MINIMUM, &saved, &owner, &screens);

    assert_eq!(planned.frame, frame(673., 406., 574., 268.));
}

#[test]
fn the_location_is_always_saved_and_the_size_only_when_it_differs_from_the_reference() {
    let reference = SavedSize {
        width: 874,
        height: 505,
    };

    assert_eq!(
        bounds_to_save(
            frame(319.4, 254.6, 874.3, 504.8),
            Some("main".to_string()),
            reference
        ),
        BoundsToSave {
            location: saved_at(319, 255, "main"),
            size: None,
        }
    );
    assert_eq!(
        bounds_to_save(frame(10., 20., 900., 505.), None, reference),
        BoundsToSave {
            location: SavedLocation {
                x: 10,
                y: 20,
                display: None
            },
            size: Some(SavedSize {
                width: 900,
                height: 505
            }),
        }
    );
}

#[test]
fn returning_to_the_default_size_replaces_a_previously_saved_size() {
    let saved_reference = SavedSize {
        width: 900,
        height: 600,
    };

    assert_eq!(
        bounds_to_save(frame(0., 0., 900., 600.), None, saved_reference).size,
        None
    );
    assert_eq!(
        bounds_to_save(frame(0., 0., 874., 505.), None, saved_reference).size,
        Some(SavedSize {
            width: 874,
            height: 505
        })
    );
}

#[test]
fn saved_bounds_round_trip_and_malformed_values_are_rejected() {
    let location = saved_at(-12, 40, "uuid-1");
    assert_eq!(SavedLocation::parse(&location.serialize()), Some(location));
    let location_without_display = SavedLocation {
        x: 5,
        y: 6,
        display: None,
    };
    assert_eq!(
        SavedLocation::parse(&location_without_display.serialize()),
        Some(location_without_display)
    );
    assert_eq!(SavedLocation::parse("10"), None);
    assert_eq!(SavedLocation::parse("a b"), None);
    assert_eq!(SavedLocation::parse("1 2 uuid extra"), None);

    let saved_size = SavedSize {
        width: 1000,
        height: 700,
    };
    assert_eq!(SavedSize::parse(&saved_size.serialize()), Some(saved_size));
    assert_eq!(SavedSize::parse("0 5"), None);
    assert_eq!(SavedSize::parse("10"), None);
    assert_eq!(SavedSize::parse("10 20 30"), None);
}

struct Probe {
    focus_handle: FocusHandle,
    controls: ProbeControls,
}

#[derive(Clone)]
struct ProbeControls {
    requests: Rc<RefCell<Vec<CloseRequest>>>,
    decision: Rc<Cell<CloseDecision>>,
}

impl ProbeControls {
    fn new() -> Self {
        Self {
            requests: Rc::default(),
            decision: Rc::new(Cell::new(CloseDecision::Close)),
        }
    }

    fn requests(&self) -> Vec<CloseRequest> {
        self.requests.borrow().clone()
    }
}

impl Focusable for Probe {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Probe {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().track_focus(&self.focus_handle)
    }
}

impl DialogWindowContent for Probe {
    fn close_requested(
        &mut self,
        request: CloseRequest,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> CloseDecision {
        self.controls.requests.borrow_mut().push(request);
        self.controls.decision.get()
    }
}

struct OtherModal {
    focus_handle: FocusHandle,
    dismissible: bool,
}

impl OtherModal {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            dismissible: true,
        }
    }

    fn stubborn(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            dismissible: false,
        }
    }
}

impl EventEmitter<DismissEvent> for OtherModal {}

impl Focusable for OtherModal {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for OtherModal {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus_handle)
    }
}

impl ModalView for OtherModal {
    fn on_before_dismiss(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> DismissDecision {
        DismissDecision::Dismiss(self.dismissible)
    }
}

fn init_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
        cx.set_global(db::AppDatabase::test_new());
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        editor::init(cx);
    });
}

fn spec(bounds_key: &'static str) -> DialogWindowSpec {
    DialogWindowSpec {
        title: SharedString::new_static("Conflicts"),
        initial_size: size(px(600.), px(420.)),
        min_size: size(px(300.), px(200.)),
        bounds_key,
    }
}

async fn open_owner(cx: &mut TestAppContext) -> (WindowHandle<MultiWorkspace>, Entity<Workspace>) {
    let fs = FakeFs::new(cx.executor());
    let project = Project::test(fs, [], cx).await;
    let window = cx.add_window(|window, cx| MultiWorkspace::test_new(project, window, cx));
    let workspace = window
        .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
        .expect("the owner window is open");
    (window, workspace)
}

fn workspace_owner(
    workspace: &Entity<Workspace>,
    window: WindowHandle<MultiWorkspace>,
) -> DialogOwner {
    DialogOwner::workspace(workspace.downgrade(), window.into())
}

fn try_open_probe(
    cx: &mut TestAppContext,
    owner: DialogOwner,
    bounds_key: &'static str,
    controls: &ProbeControls,
) -> anyhow::Result<DialogWindow<Probe>> {
    let opened: Rc<RefCell<Option<anyhow::Result<DialogWindow<Probe>>>>> = Rc::default();
    let slot = opened.clone();
    let probe_controls = controls.clone();
    cx.update(|cx| {
        open_dialog_window(
            spec(bounds_key),
            owner,
            cx,
            move |_, cx| {
                cx.new(|cx| Probe {
                    focus_handle: cx.focus_handle(),
                    controls: probe_controls,
                })
            },
            move |result, _| {
                *slot.borrow_mut() = Some(result);
            },
        );
    });
    cx.run_until_parked();
    opened.take().expect("the dialog window finished opening")
}

fn open_probe(
    cx: &mut TestAppContext,
    owner: DialogOwner,
    bounds_key: &'static str,
    controls: &ProbeControls,
) -> DialogWindow<Probe> {
    try_open_probe(cx, owner, bounds_key, controls).expect("the dialog window opened")
}

fn dispatch_in_dialog(
    window: WindowHandle<DialogWindowShell>,
    action: impl Action,
    cx: &mut TestAppContext,
) {
    let mut dialog_cx = VisualTestContext::from_window(window.into(), cx);
    dialog_cx.dispatch_action(action);
    dialog_cx.run_until_parked();
}

fn workspace_is_blocked(workspace: &Entity<Workspace>, cx: &TestAppContext) -> bool {
    workspace.read_with(cx, |workspace, cx| {
        workspace.active_modal::<WorkspaceBlocker>(cx).is_some()
    })
}

fn window_is_open(window: impl Into<AnyWindowHandle>, cx: &TestAppContext) -> bool {
    let window = window.into();
    cx.windows().contains(&window)
}

fn frame_of(window: WindowHandle<DialogWindowShell>, cx: &mut TestAppContext) -> Bounds<Pixels> {
    window
        .update(cx, |_, window, _| window.bounds())
        .expect("the dialog window is open")
}

fn blocking_children(window: WindowHandle<DialogWindowShell>, cx: &TestAppContext) -> usize {
    window
        .read_with(cx, |shell, _| shell.blocking_children)
        .expect("the dialog window is open")
}

fn active_window(cx: &TestAppContext) -> Option<AnyWindowHandle> {
    cx.update(|cx| cx.active_window())
}

#[gpui::test]
fn dialog_window_options_describe_a_titled_resizable_window_that_cannot_be_minimized(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let planned = PlannedBounds {
        frame: frame(10., 20., 600., 420.),
        display: None,
        size_reference: SavedSize {
            width: 600,
            height: 420,
        },
    };

    let options =
        cx.update(|cx| window_options(&spec("merge_tool_test_options"), &planned, 28., cx));

    let titlebar = options
        .titlebar
        .as_ref()
        .expect("the window has a titlebar");
    assert_eq!(titlebar.title, Some(SharedString::new_static("Conflicts")));
    assert!(!titlebar.appears_transparent);
    assert!(options.is_resizable);
    assert!(options.is_movable);
    assert!(!options.is_minimizable);
    assert_eq!(options.kind, WindowKind::Normal);
    assert_eq!(options.window_min_size, Some(size(px(300.), px(200.))));
    assert_eq!(
        options.window_bounds,
        Some(gpui::WindowBounds::Windowed(Bounds::new(
            point(px(10.), px(20.)),
            size(px(600.), px(392.))
        )))
    );
}

#[gpui::test]
async fn escape_closes_the_dialog_window_through_its_close_handler_and_releases_the_workspace(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let (owner_window, workspace) = open_owner(cx).await;
    let controls = ProbeControls::new();
    let dialog = open_probe(
        cx,
        workspace_owner(&workspace, owner_window),
        "merge_tool_test_escape",
        &controls,
    );
    assert!(window_is_open(dialog.window, cx));
    assert!(workspace_is_blocked(&workspace, cx));

    dispatch_in_dialog(dialog.window, menu::Cancel, cx);

    assert_eq!(controls.requests(), vec![CloseRequest::Escape]);
    assert!(!window_is_open(dialog.window, cx));
    assert!(!workspace_is_blocked(&workspace, cx));
    assert_eq!(active_window(cx), Some(AnyWindowHandle::from(owner_window)));
}

#[gpui::test]
async fn the_close_window_shortcut_reaches_the_same_close_handler(cx: &mut TestAppContext) {
    init_test(cx);
    let (owner_window, workspace) = open_owner(cx).await;
    let controls = ProbeControls::new();
    let dialog = open_probe(
        cx,
        workspace_owner(&workspace, owner_window),
        "merge_tool_test_close_window",
        &controls,
    );

    dispatch_in_dialog(dialog.window, CloseWindow, cx);

    assert_eq!(controls.requests(), vec![CloseRequest::CloseShortcut]);
    assert!(!window_is_open(dialog.window, cx));
    assert!(window_is_open(owner_window, cx));
    assert!(!workspace_is_blocked(&workspace, cx));
}

#[gpui::test]
async fn a_vetoed_close_keeps_the_window_and_the_workspace_blocker(cx: &mut TestAppContext) {
    init_test(cx);
    let (owner_window, workspace) = open_owner(cx).await;
    let controls = ProbeControls::new();
    let dialog = open_probe(
        cx,
        workspace_owner(&workspace, owner_window),
        "merge_tool_test_veto",
        &controls,
    );
    controls.decision.set(CloseDecision::Keep);

    dispatch_in_dialog(dialog.window, menu::Cancel, cx);
    dispatch_in_dialog(dialog.window, CloseWindow, cx);

    assert_eq!(
        controls.requests(),
        vec![CloseRequest::Escape, CloseRequest::CloseShortcut]
    );
    assert!(window_is_open(dialog.window, cx));
    assert!(workspace_is_blocked(&workspace, cx));

    controls.decision.set(CloseDecision::Close);
    dispatch_in_dialog(dialog.window, menu::Cancel, cx);

    assert!(!window_is_open(dialog.window, cx));
    assert!(!workspace_is_blocked(&workspace, cx));
}

#[gpui::test]
async fn a_blocked_workspace_refuses_other_modals_until_the_dialog_window_closes(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let (owner_window, workspace) = open_owner(cx).await;
    let controls = ProbeControls::new();
    let dialog = open_probe(
        cx,
        workspace_owner(&workspace, owner_window),
        "merge_tool_test_other_modals",
        &controls,
    );
    let mut owner_cx = VisualTestContext::from_window(owner_window.into(), cx);

    workspace.update_in(&mut owner_cx, |workspace, window, cx| {
        workspace.toggle_modal(window, cx, |_, cx| OtherModal::new(cx));
        assert!(workspace.active_modal::<OtherModal>(cx).is_none());
        assert!(workspace.active_modal::<WorkspaceBlocker>(cx).is_some());
        assert!(!workspace.hide_modal(window, cx));
        assert!(workspace.active_modal::<WorkspaceBlocker>(cx).is_some());
    });

    dispatch_in_dialog(dialog.window, menu::Cancel, cx);

    workspace.update_in(&mut owner_cx, |workspace, window, cx| {
        assert!(workspace.active_modal::<WorkspaceBlocker>(cx).is_none());
        workspace.toggle_modal(window, cx, |_, cx| OtherModal::new(cx));
        assert!(workspace.active_modal::<OtherModal>(cx).is_some());
    });
}

#[gpui::test]
async fn closing_the_owner_window_closes_the_dialog_window(cx: &mut TestAppContext) {
    init_test(cx);
    let (owner_window, workspace) = open_owner(cx).await;
    let controls = ProbeControls::new();
    let dialog = open_probe(
        cx,
        workspace_owner(&workspace, owner_window),
        "merge_tool_test_owner_closed",
        &controls,
    );

    owner_window
        .update(cx, |_, window, _| window.remove_window())
        .expect("the owner window is open");
    cx.run_until_parked();

    assert!(!window_is_open(dialog.window, cx));
    assert!(!window_is_open(owner_window, cx));
    assert!(controls.requests().is_empty());
}

#[gpui::test]
async fn a_nested_dialog_window_blocks_its_owner_dialog_window_and_hands_activation_back(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let (owner_window, workspace) = open_owner(cx).await;
    let first_controls = ProbeControls::new();
    let first = open_probe(
        cx,
        workspace_owner(&workspace, owner_window),
        "merge_tool_test_nested_first",
        &first_controls,
    );
    let second_controls = ProbeControls::new();
    let second = open_probe(
        cx,
        DialogOwner::dialog_window(first.window.into()),
        "merge_tool_test_nested_second",
        &second_controls,
    );
    assert_eq!(blocking_children(first.window, cx), 1);

    dispatch_in_dialog(first.window, menu::Cancel, cx);
    assert!(first_controls.requests().is_empty());
    assert!(window_is_open(first.window, cx));

    dispatch_in_dialog(second.window, menu::Cancel, cx);
    assert_eq!(second_controls.requests(), vec![CloseRequest::Escape]);
    assert!(!window_is_open(second.window, cx));
    assert_eq!(blocking_children(first.window, cx), 0);
    assert_eq!(active_window(cx), Some(AnyWindowHandle::from(first.window)));
    assert!(workspace_is_blocked(&workspace, cx));

    dispatch_in_dialog(first.window, menu::Cancel, cx);
    assert_eq!(first_controls.requests(), vec![CloseRequest::Escape]);
    assert!(!workspace_is_blocked(&workspace, cx));
}

#[gpui::test]
async fn the_location_is_saved_on_every_close_the_size_only_when_changed_and_both_are_restored(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let (owner_window, workspace) = open_owner(cx).await;
    let bounds_key = "merge_tool_test_bounds";
    let controls = ProbeControls::new();

    let first = open_probe(
        cx,
        workspace_owner(&workspace, owner_window),
        bounds_key,
        &controls,
    );
    let first_frame = frame_of(first.window, cx);
    assert_eq!(first_frame.size, size(px(600.), px(420.)));
    assert_eq!(first_frame.origin, point(px(660.), px(330.)));
    dispatch_in_dialog(first.window, menu::Cancel, cx);

    let saved = cx.update(|cx| load_saved_bounds(bounds_key, None, cx));
    assert_eq!(
        saved
            .location
            .as_ref()
            .map(|location| (location.x, location.y)),
        Some((660, 330))
    );
    assert_eq!(saved.size, None);

    let second = open_probe(
        cx,
        workspace_owner(&workspace, owner_window),
        bounds_key,
        &controls,
    );
    assert_eq!(frame_of(second.window, cx), first_frame);
    cx.simulate_window_resize(second.window.into(), size(px(900.), px(650.)));
    dispatch_in_dialog(second.window, menu::Cancel, cx);

    let saved = cx.update(|cx| load_saved_bounds(bounds_key, None, cx));
    assert_eq!(
        saved.size,
        Some(SavedSize {
            width: 900,
            height: 650
        })
    );

    let third = open_probe(
        cx,
        workspace_owner(&workspace, owner_window),
        bounds_key,
        &controls,
    );
    let third_frame = frame_of(third.window, cx);
    assert_eq!(third_frame.size, size(px(900.), px(650.)));
    assert_eq!(third_frame.origin, first_frame.origin);
}

#[gpui::test]
async fn a_dialog_close_request_from_the_title_bar_button_honours_the_content_veto(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let (owner_window, workspace) = open_owner(cx).await;
    let controls = ProbeControls::new();
    let dialog = open_probe(
        cx,
        workspace_owner(&workspace, owner_window),
        "merge_tool_test_title_bar",
        &controls,
    );
    let mut dialog_cx = VisualTestContext::from_window(dialog.window.into(), cx);

    controls.decision.set(CloseDecision::Keep);
    assert!(!dialog_cx.simulate_close());
    controls.decision.set(CloseDecision::Close);
    assert!(dialog_cx.simulate_close());

    assert_eq!(
        controls.requests(),
        vec![CloseRequest::TitleBarButton, CloseRequest::TitleBarButton]
    );
}

#[gpui::test]
async fn closing_the_owner_window_closes_every_dialog_window_nested_below_it(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let (owner_window, workspace) = open_owner(cx).await;
    let first = open_probe(
        cx,
        workspace_owner(&workspace, owner_window),
        "merge_tool_test_chain_first",
        &ProbeControls::new(),
    );
    let second = open_probe(
        cx,
        DialogOwner::dialog_window(first.window.into()),
        "merge_tool_test_chain_second",
        &ProbeControls::new(),
    );

    owner_window
        .update(cx, |_, window, _| window.remove_window())
        .expect("the owner window is open");
    cx.run_until_parked();

    assert!(!window_is_open(second.window, cx));
    assert!(!window_is_open(first.window, cx));
    assert!(!window_is_open(owner_window, cx));
}

#[gpui::test]
async fn a_workspace_whose_modal_refuses_to_close_is_not_blocked_and_no_dialog_window_opens(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let (owner_window, workspace) = open_owner(cx).await;
    let mut owner_cx = VisualTestContext::from_window(owner_window.into(), cx);
    workspace.update_in(&mut owner_cx, |workspace, window, cx| {
        workspace.toggle_modal(window, cx, |_, cx| OtherModal::stubborn(cx));
    });
    let window_count = cx.windows().len();

    let opened = try_open_probe(
        cx,
        workspace_owner(&workspace, owner_window),
        "merge_tool_test_stubborn",
        &ProbeControls::new(),
    );

    assert!(opened.is_err());
    assert_eq!(cx.windows().len(), window_count);
    assert!(!workspace_is_blocked(&workspace, cx));
    workspace.read_with(cx, |workspace, cx| {
        assert!(workspace.active_modal::<OtherModal>(cx).is_some());
    });
}

#[gpui::test]
fn saved_bounds_are_kept_separately_for_every_project(cx: &mut TestAppContext) {
    init_test(cx);
    let bounds = BoundsToSave {
        location: saved_at(30, 40, "main"),
        size: Some(SavedSize {
            width: 700,
            height: 500,
        }),
    };

    cx.update(|cx| save_bounds("merge_tool_test_scope", Some("12"), bounds.clone(), cx));
    cx.run_until_parked();

    let same_project = cx.update(|cx| load_saved_bounds("merge_tool_test_scope", Some("12"), cx));
    let other_project = cx.update(|cx| load_saved_bounds("merge_tool_test_scope", Some("13"), cx));
    let without_project = cx.update(|cx| load_saved_bounds("merge_tool_test_scope", None, cx));
    assert_eq!(
        same_project,
        SavedBounds {
            location: Some(bounds.location),
            size: bounds.size,
        }
    );
    assert_eq!(other_project, SavedBounds::default());
    assert_eq!(without_project, SavedBounds::default());
}

#[gpui::test]
async fn activating_the_owner_window_hands_the_activation_back_to_the_dialog_window(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let (owner_window, workspace) = open_owner(cx).await;
    let dialog = open_probe(
        cx,
        workspace_owner(&workspace, owner_window),
        "merge_tool_test_owner_activation",
        &ProbeControls::new(),
    );
    assert_eq!(
        active_window(cx),
        Some(AnyWindowHandle::from(dialog.window))
    );

    owner_window
        .update(cx, |_, window, _| window.activate_window())
        .expect("the owner window is open");
    cx.run_until_parked();

    assert_eq!(
        active_window(cx),
        Some(AnyWindowHandle::from(dialog.window))
    );
}
