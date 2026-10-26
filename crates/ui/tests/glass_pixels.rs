#![cfg(target_os = "macos")]

//! Renders the standard surfaces with the Metal renderer over a code-like backdrop — a dark
//! editor background with bright syntax-coloured runs — to check the Liquid Glass material:
//! popup text stays readable over bright code while the backdrop shows through muted.

use gpui::{
    AppContext as _, Context, Div, HeadlessAppContext, IntoElement, NoopTextSystem, ParentElement,
    Render, Rgba, Styled, Window, div, px, size,
};
use image::RgbaImage;
use settings::SettingsStore;
use std::sync::Arc;
use theme::LoadThemes;
use theme_settings::GlassSettings;
use ui::{ActiveTheme, DialogWidth, StyledSurface};

const WINDOW_SIZE: f32 = 300.;
const SURFACE_ORIGIN: f32 = 60.;
const SURFACE_SIZE: f32 = 160.;
const ROW_STEP: f32 = 16.;
const ROW_TOP: f32 = 10.;
const BAR_HEIGHT: f32 = 8.;
const RUN_GAP: f32 = 16.;

/// One-Dark-like syntax colours for the text-like runs behind the surface.
const RUN_COLORS: [u32; 6] = [0xe5c07b, 0x61afef, 0x98c379, 0xc678dd, 0xe06c75, 0x56b6c2];
const RUN_WIDTHS: [f32; 4] = [96., 56., 120., 40.];

/// A simulated popup row: theme foreground painted over the glass, like glyphs would be.
const TEXT_BAR_TOP: f32 = 70.;
const TEXT_BAR_LEFT: f32 = 30.;
const TEXT_BAR_WIDTH: f32 = 100.;
const TEXT_BAR_HEIGHT: f32 = 8.;

#[derive(Clone, Copy, Debug)]
enum Surface {
    Popup,
    Dialog,
    Balloon,
    Tooltip,
    ToolWindow,
    ToolWindowFilledWithPanelColour,
    ToolWindowKeepingItsFill,
    FadedPopup(f32),
}

struct SurfaceOverCode(Option<Surface>);

fn code_runs() -> Vec<Div> {
    let mut runs = Vec::new();
    let mut row = 0;
    let mut y = ROW_TOP;
    while y + BAR_HEIGHT <= WINDOW_SIZE {
        let mut x = if row % 2 == 0 { 12. } else { 28. };
        let mut run = 0;
        while x < WINDOW_SIZE {
            let width = RUN_WIDTHS[(row + run) % RUN_WIDTHS.len()].min(WINDOW_SIZE - x);
            let color = RUN_COLORS[(row + run) % RUN_COLORS.len()];
            runs.push(
                div()
                    .absolute()
                    .top(px(y))
                    .left(px(x))
                    .w(px(width))
                    .h(px(BAR_HEIGHT))
                    .bg(gpui::rgb(color)),
            );
            x += width + RUN_GAP;
            run += 1;
        }
        y += ROW_STEP;
        row += 1;
    }
    runs
}

impl Render for SurfaceOverCode {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor_background = cx.theme().colors().editor_background;
        let mut children = code_runs();
        if let Some(surface) = self.0 {
            let foreground = cx.theme().colors().text;
            let element = div()
                .absolute()
                .top(px(SURFACE_ORIGIN))
                .left(px(SURFACE_ORIGIN))
                .size(px(SURFACE_SIZE));
            let element = match surface {
                Surface::Popup => element.popup_surface(cx),
                Surface::Dialog => element
                    .dialog_surface(DialogWidth::Small, window, cx)
                    .w(px(SURFACE_SIZE)),
                Surface::Balloon => element.balloon_surface(cx),
                Surface::Tooltip => element.tooltip_surface(cx),
                Surface::ToolWindow => element.tool_window_surface(px(12.), cx),
                Surface::ToolWindowFilledWithPanelColour => {
                    element.tool_window_surface(px(12.), cx).child(
                        div()
                            .absolute()
                            .size_full()
                            .bg(cx.theme().colors().panel_background),
                    )
                }
                Surface::ToolWindowKeepingItsFill => {
                    element.tool_window_surface(px(12.), cx).child(
                        div()
                            .absolute()
                            .size_full()
                            .bg(cx.theme().colors().panel_background)
                            .keep_glass_fill(),
                    )
                }
                Surface::FadedPopup(opacity) => element.popup_surface(cx).opacity(opacity),
            };
            children.push(
                element.child(
                    div()
                        .absolute()
                        .top(px(TEXT_BAR_TOP))
                        .left(px(TEXT_BAR_LEFT))
                        .w(px(TEXT_BAR_WIDTH))
                        .h(px(TEXT_BAR_HEIGHT))
                        .bg(foreground),
                ),
            );
        }
        div()
            .size_full()
            .relative()
            .bg(editor_background)
            .children(children)
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

fn render(surface: Option<Surface>) -> (RgbaImage, f32, [f32; 3], [f32; 3]) {
    let mut cx = headless_app();
    let (surface_color, foreground) = cx.update(|cx| {
        let colors = cx.theme().colors();
        let surface = Rgba::from(colors.elevated_surface_background);
        let text = Rgba::from(colors.text);
        ([surface.r, surface.g, surface.b], [text.r, text.g, text.b])
    });
    let window = cx
        .open_window(size(px(WINDOW_SIZE), px(WINDOW_SIZE)), |_, cx| {
            cx.new(|_| SurfaceOverCode(surface))
        })
        .expect("headless window should open");
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
        .expect("window should draw");
    let image = cx
        .capture_screenshot(window.into())
        .expect("headless renderer should capture the window");
    save_for_inspection(&image, surface);
    let scale = image.height() as f32 / WINDOW_SIZE;
    (image, scale, surface_color, foreground)
}

/// Writes the rendered surface as a PNG when `FLINT_UI_SCREENSHOTS` names a directory, so the
/// glass can be inspected on a Mac without opening the app.
fn save_for_inspection(image: &RgbaImage, surface: Option<Surface>) {
    let Ok(directory) = std::env::var("FLINT_UI_SCREENSHOTS") else {
        return;
    };
    std::fs::create_dir_all(&directory).expect("the screenshot directory should be writable");
    let name = match surface {
        Some(surface) => format!("glass-{surface:?}.png"),
        None => "glass-code-backdrop.png".to_string(),
    };
    let path = std::path::Path::new(&directory).join(name);
    image
        .save(&path)
        .expect("the rendered surface should be saved as a PNG");
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

fn luminance(rgb: [f32; 3]) -> f32 {
    let linear = rgb.map(|channel| {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]
}

fn contrast(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (lighter, darker) = if luminance(a) >= luminance(b) {
        (a, b)
    } else {
        (b, a)
    };
    (luminance(lighter) + 0.05) / (luminance(darker) + 0.05)
}

/// A point inside the surface over a bright syntax run, next to the simulated text row.
fn bleed_point() -> (f32, f32) {
    // Row 8 starts at y 138 with bars 12..108, 124..180, 196..300: (152, 142) sits in the
    // middle of its second run, below the text bar (which ends at y 138).
    (152., 142.)
}

fn text_point() -> (f32, f32) {
    (
        SURFACE_ORIGIN + TEXT_BAR_LEFT + TEXT_BAR_WIDTH / 2.,
        SURFACE_ORIGIN + TEXT_BAR_TOP + TEXT_BAR_HEIGHT / 2.,
    )
}

#[test]
fn pixels_glass_surfaces_keep_text_readable_and_mute_the_code_behind_them() {
    assert!(
        GlassSettings::default().tint(false) >= 0.5,
        "the default tint must cover bright code, or popup rows wash out"
    );
    let (bare, bare_scale, _, _) = render(None);
    let (bleed_x, bleed_y) = bleed_point();
    let (text_x, text_y) = text_point();
    let bright = pixel(&bare, bleed_x, bleed_y, bare_scale);
    assert!(
        bright.iter().fold(0f32, |max, channel| max.max(*channel)) > 0.6,
        "the bleed point hits a bright syntax run outside the glass: {bright:?}"
    );

    for surface in [
        Surface::Popup,
        Surface::Dialog,
        Surface::Balloon,
        Surface::ToolWindow,
    ] {
        let (image, scale, surface_color, foreground) = render(Some(surface));

        let text = pixel(&image, text_x, text_y, scale);
        assert!(
            distance(text, foreground) < 0.05,
            "the {surface:?} row paints its text in the theme foreground over the glass: {text:?} vs {foreground:?}"
        );

        let muted = pixel(&image, bleed_x, bleed_y, scale);
        assert!(
            distance(muted, bright) > 0.25,
            "the {surface:?} mutes the bright run behind it: {muted:?} vs {bright:?}"
        );
        assert!(
            distance(muted, surface_color) < 0.3,
            "the {surface:?} pulls the backdrop toward the surface colour: {muted:?} vs {surface_color:?}"
        );
        assert!(
            contrast(foreground, muted) >= 4.,
            "the {surface:?} keeps popup text readable over bright code: {foreground:?} on {muted:?}"
        );
    }
}

#[test]
fn pixels_a_glass_surface_leaves_the_fills_of_its_content_transparent() {
    let (plain, plain_scale, _, _) = render(Some(Surface::ToolWindow));
    let (filled, filled_scale, _, _) = render(Some(Surface::ToolWindowFilledWithPanelColour));
    let (kept, kept_scale, _, _) = render(Some(Surface::ToolWindowKeepingItsFill));
    let panel = {
        let mut cx = headless_app();
        cx.update(|cx| {
            let panel = Rgba::from(cx.theme().colors().panel_background);
            [panel.r, panel.g, panel.b]
        })
    };
    let (x, y) = bleed_point();

    assert!(
        distance(
            pixel(&filled, x, y, filled_scale),
            pixel(&plain, x, y, plain_scale)
        ) < 0.02,
        "a component that fills its area with the panel colour does not cover the glass, \
         without knowing it lies on glass"
    );
    assert!(
        distance(pixel(&kept, x, y, kept_scale), panel) < 0.03,
        "a fill that hides what lies behind it keeps its colour when it says so"
    );
}

#[test]
fn pixels_a_glass_surface_fades_with_its_content() {
    let (bare, bare_scale, _, _) = render(None);
    let (bleed_x, bleed_y) = bleed_point();
    let behind = pixel(&bare, bleed_x, bleed_y, bare_scale);

    let (hidden, hidden_scale, _, _) = render(Some(Surface::FadedPopup(0.)));
    assert!(
        distance(pixel(&hidden, bleed_x, bleed_y, hidden_scale), behind) < 0.02,
        "a surface held back with an opacity of 0 leaves the backdrop as it was, without a \
         blurred patch where its glass would be"
    );

    let (half, half_scale, _, _) = render(Some(Surface::FadedPopup(0.5)));
    let (shown, shown_scale, _, _) = render(Some(Surface::Popup));
    let half_way = distance(pixel(&half, bleed_x, bleed_y, half_scale), behind);
    let full = distance(pixel(&shown, bleed_x, bleed_y, shown_scale), behind);
    assert!(
        half_way > 0.05 && half_way < full,
        "a surface at half opacity shows part of its glass: {half_way} against {full}"
    );
}

#[test]
fn pixels_tooltips_stay_opaque_like_the_macos_tooltip_material() {
    let (image, scale, surface_color, _) = render(Some(Surface::Tooltip));
    let center = SURFACE_ORIGIN + SURFACE_SIZE / 2.;
    for offset in [0., ROW_STEP] {
        let inside = pixel(&image, center + offset, center, scale);
        assert!(
            distance(inside, surface_color) < 0.02,
            "the tooltip covers the code completely: {inside:?} vs {surface_color:?}"
        );
    }
}
