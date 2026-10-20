use std::path::{Path, PathBuf};
use std::rc::Rc;

use git::repository::{Branch, UpstreamTracking, Worktree};
use gpui::{Action, App, SharedString, Window};
use ui::{ContextMenu, ContextMenuEntry, DocumentationSide, prelude::*};
use zed_actions::{CreateWorktree, NewWorktreeBranchTarget};

use crate::branch_operations::{
    BranchContext, checkout, compare, integrate, manage, quoted_ref_label,
};
use crate::branch_refs::{RefKind, RefTarget};
use crate::branches_popup::DeleteRef;

const PUSH_TAG_SUBMENU_THRESHOLD: usize = 6;
const PUSH_TAG_SUBMENU_LABEL: &str = "Push Tag";

pub const DETACHED_HEAD_REBASE_TOOLTIP: &str = "Rebase is not possible in the detached HEAD state";
pub const PROTECTED_BRANCH_DELETE_TOOLTIP: &str = "Cannot delete a protected branch";
pub const EMPTY_REPOSITORY_TOOLTIP: &str = "Action is not possible before the first commit.";
/// The key context of the popup's ref menu, which binds the `$Delete` shortcut to `DeleteRef`.
pub const REF_MENU_KEY_CONTEXT: &str = "menu BranchRefMenu";
pub const FETCH_IN_PROGRESS_TOOLTIP: &str = "Update is already running";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuAction {
    Checkout,
    OpenWorktree { path: PathBuf },
    NewBranchFrom,
    CheckoutAndRebase,
    CheckoutAndUpdate,
    CompareWithCurrent,
    ShowDiffWithWorkingTree,
    RebaseCurrentOnto,
    MergeIntoCurrent,
    NewWorktreeFrom,
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
    /// The ref the item acts on when it is not the ref the menu was opened for, like the items of
    /// the "Tracked Branch" submenu.
    pub target: Option<RefTarget>,
}

impl MenuItem {
    fn new(action: MenuAction, label: impl Into<String>) -> Self {
        Self {
            action,
            label: label.into(),
            enabled: true,
            tooltip: None,
            target: None,
        }
    }

    pub fn target_or<'a>(&'a self, menu_target: &'a RefTarget) -> &'a RefTarget {
        self.target.as_ref().unwrap_or(menu_target)
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
    Submenu {
        label: String,
        items: Vec<MenuItemPlan>,
    },
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
    /// The remote branch a local branch tracks, which gets its own "Tracked Branch" submenu.
    pub tracked_branch: Option<&'a str>,
    /// Another working tree that has the local branch checked out.
    pub other_worktree: Option<&'a Path>,
    pub fetching: bool,
    pub protected_branches: &'a [String],
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

/// The remote branch a local branch tracks, while it exists (`GitRepoInfo.getTrackingInfo`).
pub fn tracked_remote_branch(branch: &Branch) -> Option<String> {
    let upstream = branch.upstream.as_ref()?;
    if matches!(upstream.tracking, UpstreamTracking::Gone) {
        return None;
    }
    upstream.stripped_ref_name().map(str::to_string)
}

/// Another working tree that has `branch_name` checked out; `other_worktrees` leaves out the
/// working tree the popup belongs to (`GitWorkingTreesUtil.findCheckedOutWorkingTree`).
pub fn worktree_with_branch<'a>(
    branch_name: &str,
    other_worktrees: &'a [Worktree],
) -> Option<&'a Path> {
    let ref_name = format!("refs/heads/{branch_name}");
    other_worktrees
        .iter()
        .find(|worktree| worktree.ref_name.as_deref() == Some(ref_name.as_str()))
        .map(|worktree| worktree.path.as_path())
}

pub fn menu_plan(context: &MenuContext<'_>) -> Vec<MenuItemPlan> {
    let groups = match context.target.kind {
        RefKind::Local => local_groups(context),
        RefKind::Remote => remote_groups(context),
        RefKind::Tag => tag_groups(context),
    };
    let mut plan = join_groups(groups);
    if !context.has_commits {
        disable_for_empty_repository(&mut plan);
    }
    plan
}

fn join_groups(groups: Vec<Vec<MenuItemPlan>>) -> Vec<MenuItemPlan> {
    let mut plan: Vec<MenuItemPlan> = Vec::new();
    for group in groups.into_iter().filter(|group| !group.is_empty()) {
        if !plan.is_empty() {
            plan.push(MenuItemPlan::Separator);
        }
        plan.extend(group);
    }
    plan
}

fn disable_for_empty_repository(plan: &mut [MenuItemPlan]) {
    for item in plan {
        match item {
            MenuItemPlan::Separator => {}
            MenuItemPlan::Item(menu_item) => {
                menu_item.enabled = false;
                menu_item.tooltip = Some(EMPTY_REPOSITORY_TOOLTIP.to_string());
            }
            MenuItemPlan::Submenu { items, .. } => disable_for_empty_repository(items),
        }
    }
}

/// IntelliJ's "New Worktree from 'x'…" names the ref without shortening it.
fn new_worktree_from_label(name: &str) -> String {
    format!("New Worktree from '{name}'…")
}

/// `GitTrackedBranchActionGroup`: the remote branch actions, acting on the tracked branch.
fn tracked_branch_submenu(context: &MenuContext<'_>, tracked_branch: &str) -> MenuItemPlan {
    let target = RefTarget::remote(tracked_branch.to_string());
    let tracked_context = MenuContext {
        target: &target,
        other_worktree: None,
        tracked_branch: None,
        ..*context
    };
    let mut items = join_groups(remote_groups(&tracked_context));
    retarget(&mut items, &target);
    MenuItemPlan::Submenu {
        label: format!("Tracked Branch '{tracked_branch}'"),
        items,
    }
}

fn retarget(plan: &mut [MenuItemPlan], target: &RefTarget) {
    for item in plan {
        match item {
            MenuItemPlan::Separator => {}
            MenuItemPlan::Item(menu_item) => menu_item.target = Some(target.clone()),
            MenuItemPlan::Submenu { items, .. } => retarget(items, target),
        }
    }
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
    if let Some(path) = context.other_worktree {
        checkout_group.push(item(
            MenuAction::OpenWorktree {
                path: path.to_path_buf(),
            },
            "Open Worktree…",
        ));
    }
    checkout_group.push(item(
        MenuAction::NewBranchFrom,
        format!("New Branch from {quoted}…"),
    ));
    if !is_current {
        // IntelliJ cannot rebase a branch that another working tree has checked out.
        if context.other_worktree.is_none() {
            checkout_group.push(item(
                MenuAction::CheckoutAndRebase,
                format!("Checkout and Rebase onto {current}"),
            ));
        }
        // Unlike Update, IntelliJ keeps Checkout and Update enabled while a fetch runs.
        if context.has_remotes() {
            checkout_group.push(MenuItemPlan::Item(
                MenuItem::new(MenuAction::CheckoutAndUpdate, "Checkout and Update").enabled_unless(
                    !context.has_tracked_branch,
                    format!("Tracked branch is not configured for {quoted}"),
                ),
            ));
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

    let worktree_group = vec![item(
        MenuAction::NewWorktreeFrom,
        new_worktree_from_label(&context.target.name),
    )];

    let mut update_group = Vec::new();
    if context.has_remotes() {
        update_group.push(MenuItemPlan::Item(with_update_state(MenuItem::new(
            MenuAction::Update,
            "Update",
        ))));
    }
    update_group.push(item(MenuAction::Push, "Push…"));
    if let Some(tracked_branch) = context.tracked_branch {
        update_group.push(tracked_branch_submenu(context, tracked_branch));
    }

    let mut manage_group = vec![item(MenuAction::Rename, "Rename…")];
    if !is_current {
        manage_group.push(item(MenuAction::Delete, "Delete"));
    }

    vec![
        checkout_group,
        compare_group,
        integrate_group,
        worktree_group,
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
    let worktree_group = vec![item(
        MenuAction::NewWorktreeFrom,
        new_worktree_from_label(&context.target.name),
    )];
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
            integrate::is_protected_remote_reference(
                &context.target.name,
                context.protected_branches,
            ),
            PROTECTED_BRANCH_DELETE_TOOLTIP,
        ),
    )];

    vec![
        checkout_group,
        compare_group,
        integrate_group,
        worktree_group,
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
    let worktree_group = vec![item(
        MenuAction::NewWorktreeFrom,
        new_worktree_from_label(&context.target.name),
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
            items: push_items.into_iter().map(MenuItemPlan::Item).collect(),
        }]
    } else {
        push_items.into_iter().map(MenuItemPlan::Item).collect()
    };
    let manage_group = if context.is_detached_head_tag() {
        Vec::new()
    } else {
        vec![item(MenuAction::Delete, "Delete")]
    };

    vec![
        checkout_group,
        compare_group,
        integrate_group,
        worktree_group,
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
        MenuAction::OpenWorktree { path } => context.open_worktree(path, window, cx),
        MenuAction::NewWorktreeFrom => create_worktree_from(&context, &target, window, cx),
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
    /// Whether the menu's owner handles `DeleteRef`, so `Delete` can show and take its shortcut.
    pub handles_delete_shortcut: bool,
}

impl MenuRunner {
    fn run(&self, action: MenuAction, target: RefTarget, window: &mut Window, cx: &mut App) {
        (self.dismiss)(window, cx);
        let context = self.context.clone();
        window.defer(cx, move |window, cx| {
            run_menu_action(action, context, target, window, cx)
        });
    }
}

/// The shortcut an item shows. `Delete` uses IntelliJ's `$Delete` only on the ref the menu was
/// opened for, which is the ref `DeleteRef` deletes.
fn shortcut_action(menu_item: &MenuItem, handles_delete_shortcut: bool) -> Option<Box<dyn Action>> {
    match menu_item.action {
        MenuAction::Push => Some(Box::new(git::PushDialog)),
        MenuAction::Delete if handles_delete_shortcut && menu_item.target.is_none() => {
            Some(Box::new(DeleteRef))
        }
        _ => None,
    }
}

fn add_item(menu: ContextMenu, menu_item: MenuItem, runner: &Rc<MenuRunner>) -> ContextMenu {
    let shortcut = shortcut_action(&menu_item, runner.handles_delete_shortcut);
    if menu_item.enabled {
        let runner = runner.clone();
        let target = menu_item.target_or(&runner.target).clone();
        let action = menu_item.action;
        return menu.entry(menu_item.label, shortcut, move |window, cx| {
            runner.run(action.clone(), target.clone(), window, cx)
        });
    }

    let mut entry = ContextMenuEntry::new(menu_item.label).disabled(true);
    if let Some(shortcut) = shortcut {
        entry = entry.action(shortcut);
    }
    if let Some(tooltip) = menu_item.tooltip {
        entry = entry.documentation_aside(DocumentationSide::Right, move |_| {
            Label::new(tooltip.clone()).into_any_element()
        });
    }
    menu.item(entry)
}

fn add_plan(menu: ContextMenu, plan: Vec<MenuItemPlan>, runner: &Rc<MenuRunner>) -> ContextMenu {
    plan.into_iter().fold(menu, |menu, planned| match planned {
        MenuItemPlan::Separator => menu.separator(),
        MenuItemPlan::Item(menu_item) => add_item(menu, menu_item, runner),
        MenuItemPlan::Submenu { label, items } => {
            let runner = runner.clone();
            menu.submenu(label, move |submenu, _, _| {
                add_plan(submenu, items.clone(), &runner)
            })
        }
    })
}

pub fn build_ref_menu(
    plan: Vec<MenuItemPlan>,
    runner: MenuRunner,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Entity<ContextMenu> {
    let runner = Rc::new(runner);
    ContextMenu::build(window, cx, move |menu, _, _| {
        let menu = if runner.handles_delete_shortcut {
            menu.key_context(REF_MENU_KEY_CONTEXT)
        } else {
            menu
        };
        add_plan(menu, plan, &runner)
    })
}

/// IntelliJ asks for the new worktree in a dialog; Flint's worktree picker is where a new
/// worktree is named and created.
pub(super) fn open_worktree_picker(context: &BranchContext, window: &mut Window, cx: &mut App) {
    let Some(workspace) = context.workspace.upgrade() else {
        return;
    };
    workspace.update(cx, |workspace, cx| {
        let project = workspace.project().clone();
        let workspace_handle = workspace.weak_handle();
        workspace.toggle_modal(window, cx, |window, cx| {
            git_ui_core::worktree_picker::WorktreePicker::new_modal(
                project,
                workspace_handle,
                None,
                window,
                cx,
            )
        });
    });
}

pub fn worktree_branch_target(target: &RefTarget) -> NewWorktreeBranchTarget {
    match target.kind {
        RefKind::Local => NewWorktreeBranchTarget::ExistingBranch {
            name: target.name.to_string(),
        },
        RefKind::Remote => match target.remote_name() {
            Some(remote_name) => NewWorktreeBranchTarget::RemoteBranch {
                remote_name: remote_name.to_string(),
                branch_name: target.name_without_remote().to_string(),
            },
            None => NewWorktreeBranchTarget::ExistingBranch {
                name: target.full_ref_name(),
            },
        },
        RefKind::Tag => NewWorktreeBranchTarget::ExistingBranch {
            name: target.full_ref_name(),
        },
    }
}

/// IntelliJ names the worktree's directory after the ref, with the characters a file name can't
/// hold (and whitespace) replaced by `_` (`sanitizeFileName`).
fn worktree_directory_name(target: &RefTarget) -> String {
    let sanitized = target
        .name_without_remote()
        .chars()
        .map(|character| {
            if character.is_whitespace() || matches!(character, '/' | '\\' | ':') {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let trimmed = sanitized.trim_end_matches(['-', '_', '.']);
    if trimmed.is_empty() {
        "worktree".to_string()
    } else {
        trimmed.to_string()
    }
}

fn create_worktree_from(
    context: &BranchContext,
    target: &RefTarget,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(workspace) = context.workspace.upgrade() else {
        return;
    };
    let action = CreateWorktree {
        worktree_name: Some(worktree_directory_name(target)),
        branch_target: worktree_branch_target(target),
    };
    let repository = context.repository.clone();
    workspace.update(cx, |workspace, cx| {
        git_ui_core::worktree_service::handle_create_worktree_in_repository(
            workspace, repository, &action, window, None, cx,
        )
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_worktree_directories_are_named_after_the_ref_like_intellij() {
        assert_eq!(
            worktree_directory_name(&RefTarget::local("feature/login")),
            "feature_login"
        );
        assert_eq!(
            worktree_directory_name(&RefTarget::remote("origin/fix it")),
            "fix_it"
        );
        assert_eq!(worktree_directory_name(&RefTarget::tag("v1.0.")), "v1.0");
        assert_eq!(worktree_directory_name(&RefTarget::local("__")), "worktree");
    }

    struct Scenario {
        target: RefTarget,
        current_branch: Option<String>,
        head_short_sha: Option<String>,
        head_sha: Option<String>,
        tag_commit_sha: Option<String>,
        has_commits: bool,
        remotes: Vec<SharedString>,
        has_tracked_branch: bool,
        tracked_branch: Option<String>,
        other_worktree: Option<PathBuf>,
        fetching: bool,
        protected_branches: Vec<String>,
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
                tracked_branch: None,
                other_worktree: None,
                fetching: false,
                protected_branches: vec!["master".to_string(), "main".to_string()],
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
                tracked_branch: self.tracked_branch.as_deref(),
                other_worktree: self.other_worktree.as_deref(),
                fetching: self.fetching,
                protected_branches: &self.protected_branches,
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
                "New Worktree from 'feature/login'…",
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
                "New Worktree from 'main'…",
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
    fn running_fetch_keeps_checkout_and_update_enabled() {
        let mut scenario = Scenario::new(RefTarget::local("topic"));
        scenario.fetching = true;
        let plan = scenario.plan();
        let checkout_and_update = find_item(&plan, &MenuAction::CheckoutAndUpdate);
        assert!(checkout_and_update.enabled);
        assert_eq!(checkout_and_update.tooltip, None);
        assert!(!find_item(&plan, &MenuAction::Update).enabled);
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
                "New Worktree from 'origin/feature/x'…",
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
    fn the_tag_under_a_detached_head_hides_checkout_merge_and_delete() {
        let mut scenario = Scenario::new(RefTarget::tag("v1.0"));
        scenario.current_branch = None;
        scenario.tag_commit_sha = scenario.head_sha.clone();
        let plan = scenario.plan();
        assert_eq!(
            describe(&plan),
            vec![
                "Show Diff with Working Tree",
                "---",
                "New Worktree from 'v1.0'…",
                "---",
                "Push to origin"
            ]
        );
        assert_well_formed(&plan);
    }

    #[test]
    fn a_tag_keeps_checkout_merge_and_delete_unless_head_is_detached_exactly_on_it() {
        let mut scenario = Scenario::new(RefTarget::tag("v1.0"));
        scenario.tag_commit_sha = scenario.head_sha.clone();
        let attached = scenario.plan();
        assert!(has_item(&attached, &MenuAction::Checkout));
        assert!(has_item(&attached, &MenuAction::MergeIntoCurrent));
        assert!(has_item(&attached, &MenuAction::Delete));

        scenario.current_branch = None;
        scenario.tag_commit_sha = Some("ffffffffffffffffffffffffffffffffffffffff".to_string());
        let elsewhere = scenario.plan();
        assert!(has_item(&elsewhere, &MenuAction::Checkout));
        assert!(has_item(&elsewhere, &MenuAction::MergeIntoCurrent));
        assert!(has_item(&elsewhere, &MenuAction::Delete));

        scenario.head_sha = None;
        scenario.tag_commit_sha = None;
        let unknown = scenario.plan();
        assert!(has_item(&unknown, &MenuAction::Checkout));
        assert!(has_item(&unknown, &MenuAction::Delete));
    }

    #[test]
    fn push_and_delete_carry_their_intellij_shortcuts() {
        let shortcut = |action: MenuAction| shortcut_action(&MenuItem::new(action, "label"), true);
        assert!(
            shortcut(MenuAction::Push).is_some_and(|action| action.partial_eq(&git::PushDialog))
        );
        assert!(shortcut(MenuAction::Delete).is_some_and(|action| action.partial_eq(&DeleteRef)));
        for action in [
            MenuAction::Checkout,
            MenuAction::Update,
            MenuAction::Rename,
            MenuAction::PullUsingRebase,
            MenuAction::NewWorktreeFrom,
        ] {
            assert!(shortcut(action.clone()).is_none(), "{action:?}");
        }
        let mut tracked_delete = MenuItem::new(MenuAction::Delete, "Delete");
        tracked_delete.target = Some(RefTarget::remote("origin/main"));
        assert!(shortcut_action(&tracked_delete, true).is_none());
        assert!(shortcut_action(&MenuItem::new(MenuAction::Delete, "Delete"), false).is_none());
    }

    #[test]
    fn empty_repository_reason_matches_intellij() {
        let mut scenario = Scenario::new(RefTarget::local("topic"));
        scenario.has_commits = false;
        let checkout = find_item(&scenario.plan(), &MenuAction::Checkout);
        assert_eq!(
            checkout.tooltip.as_deref(),
            Some("Action is not possible before the first commit.")
        );
    }

    #[test]
    fn a_branch_checked_out_in_another_worktree_offers_open_worktree_and_no_rebase_checkout() {
        let mut scenario = Scenario::new(RefTarget::local("topic"));
        scenario.other_worktree = Some(PathBuf::from("/work/topic-worktree"));
        let plan = scenario.plan();
        assert_eq!(
            describe(&plan)[..4].to_vec(),
            vec![
                "Checkout",
                "Open Worktree…",
                "New Branch from 'topic'…",
                "Checkout and Update",
            ]
        );
        assert!(has_item(
            &plan,
            &MenuAction::OpenWorktree {
                path: PathBuf::from("/work/topic-worktree")
            }
        ));
        assert!(!has_item(&plan, &MenuAction::CheckoutAndRebase));
        assert_well_formed(&plan);

        scenario.other_worktree = None;
        let plan = scenario.plan();
        assert!(has_item(&plan, &MenuAction::CheckoutAndRebase));
        assert!(
            !plan
                .iter()
                .any(|planned| matches!(planned, MenuItemPlan::Item(item) if matches!(item.action, MenuAction::OpenWorktree { .. })))
        );
    }

    #[test]
    fn the_tracked_branch_submenu_follows_push_and_acts_on_the_remote_branch() {
        let mut scenario = Scenario::new(RefTarget::local("topic"));
        scenario.tracked_branch = Some("origin/topic".to_string());
        let plan = scenario.plan();
        let push = plan
            .iter()
            .position(|planned| matches!(planned, MenuItemPlan::Item(item) if item.action == MenuAction::Push))
            .expect("push is listed");
        let Some(MenuItemPlan::Submenu { label, items }) = plan.get(push + 1) else {
            panic!(
                "the tracked branch submenu follows Push…: {:?}",
                describe(&plan)
            );
        };
        assert_eq!(label, "Tracked Branch 'origin/topic'");
        assert_eq!(
            describe(items),
            vec![
                "Checkout",
                "New Branch from 'origin/topic'…",
                "Checkout and Rebase onto 'main'",
                "---",
                "Compare with 'main'",
                "Show Diff with Working Tree",
                "---",
                "Rebase 'main' onto 'origin/topic'",
                "Merge 'origin/topic' into 'main'",
                "---",
                "New Worktree from 'origin/topic'…",
                "---",
                "Pull into 'main' Using Rebase",
                "Pull into 'main' Using Merge",
                "---",
                "Delete",
            ]
        );
        let remote = RefTarget::remote("origin/topic");
        let local = RefTarget::local("topic");
        for planned in items {
            if let MenuItemPlan::Item(item) = planned {
                assert_eq!(item.target_or(&local), &remote, "{}", item.label);
            }
        }
        assert_eq!(
            find_item(&plan, &MenuAction::Push).target_or(&local),
            &local
        );

        scenario.tracked_branch = None;
        assert!(
            !scenario
                .plan()
                .iter()
                .any(|planned| matches!(planned, MenuItemPlan::Submenu { .. }))
        );
    }

    #[test]
    fn the_tracked_branch_is_the_remote_upstream_while_it_exists() {
        let branch = |upstream: &str, tracking: UpstreamTracking| Branch {
            is_head: false,
            ref_name: SharedString::from("refs/heads/topic"),
            upstream: Some(git::repository::Upstream {
                ref_name: SharedString::from(upstream.to_string()),
                tracking,
            }),
            most_recent_commit: None,
        };
        let tracked = UpstreamTracking::Tracked(git::repository::UpstreamTrackingStatus {
            ahead: 0,
            behind: 0,
        });
        assert_eq!(
            tracked_remote_branch(&branch("refs/remotes/origin/topic", tracked)).as_deref(),
            Some("origin/topic")
        );
        assert_eq!(
            tracked_remote_branch(&branch("refs/remotes/origin/topic", UpstreamTracking::Gone)),
            None
        );
        assert_eq!(
            tracked_remote_branch(&branch("refs/heads/main", tracked)),
            None
        );
    }

    #[test]
    fn the_worktree_of_a_branch_is_found_by_its_full_ref_name() {
        let worktrees = vec![
            Worktree {
                path: PathBuf::from("/work/main"),
                ref_name: Some(SharedString::from("refs/heads/main")),
                sha: SharedString::from("aaa"),
                is_main: true,
                is_bare: false,
            },
            Worktree {
                path: PathBuf::from("/work/topic"),
                ref_name: Some(SharedString::from("refs/heads/feature/topic")),
                sha: SharedString::from("bbb"),
                is_main: false,
                is_bare: false,
            },
        ];
        assert_eq!(
            worktree_with_branch("feature/topic", &worktrees),
            Some(Path::new("/work/topic"))
        );
        assert_eq!(
            worktree_with_branch("main", &worktrees),
            Some(Path::new("/work/main"))
        );
        assert_eq!(worktree_with_branch("topic", &worktrees), None);
    }

    #[test]
    fn new_worktree_from_keeps_long_names_and_targets_the_ref() {
        let long_name = format!("feature/{}", "x".repeat(60));
        let plan = Scenario::new(RefTarget::local(long_name.clone())).plan();
        assert_eq!(
            find_item(&plan, &MenuAction::NewWorktreeFrom).label,
            format!("New Worktree from '{long_name}'…")
        );
        assert_eq!(
            worktree_branch_target(&RefTarget::local("topic")),
            NewWorktreeBranchTarget::ExistingBranch {
                name: "topic".to_string()
            }
        );
        assert_eq!(
            worktree_branch_target(&RefTarget::remote("origin/feature/x")),
            NewWorktreeBranchTarget::RemoteBranch {
                remote_name: "origin".to_string(),
                branch_name: "feature/x".to_string()
            }
        );
        assert_eq!(
            worktree_branch_target(&RefTarget::tag("v1.0")),
            NewWorktreeBranchTarget::ExistingBranch {
                name: "refs/tags/v1.0".to_string()
            }
        );
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
                "New Worktree from 'v1.0'…",
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
                    MenuItemPlan::Submenu { items, .. } => assert!(items.iter().all(
                        |planned| !matches!(planned, MenuItemPlan::Item(menu_item) if menu_item.enabled)
                    )),
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
