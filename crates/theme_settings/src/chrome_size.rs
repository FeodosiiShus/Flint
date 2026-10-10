use gpui::Pixels;
use settings::{RegisterSetting, Settings, SettingsContent};
use theme::ChromeSizes;

const MIN_HEIGHT: u32 = 24;
const MAX_HEIGHT: u32 = 64;
const MIN_ICON_SIZE: u32 = 10;
const MAX_ICON_SIZE: u32 = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, RegisterSetting)]
pub(crate) struct ChromeSizeSettings(pub(crate) ChromeSizes);

impl Settings for ChromeSizeSettings {
    fn from_settings(content: &SettingsContent) -> Self {
        let title_bar = content.title_bar.as_ref();
        let tab_bar = content.tab_bar.as_ref();
        let toolbar = content.editor.toolbar.as_ref();
        let panel = content.panel.as_ref();
        Self(ChromeSizes {
            title_bar_height: clamp_height(title_bar.and_then(|bar| bar.height)),
            title_bar_icon_size: clamp_icon_size(title_bar.and_then(|bar| bar.icon_size)),
            tab_bar_height: clamp_height(tab_bar.and_then(|bar| bar.height)),
            tab_bar_icon_size: clamp_icon_size(tab_bar.and_then(|bar| bar.icon_size)),
            toolbar_height: clamp_height(toolbar.and_then(|bar| bar.height)),
            toolbar_icon_size: clamp_icon_size(toolbar.and_then(|bar| bar.icon_size)),
            panel_height: clamp_height(panel.and_then(|panel| panel.height)),
            panel_icon_size: clamp_icon_size(panel.and_then(|panel| panel.icon_size)),
        })
    }
}

fn clamp_height(value: Option<u32>) -> Option<Pixels> {
    clamp_pixels(value, MIN_HEIGHT, MAX_HEIGHT)
}

fn clamp_icon_size(value: Option<u32>) -> Option<Pixels> {
    clamp_pixels(value, MIN_ICON_SIZE, MAX_ICON_SIZE)
}

fn clamp_pixels(value: Option<u32>, min: u32, max: u32) -> Option<Pixels> {
    value.map(|value| Pixels::from(value.clamp(min, max)))
}

#[cfg(test)]
mod tests {
    use super::*;

    use gpui::px;
    use settings::{
        EditorSettingsContent, PanelChromeSettingsContent, TabBarSettingsContent,
        TitleBarSettingsContent, ToolbarContent,
    };

    fn chrome_sizes(content: &SettingsContent) -> ChromeSizes {
        ChromeSizeSettings::from_settings(content).0
    }

    fn uniform_content(height: Option<u32>, icon_size: Option<u32>) -> SettingsContent {
        SettingsContent {
            title_bar: Some(TitleBarSettingsContent {
                height,
                icon_size,
                ..Default::default()
            }),
            tab_bar: Some(TabBarSettingsContent {
                height,
                icon_size,
                ..Default::default()
            }),
            panel: Some(PanelChromeSettingsContent { height, icon_size }),
            editor: EditorSettingsContent {
                toolbar: Some(ToolbarContent {
                    height,
                    icon_size,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn uniform_sizes(height: Option<u32>, icon_size: Option<u32>) -> ChromeSizes {
        let height = height.map(Pixels::from);
        let icon_size = icon_size.map(Pixels::from);
        ChromeSizes {
            title_bar_height: height,
            title_bar_icon_size: icon_size,
            tab_bar_height: height,
            tab_bar_icon_size: icon_size,
            toolbar_height: height,
            toolbar_icon_size: icon_size,
            panel_height: height,
            panel_icon_size: icon_size,
        }
    }

    fn assert_height_clamped(value: u32, clamped: u32) {
        let sizes = chrome_sizes(&uniform_content(Some(value), None));
        let expected = uniform_sizes(Some(clamped), None);
        assert_eq!(sizes, expected, "height {value}");
    }

    fn assert_icon_size_clamped(value: u32, clamped: u32) {
        let sizes = chrome_sizes(&uniform_content(None, Some(value)));
        let expected = uniform_sizes(None, Some(clamped));
        assert_eq!(sizes, expected, "icon_size {value}");
    }

    #[test]
    fn chrome_size_is_unset_for_default_settings_content() {
        let sizes = chrome_sizes(&SettingsContent::default());
        assert_eq!(sizes, ChromeSizes::default());
    }

    #[test]
    fn chrome_size_is_unset_for_default_settings_file() -> anyhow::Result<()> {
        let json = settings::default_settings();
        let content: SettingsContent = settings::parse_json_with_comments(&json)?;
        assert_eq!(chrome_sizes(&content), ChromeSizes::default());
        Ok(())
    }

    #[test]
    fn chrome_size_is_unset_for_groups_with_none_fields() {
        let sizes = chrome_sizes(&uniform_content(None, None));
        assert_eq!(sizes, ChromeSizes::default());
    }

    #[test]
    fn chrome_size_is_unset_for_explicit_json_nulls() -> anyhow::Result<()> {
        let json = r#"{
            "title_bar": { "height": null, "icon_size": null },
            "tab_bar": { "height": null, "icon_size": null },
            "toolbar": { "height": null, "icon_size": null },
            "panel": { "height": null, "icon_size": null }
        }"#;
        let content: SettingsContent = settings::parse_json_with_comments(json)?;
        assert_eq!(chrome_sizes(&content), ChromeSizes::default());
        Ok(())
    }

    #[test]
    fn chrome_size_clamps_height_below_minimum() {
        assert_height_clamped(0, 24);
        assert_height_clamped(23, 24);
    }

    #[test]
    fn chrome_size_keeps_height_within_bounds() {
        assert_height_clamped(24, 24);
        assert_height_clamped(40, 40);
        assert_height_clamped(64, 64);
    }

    #[test]
    fn chrome_size_clamps_height_above_maximum() {
        assert_height_clamped(65, 64);
        assert_height_clamped(u32::MAX, 64);
    }

    #[test]
    fn chrome_size_clamps_icon_size_below_minimum() {
        assert_icon_size_clamped(0, 10);
        assert_icon_size_clamped(9, 10);
    }

    #[test]
    fn chrome_size_keeps_icon_size_within_bounds() {
        assert_icon_size_clamped(10, 10);
        assert_icon_size_clamped(20, 20);
        assert_icon_size_clamped(32, 32);
    }

    #[test]
    fn chrome_size_clamps_icon_size_above_maximum() {
        assert_icon_size_clamped(33, 32);
        assert_icon_size_clamped(u32::MAX, 32);
    }

    #[test]
    fn chrome_size_maps_title_bar_group() {
        let content = SettingsContent {
            title_bar: Some(TitleBarSettingsContent {
                height: Some(40),
                icon_size: Some(18),
                ..Default::default()
            }),
            ..Default::default()
        };
        let expected = ChromeSizes {
            title_bar_height: Some(px(40.)),
            title_bar_icon_size: Some(px(18.)),
            ..Default::default()
        };
        assert_eq!(chrome_sizes(&content), expected);
    }

    #[test]
    fn chrome_size_maps_tab_bar_group() {
        let content = SettingsContent {
            tab_bar: Some(TabBarSettingsContent {
                height: Some(36),
                icon_size: Some(16),
                ..Default::default()
            }),
            ..Default::default()
        };
        let expected = ChromeSizes {
            tab_bar_height: Some(px(36.)),
            tab_bar_icon_size: Some(px(16.)),
            ..Default::default()
        };
        assert_eq!(chrome_sizes(&content), expected);
    }

    #[test]
    fn chrome_size_maps_toolbar_group_from_editor_toolbar() {
        let content = SettingsContent {
            editor: EditorSettingsContent {
                toolbar: Some(ToolbarContent {
                    height: Some(44),
                    icon_size: Some(20),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let expected = ChromeSizes {
            toolbar_height: Some(px(44.)),
            toolbar_icon_size: Some(px(20.)),
            ..Default::default()
        };
        assert_eq!(chrome_sizes(&content), expected);
    }

    #[test]
    fn chrome_size_maps_panel_group() {
        let content = SettingsContent {
            panel: Some(PanelChromeSettingsContent {
                height: Some(34),
                icon_size: Some(18),
            }),
            ..Default::default()
        };
        let expected = ChromeSizes {
            panel_height: Some(px(34.)),
            panel_icon_size: Some(px(18.)),
            ..Default::default()
        };
        assert_eq!(chrome_sizes(&content), expected);
    }

    #[test]
    fn chrome_size_panel_group_is_independent_of_tab_bar() {
        let content = SettingsContent {
            tab_bar: Some(TabBarSettingsContent {
                height: Some(40),
                icon_size: Some(20),
                ..Default::default()
            }),
            panel: Some(PanelChromeSettingsContent {
                height: Some(10),
                icon_size: None,
            }),
            ..Default::default()
        };
        let expected = ChromeSizes {
            tab_bar_height: Some(px(40.)),
            tab_bar_icon_size: Some(px(20.)),
            panel_height: Some(px(24.)),
            panel_icon_size: None,
            ..Default::default()
        };
        assert_eq!(chrome_sizes(&content), expected);
    }

    #[test]
    fn chrome_size_maps_every_group_from_json() -> anyhow::Result<()> {
        let json = r#"{
            "title_bar": { "height": 40, "icon_size": 18 },
            "tab_bar": { "height": 36, "icon_size": 16 },
            "toolbar": { "height": 100, "icon_size": 20 },
            "panel": { "height": 50, "icon_size": 40 }
        }"#;
        let content: SettingsContent = settings::parse_json_with_comments(json)?;
        let expected = ChromeSizes {
            title_bar_height: Some(px(40.)),
            title_bar_icon_size: Some(px(18.)),
            tab_bar_height: Some(px(36.)),
            tab_bar_icon_size: Some(px(16.)),
            toolbar_height: Some(px(64.)),
            toolbar_icon_size: Some(px(20.)),
            panel_height: Some(px(50.)),
            panel_icon_size: Some(px(32.)),
        };
        assert_eq!(chrome_sizes(&content), expected);
        Ok(())
    }
}
