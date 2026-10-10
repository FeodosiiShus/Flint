use crate::{
    Copy, CopyAndTrim, CopyPermalinkToLine, Cut, DiffClipboardWithSelection, DisplayPoint,
    DisplaySnapshot, Editor, FindAllReferences, FoldAll, FoldRecursive, GoToDefinition,
    GoToImplementation, GoToTypeDefinition, OpenPermalinkToLine, Paste, Rename,
    RevealInFileManager, SelectMode, SelectionEffects, SelectionExt, ToDisplayPoint,
    ToggleCodeActions, UnfoldAll, UnfoldLines, UnfoldRecursive, actions::Fold,
    selections_collection::SelectionsCollection,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    Context, DismissEvent, Entity, FocusHandle, Focusable as _, Pixels, Point, Subscription, Window,
};
use std::ops::Range;
use workspace::{DeploySearch, OpenInTerminal};

#[derive(Debug)]
pub enum MenuPosition {
    /// When the editor is scrolled, the context menu stays on the exact
    /// same position on the screen, never disappearing.
    PinnedToScreen(Point<Pixels>),
    /// When the editor is scrolled, the context menu follows the position it is associated with.
    /// Disappears when the position is no longer visible.
    PinnedToEditor {
        source: multi_buffer::Anchor,
        offset: Point<Pixels>,
    },
}

pub struct MouseContextMenu {
    pub(crate) position: MenuPosition,
    pub(crate) context_menu: Entity<ui::ContextMenu>,
    _dismiss_subscription: Subscription,
    _cursor_move_subscription: Subscription,
}

impl std::fmt::Debug for MouseContextMenu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MouseContextMenu")
            .field("position", &self.position)
            .field("context_menu", &self.context_menu)
            .finish()
    }
}

impl MouseContextMenu {
    pub(crate) fn pinned_to_editor(
        editor: &mut Editor,
        source: multi_buffer::Anchor,
        position: Point<Pixels>,
        context_menu: Entity<ui::ContextMenu>,
        window: &mut Window,
        cx: &mut Context<Editor>,
    ) -> Option<Self> {
        let editor_snapshot = editor.snapshot(window, cx);
        let content_origin = editor.last_bounds?.origin
            + Point {
                x: editor.gutter_dimensions.width,
                y: Pixels::ZERO,
            };
        let source_position = editor.to_pixel_point(source, &editor_snapshot, window, cx)?;
        let menu_position = MenuPosition::PinnedToEditor {
            source,
            offset: position - (source_position + content_origin),
        };
        Some(MouseContextMenu::new(
            editor,
            menu_position,
            context_menu,
            window,
            cx,
        ))
    }

    pub(crate) fn new(
        editor: &Editor,
        position: MenuPosition,
        context_menu: Entity<ui::ContextMenu>,
        window: &mut Window,
        cx: &mut Context<Editor>,
    ) -> Self {
        let context_menu_focus = context_menu.focus_handle(cx);

        // Since `ContextMenu` is rendered in a deferred fashion its focus
        // handle is not linked to the Editor's until after the deferred draw
        // callback runs.
        // We need to wait for that to happen before focusing it, so that
        // calling `contains_focused` on the editor's focus handle returns
        // `true` when the `ContextMenu` is focused.
        let focus_handle = context_menu_focus.clone();
        cx.on_next_frame(window, move |_, window, cx| {
            cx.on_next_frame(window, move |_, window, cx| {
                window.focus(&focus_handle, cx);
            });
        });

        let _dismiss_subscription = cx.subscribe_in(&context_menu, window, {
            let context_menu_focus = context_menu_focus.clone();
            move |editor, _, _event: &DismissEvent, window, cx| {
                editor.mouse_context_menu.take();
                if context_menu_focus.contains_focused(window, cx) {
                    window.focus(&editor.focus_handle(cx), cx);
                }
            }
        });

        let selection_init = *editor.selections.newest_anchor();

        let _cursor_move_subscription = cx.subscribe_in(
            &cx.entity(),
            window,
            move |editor, _, event: &crate::EditorEvent, window, cx| {
                let crate::EditorEvent::SelectionsChanged { local: true } = event else {
                    return;
                };
                let display_snapshot = &editor
                    .display_map
                    .update(cx, |display_map, cx| display_map.snapshot(cx));
                let selection_init_range = selection_init.display_range(display_snapshot);
                let selection_now_range = editor
                    .selections
                    .newest_anchor()
                    .display_range(display_snapshot);
                if selection_now_range == selection_init_range {
                    return;
                }
                editor.mouse_context_menu.take();
                if context_menu_focus.contains_focused(window, cx) {
                    window.focus(&editor.focus_handle(cx), cx);
                }
            },
        );

        Self {
            position,
            context_menu,
            _dismiss_subscription,
            _cursor_move_subscription,
        }
    }
}

fn submenu_with_context(
    focus: Option<FocusHandle>,
    build: impl Fn(ui::ContextMenu) -> ui::ContextMenu + 'static,
) -> impl Fn(ui::ContextMenu, &mut Window, &mut Context<ui::ContextMenu>) -> ui::ContextMenu + 'static
{
    move |menu: ui::ContextMenu, _window: &mut Window, _cx: &mut Context<ui::ContextMenu>| {
        build(menu.when_some(focus.clone(), |menu, focus| menu.context(focus)))
    }
}

fn display_ranges<'a>(
    display_map: &'a DisplaySnapshot,
    selections: &'a SelectionsCollection,
) -> impl Iterator<Item = Range<DisplayPoint>> + 'a {
    let pending = selections.pending_anchor();
    selections
        .disjoint_anchors()
        .iter()
        .chain(pending)
        .map(move |s| s.start.to_display_point(display_map)..s.end.to_display_point(display_map))
}

pub fn deploy_context_menu(
    editor: &mut Editor,
    position: Option<Point<Pixels>>,
    point: DisplayPoint,
    window: &mut Window,
    cx: &mut Context<Editor>,
) {
    if !editor.is_focused(window) {
        window.focus(&editor.focus_handle(cx), cx);
    }

    let display_map = editor.display_snapshot(cx);
    let source_anchor = display_map.display_point_to_anchor(point, text::Bias::Right);
    let context_menu = if let Some(custom) = editor.custom_context_menu.take() {
        let menu = custom(editor, point, window, cx);
        editor.custom_context_menu = Some(custom);
        let Some(menu) = menu else {
            return;
        };
        menu
    } else {
        // Don't show context menu for inline editors (only applies to default menu)
        if !editor.mode().is_full() {
            return;
        }

        // Don't show the context menu if there isn't a project associated with this editor
        let Some(project) = editor.project.clone() else {
            return;
        };

        let snapshot = editor.snapshot(window, cx);
        let display_map = editor.display_snapshot(cx);
        let buffer = snapshot.buffer_snapshot();
        let anchor = buffer.anchor_before(point.to_point(&display_map));
        if !display_ranges(&display_map, &editor.selections).any(|r| r.contains(&point)) {
            // Move the cursor to the clicked location so that dispatched actions make sense
            editor.change_selections(SelectionEffects::no_scroll(), window, cx, |s| {
                s.clear_disjoint();
                s.set_pending_anchor_range(anchor..anchor, SelectMode::Character);
            });
        }

        let focus = window.focused(cx);
        let has_reveal_target = editor.target_file(cx).is_some();
        let has_git_repo =
            buffer
                .anchor_to_buffer_anchor(anchor)
                .is_some_and(|(buffer_anchor, _)| {
                    project
                        .read(cx)
                        .git_store()
                        .read(cx)
                        .repository_and_path_for_buffer_id(buffer_anchor.buffer_id, cx)
                        .is_some()
                });

        ui::ContextMenu::build(window, cx, |menu, _window, _cx| {
            let builder = menu
                .on_blur_subscription(Subscription::new(|| {}))
                .action(
                    "Show Context Actions",
                    Box::new(ToggleCodeActions {
                        deployed_from: None,
                    }),
                )
                .separator()
                .action("Cut", Box::new(Cut))
                .action("Copy", Box::new(Copy))
                .action("Paste", Box::new(Paste))
                .submenu(
                    "Copy / Paste Special",
                    submenu_with_context(focus.clone(), move |menu| {
                        menu.action("Copy and Trim", Box::new(CopyAndTrim)).when(
                            has_git_repo,
                            |menu| {
                                menu.action("Copy Permalink to Line", Box::new(CopyPermalinkToLine))
                            },
                        )
                    }),
                )
                .separator()
                .action("Find in Files", Box::new(DeploySearch::default()))
                .action("Find Usages", Box::new(FindAllReferences::default()))
                .submenu(
                    "Go To",
                    submenu_with_context(focus.clone(), |menu| {
                        menu.action("Declaration or Usages", Box::new(GoToDefinition::default()))
                            .action("Implementation(s)", Box::new(GoToImplementation::default()))
                            .action("Type Declaration", Box::new(GoToTypeDefinition::default()))
                    }),
                )
                .separator()
                .submenu(
                    "Folding",
                    submenu_with_context(focus.clone(), |menu| {
                        menu.action("Expand", Box::new(UnfoldLines))
                            .action("Expand Recursively", Box::new(UnfoldRecursive))
                            .action("Expand All", Box::new(UnfoldAll))
                            .separator()
                            .action("Collapse", Box::new(Fold))
                            .action("Collapse Recursively", Box::new(FoldRecursive))
                            .action("Collapse All", Box::new(FoldAll))
                    }),
                )
                .separator()
                .action("Rename…", Box::new(Rename))
                .separator()
                .when(has_reveal_target, |builder| {
                    builder
                        .submenu(
                            "Open In",
                            submenu_with_context(focus.clone(), |menu| {
                                menu.action("Finder", Box::new(RevealInFileManager))
                                    .action("Terminal", Box::new(OpenInTerminal))
                            }),
                        )
                        .separator()
                })
                .when(has_git_repo, |builder| {
                    builder
                        .submenu(
                            "Git",
                            submenu_with_context(focus.clone(), |menu| {
                                menu.action("Annotate with Git Blame", Box::new(git::Blame))
                                    .action("Show History", Box::new(git::FileHistory))
                                    .separator()
                                    .action("Open Permalink to Line", Box::new(OpenPermalinkToLine))
                                    .action("Copy Permalink to Line", Box::new(CopyPermalinkToLine))
                            }),
                        )
                        .separator()
                })
                .action(
                    "Compare with Clipboard",
                    Box::new(DiffClipboardWithSelection),
                );
            match focus {
                Some(focus) => builder.context(focus),
                None => builder,
            }
        })
    };

    editor.mouse_context_menu = match position {
        Some(position) => MouseContextMenu::pinned_to_editor(
            editor,
            source_anchor,
            position,
            context_menu,
            window,
            cx,
        ),
        None => {
            let character_size = editor.character_dimensions(window, cx);
            let menu_position = MenuPosition::PinnedToEditor {
                source: source_anchor,
                offset: gpui::point(character_size.em_width, character_size.line_height),
            };
            Some(MouseContextMenu::new(
                editor,
                menu_position,
                context_menu,
                window,
                cx,
            ))
        }
    };
    cx.notify();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        MultiBuffer,
        display_map::DisplayRow,
        editor_tests::init_test,
        test::{
            build_editor_with_project, editor_lsp_test_context::EditorLspTestContext,
            editor_test_context::EditorTestContext,
        },
    };
    use gpui::px;
    use indoc::indoc;
    use project::{FakeFs, Project};

    #[gpui::test]
    async fn test_mouse_context_menu(cx: &mut gpui::TestAppContext) {
        init_test(cx, |_| {});

        let mut cx = EditorLspTestContext::new_rust(
            lsp::ServerCapabilities {
                hover_provider: Some(lsp::HoverProviderCapability::Simple(true)),
                ..Default::default()
            },
            cx,
        )
        .await;

        cx.set_state(indoc! {"
            fn teˇst() {
                do_work();
            }
        "});
        let point = cx.display_point(indoc! {"
            fn test() {
                do_wˇork();
            }
        "});
        cx.editor(|editor, _window, _app| assert!(editor.mouse_context_menu.is_none()));

        cx.update_editor(|editor, window, cx| {
            deploy_context_menu(editor, Some(Default::default()), point, window, cx);

            // Assert that, even after deploying the editor's mouse context
            // menu, the editor's focus handle still contains the focused
            // element. The pane's tab bar relies on this to determine whether
            // to show the tab bar buttons and there was a small flicker when
            // deploying the mouse context menu that would cause this to not be
            // true, making it so that the buttons would disappear for a couple
            // of frames.
            assert!(editor.focus_handle.contains_focused(window, cx));
        });

        cx.assert_editor_state(indoc! {"
            fn test() {
                do_wˇork();
            }
        "});
        cx.editor(|editor, _window, _app| assert!(editor.mouse_context_menu.is_some()));
    }

    #[gpui::test]
    async fn test_mouse_context_menu_at_pixel_snapped_scroll_position(
        cx: &mut gpui::TestAppContext,
    ) {
        init_test(cx, |_| {});

        let mut cx = EditorTestContext::new(cx).await;
        cx.set_state(&format!("ˇ{}", "aaaaa\n".repeat(100)));
        cx.update(|window, _| window.set_scale_factor(1.25));
        cx.update_editor(|editor, _, cx| {
            editor.set_text_style_refinement(gpui::TextStyleRefinement {
                font_size: Some(gpui::px(14.).into()),
                line_height: Some(gpui::relative(1.3)),
                ..Default::default()
            });
            cx.notify();
        });
        cx.run_until_parked();

        cx.update_editor(|editor, window, cx| {
            assert_eq!(window.scale_factor(), 1.25);
            let line_height = editor
                .style(cx)
                .text
                .line_height_in_pixels(window.rem_size());
            assert_eq!(line_height, gpui::px(18.));
            assert!(window.pixel_snap_f64(f64::from(line_height)) / f64::from(line_height) < 1.);
            editor.set_scroll_position(gpui::point(0., 1.), window, cx);
            assert_eq!(editor.snapshot(window, cx).scroll_position().y, 1.);

            deploy_context_menu(
                editor,
                Some(gpui::point(gpui::px(200.), gpui::px(200.))),
                DisplayPoint::new(crate::display_map::DisplayRow(5), 0),
                window,
                cx,
            );
            assert!(editor.mouse_context_menu.is_some());
        });
        cx.run_until_parked();

        assert!(cx.debug_bounds("MENU_ITEM-Copy").is_some());
    }

    #[gpui::test]
    async fn test_mouse_context_menu_follows_intellij_editor_popup_order(
        cx: &mut gpui::TestAppContext,
    ) {
        init_test(cx, |_| {});

        let mut cx = EditorLspTestContext::new_rust(lsp::ServerCapabilities::default(), cx).await;
        cx.set_state(indoc! {"
            fn teˇst() {}
        "});
        let point = cx.display_point(indoc! {"
            fn teˇst() {}
        "});

        cx.update_editor(|editor, window, cx| {
            deploy_context_menu(editor, None, point, window, cx);
            assert!(editor.mouse_context_menu.is_some());
        });
        cx.run_until_parked();

        let top_level_items = [
            "MENU_ITEM-Show Context Actions",
            "MENU_ITEM-Cut",
            "MENU_ITEM-Copy",
            "MENU_ITEM-Paste",
            "MENU_ITEM-Copy / Paste Special",
            "MENU_ITEM-Find in Files",
            "MENU_ITEM-Find Usages",
            "MENU_ITEM-Go To",
            "MENU_ITEM-Folding",
            "MENU_ITEM-Rename…",
            "MENU_ITEM-Open In",
            "MENU_ITEM-Git",
            "MENU_ITEM-Compare with Clipboard",
        ];
        let vertical_positions = top_level_items
            .into_iter()
            .map(|selector| {
                cx.debug_bounds(selector)
                    .unwrap_or_else(|| panic!("{selector} should be rendered in the editor menu"))
                    .origin
                    .y
            })
            .collect::<Vec<_>>();
        for (index, pair) in vertical_positions.windows(2).enumerate() {
            assert!(
                pair[0] < pair[1],
                "{} should be rendered above {}",
                top_level_items[index],
                top_level_items[index + 1]
            );
        }

        for removed_item in [
            "MENU_ITEM-Go to Declaration",
            "MENU_ITEM-Format Buffer",
            "MENU_ITEM-Run to Cursor",
            "MENU_ITEM-Copy and Trim",
            "MENU_ITEM-View File History",
        ] {
            assert_eq!(
                cx.debug_bounds(removed_item),
                None,
                "{removed_item} is not a top-level item of the IntelliJ editor menu"
            );
        }
    }

    #[gpui::test]
    async fn test_mouse_context_menu_hides_open_in_and_git_for_unsaved_buffer(
        cx: &mut gpui::TestAppContext,
    ) {
        init_test(cx, |_| {});

        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let buffer = project.update(cx, |project, cx| {
            project.create_local_buffer("let value = 1;\n", None, false, cx)
        });
        let window = cx.add_window(|window, cx| {
            let editor = build_editor_with_project(
                project,
                MultiBuffer::build_from_buffer(buffer, cx),
                window,
                cx,
            );
            window.focus(&editor.focus_handle, cx);
            editor
        });
        let mut cx = EditorTestContext::for_editor(window, cx).await;
        cx.run_until_parked();

        cx.update_editor(|editor, window, cx| {
            deploy_context_menu(
                editor,
                None,
                DisplayPoint::new(DisplayRow(0), 4),
                window,
                cx,
            );
            assert!(editor.mouse_context_menu.is_some());
        });
        cx.run_until_parked();

        assert_eq!(
            cx.debug_bounds("MENU_ITEM-Open In"),
            None,
            "a buffer without a file has nothing to open in Finder or a terminal"
        );
        assert_eq!(
            cx.debug_bounds("MENU_ITEM-Git"),
            None,
            "a buffer outside a git repository has no git actions"
        );

        let show_context_actions = cx
            .debug_bounds("MENU_ITEM-Show Context Actions")
            .expect("Show Context Actions should be rendered");
        let cut = cx
            .debug_bounds("MENU_ITEM-Cut")
            .expect("Cut should be rendered");
        let rename = cx
            .debug_bounds("MENU_ITEM-Rename…")
            .expect("Rename should be rendered");
        let compare_with_clipboard = cx
            .debug_bounds("MENU_ITEM-Compare with Clipboard")
            .expect("Compare with Clipboard should be rendered");

        let single_separator_gap = cut.origin.y - show_context_actions.origin.y;
        let rename_to_compare_gap = compare_with_clipboard.origin.y - rename.origin.y;
        assert!(
            (rename_to_compare_gap - single_separator_gap).abs() < px(0.5),
            "hidden Open In and Git sections must leave exactly one separator between Rename and Compare with Clipboard, \
             gap was {rename_to_compare_gap:?}, expected {single_separator_gap:?}"
        );
    }
}
