use std::{ops::Range, sync::Arc};

use anyhow::Result;
use collections::{HashMap, HashSet};
use editor::{
    Anchor, Bias, DisplayPoint, Editor, EditorEvent, MultiBufferOffset, MultiBufferSnapshot,
    RowHighlightOptions, SelectionEffects, ToOffset as _, ToPoint as _,
    display_map::{DisplayRow, DisplaySnapshot},
    scroll::Autoscroll,
};
use git::repository::RepoPath;
use gpui::{
    Action as _, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla,
    IntoElement, Modifiers, PromptLevel, Render, Subscription, Task, TaskExt as _, WeakEntity,
    Window, point,
};
use language::{Language, Point, TransactionId};
use project::{
    Project,
    git_store::{ConflictStages, Repository, RepositoryId, RepositorySnapshot},
    project_settings::ProjectSettings,
};
use settings::Settings as _;
use three_way_merge::{
    ApplyMode, ChangeType, Chunk, ChunkEdit, ChunkState, Counters, IgnorePolicy, MergeDocument,
    MergeModel, NonConflictingScope, ResultTracker, Revision, Side, ThreeWayMerge, map_line,
};
use ui::{
    ContextMenu, ContextMenuEntry, Divider, PopoverMenu, PopoverMenuHandle, Tooltip, prelude::*,
};
use util::{ResultExt as _, paths::PathStyle};
use workspace::{Item, Workspace, item::ItemEvent, notifications::DetachAndPromptErr as _};

use super::{
    AcceptLeft, AcceptLeftSide, AcceptRight, AcceptRightSide, AppendLeftSide, AppendRightSide,
    ApplyChanges, ApplyNonConflictingAll, ApplyNonConflictingLeft, ApplyNonConflictingRight,
    FocusOppositePane, IgnoreLeftSide, IgnoreRightSide, MergeBranches, NextDifference,
    PreviousDifference, ResolveSimpleConflict, ResolveSimpleConflicts, ResolveUsingLeft,
    ResolveUsingRight, SaveAndClose, ShowSettings, ToggleSynchronizeScrolling,
    document_matches_merge,
    merge_divider::{ChangeKind, DIVIDER_WIDTH, DividerChunk, MergeDivider, display_row_for_line},
    merge_for_stages, merge_session, open_merge_tool, resolved_text, show_conflicts_dialog,
    store_merge_session, write_merge_result,
};

const ROW_HIGHLIGHT_OPACITY: f32 = 0.2;
const TOUCH_TEXT: &str = " ";
const UNRESOLVED_PROMPT_MESSAGE: &str = "Unresolved changes remain";
const APPLY_ANSWER: &str = "Apply";
const CANCEL_ANSWER: &str = "Cancel";

pub(super) enum MergeAddedRows {}
pub(super) enum MergeModifiedRows {}
pub(super) enum MergeDeletedRows {}
pub(super) enum MergeConflictRows {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum MergePane {
    Left,
    Result,
    Right,
}

impl MergePane {
    const ALL: [MergePane; 3] = [MergePane::Left, MergePane::Result, MergePane::Right];
}

struct JournalEntry {
    before: Vec<ChunkState>,
    after: Vec<ChunkState>,
    undone: bool,
}

pub struct MergeView {
    workspace: WeakEntity<Workspace>,
    project: Entity<Project>,
    repository: Entity<Repository>,
    repository_id: RepositoryId,
    pub(super) repo_path: RepoPath,
    stages: ConflictStages,
    pub(super) branches: MergeBranches,
    pub(super) left_editor: Entity<Editor>,
    pub(super) result_editor: Entity<Editor>,
    pub(super) right_editor: Entity<Editor>,
    pub(super) model: MergeModel,
    chunk_anchors: Vec<Range<Anchor>>,
    journal: HashMap<TransactionId, JournalEntry>,
    pub(super) synchronize_scrolling: bool,
    pending_scroll_events: HashSet<MergePane>,
    settings_menu_handle: PopoverMenuHandle<ContextMenu>,
    finishing: bool,
    _subscriptions: Vec<Subscription>,
}

impl MergeView {
    pub(crate) fn open(
        workspace: &mut Workspace,
        repository: Entity<Repository>,
        repo_path: RepoPath,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Task<Result<()>> {
        let repository_id = repository.read(cx).id;
        if Self::activate_existing(workspace, repository_id, &repo_path, window, cx) {
            return Task::ready(Ok(()));
        }
        let project = workspace.project().clone();
        let languages = project.read(cx).languages().clone();
        let language_path = repo_path.as_std_path().to_path_buf();
        let stages = repository.update(cx, |repository, cx| {
            repository.load_conflict_stages(repo_path.clone(), cx)
        });
        cx.spawn_in(window, async move |workspace, cx| {
            let stages = stages.await?;
            let language = languages
                .load_language_for_file_path(&language_path)
                .await
                .ok();
            workspace.update_in(cx, |workspace, window, cx| {
                if Self::activate_existing(workspace, repository_id, &repo_path, window, cx) {
                    return;
                }
                let workspace_handle = cx.weak_entity();
                let merge_view = cx.new(|cx| {
                    Self::new(
                        workspace_handle,
                        project,
                        repository,
                        repo_path,
                        stages,
                        language,
                        window,
                        cx,
                    )
                });
                workspace.add_item_to_active_pane(Box::new(merge_view), None, true, window, cx);
            })
        })
    }

    fn activate_existing(
        workspace: &mut Workspace,
        repository_id: RepositoryId,
        repo_path: &RepoPath,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> bool {
        let existing = workspace
            .items_of_type::<MergeView>(cx)
            .find(|merge_view| merge_view.read(cx).matches(repository_id, repo_path));
        let Some(existing) = existing else {
            return false;
        };
        workspace.activate_item(&existing, true, true, window, cx);
        true
    }

    fn new(
        workspace: WeakEntity<Workspace>,
        project: Entity<Project>,
        repository: Entity<Repository>,
        repo_path: RepoPath,
        stages: ConflictStages,
        language: Option<Arc<Language>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let repository_id = repository.read(cx).id;
        let branches = MergeBranches::for_snapshot(repository.read(cx));
        let fresh_merge = merge_for_stages(&stages, IgnorePolicy::default());
        let session = merge_session(repository_id, &repo_path, cx)
            .filter(|document| document_matches_merge(document, &fresh_merge));
        let restored = session.is_some();
        let (model, tracker) = match session {
            Some(document) => document.into_parts(),
            None => {
                let tracker = ResultTracker::new(&fresh_merge);
                (MergeModel::new(fresh_merge), tracker)
            }
        };

        let left_buffer = project.update(cx, |project, cx| {
            project.create_local_buffer(
                model.merge().text(Revision::Left),
                language.clone(),
                false,
                cx,
            )
        });
        let result_buffer = project.update(cx, |project, cx| {
            project.create_local_buffer(tracker.text(), language.clone(), false, cx)
        });
        let right_buffer = project.update(cx, |project, cx| {
            project.create_local_buffer(
                model.merge().text(Revision::Right),
                language.clone(),
                false,
                cx,
            )
        });

        let left_editor = cx.new(|cx| {
            let mut editor = Editor::for_buffer(left_buffer, Some(project.clone()), window, cx);
            editor.set_read_only(true);
            configure_pane_editor(&mut editor, cx);
            editor
        });
        let result_editor = cx.new(|cx| {
            let mut editor = Editor::for_buffer(result_buffer, Some(project.clone()), window, cx);
            configure_pane_editor(&mut editor, cx);
            editor
        });
        let right_editor = cx.new(|cx| {
            let mut editor = Editor::for_buffer(right_buffer, Some(project.clone()), window, cx);
            editor.set_read_only(true);
            configure_pane_editor(&mut editor, cx);
            editor
        });

        let result_snapshot = result_editor.read(cx).buffer().read(cx).snapshot(cx);
        let chunk_anchors = tracker
            .chunk_ranges()
            .iter()
            .map(|range| anchor_range(&result_snapshot, range.clone()))
            .collect();

        let subscriptions = vec![
            cx.subscribe_in(&left_editor, window, |this, _, event, window, cx| {
                this.handle_pane_event(MergePane::Left, event, window, cx);
            }),
            cx.subscribe_in(&result_editor, window, |this, _, event, window, cx| {
                this.handle_pane_event(MergePane::Result, event, window, cx);
            }),
            cx.subscribe_in(&right_editor, window, |this, _, event, window, cx| {
                this.handle_pane_event(MergePane::Right, event, window, cx);
            }),
        ];
        install_result_context_menu(&result_editor, cx.weak_entity(), cx);

        let mut merge_view = Self {
            workspace,
            project,
            repository,
            repository_id,
            repo_path,
            stages,
            branches,
            left_editor,
            result_editor,
            right_editor,
            model,
            chunk_anchors,
            journal: HashMap::default(),
            synchronize_scrolling: true,
            pending_scroll_events: HashSet::default(),
            settings_menu_handle: PopoverMenuHandle::default(),
            finishing: false,
            _subscriptions: subscriptions,
        };
        merge_view.refresh_highlights(cx);
        if !restored
            && ProjectSettings::get_global(cx)
                .git
                .merge_tool
                .auto_apply_non_conflicting
        {
            merge_view.apply_non_conflicting(NonConflictingScope::All, window, cx);
        }
        merge_view
    }

    fn matches(&self, repository_id: RepositoryId, repo_path: &RepoPath) -> bool {
        self.repository_id == repository_id && &self.repo_path == repo_path
    }

    fn editor(&self, pane: MergePane) -> &Entity<Editor> {
        match pane {
            MergePane::Left => &self.left_editor,
            MergePane::Result => &self.result_editor,
            MergePane::Right => &self.right_editor,
        }
    }

    fn result_snapshot(&self, cx: &App) -> MultiBufferSnapshot {
        self.result_editor.read(cx).buffer().read(cx).snapshot(cx)
    }

    fn result_line_ranges(&self, cx: &App) -> Vec<Range<u32>> {
        let snapshot = self.result_snapshot(cx);
        self.chunk_anchors
            .iter()
            .map(|range| anchor_line_range(&snapshot, range))
            .collect()
    }

    fn pane_line_ranges(&self, pane: MergePane, cx: &App) -> Vec<Range<u32>> {
        match pane {
            MergePane::Left => self
                .model
                .chunks()
                .iter()
                .map(|chunk| chunk.left.clone())
                .collect(),
            MergePane::Right => self
                .model
                .chunks()
                .iter()
                .map(|chunk| chunk.right.clone())
                .collect(),
            MergePane::Result => self.result_line_ranges(cx),
        }
    }

    fn chunk_at_line(&self, line: u32, snapshot: &MultiBufferSnapshot) -> Option<usize> {
        self.chunk_anchors.iter().position(|range| {
            let lines = anchor_line_range(snapshot, range);
            lines.contains(&line) || (lines.is_empty() && lines.start == line)
        })
    }

    fn cursor_line(&self, cx: &App) -> u32 {
        let editor = self.result_editor.read(cx);
        let snapshot = editor.buffer().read(cx).snapshot(cx);
        editor
            .selections
            .newest_anchor()
            .head()
            .to_point(&snapshot)
            .row
    }

    fn chunk_at_cursor(&self, cx: &App) -> Option<usize> {
        let line = self.cursor_line(cx);
        self.chunk_at_line(line, &self.result_snapshot(cx))
    }

    fn result_chunk_text(&self, chunk_index: usize, cx: &App) -> String {
        let Some(range) = self.chunk_anchors.get(chunk_index) else {
            return String::new();
        };
        self.result_snapshot(cx)
            .text_for_range(range.clone())
            .collect()
    }

    fn result_text(&self, cx: &App) -> String {
        self.result_editor.read(cx).text(cx)
    }

    fn document(&self, cx: &App) -> MergeDocument {
        let snapshot = self.result_snapshot(cx);
        let chunk_ranges = self
            .chunk_anchors
            .iter()
            .map(|range| range.start.to_offset(&snapshot).0..range.end.to_offset(&snapshot).0)
            .collect();
        MergeDocument::from_parts(
            self.model.clone(),
            ResultTracker::from_parts(snapshot.text(), chunk_ranges),
        )
    }

    fn handle_pane_event(
        &mut self,
        pane: MergePane,
        event: &EditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            EditorEvent::ScrollPositionChanged { local: true, .. } => {
                self.synchronize_scroll(pane, window, cx);
            }
            EditorEvent::TransactionUndone { transaction_id } if pane == MergePane::Result => {
                self.transaction_undone(*transaction_id, cx);
            }
            EditorEvent::Edited { transaction_id } if pane == MergePane::Result => {
                self.transaction_redone(*transaction_id, cx);
            }
            _ => {}
        }
    }

    fn transaction_undone(&mut self, transaction_id: TransactionId, cx: &mut Context<Self>) {
        let Some(entry) = self.journal.get_mut(&transaction_id) else {
            return;
        };
        if entry.undone {
            return;
        }
        entry.undone = true;
        let states = entry.before.clone();
        self.model.restore(states);
        self.refresh_highlights(cx);
        cx.notify();
    }

    fn transaction_redone(&mut self, transaction_id: TransactionId, cx: &mut Context<Self>) {
        let Some(entry) = self.journal.get_mut(&transaction_id) else {
            return;
        };
        if !entry.undone {
            return;
        }
        entry.undone = false;
        let states = entry.after.clone();
        self.model.restore(states);
        self.refresh_highlights(cx);
        cx.notify();
    }

    fn synchronize_scroll(
        &mut self,
        source: MergePane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_scroll_events.remove(&source) || !self.synchronize_scrolling {
            return;
        }
        let source_ranges = self.pane_line_ranges(source, cx);
        let source_line = self.editor(source).update(cx, |editor, cx| {
            let snapshot = editor.snapshot(window, cx);
            top_buffer_line(&snapshot.display_snapshot, snapshot.scroll_position().y)
        });
        for target in MergePane::ALL {
            if target == source {
                continue;
            }
            let target_ranges = self.pane_line_ranges(target, cx);
            let target_line = map_line(&source_ranges, &target_ranges, source_line);
            let scrolled = self.editor(target).update(cx, |editor, cx| {
                let snapshot = editor.snapshot(window, cx);
                let current = snapshot.scroll_position();
                let row = display_row_for_buffer_line(&snapshot.display_snapshot, target_line);
                editor.set_scroll_position(point(current.x, row), window, cx);
                editor.scroll_position(cx) != current
            });
            if scrolled {
                self.pending_scroll_events.insert(target);
            }
        }
    }

    fn set_synchronize_scrolling(&mut self, synchronize_scrolling: bool, cx: &mut Context<Self>) {
        self.synchronize_scrolling = synchronize_scrolling;
        self.pending_scroll_events.clear();
        cx.notify();
    }

    fn can_change_policy(&self, cx: &App) -> bool {
        self.journal.is_empty()
            && self
                .model
                .chunk_states()
                .iter()
                .all(|state| *state == ChunkState::default())
            && self.result_text(cx) == self.model.merge().text(Revision::Base)
    }

    pub(super) fn set_ignore_policy(
        &mut self,
        policy: IgnorePolicy,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if policy == self.model.merge().policy() || !self.can_change_policy(cx) {
            return;
        }
        let current = self.model.merge();
        let merge = ThreeWayMerge::new(
            current.text(Revision::Base),
            current.text(Revision::Left),
            current.text(Revision::Right),
            policy,
        );
        let tracker = ResultTracker::new(&merge);
        let snapshot = self.result_snapshot(cx);
        self.chunk_anchors = tracker
            .chunk_ranges()
            .iter()
            .map(|range| anchor_range(&snapshot, range.clone()))
            .collect();
        self.model = MergeModel::new(merge);
        self.refresh_highlights(cx);
        cx.notify();
    }

    fn has_pending_non_conflicting(&self, side: Side) -> bool {
        self.model
            .chunks()
            .iter()
            .enumerate()
            .any(|(chunk_index, chunk)| {
                !chunk.is_conflict() && self.model.is_pending(chunk_index, side)
            })
    }

    fn has_simple_conflicts(&self) -> bool {
        (0..self.model.chunks().len()).any(|chunk_index| self.model.is_simple_conflict(chunk_index))
    }

    fn apply_side(
        &mut self,
        chunk_index: usize,
        side: Side,
        mode: ApplyMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current_text = self.result_chunk_text(chunk_index, cx);
        let before = self.model.chunk_states();
        let edit = self.model.apply(chunk_index, side, mode, &current_text);
        self.commit_operation(
            before,
            edit.into_iter().collect(),
            Some(chunk_index),
            window,
            cx,
        );
    }

    fn ignore_sides(
        &mut self,
        chunk_index: usize,
        sides: &[Side],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.model.chunk_states();
        let mut changed = false;
        for side in sides {
            changed |= self.model.ignore(chunk_index, *side);
        }
        if changed {
            self.commit_operation(before, Vec::new(), Some(chunk_index), window, cx);
        }
    }

    fn resolve_using(
        &mut self,
        chunk_index: usize,
        side: Side,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.model.chunk_states();
        let edit = self.model.resolve_using(chunk_index, side);
        self.commit_operation(
            before,
            edit.into_iter().collect(),
            Some(chunk_index),
            window,
            cx,
        );
    }

    pub(crate) fn resolve_simple_chunk(
        &mut self,
        chunk_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.model.chunk_states();
        let edit = self.model.resolve_simple(chunk_index);
        self.commit_operation(
            before,
            edit.into_iter().collect(),
            Some(chunk_index),
            window,
            cx,
        );
    }

    fn apply_non_conflicting(
        &mut self,
        scope: NonConflictingScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.model.chunk_states();
        let edits = self.model.apply_non_conflicting(scope);
        self.commit_operation(before, edits, None, window, cx);
    }

    fn resolve_all_simple_conflicts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let before = self.model.chunk_states();
        let edits = self.model.resolve_simple_conflicts();
        self.commit_operation(before, edits, None, window, cx);
    }

    pub(crate) fn accept_chunk_from_divider(
        &mut self,
        chunk_index: usize,
        side: Side,
        modifiers: Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if modifiers.platform {
            self.resolve_using(chunk_index, side, window, cx);
        } else if modifiers.alt {
            self.apply_side(chunk_index, side, ApplyMode::Append, window, cx);
        } else {
            self.apply_side(chunk_index, side, ApplyMode::Replace, window, cx);
        }
    }

    pub(crate) fn ignore_chunk_from_divider(
        &mut self,
        chunk_index: usize,
        side: Side,
        modifiers: Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if modifiers.platform {
            self.ignore_sides(chunk_index, &[Side::Left, Side::Right], window, cx);
        } else {
            self.ignore_sides(chunk_index, &[side], window, cx);
        }
    }

    fn commit_operation(
        &mut self,
        before: Vec<ChunkState>,
        edits: Vec<ChunkEdit>,
        touched_chunk: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let after = self.model.chunk_states();
        if after == before && edits.is_empty() {
            return;
        }
        let snapshot = self.result_snapshot(cx);
        let buffer_edits = edits
            .into_iter()
            .filter_map(|edit| {
                let range = self.chunk_anchors.get(edit.chunk_index)?;
                Some((
                    range.start.to_offset(&snapshot)..range.end.to_offset(&snapshot),
                    edit.new_text,
                ))
            })
            .collect::<Vec<_>>();
        let touched_range = touched_chunk
            .filter(|_| buffer_edits.is_empty())
            .and_then(|chunk_index| self.chunk_anchors.get(chunk_index))
            .map(|range| range.start.to_offset(&snapshot)..range.end.to_offset(&snapshot));
        let transaction_id = self.result_editor.update(cx, |editor, cx| {
            editor.finalize_last_transaction(cx);
            let transaction_id = editor.transact(window, cx, |editor, _, cx| {
                if !buffer_edits.is_empty() {
                    editor.edit(buffer_edits, cx);
                } else if let Some(range) = touched_range {
                    touch_range(editor, range, &snapshot, cx);
                }
            });
            editor.finalize_last_transaction(cx);
            transaction_id
        });
        if let Some(transaction_id) = transaction_id {
            self.journal.retain(|_, entry| !entry.undone);
            self.journal.insert(
                transaction_id,
                JournalEntry {
                    before,
                    after,
                    undone: false,
                },
            );
        }
        self.refresh_highlights(cx);
        cx.notify();
    }

    fn refresh_highlights(&self, cx: &mut Context<Self>) {
        let mut left_rows = Vec::new();
        let mut right_rows = Vec::new();
        let mut result_rows = Vec::new();
        for (chunk_index, chunk) in self.model.chunks().iter().enumerate() {
            if self.model.is_pending(chunk_index, Side::Left) {
                left_rows.push((chunk.left.clone(), side_change_kind(chunk, Side::Left)));
            }
            if self.model.is_pending(chunk_index, Side::Right) {
                right_rows.push((chunk.right.clone(), side_change_kind(chunk, Side::Right)));
            }
            if !self.model.is_resolved(chunk_index)
                && let Some(range) = self.chunk_anchors.get(chunk_index)
            {
                result_rows.push((range.clone(), result_change_kind(chunk)));
            }
        }
        highlight_lines(&self.left_editor, left_rows, cx);
        highlight_lines(&self.right_editor, right_rows, cx);
        self.result_editor.update(cx, |editor, cx| {
            highlight_chunk_rows(editor, result_rows, cx);
        });
    }

    fn with_cursor_chunk(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        operation: impl FnOnce(&mut Self, usize, &mut Window, &mut Context<Self>),
    ) {
        if let Some(chunk_index) = self.chunk_at_cursor(cx) {
            operation(self, chunk_index, window, cx);
        }
    }

    fn navigate(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let lines = self.result_line_ranges(cx);
        let cursor_line = self.cursor_line(cx);
        let target = if forward {
            self.model.next_unresolved(&lines, cursor_line)
        } else {
            self.model.previous_unresolved(&lines, cursor_line)
        };
        let Some(range) = target.and_then(|chunk_index| self.chunk_anchors.get(chunk_index)) else {
            return;
        };
        let start = range.start;
        self.result_editor.update(cx, |editor, cx| {
            editor.change_selections(
                SelectionEffects::scroll(Autoscroll::center()),
                window,
                cx,
                |selections| {
                    selections.select_anchor_ranges([start..start]);
                },
            );
        });
    }

    fn next_difference(&mut self, _: &NextDifference, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(true, window, cx);
    }

    fn previous_difference(
        &mut self,
        _: &PreviousDifference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate(false, window, cx);
    }

    fn accept_left_side(
        &mut self,
        _: &AcceptLeftSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_cursor_chunk(window, cx, |this, chunk_index, window, cx| {
            this.apply_side(chunk_index, Side::Left, ApplyMode::Replace, window, cx);
        });
    }

    fn accept_right_side(
        &mut self,
        _: &AcceptRightSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_cursor_chunk(window, cx, |this, chunk_index, window, cx| {
            this.apply_side(chunk_index, Side::Right, ApplyMode::Replace, window, cx);
        });
    }

    fn append_left_side(
        &mut self,
        _: &AppendLeftSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_cursor_chunk(window, cx, |this, chunk_index, window, cx| {
            this.apply_side(chunk_index, Side::Left, ApplyMode::Append, window, cx);
        });
    }

    fn append_right_side(
        &mut self,
        _: &AppendRightSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_cursor_chunk(window, cx, |this, chunk_index, window, cx| {
            this.apply_side(chunk_index, Side::Right, ApplyMode::Append, window, cx);
        });
    }

    fn ignore_left_side(
        &mut self,
        _: &IgnoreLeftSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_cursor_chunk(window, cx, |this, chunk_index, window, cx| {
            this.ignore_sides(chunk_index, &[Side::Left], window, cx);
        });
    }

    fn ignore_right_side(
        &mut self,
        _: &IgnoreRightSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_cursor_chunk(window, cx, |this, chunk_index, window, cx| {
            this.ignore_sides(chunk_index, &[Side::Right], window, cx);
        });
    }

    fn resolve_using_left(
        &mut self,
        _: &ResolveUsingLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_cursor_chunk(window, cx, |this, chunk_index, window, cx| {
            this.resolve_using(chunk_index, Side::Left, window, cx);
        });
    }

    fn resolve_using_right(
        &mut self,
        _: &ResolveUsingRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_cursor_chunk(window, cx, |this, chunk_index, window, cx| {
            this.resolve_using(chunk_index, Side::Right, window, cx);
        });
    }

    fn resolve_simple_conflict(
        &mut self,
        _: &ResolveSimpleConflict,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_cursor_chunk(window, cx, |this, chunk_index, window, cx| {
            this.resolve_simple_chunk(chunk_index, window, cx);
        });
    }

    fn apply_non_conflicting_left(
        &mut self,
        _: &ApplyNonConflictingLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_non_conflicting(NonConflictingScope::Left, window, cx);
    }

    fn apply_non_conflicting_right(
        &mut self,
        _: &ApplyNonConflictingRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_non_conflicting(NonConflictingScope::Right, window, cx);
    }

    fn apply_non_conflicting_all(
        &mut self,
        _: &ApplyNonConflictingAll,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_non_conflicting(NonConflictingScope::All, window, cx);
    }

    fn resolve_simple_conflicts(
        &mut self,
        _: &ResolveSimpleConflicts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.resolve_all_simple_conflicts(window, cx);
    }

    fn toggle_synchronize_scrolling(
        &mut self,
        _: &ToggleSynchronizeScrolling,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_synchronize_scrolling(!self.synchronize_scrolling, cx);
    }

    fn focus_opposite_pane(
        &mut self,
        _: &FocusOppositePane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = if self.result_editor.read(cx).is_focused(window) {
            &self.left_editor
        } else {
            &self.result_editor
        };
        window.focus(&target.focus_handle(cx), cx);
    }

    fn show_settings(&mut self, _: &ShowSettings, window: &mut Window, cx: &mut Context<Self>) {
        let settings_menu_handle = self.settings_menu_handle.clone();
        window.defer(cx, move |window, cx| {
            settings_menu_handle.toggle(window, cx);
        });
    }

    fn accept_left(&mut self, _: &AcceptLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_with(self.stages.ours.clone(), window, cx);
    }

    fn accept_right(&mut self, _: &AcceptRight, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_with(self.stages.theirs.clone(), window, cx);
    }

    fn save_and_close(&mut self, _: &SaveAndClose, window: &mut Window, cx: &mut Context<Self>) {
        if self.finishing {
            return;
        }
        let document = self.document(cx);
        store_merge_session(self.repository_id, self.repo_path.clone(), document, cx);
        cx.emit(ItemEvent::CloseItem);
        self.show_conflicts_dialog_later(window, cx);
    }

    fn apply_changes(&mut self, _: &ApplyChanges, window: &mut Window, cx: &mut Context<Self>) {
        if self.finishing {
            return;
        }
        let counters = self.model.counters();
        if counters.is_complete() {
            self.apply_result(window, cx);
            return;
        }
        let detail = unresolved_changes_detail(&counters);
        let answer = window.prompt(
            PromptLevel::Warning,
            UNRESOLVED_PROMPT_MESSAGE,
            Some(&detail),
            &[APPLY_ANSWER, CANCEL_ANSWER],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await? == 0 {
                this.update_in(cx, |this, window, cx| this.apply_result(window, cx))?;
            }
            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
    }

    fn apply_result(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = resolved_text(&self.stages, self.result_text(cx));
        self.finish_with(text, window, cx);
    }

    fn finish_with(&mut self, text: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.finishing {
            return;
        }
        self.finishing = true;
        cx.notify();
        let write = write_merge_result(
            self.project.clone(),
            self.repository.clone(),
            self.repo_path.clone(),
            text,
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = write.await;
            this.update_in(cx, |this, window, cx| {
                this.finishing = false;
                cx.notify();
                if result.is_ok() {
                    this.open_next_conflict(window, cx);
                    cx.emit(ItemEvent::CloseItem);
                }
            })?;
            result
        })
        .detach_and_prompt_err(
            "Failed to apply the merge result",
            window,
            cx,
            |_, _, _| None,
        );
    }

    fn open_next_conflict(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next_path = next_conflicted_path(self.repository.read(cx), &self.repo_path);
        let Some(next_path) = next_path else {
            self.show_conflicts_dialog_later(window, cx);
            return;
        };
        let workspace = self.workspace.clone();
        let repository = self.repository.clone();
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |workspace, cx| {
                    open_merge_tool(workspace, repository, next_path, window, cx)
                        .detach_and_prompt_err(
                            "Failed to open the merge tool",
                            window,
                            cx,
                            |_, _, _| None,
                        );
                })
                .log_err();
        });
    }

    fn show_conflicts_dialog_later(&self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.workspace.clone();
        let repository = self.repository.clone();
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |workspace, cx| {
                    show_conflicts_dialog(workspace, repository, window, cx);
                })
                .log_err();
        });
    }

    fn divider_chunks(&self, side: Side, result_lines: &[Range<u32>]) -> Vec<DividerChunk> {
        self.model
            .chunks()
            .iter()
            .enumerate()
            .filter(|(chunk_index, _)| self.model.is_pending(*chunk_index, side))
            .filter_map(|(chunk_index, chunk)| {
                Some(DividerChunk {
                    chunk_index,
                    side_lines: chunk.lines(side.revision()),
                    result_lines: result_lines.get(chunk_index)?.clone(),
                    kind: side_change_kind(chunk, side),
                    simple_conflict: side == Side::Left
                        && self.model.is_simple_conflict(chunk_index),
                })
            })
            .collect()
    }

    fn render_toolbar(&self, counters: &Counters, cx: &mut Context<Self>) -> impl IntoElement {
        let has_left_changes = self.has_pending_non_conflicting(Side::Left);
        let has_right_changes = self.has_pending_non_conflicting(Side::Right);
        let has_simple_conflicts = self.has_simple_conflicts();
        h_flex()
            .w_full()
            .flex_none()
            .px_2()
            .py_1()
            .gap_1()
            .border_b_1()
            .border_color(cx.theme().colors().border_variant)
            .child(
                IconButton::new("merge-previous-difference", IconName::ArrowUp)
                    .icon_size(IconSize::Small)
                    .tooltip(Tooltip::for_action_title(
                        "Previous Difference",
                        &PreviousDifference,
                    ))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.navigate(false, window, cx);
                    })),
            )
            .child(
                IconButton::new("merge-next-difference", IconName::ArrowDown)
                    .icon_size(IconSize::Small)
                    .tooltip(Tooltip::for_action_title(
                        "Next Difference",
                        &NextDifference,
                    ))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.navigate(true, window, cx);
                    })),
            )
            .child(Divider::vertical())
            .child(
                Label::new("Apply non-conflicting changes:")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(
                Button::new("merge-apply-non-conflicting-left", "Left")
                    .label_size(LabelSize::Small)
                    .start_icon(Icon::new(IconName::ChevronsRight).size(IconSize::XSmall))
                    .disabled(!has_left_changes)
                    .tooltip(Tooltip::for_action_title(
                        "Apply Non-Conflicting Changes from the Left Side",
                        &ApplyNonConflictingLeft,
                    ))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.apply_non_conflicting(NonConflictingScope::Left, window, cx);
                    })),
            )
            .child(
                Button::new("merge-apply-non-conflicting-all", "All")
                    .label_size(LabelSize::Small)
                    .disabled(!has_left_changes && !has_right_changes)
                    .tooltip(Tooltip::for_action_title(
                        "Apply All Non-Conflicting Changes",
                        &ApplyNonConflictingAll,
                    ))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.apply_non_conflicting(NonConflictingScope::All, window, cx);
                    })),
            )
            .child(
                Button::new("merge-apply-non-conflicting-right", "Right")
                    .label_size(LabelSize::Small)
                    .start_icon(Icon::new(IconName::ChevronsLeft).size(IconSize::XSmall))
                    .disabled(!has_right_changes)
                    .tooltip(Tooltip::for_action_title(
                        "Apply Non-Conflicting Changes from the Right Side",
                        &ApplyNonConflictingRight,
                    ))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.apply_non_conflicting(NonConflictingScope::Right, window, cx);
                    })),
            )
            .child(Divider::vertical())
            .child(
                IconButton::new("merge-resolve-simple-conflicts", IconName::Wand)
                    .icon_size(IconSize::Small)
                    .disabled(!has_simple_conflicts)
                    .tooltip(Tooltip::for_action_title(
                        "Resolve Simple Conflicts",
                        &ResolveSimpleConflicts,
                    ))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.resolve_all_simple_conflicts(window, cx);
                    })),
            )
            .child(self.render_settings_menu(cx))
            .child(div().flex_1())
            .child(Label::new(counters.status_text()).size(LabelSize::Small))
    }

    fn render_settings_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let merge_view = cx.weak_entity();
        PopoverMenu::new("merge-tool-settings")
            .trigger_with_tooltip(
                IconButton::new("merge-tool-settings-trigger", IconName::Settings)
                    .icon_size(IconSize::Small),
                Tooltip::for_action_title("Settings", &ShowSettings),
            )
            .anchor(gpui::Anchor::TopRight)
            .with_handle(self.settings_menu_handle.clone())
            .menu(move |window, cx| {
                let merge_view = merge_view.upgrade()?;
                Some(build_settings_menu(merge_view, window, cx))
            })
    }

    fn render_pane_header(&self, pane: MergePane, cx: &mut Context<Self>) -> impl IntoElement {
        let header = h_flex()
            .w_full()
            .flex_none()
            .px_2()
            .py_1()
            .gap_1()
            .overflow_hidden()
            .border_b_1()
            .border_color(cx.theme().colors().border_variant);
        let lock = || {
            Icon::new(IconName::Lock)
                .size(IconSize::XSmall)
                .color(Color::Muted)
        };
        match pane {
            MergePane::Left => header.child(lock()).child(
                Label::new(format!("Changes from {}", self.branches.ours))
                    .size(LabelSize::Small)
                    .truncate(),
            ),
            MergePane::Result => header
                .child(Label::new("Result").size(LabelSize::Small))
                .child(
                    Label::new(self.repo_path.display(PathStyle::local()).into_owned())
                        .size(LabelSize::Small)
                        .color(Color::Muted)
                        .truncate(),
                ),
            MergePane::Right => header.child(lock()).child(
                Label::new(format!("Changes from {}", self.branches.theirs))
                    .size(LabelSize::Small)
                    .truncate(),
            ),
        }
    }

    fn render_pane(&self, pane: MergePane, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(self.render_pane_header(pane, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(self.editor(pane).clone()),
            )
    }

    fn render_bottom_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let finishing = self.finishing;
        h_flex()
            .w_full()
            .flex_none()
            .justify_between()
            .px_2()
            .py_1()
            .border_t_1()
            .border_color(cx.theme().colors().border_variant)
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("merge-accept-left", "Accept Left")
                            .disabled(finishing)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.accept_left(&AcceptLeft, window, cx);
                            })),
                    )
                    .child(
                        Button::new("merge-accept-right", "Accept Right")
                            .disabled(finishing)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.accept_right(&AcceptRight, window, cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("merge-save-and-close", "Save and Close")
                            .disabled(finishing)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_and_close(&SaveAndClose, window, cx);
                            })),
                    )
                    .child(
                        Button::new("merge-apply-changes", "Apply Changes")
                            .style(ButtonStyle::Filled)
                            .disabled(finishing)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.apply_changes(&ApplyChanges, window, cx);
                            })),
                    ),
            )
    }
}

impl EventEmitter<ItemEvent> for MergeView {}

impl Focusable for MergeView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.result_editor.focus_handle(cx)
    }
}

impl Item for MergeView {
    type Event = ItemEvent;

    fn tab_content_text(&self, _detail: usize, _cx: &App) -> SharedString {
        self.repo_path
            .file_name()
            .map(|file_name| SharedString::from(file_name.to_string()))
            .unwrap_or_else(|| {
                self.repo_path
                    .display(PathStyle::local())
                    .into_owned()
                    .into()
            })
    }

    fn tab_icon(&self, _window: &Window, _cx: &App) -> Option<Icon> {
        Some(Icon::new(IconName::GitMergeConflict).color(Color::Muted))
    }

    fn tab_tooltip_text(&self, _cx: &App) -> Option<SharedString> {
        Some(
            format!(
                "Merge Revisions for {}",
                self.repo_path.display(PathStyle::local())
            )
            .into(),
        )
    }

    fn to_item_events(event: &Self::Event, f: &mut dyn FnMut(ItemEvent)) {
        f(*event);
    }

    fn show_toolbar(&self) -> bool {
        false
    }
}

impl Render for MergeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let result_lines = self.result_line_ranges(cx);
        let counters = self.model.counters();
        let merge_view = cx.weak_entity();
        let left_divider = MergeDivider::new(
            Side::Left,
            merge_view.clone(),
            self.left_editor.clone(),
            self.result_editor.clone(),
            self.divider_chunks(Side::Left, &result_lines),
        );
        let right_divider = MergeDivider::new(
            Side::Right,
            merge_view,
            self.right_editor.clone(),
            self.result_editor.clone(),
            self.divider_chunks(Side::Right, &result_lines),
        );

        v_flex()
            .key_context("MergeView")
            .size_full()
            .bg(cx.theme().colors().editor_background)
            .on_action(cx.listener(Self::next_difference))
            .on_action(cx.listener(Self::previous_difference))
            .on_action(cx.listener(Self::accept_left_side))
            .on_action(cx.listener(Self::accept_right_side))
            .on_action(cx.listener(Self::append_left_side))
            .on_action(cx.listener(Self::append_right_side))
            .on_action(cx.listener(Self::ignore_left_side))
            .on_action(cx.listener(Self::ignore_right_side))
            .on_action(cx.listener(Self::resolve_using_left))
            .on_action(cx.listener(Self::resolve_using_right))
            .on_action(cx.listener(Self::resolve_simple_conflict))
            .on_action(cx.listener(Self::apply_non_conflicting_left))
            .on_action(cx.listener(Self::apply_non_conflicting_right))
            .on_action(cx.listener(Self::apply_non_conflicting_all))
            .on_action(cx.listener(Self::resolve_simple_conflicts))
            .on_action(cx.listener(Self::toggle_synchronize_scrolling))
            .on_action(cx.listener(Self::focus_opposite_pane))
            .on_action(cx.listener(Self::show_settings))
            .on_action(cx.listener(Self::accept_left))
            .on_action(cx.listener(Self::accept_right))
            .on_action(cx.listener(Self::save_and_close))
            .on_action(cx.listener(Self::apply_changes))
            .child(self.render_toolbar(&counters, cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        h_flex()
                            .size_full()
                            .child(self.render_pane(MergePane::Left, cx))
                            .child(div().flex_none().h_full().w(DIVIDER_WIDTH))
                            .child(self.render_pane(MergePane::Result, cx))
                            .child(div().flex_none().h_full().w(DIVIDER_WIDTH))
                            .child(self.render_pane(MergePane::Right, cx)),
                    )
                    .child(left_divider)
                    .child(right_divider),
            )
            .child(self.render_bottom_bar(cx))
    }
}

fn configure_pane_editor(editor: &mut Editor, cx: &mut Context<Editor>) {
    editor.set_should_serialize(false, cx);
    editor.set_show_git_diff_gutter(false, cx);
    editor.set_show_code_actions(false, cx);
    editor.set_show_runnables(false, cx);
    editor.set_show_breakpoints(false, cx);
}

fn install_result_context_menu(
    result_editor: &Entity<Editor>,
    merge_view: WeakEntity<MergeView>,
    cx: &mut App,
) {
    result_editor.update(cx, |editor, _| {
        editor.set_custom_context_menu(move |editor, display_point, window, cx| {
            let merge_view = merge_view.upgrade()?;
            let clicked = editor
                .display_snapshot(cx)
                .display_point_to_point(display_point, Bias::Left);
            editor.change_selections(SelectionEffects::no_scroll(), window, cx, |selections| {
                selections.select_ranges([clicked..clicked]);
            });
            let snapshot = editor.buffer().read(cx).snapshot(cx);
            let menu_state = ChunkMenuState::new(merge_view.read(cx), clicked.row, &snapshot);
            let focus_handle = editor.focus_handle(cx);
            Some(build_chunk_menu(
                merge_view.downgrade(),
                menu_state,
                focus_handle,
                window,
                cx,
            ))
        });
    });
}

struct ChunkMenuState {
    chunk_index: Option<usize>,
    left_pending: bool,
    right_pending: bool,
    unresolved: bool,
    synchronize_scrolling: bool,
}

impl ChunkMenuState {
    fn new(merge_view: &MergeView, line: u32, snapshot: &MultiBufferSnapshot) -> Self {
        let chunk_index = merge_view.chunk_at_line(line, snapshot);
        let model = &merge_view.model;
        Self {
            chunk_index,
            left_pending: chunk_index
                .is_some_and(|chunk_index| model.is_pending(chunk_index, Side::Left)),
            right_pending: chunk_index
                .is_some_and(|chunk_index| model.is_pending(chunk_index, Side::Right)),
            unresolved: chunk_index.is_some_and(|chunk_index| !model.is_resolved(chunk_index)),
            synchronize_scrolling: merge_view.synchronize_scrolling,
        }
    }
}

fn build_chunk_menu(
    merge_view: WeakEntity<MergeView>,
    state: ChunkMenuState,
    focus_handle: FocusHandle,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ContextMenu> {
    ContextMenu::build(window, cx, move |menu, _, _| {
        let ignore_view = merge_view.clone();
        let chunk_index = state.chunk_index;
        let synchronize_scrolling = state.synchronize_scrolling;
        menu.context(focus_handle)
            .action_disabled_when(
                !state.left_pending,
                "Accept Left Side",
                AcceptLeftSide.boxed_clone(),
            )
            .action_disabled_when(
                !state.right_pending,
                "Accept Right Side",
                AcceptRightSide.boxed_clone(),
            )
            .action_disabled_when(
                !state.unresolved,
                "Resolve using Left",
                ResolveUsingLeft.boxed_clone(),
            )
            .action_disabled_when(
                !state.unresolved,
                "Resolve using Right",
                ResolveUsingRight.boxed_clone(),
            )
            .item(
                ContextMenuEntry::new("Ignore")
                    .disabled(!state.unresolved)
                    .handler(move |window, cx| {
                        let Some(chunk_index) = chunk_index else {
                            return;
                        };
                        ignore_view
                            .update(cx, |merge_view, cx| {
                                merge_view.ignore_sides(
                                    chunk_index,
                                    &[Side::Left, Side::Right],
                                    window,
                                    cx,
                                );
                            })
                            .log_err();
                    }),
            )
            .separator()
            .toggleable_entry(
                "Synchronize Scrolling",
                synchronize_scrolling,
                IconPosition::Start,
                Some(ToggleSynchronizeScrolling.boxed_clone()),
                move |_, cx| {
                    merge_view
                        .update(cx, |merge_view, cx| {
                            merge_view.set_synchronize_scrolling(!synchronize_scrolling, cx);
                        })
                        .log_err();
                },
            )
    })
}

fn build_settings_menu(
    merge_view: Entity<MergeView>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ContextMenu> {
    let (synchronize_scrolling, current_policy, can_change_policy) = {
        let merge_view = merge_view.read(cx);
        (
            merge_view.synchronize_scrolling,
            merge_view.model.merge().policy(),
            merge_view.can_change_policy(cx),
        )
    };
    let merge_view = merge_view.downgrade();
    ContextMenu::build(window, cx, move |menu, _, _| {
        let scrolling_view = merge_view.clone();
        let mut menu = menu
            .toggleable_entry(
                "Synchronize Scrolling",
                synchronize_scrolling,
                IconPosition::Start,
                Some(ToggleSynchronizeScrolling.boxed_clone()),
                move |_, cx| {
                    scrolling_view
                        .update(cx, |merge_view, cx| {
                            merge_view.set_synchronize_scrolling(!synchronize_scrolling, cx);
                        })
                        .log_err();
                },
            )
            .separator()
            .header("Ignore Differences");
        for policy in IgnorePolicy::ALL {
            let policy_view = merge_view.clone();
            menu = menu.toggleable_entry_disabled_when(
                policy.label(),
                policy == current_policy,
                !can_change_policy && policy != current_policy,
                IconPosition::Start,
                None,
                move |window, cx| {
                    policy_view
                        .update(cx, |merge_view, cx| {
                            merge_view.set_ignore_policy(policy, window, cx);
                        })
                        .log_err();
                },
            );
        }
        menu
    })
}

fn touch_range(
    editor: &mut Editor,
    range: Range<MultiBufferOffset>,
    snapshot: &MultiBufferSnapshot,
    cx: &mut Context<Editor>,
) {
    if range.is_empty() {
        let start = range.start;
        editor.edit([(start..start, TOUCH_TEXT)], cx);
        editor.edit(
            [(start..MultiBufferOffset(start.0 + TOUCH_TEXT.len()), "")],
            cx,
        );
    } else {
        let text = snapshot.text_for_range(range.clone()).collect::<String>();
        editor.edit([(range, text)], cx);
    }
}

fn anchor_range(snapshot: &MultiBufferSnapshot, range: Range<usize>) -> Range<Anchor> {
    let length = snapshot.len().0;
    let start = range.start.min(length);
    let end = range.end.clamp(start, length);
    snapshot.anchor_before(MultiBufferOffset(start))..snapshot.anchor_after(MultiBufferOffset(end))
}

fn anchor_line_range(snapshot: &MultiBufferSnapshot, range: &Range<Anchor>) -> Range<u32> {
    let start = range.start.to_point(snapshot);
    let end = range.end.to_point(snapshot);
    if end <= start {
        return start.row..start.row;
    }
    start.row..end.row + u32::from(end.column > 0)
}

fn line_anchor_range(snapshot: &MultiBufferSnapshot, lines: Range<u32>) -> Range<Anchor> {
    let max_point = snapshot.max_point();
    let line_start = |line: u32| {
        if line > max_point.row {
            max_point
        } else {
            Point::new(line, 0)
        }
    };
    snapshot.anchor_before(line_start(lines.start))..snapshot.anchor_before(line_start(lines.end))
}

fn top_buffer_line(snapshot: &DisplaySnapshot, scroll_top: f64) -> f64 {
    let scroll_top = scroll_top.max(0.);
    let row = scroll_top.floor();
    let buffer_point =
        snapshot.display_point_to_point(DisplayPoint::new(DisplayRow(row as u32), 0), Bias::Left);
    f64::from(buffer_point.row) + (scroll_top - row)
}

fn display_row_for_buffer_line(snapshot: &DisplaySnapshot, line: f64) -> f64 {
    let line = line.max(0.);
    let whole = line.floor();
    display_row_for_line(snapshot, whole as u32) + (line - whole)
}

fn side_change_kind(chunk: &Chunk, side: Side) -> ChangeKind {
    if chunk.is_conflict() {
        return ChangeKind::Conflict;
    }
    match chunk.change_type(side) {
        Some(ChangeType::Added) => ChangeKind::Added,
        Some(ChangeType::Deleted) => ChangeKind::Deleted,
        Some(ChangeType::Modified) | None => ChangeKind::Modified,
    }
}

fn result_change_kind(chunk: &Chunk) -> ChangeKind {
    let side = if chunk.changes(Side::Left) {
        Side::Left
    } else {
        Side::Right
    };
    side_change_kind(chunk, side)
}

fn highlight_lines(editor: &Entity<Editor>, rows: Vec<(Range<u32>, ChangeKind)>, cx: &mut App) {
    editor.update(cx, |editor, cx| {
        let snapshot = editor.buffer().read(cx).snapshot(cx);
        let rows = rows
            .into_iter()
            .map(|(lines, kind)| (line_anchor_range(&snapshot, lines), kind))
            .collect();
        highlight_chunk_rows(editor, rows, cx);
    });
}

fn highlight_chunk_rows(
    editor: &mut Editor,
    rows: Vec<(Range<Anchor>, ChangeKind)>,
    cx: &mut Context<Editor>,
) {
    editor.clear_row_highlights::<MergeAddedRows>();
    editor.clear_row_highlights::<MergeModifiedRows>();
    editor.clear_row_highlights::<MergeDeletedRows>();
    editor.clear_row_highlights::<MergeConflictRows>();
    let options = RowHighlightOptions::default();
    for (range, kind) in rows {
        match kind {
            ChangeKind::Added => {
                editor.highlight_rows::<MergeAddedRows>(range, added_row_background, options, cx)
            }
            ChangeKind::Modified => editor.highlight_rows::<MergeModifiedRows>(
                range,
                modified_row_background,
                options,
                cx,
            ),
            ChangeKind::Deleted => editor.highlight_rows::<MergeDeletedRows>(
                range,
                deleted_row_background,
                options,
                cx,
            ),
            ChangeKind::Conflict => editor.highlight_rows::<MergeConflictRows>(
                range,
                conflict_row_background,
                options,
                cx,
            ),
        }
    }
    cx.notify();
}

fn added_row_background(cx: &App) -> Hsla {
    ChangeKind::Added.color(cx).opacity(ROW_HIGHLIGHT_OPACITY)
}

fn modified_row_background(cx: &App) -> Hsla {
    ChangeKind::Modified
        .color(cx)
        .opacity(ROW_HIGHLIGHT_OPACITY)
}

fn deleted_row_background(cx: &App) -> Hsla {
    ChangeKind::Deleted.color(cx).opacity(ROW_HIGHLIGHT_OPACITY)
}

fn conflict_row_background(cx: &App) -> Hsla {
    ChangeKind::Conflict
        .color(cx)
        .opacity(ROW_HIGHLIGHT_OPACITY)
}

fn unresolved_changes_detail(counters: &Counters) -> String {
    format!(
        "{} {} and {} {} are not resolved. Apply the result anyway?",
        counters.changes,
        if counters.changes == 1 {
            "change"
        } else {
            "changes"
        },
        counters.conflicts,
        if counters.conflicts == 1 {
            "conflict"
        } else {
            "conflicts"
        },
    )
}

fn next_conflicted_path(snapshot: &RepositorySnapshot, current: &RepoPath) -> Option<RepoPath> {
    let conflicted = snapshot
        .status()
        .filter(|entry| entry.status.is_conflicted() && entry.repo_path != *current)
        .map(|entry| entry.repo_path)
        .collect::<Vec<_>>();
    conflicted
        .iter()
        .find(|path| *path > current)
        .or_else(|| conflicted.first())
        .cloned()
}
