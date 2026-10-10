use std::rc::Rc;

use gpui::{Action, App, SharedString, Window};
use ui::{ContextMenu, ContextMenuEntry, DocumentationSide, prelude::*};

use crate::branch_operations::{
    BranchContext, checkout, compare, integrate, manage, quoted_ref_label,
};
use crate::branch_refs::{RefKind, RefTarget};

const PUSH_TAG_SUBMENU_THRESHOLD: usize = 6;
const PUSH_TAG_SUBMENU_LABEL: &str = "Push Tag";

pub const DETACHED_HEAD_REBASE_TOOLTIP: &str = "Rebase is not possible in the detached HEAD state";
pub const PROTECTED_BRANCH_DELETE_TOOLTIP: &str = "Cannot delete a protected branch";
pub const EMPTY_REPOSITORY_TOOLTIP: &str =
    "This action is not possible in an empty repository. Make initial commit first";
pub const FETCH_IN_PROGRESS_TOOLTIP: &str = "Update is already running";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuAction {
    Checkout,
    NewBranchFrom,
    CheckoutAndRebase,
    CheckoutAndUpdate,
    CompareWithCurrent,
    ShowDiffWithWorkingTree,
    RebaseCurrentOnto,
    MergeIntoCurrent,
    Update,
    Push,
    PullUsingRebase,
    PullUsingMerge,
    PushTag { remote: SharedString },
    Rename,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub action: MenuAction,
    pub label: String,
    pub enabled: bool,
    pub tooltip: Option<String>,
}

impl MenuItem {
    fn new(action: MenuAction, label: impl Into<String>) -> Self {
        Self {
            action,
            label: label.into(),
            enabled: true,
            tooltip: None,
        }
    }

    fn disabled(mut self, reason: impl Into<String>) -> Self {
        self.enabled = false;
        self.tooltip = Some(reason.into());
        self
    }

    fn enabled_unless(self, disabled: bool, reason: impl Into<String>) -> Self {
        if disabled {
            self.disabled(reason)
        } else {
            self
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuItemPlan {
    Separator,
    Item(MenuItem),
    Submenu { label: String, items: Vec<MenuItem> },
}

#[derive(Clone, Copy, Debug)]
pub struct MenuContext<'a> {
    pub target: &'a RefTarget,
    pub current_branch: Option<&'a str>,
    pub head_short_sha: Option<&'a str>,
    pub head_sha: Option<&'a str>,
    pub tag_commit_sha: Option<&'a str>,
    pub has_commits: bool,
    pub remotes: &'a [SharedString],
    pub has_tracked_branch: bool,
    pub fetching: bool,
}

impl MenuContext<'_> {
    fn is_detached(&self) -> bool {
        self.current_branch.is_none()
    }

    fn is_current(&self) -> bool {
        self.target.kind == RefKind::Local && self.current_branch == Some(&*self.target.name)
    }

    fn is_detached_head_tag(&self) -> bool {
        self.target.kind == RefKind::Tag
            && self.is_detached()
            && self.head_sha.is_some()
            && self.head_sha == self.tag_commit_sha
    }

    fn current_label(&self) -> String {
        let name = self
            .current_branch
            .or(self.head_short_sha)
            .unwrap_or("HEAD");
        quoted_ref_label(name)
    }

    fn has_remotes(&self) -> bool {
        !self.remotes.is_empty()
    }
}

pub fn menu_plan(context: &MenuContext<'_>) -> Vec<MenuItemPlan> {
    let groups = match context.target.kind {
        RefKind::Local => local_groups(context),
        RefKind::Remote => remote_groups(context),
        RefKind::Tag => tag_groups(context),
    };

    let mut plan: Vec<MenuItemPlan> = Vec::new();
    for group in groups.into_iter().filter(|group| !group.is_empty()) {
        if !plan.is_empty() {
            plan.push(MenuItemPlan::Separator);
        }
        plan.extend(group);
    }

    if !context.has_commits {
        for item in &mut plan {
            match item {
                MenuItemPlan::Separator => {}
                MenuItemPlan::Item(menu_item) => disable_for_empty_repository(menu_item),
                MenuItemPlan::Submenu { items, .. } => {
                    items.iter_mut().for_each(disable_for_empty_repository)
                }
            }
        }
    }
    plan
}

fn disable_for_empty_repository(item: &mut MenuItem) {
    item.enabled = false;
    item.tooltip = Some(EMPTY_REPOSITORY_TOOLTIP.to_string());
}

fn item(action: MenuAction, label: impl Into<String>) -> MenuItemPlan {
    MenuItemPlan::Item(MenuItem::new(action, label))
}

fn local_groups(context: &MenuContext<'_>) -> Vec<Vec<MenuItemPlan>> {
    let quoted = quoted_ref_label(&context.target.name);
    let current = context.current_label();
    let is_current = context.is_current();
    let update_disabled_reason = if !context.has_tracked_branch {
        Some(format!("Tracked branch is not configured for {quoted}"))
    } else if context.fetching {
        Some(FETCH_IN_PROGRESS_TOOLTIP.to_string())
    } else {
        None
    };
    let with_update_state = |menu_item: MenuItem| match &update_disabled_reason {
        Some(reason) => menu_item.disabled(reason.clone()),
        None => menu_item,
    };

    let mut checkout_group = Vec::new();
    if !is_current {
        checkout_group.push(item(MenuAction::Checkout, "Checkout"));
    }
    checkout_group.push(item(
        MenuAction::NewBranchFrom,
        format!("New Branch from {quoted}…"),
    ));
    if !is_current {
        checkout_group.push(item(
            MenuAction::CheckoutAndRebase,
            format!("Checkout and Rebase onto {current}"),
        ));
        if context.has_remotes() {
            checkout_group.push(MenuItemPlan::Item(with_update_state(MenuItem::new(
                MenuAction::CheckoutAndUpdate,
                "Checkout and Update",
            ))));
        }
    }

    let mut compare_group = Vec::new();
    if !is_current {
        compare_group.push(item(
            MenuAction::CompareWithCurrent,
            format!("Compare with {current}"),
        ));
    }
    compare_group.push(item(
        MenuAction::ShowDiffWithWorkingTree,
        "Show Diff with Working Tree",
    ));

    let mut integrate_group = Vec::new();
    if !is_current {
        integrate_group.push(MenuItemPlan::Item(
            MenuItem::new(
                MenuAction::RebaseCurrentOnto,
                format!("Rebase {current} onto {quoted}"),
            )
            .enabled_unless(context.is_detached(), DETACHED_HEAD_REBASE_TOOLTIP),
        ));
        integrate_group.push(item(
            MenuAction::MergeIntoCurrent,
            format!("Merge {quoted} into {current}"),
        ));
    }

    let mut update_group = Vec::new();
    if context.has_remotes() {
        update_group.push(MenuItemPlan::Item(with_update_state(MenuItem::new(
            MenuAction::Update,
            "Update",
        ))));
    }
    update_group.push(item(MenuAction::Push, "Push…"));

    let mut manage_group = vec![item(MenuAction::Rename, "Rename…")];
    if !is_current {
        manage_group.push(item(MenuAction::Delete, "Delete"));
    }

    vec![
        checkout_group,
        compare_group,
        integrate_group,
        update_group,
        manage_group,
    ]
}

fn remote_groups(context: &MenuContext<'_>) -> Vec<Vec<MenuItemPlan>> {
    let quoted = quoted_ref_label(&context.target.name);
    let current = context.current_label();
    let detached = context.is_detached();

    let checkout_group = vec![
        item(MenuAction::Checkout, "Checkout"),
        item(
            MenuAction::NewBranchFrom,
            format!("New Branch from {quoted}…"),
        ),
        item(
            MenuAction::CheckoutAndRebase,
            format!("Checkout and Rebase onto {current}"),
        ),
    ];
    let compare_group = vec![
        item(
            MenuAction::CompareWithCurrent,
            format!("Compare with {current}"),
        ),
        item(
            MenuAction::ShowDiffWithWorkingTree,
            "Show Diff with Working Tree",
        ),
    ];
    let integrate_group = vec![
        MenuItemPlan::Item(
            MenuItem::new(
                MenuAction::RebaseCurrentOnto,
                format!("Rebase {current} onto {quoted}"),
            )
            .enabled_unless(detached, DETACHED_HEAD_REBASE_TOOLTIP),
        ),
        item(
            MenuAction::MergeIntoCurrent,
            format!("Merge {quoted} into {current}"),
        ),
    ];
    let pull_group = vec![
        item(
            MenuAction::PullUsingRebase,
            format!("Pull into {current} Using Rebase"),
        ),
        item(
            MenuAction::PullUsingMerge,
            format!("Pull into {current} Using Merge"),
        ),
    ];
    let manage_group = vec![MenuItemPlan::Item(
        MenuItem::new(MenuAction::Delete, "Delete").enabled_unless(
            integrate::is_protected_remote_reference(&context.target.name),
            PROTECTED_BRANCH_DELETE_TOOLTIP,
        ),
    )];

    vec![
        checkout_group,
        compare_group,
        integrate_group,
        pull_group,
        manage_group,
    ]
}

fn tag_groups(context: &MenuContext<'_>) -> Vec<Vec<MenuItemPlan>> {
    let quoted = quoted_ref_label(&context.target.name);
    let current = context.current_label();

    let (checkout_group, integrate_group) = if context.is_detached_head_tag() {
        (Vec::new(), Vec::new())
    } else {
        (
            vec![item(MenuAction::Checkout, "Checkout")],
            vec![item(
                MenuAction::MergeIntoCurrent,
                format!("Merge {quoted} into {current}"),
            )],
        )
    };
    let compare_group = vec![item(
        MenuAction::ShowDiffWithWorkingTree,
        "Show Diff with Working Tree",
    )];

    let push_items: Vec<MenuItem> = context
        .remotes
        .iter()
        .map(|remote| {
            MenuItem::new(
                MenuAction::PushTag {
                    remote: remote.clone(),
                },
                format!("Push to {remote}"),
            )
        })
        .collect();
    let push_group = if push_items.len() >= PUSH_TAG_SUBMENU_THRESHOLD {
        vec![MenuItemPlan::Submenu {
            label: PUSH_TAG_SUBMENU_LABEL.to_string(),
            items: push_items,
        }]
    } else {
        push_items.into_iter().map(MenuItemPlan::Item).collect()
    };
    let manage_group = vec![item(MenuAction::Delete, "Delete")];

    vec![
        checkout_group,
        compare_group,
        integrate_group,
        push_group,
        manage_group,
    ]
}

pub fn run_menu_action(
    action: MenuAction,
    context: BranchContext,
    target: RefTarget,
    window: &mut Window,
    cx: &mut App,
) {
    match action {
        MenuAction::Checkout => checkout::checkout(context, target, window, cx),
        MenuAction::NewBranchFrom => checkout::new_branch_from(context, target, window, cx),
        MenuAction::CheckoutAndRebase => {
            checkout::checkout_and_rebase_onto_current(context, target, window, cx)
        }
        MenuAction::CheckoutAndUpdate => checkout::checkout_and_update(context, target, window, cx),
        MenuAction::CompareWithCurrent => {
            compare::compare_with_current(context, target, window, cx)
        }
        MenuAction::ShowDiffWithWorkingTree => {
            compare::show_diff_with_working_tree(context, target, window, cx)
        }
        MenuAction::RebaseCurrentOnto => {
            integrate::rebase_current_onto(context, target, window, cx)
        }
        MenuAction::MergeIntoCurrent => integrate::merge_into_current(context, target, window, cx),
        MenuAction::Update => integrate::update_branch(context, target, window, cx),
        MenuAction::Push => manage::push_branch(context, target, window, cx),
        MenuAction::PullUsingRebase => {
            integrate::pull_into_current(context, target, true, window, cx)
        }
        MenuAction::PullUsingMerge => {
            integrate::pull_into_current(context, target, false, window, cx)
        }
        MenuAction::PushTag { remote } => manage::push_tag(context, target, remote, window, cx),
        MenuAction::Rename => manage::rename_branch(context, target, window, cx),
        MenuAction::Delete => manage::delete_ref(context, target, window, cx),
    }
}

pub struct MenuRunner {
    pub context: BranchContext,
    pub target: RefTarget,
    pub dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl MenuRunner {
    fn run(&self, action: MenuAction, window: &mut Window, cx: &mut App) {
        (self.dismiss)(window, cx);
        let context = self.context.clone();
        let target = self.target.clone();
        window.defer(cx, move |window, cx| {
            run_menu_action(action, context, target, window, cx)
        });
    }
}

fn shortcut_action(action: &MenuAction) -> Option<Box<dyn Action>> {
    match action {
        MenuAction::Push => Some(Box::new(git::PushDialog)),
        _ => None,
    }
}

fn add_item(menu: ContextMenu, menu_item: MenuItem, runner: &Rc<MenuRunner>) -> ContextMenu {
    if menu_item.enabled {
        let runner = runner.clone();
        let action = menu_item.action;
        let shortcut = shortcut_action(&action);
        return menu.entry(menu_item.label, shortcut, move |window, cx| {
            runner.run(action.clone(), window, cx)
        });
    }

    let mut entry = ContextMenuEntry::new(menu_item.label).disabled(true);
    if let Some(tooltip) = menu_item.tooltip {
        entry = entry.documentation_aside(DocumentationSide::Right, move |_| {
            Label::new(tooltip.clone()).into_any_element()
        });
    }
    menu.item(entry)
}

pub fn build_ref_menu(
    plan: Vec<MenuItemPlan>,
    runner: MenuRunner,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Entity<ContextMenu> {
    let runner = Rc::new(runner);
    ContextMenu::build(window, cx, move |menu, _, _| {
        plan.into_iter().fold(menu, |menu, planned| match planned {
            MenuItemPlan::Separator => menu.separator(),
            MenuItemPlan::Item(menu_item) => add_item(menu, menu_item, &runner),
            MenuItemPlan::Submenu { label, items } => {
                let runner = runner.clone();
                menu.submenu(label, move |submenu, _, _| {
                    items.iter().cloned().fold(submenu, |submenu, menu_item| {
                        add_item(submenu, menu_item, &runner)
                    })
                })
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scenario {
        target: RefTarget,
        current_branch: Option<String>,
        head_short_sha: Option<String>,
        head_sha: Option<String>,
        tag_commit_sha: Option<String>,
        has_commits: bool,
        remotes: Vec<SharedString>,
        has_tracked_branch: bool,
        fetching: bool,
    }

    impl Scenario {
        fn new(target: RefTarget) -> Self {
            Self {
                target,
                current_branch: Some("main".to_string()),
                head_short_sha: Some("0123abcd".to_string()),
                head_sha: Some("0123abcd4567ef890123abcd4567ef890123abcd".to_string()),
                tag_commit_sha: None,
                has_commits: true,
                remotes: vec![SharedString::from("origin")],
                has_tracked_branch: true,
                fetching: false,
            }
        }

        fn plan(&self) -> Vec<MenuItemPlan> {
            menu_plan(&MenuContext {
                target: &self.target,
                current_branch: self.current_branch.as_deref(),
                head_short_sha: self.head_short_sha.as_deref(),
                head_sha: self.head_sha.as_deref(),
                tag_commit_sha: self.tag_commit_sha.as_deref(),
                has_commits: self.has_commits,
                remotes: &self.remotes,
                has_tracked_branch: self.has_tracked_branch,
                fetching: self.fetching,
            })
        }
    }

    fn describe(plan: &[MenuItemPlan]) -> Vec<String> {
        plan.iter()
            .map(|planned| match planned {
                MenuItemPlan::Separator => "---".to_string(),
                MenuItemPlan::Item(menu_item) if menu_item.enabled => menu_item.label.clone(),
                MenuItemPlan::Item(menu_item) => format!("[disabled] {}", menu_item.label),
                MenuItemPlan::Submenu { label, items } => {
                    format!("{label} > {} items", items.len())
                }
            })
            .collect()
    }

    fn find_item(plan: &[MenuItemPlan], action: &MenuAction) -> MenuItem {
        plan.iter()
            .find_map(|planned| match planned {
                MenuItemPlan::Item(menu_item) if &menu_item.action == action => {
                    Some(menu_item.clone())
                }
                _ => None,
            })
            .expect("menu item is part of the plan")
    }

    fn has_item(plan: &[MenuItemPlan], action: &MenuAction) -> bool {
        plan.iter().any(|planned| {
            matches!(planned, MenuItemPlan::Item(menu_item) if &menu_item.action == action)
        })
    }

    fn assert_well_formed(plan: &[MenuItemPlan]) {
        assert!(!matches!(plan.first(), Some(MenuItemPlan::Separator)));
        assert!(!matches!(plan.last(), Some(MenuItemPlan::Separator)));
        assert!(
            plan.windows(2)
                .all(|pair| !(matches!(pair[0], MenuItemPlan::Separator)
                    && matches!(pair[1], MenuItemPlan::Separator))),
            "adjacent separators in {plan:?}"
        );
    }

    #[test]
    fn other_local_branch_menu_matches_intellij_order() {
        let scenario = Scenario::new(RefTarget::local("feature/login"));
        let plan = scenario.plan();
        assert_eq!(
            describe(&plan),
            vec![
                "Checkout",
                "New Branch from 'feature/login'…",
                "Checkout and Rebase onto 'main'",
                "Checkout and Update",
                "---",
                "Compare with 'main'",
                "Show Diff with Working Tree",
                "---",
                "Rebase 'main' onto 'feature/login'",
                "Merge 'feature/login' into 'main'",
                "---",
                "Update",
                "Push…",
                "---",
                "Rename…",
                "Delete",
            ]
        );
        assert_well_formed(&plan);
    }

    #[test]
    fn current_local_branch_menu_hides_checkout_compare_merge_and_delete() {
        let scenario = Scenario::new(RefTarget::local("main"));
        let plan = scenario.plan();
        assert_eq!(
            describe(&plan),
            vec![
                "New Branch from 'main'…",
                "---",
                "Show Diff with Working Tree",
                "---",
                "Update",
                "Push…",
                "---",
                "Rename…",
            ]
        );
        assert_well_formed(&plan);
    }

    #[test]
    fn current_local_branch_without_tracking_greys_out_update_with_the_reason() {
        let mut scenario = Scenario::new(RefTarget::local("main"));
        scenario.has_tracked_branch = false;
        let update = find_item(&scenario.plan(), &MenuAction::Update);
        assert!(!update.enabled);
        assert_eq!(
            update.tooltip.as_deref(),
            Some("Tracked branch is not configured for 'main'")
        );
    }

    #[test]
    fn local_branch_without_tracking_disables_update_and_checkout_and_update() {
        let mut scenario = Scenario::new(RefTarget::local("topic"));
        scenario.has_tracked_branch = false;
        let plan = scenario.plan();
        for action in [MenuAction::Update, MenuAction::CheckoutAndUpdate] {
            let menu_item = find_item(&plan, &action);
            assert!(!menu_item.enabled);
            assert_eq!(
                menu_item.tooltip.as_deref(),
                Some("Tracked branch is not configured for 'topic'")
            );
        }
        assert!(find_item(&plan, &MenuAction::Checkout).enabled);
        assert!(find_item(&plan, &MenuAction::Push).enabled);
    }

    #[test]
    fn repository_without_remotes_hides_update_actions_for_local_branches() {
        let mut scenario = Scenario::new(RefTarget::local("topic"));
        scenario.remotes.clear();
        let plan = scenario.plan();
        assert!(!has_item(&plan, &MenuAction::Update));
        assert!(!has_item(&plan, &MenuAction::CheckoutAndUpdate));
        assert!(has_item(&plan, &MenuAction::Push));
        assert_well_formed(&plan);
    }

    #[test]
    fn running_fetch_disables_update_for_a_tracked_branch() {
        let mut scenario = Scenario::new(RefTarget::local("topic"));
        scenario.fetching = true;
        let update = find_item(&scenario.plan(), &MenuAction::Update);
        assert!(!update.enabled);
        assert_eq!(update.tooltip.as_deref(), Some(FETCH_IN_PROGRESS_TOOLTIP));
    }

    #[test]
    fn detached_head_disables_rebase_with_the_intellij_tooltip_and_uses_the_short_sha() {
        let mut scenario = Scenario::new(RefTarget::local("topic"));
        scenario.current_branch = None;
        let plan = scenario.plan();
        let rebase = find_item(&plan, &MenuAction::RebaseCurrentOnto);
        assert!(!rebase.enabled);
        assert_eq!(
            rebase.tooltip.as_deref(),
            Some("Rebase is not possible in the detached HEAD state")
        );
        assert_eq!(rebase.label, "Rebase '0123abcd' onto 'topic'");
        assert!(find_item(&plan, &MenuAction::MergeIntoCurrent).enabled);
        assert!(find_item(&plan, &MenuAction::Checkout).enabled);
    }

    #[test]
    fn detached_head_never_marks_a_local_branch_as_current() {
        let mut scenario = Scenario::new(RefTarget::local("main"));
        scenario.current_branch = None;
        let plan = scenario.plan();
        assert!(has_item(&plan, &MenuAction::Checkout));
        assert!(has_item(&plan, &MenuAction::Delete));
    }

    #[test]
    fn remote_branch_menu_matches_intellij_order() {
        let scenario = Scenario::new(RefTarget::remote("origin/feature/x"));
        let plan = scenario.plan();
        assert_eq!(
            describe(&plan),
            vec![
                "Checkout",
                "New Branch from 'origin/feature/x'…",
                "Checkout and Rebase onto 'main'",
                "---",
                "Compare with 'main'",
                "Show Diff with Working Tree",
                "---",
                "Rebase 'main' onto 'origin/feature/x'",
                "Merge 'origin/feature/x' into 'main'",
                "---",
                "Pull into 'main' Using Rebase",
                "Pull into 'main' Using Merge",
                "---",
                "Delete",
            ]
        );
        assert_well_formed(&plan);
    }

    #[test]
    fn remote_branch_menu_has_no_update_push_or_rename() {
        let plan = Scenario::new(RefTarget::remote("origin/main")).plan();
        for action in [MenuAction::Update, MenuAction::Push, MenuAction::Rename] {
            assert!(!has_item(&plan, &action));
        }
    }

    #[test]
    fn remote_branch_pull_stays_enabled_when_head_is_detached() {
        let mut scenario = Scenario::new(RefTarget::remote("origin/main"));
        scenario.current_branch = None;
        let plan = scenario.plan();
        for action in [MenuAction::PullUsingRebase, MenuAction::PullUsingMerge] {
            let menu_item = find_item(&plan, &action);
            assert!(menu_item.enabled);
            assert_eq!(menu_item.tooltip, None);
        }
        assert_eq!(
            find_item(&plan, &MenuAction::PullUsingRebase).label,
            "Pull into '0123abcd' Using Rebase"
        );
        assert!(find_item(&plan, &MenuAction::Checkout).enabled);
    }

    #[test]
    fn deleting_a_protected_remote_branch_is_disabled_with_the_reason() {
        for name in ["origin/main", "origin/master", "upstream/main"] {
            let plan = Scenario::new(RefTarget::remote(name)).plan();
            let delete = find_item(&plan, &MenuAction::Delete);
            assert!(!delete.enabled, "{name} must not be deletable");
            assert_eq!(
                delete.tooltip.as_deref(),
                Some("Cannot delete a protected branch")
            );
        }
        let plan = Scenario::new(RefTarget::remote("origin/feature/main")).plan();
        let delete = find_item(&plan, &MenuAction::Delete);
        assert!(delete.enabled);
        assert_eq!(delete.tooltip, None);
    }

    #[test]
    fn the_tag_under_a_detached_head_hides_checkout_and_merge() {
        let mut scenario = Scenario::new(RefTarget::tag("v1.0"));
        scenario.current_branch = None;
        scenario.tag_commit_sha = scenario.head_sha.clone();
        let plan = scenario.plan();
        assert_eq!(
            describe(&plan),
            vec![
                "Show Diff with Working Tree",
                "---",
                "Push to origin",
                "---",
                "Delete"
            ]
        );
        assert_well_formed(&plan);
    }

    #[test]
    fn a_tag_keeps_checkout_and_merge_unless_head_is_detached_exactly_on_it() {
        let mut scenario = Scenario::new(RefTarget::tag("v1.0"));
        scenario.tag_commit_sha = scenario.head_sha.clone();
        let attached = scenario.plan();
        assert!(has_item(&attached, &MenuAction::Checkout));
        assert!(has_item(&attached, &MenuAction::MergeIntoCurrent));

        scenario.current_branch = None;
        scenario.tag_commit_sha = Some("ffffffffffffffffffffffffffffffffffffffff".to_string());
        let elsewhere = scenario.plan();
        assert!(has_item(&elsewhere, &MenuAction::Checkout));
        assert!(has_item(&elsewhere, &MenuAction::MergeIntoCurrent));

        scenario.head_sha = None;
        scenario.tag_commit_sha = None;
        let unknown = scenario.plan();
        assert!(has_item(&unknown, &MenuAction::Checkout));
    }

    #[test]
    fn only_push_carries_a_shortcut_action() {
        assert!(shortcut_action(&MenuAction::Push).is_some());
        for action in [
            MenuAction::Checkout,
            MenuAction::Update,
            MenuAction::Rename,
            MenuAction::Delete,
            MenuAction::PullUsingRebase,
        ] {
            assert!(shortcut_action(&action).is_none(), "{action:?}");
        }
    }

    #[test]
    fn tag_menu_lists_push_to_every_remote() {
        let mut scenario = Scenario::new(RefTarget::tag("v1.0"));
        scenario.remotes = vec![SharedString::from("origin"), SharedString::from("upstream")];
        let plan = scenario.plan();
        assert_eq!(
            describe(&plan),
            vec![
                "Checkout",
                "---",
                "Show Diff with Working Tree",
                "---",
                "Merge 'v1.0' into 'main'",
                "---",
                "Push to origin",
                "Push to upstream",
                "---",
                "Delete",
            ]
        );
        assert_well_formed(&plan);
    }

    #[test]
    fn tag_menu_hides_branch_only_actions() {
        let plan = Scenario::new(RefTarget::tag("v1.0")).plan();
        for action in [
            MenuAction::NewBranchFrom,
            MenuAction::CheckoutAndRebase,
            MenuAction::CompareWithCurrent,
            MenuAction::RebaseCurrentOnto,
            MenuAction::Update,
            MenuAction::Push,
            MenuAction::PullUsingRebase,
            MenuAction::PullUsingMerge,
            MenuAction::Rename,
        ] {
            assert!(!has_item(&plan, &action), "{action:?} must be hidden");
        }
    }

    #[test]
    fn tag_menu_collapses_push_to_into_a_submenu_with_six_or_more_remotes() {
        let mut scenario = Scenario::new(RefTarget::tag("v1.0"));
        scenario.remotes = (0..6)
            .map(|index| SharedString::from(format!("remote{index}")))
            .collect();
        let plan = scenario.plan();
        assert!(describe(&plan).contains(&"Push Tag > 6 items".to_string()));
        assert!(
            !describe(&plan)
                .iter()
                .any(|line| line.starts_with("Push to"))
        );

        scenario.remotes.pop();
        let plan = scenario.plan();
        assert_eq!(
            describe(&plan)
                .iter()
                .filter(|line| line.starts_with("Push to"))
                .count(),
            5
        );
    }

    #[test]
    fn tag_menu_without_remotes_has_no_push_group_and_no_dangling_separator() {
        let mut scenario = Scenario::new(RefTarget::tag("v1.0"));
        scenario.remotes.clear();
        let plan = scenario.plan();
        assert_well_formed(&plan);
        assert!(!describe(&plan).iter().any(|line| line.starts_with("Push")));
    }

    #[test]
    fn push_tag_items_carry_their_remote() {
        let mut scenario = Scenario::new(RefTarget::tag("v1.0"));
        scenario.remotes = vec![SharedString::from("upstream")];
        let plan = scenario.plan();
        assert!(has_item(
            &plan,
            &MenuAction::PushTag {
                remote: SharedString::from("upstream")
            }
        ));
    }

    #[test]
    fn empty_repository_disables_every_item_including_submenu_children() {
        for target in [
            RefTarget::local("main"),
            RefTarget::remote("origin/main"),
            RefTarget::tag("v1"),
        ] {
            let mut scenario = Scenario::new(target);
            scenario.has_commits = false;
            scenario.remotes = (0..7)
                .map(|index| SharedString::from(format!("remote{index}")))
                .collect();
            for planned in scenario.plan() {
                match planned {
                    MenuItemPlan::Separator => {}
                    MenuItemPlan::Item(menu_item) => {
                        assert!(!menu_item.enabled, "{} should be disabled", menu_item.label)
                    }
                    MenuItemPlan::Submenu { items, .. } => {
                        assert!(items.iter().all(|menu_item| !menu_item.enabled))
                    }
                }
            }
        }
    }

    #[test]
    fn long_branch_names_are_truncated_in_labels_but_not_in_targets() {
        let long_name = "feature/".to_string() + &"x".repeat(60);
        let scenario = Scenario::new(RefTarget::local(long_name.clone()));
        let plan = scenario.plan();
        let merge = find_item(&plan, &MenuAction::MergeIntoCurrent);
        assert!(merge.label.contains('…'));
        assert!(!merge.label.contains(&long_name));
        assert_eq!(&*scenario.target.name, long_name.as_str());
    }

    #[test]
    fn missing_branch_and_sha_fall_back_to_head_label() {
        let mut scenario = Scenario::new(RefTarget::local("topic"));
        scenario.current_branch = None;
        scenario.head_short_sha = None;
        let plan = scenario.plan();
        assert_eq!(
            find_item(&plan, &MenuAction::CheckoutAndRebase).label,
            "Checkout and Rebase onto 'HEAD'"
        );
    }
}
