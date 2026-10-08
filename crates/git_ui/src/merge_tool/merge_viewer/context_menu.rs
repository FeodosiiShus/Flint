use editor::{Editor, SelectionEffects};
use gpui::{Action as _, App, Context, Entity, FocusHandle, Focusable as _, WeakEntity, Window};
use language::Point;
use merge_diff::{Side, ThreeSide};
use ui::{ContextMenu, prelude::*};
use util::ResultExt as _;

use super::{
    MergeViewer,
    actions::{
        AcceptLeftSide, AcceptRightSide, IgnoreLeftSide, IgnoreRightSide, IgnoreSelectedChanges,
        ResolveSimpleConflict, ResolveUsingLeft, ResolveUsingRight,
        ToggleCollapseUnchangedFragments, ToggleSynchronizeScrolling,
    },
    operations::PopupAction,
    selection::{EditorSelection, selection_of_editor},
};

const POPUP_ACTIONS: [PopupAction; 9] = [
    PopupAction::Accept(Side::Left),
    PopupAction::Accept(Side::Right),
    PopupAction::ResolveUsing(Side::Left),
    PopupAction::ResolveUsing(Side::Right),
    PopupAction::IgnoreSide(Side::Left),
    PopupAction::IgnoreSide(Side::Right),
    PopupAction::ResolveAutomatically,
    PopupAction::IgnoreAll,
    PopupAction::Revert,
];

fn action_for(popup: PopupAction) -> Option<Box<dyn gpui::Action>> {
    match popup {
        PopupAction::Accept(Side::Left) => Some(AcceptLeftSide.boxed_clone()),
        PopupAction::Accept(Side::Right) => Some(AcceptRightSide.boxed_clone()),
        PopupAction::ResolveUsing(Side::Left) => Some(ResolveUsingLeft.boxed_clone()),
        PopupAction::ResolveUsing(Side::Right) => Some(ResolveUsingRight.boxed_clone()),
        PopupAction::IgnoreSide(Side::Left) => Some(IgnoreLeftSide.boxed_clone()),
        PopupAction::IgnoreSide(Side::Right) => Some(IgnoreRightSide.boxed_clone()),
        PopupAction::ResolveAutomatically => Some(ResolveSimpleConflict.boxed_clone()),
        PopupAction::IgnoreAll => Some(IgnoreSelectedChanges.boxed_clone()),
        PopupAction::Revert => None,
    }
}

pub(super) fn install_context_menus(viewer: &MergeViewer, cx: &mut Context<MergeViewer>) {
    let weak_viewer = cx.weak_entity();
    for side in ThreeSide::ALL {
        let weak_viewer = weak_viewer.clone();
        viewer.editor(side).update(cx, |editor, _| {
            editor.set_custom_context_menu(move |editor, display_point, window, cx| {
                let viewer = weak_viewer.upgrade()?;
                let clicked = editor
                    .display_snapshot(cx)
                    .display_point_to_point(display_point, editor::Bias::Left);
                place_caret_for_context_menu(editor, clicked, window, cx);
                let focus_handle = editor.focus_handle(cx);
                let selection = selection_of_editor(editor, cx);
                Some(build_pane_menu(
                    &viewer,
                    side,
                    &selection,
                    focus_handle,
                    window,
                    cx,
                ))
            });
        });
    }
}

fn place_caret_for_context_menu(
    editor: &mut Editor,
    clicked: Point,
    window: &mut Window,
    cx: &mut Context<Editor>,
) {
    let snapshot = editor.display_snapshot(cx);
    let clicked_inside_selection =
        editor
            .selections
            .all::<Point>(&snapshot)
            .iter()
            .any(|selection| {
                selection.start != selection.end
                    && selection.start <= clicked
                    && clicked <= selection.end
            });
    if clicked_inside_selection {
        return;
    }
    editor.change_selections(SelectionEffects::no_scroll(), window, cx, |selections| {
        selections.select_ranges([clicked..clicked]);
    });
}

fn build_pane_menu(
    viewer: &Entity<MergeViewer>,
    side: ThreeSide,
    selection: &EditorSelection,
    focus_handle: FocusHandle,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ContextMenu> {
    let (entries, collapse_selected, sync_selected) = {
        let viewer = viewer.read(cx);
        let entries: Vec<PopupAction> = POPUP_ACTIONS
            .iter()
            .copied()
            .filter(|action| viewer.popup_action_enabled(*action, side, selection, cx))
            .collect();
        (
            entries,
            viewer.collapse_unchanged_selected(),
            viewer.sync_scroll_enabled(),
        )
    };
    let weak_viewer = viewer.downgrade();
    let has_apply_entries = !entries.is_empty();
    ContextMenu::build(window, cx, move |menu, _, _| {
        let mut menu = menu.context(focus_handle).opaque_background();
        for action in entries {
            let text = action.text(side);
            let weak_viewer = weak_viewer.clone();
            menu = menu.entry(text, action_for(action), move |_, cx| {
                run_popup_action(&weak_viewer, action, side, cx);
            });
        }
        if has_apply_entries {
            menu = menu.separator();
        }
        let collapse_viewer = weak_viewer.clone();
        let sync_viewer = weak_viewer;
        menu.toggleable_entry(
            "Collapse Unchanged Fragments",
            collapse_selected,
            IconPosition::Start,
            Some(ToggleCollapseUnchangedFragments.boxed_clone()),
            move |window, cx| {
                collapse_viewer
                    .update(cx, |viewer, cx| {
                        viewer.set_collapse_unchanged(!collapse_selected, window, cx);
                    })
                    .log_err();
            },
        )
        .toggleable_entry(
            "Synchronize Scrolling",
            sync_selected,
            IconPosition::Start,
            Some(ToggleSynchronizeScrolling.boxed_clone()),
            move |_, cx| {
                sync_viewer
                    .update(cx, |viewer, cx| viewer.toggle_synchronize_scrolling(cx))
                    .log_err();
            },
        )
    })
}

fn run_popup_action(
    viewer: &WeakEntity<MergeViewer>,
    action: PopupAction,
    side: ThreeSide,
    cx: &mut App,
) {
    viewer
        .update(cx, |viewer, cx| {
            viewer.perform_popup_action(action, side, cx)
        })
        .log_err();
}
