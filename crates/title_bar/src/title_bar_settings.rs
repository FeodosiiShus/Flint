use gpui::WindowButtonLayout;
use settings::{RegisterSetting, Settings, SettingsContent};

#[derive(Copy, Clone, Debug, RegisterSetting)]
pub struct TitleBarSettings {
    pub show_branch_status_icon: bool,
    pub show_branch_name: bool,
    pub show_worktree_name: bool,
    pub show_project_items: bool,
    pub show_menus: bool,
    pub open_menus_on_hover: bool,
    pub button_layout: Option<WindowButtonLayout>,
    pub show_project_badge: bool,
    pub show_run_widget: bool,
    pub show_search_button: bool,
    pub show_settings_button: bool,
}

impl Settings for TitleBarSettings {
    fn from_settings(s: &SettingsContent) -> Self {
        let content = s.title_bar.clone().unwrap();
        TitleBarSettings {
            show_branch_status_icon: content.show_branch_status_icon.unwrap(),
            show_branch_name: content.show_branch_name.unwrap(),
            show_worktree_name: content.show_worktree_name.unwrap(),
            show_project_items: content.show_project_items.unwrap(),
            show_menus: content.show_menus.unwrap(),
            open_menus_on_hover: content.open_menus_on_hover.unwrap(),
            button_layout: content.button_layout.unwrap_or_default().into_layout(),
            show_project_badge: content.show_project_badge.unwrap_or(true),
            show_run_widget: content.show_run_widget.unwrap_or(true),
            show_search_button: content.show_search_button.unwrap_or(true),
            show_settings_button: content.show_settings_button.unwrap_or(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::App;
    use settings::SettingsStore;

    #[gpui::test]
    fn test_default_settings_enable_main_toolbar_widgets_and_hide_worktree_name(cx: &mut App) {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);

        let settings = TitleBarSettings::get_global(cx);

        assert!(settings.show_project_badge);
        assert!(settings.show_run_widget);
        assert!(settings.show_search_button);
        assert!(settings.show_settings_button);
        assert!(!settings.show_worktree_name);
        assert!(settings.show_branch_name);
    }

    #[gpui::test]
    fn test_user_settings_can_hide_each_main_toolbar_widget(cx: &mut App) {
        let mut settings_store = SettingsStore::test(cx);
        settings_store
            .set_user_settings(
                r#"{
                    "title_bar": {
                        "show_project_badge": false,
                        "show_run_widget": false,
                        "show_search_button": false,
                        "show_settings_button": false,
                        "show_worktree_name": true
                    }
                }"#,
                cx,
            )
            .result()
            .expect("user settings should parse");
        cx.set_global(settings_store);

        let settings = TitleBarSettings::get_global(cx);

        assert!(!settings.show_project_badge);
        assert!(!settings.show_run_widget);
        assert!(!settings.show_search_button);
        assert!(!settings.show_settings_button);
        assert!(settings.show_worktree_name);
    }
}
