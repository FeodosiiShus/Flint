use std::cmp::Ordering;
use std::rc::Rc;
use std::sync::Arc;

use git::repository::Branch;
use git::{BuildCommitPermalinkParams, GitHostingProviderRegistry, Oid, parse_git_remote_url};
use gpui::{Action, App, Entity, FocusHandle, SharedString, WeakEntity, Window};
use project::git_store::Repository;
use ui::{ContextMenu, ContextMenuEntry, DocumentationSide, IconName, prelude::*};
use util::ResultExt as _;
use workspace::Workspace;

use super::{
    CherryPick, CopyRevisionNumber, GoToChildCommit, GoToParentCommit, NewBranchFromCommit,
    new_tag_dialog, operations, reset_dialog,
};
use crate::branch_operations::{BranchContext, checkout, quoted_ref_label, ref_diff};
use crate::branch_refs::{RefKind, RefTarget};
use crate::branches_popup::ref_menu::{self, MenuContext, MenuItem, MenuItemPlan};
use crate::git_log::ref_labels::{RefLabel, RefLabelKind};
use crate::merge_tool::conflict_resolution::labels::short_hash;

const COPY_REVISION_NUMBER_TEXT: &str = "Copy Revision Number";
const CHERRY_PICK_TEXT: &str = "Cherry-Pick";
const CHECKOUT_GROUP_TEXT: &str = "Checkout";
const COMPARE_WITH_LOCAL_TEXT: &str = "Compare with Local";
const RESET_TEXT: &str = "Reset Current Branch to Here…";
const UNDO_COMMIT_TEXT: &str = "Undo Commit…";
const BRANCHES_GROUP_TEXT: &str = "Branches";
const TAGS_GROUP_TEXT: &str = "Tags";
const NEW_BRANCH_TEXT: &str = "New Branch…";
const NEW_TAG_TEXT: &str = "New Tag…";
const GO_TO_CHILD_TEXT: &str = "Go to Child Commit";
const GO_TO_PARENT_TEXT: &str = "Go to Parent Commit";
const VIEW_IN_BROWSER_TEXT: &str = "View in browser";
const REVERT_MERGE_COMMIT_DESCRIPTION: &str = "Reverting merge commits is not allowed";
const UNDO_NOT_LAST_COMMIT_DESCRIPTION: &str =
    "The selected commit is not the last in the current branch";
const NEW_BRANCH_SEVERAL_COMMITS_DESCRIPTION: &str = "Select single commit to create a new branch";
const MAX_BRANCH_GROUPS: usize = 2;
const MAX_TAG_GROUPS: usize = 1;
const MASTER_BRANCH_NAMES: [&str; 2] = ["master", "main"];
const ORIGIN_MASTER_BRANCH_NAMES: [&str; 2] = ["origin/master", "origin/main"];

#[derive(Clone, Debug)]
pub(crate) struct MenuCommit {
    pub(crate) sha: git::Oid,
    pub(crate) parents: Vec<git::Oid>,
    pub(crate) labels: Vec<RefLabel>,
    pub(crate) data: Option<std::sync::Arc<git::repository::CommitData>>,
}

pub(crate) struct CommitMenuRequest {
    pub(crate) workspace: WeakEntity<Workspace>,
    pub(crate) repository: Entity<Repository>,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) commits: Vec<MenuCommit>,
    pub(crate) cherry_pick_enabled: bool,
    pub(crate) can_go_to_parent: bool,
    pub(crate) can_go_to_child: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuEntry {
    CopyRevisionNumber,
    CherryPick,
    Checkout,
    CompareWithLocal,
    ResetCurrentBranch,
    RevertCommits,
    UndoCommit,
    BranchOperations,
    NewBranch,
    NewTag,
    GoToChildCommit,
    GoToParentCommit,
    ViewInBrowser,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EntryState {
    enabled: bool,
    disabled_reason: Option<String>,
}

impl EntryState {
    fn enabled_when(enabled: bool) -> Self {
        Self {
            enabled,
            disabled_reason: None,
        }
    }

    fn disabled(reason: impl Into<String>) -> Self {
        Self {
            enabled: false,
            disabled_reason: Some(reason.into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MenuRow {
    Separator,
    Entry(MenuEntry, EntryState),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct MenuFacts {
    selected: usize,
    any_merge_commit: bool,
    single_parent_count: Option<usize>,
    single_is_head: bool,
    cherry_pick_enabled: bool,
    can_go_to_parent: bool,
    can_go_to_child: bool,
    has_ref_operations: bool,
    has_browser_link: bool,
}

fn copy_revision_number_state(facts: &MenuFacts) -> Option<EntryState> {
    Some(EntryState::enabled_when(facts.selected > 0))
}

fn cherry_pick_state(facts: &MenuFacts) -> Option<EntryState> {
    Some(EntryState::enabled_when(facts.cherry_pick_enabled))
}

fn checkout_state(facts: &MenuFacts) -> Option<EntryState> {
    (facts.selected == 1).then(|| EntryState::enabled_when(true))
}

fn compare_with_local_state(facts: &MenuFacts) -> Option<EntryState> {
    (facts.selected == 1).then(|| EntryState::enabled_when(true))
}

fn reset_state(facts: &MenuFacts) -> Option<EntryState> {
    Some(EntryState::enabled_when(facts.selected == 1))
}

fn revert_state(facts: &MenuFacts) -> Option<EntryState> {
    if facts.selected == 0 {
        return None;
    }
    if facts.any_merge_commit {
        return Some(EntryState::disabled(REVERT_MERGE_COMMIT_DESCRIPTION));
    }
    Some(EntryState::enabled_when(true))
}

fn undo_commit_state(facts: &MenuFacts) -> Option<EntryState> {
    let parent_count = facts.single_parent_count?;
    if parent_count > 1 {
        return Some(EntryState::disabled(format!(
            "Selected commit has {parent_count} parents"
        )));
    }
    if !facts.single_is_head {
        return Some(EntryState::disabled(UNDO_NOT_LAST_COMMIT_DESCRIPTION));
    }
    Some(EntryState::enabled_when(true))
}

fn branch_operations_state(facts: &MenuFacts) -> Option<EntryState> {
    (facts.selected == 1 && facts.has_ref_operations).then(|| EntryState::enabled_when(true))
}

fn new_branch_state(facts: &MenuFacts) -> Option<EntryState> {
    match facts.selected {
        0 => None,
        1 => Some(EntryState::enabled_when(true)),
        _ => Some(EntryState::disabled(NEW_BRANCH_SEVERAL_COMMITS_DESCRIPTION)),
    }
}

fn new_tag_state(facts: &MenuFacts) -> Option<EntryState> {
    (facts.selected > 0).then(|| EntryState::enabled_when(facts.selected == 1))
}

fn view_in_browser_state(facts: &MenuFacts) -> Option<EntryState> {
    facts
        .has_browser_link
        .then(|| EntryState::enabled_when(true))
}

fn revert_text(selected: usize) -> &'static str {
    if selected >= 2 {
        "Revert Commits"
    } else {
        "Revert Commit"
    }
}

fn menu_rows(facts: &MenuFacts) -> Vec<MenuRow> {
    let sections = [
        vec![
            (
                MenuEntry::CopyRevisionNumber,
                copy_revision_number_state(facts),
            ),
            (MenuEntry::CherryPick, cherry_pick_state(facts)),
        ],
        vec![
            (MenuEntry::Checkout, checkout_state(facts)),
            (MenuEntry::CompareWithLocal, compare_with_local_state(facts)),
        ],
        vec![
            (MenuEntry::ResetCurrentBranch, reset_state(facts)),
            (MenuEntry::RevertCommits, revert_state(facts)),
            (MenuEntry::UndoCommit, undo_commit_state(facts)),
        ],
        vec![
            (MenuEntry::BranchOperations, branch_operations_state(facts)),
            (MenuEntry::NewBranch, new_branch_state(facts)),
            (MenuEntry::NewTag, new_tag_state(facts)),
        ],
        vec![
            (
                MenuEntry::GoToChildCommit,
                Some(EntryState::enabled_when(facts.can_go_to_child)),
            ),
            (
                MenuEntry::GoToParentCommit,
                Some(EntryState::enabled_when(facts.can_go_to_parent)),
            ),
        ],
        vec![(MenuEntry::ViewInBrowser, view_in_browser_state(facts))],
    ];
    let mut rows = Vec::new();
    for section in sections {
        let visible: Vec<MenuRow> = section
            .into_iter()
            .filter_map(|(entry, state)| state.map(|state| MenuRow::Entry(entry, state)))
            .collect();
        if visible.is_empty() {
            continue;
        }
        if !rows.is_empty() {
            rows.push(MenuRow::Separator);
        }
        rows.extend(visible);
    }
    rows
}

fn label_rank(label: &RefLabel, current_branch: Option<&str>) -> usize {
    match label.kind {
        RefLabelKind::Head => 0,
        RefLabelKind::LocalBranch if current_branch == Some(&*label.name) => 1,
        RefLabelKind::LocalBranch if MASTER_BRANCH_NAMES.contains(&&*label.name) => 2,
        RefLabelKind::RemoteBranch if ORIGIN_MASTER_BRANCH_NAMES.contains(&&*label.name) => 3,
        RefLabelKind::LocalBranch => 4,
        RefLabelKind::RemoteBranch => 5,
        RefLabelKind::Tag => 6,
        RefLabelKind::Other => 7,
    }
}

fn compare_labels(left: &RefLabel, right: &RefLabel, current_branch: Option<&str>) -> Ordering {
    label_rank(left, current_branch)
        .cmp(&label_rank(right, current_branch))
        .then_with(|| util::paths::natural_sort(&left.name, &right.name))
}

fn sorted_labels<'a>(
    labels: impl Iterator<Item = &'a RefLabel>,
    current_branch: Option<&str>,
) -> Vec<&'a RefLabel> {
    let mut labels: Vec<&RefLabel> = labels.collect();
    labels.sort_by(|left, right| compare_labels(left, right, current_branch));
    labels
}

fn checkout_branch_names(labels: &[RefLabel], current_branch: Option<&str>) -> Vec<SharedString> {
    sorted_labels(
        labels.iter().filter(|label| {
            label.kind == RefLabelKind::LocalBranch && current_branch != Some(&*label.name)
        }),
        current_branch,
    )
    .into_iter()
    .map(|label| label.name.clone())
    .collect()
}

fn operation_branch_targets(
    labels: &[RefLabel],
    current_branch: Option<&str>,
    branches: &[Branch],
) -> Vec<RefTarget> {
    let exists = |name: &str, is_remote: bool| {
        branches
            .iter()
            .any(|branch| branch.is_remote() == is_remote && branch.name() == name)
    };
    sorted_labels(
        labels.iter().filter(|label| match label.kind {
            RefLabelKind::LocalBranch => {
                current_branch != Some(&*label.name) && exists(label.name.as_str(), false)
            }
            RefLabelKind::RemoteBranch => exists(label.name.as_str(), true),
            RefLabelKind::Head | RefLabelKind::Tag | RefLabelKind::Other => false,
        }),
        current_branch,
    )
    .into_iter()
    .map(|label| match label.kind {
        RefLabelKind::RemoteBranch => RefTarget::remote(label.name.clone()),
        _ => RefTarget::local(label.name.clone()),
    })
    .collect()
}

fn operation_tag_targets(labels: &[RefLabel], current_branch: Option<&str>) -> Vec<RefTarget> {
    sorted_labels(
        labels
            .iter()
            .filter(|label| label.kind == RefLabelKind::Tag),
        current_branch,
    )
    .into_iter()
    .map(|label| RefTarget::tag(label.name.clone()))
    .collect()
}

#[derive(Clone)]
struct RefOperation {
    target: RefTarget,
    plan: Vec<MenuItemPlan>,
}

#[derive(Clone)]
struct BrowserLink {
    url: String,
    icon: IconName,
}

struct MenuData {
    context: BranchContext,
    commits: Vec<MenuCommit>,
    checkout_branches: Vec<SharedString>,
    branch_operations: Vec<RefOperation>,
    tag_operations: Vec<RefOperation>,
    browser_link: Option<BrowserLink>,
}

struct RepositoryFacts {
    current_branch: Option<String>,
    head_sha: Option<String>,
    branches: Arc<[Branch]>,
    remote_names: Vec<SharedString>,
    default_remote_url: Option<String>,
}

impl RepositoryFacts {
    fn read(repository: &Repository) -> Self {
        let mut remote_names: Vec<SharedString> = Vec::new();
        for branch in repository.branch_list.iter() {
            if let Some(remote) = branch.remote_name()
                && !remote_names.iter().any(|known| known.as_ref() == remote)
            {
                remote_names.push(SharedString::from(remote.to_string()));
            }
        }
        Self {
            current_branch: repository
                .branch
                .as_ref()
                .map(|branch| branch.name().to_string()),
            head_sha: repository
                .head_commit
                .as_ref()
                .map(|commit| commit.sha.to_string()),
            branches: repository.branch_list.clone(),
            remote_names,
            default_remote_url: repository.default_remote_url(),
        }
    }

    fn ref_operation(&self, target: RefTarget, commit_sha: &str) -> RefOperation {
        let head_short_sha = self.head_sha.as_deref().map(short_hash);
        let has_tracked_branch = target.kind == RefKind::Local
            && self.branches.iter().any(|branch| {
                !branch.is_remote() && branch.name() == &*target.name && branch.upstream.is_some()
            });
        let plan = ref_menu::menu_plan(&MenuContext {
            target: &target,
            current_branch: self.current_branch.as_deref(),
            head_short_sha: head_short_sha.as_deref(),
            head_sha: self.head_sha.as_deref(),
            tag_commit_sha: (target.kind == RefKind::Tag).then_some(commit_sha),
            has_commits: self.head_sha.is_some(),
            remotes: &self.remote_names,
            has_tracked_branch,
            fetching: false,
        });
        RefOperation { target, plan }
    }
}

fn browser_link(remote_url: Option<String>, commit: Oid, cx: &mut App) -> Option<BrowserLink> {
    let remote_url = remote_url?;
    let registry = GitHostingProviderRegistry::default_global(cx);
    let (provider, parsed) = parse_git_remote_url(registry, &remote_url)?;
    let sha = commit.to_string();
    let url = provider
        .build_commit_permalink(&parsed, BuildCommitPermalinkParams { sha: &sha })
        .to_string();
    Some(BrowserLink {
        url,
        icon: ui::git_hosting_provider_icon(provider.name().as_str()),
    })
}

pub(crate) fn commit_context_menu(
    request: CommitMenuRequest,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ContextMenu> {
    let facts = RepositoryFacts::read(request.repository.read(cx));
    let single = match request.commits.as_slice() {
        [commit] => Some(commit.clone()),
        _ => None,
    };
    let (checkout_branches, branch_operations, tag_operations) = match &single {
        Some(commit) => {
            let sha = commit.sha.to_string();
            let current_branch = facts.current_branch.as_deref();
            (
                checkout_branch_names(&commit.labels, current_branch),
                operation_branch_targets(&commit.labels, current_branch, &facts.branches)
                    .into_iter()
                    .map(|target| facts.ref_operation(target, &sha))
                    .collect(),
                operation_tag_targets(&commit.labels, current_branch)
                    .into_iter()
                    .map(|target| facts.ref_operation(target, &sha))
                    .collect(),
            )
        }
        None => (Vec::new(), Vec::new(), Vec::new()),
    };
    let browser_link = request
        .commits
        .first()
        .and_then(|commit| browser_link(facts.default_remote_url.clone(), commit.sha, cx));
    let menu_facts = MenuFacts {
        selected: request.commits.len(),
        any_merge_commit: request
            .commits
            .iter()
            .any(|commit| commit.parents.len() > 1),
        single_parent_count: single.as_ref().map(|commit| commit.parents.len()),
        single_is_head: single.as_ref().is_some_and(|commit| {
            facts.head_sha.as_deref() == Some(commit.sha.to_string().as_str())
        }),
        cherry_pick_enabled: request.cherry_pick_enabled,
        can_go_to_parent: request.can_go_to_parent,
        can_go_to_child: request.can_go_to_child,
        has_ref_operations: !branch_operations.is_empty() || !tag_operations.is_empty(),
        has_browser_link: browser_link.is_some(),
    };
    let rows = menu_rows(&menu_facts);
    let data = Rc::new(MenuData {
        context: BranchContext::new(request.workspace, request.repository),
        commits: request.commits,
        checkout_branches,
        branch_operations,
        tag_operations,
        browser_link,
    });
    let focus_handle = request.focus_handle;
    ContextMenu::build(window, cx, move |menu, _, _| {
        rows.into_iter()
            .fold(menu.context(focus_handle), |menu, row| match row {
                MenuRow::Separator => menu.separator(),
                MenuRow::Entry(entry, state) => add_entry(menu, entry, state, &data),
            })
    })
}

fn add_entry(
    menu: ContextMenu,
    entry: MenuEntry,
    state: EntryState,
    data: &Rc<MenuData>,
) -> ContextMenu {
    match entry {
        MenuEntry::CopyRevisionNumber => action_entry(
            menu,
            COPY_REVISION_NUMBER_TEXT,
            Box::new(CopyRevisionNumber),
            state,
        ),
        MenuEntry::CherryPick => action_entry(menu, CHERRY_PICK_TEXT, Box::new(CherryPick), state),
        MenuEntry::Checkout => add_checkout(menu, data),
        MenuEntry::CompareWithLocal => handler_entry(
            menu,
            COMPARE_WITH_LOCAL_TEXT,
            state,
            data,
            compare_with_local,
        ),
        MenuEntry::ResetCurrentBranch => {
            handler_entry(menu, RESET_TEXT, state, data, |data, window, cx| {
                if let Some(commit) = data.commits.first() {
                    reset_dialog::reset_current_branch_to(
                        data.context.clone(),
                        commit.clone(),
                        window,
                        cx,
                    );
                }
            })
        }
        MenuEntry::RevertCommits => handler_entry(
            menu,
            revert_text(data.commits.len()),
            state,
            data,
            |data, window, cx| {
                operations::revert_commits(data.context.clone(), data.commits.clone(), window, cx)
            },
        ),
        MenuEntry::UndoCommit => {
            handler_entry(menu, UNDO_COMMIT_TEXT, state, data, |data, window, cx| {
                if let Some(commit) = data.commits.first() {
                    reset_dialog::undo_commit(data.context.clone(), commit.clone(), window, cx);
                }
            })
        }
        MenuEntry::BranchOperations => add_branch_operations(menu, data),
        MenuEntry::NewBranch => {
            action_entry(menu, NEW_BRANCH_TEXT, Box::new(NewBranchFromCommit), state)
        }
        MenuEntry::NewTag => handler_entry(menu, NEW_TAG_TEXT, state, data, |data, window, cx| {
            if let Some(commit) = data.commits.first() {
                new_tag_dialog::open(data.context.clone(), commit.sha, window, cx);
            }
        }),
        MenuEntry::GoToChildCommit => {
            action_entry(menu, GO_TO_CHILD_TEXT, Box::new(GoToChildCommit), state)
        }
        MenuEntry::GoToParentCommit => {
            action_entry(menu, GO_TO_PARENT_TEXT, Box::new(GoToParentCommit), state)
        }
        MenuEntry::ViewInBrowser => match data.browser_link.clone() {
            Some(BrowserLink { url, icon }) => menu.item(
                ContextMenuEntry::new(VIEW_IN_BROWSER_TEXT)
                    .icon(icon)
                    .handler(move |_, cx| cx.open_url(&url)),
            ),
            None => menu,
        },
    }
}

fn disabled_entry(
    menu: ContextMenu,
    entry: ContextMenuEntry,
    reason: Option<String>,
) -> ContextMenu {
    let mut entry = entry.disabled(true);
    if let Some(reason) = reason {
        entry = entry.documentation_aside(DocumentationSide::Right, move |_| {
            Label::new(reason.clone()).into_any_element()
        });
    }
    menu.item(entry)
}

fn action_entry(
    menu: ContextMenu,
    label: &'static str,
    action: Box<dyn Action>,
    state: EntryState,
) -> ContextMenu {
    if state.enabled {
        return menu.action_disabled_when(false, label, action);
    }
    disabled_entry(
        menu,
        ContextMenuEntry::new(label).action(action),
        state.disabled_reason,
    )
}

fn handler_entry(
    menu: ContextMenu,
    label: &'static str,
    state: EntryState,
    data: &Rc<MenuData>,
    run: fn(&MenuData, &mut Window, &mut App),
) -> ContextMenu {
    if !state.enabled {
        return disabled_entry(menu, ContextMenuEntry::new(label), state.disabled_reason);
    }
    let data = data.clone();
    menu.entry(label, None, move |window, cx| run(&data, window, cx))
}

fn compare_with_local(data: &MenuData, window: &mut Window, cx: &mut App) {
    let Some(commit) = data.commits.first() else {
        return;
    };
    let repository = data.context.repository.clone();
    let base_ref = SharedString::from(commit.sha.to_string());
    data.context
        .workspace
        .update(cx, |workspace, cx| {
            ref_diff::open_working_tree_diff(workspace, repository, base_ref, window, cx);
        })
        .log_err();
}

fn checkout_revision(data: &MenuData, window: &mut Window, cx: &mut App) {
    if let Some(commit) = data.commits.first() {
        operations::checkout_revision(data.context.clone(), commit.sha, window, cx);
    }
}

fn add_checkout(menu: ContextMenu, data: &Rc<MenuData>) -> ContextMenu {
    let Some(commit) = data.commits.first() else {
        return menu;
    };
    let short = short_hash(&commit.sha.to_string());
    if data.checkout_branches.is_empty() {
        let data = data.clone();
        return menu.entry(
            format!("Checkout Revision '{short}'"),
            None,
            move |window, cx| checkout_revision(&data, window, cx),
        );
    }
    let data = data.clone();
    menu.submenu(CHECKOUT_GROUP_TEXT, move |submenu, _, _| {
        let submenu = data
            .checkout_branches
            .iter()
            .fold(submenu, |submenu, name| {
                let context = data.context.clone();
                let target = RefTarget::local(name.clone());
                submenu.entry(name.clone(), None, move |window, cx| {
                    let context = context.clone();
                    let target = target.clone();
                    window.defer(cx, move |window, cx| {
                        checkout::checkout(context, target, window, cx)
                    });
                })
            });
        let data = data.clone();
        submenu.entry(format!("Revision '{short}'"), None, move |window, cx| {
            checkout_revision(&data, window, cx)
        })
    })
}

fn add_branch_operations(menu: ContextMenu, data: &Rc<MenuData>) -> ContextMenu {
    let menu = add_ref_operations(
        menu,
        &data.branch_operations,
        MAX_BRANCH_GROUPS,
        BRANCHES_GROUP_TEXT,
        "Branch",
        &data.context,
    );
    add_ref_operations(
        menu,
        &data.tag_operations,
        MAX_TAG_GROUPS,
        TAGS_GROUP_TEXT,
        "Tag",
        &data.context,
    )
}

fn add_ref_operations(
    menu: ContextMenu,
    operations: &[RefOperation],
    max_inline_groups: usize,
    group_text: &'static str,
    kind_text: &'static str,
    context: &BranchContext,
) -> ContextMenu {
    if operations.len() > max_inline_groups {
        let operations = operations.to_vec();
        let context = context.clone();
        return menu.submenu(group_text, move |submenu, _, _| {
            operations.iter().fold(submenu, |submenu, operation| {
                add_ref_operation(submenu, operation.target.name.clone(), operation, &context)
            })
        });
    }
    operations.iter().fold(menu, |menu, operation| {
        let label = format!("{kind_text} {}", quoted_ref_label(&operation.target.name));
        add_ref_operation(menu, label.into(), operation, context)
    })
}

fn add_ref_operation(
    menu: ContextMenu,
    label: SharedString,
    operation: &RefOperation,
    context: &BranchContext,
) -> ContextMenu {
    let operation = operation.clone();
    let context = context.clone();
    menu.submenu(label, move |submenu, _, _| {
        operation
            .plan
            .iter()
            .cloned()
            .fold(submenu, |submenu, planned| match planned {
                MenuItemPlan::Separator => submenu.separator(),
                MenuItemPlan::Item(item) => {
                    add_ref_item(submenu, item, &context, &operation.target)
                }
                MenuItemPlan::Submenu { label, items } => {
                    let context = context.clone();
                    let target = operation.target.clone();
                    submenu.submenu(label, move |nested, _, _| {
                        items.iter().cloned().fold(nested, |nested, item| {
                            add_ref_item(nested, item, &context, &target)
                        })
                    })
                }
            })
    })
}

fn add_ref_item(
    menu: ContextMenu,
    item: MenuItem,
    context: &BranchContext,
    target: &RefTarget,
) -> ContextMenu {
    if !item.enabled {
        return disabled_entry(menu, ContextMenuEntry::new(item.label), item.tooltip);
    }
    let context = context.clone();
    let target = target.clone();
    let action = item.action;
    menu.entry(item.label, None, move |window, cx| {
        let action = action.clone();
        let context = context.clone();
        let target = target.clone();
        window.defer(cx, move |window, cx| {
            ref_menu::run_menu_action(action, context, target, window, cx)
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(selected: usize) -> MenuFacts {
        MenuFacts {
            selected,
            single_parent_count: (selected == 1).then_some(1),
            cherry_pick_enabled: selected > 0,
            can_go_to_parent: selected == 1,
            can_go_to_child: selected == 1,
            ..MenuFacts::default()
        }
    }

    fn entries(rows: &[MenuRow]) -> Vec<MenuEntry> {
        rows.iter()
            .filter_map(|row| match row {
                MenuRow::Entry(entry, _) => Some(*entry),
                MenuRow::Separator => None,
            })
            .collect()
    }

    fn state_of(rows: &[MenuRow], wanted: MenuEntry) -> Option<EntryState> {
        rows.iter().find_map(|row| match row {
            MenuRow::Entry(entry, state) if *entry == wanted => Some(state.clone()),
            _ => None,
        })
    }

    fn label(name: &str, kind: RefLabelKind) -> RefLabel {
        RefLabel {
            name: SharedString::from(name.to_string()),
            kind,
        }
    }

    fn branch(ref_name: &str) -> Branch {
        Branch {
            is_head: false,
            ref_name: SharedString::from(ref_name.to_string()),
            upstream: None,
            most_recent_commit: None,
        }
    }

    #[test]
    fn single_commit_menu_keeps_the_intellij_order_with_separators_between_groups() {
        let rows = menu_rows(&MenuFacts {
            single_is_head: true,
            has_ref_operations: true,
            has_browser_link: true,
            ..facts(1)
        });
        assert_eq!(
            entries(&rows),
            vec![
                MenuEntry::CopyRevisionNumber,
                MenuEntry::CherryPick,
                MenuEntry::Checkout,
                MenuEntry::CompareWithLocal,
                MenuEntry::ResetCurrentBranch,
                MenuEntry::RevertCommits,
                MenuEntry::UndoCommit,
                MenuEntry::BranchOperations,
                MenuEntry::NewBranch,
                MenuEntry::NewTag,
                MenuEntry::GoToChildCommit,
                MenuEntry::GoToParentCommit,
                MenuEntry::ViewInBrowser,
            ]
        );
        let separators = rows
            .iter()
            .filter(|row| **row == MenuRow::Separator)
            .count();
        assert_eq!(separators, 5, "six visible groups need five separators");
        assert_ne!(rows.first(), Some(&MenuRow::Separator));
        assert_ne!(rows.last(), Some(&MenuRow::Separator));
    }

    #[test]
    fn empty_selection_shows_only_the_always_visible_actions_disabled() {
        let rows = menu_rows(&facts(0));
        assert_eq!(
            entries(&rows),
            vec![
                MenuEntry::CopyRevisionNumber,
                MenuEntry::CherryPick,
                MenuEntry::ResetCurrentBranch,
                MenuEntry::GoToChildCommit,
                MenuEntry::GoToParentCommit,
            ]
        );
        assert!(rows.iter().all(|row| match row {
            MenuRow::Entry(_, state) => !state.enabled,
            MenuRow::Separator => true,
        }));
    }

    #[test]
    fn two_commits_hide_single_commit_actions_and_explain_disabled_ones() {
        let rows = menu_rows(&facts(2));
        assert_eq!(
            entries(&rows),
            vec![
                MenuEntry::CopyRevisionNumber,
                MenuEntry::CherryPick,
                MenuEntry::ResetCurrentBranch,
                MenuEntry::RevertCommits,
                MenuEntry::NewBranch,
                MenuEntry::NewTag,
                MenuEntry::GoToChildCommit,
                MenuEntry::GoToParentCommit,
            ]
        );
        assert_eq!(
            state_of(&rows, MenuEntry::ResetCurrentBranch),
            Some(EntryState::enabled_when(false))
        );
        assert_eq!(
            state_of(&rows, MenuEntry::RevertCommits),
            Some(EntryState::enabled_when(true))
        );
        assert_eq!(
            state_of(&rows, MenuEntry::NewBranch),
            Some(EntryState::disabled(NEW_BRANCH_SEVERAL_COMMITS_DESCRIPTION))
        );
        assert_eq!(
            state_of(&rows, MenuEntry::NewTag),
            Some(EntryState::enabled_when(false))
        );
        assert_eq!(revert_text(2), "Revert Commits");
        assert_eq!(revert_text(1), "Revert Commit");
    }

    #[test]
    fn merge_commits_cannot_be_reverted_or_undone() {
        let rows = menu_rows(&MenuFacts {
            any_merge_commit: true,
            single_parent_count: Some(2),
            single_is_head: true,
            ..facts(1)
        });
        assert_eq!(
            state_of(&rows, MenuEntry::RevertCommits),
            Some(EntryState::disabled(REVERT_MERGE_COMMIT_DESCRIPTION))
        );
        assert_eq!(
            state_of(&rows, MenuEntry::UndoCommit),
            Some(EntryState::disabled("Selected commit has 2 parents"))
        );
    }

    #[test]
    fn undo_commit_is_enabled_only_for_the_head_commit() {
        let not_head = menu_rows(&facts(1));
        assert_eq!(
            state_of(&not_head, MenuEntry::UndoCommit),
            Some(EntryState::disabled(UNDO_NOT_LAST_COMMIT_DESCRIPTION))
        );
        let head = menu_rows(&MenuFacts {
            single_is_head: true,
            ..facts(1)
        });
        assert_eq!(
            state_of(&head, MenuEntry::UndoCommit),
            Some(EntryState::enabled_when(true))
        );
    }

    #[test]
    fn table_flags_drive_cherry_pick_and_navigation() {
        let rows = menu_rows(&MenuFacts {
            cherry_pick_enabled: false,
            can_go_to_parent: false,
            can_go_to_child: true,
            ..facts(1)
        });
        assert_eq!(
            state_of(&rows, MenuEntry::CherryPick),
            Some(EntryState::enabled_when(false))
        );
        assert_eq!(
            state_of(&rows, MenuEntry::GoToParentCommit),
            Some(EntryState::enabled_when(false))
        );
        assert_eq!(
            state_of(&rows, MenuEntry::GoToChildCommit),
            Some(EntryState::enabled_when(true))
        );
    }

    #[test]
    fn checkout_offers_local_branches_other_than_the_current_one_in_label_order() {
        let labels = [
            label("HEAD", RefLabelKind::Head),
            label("feature10", RefLabelKind::LocalBranch),
            label("current", RefLabelKind::LocalBranch),
            label("origin/main", RefLabelKind::RemoteBranch),
            label("feature9", RefLabelKind::LocalBranch),
            label("main", RefLabelKind::LocalBranch),
        ];
        assert_eq!(
            checkout_branch_names(&labels, Some("current")),
            vec![
                SharedString::from("main"),
                SharedString::from("feature9"),
                SharedString::from("feature10"),
            ]
        );
    }

    #[test]
    fn branch_operations_skip_the_current_branch_and_unknown_remote_refs() {
        let labels = [
            label("origin/feature", RefLabelKind::RemoteBranch),
            label("current", RefLabelKind::LocalBranch),
            label("feature", RefLabelKind::LocalBranch),
            label("origin/HEAD", RefLabelKind::RemoteBranch),
            label("origin/main", RefLabelKind::RemoteBranch),
            label("v1", RefLabelKind::Tag),
        ];
        let branches = [
            branch("refs/heads/current"),
            branch("refs/heads/feature"),
            branch("refs/remotes/origin/feature"),
            branch("refs/remotes/origin/main"),
        ];
        assert_eq!(
            operation_branch_targets(&labels, Some("current"), &branches),
            vec![
                RefTarget::remote("origin/main"),
                RefTarget::local("feature"),
                RefTarget::remote("origin/feature"),
            ]
        );
        assert_eq!(
            operation_tag_targets(&labels, Some("current")),
            vec![RefTarget::tag("v1")]
        );
    }
}
