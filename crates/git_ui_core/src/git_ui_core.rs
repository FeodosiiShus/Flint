use std::rc::Rc;

use gpui::{
    AnyView, App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, Global,
    IntoElement, ManagedView, Render, Subscription, WeakEntity, Window,
};
use project::{ProjectPath, git_store::Repository};
use workspace::Workspace;

pub mod askpass_modal;
pub mod file_diff_view;
pub mod notifications;
pub mod worktree_names;
pub mod worktree_picker;
pub mod worktree_service;

/// A type-erased picker view, e.g. for the title bar's branch popover.
///
/// The concrete picker lives in a higher-level crate, which installs a
/// builder via [`set_branch_picker_builder`]; keeping only this wrapper here
/// lets consumers avoid depending on that crate.
pub struct GitPickerPopover {
    view: AnyView,
    focus_handle: FocusHandle,
    _dismiss_subscription: Subscription,
}

impl GitPickerPopover {
    pub fn new<V: ManagedView>(view: Entity<V>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: view.focus_handle(cx),
            _dismiss_subscription: cx
                .subscribe(&view, |_, _, _: &DismissEvent, cx| cx.emit(DismissEvent)),
            view: view.into(),
        }
    }
}

impl EventEmitter<DismissEvent> for GitPickerPopover {}

impl Focusable for GitPickerPopover {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GitPickerPopover {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.view.clone()
    }
}

type BranchPickerBuilder = dyn Fn(
    WeakEntity<Workspace>,
    Option<Entity<Repository>>,
    &mut Window,
    &mut App,
) -> Entity<GitPickerPopover>;

struct BranchPickerBuilderGlobal(Rc<BranchPickerBuilder>);

impl Global for BranchPickerBuilderGlobal {}

pub fn set_branch_picker_builder(
    builder: impl Fn(
        WeakEntity<Workspace>,
        Option<Entity<Repository>>,
        &mut Window,
        &mut App,
    ) -> Entity<GitPickerPopover>
    + 'static,
    cx: &mut App,
) {
    cx.set_global(BranchPickerBuilderGlobal(Rc::new(builder)));
}

pub fn build_branch_picker(
    workspace: WeakEntity<Workspace>,
    repository: Option<Entity<Repository>>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<GitPickerPopover>> {
    let builder = cx.try_global::<BranchPickerBuilderGlobal>()?.0.clone();
    Some(builder(workspace, repository, window, cx))
}

type FileHistoryOpener = dyn Fn(&mut Workspace, &ProjectPath, &mut Window, &mut Context<Workspace>);

struct FileHistoryOpenerGlobal(Rc<FileHistoryOpener>);

impl Global for FileHistoryOpenerGlobal {}

pub fn set_file_history_opener(
    opener: impl Fn(&mut Workspace, &ProjectPath, &mut Window, &mut Context<Workspace>) + 'static,
    cx: &mut App,
) {
    cx.set_global(FileHistoryOpenerGlobal(Rc::new(opener)));
}

/// Opens the file history view for `path`, if an opener was installed via
/// [`set_file_history_opener`].
pub fn open_file_history(
    workspace: &mut Workspace,
    path: &ProjectPath,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(opener) = cx
        .try_global::<FileHistoryOpenerGlobal>()
        .map(|opener| opener.0.clone())
    else {
        return;
    };
    opener(workspace, path, window, cx);
}

/// IntelliJ leaves branch names up to this many characters over the limit untouched.
const BRANCH_NAME_LENGTH_DELTA: usize = 4;
/// IntelliJ keeps this many trailing characters of a shortened branch name.
const BRANCH_NAME_SUFFIX_LENGTH: usize = 5;

/// `GitBranchPresentation.truncateBranchName`: a name longer than `max_length` (with a few
/// characters of slack) is shortened to `max_length` characters with an ellipsis in the middle,
/// keeping its last characters. Flint has no issue navigation patterns, so no issue id is
/// protected from the cut.
pub fn truncate_branch_name(name: &str, max_length: usize) -> String {
    let length = name.chars().count();
    if length <= max_length + BRANCH_NAME_LENGTH_DELTA {
        return name.to_string();
    }
    let shortened_length = max_length.saturating_sub(1);
    let Some(prefix_length) = shortened_length.checked_sub(BRANCH_NAME_SUFFIX_LENGTH + 1) else {
        return name.to_string();
    };
    let prefix: String = name.chars().take(prefix_length).collect();
    let suffix: String = name
        .chars()
        .skip(length - BRANCH_NAME_SUFFIX_LENGTH)
        .collect();
    format!("{prefix}\u{2026}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_branch_names_and_names_within_the_slack_are_kept() {
        assert_eq!(truncate_branch_name("main", 80), "main");
        let within_slack = "a".repeat(84);
        assert_eq!(truncate_branch_name(&within_slack, 80), within_slack);
    }

    #[test]
    fn long_branch_names_get_a_middle_ellipsis_keeping_the_last_five_characters() {
        let name = format!("feature/{}-12345", "x".repeat(90));
        let truncated = truncate_branch_name(&name, 80);
        assert_eq!(truncated.chars().count(), 79);
        assert!(truncated.starts_with("feature/xxx"));
        assert!(truncated.ends_with("\u{2026}12345"));
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        let name = "\u{e9}".repeat(100);
        let truncated = truncate_branch_name(&name, 80);
        assert_eq!(truncated.chars().count(), 79);
        assert!(truncated.ends_with(&format!("\u{2026}{}", "\u{e9}".repeat(5))));
    }

    #[test]
    fn a_limit_too_small_for_the_suffix_keeps_the_name() {
        let name = "a".repeat(20);
        assert_eq!(truncate_branch_name(&name, 3), name);
    }
}
