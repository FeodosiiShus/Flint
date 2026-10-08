use std::collections::{BTreeMap, HashSet};

use git::repository::{RepoPath, UnmergedStagePresence};
use merge_diff::Side;

use super::messages;
use super::ordering::{natural_compare, sorted_files, stable_sort_by};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ModelSummary {
    pub(crate) total: usize,
    pub(crate) resolved: usize,
    pub(crate) unresolved: usize,
    pub(crate) auto_resolvable: usize,
    pub(crate) reviewed: bool,
    pub(crate) chosen_side: Option<Side>,
}

impl ModelSummary {
    pub(crate) fn is_fully_resolved(&self) -> bool {
        self.unresolved == 0
    }

    pub(crate) fn is_partially_resolved(&self) -> bool {
        self.resolved > 0 && self.unresolved > 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum GroupKind {
    Unresolved,
    Resolved,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RowId {
    Group(GroupKind),
    Directory(GroupKind, String),
    File(RepoPath),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TreeRowKind {
    Group { group: GroupKind, file_count: usize },
    Directory { label: String, file_count: usize },
    File { path: RepoPath },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TreeRow {
    pub(crate) id: RowId,
    pub(crate) depth: usize,
    pub(crate) kind: TreeRowKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TreeStateStrategy {
    Default,
    GroupingChange,
    ModelChange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BadgeView {
    pub(crate) value: String,
    pub(crate) modified: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RowViewKind {
    Group {
        group: GroupKind,
        title: &'static str,
        count_text: String,
    },
    Directory {
        label: String,
        count_text: String,
    },
    File {
        path: RepoPath,
        file_name: String,
        parent_path: Option<String>,
        badge: Option<BadgeView>,
        tooltip: Option<String>,
        yours_status: &'static str,
        theirs_status: &'static str,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RowView {
    pub(crate) id: RowId,
    pub(crate) depth: usize,
    pub(crate) expanded: Option<bool>,
    pub(crate) selected: bool,
    pub(crate) kind: RowViewKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DerivedState {
    pub(crate) selected_files: Vec<RepoPath>,
    pub(crate) resolved_files_selected: bool,
    pub(crate) all_selected_files_resolved: bool,
    pub(crate) only_revertable_files_selected: bool,
    pub(crate) all_files_resolved_and_reviewed: bool,
    pub(crate) files_that_should_be_resolved_iteratively: Vec<RepoPath>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DefaultButton {
    ReviewOrResolve,
    AcceptAndFinish,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectionMove {
    Previous,
    Next,
    First,
    Last,
    PagePrevious(usize),
    PageNext(usize),
}

pub(crate) struct DialogState {
    reversed: bool,
    presence: BTreeMap<RepoPath, UnmergedStagePresence>,
    files: Vec<RepoPath>,
    unresolved_files: Vec<RepoPath>,
    processed_files: Vec<RepoPath>,
    models: BTreeMap<RepoPath, ModelSummary>,
    model_order: Vec<RepoPath>,
    group_by_directory: bool,
    resolve_pressed_once: bool,
    resolve_pressed: bool,
    resolving: bool,
    rows: Vec<TreeRow>,
    collapsed: HashSet<RowId>,
    selected: Vec<RowId>,
    anchor: Option<RowId>,
    lead: Option<RowId>,
}

#[derive(Default)]
struct DirectoryNode {
    directories: BTreeMap<String, DirectoryNode>,
    files: Vec<RepoPath>,
}

impl DirectoryNode {
    fn insert(&mut self, components: &[&str], file: RepoPath) {
        match components.split_first() {
            Some((directory, rest)) if !rest.is_empty() => self
                .directories
                .entry((*directory).to_string())
                .or_default()
                .insert(rest, file),
            _ => self.files.push(file),
        }
    }

    fn file_count(&self) -> usize {
        self.files.len()
            + self
                .directories
                .values()
                .map(DirectoryNode::file_count)
                .sum::<usize>()
    }
}

fn path_components(path: &RepoPath) -> Vec<&str> {
    path.as_unix_str()
        .split('/')
        .filter(|part| !part.is_empty())
        .collect()
}

fn collapse(mut node: &DirectoryNode, mut label: String) -> (&DirectoryNode, String) {
    while node.files.is_empty() && node.directories.len() == 1 {
        let Some((name, child)) = node.directories.iter().next() else {
            break;
        };
        if !label.is_empty() {
            label.push('/');
        }
        label.push_str(name);
        node = child;
    }
    (node, label)
}

fn emit_directory(
    node: &DirectoryNode,
    group: GroupKind,
    parent_path: &str,
    depth: usize,
    rows: &mut Vec<TreeRow>,
) {
    let mut directories: Vec<(&String, &DirectoryNode)> = node.directories.iter().collect();
    stable_sort_by(&mut directories, &mut |left, right| {
        natural_compare(left.0, right.0)
    });
    for (name, child) in directories {
        let (collapsed, label) = collapse(child, name.clone());
        let full_path = if parent_path.is_empty() {
            label.clone()
        } else {
            format!("{parent_path}/{label}")
        };
        rows.push(TreeRow {
            id: RowId::Directory(group, full_path.clone()),
            depth,
            kind: TreeRowKind::Directory {
                label,
                file_count: collapsed.file_count(),
            },
        });
        emit_directory(collapsed, group, &full_path, depth + 1, rows);
    }
    push_file_rows(&node.files, depth, rows);
}

fn push_file_rows(files: &[RepoPath], depth: usize, rows: &mut Vec<TreeRow>) {
    let mut sorted = files.to_vec();
    stable_sort_by(&mut sorted, &mut |left, right| {
        natural_compare(
            left.file_name().unwrap_or_default(),
            right.file_name().unwrap_or_default(),
        )
    });
    for file in sorted {
        rows.push(TreeRow {
            id: RowId::File(file.clone()),
            depth,
            kind: TreeRowKind::File { path: file },
        });
    }
}

fn group_rows(
    group: GroupKind,
    files: &[RepoPath],
    group_by_directory: bool,
    rows: &mut Vec<TreeRow>,
) {
    rows.push(TreeRow {
        id: RowId::Group(group),
        depth: 0,
        kind: TreeRowKind::Group {
            group,
            file_count: files.len(),
        },
    });
    if files.is_empty() {
        return;
    }
    if group_by_directory {
        let mut root = DirectoryNode::default();
        for file in files {
            let components = path_components(file);
            root.insert(&components, file.clone());
        }
        let (top, label) = collapse(&root, String::new());
        rows.push(TreeRow {
            id: RowId::Directory(group, label.clone()),
            depth: 1,
            kind: TreeRowKind::Directory {
                label: label.clone(),
                file_count: top.file_count(),
            },
        });
        emit_directory(top, group, &label, 2, rows);
    } else {
        push_file_rows(files, 1, rows);
    }
}

impl DialogState {
    pub(crate) fn new(
        files: Vec<RepoPath>,
        presence: BTreeMap<RepoPath, UnmergedStagePresence>,
        reversed: bool,
        group_by_directory: bool,
    ) -> Self {
        let mut state = Self {
            reversed,
            presence,
            unresolved_files: files.clone(),
            files,
            processed_files: Vec::new(),
            models: BTreeMap::new(),
            model_order: Vec::new(),
            group_by_directory,
            resolve_pressed_once: false,
            resolve_pressed: false,
            resolving: false,
            rows: Vec::new(),
            collapsed: HashSet::new(),
            selected: Vec::new(),
            anchor: None,
            lead: None,
        };
        state.rebuild(TreeStateStrategy::Default);
        state
    }

    pub(crate) fn files(&self) -> &[RepoPath] {
        &self.files
    }

    pub(crate) fn processed_files(&self) -> &[RepoPath] {
        &self.processed_files
    }

    pub(crate) fn is_reversed(&self) -> bool {
        self.reversed
    }

    pub(crate) fn group_by_directory(&self) -> bool {
        self.group_by_directory
    }

    pub(crate) fn set_group_by_directory(&mut self, value: bool) {
        self.group_by_directory = value;
    }

    pub(crate) fn presence_of(&self, path: &RepoPath) -> UnmergedStagePresence {
        self.presence.get(path).copied().unwrap_or_default()
    }

    pub(crate) fn conflicts_for(
        &self,
        files: &[RepoPath],
    ) -> Vec<(RepoPath, UnmergedStagePresence)> {
        files
            .iter()
            .map(|file| (file.clone(), self.presence_of(file)))
            .collect()
    }

    pub(crate) fn set_model_summary(&mut self, path: RepoPath, summary: ModelSummary) {
        if self.models.insert(path.clone(), summary).is_none() {
            self.model_order.push(path);
        }
    }

    pub(crate) fn remove_model_summaries(&mut self, paths: &[RepoPath]) {
        for path in paths {
            self.models.remove(path);
        }
        self.model_order.retain(|path| !paths.contains(path));
    }

    pub(crate) fn is_file_resolved(&self, path: &RepoPath) -> bool {
        self.models
            .get(path)
            .is_some_and(ModelSummary::is_fully_resolved)
    }

    pub(crate) fn is_file_reviewed(&self, path: &RepoPath) -> bool {
        self.models
            .get(path)
            .is_some_and(|summary| summary.reviewed)
    }

    pub(crate) fn resolved_files_and_models(&self) -> Vec<(RepoPath, ModelSummary)> {
        self.model_order
            .iter()
            .filter_map(|path| {
                let summary = self.models.get(path)?;
                summary
                    .is_fully_resolved()
                    .then(|| (path.clone(), *summary))
            })
            .collect()
    }

    pub(crate) fn resolved_file_paths(&self) -> Vec<RepoPath> {
        self.resolved_files_and_models()
            .into_iter()
            .map(|(path, _)| path)
            .collect()
    }

    pub(crate) fn unresolved_group_files(&self) -> Vec<RepoPath> {
        self.unresolved_files
            .iter()
            .filter(|file| !self.is_file_resolved(file))
            .cloned()
            .collect()
    }

    pub(crate) fn mark_processed(&mut self, files: &[RepoPath]) {
        self.unresolved_files.retain(|file| !files.contains(file));
        for file in files {
            if !self.processed_files.contains(file) {
                self.processed_files.push(file.clone());
            }
        }
    }

    pub(crate) fn has_nothing_left_to_process(&self) -> bool {
        self.unresolved_files.is_empty()
    }

    pub(crate) fn is_resolving(&self) -> bool {
        self.resolving
    }

    pub(crate) fn begin_resolve_all(&mut self) {
        self.resolve_pressed_once = true;
        self.resolve_pressed = true;
        self.resolving = true;
    }

    pub(crate) fn finish_resolve_all(&mut self) {
        self.resolving = false;
    }

    pub(crate) fn cancel_resolve_all(&mut self) {
        self.resolve_pressed = false;
        self.resolving = false;
    }

    pub(crate) fn reset_resolve_all_pressed(&mut self) {
        self.resolve_pressed = false;
    }

    pub(crate) fn resolve_all_enabled(&self) -> bool {
        let auto_resolvable_files = self.files.iter().any(|file| {
            self.models
                .get(file)
                .is_some_and(|summary| summary.auto_resolvable > 0)
        });
        let all_files_resolved = self.files.iter().all(|file| {
            self.models
                .get(file)
                .is_some_and(|summary| summary.unresolved == 0)
        });
        if auto_resolvable_files {
            true
        } else if all_files_resolved {
            false
        } else if !self.resolve_pressed_once {
            true
        } else if self.resolve_pressed {
            false
        } else {
            !self.resolving
        }
    }

    pub(crate) fn total_resolved_changes(&self) -> usize {
        self.files
            .iter()
            .filter_map(|file| self.models.get(file))
            .map(|summary| summary.resolved)
            .sum()
    }

    pub(crate) fn total_unresolved_changes(&self) -> usize {
        self.files
            .iter()
            .filter_map(|file| self.models.get(file))
            .map(|summary| summary.unresolved)
            .sum()
    }

    pub(crate) fn files_with_unresolved_changes(&self) -> usize {
        self.files
            .iter()
            .filter_map(|file| self.models.get(file))
            .filter(|summary| summary.unresolved > 0)
            .count()
    }

    pub(crate) fn has_partially_resolved_files(&self) -> bool {
        self.files.iter().any(|file| {
            self.models
                .get(file)
                .is_some_and(ModelSummary::is_partially_resolved)
        })
    }

    pub(crate) fn rows(&self) -> &[TreeRow] {
        &self.rows
    }

    pub(crate) fn visible_indices(&self) -> Vec<usize> {
        let mut visible = Vec::new();
        let mut hidden_below_depth: Option<usize> = None;
        for (index, row) in self.rows.iter().enumerate() {
            if let Some(depth) = hidden_below_depth {
                if row.depth > depth {
                    continue;
                }
                hidden_below_depth = None;
            }
            visible.push(index);
            if self.collapsed.contains(&row.id) {
                hidden_below_depth = Some(row.depth);
            }
        }
        visible
    }

    #[cfg(test)]
    pub(crate) fn visible_rows(&self) -> Vec<&TreeRow> {
        self.visible_indices()
            .into_iter()
            .map(|index| &self.rows[index])
            .collect()
    }

    fn row_index(&self, id: &RowId) -> Option<usize> {
        self.rows.iter().position(|row| &row.id == id)
    }

    fn files_at_or_below(&self, index: usize) -> impl Iterator<Item = &RepoPath> + '_ {
        let depth = self.rows[index].depth;
        let below = self.rows[index + 1..]
            .iter()
            .take_while(move |row| row.depth > depth);
        std::iter::once(&self.rows[index])
            .chain(below)
            .filter_map(|row| match &row.kind {
                TreeRowKind::File { path } => Some(path),
                _ => None,
            })
    }

    #[cfg(test)]
    pub(crate) fn selected_ids(&self) -> &[RowId] {
        &self.selected
    }

    pub(crate) fn lead_id(&self) -> Option<&RowId> {
        self.lead.as_ref().or(self.selected.last())
    }

    pub(crate) fn is_selected(&self, id: &RowId) -> bool {
        self.selected.contains(id)
    }

    pub(crate) fn selected_files(&self) -> Vec<RepoPath> {
        let selected: HashSet<&RowId> = self.selected.iter().collect();
        let mut seen: HashSet<&RepoPath> = HashSet::new();
        let mut files = Vec::new();
        for (index, row) in self.rows.iter().enumerate() {
            if !selected.contains(&row.id) {
                continue;
            }
            for file in self.files_at_or_below(index) {
                if seen.insert(file) {
                    files.push(file.clone());
                }
            }
        }
        files
    }

    pub(crate) fn select_only(&mut self, id: RowId) {
        self.anchor = Some(id.clone());
        self.lead = Some(id.clone());
        self.selected = vec![id];
    }

    pub(crate) fn toggle_selection(&mut self, id: RowId) {
        if let Some(position) = self.selected.iter().position(|selected| selected == &id) {
            self.selected.remove(position);
        } else {
            self.selected.push(id.clone());
        }
        self.anchor = Some(id.clone());
        self.lead = Some(id);
    }

    pub(crate) fn extend_selection_to(&mut self, id: RowId) {
        let visible = self.visible_indices();
        let anchor = self
            .anchor
            .clone()
            .or_else(|| self.lead.clone())
            .unwrap_or_else(|| id.clone());
        let anchor_position = visible
            .iter()
            .position(|index| self.rows[*index].id == anchor);
        let target_position = visible.iter().position(|index| self.rows[*index].id == id);
        let (Some(anchor_position), Some(target_position)) = (anchor_position, target_position)
        else {
            self.select_only(id);
            return;
        };
        let (start, end) = if anchor_position <= target_position {
            (anchor_position, target_position)
        } else {
            (target_position, anchor_position)
        };
        self.selected = visible[start..=end]
            .iter()
            .map(|index| self.rows[*index].id.clone())
            .collect();
        self.anchor = Some(anchor);
        self.lead = Some(id);
    }

    pub(crate) fn select_all(&mut self) {
        let visible = self.visible_indices();
        self.selected = visible
            .iter()
            .map(|index| self.rows[*index].id.clone())
            .collect();
        self.anchor = self.selected.first().cloned();
        self.lead = self.selected.last().cloned();
    }

    pub(crate) fn move_selection(&mut self, movement: SelectionMove, extend: bool) {
        let visible = self.visible_indices();
        if visible.is_empty() {
            return;
        }
        let focus_position = self
            .lead_id()
            .and_then(|id| visible.iter().position(|index| &self.rows[*index].id == id));
        let target_position = match (movement, focus_position) {
            (SelectionMove::First, _) => 0,
            (SelectionMove::Last, _) => visible.len() - 1,
            (SelectionMove::Next, None) => 0,
            (SelectionMove::Previous, None) => visible.len() - 1,
            (SelectionMove::Next, Some(position)) => (position + 1).min(visible.len() - 1),
            (SelectionMove::Previous, Some(position)) => position.saturating_sub(1),
            (SelectionMove::PageNext(rows), position) => {
                (position.unwrap_or(0) + rows).min(visible.len() - 1)
            }
            (SelectionMove::PagePrevious(rows), position) => {
                position.unwrap_or(visible.len() - 1).saturating_sub(rows)
            }
        };
        let target = self.rows[visible[target_position]].id.clone();
        if extend {
            self.extend_selection_to(target);
        } else {
            self.select_only(target);
        }
    }

    pub(crate) fn select_file_matching(&mut self, query: &str) -> bool {
        let needle = query.to_lowercase();
        let visible = self.visible_indices();
        if visible.is_empty() || needle.is_empty() {
            return false;
        }
        let start = self
            .lead_id()
            .and_then(|id| visible.iter().position(|index| &self.rows[*index].id == id))
            .unwrap_or(0);
        let matching = (0..visible.len())
            .map(|offset| visible[(start + offset) % visible.len()])
            .find(|index| match &self.rows[*index].kind {
                TreeRowKind::File { path } => path
                    .file_name()
                    .is_some_and(|name| name.to_lowercase().contains(&needle)),
                _ => false,
            });
        match matching {
            Some(index) => {
                let id = self.rows[index].id.clone();
                self.select_only(id);
                true
            }
            None => false,
        }
    }

    pub(crate) fn set_expanded(&mut self, id: &RowId, expanded: bool) {
        let Some(index) = self.row_index(id) else {
            return;
        };
        if matches!(self.rows[index].kind, TreeRowKind::File { .. }) {
            return;
        }
        if expanded {
            self.collapsed.remove(id);
            return;
        }
        self.collapsed.insert(id.clone());
        self.fold_selection_into(index);
    }

    fn fold_selection_into(&mut self, index: usize) {
        let depth = self.rows[index].depth;
        let collapsed_id = self.rows[index].id.clone();
        let hidden: HashSet<&RowId> = self.rows[index + 1..]
            .iter()
            .take_while(|row| row.depth > depth)
            .map(|row| &row.id)
            .collect();
        let selected_before = self.selected.len();
        self.selected.retain(|selected| !hidden.contains(selected));
        let removed_selection = self.selected.len() != selected_before;
        let anchor_hidden = self.anchor.as_ref().is_some_and(|id| hidden.contains(id));
        let lead_hidden = self.lead.as_ref().is_some_and(|id| hidden.contains(id));
        if removed_selection && !self.selected.contains(&collapsed_id) {
            self.selected.push(collapsed_id.clone());
        }
        if anchor_hidden {
            self.anchor = Some(collapsed_id.clone());
        }
        if lead_hidden {
            self.lead = Some(collapsed_id);
        }
    }

    pub(crate) fn is_expanded(&self, id: &RowId) -> bool {
        !self.collapsed.contains(id)
    }

    pub(crate) fn collapse_or_expand_single_selection(&mut self, expand: bool) {
        let [only] = self.selected.as_slice() else {
            return;
        };
        let id = only.clone();
        let Some(index) = self.row_index(&id) else {
            return;
        };
        let is_expandable = !matches!(self.rows[index].kind, TreeRowKind::File { .. });
        let currently_expanded = self.is_expanded(&id);
        if is_expandable && currently_expanded != expand {
            self.set_expanded(&id, expand);
            return;
        }
        if expand {
            if is_expandable && currently_expanded {
                let next = self.rows.get(index + 1);
                if let Some(child) = next.filter(|child| child.depth > self.rows[index].depth) {
                    let child_id = child.id.clone();
                    self.select_only(child_id);
                }
            }
            return;
        }
        let depth = self.rows[index].depth;
        if depth == 0 {
            return;
        }
        let parent = self.rows[..index]
            .iter()
            .rev()
            .find(|row| row.depth < depth)
            .map(|row| row.id.clone());
        if let Some(parent_id) = parent {
            self.select_only(parent_id);
        }
    }

    pub(crate) fn rebuild(&mut self, strategy: TreeStateStrategy) {
        let previous_selected_files = self.selected_files();
        let previous_first_visible_index = {
            let visible = self.visible_indices();
            visible.iter().position(|index| {
                self.selected
                    .iter()
                    .any(|selected| selected == &self.rows[*index].id)
            })
        };

        let unresolved = self.unresolved_group_files();
        let resolved = self.resolved_file_paths();
        let mut rows = Vec::new();
        if !unresolved.is_empty() {
            group_rows(
                GroupKind::Unresolved,
                &unresolved,
                self.group_by_directory,
                &mut rows,
            );
        }
        if !resolved.is_empty() || self.resolve_pressed_once {
            group_rows(
                GroupKind::Resolved,
                &resolved,
                self.group_by_directory,
                &mut rows,
            );
        }
        self.rows = rows;
        self.collapsed.clear();

        let existing: HashSet<RowId> = self.rows.iter().map(|row| row.id.clone()).collect();
        match strategy {
            TreeStateStrategy::Default => {
                self.selected.clear();
                match self.rows.first().map(|first| first.id.clone()) {
                    Some(id) => self.select_only(id),
                    None => self.clear_cursor(),
                }
            }
            TreeStateStrategy::GroupingChange => {
                self.selected = previous_selected_files
                    .into_iter()
                    .map(RowId::File)
                    .filter(|id| existing.contains(id))
                    .collect();
                if self.selected.is_empty() {
                    let first_leaf = self
                        .rows
                        .iter()
                        .find(|row| matches!(row.kind, TreeRowKind::File { .. }))
                        .map(|row| row.id.clone());
                    match first_leaf {
                        Some(id) => self.select_only(id),
                        None => self.clear_cursor(),
                    }
                } else {
                    self.anchor = self.selected.first().cloned();
                    self.lead = self.selected.last().cloned();
                }
            }
            TreeStateStrategy::ModelChange => {
                self.selected.retain(|id| existing.contains(id));
                if self.selected.is_empty() {
                    let visible = self.visible_indices();
                    if visible.is_empty() {
                        self.clear_cursor();
                    } else {
                        let position = previous_first_visible_index
                            .unwrap_or(0)
                            .min(visible.len() - 1);
                        let id = self.rows[visible[position]].id.clone();
                        self.select_only(id);
                    }
                } else {
                    if self
                        .anchor
                        .as_ref()
                        .is_none_or(|anchor| !existing.contains(anchor))
                    {
                        self.anchor = self.selected.first().cloned();
                    }
                    if self
                        .lead
                        .as_ref()
                        .is_none_or(|lead| !existing.contains(lead))
                    {
                        self.lead = self.selected.last().cloned();
                    }
                }
            }
        }
    }

    fn clear_cursor(&mut self) {
        self.anchor = None;
        self.lead = None;
    }

    pub(crate) fn derived(&self) -> DerivedState {
        let selected_files = self.selected_files();
        let resolved_files_selected = selected_files
            .iter()
            .any(|file| self.is_file_resolved(file));
        let all_selected_files_resolved = selected_files
            .iter()
            .all(|file| self.is_file_resolved(file));
        let only_revertable_files_selected = !selected_files.is_empty()
            && selected_files.iter().all(|file| {
                self.models
                    .get(file)
                    .is_some_and(|summary| summary.resolved > 0)
            });
        DerivedState {
            selected_files,
            resolved_files_selected,
            all_selected_files_resolved,
            only_revertable_files_selected,
            all_files_resolved_and_reviewed: self.all_files_resolved_and_reviewed(),
            files_that_should_be_resolved_iteratively: self.pending_files(),
        }
    }

    fn pending_files(&self) -> Vec<RepoPath> {
        let processed: HashSet<&RepoPath> = self.processed_files.iter().collect();
        self.files
            .iter()
            .filter(|file| !processed.contains(file))
            .cloned()
            .collect()
    }

    fn all_files_resolved_and_reviewed(&self) -> bool {
        self.pending_files().iter().all(|file| {
            self.models
                .get(file)
                .is_some_and(|summary| summary.reviewed && summary.unresolved == 0)
        })
    }

    pub(crate) fn accept_and_finish_enabled(&self) -> bool {
        !self.resolving
            && self
                .pending_files()
                .iter()
                .all(|file| self.is_file_resolved(file))
    }

    pub(crate) fn review_button_label(&self) -> &'static str {
        if self
            .selected_files()
            .iter()
            .all(|file| self.is_file_resolved(file))
        {
            messages::REVIEW_CHANGES
        } else {
            messages::RESOLVE_MANUALLY
        }
    }

    pub(crate) fn review_button_visible(&self) -> bool {
        !self.all_files_resolved_and_reviewed()
    }

    pub(crate) fn default_button(&self) -> DefaultButton {
        if self.all_files_resolved_and_reviewed() {
            DefaultButton::AcceptAndFinish
        } else {
            DefaultButton::ReviewOrResolve
        }
    }

    pub(crate) fn files_to_open(&self, suggestion_enabled: bool) -> Vec<RepoPath> {
        let selected = self.selected_files();
        if !suggestion_enabled {
            return selected;
        }
        let flattened = !self.group_by_directory;
        let resolved_files = self.resolved_file_paths();
        let unresolved: Vec<RepoPath> = self
            .unresolved_files
            .iter()
            .filter(|file| !resolved_files.contains(file))
            .cloned()
            .collect();
        let resolved_unreviewed: Vec<RepoPath> = resolved_files
            .iter()
            .filter(|file| !self.is_file_reviewed(file))
            .cloned()
            .collect();
        let mut seen = HashSet::new();
        let mut ordered = Vec::new();
        let candidates = sorted_files(&selected, flattened)
            .into_iter()
            .chain(sorted_files(&unresolved, flattened))
            .chain(sorted_files(&resolved_unreviewed, flattened));
        for file in candidates {
            if seen.insert(file.clone()) {
                ordered.push(file);
            }
        }
        ordered
    }

    pub(crate) fn accept_button_visible(&self, id: &RowId, hovered_row: Option<&RowId>) -> bool {
        let RowId::File(path) = id else {
            return false;
        };
        if self.is_file_resolved(path) {
            return false;
        }
        match hovered_row {
            Some(hovered) => hovered == id,
            None => {
                let selected_files = self.selected_files();
                selected_files.len() == 1 && selected_files[0] == *path
            }
        }
    }

    fn column_presence(&self, path: &RepoPath) -> (bool, bool) {
        let presence = self.presence_of(path);
        if self.reversed {
            (presence.theirs, presence.ours)
        } else {
            (presence.ours, presence.theirs)
        }
    }

    pub(crate) fn row_views(&self) -> Vec<RowView> {
        self.visible_indices()
            .into_iter()
            .map(|index| self.row_view(&self.rows[index]))
            .collect()
    }

    fn row_view(&self, row: &TreeRow) -> RowView {
        let expanded = match &row.kind {
            TreeRowKind::File { .. } => None,
            _ => Some(self.is_expanded(&row.id)),
        };
        let kind = match &row.kind {
            TreeRowKind::Group { group, file_count } => RowViewKind::Group {
                group: *group,
                title: match group {
                    GroupKind::Unresolved => messages::UNRESOLVED_GROUP,
                    GroupKind::Resolved => messages::RESOLVED_GROUP,
                },
                count_text: messages::files_count(*file_count),
            },
            TreeRowKind::Directory { label, file_count } => RowViewKind::Directory {
                label: label.clone(),
                count_text: messages::files_count(*file_count),
            },
            TreeRowKind::File { path } => self.file_row_kind(path),
        };
        RowView {
            id: row.id.clone(),
            depth: row.depth,
            expanded,
            selected: self.is_selected(&row.id),
            kind,
        }
    }

    fn file_row_kind(&self, path: &RepoPath) -> RowViewKind {
        let summary = self.models.get(path);
        let (yours_present, theirs_present) = self.column_presence(path);
        let parent_path = if self.group_by_directory {
            None
        } else {
            path.parent()
                .map(|parent| parent.as_unix_str().to_string())
                .filter(|parent| !parent.is_empty())
        };
        RowViewKind::File {
            path: path.clone(),
            file_name: path.file_name().unwrap_or_default().to_string(),
            parent_path,
            badge: summary.map(|summary| BadgeView {
                value: messages::badge_value(summary.resolved, summary.total),
                modified: summary.resolved > 0,
            }),
            tooltip: summary
                .map(|summary| messages::merged_changes_tooltip(summary.resolved, summary.total)),
            yours_status: messages::column_status_text(yours_present),
            theirs_status: messages::column_status_text(theirs_present),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(value: &str) -> RepoPath {
        RepoPath::new(value).expect("valid repo path")
    }

    fn file(value: &str) -> RowId {
        RowId::File(path(value))
    }

    fn presence(ours: bool, theirs: bool) -> UnmergedStagePresence {
        UnmergedStagePresence {
            base: true,
            ours,
            theirs,
        }
    }

    fn summary(resolved: usize, unresolved: usize, auto_resolvable: usize) -> ModelSummary {
        ModelSummary {
            total: resolved + unresolved,
            resolved,
            unresolved,
            auto_resolvable,
            reviewed: false,
            chosen_side: None,
        }
    }

    fn state_with(files: &[&str], group_by_directory: bool) -> DialogState {
        let paths: Vec<RepoPath> = files.iter().map(|file| path(file)).collect();
        let presence_map = paths
            .iter()
            .map(|file| (file.clone(), presence(true, true)))
            .collect();
        DialogState::new(paths, presence_map, false, group_by_directory)
    }

    fn row_ids(state: &DialogState) -> Vec<RowId> {
        state
            .visible_rows()
            .iter()
            .map(|row| row.id.clone())
            .collect()
    }

    fn directory(label: &str, file_count: usize) -> TreeRowKind {
        TreeRowKind::Directory {
            label: label.to_string(),
            file_count,
        }
    }

    #[test]
    fn merge_tool_initial_selection_is_the_unresolved_header_and_selects_all_files() {
        let state = state_with(&["b.rs", "a.rs"], false);
        assert_eq!(state.selected_ids(), &[RowId::Group(GroupKind::Unresolved)]);
        assert_eq!(state.selected_files(), vec![path("a.rs"), path("b.rs")]);
    }

    #[test]
    fn merge_tool_flat_rows_sort_files_by_natural_name_under_the_group() {
        let state = state_with(&["src/file10.rs", "src/file2.rs", "a/z.rs"], false);
        assert_eq!(
            row_ids(&state),
            vec![
                RowId::Group(GroupKind::Unresolved),
                file("src/file2.rs"),
                file("src/file10.rs"),
                file("a/z.rs"),
            ]
        );
    }

    #[test]
    fn merge_tool_flat_rows_keep_the_input_order_for_equal_file_names() {
        let state = state_with(&["z/a.rs", "m/a.rs", "b.rs"], false);
        assert_eq!(
            row_ids(&state),
            vec![
                RowId::Group(GroupKind::Unresolved),
                file("z/a.rs"),
                file("m/a.rs"),
                file("b.rs"),
            ]
        );
    }

    #[test]
    fn merge_tool_directory_mode_wraps_root_files_and_directories_in_a_root_node() {
        let state = state_with(
            &["src/main/java/A.java", "src/main/java/B.java", "README.md"],
            true,
        );
        let rows = state.visible_rows();
        assert_eq!(rows.len(), 6);
        assert_eq!((rows[1].depth, &rows[1].kind), (1, &directory("", 3)));
        assert_eq!(
            rows[1].id,
            RowId::Directory(GroupKind::Unresolved, String::new())
        );
        assert_eq!(
            (rows[2].depth, &rows[2].kind),
            (2, &directory("src/main/java", 2))
        );
        assert_eq!(
            rows[2].id,
            RowId::Directory(GroupKind::Unresolved, "src/main/java".to_string())
        );
        assert_eq!(
            (rows[3].depth, &rows[3].id),
            (3, &file("src/main/java/A.java"))
        );
        assert_eq!(
            (rows[4].depth, &rows[4].id),
            (3, &file("src/main/java/B.java"))
        );
        assert_eq!((rows[5].depth, &rows[5].id), (2, &file("README.md")));
    }

    #[test]
    fn merge_tool_directory_mode_puts_files_that_sit_in_the_root_under_a_root_node() {
        let state = state_with(&["b.rs", "a.rs"], true);
        let rows = state.visible_rows();
        assert_eq!(rows.len(), 4);
        assert_eq!((rows[1].depth, &rows[1].kind), (1, &directory("", 2)));
        assert_eq!((rows[2].depth, &rows[2].id), (2, &file("a.rs")));
        assert_eq!((rows[3].depth, &rows[3].id), (2, &file("b.rs")));
    }

    #[test]
    fn merge_tool_directory_mode_collapses_a_lone_directory_chain_into_the_top_node() {
        let state = state_with(&["src/main/A.java", "src/main/B.java"], true);
        let rows = state.visible_rows();
        assert_eq!(rows.len(), 4);
        assert_eq!(
            (rows[1].depth, &rows[1].kind),
            (1, &directory("src/main", 2))
        );
        assert_eq!((rows[2].depth, &rows[2].id), (2, &file("src/main/A.java")));
    }

    #[test]
    fn merge_tool_directory_mode_keeps_branching_directories_separate() {
        let state = state_with(&["src/a/x.rs", "src/b/y.rs"], true);
        let rows = state.visible_rows();
        assert_eq!((rows[1].depth, &rows[1].kind), (1, &directory("src", 2)));
        assert_eq!((rows[2].depth, &rows[2].kind), (2, &directory("a", 1)));
        assert_eq!((rows[3].depth, &rows[3].id), (3, &file("src/a/x.rs")));
        assert_eq!((rows[4].depth, &rows[4].kind), (2, &directory("b", 1)));
    }

    #[test]
    fn merge_tool_directory_mode_orders_directories_before_files_and_names_naturally() {
        let state = state_with(&["b10/x.rs", "b2/x.rs", "a.rs", "A1.rs"], true);
        let ids: Vec<RowId> = state
            .visible_rows()
            .iter()
            .map(|row| row.id.clone())
            .collect();
        assert_eq!(
            ids,
            vec![
                RowId::Group(GroupKind::Unresolved),
                RowId::Directory(GroupKind::Unresolved, String::new()),
                RowId::Directory(GroupKind::Unresolved, "b2".to_string()),
                file("b2/x.rs"),
                RowId::Directory(GroupKind::Unresolved, "b10".to_string()),
                file("b10/x.rs"),
                file("a.rs"),
                file("A1.rs"),
            ]
        );
    }

    #[test]
    fn merge_tool_directory_selection_selects_descendant_files() {
        let mut state = state_with(&["src/a.rs", "src/b.rs", "other.rs"], true);
        state.select_only(RowId::Directory(GroupKind::Unresolved, "src".to_string()));
        assert_eq!(
            state.selected_files(),
            vec![path("src/a.rs"), path("src/b.rs")]
        );
    }

    #[test]
    fn merge_tool_resolved_files_move_to_the_resolved_group() {
        let mut state = state_with(&["a.rs", "b.rs"], false);
        state.set_model_summary(path("a.rs"), summary(2, 0, 0));
        state.rebuild(TreeStateStrategy::Default);
        assert_eq!(
            row_ids(&state),
            vec![
                RowId::Group(GroupKind::Unresolved),
                file("b.rs"),
                RowId::Group(GroupKind::Resolved),
                file("a.rs"),
            ]
        );
    }

    #[test]
    fn merge_tool_resolved_group_lists_equal_names_in_model_creation_order() {
        let mut state = state_with(&["m/a.rs", "z/a.rs"], false);
        state.set_model_summary(path("z/a.rs"), summary(1, 0, 0));
        state.set_model_summary(path("m/a.rs"), summary(1, 0, 0));
        assert_eq!(
            state.resolved_file_paths(),
            vec![path("z/a.rs"), path("m/a.rs")]
        );
        state.set_model_summary(path("z/a.rs"), summary(2, 0, 0));
        assert_eq!(
            state.resolved_file_paths(),
            vec![path("z/a.rs"), path("m/a.rs")]
        );
        state.remove_model_summaries(&[path("z/a.rs")]);
        state.set_model_summary(path("z/a.rs"), summary(1, 0, 0));
        assert_eq!(
            state.resolved_file_paths(),
            vec![path("m/a.rs"), path("z/a.rs")]
        );
    }

    #[test]
    fn merge_tool_resolved_header_appears_after_first_resolve_all_even_when_empty() {
        let mut state = state_with(&["a.rs"], false);
        state.begin_resolve_all();
        state.finish_resolve_all();
        state.rebuild(TreeStateStrategy::Default);
        assert_eq!(
            row_ids(&state),
            vec![
                RowId::Group(GroupKind::Unresolved),
                file("a.rs"),
                RowId::Group(GroupKind::Resolved),
            ]
        );
    }

    #[test]
    fn merge_tool_resolve_all_is_enabled_before_first_press() {
        let state = state_with(&["a.rs"], false);
        assert!(state.resolve_all_enabled());
    }

    #[test]
    fn merge_tool_resolve_all_disables_after_press_without_auto_resolvable_changes() {
        let mut state = state_with(&["a.rs"], false);
        state.begin_resolve_all();
        state.set_model_summary(path("a.rs"), summary(0, 2, 0));
        state.finish_resolve_all();
        assert!(!state.resolve_all_enabled());
        state.reset_resolve_all_pressed();
        assert!(state.resolve_all_enabled());
    }

    #[test]
    fn merge_tool_resolve_all_is_enabled_when_any_file_has_auto_resolvable_changes() {
        let mut state = state_with(&["a.rs", "b.rs"], false);
        state.begin_resolve_all();
        state.finish_resolve_all();
        state.set_model_summary(path("b.rs"), summary(0, 3, 1));
        assert!(state.resolve_all_enabled());
    }

    #[test]
    fn merge_tool_resolve_all_is_disabled_when_every_file_is_resolved() {
        let mut state = state_with(&["a.rs"], false);
        state.set_model_summary(path("a.rs"), summary(1, 0, 0));
        assert!(!state.resolve_all_enabled());
    }

    #[test]
    fn merge_tool_resolve_all_requires_a_model_for_every_file_to_count_as_all_resolved() {
        let mut state = state_with(&["a.rs", "b.rs"], false);
        state.set_model_summary(path("a.rs"), summary(1, 0, 0));
        assert!(state.resolve_all_enabled());
    }

    #[test]
    fn merge_tool_review_label_is_review_changes_for_empty_or_resolved_selection() {
        let mut state = state_with(&["a.rs"], false);
        assert_eq!(state.review_button_label(), messages::RESOLVE_MANUALLY);
        state.set_model_summary(path("a.rs"), summary(1, 0, 0));
        assert_eq!(state.review_button_label(), messages::REVIEW_CHANGES);
        state.selected.clear();
        assert_eq!(state.review_button_label(), messages::REVIEW_CHANGES);
    }

    #[test]
    fn merge_tool_accept_and_finish_requires_every_pending_file_to_be_resolved() {
        let mut state = state_with(&["a.rs", "b.rs"], false);
        assert!(!state.accept_and_finish_enabled());
        state.set_model_summary(path("a.rs"), summary(1, 0, 0));
        assert!(!state.accept_and_finish_enabled());
        state.set_model_summary(path("b.rs"), summary(2, 0, 0));
        assert!(state.accept_and_finish_enabled());
        state.begin_resolve_all();
        assert!(!state.accept_and_finish_enabled());
    }

    #[test]
    fn merge_tool_processed_files_do_not_block_accept_and_finish() {
        let mut state = state_with(&["a.rs", "b.rs"], false);
        state.set_model_summary(path("a.rs"), summary(1, 0, 0));
        state.mark_processed(&[path("b.rs")]);
        assert!(state.accept_and_finish_enabled());
        assert_eq!(state.processed_files(), &[path("b.rs")]);
    }

    #[test]
    fn merge_tool_all_reviewed_state_switches_default_button_and_hides_review() {
        let mut state = state_with(&["a.rs"], false);
        assert_eq!(state.default_button(), DefaultButton::ReviewOrResolve);
        assert!(state.review_button_visible());
        let mut reviewed = summary(1, 0, 0);
        reviewed.reviewed = true;
        state.set_model_summary(path("a.rs"), reviewed);
        assert_eq!(state.default_button(), DefaultButton::AcceptAndFinish);
        assert!(!state.review_button_visible());
    }

    #[test]
    fn merge_tool_reviewed_state_requires_every_pending_file_to_be_reviewed_and_resolved() {
        let mut state = state_with(&["a.rs", "b.rs"], false);
        let mut reviewed = summary(1, 0, 0);
        reviewed.reviewed = true;
        state.set_model_summary(path("a.rs"), reviewed);
        assert!(!state.derived().all_files_resolved_and_reviewed);
        let mut reviewed_but_unresolved = summary(1, 1, 0);
        reviewed_but_unresolved.reviewed = true;
        state.set_model_summary(path("b.rs"), reviewed_but_unresolved);
        assert!(!state.derived().all_files_resolved_and_reviewed);
        state.set_model_summary(path("b.rs"), reviewed);
        assert!(state.derived().all_files_resolved_and_reviewed);
    }

    #[test]
    fn merge_tool_revertable_selection_needs_resolved_changes_in_every_selected_file() {
        let mut state = state_with(&["a.rs", "b.rs"], false);
        state.set_model_summary(path("a.rs"), summary(1, 1, 0));
        assert!(!state.derived().only_revertable_files_selected);
        state.set_model_summary(path("b.rs"), summary(2, 0, 0));
        assert!(state.derived().only_revertable_files_selected);
        state.selected.clear();
        assert!(!state.derived().only_revertable_files_selected);
    }

    #[test]
    fn merge_tool_resolved_selection_flags_follow_the_selected_files() {
        let mut state = state_with(&["a.rs", "b.rs"], false);
        state.set_model_summary(path("a.rs"), summary(1, 0, 0));
        let derived = state.derived();
        assert!(derived.resolved_files_selected);
        assert!(!derived.all_selected_files_resolved);
        state.select_only(file("a.rs"));
        let derived = state.derived();
        assert!(derived.resolved_files_selected);
        assert!(derived.all_selected_files_resolved);
        state.select_only(file("b.rs"));
        assert!(!state.derived().resolved_files_selected);
    }

    #[test]
    fn merge_tool_partial_resolution_is_detected_per_file() {
        let mut state = state_with(&["a.rs", "b.rs"], false);
        assert!(!state.has_partially_resolved_files());
        state.set_model_summary(path("a.rs"), summary(2, 0, 0));
        state.set_model_summary(path("b.rs"), summary(0, 3, 0));
        assert!(!state.has_partially_resolved_files());
        state.set_model_summary(path("b.rs"), summary(1, 2, 0));
        assert!(state.has_partially_resolved_files());
    }

    #[test]
    fn merge_tool_status_totals_sum_changes_over_files_that_have_models() {
        let mut state = state_with(&["a.rs", "b.rs", "c.rs"], false);
        state.set_model_summary(path("a.rs"), summary(2, 0, 0));
        state.set_model_summary(path("b.rs"), summary(1, 3, 0));
        assert_eq!(state.total_resolved_changes(), 3);
        assert_eq!(state.total_unresolved_changes(), 3);
        assert_eq!(state.files_with_unresolved_changes(), 1);
    }

    #[test]
    fn merge_tool_nothing_is_left_to_process_once_every_file_is_marked_processed() {
        let mut state = state_with(&["a.rs", "b.rs"], false);
        assert!(!state.has_nothing_left_to_process());
        state.mark_processed(&[path("a.rs")]);
        assert!(!state.has_nothing_left_to_process());
        state.mark_processed(&[path("b.rs"), path("a.rs")]);
        assert!(state.has_nothing_left_to_process());
        assert_eq!(state.processed_files(), &[path("a.rs"), path("b.rs")]);
    }

    #[test]
    fn merge_tool_processed_files_leave_the_tree_on_rebuild() {
        let mut state = state_with(&["a.rs", "b.rs"], false);
        state.mark_processed(&[path("a.rs")]);
        state.rebuild(TreeStateStrategy::ModelChange);
        assert_eq!(
            row_ids(&state),
            vec![RowId::Group(GroupKind::Unresolved), file("b.rs")]
        );
    }

    #[test]
    fn merge_tool_files_to_open_puts_selection_first_then_unresolved_then_unreviewed_resolved() {
        let mut state = state_with(&["a.rs", "b.rs", "c.rs", "d.rs"], false);
        let mut reviewed = summary(1, 0, 0);
        reviewed.reviewed = true;
        state.set_model_summary(path("d.rs"), reviewed);
        state.set_model_summary(path("c.rs"), summary(1, 0, 0));
        state.rebuild(TreeStateStrategy::Default);
        state.select_only(file("b.rs"));
        assert_eq!(
            state.files_to_open(true),
            vec![path("b.rs"), path("a.rs"), path("c.rs")]
        );
    }

    #[test]
    fn merge_tool_files_to_open_orders_equal_names_by_hierarchy_in_flat_mode() {
        let state = state_with(&["z/a.rs", "m/a.rs", "b.rs"], false);
        assert_eq!(
            state.files_to_open(true),
            vec![path("m/a.rs"), path("z/a.rs"), path("b.rs")]
        );
    }

    #[test]
    fn merge_tool_files_to_open_lists_directories_first_in_directory_mode() {
        let state = state_with(&["b.rs", "z/a.rs", "m/c.rs"], true);
        assert_eq!(
            state.files_to_open(true),
            vec![path("m/c.rs"), path("z/a.rs"), path("b.rs")]
        );
    }

    #[test]
    fn merge_tool_files_to_open_without_suggestion_is_only_the_selection() {
        let mut state = state_with(&["a.rs", "b.rs"], false);
        state.select_only(file("b.rs"));
        assert_eq!(state.files_to_open(false), vec![path("b.rs")]);
    }

    #[test]
    fn merge_tool_accept_button_follows_hover_else_single_selection() {
        let mut state = state_with(&["a.rs", "b.rs"], false);
        let first_row = file("a.rs");
        let second_row = file("b.rs");
        assert!(!state.accept_button_visible(&first_row, None));
        state.select_only(first_row.clone());
        assert!(state.accept_button_visible(&first_row, None));
        assert!(!state.accept_button_visible(&second_row, None));
        assert!(state.accept_button_visible(&second_row, Some(&second_row)));
        assert!(!state.accept_button_visible(&first_row, Some(&second_row)));
        state.set_model_summary(path("b.rs"), summary(1, 0, 0));
        assert!(!state.accept_button_visible(&second_row, Some(&second_row)));
    }

    #[test]
    fn merge_tool_accept_button_is_hidden_on_rows_that_are_not_files() {
        let state = state_with(&["a.rs"], false);
        let header = RowId::Group(GroupKind::Unresolved);
        assert!(!state.accept_button_visible(&header, Some(&header)));
        assert!(!state.accept_button_visible(&header, None));
    }

    #[test]
    fn merge_tool_selection_moves_and_extends_over_visible_rows() {
        let mut state = state_with(&["a.rs", "b.rs", "c.rs"], false);
        state.move_selection(SelectionMove::Next, false);
        assert_eq!(state.selected_ids(), &[file("a.rs")]);
        state.move_selection(SelectionMove::Next, true);
        assert_eq!(state.selected_ids(), &[file("a.rs"), file("b.rs")]);
        state.move_selection(SelectionMove::Last, false);
        assert_eq!(state.selected_ids(), &[file("c.rs")]);
        state.move_selection(SelectionMove::Next, false);
        assert_eq!(state.selected_ids(), &[file("c.rs")]);
        state.move_selection(SelectionMove::First, false);
        assert_eq!(state.selected_ids(), &[RowId::Group(GroupKind::Unresolved)]);
    }

    #[test]
    fn merge_tool_selection_extends_upwards_from_the_anchor_and_shrinks_back() {
        let mut state = state_with(&["a.rs", "b.rs", "c.rs", "d.rs"], false);
        state.select_only(file("d.rs"));
        state.move_selection(SelectionMove::Previous, true);
        assert_eq!(state.selected_ids(), &[file("c.rs"), file("d.rs")]);
        state.move_selection(SelectionMove::Previous, true);
        assert_eq!(
            state.selected_ids(),
            &[file("b.rs"), file("c.rs"), file("d.rs")]
        );
        state.move_selection(SelectionMove::Next, true);
        assert_eq!(state.selected_ids(), &[file("c.rs"), file("d.rs")]);
        state.move_selection(SelectionMove::Previous, false);
        assert_eq!(state.selected_ids(), &[file("b.rs")]);
    }

    #[test]
    fn merge_tool_shift_click_extends_from_the_anchor_in_either_direction() {
        let mut state = state_with(&["a.rs", "b.rs", "c.rs", "d.rs"], false);
        state.select_only(file("c.rs"));
        state.extend_selection_to(file("a.rs"));
        assert_eq!(
            state.selected_ids(),
            &[file("a.rs"), file("b.rs"), file("c.rs")]
        );
        state.extend_selection_to(file("d.rs"));
        assert_eq!(state.selected_ids(), &[file("c.rs"), file("d.rs")]);
    }

    #[test]
    fn merge_tool_select_all_covers_every_visible_row() {
        let mut state = state_with(&["a.rs", "b.rs"], false);
        state.select_all();
        assert_eq!(state.selected_ids().len(), 3);
        assert_eq!(state.selected_files(), vec![path("a.rs"), path("b.rs")]);
        state.move_selection(SelectionMove::Previous, false);
        assert_eq!(state.selected_ids(), &[file("a.rs")]);
    }

    #[test]
    fn merge_tool_page_moves_jump_by_the_given_row_count_and_clamp_at_the_ends() {
        let mut state = state_with(&["a.rs", "b.rs", "c.rs", "d.rs", "e.rs"], false);
        state.move_selection(SelectionMove::PageNext(3), false);
        assert_eq!(state.selected_ids(), &[file("c.rs")]);
        state.move_selection(SelectionMove::PageNext(9), false);
        assert_eq!(state.selected_ids(), &[file("e.rs")]);
        state.move_selection(SelectionMove::PagePrevious(2), true);
        assert_eq!(
            state.selected_ids(),
            &[file("c.rs"), file("d.rs"), file("e.rs")]
        );
        state.move_selection(SelectionMove::PagePrevious(40), false);
        assert_eq!(state.selected_ids(), &[RowId::Group(GroupKind::Unresolved)]);
    }

    #[test]
    fn merge_tool_collapsed_groups_hide_children_but_keep_them_selected_via_the_header() {
        let mut state = state_with(&["a.rs", "b.rs"], false);
        state.set_expanded(&RowId::Group(GroupKind::Unresolved), false);
        assert_eq!(row_ids(&state), vec![RowId::Group(GroupKind::Unresolved)]);
        assert_eq!(state.selected_files().len(), 2);
        state.set_expanded(&RowId::Group(GroupKind::Unresolved), true);
        assert_eq!(state.visible_rows().len(), 3);
    }

    #[test]
    fn merge_tool_collapsing_a_node_moves_the_hidden_selection_onto_it() {
        let mut state = state_with(&["a.rs", "b.rs"], false);
        state.select_only(file("a.rs"));
        state.toggle_selection(file("b.rs"));
        state.set_expanded(&RowId::Group(GroupKind::Unresolved), false);
        assert_eq!(state.selected_ids(), &[RowId::Group(GroupKind::Unresolved)]);
        state.set_expanded(&RowId::Group(GroupKind::Unresolved), true);
        assert_eq!(state.selected_ids(), &[RowId::Group(GroupKind::Unresolved)]);
    }

    #[test]
    fn merge_tool_collapsing_an_unrelated_node_keeps_the_selection() {
        let mut state = state_with(&["a.rs"], false);
        state.begin_resolve_all();
        state.finish_resolve_all();
        state.rebuild(TreeStateStrategy::Default);
        state.select_only(file("a.rs"));
        state.set_expanded(&RowId::Group(GroupKind::Resolved), false);
        assert_eq!(state.selected_ids(), &[file("a.rs")]);
    }

    #[test]
    fn merge_tool_left_and_right_keys_collapse_expand_and_walk_the_tree() {
        let mut state = state_with(&["a.rs", "b.rs"], false);
        state.collapse_or_expand_single_selection(false);
        assert_eq!(row_ids(&state), vec![RowId::Group(GroupKind::Unresolved)]);
        state.collapse_or_expand_single_selection(false);
        assert_eq!(row_ids(&state), vec![RowId::Group(GroupKind::Unresolved)]);
        state.collapse_or_expand_single_selection(true);
        assert_eq!(state.visible_rows().len(), 3);
        state.collapse_or_expand_single_selection(true);
        assert_eq!(state.selected_ids(), &[file("a.rs")]);
        state.collapse_or_expand_single_selection(true);
        assert_eq!(state.selected_ids(), &[file("a.rs")]);
        state.collapse_or_expand_single_selection(false);
        assert_eq!(state.selected_ids(), &[RowId::Group(GroupKind::Unresolved)]);
    }

    #[test]
    fn merge_tool_toggle_selection_adds_and_removes_rows() {
        let mut state = state_with(&["a.rs", "b.rs"], false);
        state.select_only(file("a.rs"));
        state.toggle_selection(file("b.rs"));
        assert_eq!(state.selected_files(), vec![path("a.rs"), path("b.rs")]);
        state.toggle_selection(file("a.rs"));
        assert_eq!(state.selected_files(), vec![path("b.rs")]);
    }

    #[test]
    fn merge_tool_model_change_rebuild_keeps_selection_or_falls_back_to_nearest_row() {
        let mut state = state_with(&["a.rs", "b.rs", "c.rs"], false);
        state.select_only(file("b.rs"));
        state.set_model_summary(path("b.rs"), summary(1, 0, 0));
        state.rebuild(TreeStateStrategy::ModelChange);
        assert_eq!(state.selected_ids(), &[file("b.rs")]);
        state.mark_processed(&[path("b.rs")]);
        state.remove_model_summaries(&[path("b.rs")]);
        state.rebuild(TreeStateStrategy::ModelChange);
        assert_eq!(state.selected_ids(), &[file("c.rs")]);
    }

    #[test]
    fn merge_tool_grouping_change_reselects_the_same_files() {
        let mut state = state_with(&["src/a.rs", "src/b.rs"], false);
        state.select_only(file("src/b.rs"));
        state.set_group_by_directory(true);
        state.rebuild(TreeStateStrategy::GroupingChange);
        assert_eq!(state.selected_ids(), &[file("src/b.rs")]);
    }

    #[test]
    fn merge_tool_grouping_change_selects_the_first_file_when_nothing_was_selected() {
        let mut state = state_with(&["src/a.rs", "src/b.rs"], false);
        state.selected.clear();
        state.set_group_by_directory(true);
        state.rebuild(TreeStateStrategy::GroupingChange);
        assert_eq!(state.selected_ids(), &[file("src/a.rs")]);
    }

    #[test]
    fn merge_tool_row_views_report_badges_tooltips_and_status_columns() {
        let mut state = DialogState::new(
            vec![path("src/a.rs")],
            [(path("src/a.rs"), presence(true, false))]
                .into_iter()
                .collect(),
            false,
            false,
        );
        state.set_model_summary(path("src/a.rs"), summary(1, 2, 0));
        state.rebuild(TreeStateStrategy::Default);
        let views = state.row_views();
        let RowViewKind::File {
            badge,
            tooltip,
            yours_status,
            theirs_status,
            parent_path,
            ..
        } = &views[1].kind
        else {
            panic!("expected a file row");
        };
        assert_eq!(
            badge,
            &Some(BadgeView {
                value: "1/3".to_string(),
                modified: true
            })
        );
        assert_eq!(tooltip.as_deref(), Some("1 change has been merged"));
        assert_eq!(*yours_status, "Modified");
        assert_eq!(*theirs_status, "Deleted");
        assert_eq!(parent_path.as_deref(), Some("src"));
    }

    #[test]
    fn merge_tool_row_views_of_groups_and_directories_carry_file_counts() {
        let state = state_with(&["src/a.rs", "src/b.rs"], true);
        let views = state.row_views();
        let RowViewKind::Group {
            title, count_text, ..
        } = &views[0].kind
        else {
            panic!("expected a group row");
        };
        assert_eq!((*title, count_text.as_str()), ("Unresolved", "2 files"));
        let RowViewKind::Directory { label, count_text } = &views[1].kind else {
            panic!("expected a directory row");
        };
        assert_eq!((label.as_str(), count_text.as_str()), ("src", "2 files"));
        assert_eq!(views[1].expanded, Some(true));
        assert_eq!(views[2].expanded, None);
    }

    #[test]
    fn merge_tool_unresolved_model_summary_shows_a_badge_without_modified_style() {
        let mut state = state_with(&["a.rs"], false);
        state.set_model_summary(path("a.rs"), summary(0, 4, 0));
        let views = state.row_views();
        let RowViewKind::File { badge, tooltip, .. } = &views[1].kind else {
            panic!("expected a file row");
        };
        assert_eq!(
            badge,
            &Some(BadgeView {
                value: "0/4".to_string(),
                modified: false
            })
        );
        assert_eq!(tooltip.as_deref(), Some("4 changes"));
    }

    #[test]
    fn merge_tool_reversed_state_swaps_status_columns() {
        let state = DialogState::new(
            vec![path("a.rs")],
            [(path("a.rs"), presence(true, false))]
                .into_iter()
                .collect(),
            true,
            false,
        );
        let views = state.row_views();
        let RowViewKind::File {
            yours_status,
            theirs_status,
            ..
        } = &views[1].kind
        else {
            panic!("expected a file row");
        };
        assert_eq!(*yours_status, "Deleted");
        assert_eq!(*theirs_status, "Modified");
    }

    #[test]
    fn merge_tool_speed_search_selects_the_next_file_whose_name_contains_the_query() {
        let mut state = state_with(&["src/alpha.rs", "src/beta.rs", "docs/Betamax.md"], false);
        assert!(state.select_file_matching("BETA"));
        assert_eq!(state.selected_ids(), &[file("src/beta.rs")]);
        assert!(state.select_file_matching("beta"));
        assert_eq!(state.selected_ids(), &[file("src/beta.rs")]);
        assert!(state.select_file_matching("betam"));
        assert_eq!(state.selected_ids(), &[file("docs/Betamax.md")]);
        assert!(state.select_file_matching("alpha"));
        assert_eq!(state.selected_ids(), &[file("src/alpha.rs")]);
    }

    #[test]
    fn merge_tool_speed_search_ignores_directories_and_misses() {
        let mut state = state_with(&["src/alpha.rs"], true);
        assert!(!state.select_file_matching("src"));
        assert!(!state.select_file_matching("zzz"));
        assert!(!state.select_file_matching(""));
    }
}
