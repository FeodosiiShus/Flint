use serde::{Deserialize, Serialize};

pub(crate) const MAIN_LOG_TAB_ID: &str = "MAIN";
pub(crate) const USER_FILTER_ME: &str = "*";
pub(crate) const USER_FILTER_ME_LABEL: &str = "me";
pub(crate) const HEAD_REVISION: &str = "HEAD";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum GraphSortType {
    #[default]
    Normal,
    Bek,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum GraphOptions {
    Base(GraphSortType),
    LinearBek,
    FirstParent,
}

impl Default for GraphOptions {
    fn default() -> Self {
        Self::Base(GraphSortType::Normal)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum LogColumn {
    Root,
    Commit,
    Author,
    Date,
    Hash,
}

impl LogColumn {
    pub(crate) const DEFAULT_ORDER: [LogColumn; 4] = [
        LogColumn::Root,
        LogColumn::Commit,
        LogColumn::Author,
        LogColumn::Date,
    ];

    pub(crate) fn title(self) -> &'static str {
        match self {
            LogColumn::Root => "",
            LogColumn::Commit => "Subject",
            LogColumn::Author => "Author",
            LogColumn::Date => "Date",
            LogColumn::Hash => "Hash",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum BranchesSingleClickAction {
    #[default]
    None,
    UpdateBranchFilter,
    NavigateLog,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct LogAppProperties {
    pub compact_references: bool,
    pub show_tag_names: bool,
    pub labels_left_aligned: bool,
    pub prefer_commit_date: bool,
    pub show_changes_from_parents: bool,
    pub column_order: Vec<LogColumn>,
    pub branches_splitter_proportion: f32,
    pub changes_splitter_proportion: f32,
    pub changes_group_by_directory: bool,
    pub details_splitter_proportion: f32,
}

impl Default for LogAppProperties {
    fn default() -> Self {
        Self {
            compact_references: true,
            show_tag_names: false,
            labels_left_aligned: false,
            prefer_commit_date: false,
            show_changes_from_parents: false,
            column_order: LogColumn::DEFAULT_ORDER.to_vec(),
            branches_splitter_proportion: 0.3,
            changes_splitter_proportion: 0.7,
            details_splitter_proportion: 0.7,
            changes_group_by_directory: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct LogFilterState {
    pub branches: Vec<String>,
    pub users: Vec<String>,
    pub date_after: Option<i64>,
    pub date_before: Option<i64>,
    pub paths: Vec<String>,
    pub text: String,
}

impl LogFilterState {
    pub(crate) fn is_empty(&self) -> bool {
        self.branches.is_empty()
            && self.users.is_empty()
            && self.date_after.is_none()
            && self.date_before.is_none()
            && self.paths.is_empty()
            && self.text.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct LogTabProperties {
    pub show_details: bool,
    pub show_long_edges: bool,
    pub graph_options: GraphOptions,
    pub show_root_names: bool,
    pub show_only_affected_changes: bool,
    pub highlight_current_branch: bool,
    pub highlight_merge_commits: bool,
    pub highlight_my_commits: bool,
    pub filters: LogFilterState,
    pub text_filter_regex: bool,
    pub text_filter_match_case: bool,
    pub column_widths: Vec<(LogColumn, f32)>,
    pub show_branches: bool,
    pub branches_single_click: BranchesSingleClickAction,
}

impl LogTabProperties {
    pub(crate) fn new(is_main_tab: bool) -> Self {
        Self {
            show_branches: is_main_tab,
            ..Self::default()
        }
    }
}

impl Default for LogTabProperties {
    fn default() -> Self {
        Self {
            show_details: true,
            show_long_edges: false,
            graph_options: GraphOptions::default(),
            show_root_names: false,
            show_only_affected_changes: false,
            highlight_current_branch: true,
            highlight_merge_commits: true,
            highlight_my_commits: true,
            filters: LogFilterState::default(),
            text_filter_regex: false,
            text_filter_match_case: false,
            column_widths: Vec::new(),
            show_branches: false,
            branches_single_click: BranchesSingleClickAction::None,
        }
    }
}
