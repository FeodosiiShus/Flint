use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use git::repository::{GitFailureKind, RepositoryOperation};
use gpui::{App, Task, TestAppContext, Window, WindowHandle};
use project::{FakeFs, FakeGitOperation, Fs, Project};
use serde_json::json;
use settings::SettingsStore;
use util::path;
use workspace::MultiWorkspace;

use super::customizer::{
    CustomizerKind, RebaseCustomizerSpec, RebaseDescription, RebaseUpstream, SmartRestoreSpec,
    UnstashSpec, build_description, build_dialog_texts, cherry_pick_description,
    commit_message_preview, default_description, last_revision, merge_base_request,
    merge_description, rebase_description,
};
use super::dialog_texts::{DialogTexts, PaneTitleText, PaneTitleTexts, ShowDetails};
use super::driver::{
    ConflictContext, ConflictParams, DialogCycle, DriverEffects, Hook, NextStep, ProceedPlan,
    RebaseStop, ResolveMode, ResolveOutcome, ResolverBehavior, UnresolvedNotice,
    classify_rebase_stop, exception_notice, next_step, outcome_for, plain_notification_text,
    proceed_plan, resolve_conflicts_with, unresolved_after_notification_notice,
    unresolved_conflicts_notice,
};
use super::labels::{
    BranchTip, CommitFact, MergeLabels, RefInfo, RepositoryFacts, current_branch_presentable,
    load_repository_facts, merge_branch_or_cherry_pick, quoted_ref_name, rebase_branch_name,
    resolve_branch_name, resolve_merge_branch, resolve_rebase_onto_branch, short_hash,
};
use super::rich_text::{RichSegment, RichStyle, RichText};
use crate::branch_operations::{BranchNotice, NoticeAction, NoticeSeverity};
use crate::merge_tool::conflicts_dialog::request::{ConflictsDialogRequest, ConflictsDialogResult};

const HEAD_SHA: &str = "1111111111111111111111111111111111111111";
const MERGE_HEAD_SHA: &str = "2222222222222222222222222222222222222222";
const REBASE_HEAD_SHA: &str = "3333333333333333333333333333333333333333";
const CHERRY_PICK_SHA: &str = "4444444444444444444444444444444444444444";
const ONTO_SHA: &str = "5555555555555555555555555555555555555555";
const BASE_SHA: &str = "6666666666666666666666666666666666666666";
const SIXTY_FOUR_HEX: &str = "7777777777777777777777777777777777777777777777777777777777777777";

fn commit_fact(sha: &str, author_name: &str, message: &str) -> CommitFact {
    CommitFact {
        sha: sha.to_string(),
        author_name: author_name.to_string(),
        message: message.to_string(),
    }
}

fn tip(name: &str, sha: &str, is_remote: bool) -> BranchTip {
    BranchTip {
        name: name.to_string(),
        sha: sha.to_string(),
        is_remote,
    }
}

fn merging_facts() -> RepositoryFacts {
    RepositoryFacts {
        operation: Some(RepositoryOperation::Merge),
        head: Some(HEAD_SHA.to_string()),
        branch: Some("main".to_string()),
        branches: vec![
            tip("main", HEAD_SHA, false),
            tip("feature", MERGE_HEAD_SHA, false),
        ],
        merge_head: Some(commit_fact(MERGE_HEAD_SHA, "Jane Doe", "Add feature")),
        ..RepositoryFacts::default()
    }
}

fn rebasing_facts() -> RepositoryFacts {
    RepositoryFacts {
        operation: Some(RepositoryOperation::Rebase),
        head: Some(HEAD_SHA.to_string()),
        branch: Some("feature".to_string()),
        branches: vec![tip("main", ONTO_SHA, false)],
        rebase_head: Some(commit_fact(REBASE_HEAD_SHA, "Jane Doe", "Fix typo")),
        rebase_onto: Some(ONTO_SHA.to_string()),
        ..RepositoryFacts::default()
    }
}

fn cherry_picking_facts() -> RepositoryFacts {
    RepositoryFacts {
        operation: Some(RepositoryOperation::CherryPick),
        head: Some(HEAD_SHA.to_string()),
        branch: Some("main".to_string()),
        cherry_pick_head: Some(commit_fact(
            CHERRY_PICK_SHA,
            "Jane Doe",
            "Fix the bug\n\nLong body",
        )),
        ..RepositoryFacts::default()
    }
}

fn range_title(label: RichText, first: &str, second: &str) -> PaneTitleText {
    let dialog_title = label.plain_text();
    PaneTitleText {
        label,
        show_details: Some(ShowDetails::CommitRange {
            first: first.into(),
            second: second.into(),
            dialog_title: dialog_title.into(),
        }),
    }
}

fn single_commit_title(label: RichText, sha: &str) -> PaneTitleText {
    let dialog_title = label.plain_text();
    PaneTitleText {
        label,
        show_details: Some(ShowDetails::Commit {
            sha: sha.into(),
            dialog_title: dialog_title.into(),
        }),
    }
}

fn unadorned(label: RichText) -> PaneTitleText {
    PaneTitleText {
        label,
        show_details: None,
    }
}

fn texts(yours: &str, theirs: &str, left: PaneTitleText, right: PaneTitleText) -> DialogTexts {
    DialogTexts {
        dialog_title: "Conflicts".into(),
        yours_column: yours.into(),
        theirs_column: theirs.into(),
        pane_titles: PaneTitleTexts {
            left,
            result: PaneTitleText::plain("Result"),
            right,
        },
    }
}

fn rebase_spec(
    upstream: Option<RebaseUpstream>,
    branch: Option<&str>,
    initial_branch: Option<&str>,
) -> CustomizerKind {
    CustomizerKind::Rebase(RebaseCustomizerSpec {
        upstream,
        branch: branch.map(str::to_string),
        initial_branch: initial_branch.map(str::to_string),
    })
}

fn noop_resolve() -> Rc<dyn Fn(&mut Window, &mut App)> {
    Rc::new(|_window: &mut Window, _cx: &mut App| {})
}

fn action_labels(notice: &BranchNotice) -> Vec<String> {
    notice
        .actions
        .iter()
        .map(|action| action.label.to_string())
        .collect()
}

#[test]
fn short_hash_keeps_the_first_eight_characters() {
    assert_eq!(short_hash(HEAD_SHA), "11111111");
    assert_eq!(short_hash("abc"), "abc");
    assert_eq!(short_hash(""), "");
}

#[test]
fn quoted_ref_name_finds_the_first_non_empty_quoted_run() {
    assert_eq!(
        quoted_ref_name("Merge branch 'main' into feature"),
        Some("main")
    );
    assert_eq!(
        quoted_ref_name("Merge remote-tracking branch 'origin/main'"),
        Some("origin/main")
    );
    assert_eq!(quoted_ref_name("''x''"), Some("x"));
    assert_eq!(quoted_ref_name("Merge branch '' of x 'y'"), Some(" of x "));
    assert_eq!(quoted_ref_name("no quotes at all"), None);
    assert_eq!(quoted_ref_name("only 'one quote"), None);
    assert_eq!(quoted_ref_name(""), None);
}

#[test]
fn single_branch_at_a_hash_names_the_ref() {
    let branches = vec![
        tip("main", HEAD_SHA, false),
        tip("feature", MERGE_HEAD_SHA, false),
    ];

    assert_eq!(
        resolve_branch_name(&branches, MERGE_HEAD_SHA, None),
        RefInfo {
            hash: MERGE_HEAD_SHA.to_string(),
            branch_name: Some("feature".to_string()),
        }
    );
    assert_eq!(
        resolve_branch_name(&branches, ONTO_SHA, None).presentable(),
        "55555555"
    );
}

#[test]
fn several_branches_at_a_hash_are_broken_by_the_preferred_name() {
    let branches = vec![
        tip("feature", MERGE_HEAD_SHA, false),
        tip("feature-copy", MERGE_HEAD_SHA, false),
    ];

    assert_eq!(
        resolve_branch_name(&branches, MERGE_HEAD_SHA, Some("feature-copy")).branch_name,
        Some("feature-copy".to_string())
    );
    assert_eq!(
        resolve_branch_name(&branches, MERGE_HEAD_SHA, Some("unrelated")).branch_name,
        None
    );
    assert_eq!(
        resolve_branch_name(&branches, MERGE_HEAD_SHA, None).presentable(),
        "22222222"
    );
}

#[test]
fn remote_branches_name_a_ref_only_when_no_local_branch_matches() {
    let remote_only = vec![tip("origin/feature", MERGE_HEAD_SHA, true)];
    let local_and_remote = vec![
        tip("origin/feature", MERGE_HEAD_SHA, true),
        tip("feature", MERGE_HEAD_SHA, false),
    ];
    let with_origin_head = vec![
        tip("origin/HEAD", MERGE_HEAD_SHA, true),
        tip("origin/main", MERGE_HEAD_SHA, true),
    ];

    assert_eq!(
        resolve_branch_name(&remote_only, MERGE_HEAD_SHA, None).branch_name,
        Some("origin/feature".to_string())
    );
    assert_eq!(
        resolve_branch_name(&local_and_remote, MERGE_HEAD_SHA, None).branch_name,
        Some("feature".to_string())
    );
    assert_eq!(
        resolve_branch_name(&with_origin_head, MERGE_HEAD_SHA, None).branch_name,
        Some("origin/main".to_string())
    );
}

#[test]
fn branch_hashes_compare_case_insensitively() {
    let branches = vec![tip(
        "feature",
        "ABCDEF0123456789ABCDEF0123456789ABCDEF01",
        false,
    )];

    assert_eq!(
        resolve_branch_name(&branches, "abcdef0123456789abcdef0123456789abcdef01", None)
            .branch_name,
        Some("feature".to_string())
    );
}

#[test]
fn merge_labels_name_the_current_and_merged_branches() {
    let labels = MergeLabels::for_facts(&merging_facts());

    assert_eq!(labels.yours.as_deref(), Some("main"));
    assert_eq!(labels.theirs.as_deref(), Some("feature"));
    assert_eq!(labels.yours_column(), "Yours (main)");
    assert_eq!(labels.theirs_column(), "Theirs (feature)");
}

#[test]
fn merge_labels_fall_back_to_short_hashes() {
    let facts = RepositoryFacts {
        branch: None,
        branches: Vec::new(),
        ..merging_facts()
    };
    let labels = MergeLabels::for_facts(&facts);

    assert_eq!(labels.yours_column(), "Yours (11111111)");
    assert_eq!(labels.theirs_column(), "Theirs (22222222)");
}

#[test]
fn merge_labels_drop_the_parentheses_without_any_name() {
    let labels = MergeLabels::for_facts(&RepositoryFacts::default());

    assert_eq!(labels.yours_column(), "Yours");
    assert_eq!(labels.theirs_column(), "Theirs");
}

#[test]
fn merge_labels_name_the_rebase_onto_branch() {
    let named = MergeLabels::for_facts(&rebasing_facts());
    let unnamed = MergeLabels::for_facts(&RepositoryFacts {
        branches: Vec::new(),
        ..rebasing_facts()
    });

    assert_eq!(named.yours_column(), "Yours (feature)");
    assert_eq!(named.theirs_column(), "Theirs (main)");
    assert_eq!(unnamed.theirs_column(), "Theirs (55555555)");
}

#[test]
fn merge_labels_use_the_cherry_pick_literal() {
    let labels = MergeLabels::for_facts(&cherry_picking_facts());

    assert_eq!(labels.yours_column(), "Yours (main)");
    assert_eq!(labels.theirs_column(), "Theirs (cherry-pick)");
}

#[test]
fn merge_label_precedence_is_merge_then_rebase_then_cherry_pick() {
    let everything = RepositoryFacts {
        rebase_onto: Some(ONTO_SHA.to_string()),
        cherry_pick_head: Some(commit_fact(CHERRY_PICK_SHA, "Jane Doe", "Fix")),
        branches: vec![
            tip("feature", MERGE_HEAD_SHA, false),
            tip("main", ONTO_SHA, false),
        ],
        ..merging_facts()
    };
    let without_merge = RepositoryFacts {
        merge_head: None,
        ..everything.clone()
    };
    let only_cherry_pick = RepositoryFacts {
        rebase_onto: None,
        ..without_merge.clone()
    };

    assert_eq!(
        merge_branch_or_cherry_pick(&everything),
        Some("feature".to_string())
    );
    assert_eq!(
        merge_branch_or_cherry_pick(&without_merge),
        Some("main".to_string())
    );
    assert_eq!(
        merge_branch_or_cherry_pick(&only_cherry_pick),
        Some("cherry-pick".to_string())
    );
}

#[test]
fn merge_message_breaks_ties_between_branches_sharing_the_merge_head() {
    let facts = RepositoryFacts {
        branches: vec![
            tip("feature", MERGE_HEAD_SHA, false),
            tip("feature-copy", MERGE_HEAD_SHA, false),
        ],
        merge_message: Some("Merge branch 'feature-copy' into main\n\n# Conflicts:\n".to_string()),
        ..merging_facts()
    };

    let merge_branch = resolve_merge_branch(&facts).expect("MERGE_HEAD resolves");

    assert_eq!(merge_branch.branch_name, Some("feature-copy".to_string()));
}

#[test]
fn merge_message_names_a_branch_only_through_its_first_line() {
    let facts = RepositoryFacts {
        branches: vec![
            tip("feature", MERGE_HEAD_SHA, false),
            tip("feature-copy", MERGE_HEAD_SHA, false),
        ],
        merge_message: Some("Integrate the work\n\nSee 'feature-copy' for details\n".to_string()),
        ..merging_facts()
    };

    let merge_branch = resolve_merge_branch(&facts).expect("MERGE_HEAD resolves");

    assert_eq!(merge_branch.branch_name, None);
    assert_eq!(merge_branch.presentable(), "22222222");
}

#[test]
fn rebase_onto_branch_never_consults_the_merge_message() {
    let facts = RepositoryFacts {
        branches: vec![tip("main", ONTO_SHA, false), tip("other", ONTO_SHA, false)],
        merge_message: Some("Merge branch 'main' into feature\n".to_string()),
        ..rebasing_facts()
    };

    let onto = resolve_rebase_onto_branch(&facts).expect("onto resolves");

    assert_eq!(onto.branch_name, None);
    assert_eq!(onto.presentable(), "55555555");
}

#[test]
fn current_branch_presentable_prefers_the_branch_over_the_head() {
    assert_eq!(
        current_branch_presentable(&merging_facts()),
        Some("main".to_string())
    );
    assert_eq!(
        current_branch_presentable(&RepositoryFacts {
            branch: None,
            ..merging_facts()
        }),
        Some("11111111".to_string())
    );
    assert_eq!(
        current_branch_presentable(&RepositoryFacts::default()),
        None
    );
}

#[test]
fn rebase_branch_name_reads_the_head_name_file_like_the_repository_reader() {
    assert_eq!(
        rebase_branch_name("refs/heads/feature\n"),
        Some("feature".to_string())
    );
    assert_eq!(
        rebase_branch_name("refs/heads/feature/login"),
        Some("feature/login".to_string())
    );
    assert_eq!(rebase_branch_name("feature"), Some("feature".to_string()));
    assert_eq!(rebase_branch_name("detached HEAD\n"), None);
    assert_eq!(rebase_branch_name("  \n"), None);
}

#[test]
fn rich_text_merges_adjacent_runs_of_one_style_and_ignores_empty_text() {
    let text = RichText::plain("Rebasing ")
        .with_plain("branch ")
        .with_plain("")
        .with_bold("feature")
        .with_bold("-x")
        .with_code("")
        .with_line_break()
        .with_plain("done");

    assert_eq!(
        text.segments(),
        [
            RichSegment::Text {
                text: "Rebasing branch ".to_string(),
                style: RichStyle::Plain
            },
            RichSegment::Text {
                text: "feature-x".to_string(),
                style: RichStyle::Bold
            },
            RichSegment::LineBreak,
            RichSegment::Text {
                text: "done".to_string(),
                style: RichStyle::Plain
            },
        ]
    );
    assert_eq!(text.plain_text(), "Rebasing branch feature-x\ndone");
}

#[test]
fn default_description_agrees_with_the_file_count() {
    assert_eq!(
        default_description(0).plain_text(),
        "The following file has conflicts:"
    );
    assert_eq!(
        default_description(1).plain_text(),
        "The following file has conflicts:"
    );
    assert_eq!(
        default_description(2).plain_text(),
        "The following files have conflicts:"
    );
    assert_eq!(
        default_description(40).plain_text(),
        "The following files have conflicts:"
    );
}

#[test]
fn merge_description_evaluates_both_choice_patterns() {
    assert_eq!(
        merge_description(1, "feature", 1, "main"),
        RichText::plain("Merging branch ")
            .with_bold("feature")
            .with_plain(" into branch ")
            .with_bold("main")
    );
    assert_eq!(
        merge_description(2, "feature", 1, "main"),
        RichText::plain("Merging diverging branches into branch ").with_bold("main")
    );
    assert_eq!(
        merge_description(1, "feature", 2, "main"),
        RichText::plain("Merging branch ")
            .with_bold("feature")
            .with_plain(" into diverging branches")
    );
    assert_eq!(
        merge_description(3, "feature", 2, "main").plain_text(),
        "Merging diverging branches into diverging branches"
    );
    assert_eq!(
        merge_description(0, "feature", 0, "main").plain_text(),
        "Merging branch feature into branch main"
    );
}

#[test]
fn rebase_description_names_the_branch_the_base_and_the_revision() {
    let full = rebase_description(&RebaseDescription {
        rebasing_branch: Some("feature"),
        base_branch: Some("main"),
        base_hash: Some(ONTO_SHA),
        conflict_commit: None,
    });
    let without_revision = rebase_description(&RebaseDescription {
        rebasing_branch: Some("feature"),
        base_branch: Some("main"),
        base_hash: None,
        conflict_commit: None,
    });
    let hash_only = rebase_description(&RebaseDescription {
        rebasing_branch: Some("feature"),
        base_branch: None,
        base_hash: Some(ONTO_SHA),
        conflict_commit: None,
    });
    let diverging = rebase_description(&RebaseDescription {
        rebasing_branch: None,
        base_branch: None,
        base_hash: None,
        conflict_commit: None,
    });

    assert_eq!(
        full,
        RichText::plain("Rebasing branch ")
            .with_bold("feature")
            .with_plain(" onto branch ")
            .with_bold("main")
            .with_plain(", revision 55555555")
    );
    assert_eq!(
        without_revision.plain_text(),
        "Rebasing branch feature onto branch main"
    );
    assert_eq!(
        hash_only,
        RichText::plain("Rebasing branch ")
            .with_bold("feature")
            .with_plain(" onto ")
            .with_bold("55555555")
    );
    assert_eq!(diverging.plain_text(), "Rebasing onto diverging branches");
}

#[test]
fn rebase_description_appends_the_conflicting_commit_block() {
    let commit = commit_fact(REBASE_HEAD_SHA, "Jane Doe", "Fix typo");

    let description = rebase_description(&RebaseDescription {
        rebasing_branch: Some("feature"),
        base_branch: Some("main"),
        base_hash: None,
        conflict_commit: Some(&commit),
    });

    assert_eq!(
        description,
        RichText::plain("Rebasing branch ")
            .with_bold("feature")
            .with_plain(" onto branch ")
            .with_bold("main")
            .with_plain(". Current commit ")
            .with_code("33333333")
            .with_plain(" made by Jane Doe:")
            .with_line_break()
            .with_code("Fix typo")
    );
}

#[test]
fn cherry_pick_description_evaluates_both_choice_patterns() {
    assert_eq!(
        cherry_pick_description(1, "44444444", Some(("Jane Doe", "Fix the bug"))),
        RichText::plain("Conflicts during cherry-picking commit ")
            .with_code("44444444")
            .with_plain(" made by Jane Doe")
            .with_line_break()
            .with_code("Fix the bug")
    );
    assert_eq!(
        cherry_pick_description(2, "44444444", None).plain_text(),
        "Conflicts during cherry-picking multiple commits"
    );
    assert_eq!(
        cherry_pick_description(1, "44444444", None),
        RichText::plain("Conflicts during cherry-picking commit ").with_code("44444444")
    );
}

#[test]
fn commit_message_preview_keeps_up_to_three_lines_verbatim() {
    assert_eq!(
        commit_message_preview("Fix typo"),
        RichText::new().with_code("Fix typo")
    );
    assert_eq!(
        commit_message_preview("One\nTwo\nThree"),
        RichText::new()
            .with_code("One")
            .with_line_break()
            .with_code("Two")
            .with_line_break()
            .with_code("Three")
    );
    assert_eq!(
        commit_message_preview("Subject\n\nBody"),
        RichText::new()
            .with_code("Subject")
            .with_line_break()
            .with_line_break()
            .with_code("Body")
    );
    assert_eq!(
        commit_message_preview("One\r\nTwo").plain_text(),
        "One\nTwo"
    );
}

#[test]
fn commit_message_preview_cuts_longer_messages_with_three_ascii_dots() {
    assert_eq!(
        commit_message_preview("One\nTwo\nThree\nFour"),
        RichText::new()
            .with_code("One")
            .with_line_break()
            .with_code("Two")
            .with_line_break()
            .with_code("Three...")
    );
    assert_eq!(
        commit_message_preview("Subject\n\n\nBody here").plain_text(),
        "Subject..."
    );
    assert_eq!(
        commit_message_preview("One\r\nTwo\rThree\nFour").plain_text(),
        "One\nTwo\nThree..."
    );
}

#[test]
fn rebase_upstream_is_a_commit_only_for_full_length_hex_strings() {
    assert_eq!(
        RebaseUpstream::from_ref_string(ONTO_SHA),
        RebaseUpstream::Commit(ONTO_SHA.to_string())
    );
    assert_eq!(
        RebaseUpstream::from_ref_string(SIXTY_FOUR_HEX),
        RebaseUpstream::Commit(SIXTY_FOUR_HEX.to_string())
    );
    assert_eq!(
        RebaseUpstream::from_ref_string("ABCDEF0123456789ABCDEF0123456789ABCDEF01"),
        RebaseUpstream::Commit("ABCDEF0123456789ABCDEF0123456789ABCDEF01".to_string())
    );
    assert_eq!(
        RebaseUpstream::from_ref_string("deadbeef"),
        RebaseUpstream::Reference("deadbeef".to_string())
    );
    assert_eq!(
        RebaseUpstream::from_ref_string("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"),
        RebaseUpstream::Reference("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz".to_string())
    );
    assert_eq!(
        RebaseUpstream::from_ref_string("main"),
        RebaseUpstream::Reference("main".to_string())
    );
    assert_eq!(
        RebaseUpstream::from_ref_string(""),
        RebaseUpstream::Reference("HEAD".to_string())
    );
}

#[test]
fn merging_pane_titles_name_both_branches_and_their_commit_ranges() {
    let expected = texts(
        "Yours (main)",
        "Theirs (feature)",
        range_title(
            RichText::plain("Changes from ").with_bold("main"),
            BASE_SHA,
            HEAD_SHA,
        ),
        range_title(
            RichText::plain("Changes from ").with_bold("feature"),
            BASE_SHA,
            MERGE_HEAD_SHA,
        ),
    );

    assert_eq!(
        build_dialog_texts(
            &CustomizerKind::GitDefault,
            &merging_facts(),
            Some(BASE_SHA),
            false
        ),
        expected
    );
}

#[test]
fn merging_without_a_merge_base_falls_back_to_the_default_titles() {
    let forward = build_dialog_texts(&CustomizerKind::GitDefault, &merging_facts(), None, false);
    let reversed = build_dialog_texts(&CustomizerKind::GitDefault, &merging_facts(), None, true);

    assert_eq!(
        forward.pane_titles.left,
        PaneTitleText::plain("Your version")
    );
    assert_eq!(
        forward.pane_titles.right,
        PaneTitleText::plain(format!("Changes from server (revision {MERGE_HEAD_SHA})"))
    );
    assert_eq!(
        reversed.pane_titles.right,
        PaneTitleText::plain(format!("Changes from server (revision {HEAD_SHA})"))
    );
    assert_eq!(forward.pane_titles.result, PaneTitleText::plain("Result"));
    assert_eq!(forward.dialog_title, "Conflicts");
}

#[test]
fn merging_without_a_head_falls_back_to_the_default_titles() {
    let facts = RepositoryFacts {
        head: None,
        ..merging_facts()
    };

    let titles = build_dialog_texts(&CustomizerKind::GitDefault, &facts, Some(BASE_SHA), false);

    assert_eq!(
        titles.pane_titles.left,
        PaneTitleText::plain("Your version")
    );
}

#[test]
fn detached_merge_names_the_head_by_its_short_hash() {
    let facts = RepositoryFacts {
        branch: None,
        ..merging_facts()
    };

    let titles = build_dialog_texts(&CustomizerKind::GitDefault, &facts, Some(BASE_SHA), false);

    assert_eq!(
        titles.pane_titles.left.label,
        RichText::plain("Changes from ").with_bold("11111111")
    );
    assert_eq!(titles.yours_column, "Yours (11111111)");
}

#[test]
fn rebasing_pane_titles_use_the_rebase_head_and_the_onto_branch() {
    let expected = texts(
        "Yours (feature)",
        "Theirs (main)",
        single_commit_title(
            RichText::plain("Rebasing 33333333 from ").with_bold("feature"),
            REBASE_HEAD_SHA,
        ),
        range_title(
            RichText::plain("Already rebased commits and commits from ").with_bold("main"),
            BASE_SHA,
            "HEAD",
        ),
    );

    assert_eq!(
        build_dialog_texts(
            &CustomizerKind::GitDefault,
            &rebasing_facts(),
            Some(BASE_SHA),
            true
        ),
        expected
    );
}

#[test]
fn rebasing_onto_an_unnamed_commit_uses_the_simple_right_title() {
    let facts = RepositoryFacts {
        branches: Vec::new(),
        ..rebasing_facts()
    };

    let titles = build_dialog_texts(&CustomizerKind::GitDefault, &facts, Some(BASE_SHA), true);

    assert_eq!(
        titles.pane_titles.right,
        range_title(RichText::plain("Already rebased commits"), BASE_SHA, "HEAD")
    );
    assert_eq!(titles.theirs_column, "Theirs (55555555)");
}

#[test]
fn cherry_picking_pane_titles_use_local_changes_and_the_picked_commit() {
    let expected = texts(
        "Yours (main)",
        "Theirs (cherry-pick)",
        range_title(RichText::plain("Local Changes"), BASE_SHA, "HEAD"),
        single_commit_title(
            RichText::plain("Changes from cherry-pick 44444444"),
            CHERRY_PICK_SHA,
        ),
    );

    assert_eq!(
        build_dialog_texts(
            &CustomizerKind::GitDefault,
            &cherry_picking_facts(),
            Some(BASE_SHA),
            false
        ),
        expected
    );
}

#[test]
fn merge_head_takes_precedence_over_a_running_rebase_for_the_titles() {
    let facts = RepositoryFacts {
        operation: Some(RepositoryOperation::Rebase),
        ..merging_facts()
    };

    let titles = build_dialog_texts(&CustomizerKind::GitDefault, &facts, Some(BASE_SHA), false);

    assert_eq!(
        titles.pane_titles.left,
        range_title(
            RichText::plain("Changes from ").with_bold("main"),
            BASE_SHA,
            HEAD_SHA
        )
    );
    assert_eq!(
        merge_base_request(&CustomizerKind::GitDefault, &facts),
        Some((HEAD_SHA.to_string(), MERGE_HEAD_SHA.to_string()))
    );
}

#[test]
fn cherry_pick_on_a_detached_head_keeps_the_default_titles() {
    let facts = RepositoryFacts {
        branch: None,
        ..cherry_picking_facts()
    };

    let titles = build_dialog_texts(&CustomizerKind::GitDefault, &facts, Some(BASE_SHA), false);

    assert_eq!(
        titles.pane_titles.left,
        PaneTitleText::plain("Your version")
    );
    assert_eq!(
        titles.pane_titles.right,
        PaneTitleText::plain(format!("Changes from server (revision {CHERRY_PICK_SHA})"))
    );
    assert_eq!(
        merge_base_request(&CustomizerKind::GitDefault, &facts),
        None
    );
}

#[test]
fn rebasing_without_rebase_head_onto_or_merge_base_keeps_the_default_titles() {
    let without_rebase_head = RepositoryFacts {
        rebase_head: None,
        ..rebasing_facts()
    };
    let without_onto = RepositoryFacts {
        rebase_onto: None,
        ..rebasing_facts()
    };
    let expected = |theirs_column: &str| {
        texts(
            "Yours (feature)",
            theirs_column,
            PaneTitleText::plain("Your version"),
            PaneTitleText::plain(format!("Changes from server (revision {HEAD_SHA})")),
        )
    };

    assert_eq!(
        build_dialog_texts(
            &CustomizerKind::GitDefault,
            &without_rebase_head,
            Some(BASE_SHA),
            true
        ),
        expected("Theirs (main)")
    );
    assert_eq!(
        build_dialog_texts(
            &CustomizerKind::GitDefault,
            &without_onto,
            Some(BASE_SHA),
            true
        ),
        expected("Theirs")
    );
    assert_eq!(
        build_dialog_texts(&CustomizerKind::GitDefault, &rebasing_facts(), None, true),
        expected("Theirs (main)")
    );
}

#[test]
fn cherry_picking_without_a_merge_base_keeps_the_default_titles() {
    let titles = build_dialog_texts(
        &CustomizerKind::GitDefault,
        &cherry_picking_facts(),
        None,
        false,
    );

    assert_eq!(
        titles,
        texts(
            "Yours (main)",
            "Theirs (cherry-pick)",
            PaneTitleText::plain("Your version"),
            PaneTitleText::plain(format!("Changes from server (revision {CHERRY_PICK_SHA})")),
        )
    );
}

#[test]
fn reverting_and_idle_repositories_keep_the_default_titles_and_description() {
    let reverting = RepositoryFacts {
        operation: Some(RepositoryOperation::Revert),
        head: Some(HEAD_SHA.to_string()),
        branch: Some("main".to_string()),
        ..RepositoryFacts::default()
    };

    let expected = texts(
        "Yours (main)",
        "Theirs",
        PaneTitleText::plain("Your version"),
        PaneTitleText::plain("Changes from server"),
    );

    assert_eq!(
        build_dialog_texts(
            &CustomizerKind::GitDefault,
            &reverting,
            Some(BASE_SHA),
            false
        ),
        expected
    );
    assert_eq!(
        build_description(&CustomizerKind::GitDefault, &reverting, 1).plain_text(),
        "The following file has conflicts:"
    );
    assert_eq!(
        build_dialog_texts(
            &CustomizerKind::GitDefault,
            &RepositoryFacts {
                operation: None,
                ..reverting
            },
            None,
            false
        ),
        expected
    );
}

#[test]
fn last_revision_follows_the_merge_head_fallback_chain() {
    let everything = RepositoryFacts {
        cherry_pick_head: Some(commit_fact(CHERRY_PICK_SHA, "Jane Doe", "Fix")),
        rebase_current_commit: Some(REBASE_HEAD_SHA.to_string()),
        ..merging_facts()
    };
    let without_merge = RepositoryFacts {
        merge_head: None,
        ..everything.clone()
    };
    let without_cherry_pick = RepositoryFacts {
        cherry_pick_head: None,
        ..without_merge.clone()
    };
    let nothing = RepositoryFacts {
        rebase_current_commit: None,
        ..without_cherry_pick.clone()
    };

    assert_eq!(
        last_revision(&everything, false),
        Some(MERGE_HEAD_SHA.to_string())
    );
    assert_eq!(
        last_revision(&without_merge, false),
        Some(CHERRY_PICK_SHA.to_string())
    );
    assert_eq!(
        last_revision(&without_cherry_pick, false),
        Some(REBASE_HEAD_SHA.to_string())
    );
    assert_eq!(last_revision(&nothing, false), None);
    assert_eq!(last_revision(&everything, true), Some(HEAD_SHA.to_string()));
}

#[test]
fn rebase_process_texts_name_the_rebased_branch_and_the_base() {
    let kind = rebase_spec(
        Some(RebaseUpstream::Reference("main".to_string())),
        None,
        Some("feature"),
    );
    let expected = texts(
        "Yours (feature)",
        "Theirs (main)",
        single_commit_title(
            RichText::plain("Rebasing 33333333 from ").with_bold("feature"),
            REBASE_HEAD_SHA,
        ),
        range_title(
            RichText::plain("Already rebased commits and commits from ").with_bold("main"),
            BASE_SHA,
            "HEAD",
        ),
    );

    assert_eq!(
        build_dialog_texts(&kind, &rebasing_facts(), Some(BASE_SHA), true),
        expected
    );
}

#[test]
fn rebase_process_onto_a_commit_names_the_base_by_its_short_hash() {
    let kind = rebase_spec(
        Some(RebaseUpstream::Commit(ONTO_SHA.to_string())),
        Some("feature"),
        None,
    );

    let with_base = build_dialog_texts(&kind, &rebasing_facts(), Some(BASE_SHA), true);
    let without_base = build_dialog_texts(&kind, &rebasing_facts(), None, false);

    assert_eq!(with_base.theirs_column, "Theirs (55555555)");
    assert_eq!(
        with_base.pane_titles.right,
        range_title(RichText::plain("Already rebased commits"), BASE_SHA, "HEAD")
    );
    assert_eq!(
        without_base.pane_titles.right,
        unadorned(RichText::plain("Changes from ").with_bold("55555555"))
    );
}

#[test]
fn rebase_process_without_a_merge_base_uses_the_default_branch_titles() {
    let kind = rebase_spec(
        Some(RebaseUpstream::Reference("main".to_string())),
        Some("feature"),
        None,
    );
    let facts = RepositoryFacts {
        rebase_head: None,
        ..rebasing_facts()
    };

    let reversed = build_dialog_texts(&kind, &facts, None, true);
    let forward = build_dialog_texts(&kind, &facts, None, false);

    assert_eq!(
        reversed.pane_titles.left,
        unadorned(RichText::plain("Your version, branch ").with_bold("feature"))
    );
    assert_eq!(
        reversed.pane_titles.right,
        unadorned(
            RichText::plain("Changes from branch ")
                .with_bold("main")
                .with_plain(", revision 11111111")
        )
    );
    assert_eq!(
        forward.pane_titles.right,
        unadorned(RichText::plain("Changes from branch ").with_bold("main"))
    );
}

#[test]
fn rebase_process_resolves_the_head_upstream_hack_from_the_initial_branch() {
    let kind = rebase_spec(
        Some(RebaseUpstream::Reference("HEAD".to_string())),
        None,
        Some("feature"),
    );

    let rebase_texts = build_dialog_texts(&kind, &rebasing_facts(), Some(BASE_SHA), true);

    assert_eq!(rebase_texts.yours_column, "Yours (feature)");
    assert_eq!(rebase_texts.theirs_column, "Theirs (feature)");
    assert_eq!(
        merge_base_request(&kind, &rebasing_facts()),
        Some(("feature".to_string(), "feature".to_string()))
    );
}

#[test]
fn rebase_process_with_missing_inputs_behaves_like_the_default_customizer() {
    let without_upstream = rebase_spec(None, Some("feature"), Some("feature"));
    let head_without_initial = rebase_spec(
        Some(RebaseUpstream::Reference("HEAD".to_string())),
        Some("feature"),
        None,
    );
    let without_branch = rebase_spec(
        Some(RebaseUpstream::Reference("main".to_string())),
        None,
        None,
    );
    let default_texts = build_dialog_texts(
        &CustomizerKind::GitDefault,
        &merging_facts(),
        Some(BASE_SHA),
        false,
    );

    for kind in [without_upstream, head_without_initial, without_branch] {
        assert_eq!(
            build_dialog_texts(&kind, &merging_facts(), Some(BASE_SHA), false),
            default_texts
        );
        assert_eq!(
            build_description(&kind, &merging_facts(), 3),
            build_description(&CustomizerKind::GitDefault, &merging_facts(), 3)
        );
    }
}

#[test]
fn rebase_process_treats_a_blank_branch_like_a_missing_one() {
    let blank_with_initial = rebase_spec(
        Some(RebaseUpstream::Reference("main".to_string())),
        Some("   "),
        Some("feature"),
    );
    let blank_without_initial = rebase_spec(
        Some(RebaseUpstream::Reference("main".to_string())),
        Some("   "),
        None,
    );

    let named = build_dialog_texts(&blank_with_initial, &rebasing_facts(), Some(BASE_SHA), true);

    assert_eq!(named.yours_column, "Yours (feature)");
    assert_eq!(
        merge_base_request(&blank_with_initial, &rebasing_facts()),
        Some(("main".to_string(), "feature".to_string()))
    );
    assert_eq!(
        build_dialog_texts(
            &blank_without_initial,
            &merging_facts(),
            Some(BASE_SHA),
            false
        ),
        build_dialog_texts(
            &CustomizerKind::GitDefault,
            &merging_facts(),
            Some(BASE_SHA),
            false
        )
    );
}

#[test]
fn rebase_process_description_names_branch_base_and_conflicting_commit() {
    let by_reference = rebase_spec(
        Some(RebaseUpstream::Reference("main".to_string())),
        None,
        Some("feature"),
    );
    let by_commit = rebase_spec(
        Some(RebaseUpstream::Commit(ONTO_SHA.to_string())),
        Some("feature"),
        None,
    );

    assert_eq!(
        build_description(&by_reference, &rebasing_facts(), 2),
        RichText::plain("Rebasing branch ")
            .with_bold("feature")
            .with_plain(" onto branch ")
            .with_bold("main")
            .with_plain(". Current commit ")
            .with_code("33333333")
            .with_plain(" made by Jane Doe:")
            .with_line_break()
            .with_code("Fix typo")
    );
    assert_eq!(
        build_description(&by_commit, &RepositoryFacts::default(), 2),
        RichText::plain("Rebasing branch ")
            .with_bold("feature")
            .with_plain(" onto ")
            .with_bold("55555555")
    );
}

#[test]
fn default_customizer_describes_the_operation_in_progress() {
    let merging = build_description(&CustomizerKind::GitDefault, &merging_facts(), 2);
    let rebasing = build_description(&CustomizerKind::GitDefault, &rebasing_facts(), 2);
    let cherry_picking = build_description(&CustomizerKind::GitDefault, &cherry_picking_facts(), 2);
    let idle = build_description(&CustomizerKind::GitDefault, &RepositoryFacts::default(), 2);

    assert_eq!(
        merging,
        RichText::plain("Merging branch ")
            .with_bold("feature")
            .with_plain(" into branch ")
            .with_bold("main")
    );
    assert_eq!(
        rebasing,
        RichText::plain("Rebasing branch ")
            .with_bold("feature")
            .with_plain(" onto branch ")
            .with_bold("main")
            .with_plain(", revision 55555555. Current commit ")
            .with_code("33333333")
            .with_plain(" made by Jane Doe:")
            .with_line_break()
            .with_code("Fix typo")
    );
    assert_eq!(
        cherry_picking,
        RichText::plain("Conflicts during cherry-picking commit ")
            .with_code("44444444")
            .with_plain(" made by Jane Doe")
            .with_line_break()
            .with_code("Fix the bug")
            .with_line_break()
            .with_line_break()
            .with_code("Long body")
    );
    assert_eq!(idle.plain_text(), "The following files have conflicts:");
}

#[test]
fn merge_description_wins_over_rebase_and_cherry_pick() {
    let facts = RepositoryFacts {
        rebase_onto: Some(ONTO_SHA.to_string()),
        cherry_pick_head: Some(commit_fact(CHERRY_PICK_SHA, "Jane Doe", "Fix")),
        ..merging_facts()
    };

    assert_eq!(
        build_description(&CustomizerKind::GitDefault, &facts, 1).plain_text(),
        "Merging branch feature into branch main"
    );
}

#[test]
fn merging_unnamed_commit_is_described_by_its_short_hash() {
    let facts = RepositoryFacts {
        branches: Vec::new(),
        ..merging_facts()
    };

    assert_eq!(
        build_description(&CustomizerKind::GitDefault, &facts, 1).plain_text(),
        "Merging branch 22222222 into branch main"
    );
}

#[test]
fn description_override_replaces_only_the_description() {
    let overridden = CustomizerKind::DescriptionOverride(
        "Merge conflicts detected. Resolve them before continuing update.".into(),
    );
    let empty = CustomizerKind::DescriptionOverride("".into());

    assert_eq!(
        build_description(&overridden, &merging_facts(), 2),
        RichText::plain("Merge conflicts detected. Resolve them before continuing update.")
    );
    assert_eq!(
        build_description(&empty, &merging_facts(), 2),
        build_description(&CustomizerKind::GitDefault, &merging_facts(), 2)
    );
    assert_eq!(
        build_dialog_texts(&overridden, &merging_facts(), Some(BASE_SHA), false),
        build_dialog_texts(
            &CustomizerKind::GitDefault,
            &merging_facts(),
            Some(BASE_SHA),
            false
        )
    );
}

#[test]
fn unstash_customizer_uses_stash_titles_and_the_code_styled_stash_label() {
    let kind = CustomizerKind::Unstash(UnstashSpec {
        stash: "stash@{0}".into(),
        message: "WIP on main: abc1234 msg".into(),
    });

    let expected = texts(
        "Yours (main)",
        "Theirs",
        PaneTitleText::plain("Local changes"),
        PaneTitleText::plain("Changes from stash"),
    );
    let facts = RepositoryFacts {
        head: Some(HEAD_SHA.to_string()),
        branch: Some("main".to_string()),
        ..RepositoryFacts::default()
    };

    assert_eq!(build_dialog_texts(&kind, &facts, None, false), expected);
    assert_eq!(
        build_description(&kind, &facts, 1),
        RichText::plain("Conflicts during unstashing ")
            .with_code("stash@{0}\"WIP on main: abc1234 msg\"")
    );
}

#[test]
fn smart_restore_customizer_names_the_destination_and_the_saved_changes_source() {
    let stash = CustomizerKind::SmartRestore(SmartRestoreSpec {
        operation_title: "checkout".into(),
        destination_name: "feature".into(),
        stash: true,
    });
    let shelf = CustomizerKind::SmartRestore(SmartRestoreSpec {
        operation_title: "checkout".into(),
        destination_name: "feature".into(),
        stash: false,
    });

    let stash_texts = build_dialog_texts(&stash, &RepositoryFacts::default(), None, true);
    let shelf_texts = build_dialog_texts(&shelf, &RepositoryFacts::default(), None, true);

    assert_eq!(
        stash_texts.pane_titles.left,
        PaneTitleText::plain("Uncommitted changes from the stash")
    );
    assert_eq!(
        shelf_texts.pane_titles.left,
        PaneTitleText::plain("Uncommitted changes from the shelf")
    );
    assert_eq!(
        stash_texts.pane_titles.right,
        unadorned(RichText::plain("Changes from ").with_bold("feature"))
    );
    assert_eq!(
        build_description(&stash, &RepositoryFacts::default(), 1),
        RichText::plain(
            "Uncommitted changes that were saved before checkout have conflicts with files from "
        )
        .with_code("feature")
    );
}

#[test]
fn merge_base_is_requested_per_state_and_customizer() {
    let spec = rebase_spec(
        Some(RebaseUpstream::Reference("main".to_string())),
        None,
        Some("feature"),
    );
    let reverting = RepositoryFacts {
        operation: Some(RepositoryOperation::Revert),
        ..RepositoryFacts::default()
    };

    assert_eq!(
        merge_base_request(&CustomizerKind::GitDefault, &merging_facts()),
        Some((HEAD_SHA.to_string(), MERGE_HEAD_SHA.to_string()))
    );
    assert_eq!(
        merge_base_request(&CustomizerKind::GitDefault, &rebasing_facts()),
        Some(("REBASE_HEAD".to_string(), ONTO_SHA.to_string()))
    );
    assert_eq!(
        merge_base_request(&CustomizerKind::GitDefault, &cherry_picking_facts()),
        Some(("CHERRY_PICK_HEAD".to_string(), "HEAD".to_string()))
    );
    assert_eq!(
        merge_base_request(&CustomizerKind::GitDefault, &reverting),
        None
    );
    assert_eq!(
        merge_base_request(&CustomizerKind::GitDefault, &RepositoryFacts::default()),
        None
    );
    assert_eq!(
        merge_base_request(&spec, &rebasing_facts()),
        Some(("main".to_string(), "feature".to_string()))
    );
}

#[test]
fn resolver_next_step_follows_the_merge_algorithm() {
    use DialogCycle::{AllMerged, NothingToMerge, Remain};

    assert_eq!(
        next_step(NothingToMerge, ResolveMode::Initial),
        NextStep::ProceedIfNothingToMerge
    );
    assert_eq!(
        next_step(NothingToMerge, ResolveMode::FromNotification),
        NextStep::Finished
    );
    assert_eq!(
        next_step(
            AllMerged {
                should_finish_merge: false
            },
            ResolveMode::Initial
        ),
        NextStep::ProceedAfterAllMerged {
            should_finish_merge: false
        }
    );
    assert_eq!(
        next_step(
            AllMerged {
                should_finish_merge: true
            },
            ResolveMode::Initial
        ),
        NextStep::ProceedAfterAllMerged {
            should_finish_merge: true
        }
    );
    assert_eq!(
        next_step(
            AllMerged {
                should_finish_merge: false
            },
            ResolveMode::FromNotification
        ),
        NextStep::Finished
    );
    assert_eq!(
        next_step(
            AllMerged {
                should_finish_merge: true
            },
            ResolveMode::FromNotification
        ),
        NextStep::ProceedAfterAllMerged {
            should_finish_merge: true
        }
    );
    assert_eq!(
        next_step(
            Remain {
                should_finish_merge: false
            },
            ResolveMode::Initial
        ),
        NextStep::NotifyUnresolvedRemain
    );
    assert_eq!(
        next_step(
            Remain {
                should_finish_merge: true
            },
            ResolveMode::FromNotification
        ),
        NextStep::NotifyUnresolvedRemainAfterNotification
    );
    assert_eq!(
        next_step(
            Remain {
                should_finish_merge: true
            },
            ResolveMode::Initial
        ),
        NextStep::NotifyUnresolvedRemain
    );
    assert_eq!(
        next_step(
            Remain {
                should_finish_merge: false
            },
            ResolveMode::FromNotification
        ),
        NextStep::NotifyUnresolvedRemainAfterNotification
    );
}

#[test]
fn resolver_proceed_plan_follows_the_subclass_table() {
    let finish = Hook::AllMerged {
        should_finish_merge: true,
    };
    let no_finish = Hook::AllMerged {
        should_finish_merge: false,
    };

    for hook in [Hook::NothingToMerge, finish, no_finish] {
        assert_eq!(
            proceed_plan(ResolverBehavior::Plain, hook),
            ProceedPlan::Nothing
        );
        assert_eq!(
            proceed_plan(ResolverBehavior::CommitMergeAlways, hook),
            ProceedPlan::CommitMerge
        );
        assert_eq!(
            proceed_plan(ResolverBehavior::ContinueRebase, hook),
            ProceedPlan::ContinueRebase
        );
    }
    assert_eq!(
        proceed_plan(ResolverBehavior::CommitMergeWhenFinishRequested, finish),
        ProceedPlan::CommitMerge
    );
    assert_eq!(
        proceed_plan(ResolverBehavior::CommitMergeWhenFinishRequested, no_finish),
        ProceedPlan::Nothing
    );
    assert_eq!(
        proceed_plan(
            ResolverBehavior::CommitMergeWhenFinishRequested,
            Hook::NothingToMerge
        ),
        ProceedPlan::Nothing
    );
}

#[test]
fn resolver_outcome_reports_what_the_dialog_cycle_found() {
    assert_eq!(
        outcome_for(DialogCycle::NothingToMerge, true),
        ResolveOutcome {
            proceed: true,
            nothing_to_merge: true,
            all_resolved: false,
            should_finish_merge: false,
        }
    );
    assert_eq!(
        outcome_for(
            DialogCycle::AllMerged {
                should_finish_merge: true
            },
            false
        ),
        ResolveOutcome {
            proceed: false,
            nothing_to_merge: false,
            all_resolved: true,
            should_finish_merge: true,
        }
    );
    assert_eq!(
        outcome_for(
            DialogCycle::Remain {
                should_finish_merge: false
            },
            false
        ),
        ResolveOutcome {
            proceed: false,
            nothing_to_merge: false,
            all_resolved: false,
            should_finish_merge: false,
        }
    );
}

#[test]
fn rebase_stops_are_classified_from_the_git_output() {
    assert_eq!(
        classify_rebase_stop("No changes - did you forget to use 'git add'?\n"),
        RebaseStop::NothingToCommit
    );
    assert_eq!(
        classify_rebase_stop("NO CHANGES - DID YOU FORGET TO USE 'GIT ADD'?"),
        RebaseStop::NothingToCommit
    );
    assert_eq!(
        classify_rebase_stop(
            "No changes - did you forget to use 'git add'?\nResolve all conflicts manually, mark them as resolved with"
        ),
        RebaseStop::Conflict
    );
    assert_eq!(
        classify_rebase_stop("Stopped at abc... msg\nYou can amend the commit now, with"),
        RebaseStop::StoppedForEditing
    );
    assert_eq!(
        classify_rebase_stop("CONFLICT (content): Merge conflict in a.txt"),
        RebaseStop::Conflict
    );
    assert_eq!(
        classify_rebase_stop("error: could not apply abc123... subject"),
        RebaseStop::Conflict
    );
    assert_eq!(
        classify_rebase_stop("error: You must edit all merge conflicts and then"),
        RebaseStop::Conflict
    );
    assert_eq!(
        classify_rebase_stop(
            "hint: after resolving the conflicts, mark the corrected paths\nhint: with 'git add <paths>' or 'git rm <paths>'"
        ),
        RebaseStop::Conflict
    );
    assert_eq!(
        classify_rebase_stop(
            "Automatic cherry-pick failed. After resolving the conflicts, mark them with"
        ),
        RebaseStop::Conflict
    );
    assert_eq!(
        classify_rebase_stop("Failed to merge in the changes."),
        RebaseStop::Conflict
    );
    assert_eq!(
        classify_rebase_stop("fatal: something else"),
        RebaseStop::Other
    );
    assert_eq!(classify_rebase_stop(""), RebaseStop::Other);
}

#[test]
fn notification_markup_becomes_plain_text() {
    assert_eq!(
        plain_notification_text(
            "Then you may <b>continue rebase</b>. <br/> You also may <b>abort rebase</b> to restore the original branch and stop rebasing."
        ),
        "Then you may continue rebase.\nYou also may abort rebase to restore the original branch and stop rebasing."
    );
    assert_eq!(
        plain_notification_text("one<br/>two<br>three<BR />four"),
        "one\ntwo\nthree\nfour"
    );
    assert_eq!(plain_notification_text("plain text"), "plain text");
    assert_eq!(plain_notification_text("a < b"), "a < b");
    assert_eq!(plain_notification_text(""), "");
}

#[test]
fn user_merge_params_warn_with_the_branch_in_the_title() {
    let params = ConflictParams::for_user_merge("feature");

    assert!(!params.reverse);
    assert_eq!(params.customizer, CustomizerKind::GitDefault);
    assert_eq!(params.error_notification_title, "");
    let notice = unresolved_conflicts_notice(&params, noop_resolve())
        .expect("a merge with conflicts always warns");
    assert_eq!(
        notice.title.as_deref(),
        Some("feature Merged with Conflicts")
    );
    assert_eq!(notice.severity, NoticeSeverity::Warning);
    assert_eq!(notice.message, "");
    assert_eq!(action_labels(&notice), ["Resolve…"]);
}

#[test]
fn resolve_action_params_follow_the_detected_state_and_never_notify() {
    let forward = ConflictParams::for_resolve_action(false);
    let reversed = ConflictParams::for_resolve_action(true);

    assert!(!forward.reverse);
    assert!(reversed.reverse);
    assert_eq!(forward.customizer, CustomizerKind::GitDefault);
    assert_eq!(reversed.customizer, CustomizerKind::GitDefault);
    assert_eq!(forward.error_notification_title, "");
    assert!(unresolved_conflicts_notice(&forward, noop_resolve()).is_none());
    assert!(unresolved_conflicts_notice(&reversed, noop_resolve()).is_none());
}

#[test]
fn update_params_carry_the_bundle_titles_and_descriptions() {
    let merge = ConflictParams::for_update_by_merge();
    let rebase = ConflictParams::for_update_by_rebase();
    let unfinished_merge = ConflictParams::for_unfinished_merge();
    let unmerged = ConflictParams::for_unmerged_files();

    assert!(!merge.reverse);
    assert_eq!(merge.error_notification_title, "Cannot complete update");
    assert_eq!(merge.error_notification_additional_description, "");
    assert_eq!(
        merge.customizer,
        CustomizerKind::DescriptionOverride(
            "Merge conflicts detected. Resolve them before continuing update.".into()
        )
    );
    assert!(rebase.reverse);
    assert_eq!(rebase.error_notification_title, "Cannot continue rebase");
    assert_eq!(
        rebase.customizer,
        CustomizerKind::DescriptionOverride(
            "Merge conflicts detected. Resolve them before continuing rebase.".into()
        )
    );
    assert_eq!(
        rebase.error_notification_additional_description,
        "Then you may <b>continue rebase</b>. <br/> You also may <b>abort rebase</b> to restore the original branch and stop rebasing."
    );
    assert!(!unfinished_merge.reverse);
    assert_eq!(unfinished_merge.error_notification_title, "Cannot update");
    assert_eq!(
        unfinished_merge.error_notification_additional_description,
        ""
    );
    assert_eq!(
        unfinished_merge.customizer,
        CustomizerKind::DescriptionOverride(
            "You have unfinished merge. These conflicts must be resolved before update.".into()
        )
    );
    assert!(!unmerged.reverse);
    assert_eq!(unmerged.error_notification_title, "Cannot update");
    assert_eq!(unmerged.error_notification_additional_description, "");
    assert_eq!(
        unmerged.customizer,
        CustomizerKind::DescriptionOverride(
            "Unmerged files detected. These conflicts must be resolved before update.".into()
        )
    );
}

#[test]
fn unfinished_rebase_params_are_reversed_and_name_the_update_gate() {
    let params = ConflictParams::for_unfinished_rebase();

    assert!(params.reverse);
    assert_eq!(params.error_notification_title, "Cannot update");
    assert_eq!(
        params.customizer,
        CustomizerKind::DescriptionOverride(
            "You have unfinished rebase process. These conflicts must be resolved before update."
                .into()
        )
    );
    assert_eq!(
        params.error_notification_additional_description,
        "Then you may <b>continue rebase</b>. <br/> You also may <b>abort rebase</b> to restore the original branch and stop rebasing."
    );
}

#[test]
fn unmerged_files_before_an_operation_params_describe_the_pending_operation() {
    let params = ConflictParams::for_unmerged_files_before("checkout");

    assert!(!params.reverse);
    assert_eq!(params.error_notification_title, "Unresolved files remain.");
    assert_eq!(
        params.customizer,
        CustomizerKind::DescriptionOverride(
            "The following files have unresolved conflicts. You need to resolve them before checkout."
                .into()
        )
    );
    let notice = unresolved_conflicts_notice(&params, noop_resolve())
        .expect("unresolved files warn after the dialog closes");
    assert_eq!(notice.title.as_deref(), Some("Unresolved files remain."));
    assert_eq!(notice.message, "There are pending unresolved conflicts.");
    assert_eq!(action_labels(&notice), ["Resolve…"]);
}

#[test]
fn rebase_process_params_are_reversed_and_stay_silent() {
    let spec = RebaseCustomizerSpec {
        upstream: Some(RebaseUpstream::Reference("main".to_string())),
        branch: None,
        initial_branch: Some("feature".to_string()),
    };

    let params = ConflictParams::for_rebase_process(spec.clone());

    assert!(params.reverse);
    assert_eq!(params.customizer, CustomizerKind::Rebase(spec));
    assert_eq!(params.error_notification_title, "");
    assert!(unresolved_conflicts_notice(&params, noop_resolve()).is_none());
}

#[test]
fn unstash_params_notify_with_the_unstash_texts() {
    let params = ConflictParams::for_unstash("stash@{1}", "WIP on main");

    assert!(!params.reverse);
    assert_eq!(params.error_notification_title, "Unstashed with conflicts");
    let notice = unresolved_conflicts_notice(&params, noop_resolve())
        .expect("unstash warns about unresolved conflicts");
    assert_eq!(
        notice.title.as_deref(),
        Some("Conflicts were not resolved during unstash")
    );
    assert_eq!(
        notice.message,
        "Unstash is not complete, you have unresolved merges in your working tree."
    );
    assert_eq!(action_labels(&notice), ["Resolve conflicts…"]);
}

#[test]
fn smart_restore_params_put_leading_actions_before_resolve() {
    let view_saved_changes = NoticeAction {
        label: "View saved changes…".into(),
        handler: noop_resolve(),
    };

    let stash = ConflictParams::for_smart_restore("update", "main", true, vec![view_saved_changes]);
    let shelf = ConflictParams::for_smart_restore("update", "main", false, Vec::new());

    let stash_notice =
        unresolved_conflicts_notice(&stash, noop_resolve()).expect("restoring from a stash warns");
    assert!(stash.reverse);
    assert_eq!(
        stash.error_notification_title,
        "Local changes were not restored"
    );
    assert_eq!(
        stash_notice.title.as_deref(),
        Some("Local changes were restored with conflicts")
    );
    assert_eq!(
        stash_notice.message,
        "Your uncommitted changes were saved to stash.\nUnstash is not complete, you have unresolved merges in your working tree\nResolve conflicts and drop the stash."
    );
    assert_eq!(
        action_labels(&stash_notice),
        ["View saved changes…", "Resolve conflicts…"]
    );
    assert!(matches!(shelf.unresolved_notice, UnresolvedNotice::Warning));
}

#[test]
fn default_unresolved_notice_concatenates_body_and_additional_description() {
    let params = ConflictParams::for_update_by_rebase();

    let notice = unresolved_conflicts_notice(&params, noop_resolve())
        .expect("the default notice is a warning");

    assert_eq!(notice.title.as_deref(), Some("Cannot continue rebase"));
    assert_eq!(notice.severity, NoticeSeverity::Warning);
    assert_eq!(
        notice.message,
        "There are pending unresolved conflicts.Then you may continue rebase.\nYou also may abort rebase to restore the original branch and stop rebasing."
    );
    assert_eq!(action_labels(&notice), ["Resolve…"]);
}

#[test]
fn default_unresolved_notice_omits_an_empty_title() {
    let untitled = ConflictParams {
        error_notification_title: "".into(),
        ..ConflictParams::for_update_by_merge()
    };

    let notice = unresolved_conflicts_notice(&untitled, noop_resolve())
        .expect("the default notice is a warning");

    assert_eq!(notice.title, None);
    assert_eq!(notice.message, "There are pending unresolved conflicts.");
}

#[test]
fn notice_after_a_notification_uses_the_pending_title_and_the_additional_description() {
    let rebase = unresolved_after_notification_notice(
        &ConflictParams::for_update_by_rebase(),
        noop_resolve(),
    );
    let merge = unresolved_after_notification_notice(
        &ConflictParams::for_user_merge("feature"),
        noop_resolve(),
    );

    assert_eq!(
        rebase.title.as_deref(),
        Some("Pending Unresolved conflicts")
    );
    assert_eq!(rebase.severity, NoticeSeverity::Warning);
    assert_eq!(
        rebase.message,
        "Then you may continue rebase.\nYou also may abort rebase to restore the original branch and stop rebasing."
    );
    assert_eq!(action_labels(&rebase), ["Resolve…"]);
    assert_eq!(merge.message, "");
}

#[test]
fn exception_notice_prefixes_the_git_error_with_the_check_failure_description() {
    let plain = exception_notice(&ConflictParams::for_unmerged_files(), "fatal: boom");
    let with_hint = exception_notice(&ConflictParams::for_update_by_rebase(), "fatal: boom");

    assert_eq!(plain.title.as_deref(), Some("Cannot update"));
    assert_eq!(plain.severity, NoticeSeverity::Error);
    assert_eq!(
        plain.message,
        "Cannot check the working tree for unmerged files because of an error.\nfatal: boom"
    );
    assert_eq!(
        with_hint.message,
        "Cannot check the working tree for unmerged files because of an error. Then you may continue rebase.\nYou also may abort rebase to restore the original branch and stop rebasing.\nfatal: boom"
    );
    assert!(plain.actions.is_empty());
}

struct ConflictSpec {
    path: &'static str,
    base: Option<&'static str>,
    ours: Option<&'static str>,
    theirs: Option<&'static str>,
}

fn both_modified(path: &'static str) -> ConflictSpec {
    ConflictSpec {
        path,
        base: Some("base\n"),
        ours: Some("ours\n"),
        theirs: Some("theirs\n"),
    }
}

fn both_deleted(path: &'static str) -> ConflictSpec {
    ConflictSpec {
        path,
        base: Some("base\n"),
        ours: None,
        theirs: None,
    }
}

fn dot_git() -> &'static Path {
    Path::new(path!("/project/.git"))
}

fn init_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
        cx.set_global(db::AppDatabase::test_new());
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        editor::init(cx);
    });
}

struct Harness {
    fs: Arc<FakeFs>,
    window: WindowHandle<MultiWorkspace>,
    context: ConflictContext,
}

async fn setup(conflicts: &[ConflictSpec], cx: &mut TestAppContext) -> Harness {
    setup_with_git_directory(json!({}), conflicts, cx).await
}

async fn setup_with_git_directory(
    git_directory: serde_json::Value,
    conflicts: &[ConflictSpec],
    cx: &mut TestAppContext,
) -> Harness {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree(path!("/project"), json!({ ".git": git_directory }))
        .await;
    fs.set_branch_name(dot_git(), Some("main"));
    for conflict in conflicts {
        fs.set_conflict_for_repo(
            dot_git(),
            conflict.path,
            conflict.base,
            conflict.ours,
            conflict.theirs,
        );
    }
    let project = Project::test(fs.clone(), [Path::new(path!("/project"))], cx).await;
    cx.run_until_parked();
    let repository = project
        .read_with(cx, |project, cx| project.active_repository(cx))
        .expect("the project has a repository");
    let window = cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
    let workspace = window
        .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
        .expect("the window is open");
    Harness {
        fs,
        window,
        context: ConflictContext {
            workspace: workspace.downgrade(),
            window: window.into(),
            repository,
        },
    }
}

struct DialogCall {
    paths: Vec<String>,
    texts: DialogTexts,
    description: Task<RichText>,
    reversed: bool,
}

#[derive(Clone, Copy)]
enum DialogBehavior {
    CloseUnresolved { should_finish_merge: bool },
    ResolveThenClose { should_finish_merge: bool },
    DropWithoutResult,
}

#[derive(Default)]
struct Probe {
    dialogs: Rc<RefCell<Vec<DialogCall>>>,
    notices: Rc<RefCell<Vec<BranchNotice>>>,
}

impl Probe {
    fn effects(&self, behavior: DialogBehavior) -> DriverEffects {
        let dialogs = self.dialogs.clone();
        let notices = self.notices.clone();
        DriverEffects {
            open_dialog: Rc::new(move |request: ConflictsDialogRequest, cx: &mut App| {
                let ConflictsDialogRequest {
                    repository,
                    entries,
                    texts,
                    description,
                    reversed,
                    on_closed,
                    ..
                } = request;
                dialogs.borrow_mut().push(DialogCall {
                    paths: entries
                        .iter()
                        .map(|entry| entry.path.as_unix_str().to_string())
                        .collect(),
                    texts,
                    description,
                    reversed,
                });
                match behavior {
                    DialogBehavior::CloseUnresolved {
                        should_finish_merge,
                    } => on_closed(
                        ConflictsDialogResult {
                            processed_files: Vec::new(),
                            should_finish_merge,
                        },
                        cx,
                    ),
                    DialogBehavior::ResolveThenClose {
                        should_finish_merge,
                    } => {
                        let conflicts: Vec<_> = entries
                            .iter()
                            .map(|entry| (entry.path.clone(), entry.stages))
                            .collect();
                        cx.spawn(async move |cx| {
                            let accepted = repository.update(cx, |repository, cx| {
                                repository.accept_conflict_side(conflicts, false, false, cx)
                            });
                            accepted.await.expect("the dialog accepts every conflict");
                            cx.update(|app| {
                                on_closed(
                                    ConflictsDialogResult {
                                        processed_files: Vec::new(),
                                        should_finish_merge,
                                    },
                                    app,
                                )
                            });
                        })
                        .detach();
                    }
                    DialogBehavior::DropWithoutResult => drop(on_closed),
                }
            }),
            notify: Rc::new(move |notice: BranchNotice, _cx: &mut App| {
                notices.borrow_mut().push(notice)
            }),
        }
    }

    fn dialog_count(&self) -> usize {
        self.dialogs.borrow().len()
    }

    fn notice_titles(&self) -> Vec<Option<String>> {
        self.notices
            .borrow()
            .iter()
            .map(|notice| notice.title.as_ref().map(|title| title.to_string()))
            .collect()
    }
}

async fn run(
    harness: &Harness,
    probe: &Probe,
    dialog: DialogBehavior,
    params: ConflictParams,
    behavior: ResolverBehavior,
    mode: ResolveMode,
    cx: &mut TestAppContext,
) -> ResolveOutcome {
    let effects = probe.effects(dialog);
    let context = harness.context.clone();
    let task =
        cx.update(|app| resolve_conflicts_with(effects, context, params, behavior, mode, app));
    task.await
}

fn merge_in_progress(harness: &Harness) -> bool {
    harness
        .fs
        .with_git_state(dot_git(), false, |state| {
            state.refs.contains_key("MERGE_HEAD")
        })
        .expect("the fake repository exists")
}

fn unmerged_path_count(harness: &Harness) -> usize {
    harness
        .fs
        .with_git_state(dot_git(), false, |state| state.unmerged_paths.len())
        .expect("the fake repository exists")
}

fn rebase_in_progress(harness: &Harness) -> bool {
    harness
        .fs
        .with_git_state(dot_git(), false, |state| state.rebase_session.is_some())
        .expect("the fake repository exists")
}

fn commit_count(harness: &Harness) -> usize {
    harness
        .fs
        .with_git_state(dot_git(), false, |state| state.commit_history.len())
        .expect("the fake repository exists")
}

#[gpui::test]
async fn resolver_proceeds_without_opening_a_dialog_when_nothing_is_unmerged(
    cx: &mut TestAppContext,
) {
    let harness = setup(&[], cx).await;
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::CloseUnresolved {
            should_finish_merge: false,
        },
        ConflictParams::for_user_merge("feature"),
        ResolverBehavior::Plain,
        ResolveMode::Initial,
        cx,
    )
    .await;

    assert_eq!(
        outcome,
        ResolveOutcome {
            proceed: true,
            nothing_to_merge: true,
            all_resolved: false,
            should_finish_merge: false,
        }
    );
    assert_eq!(probe.dialog_count(), 0);
    assert!(probe.notices.borrow().is_empty());
}

#[gpui::test]
async fn resolver_called_from_a_notification_skips_the_nothing_to_merge_hook(
    cx: &mut TestAppContext,
) {
    let harness = setup(&[], cx).await;
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::CloseUnresolved {
            should_finish_merge: false,
        },
        ConflictParams::for_update_by_rebase(),
        ResolverBehavior::ContinueRebase,
        ResolveMode::FromNotification,
        cx,
    )
    .await;

    assert!(outcome.proceed);
    assert!(outcome.nothing_to_merge);
    assert_eq!(probe.dialog_count(), 0);
    assert!(probe.notices.borrow().is_empty());
}

#[gpui::test]
async fn resolver_ignores_unmerged_paths_that_do_not_exist_on_disk(cx: &mut TestAppContext) {
    let harness = setup(&[both_deleted("gone.txt")], cx).await;
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::CloseUnresolved {
            should_finish_merge: false,
        },
        ConflictParams::for_unmerged_files(),
        ResolverBehavior::Plain,
        ResolveMode::Initial,
        cx,
    )
    .await;

    assert_eq!(unmerged_path_count(&harness), 1);
    assert!(outcome.proceed);
    assert!(outcome.nothing_to_merge);
    assert_eq!(probe.dialog_count(), 0);
}

#[gpui::test]
async fn resolver_shows_the_sorted_existing_conflicts_and_warns_when_they_remain(
    cx: &mut TestAppContext,
) {
    let harness = setup(
        &[
            both_modified("b.txt"),
            both_deleted("gone.txt"),
            both_modified("a.txt"),
        ],
        cx,
    )
    .await;
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::CloseUnresolved {
            should_finish_merge: false,
        },
        ConflictParams::for_unmerged_files(),
        ResolverBehavior::CommitMergeWhenFinishRequested,
        ResolveMode::Initial,
        cx,
    )
    .await;

    assert_eq!(
        outcome,
        ResolveOutcome {
            proceed: false,
            nothing_to_merge: false,
            all_resolved: false,
            should_finish_merge: false,
        }
    );
    assert_eq!(probe.dialog_count(), 1);
    assert_eq!(probe.dialogs.borrow()[0].paths, ["a.txt", "b.txt"]);
    assert!(!probe.dialogs.borrow()[0].reversed);
    assert!(merge_in_progress(&harness));
    let notices = probe.notices.borrow();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].title.as_deref(), Some("Cannot update"));
    assert_eq!(
        notices[0].message,
        "There are pending unresolved conflicts."
    );
    assert_eq!(action_labels(&notices[0]), ["Resolve…"]);
}

#[gpui::test]
async fn resolve_action_reopens_the_dialog_and_reports_pending_conflicts(cx: &mut TestAppContext) {
    let harness = setup(&[both_modified("a.txt")], cx).await;
    let probe = Probe::default();
    run(
        &harness,
        &probe,
        DialogBehavior::CloseUnresolved {
            should_finish_merge: false,
        },
        ConflictParams::for_unmerged_files(),
        ResolverBehavior::Plain,
        ResolveMode::Initial,
        cx,
    )
    .await;
    let handler = probe.notices.borrow()[0].actions[0].handler.clone();

    harness
        .window
        .update(cx, |_, window, cx| handler(window, cx))
        .expect("the window is open");
    cx.run_until_parked();

    assert_eq!(probe.dialog_count(), 2);
    assert_eq!(
        probe.notice_titles(),
        [
            Some("Cannot update".to_string()),
            Some("Pending Unresolved conflicts".to_string())
        ]
    );
    assert_eq!(probe.notices.borrow()[1].message, "");
}

#[gpui::test]
async fn suppressed_unresolved_notice_stays_silent_until_resolving_from_a_notification(
    cx: &mut TestAppContext,
) {
    let harness = setup(&[both_modified("a.txt")], cx).await;
    let probe = Probe::default();
    let spec = RebaseCustomizerSpec {
        upstream: Some(RebaseUpstream::Reference("main".to_string())),
        branch: None,
        initial_branch: Some("feature".to_string()),
    };

    let initial = run(
        &harness,
        &probe,
        DialogBehavior::CloseUnresolved {
            should_finish_merge: false,
        },
        ConflictParams::for_rebase_process(spec.clone()),
        ResolverBehavior::Plain,
        ResolveMode::Initial,
        cx,
    )
    .await;
    let from_notification = run(
        &harness,
        &probe,
        DialogBehavior::CloseUnresolved {
            should_finish_merge: false,
        },
        ConflictParams::for_rebase_process(spec),
        ResolverBehavior::Plain,
        ResolveMode::FromNotification,
        cx,
    )
    .await;

    assert!(!initial.proceed);
    assert!(!from_notification.proceed);
    assert_eq!(probe.dialog_count(), 2);
    assert!(probe.dialogs.borrow().iter().all(|call| call.reversed));
    assert_eq!(
        probe.notice_titles(),
        [Some("Pending Unresolved conflicts".to_string())]
    );
}

#[gpui::test]
async fn resolver_treats_a_dropped_dialog_as_closed_without_resolving(cx: &mut TestAppContext) {
    let harness = setup(&[both_modified("a.txt")], cx).await;
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::DropWithoutResult,
        ConflictParams::for_user_merge("feature"),
        ResolverBehavior::CommitMergeWhenFinishRequested,
        ResolveMode::Initial,
        cx,
    )
    .await;

    assert!(!outcome.proceed);
    assert!(!outcome.all_resolved);
    assert_eq!(
        probe.notice_titles(),
        [Some("feature Merged with Conflicts".to_string())]
    );
}

#[gpui::test]
async fn resolver_reports_all_resolved_after_the_dialog_stages_everything(cx: &mut TestAppContext) {
    let harness = setup(&[both_modified("a.txt"), both_modified("b.txt")], cx).await;
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::ResolveThenClose {
            should_finish_merge: false,
        },
        ConflictParams::for_resolve_action(false),
        ResolverBehavior::Plain,
        ResolveMode::Initial,
        cx,
    )
    .await;

    assert_eq!(
        outcome,
        ResolveOutcome {
            proceed: true,
            nothing_to_merge: false,
            all_resolved: true,
            should_finish_merge: false,
        }
    );
    assert_eq!(unmerged_path_count(&harness), 0);
    assert!(probe.notices.borrow().is_empty());
}

#[gpui::test]
async fn merge_commit_is_made_when_finishing_was_requested(cx: &mut TestAppContext) {
    let harness = setup(&[both_modified("a.txt")], cx).await;
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::ResolveThenClose {
            should_finish_merge: true,
        },
        ConflictParams::for_user_merge("feature"),
        ResolverBehavior::CommitMergeWhenFinishRequested,
        ResolveMode::Initial,
        cx,
    )
    .await;

    assert!(outcome.proceed);
    assert!(outcome.should_finish_merge);
    assert!(!merge_in_progress(&harness));
    assert_eq!(commit_count(&harness), 1);
    assert!(probe.notices.borrow().is_empty());
}

#[gpui::test]
async fn merge_commit_is_not_made_when_the_dialog_closes_without_finishing(
    cx: &mut TestAppContext,
) {
    let harness = setup(&[both_modified("a.txt")], cx).await;
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::ResolveThenClose {
            should_finish_merge: false,
        },
        ConflictParams::for_user_merge("feature"),
        ResolverBehavior::CommitMergeWhenFinishRequested,
        ResolveMode::Initial,
        cx,
    )
    .await;

    assert!(outcome.proceed);
    assert!(outcome.all_resolved);
    assert!(!outcome.should_finish_merge);
    assert!(merge_in_progress(&harness));
    assert_eq!(commit_count(&harness), 0);
}

#[gpui::test]
async fn finishing_the_merge_does_not_commit_while_conflicts_remain(cx: &mut TestAppContext) {
    let harness = setup(&[both_modified("a.txt")], cx).await;
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::CloseUnresolved {
            should_finish_merge: true,
        },
        ConflictParams::for_user_merge("feature"),
        ResolverBehavior::CommitMergeWhenFinishRequested,
        ResolveMode::Initial,
        cx,
    )
    .await;

    assert!(!outcome.proceed);
    assert!(!outcome.all_resolved);
    assert!(merge_in_progress(&harness));
    assert_eq!(commit_count(&harness), 0);
    assert_eq!(
        probe.notice_titles(),
        [Some("feature Merged with Conflicts".to_string())]
    );
}

#[gpui::test]
async fn update_by_merge_commits_even_without_accept_and_finish(cx: &mut TestAppContext) {
    let harness = setup(&[both_modified("a.txt")], cx).await;
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::ResolveThenClose {
            should_finish_merge: false,
        },
        ConflictParams::for_update_by_merge(),
        ResolverBehavior::CommitMergeAlways,
        ResolveMode::Initial,
        cx,
    )
    .await;

    assert!(outcome.proceed);
    assert!(outcome.all_resolved);
    assert!(!merge_in_progress(&harness));
    assert_eq!(commit_count(&harness), 1);
}

#[gpui::test]
async fn update_by_merge_commits_the_merge_when_nothing_is_left_to_resolve(
    cx: &mut TestAppContext,
) {
    let harness = setup(&[], cx).await;
    harness
        .fs
        .set_operation_in_progress_for_repo(dot_git(), Some(RepositoryOperation::Merge));
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::CloseUnresolved {
            should_finish_merge: false,
        },
        ConflictParams::for_update_by_merge(),
        ResolverBehavior::CommitMergeAlways,
        ResolveMode::Initial,
        cx,
    )
    .await;

    assert_eq!(
        outcome,
        ResolveOutcome {
            proceed: true,
            nothing_to_merge: true,
            all_resolved: false,
            should_finish_merge: false,
        }
    );
    assert_eq!(probe.dialog_count(), 0);
    assert!(!merge_in_progress(&harness));
    assert_eq!(commit_count(&harness), 1);
    assert!(probe.notices.borrow().is_empty());
}

#[gpui::test]
async fn resolving_from_a_notification_without_finishing_makes_no_commit(cx: &mut TestAppContext) {
    let harness = setup(&[both_modified("a.txt")], cx).await;
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::ResolveThenClose {
            should_finish_merge: false,
        },
        ConflictParams::for_user_merge("feature"),
        ResolverBehavior::CommitMergeWhenFinishRequested,
        ResolveMode::FromNotification,
        cx,
    )
    .await;

    assert!(outcome.proceed);
    assert!(outcome.all_resolved);
    assert!(merge_in_progress(&harness));
    assert_eq!(commit_count(&harness), 0);
}

#[gpui::test]
async fn resolving_from_a_notification_and_finishing_commits_the_merge(cx: &mut TestAppContext) {
    let harness = setup(&[both_modified("a.txt")], cx).await;
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::ResolveThenClose {
            should_finish_merge: true,
        },
        ConflictParams::for_user_merge("feature"),
        ResolverBehavior::CommitMergeWhenFinishRequested,
        ResolveMode::FromNotification,
        cx,
    )
    .await;

    assert!(outcome.proceed);
    assert!(outcome.should_finish_merge);
    assert!(!merge_in_progress(&harness));
    assert_eq!(commit_count(&harness), 1);
}

#[gpui::test]
async fn merge_commit_is_skipped_when_no_merge_is_in_progress(cx: &mut TestAppContext) {
    let harness = setup(&[both_modified("a.txt")], cx).await;
    harness
        .fs
        .set_operation_in_progress_for_repo(dot_git(), None);
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::ResolveThenClose {
            should_finish_merge: true,
        },
        ConflictParams::for_unmerged_files(),
        ResolverBehavior::CommitMergeWhenFinishRequested,
        ResolveMode::Initial,
        cx,
    )
    .await;

    assert!(outcome.proceed);
    assert!(probe.notices.borrow().is_empty());
    assert_eq!(unmerged_path_count(&harness), 0);
    assert_eq!(commit_count(&harness), 0);
}

#[gpui::test]
async fn failed_merge_commit_is_reported_and_stops_the_resolver(cx: &mut TestAppContext) {
    let harness = setup(&[both_modified("a.txt"), both_deleted("gone.txt")], cx).await;
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::ResolveThenClose {
            should_finish_merge: true,
        },
        ConflictParams::for_unmerged_files(),
        ResolverBehavior::CommitMergeWhenFinishRequested,
        ResolveMode::Initial,
        cx,
    )
    .await;

    assert!(!outcome.proceed);
    assert!(outcome.all_resolved);
    assert!(merge_in_progress(&harness));
    assert_eq!(commit_count(&harness), 0);
    let notices = probe.notices.borrow();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].severity, NoticeSeverity::Error);
    assert_eq!(notices[0].title.as_deref(), Some("Cannot update"));
    assert!(
        notices[0]
            .message
            .starts_with("Cannot check the working tree for unmerged files because of an error.\n")
    );
    assert!(
        notices[0]
            .message
            .ends_with("fatal: Exiting because of an unresolved conflict.")
    );
}

#[gpui::test]
async fn unfinished_rebase_without_conflicts_is_continued_immediately(cx: &mut TestAppContext) {
    let harness = setup(&[], cx).await;
    harness
        .fs
        .set_operation_in_progress_for_repo(dot_git(), Some(RepositoryOperation::Rebase));
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::CloseUnresolved {
            should_finish_merge: false,
        },
        ConflictParams::for_update_by_rebase(),
        ResolverBehavior::ContinueRebase,
        ResolveMode::Initial,
        cx,
    )
    .await;

    assert!(outcome.proceed);
    assert!(outcome.nothing_to_merge);
    assert_eq!(probe.dialog_count(), 0);
    assert!(!rebase_in_progress(&harness));
}

#[gpui::test]
async fn rebase_is_continued_after_every_conflict_is_resolved(cx: &mut TestAppContext) {
    let harness = setup(&[both_modified("a.txt")], cx).await;
    harness
        .fs
        .set_operation_in_progress_for_repo(dot_git(), Some(RepositoryOperation::Rebase));
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::ResolveThenClose {
            should_finish_merge: false,
        },
        ConflictParams::for_update_by_rebase(),
        ResolverBehavior::ContinueRebase,
        ResolveMode::Initial,
        cx,
    )
    .await;

    assert!(outcome.proceed);
    assert!(outcome.all_resolved);
    assert_eq!(probe.dialog_count(), 1);
    assert!(probe.dialogs.borrow()[0].reversed);
    assert!(!rebase_in_progress(&harness));
    assert!(probe.notices.borrow().is_empty());
}

#[gpui::test]
async fn rebase_is_not_continued_while_conflicts_remain(cx: &mut TestAppContext) {
    let harness = setup(&[both_modified("a.txt")], cx).await;
    harness
        .fs
        .set_operation_in_progress_for_repo(dot_git(), Some(RepositoryOperation::Rebase));
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::CloseUnresolved {
            should_finish_merge: false,
        },
        ConflictParams::for_update_by_rebase(),
        ResolverBehavior::ContinueRebase,
        ResolveMode::Initial,
        cx,
    )
    .await;

    assert!(!outcome.proceed);
    assert!(rebase_in_progress(&harness));
    assert_eq!(
        probe.notice_titles(),
        [Some("Cannot continue rebase".to_string())]
    );
}

#[gpui::test]
async fn failed_rebase_continue_is_reported_as_a_rebase_error(cx: &mut TestAppContext) {
    let harness = setup(&[], cx).await;
    harness
        .fs
        .set_operation_in_progress_for_repo(dot_git(), Some(RepositoryOperation::Rebase));
    harness.fs.set_simulated_git_failure_for_repo(
        dot_git(),
        FakeGitOperation::Rebase,
        Some((GitFailureKind::Other, "fatal: boom")),
    );
    let probe = Probe::default();

    let outcome = run(
        &harness,
        &probe,
        DialogBehavior::CloseUnresolved {
            should_finish_merge: false,
        },
        ConflictParams::for_update_by_rebase(),
        ResolverBehavior::ContinueRebase,
        ResolveMode::Initial,
        cx,
    )
    .await;

    assert!(!outcome.proceed);
    assert!(outcome.nothing_to_merge);
    assert!(rebase_in_progress(&harness));
    let notices = probe.notices.borrow();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].severity, NoticeSeverity::Error);
    assert_eq!(notices[0].title.as_deref(), Some("Rebase Error"));
    assert_eq!(notices[0].message, "fatal: boom");
}

#[gpui::test]
async fn dialog_receives_the_texts_and_description_of_the_merge_in_progress(
    cx: &mut TestAppContext,
) {
    let merge_head = "2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e";
    let harness = setup(&[both_modified("a.txt")], cx).await;
    harness
        .fs
        .set_ref_for_repo(dot_git(), "MERGE_HEAD", merge_head);
    harness
        .fs
        .set_merge_base_commit_for_repo(dot_git(), "abc", merge_head, "base0000");
    let probe = Probe::default();

    run(
        &harness,
        &probe,
        DialogBehavior::CloseUnresolved {
            should_finish_merge: false,
        },
        ConflictParams::for_user_merge("feature"),
        ResolverBehavior::CommitMergeWhenFinishRequested,
        ResolveMode::Initial,
        cx,
    )
    .await;

    let call = probe
        .dialogs
        .borrow_mut()
        .pop()
        .expect("the dialog was opened");
    let description = call.description.await;
    assert_eq!(
        call.texts,
        texts(
            "Yours (main)",
            "Theirs (2b3c4d5e)",
            range_title(
                RichText::plain("Changes from ").with_bold("main"),
                "base0000",
                "abc"
            ),
            range_title(
                RichText::plain("Changes from ").with_bold("2b3c4d5e"),
                "base0000",
                merge_head
            ),
        )
    );
    assert_eq!(
        description,
        RichText::plain("Merging branch ")
            .with_bold("2b3c4d5e")
            .with_plain(" into branch ")
            .with_bold("main")
    );
}

#[gpui::test]
async fn facts_read_the_rebased_branch_and_merge_message_from_the_git_directory(
    cx: &mut TestAppContext,
) {
    let harness = setup_with_git_directory(
        json!({
            "rebase-merge": { "head-name": "refs/heads/feature\n" },
            "MERGE_MSG": "Merge branch 'feature' into main\n",
        }),
        &[],
        cx,
    )
    .await;
    harness
        .fs
        .set_operation_in_progress_for_repo(dot_git(), Some(RepositoryOperation::Rebase));
    let repository = harness.context.repository.clone();
    let fs: Arc<dyn Fs> = harness.fs.clone();

    let facts = cx
        .update(|app| app.spawn(async move |cx| load_repository_facts(&repository, &fs, cx).await))
        .await
        .expect("the facts load");

    assert_eq!(facts.operation, Some(RepositoryOperation::Rebase));
    assert_eq!(facts.branch.as_deref(), Some("feature"));
    assert_eq!(facts.head.as_deref(), Some("abc"));
    assert_eq!(
        facts.merge_message.as_deref(),
        Some("Merge branch 'feature' into main\n")
    );
    assert_eq!(facts.merge_head, None);
}

#[gpui::test]
async fn facts_use_the_checked_out_branch_outside_of_a_rebase(cx: &mut TestAppContext) {
    let harness = setup_with_git_directory(
        json!({ "rebase-merge": { "head-name": "refs/heads/ignored\n" } }),
        &[both_modified("a.txt")],
        cx,
    )
    .await;
    harness
        .fs
        .set_ref_for_repo(dot_git(), "MERGE_HEAD", MERGE_HEAD_SHA);
    let repository = harness.context.repository.clone();
    let fs: Arc<dyn Fs> = harness.fs.clone();

    let facts = cx
        .update(|app| app.spawn(async move |cx| load_repository_facts(&repository, &fs, cx).await))
        .await
        .expect("the facts load");

    assert_eq!(facts.operation, Some(RepositoryOperation::Merge));
    assert_eq!(facts.branch.as_deref(), Some("main"));
    assert_eq!(
        facts.merge_head.map(|commit| commit.sha),
        Some(MERGE_HEAD_SHA.to_string())
    );
}
