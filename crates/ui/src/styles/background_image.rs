use std::{collections::HashMap, sync::Arc};

use gpui::{
    App, Bounds, ContentMask, Corners, DevicePixels, Global, Hsla, IntoElement, ObjectFit, Pixels,
    RenderImage, Size, Styled, Window, WindowId, canvas, point, px, size,
};
use gpui_util::ResultExt;
use smallvec::SmallVec;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BackgroundImageMode {
    Plain,
    Scale,
    Tile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BackgroundImageAlignment {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BackgroundImageTarget {
    EditorAndTools,
    EmptyFrame,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BackgroundImageArea {
    Window,
    Element,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BackgroundImagePaintLayer {
    pub image: Arc<RenderImage>,
    pub mode: BackgroundImageMode,
    pub alignment: BackgroundImageAlignment,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WindowBackgroundImages {
    pub editor_and_tools: Option<BackgroundImagePaintLayer>,
    pub empty_frame: Option<BackgroundImagePaintLayer>,
}

impl WindowBackgroundImages {
    pub fn layer(&self, target: BackgroundImageTarget) -> Option<&BackgroundImagePaintLayer> {
        match target {
            BackgroundImageTarget::EditorAndTools => self.editor_and_tools.as_ref(),
            BackgroundImageTarget::EmptyFrame => self.empty_frame.as_ref(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.editor_and_tools.is_none() && self.empty_frame.is_none()
    }
}

#[derive(Default)]
pub struct BackgroundImageLayers {
    windows: HashMap<WindowId, WindowBackgroundImages>,
}

impl Global for BackgroundImageLayers {}

impl BackgroundImageLayers {
    pub fn window(window_id: WindowId, cx: &App) -> Option<&WindowBackgroundImages> {
        cx.try_global::<Self>()?.windows.get(&window_id)
    }

    pub fn layer(
        window_id: WindowId,
        target: BackgroundImageTarget,
        cx: &App,
    ) -> Option<&BackgroundImagePaintLayer> {
        Self::window(window_id, cx)?.layer(target)
    }

    pub fn set_window(window_id: WindowId, images: WindowBackgroundImages, cx: &mut App) {
        if images.is_empty() {
            Self::remove_window(window_id, cx);
        } else {
            let layers = cx.default_global::<Self>();
            layers.windows.insert(window_id, images);
        }
    }

    pub fn remove_window(window_id: WindowId, cx: &mut App) -> Option<WindowBackgroundImages> {
        if !cx.has_global::<Self>() {
            return None;
        }
        cx.global_mut::<Self>().windows.remove(&window_id)
    }
}

pub fn has_background_image(target: BackgroundImageTarget, window: &Window, cx: &App) -> bool {
    BackgroundImageLayers::layer(window.window_handle().window_id(), target, cx).is_some()
}

pub fn background_image_rects(
    area: Bounds<Pixels>,
    visible: Bounds<Pixels>,
    image_size: Size<DevicePixels>,
    scale_factor: f32,
    mode: BackgroundImageMode,
    alignment: BackgroundImageAlignment,
) -> SmallVec<[Bounds<Pixels>; 4]> {
    let mut rects = SmallVec::new();
    if image_size.width.0 <= 0
        || image_size.height.0 <= 0
        || scale_factor <= 0.
        || area.size.width <= Pixels::ZERO
        || area.size.height <= Pixels::ZERO
    {
        return rects;
    }
    let visible = visible.intersect(&area);
    if visible.size.width <= Pixels::ZERO || visible.size.height <= Pixels::ZERO {
        return rects;
    }

    let natural_size = size(
        px(image_size.width.0 as f32 / scale_factor),
        px(image_size.height.0 as f32 / scale_factor),
    );
    match mode {
        BackgroundImageMode::Plain => {
            let rect = aligned_bounds(area, natural_size, alignment, scale_factor);
            if rect.intersects(&visible) {
                rects.push(rect);
            }
        }
        BackgroundImageMode::Scale => {
            let rect = ObjectFit::Cover.get_bounds(area, image_size);
            if rect.intersects(&visible) {
                rects.push(rect);
            }
        }
        BackgroundImageMode::Tile => {
            let first_column = ((visible.left() - area.left()) / natural_size.width).floor() as i32;
            let last_column = ((visible.right() - area.left()) / natural_size.width).ceil() as i32;
            let first_row = ((visible.top() - area.top()) / natural_size.height).floor() as i32;
            let last_row = ((visible.bottom() - area.top()) / natural_size.height).ceil() as i32;
            for row in first_row..last_row {
                for column in first_column..last_column {
                    rects.push(Bounds::new(
                        point(
                            area.left() + natural_size.width * column as f32,
                            area.top() + natural_size.height * row as f32,
                        ),
                        natural_size,
                    ));
                }
            }
        }
    }
    rects
}

fn aligned_bounds(
    area: Bounds<Pixels>,
    image_size: Size<Pixels>,
    alignment: BackgroundImageAlignment,
    scale_factor: f32,
) -> Bounds<Pixels> {
    let free_width = area.size.width - image_size.width;
    let free_height = area.size.height - image_size.height;
    let x = match alignment {
        BackgroundImageAlignment::TopLeft
        | BackgroundImageAlignment::CenterLeft
        | BackgroundImageAlignment::BottomLeft => area.left(),
        BackgroundImageAlignment::TopCenter
        | BackgroundImageAlignment::Center
        | BackgroundImageAlignment::BottomCenter => area.left() + free_width / 2.,
        BackgroundImageAlignment::TopRight
        | BackgroundImageAlignment::CenterRight
        | BackgroundImageAlignment::BottomRight => area.left() + free_width,
    };
    let y = match alignment {
        BackgroundImageAlignment::TopLeft
        | BackgroundImageAlignment::TopCenter
        | BackgroundImageAlignment::TopRight => area.top(),
        BackgroundImageAlignment::CenterLeft
        | BackgroundImageAlignment::Center
        | BackgroundImageAlignment::CenterRight => area.top() + free_height / 2.,
        BackgroundImageAlignment::BottomLeft
        | BackgroundImageAlignment::BottomCenter
        | BackgroundImageAlignment::BottomRight => area.top() + free_height,
    };
    Bounds::new(
        point(
            snap_to_device_pixel(x, scale_factor),
            snap_to_device_pixel(y, scale_factor),
        ),
        image_size,
    )
}

fn snap_to_device_pixel(value: Pixels, scale_factor: f32) -> Pixels {
    px((f32::from(value) * scale_factor).round() / scale_factor)
}

pub fn window_background_image_area(window: &Window) -> Bounds<Pixels> {
    Bounds::new(point(px(0.), px(0.)), window.viewport_size())
}

pub fn paint_background_image(
    bounds: Bounds<Pixels>,
    area: Bounds<Pixels>,
    target: BackgroundImageTarget,
    surface_background: Hsla,
    require_opaque: bool,
    corner_radii: Corners<Pixels>,
    window: &mut Window,
    cx: &App,
) {
    if require_opaque && !surface_background.is_opaque() {
        return;
    }
    let window_id = window.window_handle().window_id();
    let Some(layer) = BackgroundImageLayers::layer(window_id, target, cx) else {
        return;
    };
    let rects = background_image_rects(
        area,
        bounds,
        layer.image.size(0),
        window.scale_factor(),
        layer.mode,
        layer.alignment,
    );
    let surface_mask = ContentMask {
        bounds,
        corner_radii: corner_radii.clamp_radii_for_quad_size(bounds.size),
    };
    window.with_content_mask(Some(surface_mask), |window| {
        for rect in rects {
            let image = layer.image.clone();
            window
                .paint_image(bounds, rect, Corners::default(), image, 0, false)
                .log_err();
        }
    });
}

pub fn background_image_layer(
    target: BackgroundImageTarget,
    area: BackgroundImageArea,
    surface_background: Hsla,
    require_opaque: bool,
    corner_radii: Corners<Pixels>,
) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let area = match area {
                BackgroundImageArea::Window => window_background_image_area(window),
                BackgroundImageArea::Element => bounds,
            };
            paint_background_image(
                bounds,
                area,
                target,
                surface_background,
                require_opaque,
                corner_radii,
                window,
                cx,
            );
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(0.)), size(px(1000.), px(800.)))
    }

    fn image(width: i32, height: i32) -> Size<DevicePixels> {
        size(DevicePixels(width), DevicePixels(height))
    }

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
    }

    fn plain(
        image_size: Size<DevicePixels>,
        scale_factor: f32,
        alignment: BackgroundImageAlignment,
    ) -> SmallVec<[Bounds<Pixels>; 4]> {
        background_image_rects(
            area(),
            area(),
            image_size,
            scale_factor,
            BackgroundImageMode::Plain,
            alignment,
        )
    }

    #[test]
    fn plain_mode_places_a_smaller_image_at_each_of_the_nine_anchor_points() {
        let cases = [
            (BackgroundImageAlignment::TopLeft, 0., 0.),
            (BackgroundImageAlignment::TopCenter, 400., 0.),
            (BackgroundImageAlignment::TopRight, 800., 0.),
            (BackgroundImageAlignment::CenterLeft, 0., 350.),
            (BackgroundImageAlignment::Center, 400., 350.),
            (BackgroundImageAlignment::CenterRight, 800., 350.),
            (BackgroundImageAlignment::BottomLeft, 0., 700.),
            (BackgroundImageAlignment::BottomCenter, 400., 700.),
            (BackgroundImageAlignment::BottomRight, 800., 700.),
        ];
        for (alignment, x, y) in cases {
            let rects = plain(image(200, 100), 1., alignment);
            let expected = rect(x, y, 200., 100.);
            assert_eq!(rects.as_slice(), &[expected], "{alignment:?}");
        }
    }

    #[test]
    fn plain_mode_keeps_natural_size_and_crops_an_image_larger_than_the_area() {
        let large = image(2000, 1600);
        let centered = plain(large, 1., BackgroundImageAlignment::Center);
        assert_eq!(centered.as_slice(), &[rect(-500., -400., 2000., 1600.)]);

        let corner = plain(large, 1., BackgroundImageAlignment::BottomRight);
        assert_eq!(corner.as_slice(), &[rect(-1000., -800., 2000., 1600.)]);
    }

    #[test]
    fn plain_mode_maps_one_image_pixel_to_one_device_pixel() {
        let rects = plain(image(200, 100), 2., BackgroundImageAlignment::TopLeft);
        assert_eq!(rects.as_slice(), &[rect(0., 0., 100., 50.)]);

        let rects = plain(image(200, 100), 2., BackgroundImageAlignment::BottomRight);
        assert_eq!(rects.as_slice(), &[rect(900., 750., 100., 50.)]);
    }

    #[test]
    fn plain_mode_snaps_the_origin_to_whole_device_pixels() {
        let rects = plain(image(201, 101), 1., BackgroundImageAlignment::Center);
        assert_eq!(rects.len(), 1);
        let origin = rects[0].origin;
        assert_eq!(f32::from(origin.x).fract(), 0.);
        assert_eq!(f32::from(origin.y).fract(), 0.);
    }

    #[test]
    fn scale_mode_covers_the_area_and_keeps_the_aspect_ratio() {
        let rects = background_image_rects(
            area(),
            area(),
            image(400, 100),
            1.,
            BackgroundImageMode::Scale,
            BackgroundImageAlignment::TopLeft,
        );
        assert_eq!(rects.as_slice(), &[rect(-1100., 0., 3200., 800.)]);

        let rects = background_image_rects(
            area(),
            area(),
            image(100, 400),
            2.,
            BackgroundImageMode::Scale,
            BackgroundImageAlignment::Center,
        );
        assert_eq!(rects.as_slice(), &[rect(0., -1600., 1000., 4000.)]);
    }

    #[test]
    fn tile_mode_repeats_from_the_area_origin_and_lets_edge_tiles_overflow() {
        let rects = background_image_rects(
            area(),
            area(),
            image(300, 300),
            1.,
            BackgroundImageMode::Tile,
            BackgroundImageAlignment::Center,
        );
        assert_eq!(rects.len(), 12);
        assert_eq!(rects[0], rect(0., 0., 300., 300.));
        assert_eq!(rects[11], rect(900., 600., 300., 300.));
        assert!(rects[11].right() > area().right());
        assert!(rects[11].bottom() > area().bottom());
    }

    #[test]
    fn tile_mode_only_returns_tiles_that_touch_the_visible_bounds() {
        let rects = background_image_rects(
            area(),
            rect(350., 0., 300., 100.),
            image(300, 300),
            1.,
            BackgroundImageMode::Tile,
            BackgroundImageAlignment::TopLeft,
        );
        assert_eq!(
            rects.as_slice(),
            &[rect(300., 0., 300., 300.), rect(600., 0., 300., 300.)]
        );
    }

    #[test]
    fn tile_mode_uses_device_pixel_tile_size_on_high_density_displays() {
        let rects = background_image_rects(
            area(),
            area(),
            image(500, 400),
            2.,
            BackgroundImageMode::Tile,
            BackgroundImageAlignment::TopLeft,
        );
        assert_eq!(rects.len(), 16);
        assert_eq!(rects[5], rect(250., 200., 250., 200.));
    }

    #[test]
    fn nothing_is_painted_outside_the_area_or_for_an_empty_image() {
        let outside = background_image_rects(
            area(),
            rect(2000., 2000., 10., 10.),
            image(100, 100),
            1.,
            BackgroundImageMode::Scale,
            BackgroundImageAlignment::Center,
        );
        assert!(outside.is_empty());

        let empty = plain(image(0, 100), 1., BackgroundImageAlignment::Center);
        assert!(empty.is_empty());
    }
}
