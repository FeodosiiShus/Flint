//! Checks that everything a window paints follows the UI standard, so a test can open any
//! surface of the app and assert it in one call.

use gpui::{Bounds, Pixels, Quad, ScaledPixels, VisualTestContext};

use crate::{CloseButtonSize, SurfaceKind};

const TOLERANCE: f32 = 0.01;

fn near(a: ScaledPixels, b: ScaledPixels) -> bool {
    (a.0 - b.0).abs() < TOLERANCE
}

fn same_bounds(a: &Bounds<ScaledPixels>, b: &Bounds<ScaledPixels>) -> bool {
    near(a.origin.x, b.origin.x)
        && near(a.origin.y, b.origin.y)
        && near(a.size.width, b.size.width)
        && near(a.size.height, b.size.height)
}

fn is_whole(value: f32) -> bool {
    (value - value.round()).abs() < TOLERANCE
}

/// The surface kind tagged on an element, from its `SURFACE-…` selector.
fn surface_kind(selector: &str) -> Option<SurfaceKind> {
    SurfaceKind::ALL
        .into_iter()
        .find(|kind| kind.debug_selector() == selector)
}

/// GPUI paints an element's fill and its border as two quads with the same bounds; the border
/// quad carries the border widths.
fn chrome_quad(quads: &[Quad], bounds: &Bounds<ScaledPixels>) -> Option<Quad> {
    let at_bounds: Vec<&Quad> = quads
        .iter()
        .filter(|quad| same_bounds(&quad.bounds, bounds))
        .collect();
    at_bounds
        .iter()
        .find(|quad| quad.border_widths.top.0 > 0.)
        .or_else(|| at_bounds.first())
        .map(|quad| (*quad).clone())
}

/// Returns every way the last frame breaks the UI standard, or nothing.
pub fn standard_violations(cx: &mut VisualTestContext) -> Vec<String> {
    let (scale, viewport, quads) = cx.update(|window, _| {
        (
            window.scale_factor(),
            window.viewport_size(),
            window.painted_quads(),
        )
    });
    let mut violations = Vec::new();

    for (selector, bounds) in cx.debug_bounds_with_prefix("SURFACE-") {
        let Some(kind) = surface_kind(&selector) else {
            violations.push(format!("{selector} is not a known surface kind"));
            continue;
        };
        let standard = kind.standard();
        let scaled = bounds.scale(scale);
        let Some(quad) = chrome_quad(&quads, &scaled) else {
            violations.push(format!("{selector} at {bounds:?} paints no chrome"));
            continue;
        };
        let radius = standard.corner_radius.scale(scale);
        for corner in [
            quad.corner_radii.top_left,
            quad.corner_radii.top_right,
            quad.corner_radii.bottom_right,
            quad.corner_radii.bottom_left,
        ] {
            if !near(corner, radius) {
                violations.push(format!(
                    "{selector} at {bounds:?} has a {corner:?} corner, the standard is {radius:?}"
                ));
                break;
            }
        }
        let border = standard.border_width.scale(scale);
        for edge in [
            quad.border_widths.top,
            quad.border_widths.right,
            quad.border_widths.bottom,
            quad.border_widths.left,
        ] {
            if !near(edge, border) {
                violations.push(format!(
                    "{selector} at {bounds:?} has a {edge:?} border, the standard is {border:?}"
                ));
                break;
            }
        }
        if kind != SurfaceKind::Tooltip
            && (bounds.left() < Pixels::ZERO
                || bounds.top() < Pixels::ZERO
                || bounds.right() > viewport.width
                || bounds.bottom() > viewport.height)
        {
            violations.push(format!(
                "{selector} at {bounds:?} runs out of the {viewport:?} window"
            ));
        }
        for (name, value) in [
            ("left edge", scaled.origin.x.0),
            ("top edge", scaled.origin.y.0),
            ("width", scaled.size.width.0),
            ("height", scaled.size.height.0),
        ] {
            if !is_whole(value) {
                violations.push(format!(
                    "{selector} at {bounds:?}: its {name} falls between device pixels at {scale}x"
                ));
                break;
            }
        }
    }

    let close_buttons: Vec<(String, Bounds<Pixels>)> = cx.debug_bounds_with_prefix("CLOSE_BUTTON-");
    for (_, bounds) in cx.debug_bounds_with_prefix("ICON-Close") {
        if !close_buttons
            .iter()
            .any(|(_, standard)| *standard == bounds)
        {
            violations.push(format!(
                "a close glyph at {bounds:?} is not drawn by ui::CloseButton"
            ));
        }
    }
    let sides = [
        CloseButtonSize::Header.metrics().box_side,
        CloseButtonSize::Inline.metrics().box_side,
    ];
    for (selector, bounds) in close_buttons {
        let square =
            (f32::from(bounds.size.width) - f32::from(bounds.size.height)).abs() < TOLERANCE;
        let standard_side = sides
            .iter()
            .any(|side| (f32::from(bounds.size.width) - f32::from(*side)).abs() < TOLERANCE);
        if !square || !standard_side {
            violations.push(format!(
                "{selector} is {:?}, the standard close buttons are {sides:?} squares",
                bounds.size
            ));
        }
    }
    violations
}

/// Fails the test with every way the last frame breaks the UI standard.
#[track_caller]
pub fn assert_follows_the_ui_standard(cx: &mut VisualTestContext, scenario: &str) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let violations = standard_violations(cx);
    assert!(
        violations.is_empty(),
        "{scenario} breaks the UI standard:\n{}",
        violations.join("\n")
    );
}
