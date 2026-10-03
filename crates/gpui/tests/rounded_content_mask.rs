#![cfg(target_os = "macos")]

use gpui::{
    AnyElement, AppContext as _, Context, HeadlessAppContext, IntoElement, NoopTextSystem,
    ParentElement, PathBuilder, Pixels, Render, Styled, Window, canvas, div, px, rgb, size,
};
use image::RgbaImage;
use std::sync::Arc;

const WINDOW_BACKGROUND: u32 = 0x0000ff;
const CHILD_COLOR: u32 = 0xff0000;
const CHANNEL_TOLERANCE: u8 = 2;

#[derive(Clone, Copy)]
enum ChildKind {
    Quad,
    Path,
}

struct RoundedContainer {
    corner_radius: Pixels,
    child_kind: ChildKind,
}

fn full_size_child(child_kind: ChildKind) -> AnyElement {
    match child_kind {
        ChildKind::Quad => div().size_full().bg(rgb(CHILD_COLOR)).into_any_element(),
        ChildKind::Path => canvas(
            |_, _, _| {},
            |bounds, _, window, _| {
                let mut builder = PathBuilder::fill();
                builder.move_to(bounds.origin);
                builder.line_to(bounds.top_right());
                builder.line_to(bounds.bottom_right());
                builder.line_to(bounds.bottom_left());
                builder.close();
                let path = builder.build().expect("rectangle path should build");
                window.paint_path(path, rgb(CHILD_COLOR));
            },
        )
        .size_full()
        .into_any_element(),
    }
}

impl Render for RoundedContainer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(rgb(WINDOW_BACKGROUND)).child(
            div()
                .absolute()
                .top(px(10.))
                .left(px(10.))
                .size(px(80.))
                .rounded(self.corner_radius)
                .overflow_hidden()
                .child(full_size_child(self.child_kind)),
        )
    }
}

fn render_rounded_container(corner_radius: Pixels, child_kind: ChildKind) -> RgbaImage {
    let mut cx = HeadlessAppContext::with_platform(
        Arc::new(NoopTextSystem::new()),
        Arc::new(()),
        gpui_platform::current_headless_renderer,
    );
    let window = cx
        .open_window(size(px(100.), px(100.)), |_, cx| {
            cx.new(|_| RoundedContainer {
                corner_radius,
                child_kind,
            })
        })
        .expect("headless window should open");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
        .expect("window should draw");
    cx.capture_screenshot(window.into())
        .expect("headless renderer should capture the window")
}

fn assert_pixel(image: &RgbaImage, x: u32, y: u32, expected: u32, description: &str) {
    let actual = image.get_pixel(x, y).0;
    let expected_channels = [
        (expected >> 16) as u8,
        (expected >> 8) as u8,
        expected as u8,
        u8::MAX,
    ];
    let matches = actual
        .iter()
        .zip(expected_channels)
        .all(|(actual, expected)| actual.abs_diff(expected) <= CHANNEL_TOLERANCE);
    assert!(
        matches,
        "{description}: pixel ({x}, {y}) is {actual:?}, expected {expected_channels:?}"
    );
}

#[test]
fn rounded_overflow_hidden_container_clips_children_to_its_corners() {
    let image = render_rounded_container(px(20.), ChildKind::Quad);
    assert_eq!(image.dimensions(), (200, 200));

    assert_pixel(
        &image,
        22,
        22,
        WINDOW_BACKGROUND,
        "corner outside the radius",
    );
    assert_pixel(
        &image,
        177,
        177,
        WINDOW_BACKGROUND,
        "opposite corner outside the radius",
    );
    assert_pixel(&image, 36, 36, CHILD_COLOR, "corner just inside the radius");
    assert_pixel(
        &image,
        21,
        100,
        CHILD_COLOR,
        "straight edge just inside the container",
    );
    assert_pixel(&image, 100, 100, CHILD_COLOR, "center of the container");
    assert_pixel(&image, 18, 100, WINDOW_BACKGROUND, "outside the container");
}

#[test]
fn rounded_overflow_hidden_container_clips_child_paths_to_its_corners() {
    let image = render_rounded_container(px(20.), ChildKind::Path);

    assert_pixel(
        &image,
        22,
        22,
        WINDOW_BACKGROUND,
        "corner outside the radius",
    );
    assert_pixel(&image, 36, 36, CHILD_COLOR, "corner just inside the radius");
    assert_pixel(&image, 100, 100, CHILD_COLOR, "center of the container");
}

#[test]
fn square_overflow_hidden_container_keeps_children_in_its_corners() {
    let image = render_rounded_container(px(0.), ChildKind::Quad);

    assert_pixel(&image, 22, 22, CHILD_COLOR, "corner of a square container");
    assert_pixel(
        &image,
        20,
        20,
        CHILD_COLOR,
        "first pixel of a square container",
    );
    assert_pixel(
        &image,
        19,
        19,
        WINDOW_BACKGROUND,
        "outside a square container",
    );
    assert_pixel(
        &image,
        100,
        100,
        CHILD_COLOR,
        "center of a square container",
    );
}
