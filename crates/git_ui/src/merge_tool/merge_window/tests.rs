use std::{cell::RefCell, rc::Rc};

use gpui::{
    Action, AnyWindowHandle, AppContext as _, Entity, TestAppContext, VisualTestContext,
    WeakEntity, WindowHandle,
};
use merge_diff::{Side, ThreeSide};
use project::{FakeFs, Project};
use settings::SettingsStore;
use workspace::{CloseWindow, MultiWorkspace};

use super::{
    MergePaneTitle, MergeRequest, MergeResult, MergeWindow,
    finish::{ConfirmationIcon, ConfirmationText},
    open_merge_window_with,
};
use crate::merge_tool::{
    conflict_resolution::rich_text::RichText,
    dialog_window::{DialogOwner, DialogWindow, DialogWindowShell},
    merge_model::{
        FileConflictType, MergeBuffers, MergeConflictModel, MergeTexts, messages::MergeStatus,
    },
    merge_viewer::{
        MergeViewer, RediffState,
        actions::{
            AcceptLeft, AcceptLeftSide, AcceptRight, AcceptRightSide, ApplyChanges,
            ApplyNonConflictingAll, ApplyNonConflictingLeft, ApplyNonConflictingRight,
            IgnoreLeftSide, NextDifference, PreviousDifference, ResolveUsingLeft,
            RevertConflictResolution, SaveAndClose,
        },
    },
};

const BASE: &str = "a\nb\nc\n";
const LEFT_CONFLICT: &str = "a\nL\nc\n";
const RIGHT_CONFLICT: &str = "a\nR\nc\n";
const LEFT_ONLY_CHANGE: &str = "a\nB\nc\n";
const ORIGINAL: &str = "ORIGINAL\n";
const TEN_BASE: &str = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
const TEN_FIRST_EDITED: &str = "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\n";
const TEN_SIXTH_EDITED: &str = "a\nb\nc\nd\ne\nf\nG\nh\ni\nj\n";
const TEN_BOTH_EDITED: &str = "a\nB\nc\nd\ne\nf\nG\nh\ni\nj\n";

struct Harness {
    window: WindowHandle<DialogWindowShell>,
    content: WeakEntity<MergeWindow>,
    model: Entity<MergeConflictModel>,
    results: Rc<RefCell<Vec<MergeResult>>>,
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

fn plain_titles() -> [MergePaneTitle; 3] {
    [
        MergePaneTitle::new(RichText::plain("Left")),
        MergePaneTitle::new(RichText::plain("Result")),
        MergePaneTitle::new(RichText::plain("Right")),
    ]
}

async fn open_merge_unsettled(
    cx: &mut TestAppContext,
    left: &str,
    base: &str,
    right: &str,
    iterative: bool,
) -> Harness {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    let project = Project::test(fs, [], cx).await;
    let owner_window = cx.add_window(|window, cx| MultiWorkspace::test_new(project, window, cx));
    let workspace = owner_window
        .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
        .expect("the owner window is open");
    let owner = DialogOwner::workspace(workspace.downgrade(), owner_window.into());
    let texts = MergeTexts::new(left, base, right);
    let model = cx.update(|cx| {
        let buffers = MergeBuffers::detached(&texts, None, cx);
        cx.new(|cx| MergeConflictModel::new(buffers, &texts, FileConflictType::Default, cx))
    });
    let results: Rc<RefCell<Vec<MergeResult>>> = Rc::default();
    let recorded = results.clone();
    let opened: Rc<RefCell<Option<DialogWindow<MergeWindow>>>> = Rc::default();
    let opened_slot = opened.clone();
    let request = MergeRequest {
        window_title: "Merge Revisions for /tmp/file.txt".into(),
        pane_titles: plain_titles(),
        model: model.clone(),
        file_name: "file.txt".into(),
        iterative,
        original_content: ORIGINAL.to_string(),
        on_finished: Box::new(move |result, _| recorded.borrow_mut().push(result)),
    };
    cx.update(|cx| {
        open_merge_window_with(
            request,
            owner,
            cx,
            Some(Box::new(move |dialog, _| {
                *opened_slot.borrow_mut() = Some(dialog);
            })),
        );
    });
    let dialog = opened.take().expect("the merge window opened");
    Harness {
        window: dialog.window,
        content: dialog.content.downgrade(),
        model,
        results,
    }
}

fn settle(harness: &Harness, cx: &mut TestAppContext) {
    cx.run_until_parked();
    let mut dialog_cx = VisualTestContext::from_window(harness.window.into(), cx);
    let delivered = dialog_cx.update(|window, cx| window.simulate_next_frame(cx));
    dialog_cx.run_until_parked();
    assert!(
        delivered > 0,
        "the first frame carries the viewer's initial caret placement"
    );
    assert_eq!(rediff_state(harness, cx), RediffState::Finished);
}

async fn open_merge(
    cx: &mut TestAppContext,
    left: &str,
    base: &str,
    right: &str,
    iterative: bool,
) -> Harness {
    let harness = open_merge_unsettled(cx, left, base, right, iterative).await;
    settle(&harness, cx);
    harness
}

fn dispatch(harness: &Harness, action: impl Action, cx: &mut TestAppContext) {
    let mut dialog_cx = VisualTestContext::from_window(harness.window.into(), cx);
    dialog_cx.dispatch_action(action);
    dialog_cx.run_until_parked();
}

fn remove_window_without_asking(harness: &Harness, cx: &mut TestAppContext) {
    AnyWindowHandle::from(harness.window)
        .update(cx, |_, window, _| window.remove_window())
        .expect("the merge window is open");
    cx.run_until_parked();
}

fn result_text(harness: &Harness, cx: &TestAppContext) -> String {
    harness
        .model
        .read_with(cx, |model, cx| model.current_result_text(cx))
}

fn merge_status(harness: &Harness, cx: &TestAppContext) -> MergeStatus {
    harness.model.read_with(cx, |model, _| model.status())
}

fn resolved_sides(harness: &Harness, cx: &TestAppContext) -> Option<(bool, bool)> {
    harness.model.read_with(cx, |model, _| {
        model.change(0).map(|change| {
            (
                change.is_side_resolved(Side::Left),
                change.is_side_resolved(Side::Right),
            )
        })
    })
}

fn is_window_open(harness: &Harness, cx: &TestAppContext) -> bool {
    cx.windows()
        .contains(&AnyWindowHandle::from(harness.window))
}

fn finished_results(harness: &Harness) -> Vec<MergeResult> {
    harness.results.borrow().clone()
}

fn shown_confirmation(harness: &Harness, cx: &TestAppContext) -> Option<ConfirmationText> {
    harness
        .content
        .read_with(cx, |merge_window, _| merge_window.shown_confirmation())
        .ok()
        .flatten()
}

fn confirmation_title(harness: &Harness, cx: &TestAppContext) -> Option<String> {
    shown_confirmation(harness, cx).map(|confirmation| confirmation.title)
}

fn viewer_of(harness: &Harness, cx: &TestAppContext) -> Entity<MergeViewer> {
    harness
        .content
        .read_with(cx, |merge_window, _| merge_window.viewer().clone())
        .expect("the merge window is open")
}

fn rediff_state(harness: &Harness, cx: &TestAppContext) -> RediffState {
    viewer_of(harness, cx).read_with(cx, |viewer, _| viewer.rediff_state())
}

fn caret_line(harness: &Harness, cx: &mut TestAppContext) -> i64 {
    let viewer = viewer_of(harness, cx);
    let mut dialog_cx = VisualTestContext::from_window(harness.window.into(), cx);
    let (caret_line, _) = viewer.update(&mut dialog_cx, |viewer, cx| {
        viewer.caret_line_and_line_count(ThreeSide::Base, cx)
    });
    caret_line
}

#[gpui::test]
async fn merge_tool_window_opens_on_the_first_conflict_with_the_base_text_in_the_result(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, LEFT_CONFLICT, BASE, RIGHT_CONFLICT, true).await;
    assert_eq!(result_text(&harness, cx), BASE);
    assert_eq!(
        merge_status(&harness, cx),
        MergeStatus::Differences("No changes, 1 conflict".to_string())
    );
    assert_eq!(caret_line(&harness, cx), 1);
    assert!(is_window_open(&harness, cx));
    assert!(finished_results(&harness).is_empty());
}

#[gpui::test]
async fn merge_tool_window_accept_left_replaces_the_result_with_exactly_the_left_text(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, TEN_FIRST_EDITED, TEN_BASE, TEN_SIXTH_EDITED, true).await;
    dispatch(&harness, AcceptLeft, cx);
    assert_eq!(finished_results(&harness), vec![MergeResult::Left]);
    assert_eq!(result_text(&harness, cx), TEN_FIRST_EDITED);
    assert!(!is_window_open(&harness, cx));
}

#[gpui::test]
async fn merge_tool_window_accept_right_replaces_the_result_with_exactly_the_right_text(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, TEN_FIRST_EDITED, TEN_BASE, TEN_SIXTH_EDITED, true).await;
    dispatch(&harness, AcceptRight, cx);
    assert_eq!(finished_results(&harness), vec![MergeResult::Right]);
    assert_eq!(result_text(&harness, cx), TEN_SIXTH_EDITED);
    assert!(!is_window_open(&harness, cx));
}

#[gpui::test]
async fn merge_tool_window_apply_changes_after_resolving_everything_finishes_without_asking(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, LEFT_ONLY_CHANGE, BASE, BASE, true).await;
    dispatch(&harness, ApplyNonConflictingAll, cx);
    assert_eq!(result_text(&harness, cx), LEFT_ONLY_CHANGE);
    assert_eq!(
        merge_status(&harness, cx),
        MergeStatus::AllConflictsResolved
    );
    assert!(is_window_open(&harness, cx));
    dispatch(&harness, ApplyChanges, cx);
    assert_eq!(finished_results(&harness), vec![MergeResult::Resolved]);
    assert_eq!(result_text(&harness, cx), LEFT_ONLY_CHANGE);
    assert!(!is_window_open(&harness, cx));
}

#[gpui::test]
async fn merge_tool_window_apply_changes_in_the_iterative_flow_asks_before_saving_unresolved_conflicts(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, LEFT_CONFLICT, BASE, RIGHT_CONFLICT, true).await;
    dispatch(&harness, ApplyChanges, cx);
    assert_eq!(
        shown_confirmation(&harness, cx),
        Some(ConfirmationText {
            title: "Unprocessed Changes".to_string(),
            message: "1 unresolved conflict still remains. The file will be saved exactly as shown in the Result pane.".to_string(),
            yes_label: "Save Current Result".to_string(),
            no_label: "Back to Resolving".to_string(),
            icon: Some(ConfirmationIcon::Warning),
        })
    );
    assert!(finished_results(&harness).is_empty());
    assert!(is_window_open(&harness, cx));

    dispatch(&harness, menu::Cancel, cx);
    assert_eq!(shown_confirmation(&harness, cx), None);
    assert!(finished_results(&harness).is_empty());
    assert!(is_window_open(&harness, cx));
    assert_eq!(
        merge_status(&harness, cx),
        MergeStatus::Differences("No changes, 1 conflict".to_string())
    );

    dispatch(&harness, ApplyChanges, cx);
    dispatch(&harness, menu::Confirm, cx);
    assert_eq!(finished_results(&harness), vec![MergeResult::Resolved]);
    assert_eq!(result_text(&harness, cx), BASE);
    assert_eq!(
        merge_status(&harness, cx),
        MergeStatus::AllConflictsResolved
    );
    assert!(!is_window_open(&harness, cx));
}

#[gpui::test]
async fn merge_tool_window_apply_changes_in_the_one_shot_flow_asks_in_its_own_wording_and_keeps_the_result(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, LEFT_CONFLICT, BASE, RIGHT_CONFLICT, false).await;
    dispatch(&harness, ApplyChanges, cx);
    assert_eq!(
        shown_confirmation(&harness, cx),
        Some(ConfirmationText {
            title: "Apply Changes".to_string(),
            message: "There is one conflict left unprocessed.\nSave changes and mark the conflict resolved anyway?".to_string(),
            yes_label: "Apply Changes and Mark Resolved".to_string(),
            no_label: "Continue Merge".to_string(),
            icon: Some(ConfirmationIcon::Question),
        })
    );
    assert!(finished_results(&harness).is_empty());

    dispatch(&harness, menu::Confirm, cx);
    assert_eq!(finished_results(&harness), vec![MergeResult::Resolved]);
    assert_eq!(result_text(&harness, cx), BASE);
    assert_eq!(
        merge_status(&harness, cx),
        MergeStatus::AllConflictsResolved
    );
    assert!(!is_window_open(&harness, cx));
}

#[gpui::test]
async fn merge_tool_window_apply_changes_is_ignored_while_the_differences_are_still_computed(
    cx: &mut TestAppContext,
) {
    let harness = open_merge_unsettled(cx, LEFT_CONFLICT, BASE, RIGHT_CONFLICT, true).await;
    assert_eq!(rediff_state(&harness, cx), RediffState::Running);
    dispatch(&harness, ApplyChanges, cx);
    assert_eq!(shown_confirmation(&harness, cx), None);
    assert!(finished_results(&harness).is_empty());
    assert!(is_window_open(&harness, cx));

    settle(&harness, cx);
    dispatch(&harness, ApplyChanges, cx);
    assert_eq!(
        confirmation_title(&harness, cx),
        Some("Unprocessed Changes".to_string())
    );
}

#[gpui::test]
async fn merge_tool_window_save_and_close_in_the_iterative_flow_keeps_the_edited_result(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, LEFT_CONFLICT, BASE, RIGHT_CONFLICT, true).await;
    dispatch(&harness, ResolveUsingLeft, cx);
    assert_eq!(result_text(&harness, cx), LEFT_CONFLICT);
    dispatch(&harness, SaveAndClose, cx);
    assert_eq!(finished_results(&harness), vec![MergeResult::Cancel]);
    assert_eq!(result_text(&harness, cx), LEFT_CONFLICT);
    assert!(!is_window_open(&harness, cx));
}

#[gpui::test]
async fn merge_tool_window_cancel_in_the_one_shot_flow_asks_then_restores_the_original(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, LEFT_CONFLICT, BASE, RIGHT_CONFLICT, false).await;
    dispatch(&harness, ResolveUsingLeft, cx);
    dispatch(&harness, SaveAndClose, cx);
    assert_eq!(
        confirmation_title(&harness, cx),
        Some("Cancel Merge".to_string())
    );
    assert!(finished_results(&harness).is_empty());
    assert_eq!(result_text(&harness, cx), LEFT_CONFLICT);
    assert!(is_window_open(&harness, cx));

    dispatch(&harness, menu::Confirm, cx);
    assert_eq!(finished_results(&harness), vec![MergeResult::Cancel]);
    assert_eq!(result_text(&harness, cx), ORIGINAL);
    assert!(!is_window_open(&harness, cx));
}

#[gpui::test]
async fn merge_tool_window_cancel_without_edits_in_the_one_shot_flow_restores_the_original_without_asking(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, LEFT_CONFLICT, BASE, RIGHT_CONFLICT, false).await;
    dispatch(&harness, SaveAndClose, cx);
    assert_eq!(finished_results(&harness), vec![MergeResult::Cancel]);
    assert_eq!(result_text(&harness, cx), ORIGINAL);
    assert!(!is_window_open(&harness, cx));
}

#[gpui::test]
async fn merge_tool_window_close_shortcut_in_the_iterative_flow_cancels_without_asking(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, LEFT_CONFLICT, BASE, RIGHT_CONFLICT, true).await;
    dispatch(&harness, ResolveUsingLeft, cx);
    dispatch(&harness, CloseWindow, cx);
    assert_eq!(finished_results(&harness), vec![MergeResult::Cancel]);
    assert_eq!(result_text(&harness, cx), LEFT_CONFLICT);
    assert!(!is_window_open(&harness, cx));
}

#[gpui::test]
async fn merge_tool_window_close_shortcut_after_edits_in_the_one_shot_flow_asks_first(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, LEFT_CONFLICT, BASE, RIGHT_CONFLICT, false).await;
    dispatch(&harness, ResolveUsingLeft, cx);
    dispatch(&harness, CloseWindow, cx);
    assert_eq!(
        confirmation_title(&harness, cx),
        Some("Cancel Merge".to_string())
    );
    assert!(is_window_open(&harness, cx));
    assert!(finished_results(&harness).is_empty());

    dispatch(&harness, CloseWindow, cx);
    assert!(confirmation_title(&harness, cx).is_some());
    assert!(is_window_open(&harness, cx));

    dispatch(&harness, menu::Cancel, cx);
    assert_eq!(confirmation_title(&harness, cx), None);
    assert!(is_window_open(&harness, cx));
    assert!(finished_results(&harness).is_empty());
    assert_eq!(result_text(&harness, cx), LEFT_CONFLICT);
}

#[gpui::test]
async fn merge_tool_window_escape_after_edits_in_the_one_shot_flow_asks_and_no_keeps_the_window(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, LEFT_CONFLICT, BASE, RIGHT_CONFLICT, false).await;
    dispatch(&harness, ResolveUsingLeft, cx);
    dispatch(&harness, editor::actions::Cancel, cx);
    assert_eq!(
        confirmation_title(&harness, cx),
        Some("Cancel Merge".to_string())
    );
    dispatch(&harness, menu::Cancel, cx);
    assert_eq!(confirmation_title(&harness, cx), None);
    assert!(is_window_open(&harness, cx));
    assert!(finished_results(&harness).is_empty());
    assert_eq!(result_text(&harness, cx), LEFT_CONFLICT);
}

#[gpui::test]
async fn merge_tool_window_escape_without_edits_closes_the_window_as_cancelled(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, LEFT_CONFLICT, BASE, RIGHT_CONFLICT, false).await;
    dispatch(&harness, editor::actions::Cancel, cx);
    assert_eq!(finished_results(&harness), vec![MergeResult::Cancel]);
    assert_eq!(result_text(&harness, cx), ORIGINAL);
    assert!(!is_window_open(&harness, cx));
}

#[gpui::test]
async fn merge_tool_window_accept_right_after_edits_asks_before_discarding_them(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, LEFT_CONFLICT, BASE, RIGHT_CONFLICT, true).await;
    dispatch(&harness, ResolveUsingLeft, cx);
    dispatch(&harness, AcceptRight, cx);
    assert_eq!(
        confirmation_title(&harness, cx),
        Some("Accept Right".to_string())
    );
    assert!(finished_results(&harness).is_empty());
    assert_eq!(result_text(&harness, cx), LEFT_CONFLICT);

    dispatch(&harness, menu::Cancel, cx);
    assert_eq!(confirmation_title(&harness, cx), None);
    assert!(is_window_open(&harness, cx));
    assert_eq!(result_text(&harness, cx), LEFT_CONFLICT);

    dispatch(&harness, AcceptRight, cx);
    dispatch(&harness, menu::Confirm, cx);
    assert_eq!(finished_results(&harness), vec![MergeResult::Right]);
    assert_eq!(result_text(&harness, cx), RIGHT_CONFLICT);
    assert!(!is_window_open(&harness, cx));
}

#[gpui::test]
async fn merge_tool_window_ignoring_one_side_then_accepting_the_other_resolves_the_conflict(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, LEFT_CONFLICT, BASE, RIGHT_CONFLICT, true).await;
    assert_eq!(resolved_sides(&harness, cx), Some((false, false)));
    dispatch(&harness, IgnoreLeftSide, cx);
    assert_eq!(resolved_sides(&harness, cx), Some((true, false)));
    assert_eq!(result_text(&harness, cx), BASE);
    assert_eq!(
        merge_status(&harness, cx),
        MergeStatus::Differences("No changes, 1 conflict".to_string())
    );

    dispatch(&harness, AcceptRightSide, cx);
    assert_eq!(result_text(&harness, cx), RIGHT_CONFLICT);
    assert_eq!(
        merge_status(&harness, cx),
        MergeStatus::AllConflictsResolved
    );
}

#[gpui::test]
async fn merge_tool_window_accept_left_side_applies_only_the_change_under_the_caret(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, TEN_BOTH_EDITED, TEN_BASE, TEN_BASE, true).await;
    dispatch(&harness, AcceptLeftSide, cx);
    assert_eq!(result_text(&harness, cx), TEN_FIRST_EDITED);
    assert_eq!(
        merge_status(&harness, cx),
        MergeStatus::Differences("1 change, No conflicts".to_string())
    );

    dispatch(&harness, NextDifference, cx);
    dispatch(&harness, AcceptLeftSide, cx);
    assert_eq!(result_text(&harness, cx), TEN_BOTH_EDITED);
    assert_eq!(
        merge_status(&harness, cx),
        MergeStatus::AllConflictsResolved
    );
}

#[gpui::test]
async fn merge_tool_window_next_and_previous_difference_move_the_caret_and_stop_at_the_ends(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, TEN_BOTH_EDITED, TEN_BASE, TEN_BASE, true).await;
    assert_eq!(caret_line(&harness, cx), 1);

    dispatch(&harness, NextDifference, cx);
    assert_eq!(caret_line(&harness, cx), 6);
    dispatch(&harness, NextDifference, cx);
    assert_eq!(caret_line(&harness, cx), 6);

    dispatch(&harness, PreviousDifference, cx);
    assert_eq!(caret_line(&harness, cx), 1);
    dispatch(&harness, PreviousDifference, cx);
    assert_eq!(caret_line(&harness, cx), 1);
}

#[gpui::test]
async fn merge_tool_window_apply_non_conflicting_changes_from_one_side_leaves_the_other_sides_changes(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, TEN_FIRST_EDITED, TEN_BASE, TEN_SIXTH_EDITED, true).await;
    assert_eq!(
        merge_status(&harness, cx),
        MergeStatus::Differences("2 changes, No conflicts".to_string())
    );

    dispatch(&harness, ApplyNonConflictingLeft, cx);
    assert_eq!(result_text(&harness, cx), TEN_FIRST_EDITED);
    assert_eq!(
        merge_status(&harness, cx),
        MergeStatus::Differences("1 change, No conflicts".to_string())
    );

    dispatch(&harness, ApplyNonConflictingRight, cx);
    assert_eq!(result_text(&harness, cx), TEN_BOTH_EDITED);
    assert_eq!(
        merge_status(&harness, cx),
        MergeStatus::AllConflictsResolved
    );
}

#[gpui::test]
async fn merge_tool_window_reverting_the_resolution_asks_and_restores_the_conflict(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, LEFT_CONFLICT, BASE, RIGHT_CONFLICT, true).await;
    dispatch(&harness, ResolveUsingLeft, cx);
    assert_eq!(
        merge_status(&harness, cx),
        MergeStatus::AllConflictsResolved
    );

    dispatch(&harness, RevertConflictResolution, cx);
    assert_eq!(
        confirmation_title(&harness, cx),
        Some("Confirm Revert".to_string())
    );
    assert_eq!(result_text(&harness, cx), LEFT_CONFLICT);

    dispatch(&harness, menu::Cancel, cx);
    assert_eq!(confirmation_title(&harness, cx), None);
    assert_eq!(result_text(&harness, cx), LEFT_CONFLICT);
    assert_eq!(
        merge_status(&harness, cx),
        MergeStatus::AllConflictsResolved
    );

    dispatch(&harness, RevertConflictResolution, cx);
    dispatch(&harness, menu::Confirm, cx);
    assert_eq!(confirmation_title(&harness, cx), None);
    assert_eq!(result_text(&harness, cx), BASE);
    assert_eq!(
        merge_status(&harness, cx),
        MergeStatus::Differences("No changes, 1 conflict".to_string())
    );
    assert!(finished_results(&harness).is_empty());
    assert!(is_window_open(&harness, cx));
}

#[gpui::test]
async fn merge_tool_window_balloon_link_finishes_as_resolved_without_asking(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, LEFT_CONFLICT, BASE, RIGHT_CONFLICT, true).await;
    dispatch(&harness, ResolveUsingLeft, cx);
    let viewer = viewer_of(&harness, cx);
    let mut dialog_cx = VisualTestContext::from_window(harness.window.into(), cx);
    viewer.update(&mut dialog_cx, |viewer, cx| {
        viewer.request_finish_from_balloon(cx)
    });
    dialog_cx.run_until_parked();
    assert_eq!(finished_results(&harness), vec![MergeResult::Resolved]);
    assert_eq!(result_text(&harness, cx), LEFT_CONFLICT);
    assert!(!is_window_open(&harness, cx));
}

#[gpui::test]
async fn merge_tool_window_released_after_finishing_does_not_finish_a_second_time(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, LEFT_CONFLICT, BASE, RIGHT_CONFLICT, false).await;
    dispatch(&harness, AcceptLeft, cx);
    assert!(harness.content.upgrade().is_none());
    assert_eq!(finished_results(&harness), vec![MergeResult::Left]);
    assert_eq!(result_text(&harness, cx), LEFT_CONFLICT);
}

#[gpui::test]
async fn merge_tool_window_removed_without_a_decision_reports_cancel_and_restores_the_original(
    cx: &mut TestAppContext,
) {
    let harness = open_merge(cx, LEFT_CONFLICT, BASE, RIGHT_CONFLICT, false).await;
    dispatch(&harness, ResolveUsingLeft, cx);
    assert_eq!(result_text(&harness, cx), LEFT_CONFLICT);
    remove_window_without_asking(&harness, cx);
    assert!(harness.content.upgrade().is_none());
    assert_eq!(finished_results(&harness), vec![MergeResult::Cancel]);
    assert_eq!(result_text(&harness, cx), ORIGINAL);
    assert!(!is_window_open(&harness, cx));
}
