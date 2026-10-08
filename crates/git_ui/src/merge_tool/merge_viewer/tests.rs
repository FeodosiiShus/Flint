use editor::{SelectionEffects, SoftWrap};
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext, px, size};
use language::{Point, language_settings::ShowWhitespaceSetting};
use merge_diff::{ComparisonPolicy, Side, ThreeSide};
use settings::SettingsStore;

use super::{
    Appearance, MergeViewer, RediffState, ViewerNotification,
    operations::PopupAction,
    selection::{CaretSnapshot, EditorSelection, SelectedLines},
    viewer_settings,
};
use crate::merge_tool::{
    conflict_resolution::rich_text::RichText,
    merge_model::{FileConflictType, MergeBuffers, MergeConflictModel, MergeTexts},
    merge_window::MergePaneTitle,
};

const BASE: &str = "a\nb\nc\n";
const LEFT_ONLY_CHANGE: &str = "a\nB\nc\n";

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

fn new_model(
    cx: &mut TestAppContext,
    left: &str,
    base: &str,
    right: &str,
) -> Entity<MergeConflictModel> {
    let texts = MergeTexts::new(left, base, right);
    cx.update(|cx| {
        let buffers = MergeBuffers::detached(&texts, None, cx);
        cx.new(|cx| MergeConflictModel::new(buffers, &texts, FileConflictType::Default, cx))
    })
}

fn settled_viewer<'a>(
    cx: &'a mut TestAppContext,
    left: &str,
    base: &str,
    right: &str,
) -> (Entity<MergeViewer>, &'a mut VisualTestContext) {
    init_test(cx);
    let model = new_model(cx, left, base, right);
    let (viewer, cx) = cx
        .add_window_view(|window, cx| MergeViewer::new(model.clone(), plain_titles(), window, cx));
    cx.run_until_parked();
    assert_eq!(
        viewer.read_with(cx, |viewer, _| viewer.rediff_state()),
        RediffState::Finished
    );
    (viewer, cx)
}

fn selection_of_lines(start_line: usize, end_line: usize, has_selection: bool) -> EditorSelection {
    let carets = vec![CaretSnapshot {
        start_line,
        end_line,
        has_selection,
        end_at_document_end: false,
    }];
    let lines = SelectedLines::from_carets(&carets, 4);
    EditorSelection { carets, lines }
}

#[test]
fn merge_tool_viewer_popup_actions_are_visible_only_in_their_editors() {
    let expectations = [
        (PopupAction::Accept(Side::Left), [true, true, false]),
        (PopupAction::Accept(Side::Right), [false, true, true]),
        (PopupAction::ResolveUsing(Side::Left), [true, true, false]),
        (PopupAction::ResolveUsing(Side::Right), [false, true, true]),
        (PopupAction::IgnoreSide(Side::Left), [true, false, false]),
        (PopupAction::IgnoreSide(Side::Right), [false, false, true]),
        (PopupAction::ResolveAutomatically, [false, true, false]),
        (PopupAction::IgnoreAll, [false, true, false]),
        (PopupAction::Revert, [true, true, true]),
    ];
    for (action, expected) in expectations {
        let visibility = ThreeSide::ALL.map(|editor_side| action.is_visible(editor_side));
        assert_eq!(visibility, expected, "{action:?}");
    }
}

#[test]
fn merge_tool_viewer_popup_texts_follow_the_editor_side() {
    let expectations = [
        (
            PopupAction::Accept(Side::Left),
            ThreeSide::Base,
            "Accept Left Side",
        ),
        (
            PopupAction::Accept(Side::Right),
            ThreeSide::Base,
            "Accept Right Side",
        ),
        (PopupAction::Accept(Side::Left), ThreeSide::Left, "Accept"),
        (PopupAction::Accept(Side::Right), ThreeSide::Right, "Accept"),
        (
            PopupAction::ResolveUsing(Side::Left),
            ThreeSide::Base,
            "Resolve using Left",
        ),
        (
            PopupAction::ResolveUsing(Side::Left),
            ThreeSide::Left,
            "Resolve using Left",
        ),
        (
            PopupAction::ResolveUsing(Side::Right),
            ThreeSide::Base,
            "Resolve using Right",
        ),
        (
            PopupAction::ResolveUsing(Side::Right),
            ThreeSide::Right,
            "Resolve using Right",
        ),
        (
            PopupAction::IgnoreSide(Side::Left),
            ThreeSide::Left,
            "Ignore",
        ),
        (
            PopupAction::IgnoreSide(Side::Right),
            ThreeSide::Right,
            "Ignore",
        ),
        (PopupAction::IgnoreAll, ThreeSide::Base, "Ignore"),
        (
            PopupAction::ResolveAutomatically,
            ThreeSide::Base,
            "Resolve Automatically",
        ),
        (PopupAction::Revert, ThreeSide::Left, "Revert"),
    ];
    for (action, editor_side, expected) in expectations {
        assert_eq!(
            action.text(editor_side),
            expected,
            "{action:?} in {editor_side:?}"
        );
    }
}

#[test]
fn merge_tool_viewer_command_names_append_in_merge_to_the_popup_text() {
    let expectations = [
        (
            PopupAction::Accept(Side::Left),
            ThreeSide::Left,
            "Accept in merge",
        ),
        (
            PopupAction::Accept(Side::Left),
            ThreeSide::Base,
            "Accept Left Side in merge",
        ),
        (
            PopupAction::Accept(Side::Right),
            ThreeSide::Base,
            "Accept Right Side in merge",
        ),
        (
            PopupAction::ResolveUsing(Side::Left),
            ThreeSide::Base,
            "Resolve using Left in merge",
        ),
        (PopupAction::IgnoreAll, ThreeSide::Base, "Ignore in merge"),
        (
            PopupAction::IgnoreSide(Side::Right),
            ThreeSide::Right,
            "Ignore in merge",
        ),
        (
            PopupAction::ResolveAutomatically,
            ThreeSide::Base,
            "Resolve Automatically in merge",
        ),
    ];
    for (action, editor_side, expected) in expectations {
        assert_eq!(
            action.command_name(editor_side),
            expected,
            "{action:?} in {editor_side:?}"
        );
    }
}

#[test]
fn merge_tool_viewer_notification_texts_match_the_diff_bundle() {
    assert_eq!(
        ViewerNotification::DiffTooBig.text(),
        "Unable to calculate diff. File is too big and there are too many changes."
    );
    assert_eq!(
        ViewerNotification::ReadOnlyFile.text(),
        "Cannot resolve conflicts in a read-only file"
    );
}

#[gpui::test]
async fn merge_tool_viewer_revert_entry_appears_only_for_a_selection_and_never_acts(
    cx: &mut TestAppContext,
) {
    let (viewer, cx) = settled_viewer(cx, LEFT_ONLY_CHANGE, BASE, BASE);
    viewer.update_in(cx, |viewer, window, cx| {
        viewer.apply_non_conflicting(ThreeSide::Base, window, cx);
    });
    cx.run_until_parked();

    let selection = selection_of_lines(1, 2, true);
    let caret_on_change = selection_of_lines(1, 1, false);
    let revealed = viewer.read_with(cx, |viewer, cx| {
        ThreeSide::ALL.map(|side| {
            (
                viewer.popup_action_enabled(PopupAction::Revert, side, &selection, cx),
                viewer.popup_action_enabled(PopupAction::Revert, side, &caret_on_change, cx),
                viewer.selected_changes(PopupAction::Revert, side, &selection, cx),
            )
        })
    });
    let nothing_selected: Vec<usize> = Vec::new();
    for (side_index, (with_selection, with_caret, selected)) in revealed.into_iter().enumerate() {
        assert!(with_selection, "side {side_index}");
        assert!(!with_caret, "side {side_index}");
        assert_eq!(selected, nothing_selected, "side {side_index}");
    }

    viewer.update_in(cx, |viewer, window, cx| {
        viewer.editor(ThreeSide::Base).update(cx, |editor, cx| {
            editor.change_selections(SelectionEffects::no_scroll(), window, cx, |selections| {
                selections.select_ranges([Point::new(0, 0)..Point::new(3, 0)]);
            });
        });
        viewer.perform_popup_action(PopupAction::Revert, ThreeSide::Base, cx);
    });
    cx.run_until_parked();

    viewer.read_with(cx, |viewer, cx| {
        assert_eq!(
            viewer.editor(ThreeSide::Base).read(cx).text(cx),
            LEFT_ONLY_CHANGE
        );
        assert!(viewer.can_revert_conflict_resolution(cx));
    });
}

#[gpui::test]
async fn merge_tool_viewer_focus_opposite_pane_cycles_left_base_right(cx: &mut TestAppContext) {
    let (viewer, cx) = settled_viewer(cx, LEFT_ONLY_CHANGE, BASE, BASE);
    let visited = viewer.update_in(cx, |viewer, window, cx| {
        assert_eq!(viewer.current_side, ThreeSide::Base);
        let mut visited = Vec::new();
        for _ in 0..3 {
            viewer.focus_opposite_pane(false, window, cx);
            visited.push(viewer.current_side);
        }
        visited
    });
    assert_eq!(
        visited,
        vec![ThreeSide::Right, ThreeSide::Left, ThreeSide::Base]
    );
}

#[gpui::test]
async fn merge_tool_viewer_balloon_survives_its_own_keystroke_and_hides_on_the_next(
    cx: &mut TestAppContext,
) {
    let (viewer, cx) = settled_viewer(cx, LEFT_ONLY_CHANGE, BASE, BASE);
    viewer.update_in(cx, |viewer, window, cx| {
        viewer.apply_non_conflicting(ThreeSide::Base, window, cx);
    });
    cx.run_until_parked();

    viewer.update_in(cx, |viewer, window, cx| {
        assert!(viewer.balloon_visible);
        viewer.on_keystroke(window, cx);
        assert!(viewer.balloon_visible);
    });

    cx.simulate_keystrokes("a");

    viewer.read_with(cx, |viewer, _| assert!(!viewer.balloon_visible));
}

#[gpui::test]
async fn merge_tool_viewer_balloon_hides_when_the_window_is_resized(cx: &mut TestAppContext) {
    let (viewer, cx) = settled_viewer(cx, LEFT_ONLY_CHANGE, BASE, BASE);
    let initial_viewport = viewer.update_in(cx, |viewer, window, cx| {
        viewer.apply_non_conflicting(ThreeSide::Base, window, cx);
        window.viewport_size()
    });
    cx.run_until_parked();
    viewer.read_with(cx, |viewer, _| assert!(viewer.balloon_visible));

    cx.simulate_resize(initial_viewport);
    viewer.read_with(cx, |viewer, _| assert!(viewer.balloon_visible));

    let narrower_viewport = size(initial_viewport.width - px(100.), initial_viewport.height);
    cx.simulate_resize(narrower_viewport);
    viewer.read_with(cx, |viewer, _| {
        assert!(!viewer.balloon_visible);
        assert_eq!(viewer.viewport_size, narrower_viewport);
    });
}

#[gpui::test]
async fn merge_tool_viewer_appearance_changes_reach_the_editors_and_are_remembered(
    cx: &mut TestAppContext,
) {
    let (viewer, cx) = settled_viewer(cx, LEFT_ONLY_CHANGE, BASE, BASE);
    let soft_wrap_off = |viewer: &MergeViewer, cx: &gpui::App| {
        ThreeSide::ALL.map(|side| {
            matches!(
                viewer.editor(side).read(cx).soft_wrap_mode(cx),
                SoftWrap::None
            )
        })
    };
    viewer.read_with(cx, |viewer, cx| {
        assert_eq!(soft_wrap_off(viewer, cx), [true; 3]);
        for side in ThreeSide::ALL {
            assert_eq!(
                viewer.editor(side).read(cx).show_whitespaces_setting(cx),
                ShowWhitespaceSetting::None
            );
        }
    });

    let changed = Appearance {
        soft_wrap: true,
        indent_guides: true,
        show_whitespaces: true,
        line_numbers: false,
    };
    viewer.update(cx, |viewer, cx| viewer.apply_appearance(changed, cx));
    cx.run_until_parked();

    viewer.read_with(cx, |viewer, cx| {
        assert_eq!(soft_wrap_off(viewer, cx), [false; 3]);
        for side in ThreeSide::ALL {
            assert_eq!(
                viewer.editor(side).read(cx).show_whitespaces_setting(cx),
                ShowWhitespaceSetting::All
            );
        }
        assert_eq!(viewer.appearance(), changed);
    });
    cx.update(|_, cx| assert_eq!(viewer_settings::load(cx).appearance, changed));
}

#[gpui::test]
async fn merge_tool_viewer_new_viewers_start_from_the_remembered_settings(cx: &mut TestAppContext) {
    init_test(cx);
    let remembered = Appearance {
        soft_wrap: false,
        indent_guides: false,
        show_whitespaces: true,
        line_numbers: false,
    };
    cx.update(|cx| {
        viewer_settings::store_appearance(remembered, cx);
        viewer_settings::store_highlight_by_word(false, cx);
        viewer_settings::store_expand_by_default(false, cx);
        viewer_settings::store_ignore_policy(ComparisonPolicy::TrimWhitespaces, cx);
    });
    cx.run_until_parked();

    let model = new_model(cx, LEFT_ONLY_CHANGE, BASE, BASE);
    let (viewer, cx) = cx
        .add_window_view(|window, cx| MergeViewer::new(model.clone(), plain_titles(), window, cx));
    cx.run_until_parked();

    viewer.read_with(cx, |viewer, _| {
        assert_eq!(viewer.appearance(), remembered);
        assert!(!viewer.highlight_by_word());
        assert!(viewer.collapse_unchanged_selected());
        assert_eq!(viewer.ignore_policy(), ComparisonPolicy::TrimWhitespaces);
    });
}

#[gpui::test]
async fn merge_tool_viewer_ignore_policy_change_restarts_the_merge_and_is_remembered(
    cx: &mut TestAppContext,
) {
    let (viewer, cx) = settled_viewer(cx, LEFT_ONLY_CHANGE, BASE, BASE);
    viewer.update_in(cx, |viewer, window, cx| {
        viewer.request_ignore_policy(ComparisonPolicy::IgnoreWhitespaces, window, cx);
    });
    cx.run_until_parked();

    viewer.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.ignore_policy(), ComparisonPolicy::IgnoreWhitespaces);
        assert_eq!(
            viewer.model.read(cx).ignore_policy(),
            ComparisonPolicy::IgnoreWhitespaces
        );
        assert_eq!(viewer.rediff_state(), RediffState::Finished);
    });
    cx.update(|_, cx| {
        assert_eq!(
            viewer_settings::stored_ignore_policy(cx),
            ComparisonPolicy::IgnoreWhitespaces
        );
    });
}
