use gpui::{
    Anchor, AnyElement, Context, DismissEvent, Entity, Focusable, Pixels, Point, Subscription,
    Window, anchored, deferred,
};
use merge_diff::Side;
use ui::{ContextMenu, ContextMenuEntry, IconPosition, PopoverMenu, Tooltip, prelude::*};
use util::ResultExt as _;

use super::ConflictsDialog;
use super::messages;
use super::persistence;
use super::state::TreeStateStrategy;

pub(super) struct RowContextMenu {
    menu: Entity<ContextMenu>,
    position: Point<Pixels>,
    _subscription: Subscription,
}

impl ConflictsDialog {
    pub(super) fn render_view_options(&self, cx: &mut Context<Self>) -> AnyElement {
        let dialog = cx.weak_entity();
        let group_by_directory = self.state.group_by_directory();
        PopoverMenu::new("conflicts-view-options")
            .trigger_with_tooltip(
                IconButton::new("conflicts-view-options-trigger", IconName::GeneralShow)
                    .icon_size(IconSize::Small),
                Tooltip::text(messages::VIEW_OPTIONS),
            )
            .menu(move |window, cx| {
                let dialog = dialog.clone();
                Some(ContextMenu::build(window, cx, move |menu, _, _| {
                    menu.item(
                        ContextMenuEntry::new(messages::GROUP_BY_DIRECTORY)
                            .icon(IconName::ToggleVisibility)
                            .icon_position(IconPosition::End)
                            .toggleable(IconPosition::Start, group_by_directory)
                            .handler(move |window, cx| {
                                dialog
                                    .update(cx, |dialog, cx| {
                                        dialog.toggle_group_by_directory(window, cx);
                                    })
                                    .log_err();
                            }),
                    )
                }))
            })
            .anchor(Anchor::TopRight)
            .into_any_element()
    }

    pub(super) fn toggle_group_by_directory(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let enabled = !self.state.group_by_directory();
        self.state.set_group_by_directory(enabled);
        persistence::write_group_by_directory(&self.grouping_key, enabled, cx);
        self.state.rebuild(TreeStateStrategy::GroupingChange);
        self.scroll_selection_into_view();
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    pub(super) fn deploy_context_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dialog = cx.weak_entity();
        let derived = self.state.derived();
        let accept_enabled = !derived.resolved_files_selected;
        let revert_enabled = derived.only_revertable_files_selected;
        let yours_label = self.texts.yours_column.clone();
        let theirs_label = self.texts.theirs_column.clone();
        let menu = ContextMenu::build(window, cx, move |menu, _, _| {
            let accept_yours_dialog = dialog.clone();
            let accept_theirs_dialog = dialog.clone();
            let revert_dialog = dialog.clone();
            menu.item(
                ContextMenuEntry::new(messages::accept_menu_label(&yours_label))
                    .disabled(!accept_enabled)
                    .handler(move |window, cx| {
                        accept_yours_dialog
                            .update(cx, |dialog, cx| {
                                dialog.accept_for_resolution(Side::Left, window, cx);
                            })
                            .log_err();
                    }),
            )
            .item(
                ContextMenuEntry::new(messages::accept_menu_label(&theirs_label))
                    .disabled(!accept_enabled)
                    .handler(move |window, cx| {
                        accept_theirs_dialog
                            .update(cx, |dialog, cx| {
                                dialog.accept_for_resolution(Side::Right, window, cx);
                            })
                            .log_err();
                    }),
            )
            .separator()
            .item(
                ContextMenuEntry::new(messages::REVERT_CONFLICT_RESOLUTION)
                    .icon(IconName::VcsRevert)
                    .disabled(!revert_enabled)
                    .handler(move |window, cx| {
                        revert_dialog
                            .update(cx, |dialog, cx| {
                                dialog.revert_selected_resolution(window, cx);
                            })
                            .log_err();
                    }),
            )
        });
        let menu_focus_handle = menu.focus_handle(cx);
        window.defer(cx, move |window, cx| {
            window.focus(&menu_focus_handle, cx);
        });
        let subscription =
            cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
                if this.context_menu.as_ref().is_some_and(|context_menu| {
                    context_menu
                        .menu
                        .focus_handle(cx)
                        .contains_focused(window, cx)
                }) {
                    window.focus(&this.focus_handle, cx);
                }
                this.context_menu = None;
                cx.notify();
            });
        self.context_menu = Some(RowContextMenu {
            menu,
            position,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(super) fn render_context_menu(&self) -> Option<AnyElement> {
        let context_menu = self.context_menu.as_ref()?;
        Some(
            deferred(
                anchored()
                    .position(context_menu.position)
                    .anchor(Anchor::TopLeft)
                    .child(context_menu.menu.clone()),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}
