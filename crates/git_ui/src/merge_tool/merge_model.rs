#[cfg(test)]
mod buffer_sync_tests;
pub(crate) mod change;
pub(crate) mod commands;
pub(crate) mod document_edit;
pub(crate) mod iterative_data_holder;
pub(crate) mod journal;
pub(crate) mod line_separator;
pub(crate) mod line_text;
pub(crate) mod messages;
pub(crate) mod operations;
pub(crate) mod range_model;
#[cfg(test)]
mod tests;

use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Subscription, Task};
use language::{
    Buffer, BufferEvent, Capability, Edit, Language, TextBufferSnapshot, TransactionId,
};
use merge_diff::{
    CancellationChecker, ComparisonError, ComparisonPolicy, ConflictKind, MergeConflictType,
    MergeRange, Side, ThreeSide,
};
use project::Project;
use std::{
    fmt,
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use change::ChangeState;
pub(crate) use change::{ChangeCounters, MergeChange};
use document_edit::{DocumentEvent, PlannedEdit, plan_modification};
use journal::UndoJournal;
use line_separator::{LineSeparator, PaneSeparators};
use line_text::{LineText, normalize_line_endings};
use messages::{MergeStatus, merge_status};
use range_model::{MergeRangeModel, to_index, to_line};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileConflictType {
    Default,
    AddedAdded,
    ModifiedDeleted,
    DeletedModified,
}

impl FileConflictType {
    fn is_modify_delete(self) -> bool {
        matches!(self, Self::ModifiedDeleted | Self::DeletedModified)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MergeTexts {
    pub(crate) left: String,
    pub(crate) base: String,
    pub(crate) right: String,
    pub(crate) line_separators: PaneSeparators,
}

impl MergeTexts {
    pub(crate) fn new(left: &str, base: &str, right: &str) -> Self {
        Self {
            line_separators: [
                LineSeparator::detect(left),
                LineSeparator::detect(base),
                LineSeparator::detect(right),
            ],
            left: normalize_line_endings(left),
            base: normalize_line_endings(base),
            right: normalize_line_endings(right),
        }
    }
}

struct SideTexts {
    left: LineText,
    base: LineText,
    right: LineText,
}

impl SideTexts {
    fn new(texts: &MergeTexts) -> Self {
        Self {
            left: LineText::new(texts.left.clone()),
            base: LineText::new(texts.base.clone()),
            right: LineText::new(texts.right.clone()),
        }
    }

    fn side(&self, side: ThreeSide) -> &LineText {
        match side {
            ThreeSide::Left => &self.left,
            ThreeSide::Base => &self.base,
            ThreeSide::Right => &self.right,
        }
    }
}

#[derive(Clone)]
pub(crate) struct MergeBuffers {
    pub(crate) left: Entity<Buffer>,
    pub(crate) result: Entity<Buffer>,
    pub(crate) right: Entity<Buffer>,
}

impl MergeBuffers {
    pub(crate) fn in_project(
        texts: &MergeTexts,
        language: Option<Arc<Language>>,
        project: &Entity<Project>,
        cx: &mut App,
    ) -> Self {
        let create = |text: &str, cx: &mut App| {
            project.update(cx, |project, cx| {
                project.create_local_buffer(text, language.clone(), false, cx)
            })
        };
        let left = create(&texts.left, cx);
        let result = create(&texts.base, cx);
        let right = create(&texts.right, cx);
        Self::into_read_only_sides(left, result, right, cx)
    }

    #[cfg(test)]
    pub(crate) fn detached(
        texts: &MergeTexts,
        language: Option<Arc<Language>>,
        cx: &mut App,
    ) -> Self {
        let create = |text: &str, cx: &mut App| {
            cx.new(|cx| {
                let mut buffer = Buffer::local(text, cx);
                buffer.set_language(language.clone(), cx);
                buffer
            })
        };
        let left = create(&texts.left, cx);
        let result = create(&texts.base, cx);
        let right = create(&texts.right, cx);
        Self::into_read_only_sides(left, result, right, cx)
    }

    fn into_read_only_sides(
        left: Entity<Buffer>,
        result: Entity<Buffer>,
        right: Entity<Buffer>,
        cx: &mut App,
    ) -> Self {
        for side_buffer in [&left, &right] {
            side_buffer.update(cx, |buffer, cx| {
                buffer.set_capability(Capability::ReadOnly, cx)
            });
        }
        Self {
            left,
            result,
            right,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct MergeDiffData {
    pub(crate) fragments: Vec<MergeRange>,
    pub(crate) conflict_types: Vec<MergeConflictType>,
    pub(crate) ignore_policy: ComparisonPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MergeModelError {
    DiffTooBig,
    Canceled,
    ReadOnlyResult,
    InconsistentFragments,
}

impl fmt::Display for MergeModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DiffTooBig => formatter.write_str(
                "Unable to calculate diff. File is too big and there are too many changes.",
            ),
            Self::Canceled => formatter.write_str("Unable to calculate diff"),
            Self::ReadOnlyResult => {
                formatter.write_str("Cannot resolve conflicts in a read-only file")
            }
            Self::InconsistentFragments => formatter.write_str("Unable to calculate diff"),
        }
    }
}

impl std::error::Error for MergeModelError {}

impl From<ComparisonError> for MergeModelError {
    fn from(error: ComparisonError) -> Self {
        match error {
            ComparisonError::DiffTooBig => Self::DiffTooBig,
            ComparisonError::Canceled => Self::Canceled,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MergeModelEvent {
    Rediffed,
    ChangeResolved(usize),
    ChangeSideResolved { index: usize, side: Side },
    ChangeReset(usize),
    ChangeProcessed(usize),
    BulkProcessingFinished,
    ContentModifiedChanged(bool),
}

struct SharedCancellation(Arc<AtomicBool>);

impl CancellationChecker for SharedCancellation {
    fn is_canceled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone)]
struct SyncedBuffer {
    snapshot: TextBufferSnapshot,
    undo_top: Option<TransactionId>,
    redo_top: Option<TransactionId>,
}

impl SyncedBuffer {
    fn capture(buffer: &Buffer) -> Self {
        Self {
            snapshot: buffer.text_snapshot(),
            undo_top: buffer.peek_undo_stack().map(|entry| entry.transaction_id()),
            redo_top: buffer.peek_redo_stack().map(|entry| entry.transaction_id()),
        }
    }

    fn differs_from(&self, other: &Self) -> bool {
        self.snapshot.version != other.snapshot.version
            || self.undo_top != other.undo_top
            || self.redo_top != other.redo_top
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BufferTransition {
    Edit,
    Undo(TransactionId),
    Redo(TransactionId),
}

impl BufferTransition {
    fn classify(previous: &SyncedBuffer, current: &SyncedBuffer) -> Self {
        if let Some(undone) = current.redo_top
            && current.redo_top != previous.redo_top
            && current.redo_top == previous.undo_top
        {
            return Self::Undo(undone);
        }
        if let Some(redone) = current.undo_top
            && current.undo_top != previous.undo_top
            && current.undo_top == previous.redo_top
        {
            return Self::Redo(redone);
        }
        Self::Edit
    }
}

pub(crate) struct MergeConflictModel {
    buffers: MergeBuffers,
    sides: Arc<SideTexts>,
    conflict_type: FileConflictType,
    line_separators: PaneSeparators,
    ranges: MergeRangeModel,
    changes: Vec<MergeChange>,
    result: LineText,
    synced: SyncedBuffer,
    journal: UndoJournal,
    precalculated: Option<Arc<MergeDiffData>>,
    ignore_policy: ComparisonPolicy,
    initialized: bool,
    inside_command: bool,
    content_modified: bool,
    was_reviewed: bool,
    chosen_side: Option<Side>,
    rediff_generation: u64,
    rediff_cancellation: Option<Arc<AtomicBool>>,
    _result_subscription: Subscription,
}

impl EventEmitter<MergeModelEvent> for MergeConflictModel {}

impl MergeConflictModel {
    pub(crate) fn new(
        buffers: MergeBuffers,
        texts: &MergeTexts,
        conflict_type: FileConflictType,
        cx: &mut Context<Self>,
    ) -> Self {
        let synced = SyncedBuffer::capture(buffers.result.read(cx));
        let result = LineText::new(synced.snapshot.text());
        let result_subscription = cx.subscribe(
            &buffers.result,
            |model: &mut Self,
             _buffer: Entity<Buffer>,
             event: &BufferEvent,
             cx: &mut Context<Self>| {
                model.handle_result_buffer_event(event, cx);
            },
        );
        Self {
            buffers,
            sides: Arc::new(SideTexts::new(texts)),
            conflict_type,
            line_separators: texts.line_separators,
            ranges: MergeRangeModel::default(),
            changes: Vec::new(),
            result,
            synced,
            journal: UndoJournal::default(),
            precalculated: None,
            ignore_policy: ComparisonPolicy::Default,
            initialized: false,
            inside_command: false,
            content_modified: false,
            was_reviewed: false,
            chosen_side: None,
            rediff_generation: 0,
            rediff_cancellation: None,
            _result_subscription: result_subscription,
        }
    }

    pub(crate) fn buffers(&self) -> &MergeBuffers {
        &self.buffers
    }

    pub(crate) fn line_separators(&self) -> PaneSeparators {
        self.line_separators
    }

    pub(crate) fn ignore_policy(&self) -> ComparisonPolicy {
        self.ignore_policy
    }

    pub(crate) fn file_conflict_type(&self) -> FileConflictType {
        self.conflict_type
    }

    pub(crate) fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub(crate) fn side_text(&self, side: ThreeSide) -> &str {
        self.sides.side(side).text()
    }

    pub(crate) fn result_text(&self) -> &str {
        self.result.text()
    }

    pub(crate) fn current_result_text(&self, cx: &App) -> String {
        self.buffers.result.read(cx).text_snapshot().text()
    }

    pub(crate) fn changes(&self) -> &[MergeChange] {
        &self.changes
    }

    pub(crate) fn change(&self, index: usize) -> Option<&MergeChange> {
        self.changes.get(index)
    }

    pub(crate) fn result_lines(&self, index: usize) -> Range<usize> {
        to_index(self.ranges.line_start(index))..to_index(self.ranges.line_end(index))
    }

    pub(crate) fn start_line(&self, index: usize, side: ThreeSide) -> usize {
        match (side, self.changes.get(index)) {
            (ThreeSide::Base, _) => self.result_lines(index).start,
            (_, Some(change)) => change.fragment_lines(side).start,
            (_, None) => 0,
        }
    }

    pub(crate) fn end_line(&self, index: usize, side: ThreeSide) -> usize {
        match (side, self.changes.get(index)) {
            (ThreeSide::Base, _) => self.result_lines(index).end,
            (_, Some(change)) => change.fragment_lines(side).end,
            (_, None) => 0,
        }
    }

    pub(crate) fn unresolved_changes(&self) -> Vec<usize> {
        self.indices_where(|change| !change.is_resolved())
    }

    pub(crate) fn resolved_changes(&self) -> Vec<usize> {
        self.indices_where(MergeChange::is_resolved)
    }

    pub(crate) fn auto_resolvable_changes(&self) -> Vec<usize> {
        (0..self.changes.len())
            .filter(|index| self.can_resolve_change_automatically(*index, ThreeSide::Base))
            .collect()
    }

    pub(crate) fn has_non_conflicted_changes(&self, side: ThreeSide) -> bool {
        self.changes.iter().any(|change| {
            !change.is_conflict() && self.can_resolve_change_automatically(change.index(), side)
        })
    }

    pub(crate) fn has_auto_resolvable_conflicted_changes(&self) -> bool {
        (0..self.changes.len())
            .any(|index| self.can_resolve_change_automatically(index, ThreeSide::Base))
    }

    pub(crate) fn is_fully_resolved(&self) -> bool {
        self.changes.iter().all(MergeChange::is_resolved)
    }

    pub(crate) fn counters(&self) -> Option<ChangeCounters> {
        if !self.initialized {
            return None;
        }
        let mut counters = ChangeCounters::default();
        for change in self.changes.iter().filter(|change| !change.is_resolved()) {
            if change.is_conflict() {
                counters.conflicts += 1;
            } else {
                counters.changes += 1;
            }
        }
        Some(counters)
    }

    pub(crate) fn status(&self) -> MergeStatus {
        merge_status(self.counters())
    }

    pub(crate) fn content_modified(&self) -> bool {
        self.content_modified
    }

    pub(crate) fn was_reviewed(&self) -> bool {
        self.was_reviewed
    }

    pub(crate) fn mark_reviewed(&mut self) {
        self.was_reviewed = true;
    }

    pub(crate) fn chosen_side(&self) -> Option<Side> {
        self.chosen_side
    }

    pub(crate) fn set_chosen_side(&mut self, side: Option<Side>) {
        self.chosen_side = side;
    }

    fn indices_where(&self, predicate: impl Fn(&MergeChange) -> bool) -> Vec<usize> {
        self.changes
            .iter()
            .filter(|change| predicate(change))
            .map(MergeChange::index)
            .collect()
    }

    pub(crate) fn rediff(
        &mut self,
        ignore_policy: ComparisonPolicy,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), MergeModelError>> {
        if let Some(precalculated) = &self.precalculated
            && precalculated.ignore_policy == ignore_policy
        {
            return Task::ready(Ok(()));
        }

        self.precalculated = None;
        self.cancel_pending_rediff();
        self.rediff_generation += 1;
        let generation = self.rediff_generation;
        let cancellation = Arc::new(AtomicBool::new(false));
        self.rediff_cancellation = Some(cancellation.clone());

        let sides = self.sides.clone();
        let background_cancellation = cancellation.clone();
        let computation = cx.background_spawn(async move {
            compute_diff_data(
                &sides,
                ignore_policy,
                &SharedCancellation(background_cancellation),
            )
        });
        let cancel_when_dropped = CancelOnDrop(cancellation);
        cx.spawn(async move |model, cx| {
            let _cancel_when_dropped = cancel_when_dropped;
            let data = computation.await?;
            model
                .update(cx, |model, cx| {
                    if model.rediff_generation != generation {
                        return Err(MergeModelError::Canceled);
                    }
                    model.ignore_policy = ignore_policy;
                    model.apply_diff_data(Arc::new(data), cx)
                })
                .map_err(|_| MergeModelError::Canceled)?
        })
    }

    pub(crate) fn restart_with_policy(
        &mut self,
        ignore_policy: ComparisonPolicy,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), MergeModelError>> {
        self.precalculated = None;
        self.rediff(ignore_policy, cx)
    }

    fn cancel_pending_rediff(&mut self) {
        if let Some(cancellation) = self.rediff_cancellation.take() {
            cancellation.store(true, Ordering::Relaxed);
        }
    }

    fn apply_diff_data(
        &mut self,
        data: Arc<MergeDiffData>,
        cx: &mut Context<Self>,
    ) -> Result<(), MergeModelError> {
        if !self.is_result_writable(cx) {
            return Err(MergeModelError::ReadOnlyResult);
        }
        if data.fragments.len() != data.conflict_types.len() {
            return Err(MergeModelError::InconsistentFragments);
        }
        self.set_initial_output_content(cx);
        self.set_content_modified(false, cx);
        self.init_with_data(&data, cx);
        self.precalculated = Some(data);
        Ok(())
    }

    fn set_initial_output_content(&mut self, cx: &mut Context<Self>) {
        self.sync_with_buffer(cx);
        let base_text = self.sides.base.text().to_string();
        self.buffers.result.update(cx, |buffer, cx| {
            if buffer.text_snapshot().text() != base_text {
                buffer.set_text(base_text.as_str(), cx);
            }
            while let Some(transaction) =
                buffer.peek_undo_stack().map(|entry| entry.transaction_id())
            {
                if buffer.forget_transaction(transaction).is_none() {
                    break;
                }
            }
            while let Some(transaction) =
                buffer.peek_redo_stack().map(|entry| entry.transaction_id())
            {
                if buffer.forget_transaction(transaction).is_none() {
                    break;
                }
            }
        });
        self.result = LineText::new(base_text);
        self.journal.clear();
        self.synced = SyncedBuffer::capture(self.buffers.result.read(cx));
    }

    fn init_with_data(&mut self, data: &MergeDiffData, cx: &mut Context<Self>) {
        let ranges: Vec<(usize, usize)> = data
            .fragments
            .iter()
            .map(|fragment| (fragment.base.start, fragment.base.end))
            .collect();
        self.ranges.set_changes(&ranges);
        self.changes = data
            .fragments
            .iter()
            .zip(data.conflict_types.iter())
            .enumerate()
            .map(|(index, (fragment, conflict_type))| {
                MergeChange::new(index, fragment.clone(), *conflict_type)
            })
            .collect();
        self.initialized = true;
        cx.emit(MergeModelEvent::Rediffed);
    }

    fn is_result_writable(&self, cx: &App) -> bool {
        !self.buffers.result.read(cx).read_only()
    }

    fn set_content_modified(&mut self, value: bool, cx: &mut Context<Self>) {
        if self.content_modified != value {
            self.content_modified = value;
            cx.emit(MergeModelEvent::ContentModifiedChanged(value));
        }
    }

    fn handle_result_buffer_event(&mut self, event: &BufferEvent, cx: &mut Context<Self>) {
        match event {
            BufferEvent::Edited { .. } | BufferEvent::Operation { .. } => {
                self.sync_with_buffer(cx);
            }
            _ => {}
        }
    }

    fn sync_with_buffer(&mut self, cx: &mut Context<Self>) {
        let current = SyncedBuffer::capture(self.buffers.result.read(cx));
        if !self.synced.differs_from(&current) {
            return;
        }

        let transition = BufferTransition::classify(&self.synced, &current);
        if transition == BufferTransition::Edit && self.synced.redo_top.is_some() {
            self.journal.discard_undone();
        }
        let result_buffer = self.buffers.result.clone();
        self.journal.reconcile(|transaction| {
            result_buffer
                .read(cx)
                .get_transaction(transaction)
                .is_some()
        });

        let user_transaction = match transition {
            BufferTransition::Edit => current.undo_top,
            BufferTransition::Undo(_) | BufferTransition::Redo(_) => None,
        };
        if let Some(transaction) = user_transaction
            && !self.journal.has_entry(transaction)
        {
            let states = self.collect_states(&[]);
            self.journal.record_user_states(transaction, states);
        }

        let edits: Vec<Edit<usize>> = current
            .snapshot
            .edits_since::<usize>(&self.synced.snapshot.version)
            .collect();
        let new_text = current.snapshot.text();

        let mut replayed = self.result.len() == self.synced.snapshot.len();
        if replayed {
            for edit in &edits {
                if !self.replay_buffer_edit(edit, &new_text, cx) {
                    replayed = false;
                    break;
                }
            }
        }
        if !replayed || self.result.text() != new_text {
            log::error!("merge result mirror diverged from its buffer; resynchronizing");
            self.result = LineText::new(new_text);
        }

        self.synced = current;

        match transition {
            BufferTransition::Edit => {}
            BufferTransition::Undo(transaction) => {
                let states = self.journal.states_restored_by_undo(transaction);
                self.journal.mark_undone(transaction);
                self.restore_states(&states, cx);
            }
            BufferTransition::Redo(transaction) => {
                self.journal.mark_redone(transaction);
                let states = self.journal.states_restored_by_redo(transaction);
                self.restore_states(&states, cx);
            }
        }
    }

    fn replay_buffer_edit(
        &mut self,
        edit: &Edit<usize>,
        new_text: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let old_len = edit.old.end.saturating_sub(edit.old.start);
        let range = edit.new.start..edit.new.start + old_len;
        let Some(replacement) = new_text.get(edit.new.clone()) else {
            return false;
        };
        if !self.result.is_valid_range(&range) {
            return false;
        }
        let planned = PlannedEdit::from_buffer_edit(range.clone(), replacement.to_string());
        if let Some(event) = DocumentEvent::from_edit(self.result.text(), &planned) {
            self.process_document_event(&event, cx);
        }
        self.result.replace_range(range, replacement);
        true
    }

    fn process_document_event(&mut self, event: &DocumentEvent, cx: &mut Context<Self>) {
        self.enter_bulk_block();
        let modified = !event.whole_text_replaced && event.changes_content();
        self.set_content_modified(modified, cx);

        if !self.changes.is_empty() {
            let (line1, line2) = event.affected_line_range(&self.result);
            let shift = event.lines_shift();
            for index in 0..self.changes.len() {
                if self.process_document_change(index, line1, line2, shift, cx) {
                    self.invalidate_change(index, cx);
                }
            }
        }
        self.exit_bulk_block(cx);
    }

    fn process_document_change(
        &mut self,
        index: usize,
        old_line1: i32,
        old_line2: i32,
        shift: i32,
        cx: &mut Context<Self>,
    ) -> bool {
        let affected = self
            .ranges
            .process_document_change(index, old_line1, old_line2, shift);
        let emptied_deleted_change = self.changes.get(index).is_some_and(|change| {
            self.ranges.line_start(index) == self.ranges.line_end(index)
                && change.conflict_type.kind == ConflictKind::Deleted
                && !change.is_resolved()
        });
        if emptied_deleted_change {
            self.mark_change_resolved(index, cx);
        }
        affected
    }

    fn enter_bulk_block(&mut self) {
        self.ranges.enter_bulk();
    }

    fn exit_bulk_block(&mut self, cx: &mut Context<Self>) {
        if let Some(queued) = self.ranges.exit_bulk() {
            for index in queued {
                cx.emit(MergeModelEvent::ChangeProcessed(index));
            }
            cx.emit(MergeModelEvent::BulkProcessingFinished);
        }
    }

    fn invalidate_change(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.ranges.invalidate(index) {
            cx.emit(MergeModelEvent::ChangeProcessed(index));
        }
    }

    fn restore_states(&mut self, states: &[ChangeState], cx: &mut Context<Self>) {
        if self.changes.is_empty() {
            return;
        }
        self.enter_bulk_block();
        for state in states {
            self.restore_change_state(state, cx);
            self.invalidate_change(state.index, cx);
        }
        self.exit_bulk_block(cx);
    }

    fn restore_change_state(&mut self, state: &ChangeState, cx: &mut Context<Self>) {
        let Some(change) = self.changes.get_mut(state.index) else {
            return;
        };
        let was_resolved = change.is_resolved();
        change.restore_state(state);
        let is_resolved = change.is_resolved();
        self.ranges
            .set_range(state.index, state.start_line, state.end_line);
        if was_resolved != is_resolved {
            cx.emit(MergeModelEvent::ChangeResolved(state.index));
        }
    }

    fn collect_states(&self, affected: &[usize]) -> Vec<ChangeState> {
        let state_of = |index: usize| {
            self.changes.get(index).map(|change| {
                change.store_state(self.ranges.line_start(index), self.ranges.line_end(index))
            })
        };
        if affected.is_empty() {
            (0..self.changes.len()).filter_map(state_of).collect()
        } else {
            affected.iter().copied().filter_map(state_of).collect()
        }
    }

    pub(crate) fn execute_merge_command(
        &mut self,
        command_name: &str,
        affected_changes: Option<&[usize]>,
        cx: &mut Context<Self>,
        task: impl FnOnce(&mut Self, &mut Context<Self>),
    ) -> bool {
        if !self.is_result_writable(cx) {
            log::warn!("Result document is read-only; skipping merge command '{command_name}'");
            return false;
        }
        if self.inside_command {
            log::error!("Nested merge command '{command_name}' rejected");
            return false;
        }
        self.sync_with_buffer(cx);

        let all_affected = match affected_changes {
            Some(direct_changes) => self.ranges.collect_affected_changes(direct_changes),
            None => Vec::new(),
        };
        self.buffers.result.update(cx, |buffer, _| {
            buffer.finalize_last_transaction();
            buffer.start_transaction();
        });
        self.inside_command = true;
        self.enter_bulk_block();
        let states_before = self.collect_states(&all_affected);
        task(self, cx);
        let states_after = self.collect_states(&all_affected);
        self.exit_bulk_block(cx);
        self.inside_command = false;

        let transaction = self.finish_command_transaction(cx);
        self.journal.discard_undone();
        log::debug!("merge command '{command_name}' recorded as {transaction:?}");
        self.journal
            .record_command(transaction, states_before, states_after);
        self.synced = SyncedBuffer::capture(self.buffers.result.read(cx));
        self.set_content_modified(true, cx);
        true
    }

    fn finish_command_transaction(&mut self, cx: &mut Context<Self>) -> TransactionId {
        let edited_transaction = self.buffers.result.update(cx, |buffer, cx| {
            let transaction = buffer.end_transaction(cx);
            buffer.finalize_last_transaction();
            transaction
        });
        match edited_transaction {
            Some(transaction) => transaction,
            None => self.buffers.result.update(cx, |buffer, _| {
                while let Some(transaction) =
                    buffer.peek_redo_stack().map(|entry| entry.transaction_id())
                {
                    if buffer.forget_transaction(transaction).is_none() {
                        break;
                    }
                }
                let transaction = buffer.push_empty_transaction(Instant::now());
                buffer.finalize_last_transaction();
                transaction
            }),
        }
    }

    fn apply_modification(
        &mut self,
        start_line: usize,
        end_line: usize,
        new_lines: &[String],
        cx: &mut Context<Self>,
    ) {
        if let Some(edit) = plan_modification(&self.result, start_line, end_line, new_lines) {
            self.apply_planned_edit(&edit, cx);
        }
    }

    fn apply_planned_edit(&mut self, edit: &PlannedEdit, cx: &mut Context<Self>) {
        let Some(event) = DocumentEvent::from_edit(self.result.text(), edit) else {
            return;
        };
        self.process_document_event(&event, cx);
        if event.old_len != 0 || !event.new_text.is_empty() {
            let range = event.range();
            let replacement = event.new_text.clone();
            self.buffers.result.update(cx, |buffer, cx| {
                buffer.edit([(range, replacement)], None, cx);
            });
            self.result.replace_range(event.range(), &event.new_text);
        }
    }

    fn reset_output_content(&mut self, cx: &mut Context<Self>) {
        let edit = PlannedEdit::set_text(self.result.len(), self.sides.base.text().to_string());
        self.apply_planned_edit(&edit, cx);
    }

    fn replace_result_lines(
        &mut self,
        index: usize,
        new_content: &[String],
        cx: &mut Context<Self>,
    ) {
        let range = self.result_lines(index);
        self.apply_modification(range.start, range.end, new_content, cx);
        if range.start == range.end {
            self.move_changes_after_insertion(
                index,
                range.start,
                range.start + new_content.len(),
                cx,
            );
        }
    }

    fn append_result_lines(
        &mut self,
        index: usize,
        new_content: &[String],
        cx: &mut Context<Self>,
    ) {
        let range = self.result_lines(index);
        self.apply_modification(range.end, range.end, new_content, cx);
        self.move_changes_after_insertion(index, range.start, range.end + new_content.len(), cx);
    }

    fn move_changes_after_insertion(
        &mut self,
        index: usize,
        new_start_line: usize,
        new_end_line: usize,
        cx: &mut Context<Self>,
    ) {
        let changed = self.ranges.move_changes_after_insertion(
            index,
            to_line(new_start_line),
            to_line(new_end_line),
        );
        for changed_index in changed {
            self.invalidate_change(changed_index, cx);
        }
    }
}

fn compute_diff_data(
    sides: &SideTexts,
    ignore_policy: ComparisonPolicy,
    cancellation: &dyn CancellationChecker,
) -> Result<MergeDiffData, MergeModelError> {
    let fragments = merge_diff::merge_lines(
        sides.left.text(),
        sides.base.text(),
        sides.right.text(),
        ignore_policy,
        cancellation,
    )?;
    let conflict_types = conflict_types_for(&fragments, sides, ignore_policy)?;
    Ok(MergeDiffData {
        fragments,
        conflict_types,
        ignore_policy,
    })
}

fn conflict_types_for(
    fragments: &[MergeRange],
    sides: &SideTexts,
    ignore_policy: ComparisonPolicy,
) -> Result<Vec<MergeConflictType>, MergeModelError> {
    let left = merge_diff::LineTexts::new(sides.left.text());
    let base = merge_diff::LineTexts::new(sides.base.text());
    let right = merge_diff::LineTexts::new(sides.right.text());
    fragments
        .iter()
        .map(|fragment| {
            merge_diff::line_merge_type(fragment, [&left, &base, &right], ignore_policy)
                .ok_or(MergeModelError::InconsistentFragments)
        })
        .collect()
}
