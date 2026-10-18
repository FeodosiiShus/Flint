use gpui::{App, Pixels, px};
use theme::UiDensity;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComponentCornerRadii {
    pub button: Pixels,
    pub input: Pixels,
    pub checkbox: Pixels,
    pub selection: Pixels,
    pub toolbar_widget: Pixels,
    pub banner: Pixels,
}

pub const COMPONENT_CORNER_RADII: ComponentCornerRadii = ComponentCornerRadii {
    button: px(4.),
    input: px(4.),
    checkbox: px(3.),
    selection: px(4.),
    toolbar_widget: px(6.),
    banner: px(6.),
};

impl ComponentCornerRadii {
    /// IntelliJ's Compact Mode copies every `.compact` key over its base key: `Component.arc` 8
    /// becomes 6 and `Banner.arc` 12 becomes 8. `MainToolbar.Dropdown.arc` has no compact key.
    pub fn for_density(density: UiDensity) -> Self {
        match density {
            UiDensity::Compact => Self {
                input: px(3.),
                banner: px(4.),
                ..COMPONENT_CORNER_RADII
            },
            UiDensity::Default | UiDensity::Comfortable => COMPONENT_CORNER_RADII,
        }
    }
}

pub fn component_corner_radii(cx: &App) -> ComponentCornerRadii {
    ComponentCornerRadii::for_density(theme::theme_settings(cx).ui_density(cx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_corner_radii_use_the_islands_compact_arcs_in_compact_density() {
        let compact = ComponentCornerRadii::for_density(UiDensity::Compact);
        assert_eq!(compact.input, px(3.), "Component.arc.compact 6");
        assert_eq!(compact.banner, px(4.), "Banner.arc.compact 8");
        assert_eq!(
            compact.toolbar_widget,
            px(6.),
            "MainToolbar.Dropdown.arc 12 has no compact key"
        );
        assert_eq!(compact.button, px(4.), "Button.arc has no compact key");
        assert_eq!(compact.checkbox, px(3.));
        assert_eq!(compact.selection, px(4.));
    }

    #[test]
    fn component_corner_radii_keep_the_regular_arcs_outside_compact_density() {
        for density in [UiDensity::Default, UiDensity::Comfortable] {
            assert_eq!(
                ComponentCornerRadii::for_density(density),
                COMPONENT_CORNER_RADII,
                "{density:?} uses the regular arcs"
            );
        }
        assert_eq!(COMPONENT_CORNER_RADII.input, px(4.), "Component.arc 8");
        assert_eq!(COMPONENT_CORNER_RADII.banner, px(6.), "Banner.arc 12");
    }
}
