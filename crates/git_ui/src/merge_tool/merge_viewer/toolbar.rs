use gpui::{Action as _, App, Context, Entity, WeakEntity, Window, px};
use merge_diff::{ComparisonPolicy, ThreeSide};
use ui::{CommonAnimationExt, ContextMenu, Divider, PopoverMenu, Tooltip, prelude::*};
use util::ResultExt as _;

use super::{
    Appearance, MergeViewer,
    actions::{
        ApplyNonConflictingAll, ApplyNonConflictingLeft, ApplyNonConflictingRight, NextDifference,
        PreviousDifference, ResolveSimpleConflicts, RevertConflictResolution,
        ToggleCollapseUnchangedFragments, ToggleSynchronizeScrolling,
    },
};
use crate::merge_tool::{
    merge_model::messages::MergeStatus,
    palette::{current_palette, hex_color},
};

const ISLAND_HEIGHT: f32 = 40.0;
const ISLAND_RADIUS: f32 = 6.0;
const TOOLBAR_GAP: f32 = 10.0;
const NON_CONFLICTING_TITLE: &str = "Apply non-conflicting changes:";
const SPINNER_TOOLTIP: &str = "Computing differences";
const GREEN_CHECKMARK_LIGHT: u32 = 0x369650;

const IGNORE_POLICIES: [(ComparisonPolicy, &str); 3] = [
    (ComparisonPolicy::Default, "None"),
    (ComparisonPolicy::TrimWhitespaces, "Trim whitespaces"),
    (ComparisonPolicy::IgnoreWhitespaces, "Ignore whitespaces"),
];

impl MergeViewer {
    pub(super) fn render_header(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let palette = current_palette(cx);
        let island = h_flex()
            .w_full()
            .h(px(ISLAND_HEIGHT))
            .px(px(7.0))
            .gap(px(TOOLBAR_GAP))
            .items_center()
            .rounded(px(ISLAND_RADIUS))
            .border_1()
            .border_color(hex_color(palette.header_island_border))
            .bg(hex_color(palette.header_island_fill))
            .child(self.render_toolbar(cx))
            .child(self.render_status(cx))
            .child(self.render_settings_button(cx));
        div()
            .w_full()
            .flex_none()
            .pt(px(2.0))
            .px(px(6.0))
            .child(island)
    }

    fn render_toolbar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let can_previous = self.can_navigate(false, false, cx);
        let can_next = self.can_navigate(false, true, cx);
        let has_left = self
            .model
            .read(cx)
            .has_non_conflicted_changes(ThreeSide::Left);
        let has_all = self
            .model
            .read(cx)
            .has_non_conflicted_changes(ThreeSide::Base);
        let has_right = self
            .model
            .read(cx)
            .has_non_conflicted_changes(ThreeSide::Right);
        let can_resolve = self.model.read(cx).has_auto_resolvable_conflicted_changes();
        let can_revert = self.can_revert_conflict_resolution(cx);
        let collapse_selected = self.collapse_unchanged_selected();
        h_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .items_center()
            .child(
                IconButton::new("merge-previous-difference", IconName::GeneralUp)
                    .icon_size(IconSize::Small)
                    .disabled(!can_previous)
                    .tooltip(Tooltip::for_action_title(
                        "Previous Difference",
                        &PreviousDifference,
                    ))
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.navigate(false, false, window, cx);
                    })),
            )
            .child(
                IconButton::new("merge-next-difference", IconName::GeneralDown)
                    .icon_size(IconSize::Small)
                    .disabled(!can_next)
                    .tooltip(Tooltip::for_action_title(
                        "Next Difference",
                        &NextDifference,
                    ))
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.navigate(false, true, window, cx);
                    })),
            )
            .child(Divider::vertical())
            .child(
                IconButton::new("merge-collapse-unchanged", IconName::CollapseAll)
                    .icon_size(IconSize::Small)
                    .toggle_state(collapse_selected)
                    .tooltip(Tooltip::for_action_title(
                        "Collapse Unchanged Fragments",
                        &ToggleCollapseUnchangedFragments,
                    ))
                    .on_click(cx.listener(move |viewer, _, window, cx| {
                        viewer.set_collapse_unchanged(!collapse_selected, window, cx);
                    })),
            )
            .child(Divider::vertical())
            .child(
                Label::new(NON_CONFLICTING_TITLE)
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(
                Button::new("merge-apply-non-conflicting-left", "Left")
                    .label_size(LabelSize::Small)
                    .start_icon(
                        Icon::new(IconName::DiffApplyNotConflictsLeft).size(IconSize::Small),
                    )
                    .disabled(!has_left)
                    .tooltip(Tooltip::for_action_title(
                        "Apply Non-Conflicting Changes from the Left Side",
                        &ApplyNonConflictingLeft,
                    ))
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.apply_non_conflicting(ThreeSide::Left, window, cx);
                    })),
            )
            .child(
                Button::new("merge-apply-non-conflicting-all", "All")
                    .label_size(LabelSize::Small)
                    .start_icon(Icon::new(IconName::DiffApplyNotConflicts).size(IconSize::Small))
                    .disabled(!has_all)
                    .tooltip(Tooltip::for_action_title(
                        "Apply All Non-Conflicting Changes",
                        &ApplyNonConflictingAll,
                    ))
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.apply_non_conflicting(ThreeSide::Base, window, cx);
                    })),
            )
            .child(
                Button::new("merge-apply-non-conflicting-right", "Right")
                    .label_size(LabelSize::Small)
                    .start_icon(
                        Icon::new(IconName::DiffApplyNotConflictsRight).size(IconSize::Small),
                    )
                    .disabled(!has_right)
                    .tooltip(Tooltip::for_action_title(
                        "Apply Non-Conflicting Changes from the Right Side",
                        &ApplyNonConflictingRight,
                    ))
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.apply_non_conflicting(ThreeSide::Right, window, cx);
                    })),
            )
            .child(Divider::vertical())
            .child(
                IconButton::new(
                    "merge-resolve-simple-conflicts",
                    IconName::DiffMagicResolveToolbar,
                )
                .icon_size(IconSize::Small)
                .disabled(!can_resolve)
                .tooltip(Tooltip::for_action_title(
                    "Resolve Simple Conflicts",
                    &ResolveSimpleConflicts,
                ))
                .on_click(cx.listener(|viewer, _, window, cx| {
                    viewer.resolve_simple_conflicts(window, cx);
                })),
            )
            .child(
                IconButton::new("merge-revert-conflict-resolution", IconName::DiffRevert)
                    .icon_size(IconSize::Small)
                    .disabled(!can_revert)
                    .tooltip(Tooltip::for_action_title(
                        "Revert Conflict Resolution",
                        &RevertConflictResolution,
                    ))
                    .on_click(cx.listener(|viewer, _, _, cx| {
                        viewer.request_revert_conflict_resolution(cx);
                    })),
            )
    }

    fn render_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = current_palette(cx);
        let status = self.model.read(cx).status();
        let resolved = status == MergeStatus::AllConflictsResolved;
        let text = status.text().map(|text| text.to_string());
        h_flex()
            .flex_none()
            .gap_1()
            .px(px(2.0))
            .items_center()
            .when(self.is_busy(), |status_row| {
                status_row.child(
                    div()
                        .id("merge-status-spinner")
                        .tooltip(Tooltip::text(SPINNER_TOOLTIP))
                        .child(
                            Icon::new(IconName::LoadCircle)
                                .size(IconSize::Small)
                                .color(Color::Muted)
                                .with_rotate_animation(2),
                        ),
                )
            })
            .when(resolved, |status_row| {
                let checkmark = if palette.kind.is_dark() {
                    palette.status_no_conflicts
                } else {
                    GREEN_CHECKMARK_LIGHT
                };
                status_row.child(
                    Icon::new(IconName::GreenCheckmark)
                        .size(IconSize::Small)
                        .color(Color::Custom(hex_color(checkmark))),
                )
            })
            .when_some(text, |status_row, text| {
                let label = Label::new(text);
                status_row.child(if resolved {
                    label.color(Color::Custom(hex_color(palette.status_no_conflicts)))
                } else {
                    label
                })
            })
    }

    fn render_settings_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let viewer = cx.entity();
        PopoverMenu::new("merge-viewer-settings")
            .trigger_with_tooltip(
                IconButton::new("merge-viewer-settings-trigger", IconName::GeneralSettings)
                    .icon_size(IconSize::Small),
                Tooltip::text("Settings"),
            )
            .anchor(gpui::Anchor::TopRight)
            .with_handle(self.settings_menu.clone())
            .menu(move |window, cx| Some(build_settings_menu(viewer.clone(), window, cx)))
    }
}

fn build_settings_menu(
    viewer: Entity<MergeViewer>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ContextMenu> {
    let (sync_scroll, ignore_policy, by_word, appearance) = {
        let viewer = viewer.read(cx);
        (
            viewer.sync_scroll_enabled(),
            viewer.ignore_policy(),
            viewer.highlight_by_word(),
            viewer.appearance(),
        )
    };
    let weak_viewer = viewer.downgrade();
    ContextMenu::build(window, cx, move |menu, _, _| {
        let sync_viewer = weak_viewer.clone();
        let mut menu = menu.opaque_background().toggleable_entry(
            "Synchronize Scrolling",
            sync_scroll,
            IconPosition::Start,
            Some(ToggleSynchronizeScrolling.boxed_clone()),
            move |_, cx| {
                sync_viewer
                    .update(cx, |viewer, cx| viewer.toggle_synchronize_scrolling(cx))
                    .log_err();
            },
        );
        menu = menu.separator().header("Ignore Differences");
        for (policy, label) in IGNORE_POLICIES {
            let policy_viewer = weak_viewer.clone();
            menu = menu.toggleable_entry(
                label,
                policy == ignore_policy,
                IconPosition::Start,
                None,
                move |window, cx| {
                    policy_viewer
                        .update(cx, |viewer, cx| {
                            viewer.request_ignore_policy(policy, window, cx);
                        })
                        .log_err();
                },
            );
        }
        menu = menu.separator().header("Highlighting Differences");
        for (highlight_by_word, label) in [(false, "Lines"), (true, "Words")] {
            let highlight_viewer = weak_viewer.clone();
            menu = menu.toggleable_entry(
                label,
                highlight_by_word == by_word,
                IconPosition::Start,
                None,
                move |_, cx| {
                    highlight_viewer
                        .update(cx, |viewer, cx| {
                            viewer.set_highlight_by_word(highlight_by_word, cx);
                        })
                        .log_err();
                },
            );
        }
        let appearance_viewer = weak_viewer;
        menu.separator()
            .submenu("Appearance", move |submenu, _, _| {
                let submenu = submenu.opaque_background();
                let submenu = appearance_entry(
                    submenu,
                    "Show Whitespaces",
                    appearance.show_whitespaces,
                    &appearance_viewer,
                    |current| Appearance {
                        show_whitespaces: !current.show_whitespaces,
                        ..current
                    },
                    appearance,
                );
                let submenu = appearance_entry(
                    submenu,
                    "Show Line Numbers",
                    appearance.line_numbers,
                    &appearance_viewer,
                    |current| Appearance {
                        line_numbers: !current.line_numbers,
                        ..current
                    },
                    appearance,
                );
                let submenu = appearance_entry(
                    submenu,
                    "Show Indent Guides",
                    appearance.indent_guides,
                    &appearance_viewer,
                    |current| Appearance {
                        indent_guides: !current.indent_guides,
                        ..current
                    },
                    appearance,
                );
                appearance_entry(
                    submenu,
                    "Soft-Wrap",
                    appearance.soft_wrap,
                    &appearance_viewer,
                    |current| Appearance {
                        soft_wrap: !current.soft_wrap,
                        ..current
                    },
                    appearance,
                )
            })
    })
}

fn appearance_entry(
    menu: ContextMenu,
    label: &'static str,
    selected: bool,
    viewer: &WeakEntity<MergeViewer>,
    toggled: fn(Appearance) -> Appearance,
    appearance: Appearance,
) -> ContextMenu {
    let viewer = viewer.clone();
    menu.toggleable_entry(label, selected, IconPosition::Start, None, move |_, cx| {
        viewer
            .update(cx, |viewer, cx| {
                viewer.apply_appearance(toggled(appearance), cx);
            })
            .log_err();
    })
}
