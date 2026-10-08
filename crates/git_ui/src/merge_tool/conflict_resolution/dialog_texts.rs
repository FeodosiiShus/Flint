use gpui::SharedString;

use super::rich_text::RichText;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ShowDetails {
    Commit {
        sha: SharedString,
        dialog_title: SharedString,
    },
    CommitRange {
        first: SharedString,
        second: SharedString,
        dialog_title: SharedString,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PaneTitleText {
    pub(crate) label: RichText,
    pub(crate) show_details: Option<ShowDetails>,
}

impl PaneTitleText {
    pub(crate) fn plain(label: impl Into<String>) -> Self {
        Self {
            label: RichText::plain(label),
            show_details: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PaneTitleTexts {
    pub(crate) left: PaneTitleText,
    pub(crate) result: PaneTitleText,
    pub(crate) right: PaneTitleText,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DialogTexts {
    pub(crate) dialog_title: SharedString,
    pub(crate) yours_column: SharedString,
    pub(crate) theirs_column: SharedString,
    pub(crate) pane_titles: PaneTitleTexts,
}
