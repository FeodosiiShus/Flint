use std::{num::NonZeroUsize, path::PathBuf, time::Duration};

use crate::DockPosition;
use collections::HashMap;
use gpui::{App, Pixels, Subscription, px};
use serde::Deserialize;
pub use settings::{
    AutosaveSetting, BottomDockLayout, InactiveOpacity, PaneSplitDirectionHorizontal,
    PaneSplitDirectionVertical, RegisterSetting, RestoreOnStartupBehavior, Settings,
};
use settings::{CommandAliasTarget, SettingsStore};

#[derive(RegisterSetting)]
pub struct WorkspaceSettings {
    pub active_pane_modifiers: ActivePanelModifiers,
    pub bottom_dock_layout: settings::BottomDockLayout,
    pub pane_split_direction_horizontal: settings::PaneSplitDirectionHorizontal,
    pub pane_split_direction_vertical: settings::PaneSplitDirectionVertical,
    pub centered_layout: settings::CenteredLayoutSettings,
    pub confirm_quit: bool,
    pub autosave: AutosaveSetting,
    pub restore_on_startup: settings::RestoreOnStartupBehavior,
    pub cli_default_open_behavior: settings::CliDefaultOpenBehavior,
    pub default_open_behavior: settings::DefaultOpenBehavior,
    pub restore_on_file_reopen: bool,
    pub reveal_if_open: bool,
    pub drop_target_size: f32,
    pub use_system_path_prompts: bool,
    pub use_system_prompts: bool,
    pub accessible_mode: bool,
    pub command_aliases: HashMap<String, CommandAliasTarget>,
    pub max_tabs: Option<NonZeroUsize>,
    pub when_closing_with_no_tabs: settings::CloseWindowWhenNoItems,
    pub on_new_window: settings::OnNewWindow,
    pub on_last_window_closed: settings::OnLastWindowClosed,
    pub text_rendering_mode: settings::TextRenderingMode,
    pub resize_all_panels_in_dock: Vec<DockPosition>,
    pub close_on_file_delete: bool,
    pub close_panel_on_toggle: bool,
    pub window_title_format: String,
    pub window_title_separator: String,
    pub use_system_window_tabs: bool,
    pub fullscreen_mode: settings::FullscreenMode,
    pub zoomed_padding: bool,
    pub window_decorations: settings::WindowDecorations,
    pub focus_follows_mouse: FocusFollowsMouse,
    pub islands: IslandsSettings,
    pub tool_window_bars: ToolWindowBarsSettings,
    pub tool_window_headers: ToolWindowHeadersSettings,
}

const ISLANDS_GAP_RANGE: (u32, u32) = (0, 16);
const ISLANDS_CORNER_RADIUS_RANGE: (u32, u32) = (0, 24);
const TOOL_WINDOW_BAR_ICON_SIZE_RANGE: (u32, u32) = (12, 32);
const TOOL_WINDOW_BAR_BUTTON_PADDING: Pixels = px(20.);
pub const INACTIVE_FRAME_CONTENT_OPACITY: f32 = 0.56;

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct IslandsSettings {
    pub enabled: bool,
    pub gap: Pixels,
    pub corner_radius: Pixels,
    pub dim_inactive_window: bool,
}

impl IslandsSettings {
    fn from_content(
        content: Option<&settings::IslandsSettingsContent>,
        density: Option<settings::UiDensity>,
    ) -> Self {
        let compact = is_compact(density);
        let default_gap = if compact { 3 } else { 4 };
        let default_corner_radius = if compact { 8 } else { 10 };
        Self {
            enabled: content.and_then(|islands| islands.enabled).unwrap_or(true),
            gap: clamped_pixels(
                content.and_then(|islands| islands.gap),
                default_gap,
                ISLANDS_GAP_RANGE,
            ),
            corner_radius: clamped_pixels(
                content.and_then(|islands| islands.corner_radius),
                default_corner_radius,
                ISLANDS_CORNER_RADIUS_RANGE,
            ),
            dim_inactive_window: content
                .and_then(|islands| islands.dim_inactive_window)
                .unwrap_or(true),
        }
    }

    pub fn half_gap(&self) -> Pixels {
        self.gap / 2.
    }

    pub fn frame_content_opacity(&self, window_active: bool) -> f32 {
        if self.enabled && self.dim_inactive_window && !window_active {
            INACTIVE_FRAME_CONTENT_OPACITY
        } else {
            1.0
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ToolWindowBarsSettings {
    pub show: bool,
    pub icon_size: Pixels,
    pub show_names: bool,
    pub icon_style: settings::ToolWindowIconStyle,
}

impl ToolWindowBarsSettings {
    fn from_content(
        content: Option<&settings::ToolWindowBarsSettingsContent>,
        density: Option<settings::UiDensity>,
    ) -> Self {
        let default_icon_size = if is_compact(density) { 16 } else { 20 };
        Self {
            show: content.and_then(|bars| bars.show).unwrap_or(true),
            icon_size: clamped_pixels(
                content.and_then(|bars| bars.icon_size),
                default_icon_size,
                TOOL_WINDOW_BAR_ICON_SIZE_RANGE,
            ),
            show_names: content.and_then(|bars| bars.show_names).unwrap_or(false),
            icon_style: content.and_then(|bars| bars.icon_style).unwrap_or_default(),
        }
    }

    pub fn bar_width(&self) -> Pixels {
        self.icon_size + TOOL_WINDOW_BAR_BUTTON_PADDING
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ToolWindowHeadersSettings {
    pub show: bool,
    pub always_show_actions: bool,
}

impl ToolWindowHeadersSettings {
    fn from_content(content: Option<&settings::ToolWindowHeadersSettingsContent>) -> Self {
        Self {
            show: content.and_then(|headers| headers.show).unwrap_or(true),
            always_show_actions: content
                .and_then(|headers| headers.always_show_actions)
                .unwrap_or(false),
        }
    }
}

fn is_compact(density: Option<settings::UiDensity>) -> bool {
    density == Some(settings::UiDensity::Compact)
}

fn clamped_pixels(value: Option<u32>, default: u32, (min, max): (u32, u32)) -> Pixels {
    Pixels::from(value.unwrap_or(default).clamp(min, max))
}

#[cfg(target_os = "macos")]
pub fn closing_last_window_quits_app(cx: &App) -> bool {
    WorkspaceSettings::get_global(cx)
        .on_last_window_closed
        .is_quit_app()
}

#[cfg(not(target_os = "macos"))]
pub fn closing_last_window_quits_app(_cx: &App) -> bool {
    true
}

#[derive(Copy, Clone, Deserialize)]
pub struct FocusFollowsMouse {
    pub enabled: bool,
    pub debounce: Duration,
}

#[derive(Copy, Clone, PartialEq, Debug, Default)]
pub struct ActivePanelModifiers {
    /// Size of the border surrounding the active pane.
    /// When set to 0, the active pane doesn't have any border.
    /// The border is drawn inset.
    ///
    /// Default: `0.0`
    // TODO: make this not an option, it is never None
    pub border_size: Option<f32>,
    /// Opacity of inactive panels.
    /// When set to 1.0, the inactive panes have the same opacity as the active one.
    /// If set to 0, the inactive panes content will not be visible at all.
    /// Values are clamped to the [0.0, 1.0] range.
    ///
    /// Default: `1.0`
    // TODO: make this not an option, it is never None
    pub inactive_opacity: Option<InactiveOpacity>,
}

#[derive(Deserialize, RegisterSetting)]
pub struct TabBarSettings {
    pub show: bool,
    pub show_nav_history_buttons: bool,
    pub show_tab_bar_buttons: bool,
    pub show_pinned_tabs_in_separate_row: bool,
    pub show_hidden_tabs_button: bool,
}

impl Settings for WorkspaceSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let workspace = &content.workspace;
        Self {
            active_pane_modifiers: ActivePanelModifiers {
                border_size: Some(
                    *workspace
                        .active_pane_modifiers
                        .unwrap()
                        .border_size
                        .unwrap(),
                ),
                inactive_opacity: Some(
                    workspace
                        .active_pane_modifiers
                        .unwrap()
                        .inactive_opacity
                        .unwrap(),
                ),
            },
            bottom_dock_layout: workspace.bottom_dock_layout.unwrap(),
            pane_split_direction_horizontal: workspace.pane_split_direction_horizontal.unwrap(),
            pane_split_direction_vertical: workspace.pane_split_direction_vertical.unwrap(),
            centered_layout: workspace.centered_layout.unwrap(),
            confirm_quit: workspace.confirm_quit.unwrap(),
            autosave: workspace.autosave.unwrap(),
            restore_on_startup: workspace.restore_on_startup.unwrap(),
            cli_default_open_behavior: workspace.cli_default_open_behavior.unwrap(),
            default_open_behavior: workspace.default_open_behavior.unwrap(),
            restore_on_file_reopen: workspace.restore_on_file_reopen.unwrap(),
            reveal_if_open: workspace.reveal_if_open.unwrap(),
            drop_target_size: workspace.drop_target_size.unwrap(),
            use_system_path_prompts: workspace.use_system_path_prompts.unwrap(),
            use_system_prompts: workspace.use_system_prompts.unwrap(),
            accessible_mode: workspace.accessible_mode.unwrap(),
            command_aliases: workspace.command_aliases.clone(),
            max_tabs: workspace.max_tabs,
            when_closing_with_no_tabs: workspace.when_closing_with_no_tabs.unwrap(),
            on_new_window: workspace.on_new_window.unwrap(),
            on_last_window_closed: workspace.on_last_window_closed.unwrap(),
            text_rendering_mode: workspace.text_rendering_mode.unwrap(),
            resize_all_panels_in_dock: workspace
                .resize_all_panels_in_dock
                .clone()
                .unwrap()
                .into_iter()
                .map(Into::into)
                .collect(),
            close_on_file_delete: workspace.close_on_file_delete.unwrap(),
            close_panel_on_toggle: workspace.close_panel_on_toggle.unwrap(),
            window_title_format: workspace.window_title_format.clone().unwrap(),
            window_title_separator: workspace.window_title_separator.clone().unwrap(),
            use_system_window_tabs: workspace.use_system_window_tabs.unwrap(),
            fullscreen_mode: workspace.fullscreen_mode.unwrap(),
            zoomed_padding: workspace.zoomed_padding.unwrap(),
            window_decorations: workspace.window_decorations.unwrap(),
            focus_follows_mouse: FocusFollowsMouse {
                enabled: workspace
                    .focus_follows_mouse
                    .unwrap()
                    .enabled
                    .unwrap_or(false),
                debounce: Duration::from_millis(
                    workspace
                        .focus_follows_mouse
                        .unwrap()
                        .debounce_ms
                        .unwrap_or(250),
                ),
            },
            islands: IslandsSettings::from_content(
                workspace.islands.as_ref(),
                content.theme.ui_density,
            ),
            tool_window_bars: ToolWindowBarsSettings::from_content(
                workspace.tool_window_bars.as_ref(),
                content.theme.ui_density,
            ),
            tool_window_headers: ToolWindowHeadersSettings::from_content(
                workspace.tool_window_headers.as_ref(),
            ),
        }
    }
}

/// Provides convenient access to whether "accessible mode" is enabled, mirroring
/// [`theme::ActiveTheme`] for the active theme. Import this trait to call
/// `cx.accessible_mode()`.
pub trait AccessibleMode {
    /// Returns whether accessible mode is enabled.
    fn accessible_mode(&self) -> bool;
}

impl AccessibleMode for App {
    fn accessible_mode(&self) -> bool {
        WorkspaceSettings::get_global(self).accessible_mode
    }
}

/// Observes changes to the accessible-mode setting, invoking `callback` with the
/// new value whenever it changes. Mirrors the common
/// `cx.observe_global::<SettingsStore>` pattern, but only fires when the value
/// actually changes. The returned [`Subscription`] must be retained for the
/// callback to keep firing.
pub fn observe_accessible_mode(
    cx: &mut App,
    mut callback: impl FnMut(bool, &mut App) + 'static,
) -> Subscription {
    let mut last = cx.accessible_mode();
    cx.observe_global::<SettingsStore>(move |cx| {
        let current = cx.accessible_mode();
        if current != last {
            last = current;
            callback(current, cx);
        }
    })
}

impl Settings for TabBarSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let tab_bar = content.tab_bar.clone().unwrap();
        TabBarSettings {
            show: tab_bar.show.unwrap(),
            show_nav_history_buttons: tab_bar.show_nav_history_buttons.unwrap(),
            show_tab_bar_buttons: tab_bar.show_tab_bar_buttons.unwrap(),
            show_pinned_tabs_in_separate_row: tab_bar.show_pinned_tabs_in_separate_row.unwrap(),
            show_hidden_tabs_button: tab_bar.show_hidden_tabs_button.unwrap_or(true),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BackgroundImageLayerSettings {
    pub path: PathBuf,
    pub opacity: u32,
    pub fill: settings::BackgroundImageFill,
    pub anchor: settings::BackgroundImageAnchor,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
}

impl BackgroundImageLayerSettings {
    fn from_content(content: Option<&settings::BackgroundImageLayerContent>) -> Option<Self> {
        let content = content?;
        let trimmed_path = content.path.as_deref()?.trim();
        if trimmed_path.is_empty() {
            return None;
        }
        let opacity = content.opacity.unwrap_or_default();
        Some(Self {
            path: PathBuf::from(shellexpand::tilde(trimmed_path).into_owned()),
            opacity: opacity.0.min(settings::BackgroundImageOpacity::MAX),
            fill: content.fill.unwrap_or_default(),
            anchor: content.anchor.unwrap_or_default(),
            flip_horizontal: content.flip_horizontal.unwrap_or_default(),
            flip_vertical: content.flip_vertical.unwrap_or_default(),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, RegisterSetting)]
pub struct BackgroundImageSettings {
    pub editor_and_tools: Option<BackgroundImageLayerSettings>,
    pub empty_frame: Option<BackgroundImageLayerSettings>,
}

impl Settings for BackgroundImageSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let Some(background_image) = content.project.background_image.as_ref() else {
            return Self::default();
        };
        let editor_and_tools = background_image.editor_and_tools.as_ref();
        let empty_frame = background_image.empty_frame.as_ref();
        Self {
            editor_and_tools: BackgroundImageLayerSettings::from_content(editor_and_tools),
            empty_frame: BackgroundImageLayerSettings::from_content(empty_frame),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use settings::{
        IslandsSettingsContent, ToolWindowBarsSettingsContent, ToolWindowHeadersSettingsContent,
        UiDensity,
    };

    #[test]
    fn islands_use_webstorm_metrics_when_unset() {
        let islands = IslandsSettings::from_content(None, None);
        assert_eq!(
            islands,
            IslandsSettings {
                enabled: true,
                gap: px(4.),
                corner_radius: px(10.),
                dim_inactive_window: true,
            }
        );
        assert_eq!(islands.half_gap(), px(2.));

        let default_density = IslandsSettings::from_content(
            Some(&IslandsSettingsContent::default()),
            Some(UiDensity::Default),
        );
        assert_eq!(default_density, islands);

        let comfortable = IslandsSettings::from_content(None, Some(UiDensity::Comfortable));
        assert_eq!(
            comfortable, islands,
            "only compact density shrinks the island metrics"
        );
    }

    #[test]
    fn islands_shrink_in_compact_density() {
        let islands = IslandsSettings::from_content(None, Some(UiDensity::Compact));
        assert_eq!(islands.gap, px(3.));
        assert_eq!(islands.corner_radius, px(8.));
        assert_eq!(islands.half_gap(), px(1.5));
    }

    #[test]
    fn explicit_island_values_override_density_defaults() {
        let content = IslandsSettingsContent {
            enabled: Some(false),
            gap: Some(6),
            corner_radius: Some(12),
            dim_inactive_window: Some(false),
        };
        let islands = IslandsSettings::from_content(Some(&content), Some(UiDensity::Compact));
        assert_eq!(
            islands,
            IslandsSettings {
                enabled: false,
                gap: px(6.),
                corner_radius: px(12.),
                dim_inactive_window: false,
            }
        );
    }

    #[test]
    fn island_values_are_clamped() {
        let too_large = IslandsSettingsContent {
            enabled: None,
            gap: Some(17),
            corner_radius: Some(25),
            dim_inactive_window: None,
        };
        let islands = IslandsSettings::from_content(Some(&too_large), None);
        assert_eq!(islands.gap, px(16.));
        assert_eq!(islands.corner_radius, px(24.));

        let at_bounds = IslandsSettingsContent {
            enabled: None,
            gap: Some(0),
            corner_radius: Some(0),
            dim_inactive_window: None,
        };
        let islands = IslandsSettings::from_content(Some(&at_bounds), None);
        assert_eq!(islands.gap, px(0.));
        assert_eq!(islands.corner_radius, px(0.));
    }

    #[test]
    fn tool_window_bars_use_webstorm_metrics_when_unset() {
        let bars = ToolWindowBarsSettings::from_content(None, None);
        assert_eq!(
            bars,
            ToolWindowBarsSettings {
                show: true,
                icon_size: px(20.),
                show_names: false,
                icon_style: settings::ToolWindowIconStyle::Jetbrains,
            }
        );
        assert_eq!(bars.bar_width(), px(40.));

        let compact = ToolWindowBarsSettings::from_content(None, Some(UiDensity::Compact));
        assert_eq!(compact.icon_size, px(16.));
        assert_eq!(compact.bar_width(), px(36.));
    }

    #[test]
    fn tool_window_bar_icon_size_is_clamped() {
        let too_small = ToolWindowBarsSettingsContent {
            show: Some(false),
            icon_size: Some(11),
            show_names: Some(true),
            icon_style: Some(settings::ToolWindowIconStyle::Zed),
        };
        let bars = ToolWindowBarsSettings::from_content(Some(&too_small), None);
        assert_eq!(
            bars,
            ToolWindowBarsSettings {
                show: false,
                icon_size: px(12.),
                show_names: true,
                icon_style: settings::ToolWindowIconStyle::Zed,
            }
        );

        let too_large = ToolWindowBarsSettingsContent {
            icon_size: Some(33),
            ..ToolWindowBarsSettingsContent::default()
        };
        let bars = ToolWindowBarsSettings::from_content(Some(&too_large), Some(UiDensity::Compact));
        assert_eq!(bars.icon_size, px(32.));
        assert_eq!(bars.bar_width(), px(52.));
    }

    #[test]
    fn tool_window_bar_icon_style_defaults_to_jetbrains_when_content_is_missing() {
        let bars = ToolWindowBarsSettings::from_content(None, None);
        assert_eq!(bars.icon_style, settings::ToolWindowIconStyle::Jetbrains);
    }

    #[test]
    fn tool_window_bar_icon_style_honours_explicit_zed_value() {
        let content = ToolWindowBarsSettingsContent {
            icon_style: Some(settings::ToolWindowIconStyle::Zed),
            ..ToolWindowBarsSettingsContent::default()
        };
        let bars = ToolWindowBarsSettings::from_content(Some(&content), None);
        assert_eq!(bars.icon_style, settings::ToolWindowIconStyle::Zed);
    }

    #[test]
    fn tool_window_bar_icon_style_keeps_default_when_only_icon_size_is_set() {
        let content = ToolWindowBarsSettingsContent {
            icon_size: Some(24),
            ..ToolWindowBarsSettingsContent::default()
        };
        let bars = ToolWindowBarsSettings::from_content(Some(&content), None);
        assert_eq!(bars.icon_size, px(24.));
        assert_eq!(bars.icon_style, settings::ToolWindowIconStyle::Jetbrains);
    }

    #[test]
    fn island_frame_content_is_dimmed_only_while_the_window_is_inactive() {
        let islands = IslandsSettings::from_content(None, None);
        assert_eq!(islands.frame_content_opacity(true), 1.0);
        assert_eq!(islands.frame_content_opacity(false), 0.56);
    }

    #[test]
    fn island_frame_content_is_never_dimmed_when_islands_are_disabled() {
        let content = IslandsSettingsContent {
            enabled: Some(false),
            ..IslandsSettingsContent::default()
        };
        let islands = IslandsSettings::from_content(Some(&content), None);
        assert!(islands.dim_inactive_window);
        assert_eq!(islands.frame_content_opacity(false), 1.0);
        assert_eq!(islands.frame_content_opacity(true), 1.0);
    }

    #[test]
    fn island_frame_content_is_never_dimmed_when_dimming_is_turned_off() {
        let content = IslandsSettingsContent {
            dim_inactive_window: Some(false),
            ..IslandsSettingsContent::default()
        };
        let islands = IslandsSettings::from_content(Some(&content), None);
        assert!(islands.enabled);
        assert_eq!(islands.frame_content_opacity(false), 1.0);
        assert_eq!(islands.frame_content_opacity(true), 1.0);
    }

    #[test]
    fn tool_window_headers_show_with_hover_only_actions_when_unset() {
        let expected = ToolWindowHeadersSettings {
            show: true,
            always_show_actions: false,
        };
        assert_eq!(ToolWindowHeadersSettings::from_content(None), expected);

        let unset = ToolWindowHeadersSettingsContent::default();
        assert_eq!(
            ToolWindowHeadersSettings::from_content(Some(&unset)),
            expected
        );
    }

    #[test]
    fn explicit_tool_window_header_values_override_defaults() {
        let content = ToolWindowHeadersSettingsContent {
            show: Some(false),
            always_show_actions: Some(true),
        };
        assert_eq!(
            ToolWindowHeadersSettings::from_content(Some(&content)),
            ToolWindowHeadersSettings {
                show: false,
                always_show_actions: true,
            }
        );
    }
}
