#![cfg(target_os = "macos")]

//! Renders the standard surfaces with the Metal renderer over sharp stripes to check the
//! Liquid Glass material: popups blur what lies behind them, tooltips stay opaque.

use gpui::{
    AppContext as _, Context, HeadlessAppContext, IntoElement, NoopTextSystem, ParentElement,
    Render, Rgba, Styled, Window, div, px, size,
};
use image::RgbaImage;
use settings::SettingsStore;
use std::sync::Arc;
use theme::LoadThemes;
use ui::{ActiveTheme, GLASS_MATERIAL, StyledSurface};

const WINDOW_SIZE: f32 = 300.;
const SURFACE_ORIGIN: f32 = 60.;
const SURFACE_SIZE: f32 = 160.;
const STRIPE_WIDTH: f32 = 4.;
const RED: u32 = 0xff0000;
const BLUE: u32 = 0x0000ff;

enum Surface {
    Popup,
    Tooltip,
}

struct SurfaceOverStripes(Surface);

impl Render for SurfaceOverStripes {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let stripes = (0..(WINDOW_SIZE / STRIPE_WIDTH) as usize).map(|index| {
            div()
                .w(px(STRIPE_WIDTH))
                .h_full()
                .bg(gpui::rgb(if index % 2 == 0 { RED } else { BLUE }))
        });
        let surface = div()
            .absolute()
            .top(px(SURFACE_ORIGIN))
            .left(px(SURFACE_ORIGIN))
            .size(px(SURFACE_SIZE));
        let surface = match self.0 {
            Surface::Popup => surface.popup_surface(cx),
            Surface::Tooltip => surface.tooltip_surface(cx),
        };
        div()
            .size_full()
            .flex()
            .flex_row()
            .children(stripes)
            .child(surface)
    }
}

fn headless_app() -> HeadlessAppContext {
    let mut cx = HeadlessAppContext::with_platform(
        Arc::new(NoopTextSystem::new()),
        Arc::new(()),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
        theme_settings::init(LoadThemes::JustBase, cx);
    });
    cx.run_until_parked();
    cx
}

fn render(surface: Surface) -> (RgbaImage, f32, [f32; 3]) {
    let mut cx = headless_app();
    let surface_color = cx.update(|cx| {
        let color = Rgba::from(cx.theme().colors().elevated_surface_background);
        [color.r, color.g, color.b]
    });
    let window = cx
        .open_window(size(px(WINDOW_SIZE), px(WINDOW_SIZE)), |_, cx| {
            cx.new(|_| SurfaceOverStripes(surface))
        })
        .expect("headless window should open");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
        .expect("window should draw");
    let image = cx
        .capture_screenshot(window.into())
        .expect("headless renderer should capture the window");
    let scale = image.height() as f32 / WINDOW_SIZE;
    (image, scale, surface_color)
}

fn pixel(image: &RgbaImage, x: f32, y: f32, scale: f32) -> [f32; 3] {
    let [red, green, blue, _] = image.get_pixel((x * scale) as u32, (y * scale) as u32).0;
    [red, green, blue].map(|channel| channel as f32 / 255.)
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(a, b)| (a - b).abs())
        .fold(0., f32::max)
}

#[test]
fn pixels_glass_popups_blur_the_content_behind_them() {
    let (image, scale, surface_color) = render(Surface::Popup);
    let center = SURFACE_ORIGIN + SURFACE_SIZE / 2.;

    let outside_red = pixel(&image, 1., center, scale);
    let outside_blue = pixel(&image, 1. + STRIPE_WIDTH, center, scale);
    assert!(
        distance(outside_red, [1., 0., 0.]) < 0.02 && distance(outside_blue, [0., 0., 1.]) < 0.02,
        "the stripes outside the popup stay sharp: {outside_red:?} {outside_blue:?}"
    );

    let first = pixel(&image, center, center, scale);
    let next = pixel(&image, center + STRIPE_WIDTH, center, scale);
    assert!(
        distance(first, next) < 0.03,
        "neighbouring stripes blur into one color under the glass: {first:?} {next:?}"
    );

    let opacity = GLASS_MATERIAL.surface_opacity;
    let behind = [0.5, 0., 0.5];
    let expected: [f32; 3] = std::array::from_fn(|index| {
        surface_color[index] * opacity + behind[index] * (1. - opacity)
    });
    assert!(
        distance(first, expected) < 0.02,
        "the glass lays the surface color over the blurred content: got {first:?}, expected {expected:?}"
    );
}

#[test]
fn pixels_tooltips_stay_opaque_like_the_macos_tooltip_material() {
    let (image, scale, surface_color) = render(Surface::Tooltip);
    let center = SURFACE_ORIGIN + SURFACE_SIZE / 2.;
    for offset in [0., STRIPE_WIDTH] {
        let inside = pixel(&image, center + offset, center, scale);
        assert!(
            distance(inside, surface_color) < 0.02,
            "the tooltip covers the stripes completely: {inside:?} vs {surface_color:?}"
        );
    }
}
