use gpui::{App, Hsla, Img, img, px};
use ui::prelude::*;

use crate::merge_tool::palette::{current_palette, hex_color};

const DIALOG_BACKGROUND_DARK: u32 = 0x191A1C;
const DIALOG_BACKGROUND_LIGHT: u32 = 0xF7F8F9;

pub(crate) fn dialog_background(cx: &App) -> Hsla {
    hex_color(if current_palette(cx).kind.is_dark() {
        DIALOG_BACKGROUND_DARK
    } else {
        DIALOG_BACKGROUND_LIGHT
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ThemedImage {
    DialogQuestion,
    DialogWarning,
    ReadOnly,
    BannerWarning,
    BannerInfo,
    BannerError,
}

impl ThemedImage {
    pub(crate) fn path(self, dark: bool) -> &'static str {
        match (self, dark) {
            (Self::DialogQuestion, true) => "images/merge_dialog_question_dark.svg",
            (Self::DialogQuestion, false) => "images/merge_dialog_question_light.svg",
            (Self::DialogWarning, true) => "images/merge_dialog_warning_dark.svg",
            (Self::DialogWarning, false) => "images/merge_dialog_warning_light.svg",
            (Self::ReadOnly, true) => "images/merge_readonly_dark.svg",
            (Self::ReadOnly, false) => "images/merge_readonly_light.svg",
            (Self::BannerWarning, true) => "images/merge_banner_warning_dark.svg",
            (Self::BannerWarning, false) => "images/merge_banner_warning_light.svg",
            (Self::BannerInfo, true) => "images/merge_banner_info_dark.svg",
            (Self::BannerInfo, false) => "images/merge_banner_info_light.svg",
            (Self::BannerError, true) => "images/merge_banner_error_dark.svg",
            (Self::BannerError, false) => "images/merge_banner_error_light.svg",
        }
    }
}

pub(crate) fn themed_image(image: ThemedImage, size: f32, cx: &App) -> Img {
    let dark = current_palette(cx).kind.is_dark();
    img(image.path(dark)).w(px(size)).h(px(size)).flex_none()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::ThemedImage;

    const ALL_IMAGES: [ThemedImage; 6] = [
        ThemedImage::DialogQuestion,
        ThemedImage::DialogWarning,
        ThemedImage::ReadOnly,
        ThemedImage::BannerWarning,
        ThemedImage::BannerInfo,
        ThemedImage::BannerError,
    ];

    fn asset_contents(path: &str) -> String {
        let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        std::fs::read_to_string(assets.join(path))
            .unwrap_or_else(|error| panic!("asset {path} cannot be read: {error}"))
    }

    #[test]
    fn merge_tool_chrome_every_themed_image_ships_as_an_svg_asset_in_both_variants() {
        for image in ALL_IMAGES {
            for dark in [true, false] {
                let contents = asset_contents(image.path(dark));
                assert!(
                    contents.trim_start().starts_with("<svg"),
                    "{image:?} dark={dark} is not a bare svg document"
                );
            }
        }
    }

    #[test]
    fn merge_tool_chrome_dark_and_light_variants_are_distinct_files() {
        for image in ALL_IMAGES {
            assert_ne!(image.path(true), image.path(false));
            assert_ne!(
                asset_contents(image.path(true)),
                asset_contents(image.path(false))
            );
        }
    }

    fn raster_and_logical_size(image: ThemedImage) -> (u32, u32) {
        match image {
            ThemedImage::DialogQuestion | ThemedImage::DialogWarning => (56, 28),
            ThemedImage::BannerWarning | ThemedImage::BannerInfo | ThemedImage::BannerError => {
                (32, 16)
            }
            ThemedImage::ReadOnly => (16, 16),
        }
    }

    #[test]
    fn merge_tool_chrome_themed_images_declare_their_raster_size_and_view_box() {
        for image in ALL_IMAGES {
            for dark in [true, false] {
                let contents = asset_contents(image.path(dark));
                let (raster, logical) = raster_and_logical_size(image);
                let expected = format!(
                    "width=\"{raster}\" height=\"{raster}\" viewBox=\"0 0 {logical} {logical}\""
                );
                assert!(
                    contents.contains(&expected),
                    "{image:?} dark={dark} does not declare {expected}"
                );
            }
        }
    }
}
