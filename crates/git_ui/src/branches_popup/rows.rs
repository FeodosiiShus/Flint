use collections::{HashMap, HashSet};
use gpui::SharedString;
use project::git_store::RepositoryId;
use ui::{ContextMenu, PopoverMenuHandle};

use super::top_actions::{self, TopAction, TopActionItem, TopActionKind};
use super::tree::{self, FolderRow, RefRow, SectionKind, SectionRow, TreeRow};
use crate::branch_refs::{RefKind, RefTarget};

const CLEAR_FILTER_QUERY: &str = "/";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryChoice {
    pub id: RepositoryId,
    pub name: SharedString,
    pub branch: Option<SharedString>,
    pub is_active: bool,
    /// The position among the repositories sorted by name, which picks the repository color.
    pub color_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryRow {
    pub choice: RepositoryChoice,
    pub highlight_positions: Vec<usize>,
    pub score: i64,
    pub matched: bool,
}

#[derive(Clone)]
pub struct RefEntry {
    pub row: RefRow,
    pub menu_handle: PopoverMenuHandle<ContextMenu>,
}

#[derive(Clone)]
pub enum PopupRow {
    Separator,
    Action {
        action: TopAction,
        highlight_positions: Vec<usize>,
        score: i64,
        matched: bool,
    },
    Repository(RepositoryRow),
    Section(SectionRow),
    Folder(FolderRow),
    Ref(RefEntry),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowKey {
    Action(TopActionKind),
    Repository(RepositoryId),
    Section(SectionKind),
    Folder(SectionKind, SharedString),
    Ref(SectionKind, RefTarget),
}

impl PopupRow {
    pub fn is_selectable(&self) -> bool {
        !matches!(self, PopupRow::Separator)
    }

    /// While a query filters the popup, IntelliJ's arrow keys move over leaves only, and over
    /// actions and repositories only when the search results include them
    /// (`GitBranchesTreeUtil.canSelect`).
    pub fn is_selectable_while_filtering(&self) -> bool {
        match self {
            PopupRow::Ref(_) => true,
            PopupRow::Action { matched, .. } => *matched,
            PopupRow::Repository(row) => row.matched,
            PopupRow::Separator | PopupRow::Section(_) | PopupRow::Folder(_) => false,
        }
    }

    pub fn key(&self) -> Option<RowKey> {
        match self {
            PopupRow::Separator => None,
            PopupRow::Action { action, .. } => Some(RowKey::Action(action.kind)),
            PopupRow::Repository(row) => Some(RowKey::Repository(row.choice.id)),
            PopupRow::Section(section) => Some(RowKey::Section(section.kind)),
            PopupRow::Folder(folder) => Some(RowKey::Folder(folder.section, folder.path.clone())),
            PopupRow::Ref(entry) => Some(RowKey::Ref(entry.row.section, entry.row.target.clone())),
        }
    }

    pub fn tree_depth(&self) -> Option<usize> {
        match self {
            PopupRow::Section(_) => Some(0),
            PopupRow::Folder(folder) => Some(folder.depth),
            PopupRow::Ref(entry) => Some(entry.row.depth),
            PopupRow::Separator | PopupRow::Action { .. } | PopupRow::Repository(_) => None,
        }
    }

    pub fn is_tree_container(&self) -> bool {
        matches!(self, PopupRow::Section(_) | PopupRow::Folder(_))
    }

    pub fn node_id(&self) -> Option<SharedString> {
        match self {
            PopupRow::Section(section) => Some(tree::node_id_for_section(section.kind)),
            PopupRow::Folder(folder) => {
                Some(tree::node_id_for_folder(folder.section, &folder.path))
            }
            _ => None,
        }
    }

    pub fn is_expanded(&self) -> Option<bool> {
        match self {
            PopupRow::Section(section) => Some(section.expanded),
            PopupRow::Folder(folder) => Some(folder.expanded),
            _ => None,
        }
    }
}

pub fn is_filtering(query: &str) -> bool {
    let trimmed = query.trim();
    !trimmed.is_empty() && trimmed != CLEAR_FILTER_QUERY
}

pub struct RowsInput<'a> {
    pub query: &'a str,
    pub show_actions_in_search: bool,
    pub show_repositories_in_search: bool,
    pub top_actions: &'a [TopActionItem],
    pub repositories: &'a [RepositoryChoice],
    pub tree_rows: Vec<TreeRow>,
    pub menu_handles: &'a mut HashMap<(SectionKind, RefTarget), PopoverMenuHandle<ContextMenu>>,
}

fn unfiltered_repository_rows(repositories: &[RepositoryChoice], matched: bool) -> Vec<PopupRow> {
    repositories
        .iter()
        .map(|choice| {
            PopupRow::Repository(RepositoryRow {
                choice: choice.clone(),
                highlight_positions: Vec::new(),
                score: 0,
                matched,
            })
        })
        .collect()
}

fn unfiltered_action_rows(top_actions: &[TopActionItem], matched: bool) -> Vec<PopupRow> {
    top_actions
        .iter()
        .map(|item| match item {
            TopActionItem::Action(action) => PopupRow::Action {
                action: action.clone(),
                highlight_positions: Vec::new(),
                score: 0,
                matched,
            },
            TopActionItem::Separator => PopupRow::Separator,
        })
        .collect()
}

pub fn build_popup_rows(input: RowsInput<'_>) -> Vec<PopupRow> {
    let filtering = is_filtering(input.query);
    let multiple_repositories = input.repositories.len() > 1;

    let mut groups: Vec<Vec<PopupRow>> = Vec::new();

    if filtering {
        let matched_actions: Vec<PopupRow> = if input.show_actions_in_search {
            top_actions::matching_actions(input.top_actions, input.query.trim())
                .into_iter()
                .map(|matched| PopupRow::Action {
                    action: matched.action,
                    highlight_positions: matched.highlight_positions,
                    score: matched.score,
                    matched: true,
                })
                .collect()
        } else {
            Vec::new()
        };
        let matched_repositories: Vec<PopupRow> = if multiple_repositories
            && input.show_repositories_in_search
        {
            input
                .repositories
                .iter()
                .filter_map(|choice| {
                    tree::match_popup_query(input.query, &choice.name).map(|(score, positions)| {
                        PopupRow::Repository(RepositoryRow {
                            choice: choice.clone(),
                            highlight_positions: positions,
                            score,
                            matched: true,
                        })
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        // IntelliJ does not filter actions or repositories whose search toggle is off: the whole
        // list stays above the matches and goes away only when nothing else matches.
        let anything_matched = !matched_actions.is_empty()
            || !matched_repositories.is_empty()
            || !input.tree_rows.is_empty();
        if input.show_actions_in_search {
            groups.push(matched_actions);
        } else if anything_matched {
            groups.push(unfiltered_action_rows(input.top_actions, false));
        }
        if input.show_repositories_in_search {
            groups.push(matched_repositories);
        } else if multiple_repositories && anything_matched {
            groups.push(unfiltered_repository_rows(input.repositories, false));
        }
    } else {
        groups.push(unfiltered_action_rows(input.top_actions, true));
        if multiple_repositories {
            groups.push(unfiltered_repository_rows(input.repositories, true));
        }
    }

    let menu_handles = input.menu_handles;
    groups.push(
        input
            .tree_rows
            .into_iter()
            .map(|tree_row| match tree_row {
                TreeRow::Section(section) => PopupRow::Section(section),
                TreeRow::Folder(folder) => PopupRow::Folder(folder),
                TreeRow::Ref(row) => {
                    let menu_handle = menu_handles
                        .entry((row.section, row.target.clone()))
                        .or_default()
                        .clone();
                    PopupRow::Ref(RefEntry { row, menu_handle })
                }
            })
            .collect(),
    );

    let mut rows: Vec<PopupRow> = Vec::new();
    for group in groups.into_iter().filter(|group| !group.is_empty()) {
        if !rows.is_empty() {
            rows.push(PopupRow::Separator);
        }
        rows.extend(group);
    }

    let present: HashSet<(SectionKind, &RefTarget)> = rows
        .iter()
        .filter_map(|row| match row {
            PopupRow::Ref(entry) => Some((entry.row.section, &entry.row.target)),
            _ => None,
        })
        .collect();
    menu_handles.retain(|(section, target), handle| {
        present.contains(&(*section, target)) || handle.is_deployed()
    });
    rows
}

pub fn first_selectable(rows: &[PopupRow]) -> Option<usize> {
    rows.iter().position(PopupRow::is_selectable)
}

const RECENT_MATCH_PRIORITY: u8 = 2;

fn match_rank(row: &PopupRow) -> Option<(u8, i64)> {
    match row {
        PopupRow::Action {
            score,
            matched: true,
            ..
        } => Some((0, *score)),
        PopupRow::Action { matched: false, .. } => None,
        PopupRow::Repository(repository_row) if repository_row.matched => {
            Some((1, repository_row.score))
        }
        PopupRow::Repository(_) => None,
        PopupRow::Ref(entry) => {
            let priority = match entry.row.section {
                SectionKind::Recent => RECENT_MATCH_PRIORITY,
                SectionKind::Local => RECENT_MATCH_PRIORITY + 1,
                SectionKind::Remote => RECENT_MATCH_PRIORITY + 2,
                SectionKind::Tags => RECENT_MATCH_PRIORITY + 3,
            };
            Some((priority, entry.row.score))
        }
        PopupRow::Separator | PopupRow::Section(_) | PopupRow::Folder(_) => None,
    }
}

pub fn best_match(rows: &[PopupRow]) -> Option<usize> {
    let mut best: Option<(usize, u8, i64)> = None;
    for (index, row) in rows.iter().enumerate() {
        let Some((priority, score)) = match_rank(row) else {
            continue;
        };
        let improves = match best {
            None => true,
            Some((_, best_priority, best_score)) => {
                priority < best_priority || (priority == best_priority && score > best_score)
            }
        };
        if improves {
            best = Some((index, priority, score));
        }
    }
    best.map(|(index, _, _)| index)
}

pub fn position_of_key(rows: &[PopupRow], key: &RowKey) -> Option<usize> {
    rows.iter().position(|row| row.key().as_ref() == Some(key))
}

pub fn position_of_local_branch(rows: &[PopupRow], name: &str) -> Option<usize> {
    rows.iter().position(|row| {
        matches!(
            row,
            PopupRow::Ref(entry)
                if entry.row.target.kind == RefKind::Local && &*entry.row.target.name == name
        )
    })
}

// IntelliJ preselects the current branch, never the tag a detached HEAD sits on.
pub fn position_of_current_branch(rows: &[PopupRow]) -> Option<usize> {
    rows.iter().position(|row| {
        matches!(
            row,
            PopupRow::Ref(entry) if entry.row.is_current && entry.row.target.kind != RefKind::Tag
        )
    })
}

pub fn parent_index(rows: &[PopupRow], index: usize) -> Option<usize> {
    let depth = rows.get(index)?.tree_depth()?;
    let parent_depth = depth.checked_sub(1)?;
    rows[..index]
        .iter()
        .rposition(|row| row.is_tree_container() && row.tree_depth() == Some(parent_depth))
}

pub fn first_child_index(rows: &[PopupRow], index: usize) -> Option<usize> {
    let row = rows.get(index)?;
    if !row.is_tree_container() {
        return None;
    }
    let depth = row.tree_depth()?;
    let next = rows.get(index + 1)?;
    (next.tree_depth() == Some(depth + 1)).then_some(index + 1)
}

#[cfg(test)]
pub fn summarize(rows: &[PopupRow]) -> Vec<String> {
    rows.iter()
        .map(|row| match row {
            PopupRow::Separator => "---".to_string(),
            PopupRow::Action { action, .. } => format!("action:{}", action.label),
            PopupRow::Repository(row) => format!("repo:{}", row.choice.name),
            PopupRow::Section(section) => format!(
                "section:{}:{}",
                section.kind.label(),
                if section.expanded { "open" } else { "closed" }
            ),
            PopupRow::Folder(folder) => format!(
                "{}folder:{}:{}",
                "  ".repeat(folder.depth),
                folder.label,
                if folder.expanded { "open" } else { "closed" }
            ),
            PopupRow::Ref(entry) => {
                format!("{}ref:{}", "  ".repeat(entry.row.depth), entry.row.label)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::branches_popup::top_actions::{TopActionsInput, top_actions_plan};
    use crate::branches_popup::tree::TreeInput;
    use collections::HashSet;
    use git::repository::UpstreamTrackingStatus;
    use git::repository::{Branch, RepositoryOperation, Tag, Upstream, UpstreamTracking};

    fn local(name: &str) -> Branch {
        Branch {
            is_head: false,
            ref_name: SharedString::from(format!("refs/heads/{name}")),
            upstream: None,
            most_recent_commit: None,
        }
    }

    fn remote(name: &str) -> Branch {
        Branch {
            is_head: false,
            ref_name: SharedString::from(format!("refs/remotes/{name}")),
            upstream: None,
            most_recent_commit: None,
        }
    }

    fn tracking(name: &str, upstream: &str, ahead: u32, behind: u32) -> Branch {
        Branch {
            upstream: Some(Upstream {
                ref_name: SharedString::from(format!("refs/remotes/{upstream}")),
                tracking: UpstreamTracking::Tracked(UpstreamTrackingStatus { ahead, behind }),
            }),
            ..local(name)
        }
    }

    fn repositories(names: &[&str]) -> Vec<RepositoryChoice> {
        names
            .iter()
            .enumerate()
            .map(|(index, name)| RepositoryChoice {
                id: RepositoryId(index as u64),
                name: SharedString::from(name.to_string()),
                branch: Some(SharedString::from("main")),
                is_active: index == 0,
                color_index: index,
            })
            .collect()
    }

    struct Fixture {
        local: Vec<Branch>,
        remote: Vec<Branch>,
        tags: Vec<Tag>,
        recent: Vec<SharedString>,
        current_branch: Option<&'static str>,
        head_tag: Option<&'static str>,
        expanded: HashSet<SharedString>,
        favorites: HashSet<RefTarget>,
        has_commits: bool,
        operation: Option<RepositoryOperation>,
        repositories: Vec<RepositoryChoice>,
        show_actions_in_search: bool,
        show_repositories_in_search: bool,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                local: vec![
                    tracking("main", "origin/main", 0, 2),
                    local("feature/login"),
                    local("topic"),
                ],
                remote: vec![remote("origin/main"), remote("origin/HEAD")],
                tags: Vec::new(),
                recent: vec![SharedString::from("main"), SharedString::from("topic")],
                current_branch: Some("main"),
                head_tag: None,
                expanded: tree::default_expanded(),
                favorites: HashSet::default(),
                has_commits: true,
                operation: None,
                repositories: repositories(&["only"]),
                show_actions_in_search: true,
                show_repositories_in_search: true,
            }
        }

        fn rows(&self, query: &str) -> Vec<PopupRow> {
            self.rows_with_handles(query, &mut HashMap::default())
        }

        fn rows_with_handles(
            &self,
            query: &str,
            menu_handles: &mut HashMap<(SectionKind, RefTarget), PopoverMenuHandle<ContextMenu>>,
        ) -> Vec<PopupRow> {
            let tree_rows = tree::build_rows(&TreeInput {
                local: &self.local,
                remote: &self.remote,
                tags: &self.tags,
                recent: &self.recent,
                current_branch: self.current_branch,
                head_tag: self.head_tag,
                favorites: &self.favorites,
                options: tree::TreeOptions::default(),
                expanded: &self.expanded,
                expand_all: is_filtering(query),
                query,
            });
            let top_actions = top_actions_plan(&TopActionsInput {
                has_commits: self.has_commits,
                operation: self.operation,
                update_project_shows_options: true,
            });
            build_popup_rows(RowsInput {
                query,
                show_actions_in_search: self.show_actions_in_search,
                show_repositories_in_search: self.show_repositories_in_search,
                top_actions: &top_actions,
                repositories: &self.repositories,
                tree_rows,
                menu_handles,
            })
        }
    }

    #[test]
    fn default_rows_show_actions_then_collapsed_sections_with_recent_expanded() {
        let rows = Fixture::new().rows("");
        assert_eq!(
            summarize(&rows),
            vec![
                "action:Update Project…",
                "action:Commit…",
                "action:Push…",
                "---",
                "action:New Branch…",
                "action:New Worktree…",
                "action:Checkout Tag or Revision…",
                "---",
                "section:Recent:open",
                "  ref:main",
                "  ref:topic",
                "section:Local:closed",
                "section:Remote:closed",
            ]
        );
    }

    #[test]
    fn rows_never_start_end_or_double_up_on_separators() {
        let fixture = Fixture::new();
        for query in ["", "m", "zzz", "new"] {
            let rows = fixture.rows(query);
            assert!(!matches!(rows.first(), Some(PopupRow::Separator)));
            assert!(!matches!(rows.last(), Some(PopupRow::Separator)));
            assert!(rows.windows(2).all(|pair| {
                !(matches!(pair[0], PopupRow::Separator) && matches!(pair[1], PopupRow::Separator))
            }));
        }
    }

    #[test]
    fn ongoing_rebase_rows_sit_between_the_separator_and_new_branch() {
        let mut fixture = Fixture::new();
        fixture.operation = Some(RepositoryOperation::Rebase);
        let summary = summarize(&fixture.rows(""));
        assert_eq!(
            summary[3..10].to_vec(),
            vec![
                "---",
                "action:Abort Rebase",
                "action:Continue Rebase",
                "action:Skip Commit",
                "action:New Branch…",
                "action:New Worktree…",
                "action:Checkout Tag or Revision…",
            ]
        );
    }

    #[test]
    fn query_shows_matching_actions_above_refs_and_forces_sections_open() {
        let rows = Fixture::new().rows("new");
        assert_eq!(
            summarize(&rows),
            vec!["action:New Branch…", "action:New Worktree…"]
        );

        let rows = Fixture::new().rows("top");
        assert_eq!(summarize(&rows), vec!["section:Recent:open", "  ref:topic"]);
    }

    #[test]
    fn query_matching_both_action_and_ref_lists_actions_then_a_separator_then_refs() {
        let rows = Fixture::new().rows("c");
        assert_eq!(
            summarize(&rows),
            vec![
                "action:Update Project…",
                "action:Commit…",
                "action:New Branch…",
                "action:Checkout Tag or Revision…",
                "---",
                "section:Recent:open",
                "  ref:topic",
            ]
        );
        let rows = Fixture::new().rows("push");
        assert_eq!(summarize(&rows), vec!["action:Push…"]);
    }

    #[test]
    fn disabling_actions_in_search_results_keeps_every_action_unfiltered_above_matching_refs() {
        let mut fixture = Fixture::new();
        fixture.show_actions_in_search = false;
        assert!(fixture.rows("new").is_empty());

        let rows = fixture.rows("top");
        assert_eq!(
            summarize(&rows),
            vec![
                "action:Update Project…",
                "action:Commit…",
                "action:Push…",
                "---",
                "action:New Branch…",
                "action:New Worktree…",
                "action:Checkout Tag or Revision…",
                "---",
                "section:Recent:open",
                "  ref:topic",
            ]
        );
        assert!(rows.iter().all(|row| match row {
            PopupRow::Action {
                highlight_positions,
                ..
            } => highlight_positions.is_empty(),
            _ => true,
        }));
        let best = best_match(&rows).expect("the topic branch matches");
        assert!(matches!(
            &rows[best],
            PopupRow::Ref(entry) if entry.row.target == RefTarget::local("topic")
        ));
    }

    #[test]
    fn query_highlights_are_reported_for_action_rows() {
        let rows = Fixture::new().rows("com");
        match &rows[0] {
            PopupRow::Action {
                action,
                highlight_positions,
                ..
            } => {
                assert_eq!(&*action.label, "Commit…");
                assert_eq!(highlight_positions, &vec![0, 1, 2]);
            }
            _ => panic!("expected the Commit action first"),
        }
    }

    #[test]
    fn slash_query_behaves_like_an_empty_query() {
        let fixture = Fixture::new();
        assert_eq!(summarize(&fixture.rows("/")), summarize(&fixture.rows("")));
        assert!(!is_filtering("/"));
        assert!(!is_filtering("   "));
        assert!(is_filtering(" a "));
    }

    #[test]
    fn repository_rows_appear_between_actions_and_tree_only_with_multiple_repositories() {
        let mut fixture = Fixture::new();
        assert!(
            !summarize(&fixture.rows(""))
                .iter()
                .any(|line| line.starts_with("repo:"))
        );
        fixture.repositories = repositories(&["alpha", "beta"]);
        let summary = summarize(&fixture.rows(""));
        let alpha = summary
            .iter()
            .position(|line| line == "repo:alpha")
            .unwrap();
        assert_eq!(summary[alpha + 1], "repo:beta");
        assert_eq!(summary[alpha - 1], "---");
        assert_eq!(summary[alpha + 2], "---");
        assert_eq!(summary[alpha + 3], "section:Recent:open");
    }

    #[test]
    fn repository_rows_are_filtered_by_name_in_query_mode() {
        let mut fixture = Fixture::new();
        fixture.repositories = repositories(&["alpha", "beta"]);
        let summary = summarize(&fixture.rows("alp"));
        assert_eq!(summary, vec!["repo:alpha"]);
    }

    #[test]
    fn repositories_have_their_own_search_toggle_independent_of_actions() {
        let mut fixture = Fixture::new();
        fixture.repositories = repositories(&["alpha", "beta"]);
        fixture.show_actions_in_search = false;
        let summary = summarize(&fixture.rows("alp"));
        assert_eq!(summary.last().map(String::as_str), Some("repo:alpha"));
        assert!(!summary.contains(&"repo:beta".to_string()));

        fixture.show_actions_in_search = true;
        fixture.show_repositories_in_search = false;
        assert!(fixture.rows("alp").is_empty());

        let rows = fixture.rows("top");
        let summary = summarize(&rows);
        assert!(summary.contains(&"repo:alpha".to_string()));
        assert!(summary.contains(&"repo:beta".to_string()));
        assert!(rows.iter().all(|row| match row {
            PopupRow::Repository(row) => !row.matched && row.highlight_positions.is_empty(),
            _ => true,
        }));
    }

    #[test]
    fn while_filtering_only_leaves_and_searched_rows_can_be_selected() {
        let mut fixture = Fixture::new();
        fixture.local.push(local("new"));
        fixture.repositories = repositories(&["new-repo", "beta"]);
        let rows = fixture.rows("new");
        let selectable: Vec<String> = rows
            .iter()
            .filter(|row| row.is_selectable_while_filtering())
            .flat_map(|row| summarize(std::slice::from_ref(row)))
            .collect();
        assert_eq!(
            selectable,
            vec![
                "action:New Branch…",
                "action:New Worktree…",
                "repo:new-repo",
                "  ref:new"
            ]
        );

        fixture.show_actions_in_search = false;
        fixture.show_repositories_in_search = false;
        let rows = fixture.rows("new");
        let selectable: Vec<String> = rows
            .iter()
            .filter(|row| row.is_selectable_while_filtering())
            .flat_map(|row| summarize(std::slice::from_ref(row)))
            .collect();
        assert_eq!(selectable, vec!["  ref:new"]);
    }

    #[test]
    fn a_local_branch_row_is_found_by_name_in_the_first_section_that_lists_it() {
        let mut fixture = Fixture::new();
        fixture
            .expanded
            .insert(tree::node_id_for_section(SectionKind::Local));
        let rows = fixture.rows("");
        let index = position_of_local_branch(&rows, "topic").expect("topic is listed");
        assert!(matches!(
            &rows[index],
            PopupRow::Ref(entry) if entry.row.section == SectionKind::Recent
        ));
        assert_eq!(position_of_local_branch(&rows, "origin/main"), None);
        assert_eq!(position_of_local_branch(&rows, "missing"), None);
    }

    #[test]
    fn empty_result_for_unmatched_query_has_no_rows() {
        assert!(Fixture::new().rows("qqqq").is_empty());
    }

    #[test]
    fn expanded_local_section_lists_the_current_branch_then_folders_then_plain_refs() {
        let mut fixture = Fixture::new();
        fixture
            .expanded
            .insert(tree::node_id_for_section(SectionKind::Local));
        let summary = summarize(&fixture.rows(""));
        let local_section = summary
            .iter()
            .position(|line| line == "section:Local:open")
            .unwrap();
        assert_eq!(
            summary[local_section + 1..].to_vec(),
            vec![
                "  ref:main",
                "  folder:feature:closed",
                "  ref:topic",
                "section:Remote:closed"
            ]
        );
    }

    #[test]
    fn row_keys_distinguish_sections_folders_and_refs_of_equal_names() {
        let mut fixture = Fixture::new();
        fixture
            .expanded
            .insert(tree::node_id_for_section(SectionKind::Local));
        fixture
            .expanded
            .insert(tree::node_id_for_section(SectionKind::Remote));
        fixture
            .expanded
            .insert(tree::node_id_for_folder(SectionKind::Remote, "origin"));
        let rows = fixture.rows("");
        let keys: Vec<RowKey> = rows.iter().filter_map(PopupRow::key).collect();
        for (index, key) in keys.iter().enumerate() {
            assert_eq!(
                keys.iter().position(|other| other == key),
                Some(index),
                "duplicate key {key:?}"
            );
        }
    }

    #[test]
    fn position_of_key_finds_rows_and_misses_removed_ones() {
        let rows = Fixture::new().rows("");
        let key = RowKey::Ref(SectionKind::Recent, RefTarget::local("topic"));
        assert!(position_of_key(&rows, &key).is_some());
        let missing = RowKey::Ref(SectionKind::Local, RefTarget::local("topic"));
        assert_eq!(position_of_key(&rows, &missing), None);
    }

    #[test]
    fn first_selectable_skips_leading_separators_only() {
        assert_eq!(first_selectable(&[]), None);
        let rows = vec![PopupRow::Separator];
        assert_eq!(first_selectable(&rows), None);
        assert_eq!(first_selectable(&Fixture::new().rows("")), Some(0));
    }

    #[test]
    fn current_branch_row_is_found_for_preselection() {
        let rows = Fixture::new().rows("");
        let index = position_of_current_branch(&rows).expect("main is current");
        match &rows[index] {
            PopupRow::Ref(entry) => assert_eq!(&*entry.row.target.name, "main"),
            _ => panic!("expected a ref row"),
        }
    }

    #[test]
    fn the_tag_of_a_detached_head_is_marked_but_never_preselected() {
        let mut fixture = Fixture::new();
        fixture.recent.clear();
        fixture.current_branch = None;
        fixture.head_tag = Some("v1.0");
        fixture.tags = vec![Tag {
            name: SharedString::new_static("v1.0"),
            commit_sha: SharedString::new_static("0123abcd"),
        }];
        fixture
            .expanded
            .insert(tree::node_id_for_section(SectionKind::Tags));
        let rows = fixture.rows("");
        assert!(rows.iter().any(|row| matches!(
            row,
            PopupRow::Ref(entry) if entry.row.target == RefTarget::tag("v1.0") && entry.row.is_current
        )));
        assert_eq!(position_of_current_branch(&rows), None);
    }

    #[test]
    fn parent_and_child_navigation_follows_tree_depth() {
        let mut fixture = Fixture::new();
        fixture
            .expanded
            .insert(tree::node_id_for_section(SectionKind::Local));
        fixture
            .expanded
            .insert(tree::node_id_for_folder(SectionKind::Local, "feature"));
        let rows = fixture.rows("");
        let summary = summarize(&rows);
        let section = summary
            .iter()
            .position(|line| line == "section:Local:open")
            .unwrap();
        let current = section + 1;
        let folder = current + 1;
        let leaf = folder + 1;
        assert_eq!(summary[current], "  ref:main");
        assert_eq!(summary[folder], "  folder:feature:open");
        assert_eq!(summary[leaf], "    ref:login");

        assert_eq!(parent_index(&rows, leaf), Some(folder));
        assert_eq!(parent_index(&rows, folder), Some(section));
        assert_eq!(parent_index(&rows, current), Some(section));
        assert_eq!(parent_index(&rows, section), None);
        assert_eq!(first_child_index(&rows, section), Some(current));
        assert_eq!(first_child_index(&rows, folder), Some(leaf));
        assert_eq!(first_child_index(&rows, leaf), None);
    }

    #[test]
    fn collapsed_container_has_no_first_child() {
        let rows = Fixture::new().rows("");
        let summary = summarize(&rows);
        let closed = summary
            .iter()
            .position(|line| line == "section:Local:closed")
            .unwrap();
        assert_eq!(first_child_index(&rows, closed), None);
    }

    #[test]
    fn recent_refs_have_the_recent_section_as_parent() {
        let rows = Fixture::new().rows("");
        let summary = summarize(&rows);
        let recent = summary
            .iter()
            .position(|line| line == "section:Recent:open")
            .unwrap();
        assert_eq!(parent_index(&rows, recent + 1), Some(recent));
        assert_eq!(parent_index(&rows, recent + 2), Some(recent));
    }

    #[test]
    fn actions_and_separators_have_no_tree_parent() {
        let rows = Fixture::new().rows("");
        assert_eq!(parent_index(&rows, 0), None);
        assert_eq!(parent_index(&rows, 3), None);
        assert_eq!(parent_index(&rows, 999), None);
    }

    #[test]
    fn ref_rows_report_incoming_commits_from_the_tree_model() {
        let rows = Fixture::new().rows("");
        let main = rows
            .iter()
            .find_map(|row| match row {
                PopupRow::Ref(entry) if &*entry.row.target.name == "main" => {
                    Some(entry.row.clone())
                }
                _ => None,
            })
            .expect("main row");
        assert_eq!(main.incoming, 2);
        assert_eq!(main.outgoing, 0);
        assert_eq!(main.tracked.as_deref(), Some("origin/main"));
    }

    #[test]
    fn best_match_skips_section_headers_but_not_actions_or_refs() {
        let fixture = Fixture::new();
        assert_eq!(best_match(&fixture.rows("")), Some(0));
        let rows = fixture.rows("top");
        assert_eq!(first_selectable(&rows), Some(0));
        assert_eq!(best_match(&rows), Some(1));
        assert_eq!(best_match(&fixture.rows("qqqq")), None);
    }

    #[test]
    fn best_match_prefers_the_higher_scoring_ref_over_display_order() {
        let mut fixture = Fixture::new();
        fixture.recent.clear();
        fixture.remote.clear();
        fixture.local = vec![local("a-main"), local("main2"), local("topic")];
        let rows = fixture.rows("main");
        assert_eq!(
            summarize(&rows),
            vec!["section:Local:open", "  ref:a-main", "  ref:main2"]
        );
        assert_eq!(best_match(&rows), Some(2));
    }

    #[test]
    fn best_match_prefers_a_recent_ref_over_a_better_scoring_local_ref() {
        let mut fixture = Fixture::new();
        fixture.remote.clear();
        fixture.local.push(local("zz-main"));
        fixture.recent = vec![SharedString::from("zz-main")];
        let rows = fixture.rows("main");
        assert_eq!(
            summarize(&rows),
            vec![
                "section:Recent:open",
                "  ref:zz-main",
                "section:Local:open",
                "  ref:main"
            ]
        );
        assert_eq!(best_match(&rows), Some(1));
    }

    #[test]
    fn best_match_prefers_an_action_over_any_ref() {
        let mut fixture = Fixture::new();
        fixture.local.push(local("new"));
        let rows = fixture.rows("new");
        assert_eq!(
            summarize(&rows),
            vec![
                "action:New Branch…",
                "action:New Worktree…",
                "---",
                "section:Local:open",
                "  ref:new"
            ]
        );
        assert_eq!(best_match(&rows), Some(0));
    }

    #[test]
    fn best_match_falls_back_from_local_to_remote_to_tags() {
        let mut fixture = Fixture::new();
        fixture.recent.clear();
        fixture.local = vec![local("topic")];
        fixture.remote = vec![remote("origin/release")];
        let rows = fixture.rows("release");
        let index = best_match(&rows).expect("the remote branch matches");
        assert!(matches!(
            &rows[index],
            PopupRow::Ref(entry) if entry.row.target == RefTarget::remote("origin/release")
        ));
    }

    #[test]
    fn menu_handles_are_kept_for_surviving_refs_and_pruned_for_removed_ones() {
        let fixture = Fixture::new();
        let mut menu_handles = HashMap::default();
        fixture.rows_with_handles("", &mut menu_handles);
        assert_eq!(menu_handles.len(), 2);
        fixture.rows_with_handles("top", &mut menu_handles);
        assert_eq!(
            menu_handles.keys().cloned().collect::<Vec<_>>(),
            vec![(SectionKind::Recent, RefTarget::local("topic"))]
        );
    }

    #[test]
    fn a_branch_shown_in_recent_and_local_gets_a_separate_menu_handle_per_row() {
        let mut fixture = Fixture::new();
        fixture
            .expanded
            .insert(tree::node_id_for_section(SectionKind::Local));
        let mut menu_handles = HashMap::default();
        fixture.rows_with_handles("", &mut menu_handles);
        let main = RefTarget::local("main");
        assert!(menu_handles.contains_key(&(SectionKind::Recent, main.clone())));
        assert!(menu_handles.contains_key(&(SectionKind::Local, main)));
    }
}
