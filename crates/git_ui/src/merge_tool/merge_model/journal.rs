use std::collections::{BTreeMap, HashMap};

use language::TransactionId;

use super::change::ChangeState;

#[derive(Default)]
struct JournalEntry {
    before: BTreeMap<usize, ChangeState>,
    after: Option<Vec<ChangeState>>,
    undone: bool,
}

#[derive(Default)]
pub(crate) struct UndoJournal {
    entries: HashMap<TransactionId, JournalEntry>,
    order: Vec<TransactionId>,
}

impl UndoJournal {
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    pub(crate) fn has_entry(&self, transaction: TransactionId) -> bool {
        self.entries.contains_key(&transaction)
    }

    pub(crate) fn record_command(
        &mut self,
        transaction: TransactionId,
        before: Vec<ChangeState>,
        after: Vec<ChangeState>,
    ) {
        let entry = self.entry_mut(transaction);
        for state in before {
            entry.before.entry(state.index).or_insert(state);
        }
        entry.after = Some(after);
    }

    pub(crate) fn record_user_states(
        &mut self,
        transaction: TransactionId,
        states: impl IntoIterator<Item = ChangeState>,
    ) {
        let entry = self.entry_mut(transaction);
        for state in states {
            entry.before.entry(state.index).or_insert(state);
        }
    }

    pub(crate) fn mark_undone(&mut self, transaction: TransactionId) {
        if let Some(entry) = self.entries.get_mut(&transaction) {
            entry.undone = true;
        }
    }

    pub(crate) fn mark_redone(&mut self, transaction: TransactionId) {
        if let Some(entry) = self.entries.get_mut(&transaction) {
            entry.undone = false;
        }
    }

    pub(crate) fn states_restored_by_undo(&self, transaction: TransactionId) -> Vec<ChangeState> {
        self.entries
            .get(&transaction)
            .map(|entry| entry.before.values().copied().collect())
            .unwrap_or_default()
    }

    pub(crate) fn states_restored_by_redo(&self, transaction: TransactionId) -> Vec<ChangeState> {
        self.entries
            .get(&transaction)
            .and_then(|entry| entry.after.clone())
            .unwrap_or_default()
    }

    pub(crate) fn discard_undone(&mut self) {
        let entries = &mut self.entries;
        entries.retain(|_, entry| !entry.undone);
        self.order
            .retain(|transaction| entries.contains_key(transaction));
    }

    pub(crate) fn reconcile(&mut self, exists: impl Fn(TransactionId) -> bool) {
        let mut position = 0;
        while position < self.order.len() {
            let transaction = self.order[position];
            if exists(transaction) {
                position += 1;
                continue;
            }
            self.order.remove(position);
            let absorbed = self.entries.remove(&transaction);
            let absorber = position
                .checked_sub(1)
                .and_then(|previous| self.order.get(previous))
                .copied();
            if let (Some(absorbed), Some(absorber)) = (absorbed, absorber)
                && let Some(target) = self.entries.get_mut(&absorber)
            {
                for (index, state) in absorbed.before {
                    target.before.entry(index).or_insert(state);
                }
            }
        }
    }

    fn entry_mut(&mut self, transaction: TransactionId) -> &mut JournalEntry {
        if !self.entries.contains_key(&transaction) {
            self.order.push(transaction);
        }
        self.entries.entry(transaction).or_default()
    }
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext as _, TestAppContext};
    use language::Buffer;

    use super::*;

    fn state(index: usize, start_line: i32, end_line: i32, resolved: bool) -> ChangeState {
        ChangeState {
            index,
            start_line,
            end_line,
            resolved: [resolved, resolved],
            oneside_applied_conflict: false,
        }
    }

    fn transactions(cx: &mut TestAppContext, count: usize) -> Vec<TransactionId> {
        let buffer = cx.new(|cx| Buffer::local("", cx));
        buffer.update(cx, |buffer, cx| {
            (0..count)
                .filter_map(|_| {
                    buffer.start_transaction();
                    buffer.edit([(0..0, "x")], None, cx);
                    let transaction = buffer.end_transaction(cx);
                    buffer.finalize_last_transaction();
                    transaction
                })
                .collect::<Vec<_>>()
        })
    }

    #[gpui::test]
    async fn user_states_keep_the_earliest_snapshot_of_every_change(cx: &mut TestAppContext) {
        let transaction = transactions(cx, 1)[0];
        let mut journal = UndoJournal::default();
        assert!(!journal.has_entry(transaction));

        journal.record_user_states(
            transaction,
            vec![state(0, 1, 2, false), state(1, 4, 5, false)],
        );
        journal.record_user_states(transaction, vec![state(0, 2, 3, true)]);
        assert!(journal.has_entry(transaction));

        assert_eq!(
            journal.states_restored_by_undo(transaction),
            vec![state(0, 1, 2, false), state(1, 4, 5, false)]
        );
        assert!(journal.states_restored_by_redo(transaction).is_empty());
    }

    #[gpui::test]
    async fn commands_restore_their_before_states_on_undo_and_after_states_on_redo(
        cx: &mut TestAppContext,
    ) {
        let transaction = transactions(cx, 1)[0];
        let mut journal = UndoJournal::default();

        journal.record_command(
            transaction,
            vec![state(2, 5, 6, false)],
            vec![state(2, 5, 8, true)],
        );

        assert_eq!(
            journal.states_restored_by_undo(transaction),
            vec![state(2, 5, 6, false)]
        );
        assert_eq!(
            journal.states_restored_by_redo(transaction),
            vec![state(2, 5, 8, true)]
        );
    }

    #[gpui::test]
    async fn unknown_transactions_restore_nothing(cx: &mut TestAppContext) {
        let transactions = transactions(cx, 2);
        let mut journal = UndoJournal::default();
        journal.record_user_states(transactions[0], vec![state(0, 1, 2, false)]);

        assert!(journal.states_restored_by_undo(transactions[1]).is_empty());
        assert!(journal.states_restored_by_redo(transactions[1]).is_empty());
    }

    #[gpui::test]
    async fn discarding_undone_entries_keeps_the_redone_and_untouched_ones(
        cx: &mut TestAppContext,
    ) {
        let transactions = transactions(cx, 3);
        let mut journal = UndoJournal::default();
        for (position, transaction) in transactions.iter().enumerate() {
            journal.record_user_states(*transaction, vec![state(position, 1, 2, false)]);
        }
        journal.mark_undone(transactions[1]);
        journal.mark_undone(transactions[2]);
        journal.mark_redone(transactions[2]);

        journal.discard_undone();

        assert_eq!(
            journal.states_restored_by_undo(transactions[0]),
            vec![state(0, 1, 2, false)]
        );
        assert!(journal.states_restored_by_undo(transactions[1]).is_empty());
        assert_eq!(
            journal.states_restored_by_undo(transactions[2]),
            vec![state(2, 1, 2, false)]
        );
    }

    #[gpui::test]
    async fn absorbed_transactions_hand_their_missing_states_to_the_previous_entry(
        cx: &mut TestAppContext,
    ) {
        let transactions = transactions(cx, 3);
        let mut journal = UndoJournal::default();
        journal.record_user_states(transactions[0], vec![state(0, 1, 2, false)]);
        journal.record_user_states(
            transactions[1],
            vec![state(0, 2, 3, true), state(1, 4, 5, false)],
        );
        journal.record_user_states(transactions[2], vec![state(2, 7, 8, false)]);

        let absorbed = transactions[1];
        journal.reconcile(|transaction| transaction != absorbed);

        assert_eq!(
            journal.states_restored_by_undo(transactions[0]),
            vec![state(0, 1, 2, false), state(1, 4, 5, false)]
        );
        assert!(journal.states_restored_by_undo(absorbed).is_empty());
        assert_eq!(
            journal.states_restored_by_undo(transactions[2]),
            vec![state(2, 7, 8, false)]
        );
    }

    #[gpui::test]
    async fn an_absorbed_first_entry_is_dropped_without_a_previous_entry(cx: &mut TestAppContext) {
        let transactions = transactions(cx, 2);
        let mut journal = UndoJournal::default();
        journal.record_user_states(transactions[0], vec![state(0, 1, 2, false)]);
        journal.record_user_states(transactions[1], vec![state(1, 4, 5, false)]);

        let absorbed = transactions[0];
        journal.reconcile(|transaction| transaction != absorbed);

        assert!(journal.states_restored_by_undo(absorbed).is_empty());
        assert_eq!(
            journal.states_restored_by_undo(transactions[1]),
            vec![state(1, 4, 5, false)]
        );
    }

    #[gpui::test]
    async fn cleared_journals_forget_every_entry(cx: &mut TestAppContext) {
        let transaction = transactions(cx, 1)[0];
        let mut journal = UndoJournal::default();
        journal.record_command(
            transaction,
            vec![state(0, 1, 2, false)],
            vec![state(0, 1, 2, true)],
        );

        journal.clear();

        assert!(journal.states_restored_by_undo(transaction).is_empty());
        assert!(journal.states_restored_by_redo(transaction).is_empty());
    }
}
