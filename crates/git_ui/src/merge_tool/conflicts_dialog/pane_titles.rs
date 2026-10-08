use std::rc::Rc;

use git::repository::RepoPath;
use gpui::{App, Entity, SharedString, Window};
use project::git_store::Repository;

use super::commit_details::open_commit_details;
use crate::merge_tool::conflict_resolution::dialog_texts::{PaneTitleText, PaneTitleTexts};
use crate::merge_tool::merge_window::MergePaneTitle;

#[derive(Clone)]
pub(crate) struct TitleContext {
    pub(crate) repository: Entity<Repository>,
    pub(crate) file: RepoPath,
    pub(crate) absolute_path: SharedString,
}

fn side_title(text: &PaneTitleText, context: &TitleContext) -> MergePaneTitle {
    let title = MergePaneTitle::new(text.label.clone());
    match text.show_details.clone() {
        Some(details) => {
            let repository = context.repository.clone();
            let file = context.file.clone();
            title.with_show_details(Rc::new(move |window: &mut Window, cx: &mut App| {
                open_commit_details(
                    details.clone(),
                    repository.clone(),
                    file.clone(),
                    window,
                    cx,
                );
            }))
        }
        None => title,
    }
}

pub(crate) fn merge_pane_titles(
    texts: &PaneTitleTexts,
    context: &TitleContext,
) -> [MergePaneTitle; 3] {
    let result = MergePaneTitle::new(texts.result.label.clone()).with_detail(
        SharedString::from(context.file.as_unix_str().to_string()),
        context.absolute_path.clone(),
    );
    [
        side_title(&texts.left, context),
        result,
        side_title(&texts.right, context),
    ]
}
