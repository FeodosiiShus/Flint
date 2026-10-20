//! Renders the standard components at the scales and font sizes a Mac runs them at and checks
//! the geometry they paint against `ui::SurfaceKind`, `ui::CloseButtonSize` and
//! `ui::DIALOG_METRICS`.

use gpui::{
    AnyElement, App, Bounds, Context, IntoElement, ParentElement, Pixels, Quad, Render,
    ScaledPixels, Styled, TestAppContext, VisualTestContext, Window, div, point, px, size,
};
use settings::SettingsStore;
use ui::{
    BALLOON_METRICS, Balloon, CloseButton, CloseButtonSize, DIALOG_METRICS, DialogButton,
    DialogWidth, Modal, ModalFooter, ModalHeader, Section, Severity, StyledSurface, SurfaceKind,
    prelude::*,
};

/// (rem size, scale factor): a non-Retina display at IntelliJ's 13 px, Retina at Flint's default
/// 15 px, and Retina with a large UI font.
const CONFIGURATIONS: [(f32, f32); 3] = [(13., 1.), (15., 2.), (20., 2.)];
const TOLERANCE: f32 = 0.01;

type Build = Box<dyn Fn(&mut Window, &mut App) -> AnyElement>;

struct Probe {
    rem_size: Pixels,
    build: Build,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_rem_size(self.rem_size);
        div().size_full().p_4().child((self.build)(window, cx))
    }
}

fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
        theme_settings::init(theme::LoadThemes::JustBase, cx);
    });
}

fn open_probe<'a>(
    cx: &'a mut TestAppContext,
    rem_size: f32,
    scale: f32,
    build: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
) -> &'a mut VisualTestContext {
    let (_, cx) = cx.add_window_view(move |_, _| Probe {
        rem_size: px(rem_size),
        build: Box::new(build),
    });
    cx.simulate_scale_factor_change(scale);
    redraw(cx);
    cx
}

fn redraw(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn scaled(bounds: Bounds<Pixels>, scale: f32) -> Bounds<ScaledPixels> {
    bounds.scale(scale)
}

fn near(a: ScaledPixels, b: ScaledPixels) -> bool {
    (a.0 - b.0).abs() < TOLERANCE
}

fn same_bounds(a: &Bounds<ScaledPixels>, b: &Bounds<ScaledPixels>) -> bool {
    near(a.origin.x, b.origin.x)
        && near(a.origin.y, b.origin.y)
        && near(a.size.width, b.size.width)
        && near(a.size.height, b.size.height)
}

#[track_caller]
fn quads_at(cx: &mut VisualTestContext, bounds: Bounds<Pixels>, description: &str) -> Vec<Quad> {
    let scale = cx.update(|window, _| window.scale_factor());
    let target = scaled(bounds, scale);
    let quads: Vec<Quad> = cx
        .update(|window, _| window.painted_quads())
        .into_iter()
        .filter(|quad| same_bounds(&quad.bounds, &target))
        .collect();
    assert!(
        !quads.is_empty(),
        "{description}: no quad painted at {bounds:?}"
    );
    quads
}

/// GPUI paints an element's fill and its border as two quads at the same bounds.
#[track_caller]
fn quad_at(cx: &mut VisualTestContext, bounds: Bounds<Pixels>, description: &str) -> Quad {
    let quads = quads_at(cx, bounds, description);
    quads
        .iter()
        .find(|quad| quad.border_widths.top.0 > 0.)
        .or_else(|| quads.first())
        .cloned()
        .unwrap_or_else(|| panic!("{description}: no quad painted at {bounds:?}"))
}

#[track_caller]
fn bounds_of(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is painted"))
}

/// A plain icon button with a close glyph is a hand-made close button; every close glyph in an
/// icon button must belong to a [`CloseButton`].
#[track_caller]
fn assert_every_close_glyph_is_a_close_button(cx: &mut VisualTestContext) {
    let standard: Vec<Bounds<Pixels>> = cx
        .debug_bounds_with_prefix("CLOSE_BUTTON-")
        .into_iter()
        .map(|(_, bounds)| bounds)
        .collect();
    for (_, bounds) in cx.debug_bounds_with_prefix("ICON-Close") {
        assert!(
            standard.contains(&bounds),
            "a close glyph at {bounds:?} is not drawn by ui::CloseButton"
        );
    }
}

#[track_caller]
fn assert_px(actual: Pixels, expected: Pixels, description: &str) {
    assert!(
        (f32::from(actual) - f32::from(expected)).abs() < TOLERANCE,
        "{description}: {actual:?}, expected {expected:?}"
    );
}

#[track_caller]
fn assert_surface_chrome(cx: &mut VisualTestContext, kind: SurfaceKind, bounds: Bounds<Pixels>) {
    let scale = cx.update(|window, _| window.scale_factor());
    let standard = kind.standard();
    let quad = quad_at(cx, bounds, kind.debug_selector());
    for radius in [
        quad.corner_radii.top_left,
        quad.corner_radii.top_right,
        quad.corner_radii.bottom_right,
        quad.corner_radii.bottom_left,
    ] {
        assert!(
            near(radius, standard.corner_radius.scale(scale)),
            "{kind:?} at {scale}x: corner radius {radius:?}, expected {:?}",
            standard.corner_radius
        );
    }
    for width in [
        quad.border_widths.top,
        quad.border_widths.right,
        quad.border_widths.bottom,
        quad.border_widths.left,
    ] {
        assert!(
            near(width, standard.border_width.scale(scale)),
            "{kind:?} at {scale}x: border {width:?}, expected {:?}",
            standard.border_width
        );
    }
}

#[gpui::test]
fn close_buttons_keep_one_size_and_a_circular_hover_at_every_font_and_scale(
    cx: &mut TestAppContext,
) {
    init(cx);
    for (rem_size, scale) in CONFIGURATIONS {
        let cx = open_probe(cx, rem_size, scale, |_, _| {
            h_flex()
                .gap_4()
                .child(CloseButton::new("header", CloseButtonSize::Header))
                .child(CloseButton::new("inline", CloseButtonSize::Inline))
                .into_any_element()
        });
        for (selector, size) in [
            ("CLOSE_BUTTON-header", CloseButtonSize::Header),
            ("CLOSE_BUTTON-inline", CloseButtonSize::Inline),
        ] {
            let metrics = size.metrics();
            let bounds = bounds_of(cx, selector);
            let description = format!("{selector} at rem {rem_size}, {scale}x");
            assert_px(bounds.size.width, metrics.box_side, &description);
            assert_px(bounds.size.height, metrics.box_side, &description);
            assert_every_close_glyph_is_a_close_button(cx);

            cx.simulate_mouse_move(bounds.center(), None, Default::default());
            redraw(cx);
            let hover = quad_at(cx, bounds, &description);
            assert!(
                near(
                    hover.corner_radii.top_left,
                    metrics.hover_radius.scale(scale)
                ),
                "{description}: the hover fill is a circle, got radius {:?}",
                hover.corner_radii.top_left
            );
            cx.simulate_mouse_move(point(px(1.), px(1.)), None, Default::default());
            redraw(cx);
        }
    }
}

#[gpui::test]
fn every_surface_kind_paints_its_standard_radius_and_border(cx: &mut TestAppContext) {
    init(cx);
    for (rem_size, scale) in CONFIGURATIONS {
        let cx = open_probe(cx, rem_size, scale, |window, cx| {
            v_flex()
                .gap_4()
                .child(div().size(px(120.)).popup_surface(cx))
                .child(
                    div()
                        .h(px(80.))
                        .dialog_surface(DialogWidth::Small, window, cx),
                )
                .child(div().size(px(90.)).balloon_surface(cx))
                .child(div().size(px(60.)).tooltip_surface(cx))
                .into_any_element()
        });
        let surfaces = cx.debug_bounds_with_prefix("SURFACE-");
        assert_eq!(surfaces.len(), 4, "every surface is tagged: {surfaces:?}");
        for kind in SurfaceKind::ALL {
            let bounds = surfaces
                .iter()
                .find(|(selector, _)| selector == kind.debug_selector())
                .map(|(_, bounds)| *bounds)
                .unwrap_or_else(|| panic!("{kind:?} is painted"));
            assert_surface_chrome(cx, kind, bounds);
        }
    }
}

#[gpui::test]
fn dialogs_take_the_standard_width_and_shrink_to_fit_small_windows(cx: &mut TestAppContext) {
    init(cx);
    for (rem_size, scale) in CONFIGURATIONS {
        let cx = open_probe(cx, rem_size, scale, |window, cx| {
            div()
                .h(px(2000.))
                .dialog_surface(DialogWidth::Large, window, cx)
                .into_any_element()
        });
        cx.simulate_resize(size(px(1600.), px(1000.)));
        redraw(cx);
        let wide = bounds_of(cx, SurfaceKind::Dialog.debug_selector());
        assert_px(
            wide.size.width,
            DialogWidth::Large.pixels(px(rem_size)),
            &format!("Large dialog at rem {rem_size}"),
        );
        assert!(
            wide.size.height <= px(1000.) - ui::DIALOG_TOP_OFFSET,
            "a tall dialog stops above the window bottom: {wide:?}"
        );

        cx.simulate_resize(size(px(500.), px(400.)));
        redraw(cx);
        let narrow = bounds_of(cx, SurfaceKind::Dialog.debug_selector());
        let margin = SurfaceKind::Dialog.standard().viewport_margin;
        assert_px(
            narrow.size.width,
            px(500.) - margin * 2.,
            "a Large dialog in a 500 px window",
        );
        assert_eq!(
            narrow.size.width,
            narrow.size.width.round(),
            "the fitted width stays on whole pixels"
        );
    }
}

#[gpui::test]
fn dialog_header_and_footer_follow_the_standard(cx: &mut TestAppContext) {
    init(cx);
    for (rem_size, scale) in CONFIGURATIONS {
        let cx =
            open_probe(cx, rem_size, scale, |window, cx| {
                div()
                    .dialog_surface(DialogWidth::Medium, window, cx)
                    .child(
                        Modal::new("standard-dialog", None)
                            .header(
                                ModalHeader::new()
                                    .headline("Push feature")
                                    .show_dismiss_button(true),
                            )
                            .section(Section::new().child(Label::new("1 commit to push")))
                            .footer(
                                ModalFooter::new()
                                    .end_button(
                                        div()
                                            .debug_selector(|| "CANCEL".into())
                                            .child(DialogButton::new("cancel", "Cancel")),
                                    )
                                    .end_button(div().debug_selector(|| "PUSH".into()).child(
                                        DialogButton::new("push", "Push").default_button(true),
                                    )),
                            ),
                    )
                    .into_any_element()
            });
        let description = format!("rem {rem_size}, {scale}x");
        let dialog = bounds_of(cx, SurfaceKind::Dialog.debug_selector());
        let close = bounds_of(cx, "CLOSE_BUTTON-dismiss");
        let header = CloseButtonSize::Header.metrics();
        assert_px(close.size.width, header.box_side, &description);
        let inset = SurfaceKind::Dialog.standard().content_inset;
        assert_px(
            dialog.right() - close.right(),
            inset,
            &format!("the close button is inset like the content, {description}"),
        );
        assert_px(
            close.top() - dialog.top(),
            inset,
            &format!("the close button sits in the corner like the buttons, {description}"),
        );

        let cancel = bounds_of(cx, "CANCEL");
        let push = bounds_of(cx, "PUSH");
        assert_px(
            push.left() - cancel.right(),
            DIALOG_METRICS.button_gap,
            &format!("gap between dialog buttons, {description}"),
        );
        assert_px(
            dialog.right() - push.right(),
            inset,
            &format!("the default button ends on the content inset, {description}"),
        );
        assert_px(
            dialog.bottom() - push.bottom(),
            inset,
            &format!("the button row sits on the bottom inset, {description}"),
        );
        assert!(
            push.left() > cancel.left(),
            "the default button comes last, as on macOS"
        );
        assert_every_close_glyph_is_a_close_button(cx);
    }
}

#[gpui::test]
fn balloon_titles_wrap_and_the_close_button_has_no_tooltip(cx: &mut TestAppContext) {
    init(cx);
    for (rem_size, scale) in CONFIGURATIONS {
        let cx = open_probe(cx, rem_size, scale, |_, _| {
            Balloon::new("push-result", Severity::Info)
                .title("Pushed 1 commit to origin/QA-10-Zod-CP-UI-API-with-a-long-branch-name")
                .action("Create Pull Request", |_, _, _| {})
                .on_close(|_, _, _| {})
                .into_any_element()
        });
        let description = format!("rem {rem_size}, {scale}x");
        let balloon = bounds_of(cx, SurfaceKind::Balloon.debug_selector());
        assert_px(balloon.size.width, BALLOON_METRICS.width, &description);
        assert_surface_chrome(cx, SurfaceKind::Balloon, balloon);

        let title = bounds_of(cx, "BALLOON_TITLE");
        let line_height = cx.update(|window, _| window.line_height());
        assert!(
            title.size.height > line_height * 1.5,
            "a long title wraps onto more lines instead of being cut off ({description}): {title:?}"
        );

        let close = bounds_of(cx, "CLOSE_BUTTON-balloon-close");
        assert_px(
            balloon.right() - close.right(),
            BALLOON_METRICS.close_inset,
            &format!("close button right inset, {description}"),
        );
        assert_px(
            close.top() - balloon.top(),
            BALLOON_METRICS.close_inset,
            &format!("close button top inset, {description}"),
        );
        cx.simulate_mouse_move(balloon.center(), None, Default::default());
        redraw(cx);
        cx.simulate_mouse_move(close.center(), None, Default::default());
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(2));
        redraw(cx);
        let circle = quad_at(cx, close, &description);
        assert!(
            near(
                circle.corner_radii.top_left,
                CloseButtonSize::Header.metrics().hover_radius.scale(scale)
            ),
            "the balloon close button shows the standard hover circle ({description})"
        );
        assert!(
            cx.debug_bounds(SurfaceKind::Tooltip.debug_selector())
                .is_none(),
            "the balloon close button has no tooltip that could pass for a second button"
        );
        cx.simulate_mouse_move(point(px(1.), px(1.)), None, Default::default());
        redraw(cx);
    }
}

#[gpui::test]
fn tooltips_are_bordered_and_keep_clear_of_the_pointer(cx: &mut TestAppContext) {
    init(cx);
    for (rem_size, scale) in CONFIGURATIONS {
        let cx = open_probe(cx, rem_size, scale, |_, _| {
            div()
                .pt_8()
                .pl_8()
                .child(
                    IconButton::new("help", IconName::Info).tooltip(ui::Tooltip::text("Close Tab")),
                )
                .into_any_element()
        });
        let button = bounds_of(cx, "ICON-Info");
        let pointer = button.center();
        cx.simulate_mouse_move(pointer, None, Default::default());
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(2));
        redraw(cx);
        let tooltip = bounds_of(cx, SurfaceKind::Tooltip.debug_selector());
        assert_surface_chrome(cx, SurfaceKind::Tooltip, tooltip);
        assert!(
            tooltip.top() >= pointer.y + ui::TOOLTIP_CURSOR_OFFSET,
            "the tooltip starts below the pointer (rem {rem_size}, {scale}x): {tooltip:?}"
        );
        cx.simulate_mouse_move(point(px(1.), px(1.)), None, Default::default());
        redraw(cx);
    }
}
