use git::repository::RepositoryOperation;
use gpui::{Action, App, SharedString, Window};
use ui::IconName;

use super::tree;
use crate::branch_operations::BranchContext;
use crate::branch_operations::checkout;
use crate::branch_operations::integrate;
use crate::branch_operations::manage;
use crate::branch_operations::ongoing::{self, OngoingOperationAction};

pub const NEW_BRANCH_IN_EMPTY_REPOSITORY_TOOLTIP: &str =
    "Cannot create new branch in empty repository. Make initial commit first";
pub const CHECKOUT_IN_EMPTY_REPOSITORY_TOOLTIP: &str =
    "Cannot checkout in empty repository. Make initial commit first";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopActionKind {
    UpdateProject,
    Commit,
    Push,
    Ongoing(OngoingOperationAction),
    NewBranch,
    CheckoutTagOrRevision,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopAction {
    pub kind: TopActionKind,
    pub label: SharedString,
    pub icon: Option<IconName>,
    pub enabled: bool,
    pub disabled_reason: Option<&'static str>,
}

impl TopAction {
    pub fn shortcut_action(&self) -> Option<Box<dyn Action>> {
        match self.kind {
            TopActionKind::UpdateProject => Some(git::UpdateProject.boxed_clone()),
            TopActionKind::Commit => Some(git::Commit.boxed_clone()),
            TopActionKind::Push => Some(git::PushDialog.boxed_clone()),
            TopActionKind::NewBranch => Some(git::NewBranch.boxed_clone()),
            TopActionKind::CheckoutTagOrRevision => Some(git::CheckoutTagOrRevision.boxed_clone()),
            TopActionKind::Ongoing(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TopActionItem {
    Action(TopAction),
    Separator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TopActionsInput {
    pub has_commits: bool,
    pub operation: Option<RepositoryOperation>,
    pub update_project_shows_options: bool,
}

fn action(kind: TopActionKind, label: &str, icon: Option<IconName>) -> TopAction {
    TopAction {
        kind,
        label: SharedString::from(label.to_string()),
        icon,
        enabled: true,
        disabled_reason: None,
    }
}

fn ongoing_icon(action: OngoingOperationAction) -> IconName {
    match action {
        OngoingOperationAction::AbortRebase | OngoingOperationAction::AbortMerge => IconName::Stop,
        OngoingOperationAction::ContinueRebase => IconName::PlayFilled,
        OngoingOperationAction::SkipCommit => IconName::ArrowRight,
    }
}

pub fn top_actions_plan(input: &TopActionsInput) -> Vec<TopActionItem> {
    let mut items = vec![
        TopActionItem::Action(action(
            TopActionKind::UpdateProject,
            integrate::update_project_label(input.update_project_shows_options),
            Some(IconName::ArrowDown),
        )),
        TopActionItem::Action(action(
            TopActionKind::Commit,
            "Commit…",
            Some(IconName::GitCommit),
        )),
        TopActionItem::Action(action(
            TopActionKind::Push,
            "Push…",
            Some(IconName::ArrowUpRight),
        )),
        TopActionItem::Separator,
    ];

    for ongoing_action in ongoing::ongoing_actions(input.operation) {
        items.push(TopActionItem::Action(action(
            TopActionKind::Ongoing(ongoing_action),
            ongoing_action.label(),
            Some(ongoing_icon(ongoing_action)),
        )));
    }

    let mut new_branch = action(
        TopActionKind::NewBranch,
        "New Branch…",
        Some(IconName::Plus),
    );
    let mut checkout_revision = action(
        TopActionKind::CheckoutTagOrRevision,
        "Checkout Tag or Revision…",
        None,
    );
    if !input.has_commits {
        new_branch.enabled = false;
        new_branch.disabled_reason = Some(NEW_BRANCH_IN_EMPTY_REPOSITORY_TOOLTIP);
        checkout_revision.enabled = false;
        checkout_revision.disabled_reason = Some(CHECKOUT_IN_EMPTY_REPOSITORY_TOOLTIP);
    }
    items.push(TopActionItem::Action(new_branch));
    items.push(TopActionItem::Action(checkout_revision));
    items
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchedTopAction {
    pub action: TopAction,
    pub highlight_positions: Vec<usize>,
    pub score: i64,
}

pub fn matching_actions(plan: &[TopActionItem], query: &str) -> Vec<MatchedTopAction> {
    plan.iter()
        .filter_map(|item| match item {
            TopActionItem::Action(action) => {
                tree::match_query(query, &action.label).map(|(score, positions)| MatchedTopAction {
                    action: action.clone(),
                    highlight_positions: positions,
                    score,
                })
            }
            TopActionItem::Separator => None,
        })
        .collect()
}

pub fn run_top_action(
    kind: TopActionKind,
    shift_held: bool,
    context: BranchContext,
    window: &mut Window,
    cx: &mut App,
) {
    match kind {
        TopActionKind::UpdateProject if shift_held => {
            integrate::update_project_options(context, window, cx)
        }
        TopActionKind::UpdateProject => integrate::update_project(context, window, cx),
        TopActionKind::Commit => {
            window.dispatch_action(git::Commit.boxed_clone(), cx);
        }
        TopActionKind::Push => manage::push_current(context, window, cx),
        TopActionKind::Ongoing(ongoing_action) => {
            ongoing::run_ongoing_action(context, ongoing_action, window, cx)
        }
        TopActionKind::NewBranch => checkout::new_branch(context, window, cx),
        TopActionKind::CheckoutTagOrRevision => {
            checkout::checkout_tag_or_revision(context, window, cx)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(items: &[TopActionItem]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                TopActionItem::Action(action) => action.label.to_string(),
                TopActionItem::Separator => "---".to_string(),
            })
            .collect()
    }

    fn plan(has_commits: bool, operation: Option<RepositoryOperation>) -> Vec<TopActionItem> {
        top_actions_plan(&TopActionsInput {
            has_commits,
            operation,
            update_project_shows_options: true,
        })
    }

    fn find(items: &[TopActionItem], kind: TopActionKind) -> TopAction {
        items
            .iter()
            .find_map(|item| match item {
                TopActionItem::Action(action) if action.kind == kind => Some(action.clone()),
                _ => None,
            })
            .expect("action is part of the plan")
    }

    #[test]
    fn plan_without_ongoing_operation_lists_the_intellij_actions_in_order() {
        assert_eq!(
            labels(&plan(true, None)),
            vec![
                "Update Project…",
                "Commit…",
                "Push…",
                "---",
                "New Branch…",
                "Checkout Tag or Revision…",
            ]
        );
    }

    #[test]
    fn rebase_in_progress_adds_abort_continue_and_skip_before_new_branch() {
        assert_eq!(
            labels(&plan(true, Some(RepositoryOperation::Rebase))),
            vec![
                "Update Project…",
                "Commit…",
                "Push…",
                "---",
                "Abort Rebase",
                "Continue Rebase",
                "Skip Commit",
                "New Branch…",
                "Checkout Tag or Revision…",
            ]
        );
    }

    #[test]
    fn merge_in_progress_adds_abort_merge_only() {
        let items = plan(true, Some(RepositoryOperation::Merge));
        let items = labels(&items);
        assert!(items.contains(&"Abort Merge".to_string()));
        assert!(!items.contains(&"Abort Rebase".to_string()));
    }

    #[test]
    fn cherry_pick_in_progress_adds_no_ongoing_rows() {
        assert_eq!(
            labels(&plan(true, Some(RepositoryOperation::CherryPick))),
            labels(&plan(true, None))
        );
    }

    #[test]
    fn empty_repository_disables_new_branch_and_checkout_with_the_reason() {
        let items = plan(false, None);
        let new_branch = find(&items, TopActionKind::NewBranch);
        assert!(!new_branch.enabled);
        assert_eq!(
            new_branch.disabled_reason,
            Some("Cannot create new branch in empty repository. Make initial commit first")
        );
        let checkout = find(&items, TopActionKind::CheckoutTagOrRevision);
        assert!(!checkout.enabled);
        assert!(checkout.disabled_reason.is_some());
        assert!(find(&items, TopActionKind::Commit).enabled);
    }

    #[test]
    fn repository_with_commits_enables_every_action() {
        for item in plan(true, None) {
            if let TopActionItem::Action(action) = item {
                assert!(action.enabled, "{} should be enabled", action.label);
                assert_eq!(action.disabled_reason, None);
            }
        }
    }

    #[test]
    fn only_git_actions_carry_a_shortcut_action() {
        let items = plan(true, Some(RepositoryOperation::Rebase));
        assert!(
            find(&items, TopActionKind::UpdateProject)
                .shortcut_action()
                .is_some()
        );
        assert!(
            find(&items, TopActionKind::Commit)
                .shortcut_action()
                .is_some()
        );
        assert!(
            find(&items, TopActionKind::Push)
                .shortcut_action()
                .is_some()
        );
        assert!(
            find(&items, TopActionKind::NewBranch)
                .shortcut_action()
                .is_some()
        );
        assert!(
            find(&items, TopActionKind::CheckoutTagOrRevision)
                .shortcut_action()
                .is_some()
        );
        assert!(
            find(
                &items,
                TopActionKind::Ongoing(OngoingOperationAction::AbortRebase)
            )
            .shortcut_action()
            .is_none()
        );
    }

    #[test]
    fn matching_actions_filters_by_fuzzy_label_and_reports_highlights() {
        let items = plan(true, None);
        let matched = matching_actions(&items, "new");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].action.kind, TopActionKind::NewBranch);
        assert_eq!(matched[0].highlight_positions, vec![0, 1, 2]);
    }

    #[test]
    fn matching_actions_with_unrelated_query_is_empty() {
        assert!(matching_actions(&plan(true, None), "zzzz").is_empty());
    }

    #[test]
    fn matching_actions_skips_separators_and_keeps_plan_order() {
        let matched = matching_actions(&plan(true, None), "c");
        let labels: Vec<String> = matched
            .iter()
            .map(|matched| matched.action.label.to_string())
            .collect();
        assert_eq!(
            labels,
            vec![
                "Update Project…",
                "Commit…",
                "New Branch…",
                "Checkout Tag or Revision…"
            ]
        );
    }

    #[test]
    fn matching_actions_is_case_insensitive() {
        let matched = matching_actions(&plan(true, None), "PUSH");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].action.kind, TopActionKind::Push);
    }

    #[test]
    fn update_project_drops_the_ellipsis_when_the_options_dialog_is_suppressed() {
        let items = top_actions_plan(&TopActionsInput {
            has_commits: true,
            operation: None,
            update_project_shows_options: false,
        });
        assert_eq!(
            find(&items, TopActionKind::UpdateProject).label,
            "Update Project"
        );
        assert_eq!(
            find(&plan(true, None), TopActionKind::UpdateProject).label,
            "Update Project…"
        );
    }

    #[test]
    fn push_row_shows_the_shortcut_of_the_push_dialog_action() {
        let items = plan(true, None);
        let shortcut = find(&items, TopActionKind::Push)
            .shortcut_action()
            .expect("push has a shortcut action");
        assert!(shortcut.partial_eq(&git::PushDialog));
        assert!(!shortcut.partial_eq(&git::Push));
    }

    #[test]
    fn matching_actions_report_a_higher_score_for_an_earlier_match() {
        let prefix = matching_actions(&plan(true, None), "new");
        let offset = matching_actions(&plan(true, None), "ew");
        assert_eq!(prefix.len(), 1);
        assert_eq!(offset.len(), 1);
        assert!(prefix[0].score > offset[0].score);
    }
}
