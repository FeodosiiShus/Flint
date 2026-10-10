#![cfg(target_os = "macos")]

use gpui::{
    AppContext as _, Context, Entity, HeadlessAppContext, Hsla, IntoElement, NoopTextSystem,
    ParentElement, Pixels, Render, Rgba, Size, Styled, UpdateGlobal as _, Window, div, px, size,
};
use image::RgbaImage;
use menu::SelectFirst;
use settings::{SettingsStore, ThemeColorsContent, ThemeStyleContent};
use std::sync::Arc;
use theme::LoadThemes;
use ui::{
    ActiveTheme, ContextMenu, POPUP_MENU_METRICS, POPUP_SURFACE_METRICS, PopupRowMetrics,
    PopupRowStyle, StyledExt, TOOLTIP_METRICS, tooltip_container,
};

const TRANSLUCENT_ELEVATED_SURFACE: &str = "#22272f99";
const OPAQUE_ELEVATED_SURFACE: u32 = 0x22272f;
const SELECTION_BACKGROUND: &str = "#2a4371";
const SEPARATOR_COLOR: &str = "#6f737a";
const TRANSLUCENT_ELEMENT_BACKGROUND: &str = "#ffffff20";
const BACKDROP: u32 = 0xff0000;
const WINDOW_SIZE: f32 = 300.;
const POPUP_ORIGIN: f32 = 40.;
const POPUP_SIZE: f32 = 120.;
const MENU_ORIGIN: f32 = 20.;
const MENU_WIDTH: f32 = 240.;
const TOOLTIP_ORIGIN: f32 = 20.;
const TOOLTIP_CONTENT_WIDTH: f32 = 100.;
const TOOLTIP_CONTENT_HEIGHT: f32 = 40.;
const CHANNEL_TOLERANCE: u8 = 2;

struct PopupOverBackdrop;

impl Render for PopupOverBackdrop {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(gpui::rgb(BACKDROP)).child(
            div()
                .absolute()
                .top(px(POPUP_ORIGIN))
                .left(px(POPUP_ORIGIN))
                .size(px(POPUP_SIZE))
                .elevation_2(cx),
        )
    }
}

struct MenuOverBackdrop {
    menu: Entity<ContextMenu>,
}

impl Render for MenuOverBackdrop {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(gpui::rgb(BACKDROP)).child(
            div()
                .absolute()
                .top(px(MENU_ORIGIN))
                .left(px(MENU_ORIGIN))
                .w(px(MENU_WIDTH))
                .child(self.menu.clone()),
        )
    }
}

struct TooltipOverBackdrop;

impl Render for TooltipOverBackdrop {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(gpui::rgb(BACKDROP)).child(
            div()
                .absolute()
                .top(px(TOOLTIP_ORIGIN))
                .left(px(TOOLTIP_ORIGIN))
                .child(tooltip_container(cx, |container, _| {
                    container.child(
                        div()
                            .w(px(TOOLTIP_CONTENT_WIDTH))
                            .h(px(TOOLTIP_CONTENT_HEIGHT)),
                    )
                })),
        )
    }
}

fn headless_app_with_popup_overrides() -> HeadlessAppContext {
    let mut cx = HeadlessAppContext::with_platform(
        Arc::new(NoopTextSystem::new()),
        Arc::new(()),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
        theme_settings::init(LoadThemes::JustBase, cx);
        SettingsStore::update_global(cx, |store, cx| {
            store.update_user_settings(cx, |settings| {
                settings.theme.experimental_theme_overrides = Some(ThemeStyleContent {
                    colors: ThemeColorsContent {
                        elevated_surface_background: Some(TRANSLUCENT_ELEVATED_SURFACE.into()),
                        ghost_element_selected: Some(SELECTION_BACKGROUND.into()),
                        border_variant: Some(SEPARATOR_COLOR.into()),
                        element_background: Some(TRANSLUCENT_ELEMENT_BACKGROUND.into()),
                        ..Default::default()
                    },
                    ..Default::default()
                });
            });
        });
        theme_settings::reload_theme(cx);
    });
    cx.run_until_parked();
    cx
}

fn capture(cx: &mut HeadlessAppContext, window: gpui::AnyWindowHandle) -> RgbaImage {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx))
        .expect("window should draw");
    cx.capture_screenshot(window)
        .expect("headless renderer should capture the window")
}

fn window_size() -> Size<Pixels> {
    size(px(WINDOW_SIZE), px(WINDOW_SIZE))
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

fn opaque_pixel(color: Hsla) -> [u8; 3] {
    to_pixel(rgb_channels(color))
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
fn assert_pixel_near(actual: [u8; 3], expected: [u8; 3], description: &str) {
    assert!(
        pixels_near(actual, expected, CHANNEL_TOLERANCE),
        "{description}: got {actual:?}, expected {expected:?} within {CHANNEL_TOLERANCE}"
    );
}

#[track_caller]
fn assert_position_near(measured: u32, expected: f32, description: &str) {
    assert!(
        (measured as f32 - expected).abs() <= 1.,
        "{description}: measured {measured}px, expected {expected}px"
    );
}

struct Scale(f32);

impl Scale {
    fn of(image: &RgbaImage) -> Self {
        Self(image.height() as f32 / WINDOW_SIZE)
    }

    fn physical(&self, logical: Pixels) -> f32 {
        f32::from(logical) * self.0
    }

    fn pixel(&self, logical: Pixels) -> u32 {
        self.physical(logical).round() as u32
    }
}

#[test]
fn pixels_popup_surface_stays_opaque_when_the_theme_override_is_translucent() {
    let mut cx = headless_app_with_popup_overrides();
    let surface = cx.update(|cx| cx.theme().colors().elevated_surface_background);
    assert!(
        surface.is_opaque(),
        "the translucent elevated_surface.background override must be made opaque, got alpha {}",
        surface.a
    );

    let window = cx
        .open_window(window_size(), |_, cx| cx.new(|_| PopupOverBackdrop))
        .expect("headless window should open");
    let image = capture(&mut cx, window.into());
    let scale = Scale::of(&image);
    let popup_center = scale.pixel(px(POPUP_ORIGIN + POPUP_SIZE / 2.));

    assert_pixel_near(
        pixel_at(&image, scale.pixel(px(4.)), scale.pixel(px(4.))),
        hex_pixel(BACKDROP),
        "the backdrop is bright red so any blending would show",
    );
    assert_pixel_near(
        pixel_at(&image, popup_center, popup_center),
        hex_pixel(OPAQUE_ELEVATED_SURFACE),
        "the popup paints the override's color fully opaque instead of blending it with the red backdrop",
    );
}

#[test]
fn pixels_context_menu_rows_and_separators_follow_the_intellij_popup_metrics() {
    let mut cx = headless_app_with_popup_overrides();
    let (surface, selection, separator) = cx.update(|cx| {
        let colors = cx.theme().colors();
        (
            opaque_pixel(colors.elevated_surface_background),
            opaque_pixel(colors.ghost_element_selected),
            opaque_pixel(colors.border_variant),
        )
    });
    let window = cx
        .open_window(window_size(), |window, cx| {
            let menu = ContextMenu::build(window, cx, |menu, _, _| {
                menu.entry("First", None, |_, _| {})
                    .separator()
                    .entry("Second", None, |_, _| {})
            });
            menu.update(cx, |menu, cx| menu.select_first(&SelectFirst, window, cx));
            cx.new(|_| MenuOverBackdrop { menu })
        })
        .expect("headless window should open");
    let image = capture(&mut cx, window.into());
    let scale = Scale::of(&image);

    let row = PopupRowMetrics::for_style(PopupRowStyle::Menu);
    let border = POPUP_SURFACE_METRICS.border_width;
    let menu_left = px(MENU_ORIGIN);
    let menu_top = px(MENU_ORIGIN);
    let menu_right = px(MENU_ORIGIN + MENU_WIDTH);
    let center_column = scale.pixel(px(MENU_ORIGIN + MENU_WIDTH / 2.));
    let is_selection = |column: u32, row: u32| {
        pixels_near(pixel_at(&image, column, row), selection, CHANNEL_TOLERANCE)
    };

    let scan_start = scale.pixel(menu_top + border);
    let pill_top = (scan_start..image.height())
        .find(|&pixel_row| is_selection(center_column, pixel_row))
        .expect("the selected entry paints a selection pill");
    let expected_pill_top =
        scale.physical(menu_top + border + POPUP_MENU_METRICS.vertical_padding + row.pill_inset_y);
    assert_position_near(
        pill_top,
        expected_pill_top,
        "the first pill starts below the 1px border, the 6px menu padding and the 1px row inset",
    );
    assert_pixel_near(
        pixel_at(&image, center_column, pill_top - 1),
        surface,
        "the row inset above the pill shows the popup surface",
    );

    let pill_bottom = (pill_top..image.height())
        .find(|&pixel_row| !is_selection(center_column, pixel_row))
        .expect("the selection pill ends");
    assert_position_near(
        pill_bottom - pill_top,
        scale.physical(row.pill_height),
        "a single-line menu pill is 24px tall, so the row is 26px with its insets",
    );

    let pill_middle_row = pill_top + scale.pixel(row.pill_height) / 2;
    let pill_left = (0..image.width())
        .find(|&column| is_selection(column, pill_middle_row))
        .expect("the pill has a left edge");
    let pill_right = (0..image.width())
        .rev()
        .find(|&column| is_selection(column, pill_middle_row))
        .expect("the pill has a right edge");
    assert_position_near(
        pill_left,
        scale.physical(menu_left + border + row.pill_inset_x),
        "the pill is 8px from the popup's outer left edge",
    );
    assert_position_near(
        pill_right + 1,
        scale.physical(menu_right - border - row.pill_inset_x),
        "the pill is 8px from the popup's outer right edge",
    );
    assert_pixel_near(
        pixel_at(
            &image,
            scale.pixel(menu_left + border + px(3.)),
            pill_middle_row,
        ),
        surface,
        "between the border and the pill the popup surface shows",
    );
    assert!(
        !is_selection(pill_left, pill_top),
        "the pill's top-left corner pixel is outside its 4px radius and must stay unfilled"
    );
    assert!(
        !is_selection(pill_right, pill_top),
        "the pill's top-right corner pixel is outside its 4px radius and must stay unfilled"
    );

    let separator_top = pill_bottom as f32 + scale.physical(row.pill_inset_y);
    let separator_line_row =
        (separator_top + scale.physical(POPUP_MENU_METRICS.separator_line_offset)).round() as u32;
    let line_left = scale.pixel(menu_left + border + POPUP_MENU_METRICS.separator_inset);
    let line_right = scale.pixel(menu_right - border - POPUP_MENU_METRICS.separator_inset);
    assert_pixel_near(
        pixel_at(&image, center_column, separator_line_row),
        separator,
        "the separator line is drawn 4px into the 9px separator",
    );
    assert_pixel_near(
        pixel_at(&image, center_column, separator_line_row - 1),
        surface,
        "the separator line is 1px thick (above)",
    );
    assert_pixel_near(
        pixel_at(
            &image,
            center_column,
            separator_line_row + scale.pixel(px(1.)),
        ),
        surface,
        "the separator line is 1px thick (below)",
    );
    assert_pixel_near(
        pixel_at(&image, line_left, separator_line_row),
        separator,
        "the separator line starts 10px inside the left border",
    );
    assert_pixel_near(
        pixel_at(&image, line_left - 1, separator_line_row),
        surface,
        "the separator line is inset 10px from the left border",
    );
    assert_pixel_near(
        pixel_at(&image, line_right - 1, separator_line_row),
        separator,
        "the separator line ends 10px inside the right border",
    );
    assert_pixel_near(
        pixel_at(&image, line_right, separator_line_row),
        surface,
        "the separator line is inset 10px from the right border",
    );
}

#[test]
fn pixels_tooltip_is_a_borderless_opaque_feedback_surface_with_intellij_insets() {
    let mut cx = headless_app_with_popup_overrides();
    let (tooltip_background, border) = cx.update(|cx| {
        let colors = cx.theme().colors();
        (
            to_pixel(composite_over(
                colors.element_background,
                rgb_channels(colors.elevated_surface_background),
            )),
            opaque_pixel(colors.border_variant),
        )
    });

    let window = cx
        .open_window(window_size(), |_, cx| cx.new(|_| TooltipOverBackdrop))
        .expect("headless window should open");
    let image = capture(&mut cx, window.into());
    let scale = Scale::of(&image);
    let is_tooltip = |column: u32, row: u32| {
        pixels_near(
            pixel_at(&image, column, row),
            tooltip_background,
            CHANNEL_TOLERANCE,
        )
    };

    let probe_row = scale.pixel(px(TOOLTIP_ORIGIN + 40.));
    let left = (0..image.width())
        .find(|&column| is_tooltip(column, probe_row))
        .expect("the tooltip paints element_background composited over the popup surface");
    let right = (0..image.width())
        .rev()
        .find(|&column| is_tooltip(column, probe_row))
        .expect("the tooltip has a right edge");
    assert!(
        (left..=right).all(|column| is_tooltip(column, probe_row)),
        "the tooltip background is one opaque color across the row"
    );
    assert!(
        (0..image.width()).all(|column| !pixels_near(
            pixel_at(&image, column, probe_row),
            border,
            CHANNEL_TOLERANCE
        )),
        "macOS tooltips paint no border"
    );
    assert_position_near(
        right + 1 - left,
        scale.physical(TOOLTIP_METRICS.padding_x * 2. + px(TOOLTIP_CONTENT_WIDTH)),
        "the tooltip pads its content by 12px on both sides",
    );

    let center_column = (left + right) / 2;
    let top = (0..image.height())
        .find(|&row| is_tooltip(center_column, row))
        .expect("the tooltip has a top edge");
    let bottom = (0..image.height())
        .rev()
        .find(|&row| is_tooltip(center_column, row))
        .expect("the tooltip has a bottom edge");
    assert_position_near(
        bottom + 1 - top,
        scale.physical(
            TOOLTIP_METRICS.padding_top
                + TOOLTIP_METRICS.padding_bottom
                + px(TOOLTIP_CONTENT_HEIGHT),
        ),
        "the tooltip pads its content by 8px on top and 9px at the bottom",
    );
    assert!(
        !is_tooltip(left, top),
        "the tooltip's top-left corner pixel is outside its 4px radius and must stay unfilled"
    );
}
