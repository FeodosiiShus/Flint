use super::change::ChangeCounters;

pub(crate) const ALL_CONFLICTS_RESOLVED: &str = "All conflicts resolved";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MergeStatus {
    Hidden,
    AllConflictsResolved,
    Differences(String),
}

impl MergeStatus {
    pub(crate) fn text(&self) -> Option<&str> {
        match self {
            Self::Hidden => None,
            Self::AllConflictsResolved => Some(ALL_CONFLICTS_RESOLVED),
            Self::Differences(text) => Some(text.as_str()),
        }
    }
}

pub(crate) fn merge_status(counters: Option<ChangeCounters>) -> MergeStatus {
    match counters {
        None => MergeStatus::Hidden,
        Some(counters) if counters.is_all_resolved() => MergeStatus::AllConflictsResolved,
        Some(counters) => MergeStatus::Differences(differences_status_text(
            counters.changes,
            counters.conflicts,
        )),
    }
}

pub(crate) fn differences_status_text(changes: usize, conflicts: usize) -> String {
    format!(
        "{}, {}",
        choose(
            changes,
            [
                (0, "No changes".to_string()),
                (1, "1 change".to_string()),
                (2, format!("{} changes", format_number(changes))),
            ]
        ),
        choose(
            conflicts,
            [
                (0, "No conflicts".to_string()),
                (1, "1 conflict".to_string()),
                (2, format!("{} conflicts", format_number(conflicts))),
            ]
        ),
    )
}

pub(crate) fn iterative_partially_resolved_confirmation_message(
    conflicts: usize,
    changes: usize,
) -> String {
    let unresolved_conflicts = choose(
        conflicts,
        [
            (0, String::new()),
            (1, "1 unresolved conflict".to_string()),
            (
                2,
                format!("{} unresolved conflicts", format_number(conflicts)),
            ),
        ],
    );
    let conjunction = if conflicts == 0 {
        String::new()
    } else {
        choose(changes, [(0, String::new()), (1, " and ".to_string())])
    };
    let unprocessed_changes = choose(
        changes,
        [
            (0, String::new()),
            (1, "1 unprocessed change".to_string()),
            (2, format!("{} unprocessed changes", format_number(changes))),
        ],
    );
    let verb = if conflicts == 0 {
        choose(
            changes,
            [(1, "remains".to_string()), (2, "remain".to_string())],
        )
    } else {
        choose(
            changes,
            [
                (0, "remains".to_string()),
                (1, "remain".to_string()),
                (2, "remain".to_string()),
            ],
        )
    };
    format!(
        "{unresolved_conflicts}{conjunction}{unprocessed_changes} still {verb}. The file will be saved exactly as shown in the Result pane."
    )
}

pub(crate) fn partially_resolved_confirmation_message(changes: usize, conflicts: usize) -> String {
    let verb = if changes == 0 {
        choose(conflicts, [(1, "is".to_string()), (2, "are".to_string())])
    } else {
        choose(changes, [(1, "is".to_string()), (2, "are".to_string())])
    };
    let changes_phrase = choose(
        changes,
        [
            (0, String::new()),
            (1, "one change".to_string()),
            (2, format!("{} changes", format_number(changes))),
        ],
    );
    let conjunction = if changes == 0 {
        String::new()
    } else {
        choose(conflicts, [(0, String::new()), (1, " and ".to_string())])
    };
    let conflicts_phrase = choose(
        conflicts,
        [
            (0, String::new()),
            (1, "one conflict".to_string()),
            (2, format!("{} conflicts", format_number(conflicts))),
        ],
    );
    format!(
        "There {verb} {changes_phrase}{conjunction}{conflicts_phrase} left unprocessed.\nSave changes and mark the conflict resolved anyway?"
    )
}

fn choose<const COUNT: usize>(value: usize, options: [(usize, String); COUNT]) -> String {
    let mut selected = None;
    for (limit, text) in options {
        if selected.is_none() || value >= limit {
            selected = Some(text);
        } else {
            break;
        }
    }
    selected.unwrap_or_default()
}

fn format_number(value: usize) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (position, digit) in digits.chars().enumerate() {
        if position > 0 && (digits.len() - position) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_text_matches_the_evaluated_bundle_examples() {
        assert_eq!(differences_status_text(2, 1), "2 changes, 1 conflict");
        assert_eq!(differences_status_text(0, 3), "No changes, 3 conflicts");
        assert_eq!(differences_status_text(1, 0), "1 change, No conflicts");
        assert_eq!(differences_status_text(0, 0), "No changes, No conflicts");
    }

    #[test]
    fn status_text_groups_thousands_like_message_format_numbers() {
        assert_eq!(
            differences_status_text(1234, 1_000_000),
            "1,234 changes, 1,000,000 conflicts"
        );
        assert_eq!(
            differences_status_text(999, 100),
            "999 changes, 100 conflicts"
        );
    }

    #[test]
    fn merge_status_distinguishes_uninitialized_resolved_and_pending() {
        assert_eq!(merge_status(None), MergeStatus::Hidden);
        assert_eq!(
            merge_status(Some(ChangeCounters {
                changes: 0,
                conflicts: 0
            })),
            MergeStatus::AllConflictsResolved
        );
        assert_eq!(
            merge_status(Some(ChangeCounters {
                changes: 2,
                conflicts: 1
            })),
            MergeStatus::Differences("2 changes, 1 conflict".to_string())
        );
    }

    #[test]
    fn status_text_is_hidden_until_the_counters_exist() {
        assert_eq!(MergeStatus::Hidden.text(), None);
        assert_eq!(
            MergeStatus::AllConflictsResolved.text(),
            Some("All conflicts resolved")
        );
        assert_eq!(
            merge_status(Some(ChangeCounters {
                changes: 1,
                conflicts: 0
            }))
            .text(),
            Some("1 change, No conflicts")
        );
    }

    const SAVED_SUFFIX: &str = " The file will be saved exactly as shown in the Result pane.";

    #[test]
    fn iterative_confirmation_message_covers_the_plural_matrix() {
        let expectations = [
            (1, 0, "1 unresolved conflict still remains."),
            (2, 0, "2 unresolved conflicts still remains."),
            (
                1,
                1,
                "1 unresolved conflict and 1 unprocessed change still remain.",
            ),
            (
                3,
                2,
                "3 unresolved conflicts and 2 unprocessed changes still remain.",
            ),
            (0, 1, "1 unprocessed change still remains."),
            (0, 2, "2 unprocessed changes still remain."),
            (
                2,
                1,
                "2 unresolved conflicts and 1 unprocessed change still remain.",
            ),
        ];
        for (conflicts, changes, expected) in expectations {
            assert_eq!(
                iterative_partially_resolved_confirmation_message(conflicts, changes),
                format!("{expected}{SAVED_SUFFIX}"),
                "conflicts={conflicts} changes={changes}"
            );
        }
    }

    #[test]
    fn one_shot_confirmation_message_covers_the_plural_matrix() {
        let expectations = [
            (1, 0, "There is one change left unprocessed."),
            (2, 0, "There are 2 changes left unprocessed."),
            (0, 1, "There is one conflict left unprocessed."),
            (0, 3, "There are 3 conflicts left unprocessed."),
            (
                1,
                1,
                "There is one change and one conflict left unprocessed.",
            ),
            (
                3,
                2,
                "There are 3 changes and 2 conflicts left unprocessed.",
            ),
        ];
        for (changes, conflicts, expected) in expectations {
            assert_eq!(
                partially_resolved_confirmation_message(changes, conflicts),
                format!("{expected}\nSave changes and mark the conflict resolved anyway?"),
                "changes={changes} conflicts={conflicts}"
            );
        }
    }

    #[test]
    fn choose_falls_back_to_the_first_option_below_the_first_limit() {
        assert_eq!(
            choose(0, [(1, "one".to_string()), (2, "many".to_string())]),
            "one"
        );
        assert_eq!(
            choose(5, [(1, "one".to_string()), (2, "many".to_string())]),
            "many"
        );
    }
}
