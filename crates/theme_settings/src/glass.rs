use gpui::{Pixels, px};
use settings::{RegisterSetting, Settings, SettingsContent};

/// How much of the theme's surface colour covers the glass when nothing is set, in percent.
pub const DEFAULT_GLASS_TINT_PERCENT: u32 = 20;
/// How far the emulated blur spreads when nothing is set.
pub const DEFAULT_GLASS_BLUR_RADIUS: Pixels = px(20.);
const MAX_BLUR_RADIUS: f32 = 64.;

/// The Liquid Glass material shared by popups, dialogs and glass tool windows: the theme's surface
/// colour laid over a blurred backdrop. On macOS 26 the system draws the blur, so only the tint
/// reaches it; elsewhere the renderer draws both.
#[derive(Clone, Copy, Debug, PartialEq, RegisterSetting)]
pub struct GlassSettings {
    /// How much of the theme's surface colour covers the glass, from 0 to 1.
    pub tint: f32,
    /// How far the renderer's emulated blur spreads; macOS 26 blurs with the system's own.
    pub blur_radius: Pixels,
}

impl Default for GlassSettings {
    fn default() -> Self {
        Self {
            tint: DEFAULT_GLASS_TINT_PERCENT as f32 / 100.,
            blur_radius: DEFAULT_GLASS_BLUR_RADIUS,
        }
    }
}

impl Settings for GlassSettings {
    fn from_settings(content: &SettingsContent) -> Self {
        let content = content.glass.as_ref();
        let defaults = Self::default();
        Self {
            tint: content
                .and_then(|glass| glass.tint)
                .map_or(defaults.tint, |tint| tint.min(100) as f32 / 100.),
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
    fn the_glass_defaults_to_a_light_tint_over_the_system_material() {
        let glass = GlassSettings::default();
        assert_eq!(glass.tint, 0.2);
        assert_eq!(glass.blur_radius, px(20.));
    }

    #[test]
    fn the_tint_is_a_percentage_and_the_blur_is_clamped() {
        let glass = from_json(r#"{"glass": {"tint": 55, "blur_radius": 500}}"#);
        assert!((glass.tint - 0.55).abs() < f32::EPSILON);
        assert_eq!(
            glass.blur_radius,
            px(MAX_BLUR_RADIUS),
            "a larger blur than the renderer supports is clamped"
        );

        let glass = from_json(r#"{"glass": {"tint": 250, "blur_radius": -3}}"#);
        assert_eq!(glass.tint, 1., "the tint never exceeds the surface colour");
        assert_eq!(glass.blur_radius, px(0.));
    }

    #[test]
    fn an_empty_setting_keeps_the_material_defaults() {
        assert_eq!(from_json("{}"), GlassSettings::default());
        assert_eq!(from_json(r#"{"glass": {}}"#), GlassSettings::default());
    }
}
