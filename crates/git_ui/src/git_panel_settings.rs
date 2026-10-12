use gpui::Pixels;
use settings::{GitPanelGroupBy, GitPanelSortBy, IntoGpui, RegisterSetting, Settings, StatusStyle};
use workspace::dock::DockPosition;

#[derive(Debug, Clone, PartialEq, RegisterSetting)]
pub struct GitPanelSettings {
    pub button: bool,
    pub dock: DockPosition,
    pub default_width: Pixels,
    pub default_height: Pixels,
    pub status_style: StatusStyle,
    pub fallback_branch_name: String,
    pub sort_by: GitPanelSortBy,
    pub group_by: GitPanelGroupBy,
    pub collapse_untracked_diff: bool,
    pub tree_view: bool,
    pub show_count_badge: bool,
    pub starts_open: bool,
    pub commit_title_max_length: usize,
}

impl Settings for GitPanelSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let git_panel = content.git_panel.clone().unwrap();
        Self {
            button: git_panel.button.unwrap(),
            dock: git_panel.dock.unwrap().into(),
            default_width: git_panel.default_width.unwrap().into_gpui(),
            default_height: git_panel.default_height.unwrap().into_gpui(),
            status_style: git_panel.status_style.unwrap(),
            fallback_branch_name: git_panel.fallback_branch_name.unwrap(),
            sort_by: git_panel.sort_by.unwrap(),
            group_by: git_panel.group_by.unwrap(),
            collapse_untracked_diff: git_panel.collapse_untracked_diff.unwrap(),
            tree_view: git_panel.tree_view.unwrap(),
            show_count_badge: git_panel.show_count_badge.unwrap(),
            starts_open: git_panel.starts_open.unwrap(),
            commit_title_max_length: git_panel.commit_title_max_length.unwrap(),
        }
    }
}
