use gpui::{Pixels, px};
use settings::{RegisterSetting, Settings, SettingsContent};

/// How much of the theme's surface colour covers the system's own glass when nothing is set, in
/// percent. The system's blur already keeps the text behind a surface from showing through, so a
/// light tint is enough and the backdrop's colours still reach the surface.
pub const DEFAULT_NATIVE_GLASS_TINT_PERCENT: u32 = 30;
/// How much of the theme's surface colour covers the renderer's emulated glass when nothing is
/// set, in percent. Its blur is a plain gaussian that leaves bright code recognisable, so the
/// tint has to be heavy enough to keep popup text readable over it.
pub const DEFAULT_EMULATED_GLASS_TINT_PERCENT: u32 = 60;
/// How far the emulated blur spreads when nothing is set.
pub const DEFAULT_GLASS_BLUR_RADIUS: Pixels = px(20.);
const MAX_BLUR_RADIUS: f32 = 64.;

/// The Liquid Glass material shared by popups, dialogs, balloons and tool windows: the theme's
/// surface colour laid over a blurred backdrop. On macOS 26 the system draws the blur, so only
/// the tint reaches it; elsewhere the renderer draws both.
#[derive(Clone, Copy, Debug, PartialEq, RegisterSetting)]
pub struct GlassSettings {
    /// How much of the theme's surface colour covers the system's glass, from 0 to 1.
    pub native_tint: f32,
    /// How much of the theme's surface colour covers the renderer's emulated glass, from 0 to 1.
    pub emulated_tint: f32,
    /// How far the renderer's emulated blur spreads; macOS 26 blurs with the system's own.
    pub blur_radius: Pixels,
}

impl GlassSettings {
    /// How much of the theme's surface colour covers the glass, where `native` says whether the
    /// system draws it.
    pub fn tint(&self, native: bool) -> f32 {
        if native {
            self.native_tint
        } else {
            self.emulated_tint
        }
    }
}

impl Default for GlassSettings {
    fn default() -> Self {
        Self {
            native_tint: DEFAULT_NATIVE_GLASS_TINT_PERCENT as f32 / 100.,
            emulated_tint: DEFAULT_EMULATED_GLASS_TINT_PERCENT as f32 / 100.,
            blur_radius: DEFAULT_GLASS_BLUR_RADIUS,
        }
    }
}

impl Settings for GlassSettings {
    fn from_settings(content: &SettingsContent) -> Self {
        let content = content.glass.as_ref();
        let defaults = Self::default();
        let tint = content
            .and_then(|glass| glass.tint)
            .map(|percent| percent.min(100) as f32 / 100.);
        Self {
            native_tint: tint.unwrap_or(defaults.native_tint),
            emulated_tint: tint.unwrap_or(defaults.emulated_tint),
            blur_radius: content
                .and_then(|glass| glass.blur_radius)
                .map_or(defaults.blur_radius, |radius| {
                    px(radius.clamp(0., MAX_BLUR_RADIUS))
                }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use settings::SettingsContent;

    fn from_json(json: &str) -> GlassSettings {
        let content: SettingsContent = settings::parse_json_with_comments(json).unwrap();
        GlassSettings::from_settings(&content)
    }

    #[test]
    fn each_material_defaults_to_the_tint_that_keeps_its_text_readable() {
        let glass = GlassSettings::default();
        assert_eq!(glass.tint(true), 0.3);
        assert_eq!(glass.tint(false), 0.6);
        assert_eq!(glass.blur_radius, px(20.));
        assert!(
            glass.tint(false) > glass.tint(true),
            "the emulated blur diffuses less than the system's, so it needs the heavier tint"
        );
    }

    #[test]
    fn a_tint_that_is_set_applies_to_both_materials_and_the_blur_is_clamped() {
        let glass = from_json(r#"{"glass": {"tint": 55, "blur_radius": 500}}"#);
        for native in [true, false] {
            assert!((glass.tint(native) - 0.55).abs() < f32::EPSILON);
        }
        assert_eq!(
            glass.blur_radius,
            px(MAX_BLUR_RADIUS),
            "a larger blur than the renderer supports is clamped"
        );

        let glass = from_json(r#"{"glass": {"tint": 250, "blur_radius": -3}}"#);
        for native in [true, false] {
            assert_eq!(
                glass.tint(native),
                1.,
                "the tint never exceeds the surface colour"
            );
        }
        assert_eq!(glass.blur_radius, px(0.));
    }

    #[test]
    fn an_empty_setting_keeps_the_material_defaults() {
        assert_eq!(from_json("{}"), GlassSettings::default());
        assert_eq!(from_json(r#"{"glass": {}}"#), GlassSettings::default());
    }

    #[test]
    fn setting_only_the_blur_keeps_each_materials_tint() {
        let glass = from_json(r#"{"glass": {"blur_radius": 8}}"#);
        assert_eq!(glass.blur_radius, px(8.));
        assert_eq!(
            (glass.native_tint, glass.emulated_tint),
            (
                GlassSettings::default().native_tint,
                GlassSettings::default().emulated_tint
            )
        );
    }
}
