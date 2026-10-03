#![cfg(target_os = "macos")]

use gpui::{
    AppContext as _, Context, HeadlessAppContext, Hsla, IntoElement, NoopTextSystem, ParentElement,
    Pixels, Render, Rgba, Size, Styled, Window, div, px, rgb, size,
};
use image::RgbaImage;
use settings::SettingsStore;
use std::sync::Arc;
use theme::LoadThemes;
use ui::{ActiveTheme, Tab, TabBar, TabPosition, Toggleable, project_gradient_layer};

const GRADIENT_WINDOW_WIDTH: f32 = 200.;
const GRADIENT_WINDOW_HEIGHT: f32 = 40.;
const GRADIENT_SURFACE: u32 = 0x406080;
const GRADIENT_CHANNEL_TOLERANCE: u8 = 4;
const TINT_MARGIN: i32 = 20;
const TAB_STRIP_WIDTH: f32 = 200.;
const TAB_LABEL_WIDTH: f32 = 48.;
const ISLAND_TAB_ACCENT_FILL_OPACITY: f32 = 0.15;
const CHANNEL_TOLERANCE: u8 = 2;
const CLEAR_CHANNELS: [f32; 3] = [0.; 3];

struct GradientSurface;

impl Render for GradientSurface {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(GRADIENT_SURFACE))
            .child(project_gradient_layer(gpui::red()))
    }
}

struct IslandTabStrip {
    pane_focused: bool,
}

impl Render for IslandTabStrip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            TabBar::new("tab_bar").islands(true).child(
                Tab::new("selected_tab")
                    .position(TabPosition::First)
                    .toggle_state(true)
                    .focused(self.pane_focused)
                    .islands(true)
                    .child(div().w(px(TAB_LABEL_WIDTH))),
            ),
        )
    }
}

struct PillBounds {
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
}

impl PillBounds {
    fn center_column(&self) -> u32 {
        (self.left + self.right) / 2
    }

    fn center_row(&self) -> u32 {
        (self.top + self.bottom) / 2
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
    cx
}

fn render_view<V: Render>(
    cx: &mut HeadlessAppContext,
    window_size: Size<Pixels>,
    view: V,
) -> RgbaImage {
    let window = cx
        .open_window(window_size, |_, cx| cx.new(|_| view))
        .expect("headless window should open");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
        .expect("window should draw");
    cx.capture_screenshot(window.into())
        .expect("headless renderer should capture the window")
}

fn render_island_tab_strip(cx: &mut HeadlessAppContext, pane_focused: bool) -> RgbaImage {
    let bar_height = cx.update(|cx| Tab::container_height(cx));
    render_view(
        cx,
        size(px(TAB_STRIP_WIDTH), bar_height),
        IslandTabStrip { pane_focused },
    )
}

fn rgb_channels(color: Hsla) -> [f32; 3] {
    let rgba = Rgba::from(color);
    [rgba.r, rgba.g, rgba.b]
}

fn composite_over(color: Hsla, backdrop: [f32; 3]) -> [f32; 3] {
    let channels = rgb_channels(color);
    std::array::from_fn(|index| channels[index] * color.a + backdrop[index] * (1. - color.a))
}

fn to_pixel(channels: [f32; 3]) -> [u8; 3] {
    channels.map(|channel| (channel.clamp(0., 1.) * 255.).round() as u8)
}

fn hex_pixel(hex: u32) -> [u8; 3] {
    let [_, red, green, blue] = hex.to_be_bytes();
    [red, green, blue]
}

fn pixel_at(image: &RgbaImage, column: u32, row: u32) -> [u8; 3] {
    let [red, green, blue, _] = image.get_pixel(column, row).0;
    [red, green, blue]
}

fn pixels_near(actual: [u8; 3], expected: [u8; 3], tolerance: u8) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(actual, expected)| actual.abs_diff(expected) <= tolerance)
}

#[track_caller]
fn assert_pixel_near(actual: [u8; 3], expected: [u8; 3], tolerance: u8, description: &str) {
    assert!(
        pixels_near(actual, expected, tolerance),
        "{description}: got {actual:?}, expected {expected:?} within {tolerance}"
    );
}

fn find_pill(image: &RgbaImage, bar: [u8; 3]) -> PillBounds {
    let differs_from_bar =
        |column: u32, row: u32| !pixels_near(pixel_at(image, column, row), bar, CHANNEL_TOLERANCE);
    let middle_row = image.height() / 2;
    let left = (0..image.width())
        .find(|&column| differs_from_bar(column, middle_row))
        .expect("the pill should cross the middle row of the tab bar");
    let right = (0..image.width())
        .rev()
        .find(|&column| differs_from_bar(column, middle_row))
        .expect("the pill should have a right edge");
    let center_column = (left + right) / 2;
    let top = (0..image.height())
        .find(|&row| differs_from_bar(center_column, row))
        .expect("the pill should have a top edge");
    let bottom = (0..image.height())
        .rev()
        .find(|&row| differs_from_bar(center_column, row))
        .expect("the pill should have a bottom edge");
    PillBounds {
        left,
        top,
        right,
        bottom,
    }
}

#[test]
fn pixels_project_gradient_tints_the_left_edge_and_fades_into_the_surface() {
    let mut cx = headless_app();
    let image = render_view(
        &mut cx,
        size(px(GRADIENT_WINDOW_WIDTH), px(GRADIENT_WINDOW_HEIGHT)),
        GradientSurface,
    );
    let surface = hex_pixel(GRADIENT_SURFACE);
    let [surface_red, surface_green, surface_blue] = surface.map(i32::from);
    let width = image.width();
    let row = image.height() / 2;

    for column in [0, 1] {
        let pixel = pixel_at(&image, column, row);
        let [red, green, blue] = pixel.map(i32::from);
        assert!(
            red >= surface_red + TINT_MARGIN
                && green <= surface_green - TINT_MARGIN
                && blue <= surface_blue - TINT_MARGIN,
            "column {column} should be tinted towards red, got {pixel:?} over {surface:?}"
        );
        assert!(
            green >= surface_green / 3 && blue >= surface_blue / 3,
            "column {column} should stay translucent, got {pixel:?} over {surface:?}"
        );
    }

    let reds_across_the_fade =
        [0, width / 8, width / 4].map(|column| i32::from(pixel_at(&image, column, row)[0]));
    assert!(
        reds_across_the_fade
            .windows(2)
            .all(|pair| pair[0] > pair[1] + i32::from(GRADIENT_CHANNEL_TOLERANCE)),
        "the tint should fade towards the right, got reds {reds_across_the_fade:?}"
    );

    for column in [width * 3 / 4, width - 1] {
        assert_pixel_near(
            pixel_at(&image, column, row),
            surface,
            GRADIENT_CHANNEL_TOLERANCE,
            &format!("column {column} lies past the fade end and shows the bare surface"),
        );
    }
}

#[test]
fn pixels_selected_island_tab_in_a_focused_pane_is_an_accent_pill_with_rounded_corners() {
    let mut cx = headless_app();
    let image = render_island_tab_strip(&mut cx, true);
    let (bar_background, text_accent) = cx.update(|cx| {
        let colors = cx.theme().colors();
        (colors.editor_background, colors.text_accent)
    });
    let bar = composite_over(bar_background, CLEAR_CHANNELS);
    let fill = composite_over(text_accent.opacity(ISLAND_TAB_ACCENT_FILL_OPACITY), bar);
    let expected_border = to_pixel(composite_over(text_accent, fill));
    let expected_fill = to_pixel(fill);
    let expected_bar = to_pixel(bar);
    assert!(
        !pixels_near(expected_fill, expected_bar, CHANNEL_TOLERANCE),
        "the accent fill {expected_fill:?} should stand out from the bar {expected_bar:?}"
    );
    assert!(
        !pixels_near(expected_border, expected_fill, CHANNEL_TOLERANCE),
        "the accent border {expected_border:?} should stand out from the fill {expected_fill:?}"
    );

    let pill = find_pill(&image, expected_bar);
    assert_pixel_near(
        pixel_at(&image, pill.center_column(), pill.top),
        expected_border,
        CHANNEL_TOLERANCE,
        "the pill border is the accent color",
    );
    assert_pixel_near(
        pixel_at(&image, pill.center_column(), pill.center_row()),
        expected_fill,
        CHANNEL_TOLERANCE,
        "the pill center is the bar tinted with the accent fill",
    );
    assert_pixel_near(
        pixel_at(&image, pill.left, pill.top),
        expected_bar,
        CHANNEL_TOLERANCE,
        "the corner of the pill's bounding box lies outside its rounded radius",
    );
}

#[test]
fn pixels_selected_island_tab_in_an_unfocused_pane_is_an_unfilled_pill_with_the_border_color() {
    let mut cx = headless_app();
    let image = render_island_tab_strip(&mut cx, false);
    let (bar_background, border) = cx.update(|cx| {
        let colors = cx.theme().colors();
        (colors.editor_background, colors.border)
    });
    let bar = composite_over(bar_background, CLEAR_CHANNELS);
    let expected_border = to_pixel(composite_over(border, bar));
    let expected_bar = to_pixel(bar);
    assert!(
        !pixels_near(expected_border, expected_bar, CHANNEL_TOLERANCE),
        "the border color {expected_border:?} should stand out from the bar {expected_bar:?}"
    );

    let pill = find_pill(&image, expected_bar);
    assert_pixel_near(
        pixel_at(&image, pill.center_column(), pill.top),
        expected_border,
        CHANNEL_TOLERANCE,
        "the unfocused pill border is the theme border color",
    );
    assert_pixel_near(
        pixel_at(&image, pill.center_column(), pill.center_row()),
        expected_bar,
        CHANNEL_TOLERANCE,
        "the unfocused pill has no fill",
    );
    assert_pixel_near(
        pixel_at(&image, pill.left, pill.top),
        expected_bar,
        CHANNEL_TOLERANCE,
        "the corner of the unfocused pill's bounding box lies outside its rounded radius",
    );
}
