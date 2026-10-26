//! Opens every kind of floating surface the app has, in the configurations a Mac runs it in,
//! and checks each frame against the UI standard.

use std::path::{Path, PathBuf};

use git_ui::branch_operations::{BranchContext, BranchNotice, push_dialog::PushDialog};
use gpui::{
    AppContext as _, BorrowAppContext as _, Entity, Modifiers, MouseButton, TestAppContext,
    VisualTestContext, px,
};
use notifications::status_toast::StatusToast;
use serde_json::json;
use settings::{SettingsStore, UiDensity};
use ui::standard_conformance::assert_follows_the_ui_standard;
use ui::{BALLOON_METRICS, DialogWidth, POPUP_MAX_CONTENT_WIDTH, SurfaceKind};
use util::path;
use workspace::notifications::{NotificationId, simple_message_notification::MessageNotification};
use workspace::{MultiWorkspace, OpenOptions, Panel as _, Workspace, open_paths};

use super::tests::init_test;

/// (ui font size, scale factor, density): IntelliJ's 13 px on a non-Retina display, Flint's
/// default on Retina, and a large compact UI on Retina.
const CONFIGURATIONS: [(f32, f32, UiDensity); 3] = [
    (13., 1., UiDensity::Default),
    (15., 2., UiDensity::Default),
    (20., 2., UiDensity::Compact),
];

const LONG_BRANCH: &str = "QA-10-Zod-CP-UI-API-with-a-branch-name-long-enough-to-wrap-and-elide";

struct Notices;

struct Project {
    workspace: Entity<Workspace>,
    cx: VisualTestContext,
}

async fn open_project(cx: &mut TestAppContext) -> Project {
    let app_state = init_test(cx);
    let root = path!("/ui-standard");
    let fs = app_state.fs.as_fake();
    fs.insert_tree(
        root,
        json!({
            ".git": {},
            "src": { "main.rs": "fn main() {}\n", "lib.rs": "pub fn lib() {}\n" },
        }),
    )
    .await;
    let dot_git = Path::new(root).join(".git");
    fs.insert_branches(
        &dot_git,
        &[
            "main",
            LONG_BRANCH,
            "origin/main",
            &format!("origin/{LONG_BRANCH}"),
        ],
    );
    fs.set_branch_name(&dot_git, Some(LONG_BRANCH));
    fs.set_remote_for_repo(&dot_git, "origin", "https://example.com/repo.git");
    cx.run_until_parked();

    cx.update(|cx| {
        open_paths(
            &[PathBuf::from(root)],
            app_state.clone(),
            OpenOptions::default(),
            cx,
        )
    })
    .await
    .expect("the project opens");
    cx.run_until_parked();

    let window = cx.update(|cx| {
        cx.windows()[0]
            .downcast::<MultiWorkspace>()
            .expect("a project window")
    });
    let workspace = window
        .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
        .expect("the window is alive");
    let mut visual_cx = VisualTestContext::from_window(window.into(), cx);
    for file in ["src/main.rs", "src/lib.rs"] {
        let open = workspace.update_in(&mut visual_cx, |workspace, window, cx| {
            workspace.open_abs_path(
                Path::new(root).join(file),
                OpenOptions::default(),
                window,
                cx,
            )
        });
        open.await.expect("the file opens");
    }
    visual_cx.run_until_parked();
    Project {
        workspace,
        cx: visual_cx,
    }
}

fn configure(project: &mut Project, ui_font_size: f32, scale: f32, density: UiDensity) {
    project.cx.update(|_, cx| {
        cx.update_global::<SettingsStore, _>(|store, cx| {
            store.update_user_settings(cx, |settings| {
                settings.theme.ui_font_size = Some(ui_font_size.into());
                settings.theme.ui_density = Some(density);
                settings.workspace.use_system_prompts = Some(false);
            });
        });
    });
    project.cx.simulate_scale_factor_change(scale);
    project.cx.run_until_parked();
}

fn close_everything(project: &mut Project) {
    project
        .workspace
        .update_in(&mut project.cx, |workspace, window, cx| {
            workspace.hide_modal(window, cx);
            workspace.clear_all_notifications(cx);
        });
    project.cx.run_until_parked();
}

fn repository_context(project: &mut Project) -> BranchContext {
    let repository = project
        .workspace
        .read_with(&project.cx, |workspace, cx| {
            workspace.project().read(cx).active_repository(cx)
        })
        .expect("the project has a repository");
    BranchContext::new(project.workspace.downgrade(), repository)
}

#[track_caller]
fn bounds_of(project: &mut Project, selector: &'static str) -> gpui::Bounds<gpui::Pixels> {
    project
        .cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is painted"))
}

#[gpui::test]
async fn every_surface_of_the_app_follows_the_ui_standard(cx: &mut TestAppContext) {
    let mut project = open_project(cx).await;
    for (ui_font_size, scale, density) in CONFIGURATIONS {
        configure(&mut project, ui_font_size, scale, density);
        let configuration = format!("at {ui_font_size} px, {scale}x, {density:?}");

        assert_follows_the_ui_standard(
            &mut project.cx,
            &format!("the workspace with editor tabs {configuration}"),
        );

        project
            .workspace
            .update_in(&mut project.cx, |workspace, window, cx| {
                git_ui::branches_popup::open_modal(workspace, window, cx)
            });
        assert_follows_the_ui_standard(
            &mut project.cx,
            &format!("the branches popup {configuration}"),
        );
        let popup = bounds_of(&mut project, SurfaceKind::Popup.debug_selector());
        assert!(
            popup.size.width <= POPUP_MAX_CONTENT_WIDTH,
            "the branches popup stops at the standard width {configuration}: {popup:?}"
        );
        close_everything(&mut project);

        let context = repository_context(&mut project);
        project
            .workspace
            .update_in(&mut project.cx, |workspace, window, cx| {
                workspace.toggle_modal(window, cx, |window, cx| {
                    PushDialog::new(context, LONG_BRANCH.into(), window, cx)
                })
            });
        assert_follows_the_ui_standard(
            &mut project.cx,
            &format!("the push dialog {configuration}"),
        );
        let dialog = bounds_of(&mut project, SurfaceKind::Dialog.debug_selector());
        let rem_size = project.cx.update(|window, _| window.rem_size());
        assert_eq!(
            dialog.size.width,
            DialogWidth::Large.pixels(rem_size),
            "the push dialog has the Large standard width {configuration}"
        );
        assert!(
            project.cx.debug_bounds("CLOSE_BUTTON-dismiss").is_some(),
            "the push dialog closes with the standard close button {configuration}"
        );
        close_everything(&mut project);

        let context = repository_context(&mut project);
        project.cx.update(|_, cx| {
            context.notify(
                BranchNotice::info("")
                    .title(format!("Pushed 1 commit to origin/{LONG_BRANCH}"))
                    .action("Create Pull Request", |_, _| {}),
                cx,
            )
        });
        assert_follows_the_ui_standard(
            &mut project.cx,
            &format!("the push result balloon {configuration}"),
        );
        let balloon = bounds_of(&mut project, SurfaceKind::Balloon.debug_selector());
        assert_eq!(balloon.size.width, BALLOON_METRICS.width);
        let title = bounds_of(&mut project, "BALLOON_TITLE");
        let line_height = project.cx.update(|window, _| window.line_height());
        assert!(
            title.size.height > line_height * 1.5,
            "the push result title wraps instead of being cut off {configuration}: {title:?}"
        );
        close_everything(&mut project);

        let toast = project
            .workspace
            .update_in(&mut project.cx, |workspace, _, cx| {
                workspace.show_notification(NotificationId::unique::<Notices>(), cx, |cx| {
                    cx.new(|cx| {
                        MessageNotification::new("Settings could not be parsed", cx)
                            .with_title("Invalid settings")
                            .primary_message("Open Settings")
                    })
                });
                let toast = StatusToast::new("Fetched origin", cx, |toast, _| {
                    toast.action("View Log", |_, _| {})
                });
                workspace.toggle_status_toast(toast.clone(), cx);
                toast
            });
        assert_follows_the_ui_standard(
            &mut project.cx,
            &format!("a message notification and a status toast {configuration}"),
        );
        let balloons = project
            .cx
            .debug_bounds_with_prefix(SurfaceKind::Balloon.debug_selector());
        assert_eq!(
            balloons.len(),
            2,
            "the toast is a balloon in the notification stack {configuration}"
        );
        for (_, balloon) in &balloons {
            assert_eq!(balloon.size.width, BALLOON_METRICS.width);
        }
        project
            .workspace
            .update_in(&mut project.cx, |workspace, _, cx| {
                workspace.toggle_status_toast(toast, cx)
            });
        close_everything(&mut project);

        project
            .cx
            .dispatch_action(zed_actions::command_palette::Toggle);
        project.cx.run_until_parked();
        assert_follows_the_ui_standard(
            &mut project.cx,
            &format!("the command palette {configuration}"),
        );
        close_everything(&mut project);

        let tab = project
            .cx
            .debug_bounds_with_prefix("TAB-")
            .into_iter()
            .map(|(_, bounds)| bounds)
            .next()
            .expect("an editor tab is painted");
        project
            .cx
            .simulate_mouse_down(tab.center(), MouseButton::Right, Modifiers::none());
        project
            .cx
            .simulate_mouse_up(tab.center(), MouseButton::Right, Modifiers::none());
        project.cx.run_until_parked();
        assert_follows_the_ui_standard(
            &mut project.cx,
            &format!("the tab context menu {configuration}"),
        );
        let viewport = project.cx.update(|window, _| window.viewport_size());
        project.cx.simulate_click(
            gpui::point(viewport.width / 2., viewport.height / 2.),
            Modifiers::none(),
        );
        project.cx.run_until_parked();
        assert!(
            project
                .cx
                .debug_bounds(SurfaceKind::Popup.debug_selector())
                .is_none(),
            "a click outside closes the tab context menu {configuration}"
        );

        let close = project
            .cx
            .debug_bounds_with_prefix("CLOSE_BUTTON-")
            .into_iter()
            .map(|(_, bounds)| bounds)
            .next()
            .expect("the active tab shows its close button");
        // An inactive tab reveals its close button only once the tab itself is hovered.
        project.cx.simulate_mouse_move(
            close.center() - gpui::point(px(20.), px(0.)),
            None,
            Modifiers::none(),
        );
        project.cx.run_until_parked();
        project
            .cx
            .simulate_mouse_move(close.center(), None, Modifiers::none());
        project
            .cx
            .executor()
            .advance_clock(std::time::Duration::from_secs(2));
        project.cx.run_until_parked();
        assert_follows_the_ui_standard(
            &mut project.cx,
            &format!("the close tab tooltip {configuration}"),
        );
        assert!(
            project
                .cx
                .debug_bounds(SurfaceKind::Tooltip.debug_selector())
                .is_some(),
            "hovering the tab close button shows its tooltip {configuration}"
        );
        project
            .cx
            .simulate_mouse_move(gpui::point(px(1.), px(1.)), None, Modifiers::none());
        project.cx.run_until_parked();
    }
}

#[gpui::test]
async fn every_tool_window_is_glass_but_the_project_panel(cx: &mut TestAppContext) {
    let project = open_project(cx).await;
    project.workspace.read_with(&project.cx, |workspace, cx| {
        let project_panel = workspace
            .panel::<project_panel::ProjectPanel>(cx)
            .expect("the Project panel is part of the window");
        assert!(
            !project_panel.read(cx).glass_background(cx),
            "the Project panel reads like the editor, so it stays opaque"
        );

        let glass_panels = [
            (
                "Terminal",
                workspace
                    .panel::<terminal_view::terminal_panel::TerminalPanel>(cx)
                    .map(|panel| panel.read(cx).glass_background(cx)),
            ),
            (
                "Git",
                workspace
                    .panel::<git_ui::git_panel::GitPanel>(cx)
                    .map(|panel| panel.read(cx).glass_background(cx)),
            ),
            (
                "Problems",
                workspace
                    .panel::<diagnostics::ProjectDiagnosticsPanel>(cx)
                    .map(|panel| panel.read(cx).glass_background(cx)),
            ),
            (
                "Search",
                workspace
                    .panel::<search_everywhere::SearchPanel>(cx)
                    .map(|panel| panel.read(cx).glass_background(cx)),
            ),
            (
                "Language Services",
                workspace
                    .panel::<language_tools::LanguageServicesPanel>(cx)
                    .map(|panel| panel.read(cx).glass_background(cx)),
            ),
        ];
        for (name, glass_background) in glass_panels {
            assert_eq!(
                glass_background,
                Some(true),
                "the {name} tool window is part of the window and is glass"
            );
        }
    });
}

/// The opaque fills painted inside the island of the tool window of panel type `T` that cover a
/// large part of it: on glass, those hide the blurred background the island is made of, which is
/// how a placeholder written for an opaque panel flashes over the glass. The window is measured in
/// the first frame after the tool window opens, when its content is still starting, and again
/// once everything has loaded.
fn opaque_fills_covering_the_island<T: workspace::Panel>(project: &mut Project) -> Vec<String> {
    use workspace::dock::DockPosition;

    project
        .workspace
        .update_in(&mut project.cx, |workspace, window, cx| {
            workspace.focus_panel::<T>(window, cx);
        });
    let selector = project
        .workspace
        .read_with(&project.cx, |workspace, cx| {
            [
                (DockPosition::Left, "tool_window_left"),
                (DockPosition::Bottom, "tool_window_bottom"),
                (DockPosition::Right, "tool_window_right"),
            ]
            .into_iter()
            .find(|(position, _)| {
                workspace
                    .dock_at_position(*position)
                    .read(cx)
                    .panel::<T>()
                    .is_some()
            })
            .map(|(_, selector)| selector)
        })
        .expect("the panel lives in one of the docks");

    let mut offenders = Vec::new();
    for stage in ["while starting", "once loaded"] {
        if stage == "while starting" {
            project.cx.update(|window, cx| window.draw(cx).clear(cx));
        } else {
            project.cx.run_until_parked();
        }
        let island = bounds_of(project, selector);
        let (scale, quads, backdrops) = project.cx.update(|window, _| {
            (
                window.scale_factor(),
                window.painted_quads(),
                window.painted_backdrops(),
            )
        });
        let island = island.scale(scale);
        let island_area = island.size.width.0 * island.size.height.0;
        let glass_order = backdrops
            .iter()
            .filter(|backdrop| backdrop.bounds.contains(&island.center()))
            .map(|backdrop| backdrop.order)
            .max()
            .expect("the island blurs its background");
        offenders.extend(
            quads
                .into_iter()
                .filter(|quad| quad.order > glass_order)
                .filter(|quad| {
                    quad.background
                        .as_solid()
                        .is_some_and(|color| color.a >= 0.9)
                })
                .filter_map(|quad| {
                    let covered = quad.bounds.intersect(&island);
                    let area = covered.size.width.0.max(0.) * covered.size.height.0.max(0.);
                    (area >= island_area * 0.15).then(|| {
                        format!(
                            "{stage}: {:?} covers {:.0}% of the island",
                            quad.background.as_solid(),
                            area * 100. / island_area
                        )
                    })
                }),
        );
    }
    offenders
}

#[gpui::test]
async fn no_tool_window_covers_its_glass_with_an_opaque_fill(cx: &mut TestAppContext) {
    // Opening the Terminal tool window starts a real shell, whose reader runs on its own thread.
    cx.executor().allow_parking();
    let mut project = open_project(cx).await;
    // A background image is what makes an empty pane paint the editor background over it.
    project.cx.update(|window, cx| {
        let images = ui::WindowBackgroundImages {
            editor_and_tools: Some(ui::BackgroundImagePaintLayer {
                image: std::sync::Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
                    image::RgbaImage::new(4, 4),
                )])),
                mode: ui::BackgroundImageMode::Scale,
                alignment: ui::BackgroundImageAlignment::Center,
            }),
            empty_frame: None,
        };
        ui::BackgroundImageLayers::set_window(window.window_handle().window_id(), images, cx);
        window.refresh();
    });
    project.cx.run_until_parked();
    let mut offenders = Vec::new();
    for (name, fills) in [
        (
            "Terminal",
            opaque_fills_covering_the_island::<terminal_view::terminal_panel::TerminalPanel>(
                &mut project,
            ),
        ),
        (
            "Git",
            opaque_fills_covering_the_island::<git_ui::git_panel::GitPanel>(&mut project),
        ),
        (
            "Problems",
            opaque_fills_covering_the_island::<diagnostics::ProjectDiagnosticsPanel>(&mut project),
        ),
        (
            "Search",
            opaque_fills_covering_the_island::<search_everywhere::SearchPanel>(&mut project),
        ),
        (
            "Language Services",
            opaque_fills_covering_the_island::<language_tools::LanguageServicesPanel>(&mut project),
        ),
    ] {
        offenders.extend(fills.into_iter().map(|fill| format!("{name}: {fill}")));
    }
    assert!(
        offenders.is_empty(),
        "tool windows paint opaque fills over their glass:\n{}",
        offenders.join("\n")
    );
}
