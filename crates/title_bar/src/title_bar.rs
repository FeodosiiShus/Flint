mod application_menu;
mod title_bar_settings;
mod toolbar_widgets;

use crate::application_menu::{ApplicationMenu, show_menus};
use git_ui_core::worktree_picker::WorktreePicker;
pub use platform_title_bar::{
    self, DraggedWindowTab, MergeAllWindows, MoveTabToNewWindow, PlatformTitleBar,
    ShowNextWindowTab, ShowPreviousWindowTab,
};
use project::{linked_worktree_short_name, repo_identity_path_if_local};

#[cfg(not(target_os = "macos"))]
use crate::application_menu::{
    ActivateDirection, ActivateMenuLeft, ActivateMenuRight, OpenApplicationMenu,
};

use gpui::{
    AnyElement, App, Context, Entity, Hsla, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Render, Styled, Subscription, WeakEntity, Window, actions, div, point, px, rgb,
};
use project::{
    Project, git_store::GitStoreEvent, project_settings::ProjectSettings,
    trusted_worktrees::TrustedWorktrees,
};
use recent_projects::RecentProjects;
use settings::Settings as _;

use std::{path::Path, sync::Arc};
use theme::UiDensity;
use title_bar_settings::TitleBarSettings;
use toolbar_widgets::{
    ToolbarWidget, ToolbarWidgetColors, avatar_initials, toolbar_widget_icon_color,
    upstream_tracking,
};
use ui::{
    ContextMenu, IconButton, PopoverMenu, TintColor, Tooltip, prelude::*,
    utils::platform_title_bar_height,
};
use util::{ResultExt, paths::PathExt as _};
use workspace::{
    AccessibleMode, MultiWorkspace, ToggleWorktreeSecurity, Workspace, WorkspaceSettings,
};
use zed_actions::OpenSettings;

const MAX_BRANCH_NAME_LENGTH: usize = 40;
const MAX_SHORT_SHA_LENGTH: usize = 8;
pub const SEARCH_BUTTON_DEBUG_SELECTOR: &str = "title_bar_search_button";
const SETTINGS_BUTTON_DEBUG_SELECTOR: &str = "title_bar_settings_button";
const NAVIGATION_BUTTONS_DEBUG_SELECTOR: &str = "title_bar_navigation_buttons";
const TITLE_BAR_ACTIONS_DEBUG_SELECTOR: &str = "title_bar_actions";
const PROJECT_WIDGET_DEBUG_SELECTOR: &str = "title_bar_project_widget";
const WORKTREE_AND_BRANCH_DEBUG_SELECTOR: &str = "title_bar_worktree_and_branch";
const VERSION_CONTROL_SETUP_DEBUG_SELECTOR: &str = "title_bar_version_control_setup";
const DETACHED_HEAD_TOOLTIP: &str = "Git: Detached HEAD doesn't point to any branch";
const PROJECT_GRADIENT_COLOR: u32 = 0x955AE0;
const TITLE_BAR_BUTTON_SIDE: Pixels = px(22.);
const TITLE_BAR_BUTTON_RADIUS: Pixels = px(6.);
const TITLE_BAR_BUTTON_RADIUS_COMPACT: Pixels = px(4.);
const TITLE_BAR_ACTION_GAP: Pixels = px(10.);

fn linked_worktree_name_anchor<'a>(
    main_worktree_path: Option<&'a Path>,
    repository_identity_path: Option<&'a Path>,
    is_linked_worktree: bool,
) -> Option<&'a Path> {
    main_worktree_path.or_else(|| {
        is_linked_worktree
            .then_some(repository_identity_path)
            .flatten()
    })
}

actions!(
    collab,
    [
        /// Switches to a different git branch.
        SwitchBranch,
    ]
);

pub fn init(cx: &mut App) {
    platform_title_bar::PlatformTitleBar::init(cx);

    cx.observe_new(|workspace: &mut Workspace, window, cx| {
        let Some(window) = window else {
            return;
        };
        let multi_workspace = workspace.multi_workspace().cloned();
        let item = cx.new(|cx| TitleBar::new("title-bar", workspace, multi_workspace, window, cx));
        workspace.set_titlebar_item(item.into(), window, cx);

        #[cfg(not(target_os = "macos"))]
        workspace.register_action(|workspace, action: &OpenApplicationMenu, window, cx| {
            if let Some(titlebar) = workspace
                .titlebar_item()
                .and_then(|item| item.downcast::<TitleBar>().ok())
            {
                titlebar.update(cx, |titlebar, cx| {
                    if let Some(ref menu) = titlebar.application_menu {
                        menu.update(cx, |menu, cx| menu.open_menu(action, window, cx));
                    }
                });
            }
        });

        #[cfg(not(target_os = "macos"))]
        workspace.register_action(|workspace, _: &ActivateMenuRight, window, cx| {
            if let Some(titlebar) = workspace
                .titlebar_item()
                .and_then(|item| item.downcast::<TitleBar>().ok())
            {
                titlebar.update(cx, |titlebar, cx| {
                    if let Some(ref menu) = titlebar.application_menu {
                        menu.update(cx, |menu, cx| {
                            menu.navigate_menus_in_direction(ActivateDirection::Right, window, cx)
                        });
                    }
                });
            }
        });

        #[cfg(not(target_os = "macos"))]
        workspace.register_action(|workspace, _: &ActivateMenuLeft, window, cx| {
            if let Some(titlebar) = workspace
                .titlebar_item()
                .and_then(|item| item.downcast::<TitleBar>().ok())
            {
                titlebar.update(cx, |titlebar, cx| {
                    if let Some(ref menu) = titlebar.application_menu {
                        menu.update(cx, |menu, cx| {
                            menu.navigate_menus_in_direction(ActivateDirection::Left, window, cx)
                        });
                    }
                });
            }
        });
    })
    .detach();
}

pub struct TitleBar {
    platform_titlebar: Entity<PlatformTitleBar>,
    project: Entity<Project>,
    workspace: WeakEntity<Workspace>,
    multi_workspace: Option<WeakEntity<MultiWorkspace>>,
    application_menu: Option<Entity<ApplicationMenu>>,
    _subscriptions: Vec<Subscription>,
}

impl Render for TitleBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.multi_workspace.is_none() {
            if let Some(mw) = self
                .workspace
                .upgrade()
                .and_then(|ws| ws.read(cx).multi_workspace().cloned())
            {
                self.multi_workspace = Some(mw.clone());
                self.platform_titlebar.update(cx, |titlebar, _cx| {
                    titlebar.set_multi_workspace(mw);
                });
            }
        }

        let title_bar_settings = *TitleBarSettings::get_global(cx);
        let button_layout = title_bar_settings.button_layout;
        let is_git_enabled = ProjectSettings::get_global(cx).git.enabled.status;

        let show_menus = show_menus(cx);

        let mut project_name = None;
        let mut project_path = None;
        let mut repository = None;
        let mut linked_worktree_name = None;
        if let Some(worktree) = self.effective_active_worktree(cx) {
            repository = self.get_repository_for_worktree(&worktree, cx);
            project_name = worktree
                .read(cx)
                .root_name()
                .file_name()
                .map(|name| SharedString::from(name.to_string()));
            project_path = Some(worktree.read(cx).abs_path());
            if let Some(repo) = &repository {
                let repo = repo.read(cx);
                let identity_fallback =
                    repo_identity_path_if_local(&repo.common_dir_abs_path, repo.path_style);
                linked_worktree_name = linked_worktree_name_anchor(
                    repo.main_worktree_abs_path(),
                    identity_fallback,
                    repo.is_linked_worktree(),
                )
                .and_then(|name_anchor_path| {
                    linked_worktree_short_name(
                        name_anchor_path,
                        repo.work_directory_abs_path.as_ref(),
                    )
                })
                .or_else(|| {
                    repo.is_linked_worktree()
                        .then_some(project_name.clone())
                        .flatten()
                });
            }
        }

        // WebStorm's Islands turn the project gradient off while any background image is set.
        let project_gradient = (project_name.is_some()
            && title_bar_settings.show_project_gradient
            && !ui::has_any_background_image(window, cx))
        .then(Self::project_gradient_color);
        // IntelliJ keeps its VCS widget in a project without version control, as a
        // "Version Control" combo that offers to create a repository.
        let show_version_control_setup = project_name.is_some()
            && repository.is_none()
            && is_git_enabled
            && title_bar_settings.show_branch_name;

        let (can_go_back, can_go_forward) = self
            .workspace
            .upgrade()
            .map(|workspace| {
                workspace.read_with(cx, |workspace, cx| {
                    let pane = workspace.active_pane().read(cx);
                    (pane.can_navigate_backward(), pane.can_navigate_forward())
                })
            })
            .unwrap_or((false, false));

        let project_items = h_flex()
            .h_full()
            .gap_0p5()
            .map(|title_bar| {
                let mut render_project_items = true;
                title_bar
                    .child(Self::render_navigation_buttons(
                        can_go_back,
                        can_go_forward,
                        self.workspace.clone(),
                        cx,
                    ))
                    .when_some(
                        self.application_menu.clone().filter(|_| !show_menus),
                        |title_bar, menu| {
                            // Hide the project/branch items to make room when the
                            // menu bar is expanded -- except in accessible mode,
                            // where the menu bar is always expanded but those
                            // controls must still remain reachable.
                            render_project_items &= !menu
                                .update(cx, |menu, cx| menu.all_menus_shown(cx))
                                || cx.accessible_mode();
                            title_bar.child(menu)
                        },
                    )
                    .children(self.render_restricted_mode(cx))
                    .when(render_project_items, |title_bar| {
                        title_bar
                            .when_some(project_name.clone(), |title_bar, project_name| {
                                title_bar
                                    .child(self.render_project_widget(project_name, project_path))
                            })
                            .when_some(
                                repository.filter(|_| is_git_enabled),
                                |title_bar, repository| {
                                    title_bar.children(self.render_worktree_and_branch(
                                        repository,
                                        linked_worktree_name,
                                        cx,
                                    ))
                                },
                            )
                            .when(show_version_control_setup, |title_bar| {
                                title_bar.child(Self::render_version_control_setup())
                            })
                    })
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .into_any_element();

        let title_bar_actions = Self::render_title_bar_actions(
            title_bar_settings.show_search_button,
            project_gradient.is_some(),
            cx,
        );

        if show_menus {
            self.platform_titlebar.update(cx, |this, _| {
                this.set_button_layout(button_layout);
                this.set_project_gradient(project_gradient);
                this.set_children(
                    self.application_menu
                        .clone()
                        .map(|menu| menu.into_any_element()),
                );
            });

            let height = platform_title_bar_height(window, cx);
            let title_bar_color = self.platform_titlebar.update(cx, |platform_titlebar, cx| {
                platform_titlebar.title_bar_color(window, cx)
            });
            let content_opacity = WorkspaceSettings::get_global(cx)
                .islands
                .frame_content_opacity(window.is_window_active());

            v_flex()
                .w_full()
                .child(self.platform_titlebar.clone().into_any_element())
                .child(
                    h_flex()
                        .bg(title_bar_color)
                        .child(ui::background_image_layer(
                            ui::BackgroundImageTarget::EditorAndTools,
                            ui::BackgroundImageArea::Window,
                            title_bar_color,
                            false,
                            gpui::Corners::default(),
                        ))
                        .when_some(project_gradient, |menu_row, color| {
                            menu_row.child(platform_title_bar::project_gradient_overlay(
                                color,
                                content_opacity,
                            ))
                        })
                        .h(height)
                        .pl_2()
                        .w_full()
                        .child(
                            h_flex()
                                .size_full()
                                .justify_between()
                                .opacity(content_opacity)
                                .child(project_items)
                                .child(title_bar_actions),
                        ),
                )
                .into_any_element()
        } else {
            self.platform_titlebar.update(cx, |this, _| {
                this.set_button_layout(button_layout);
                this.set_project_gradient(project_gradient);
                this.set_children([project_items, title_bar_actions.into_any_element()]);
            });
            self.platform_titlebar.clone().into_any_element()
        }
    }
}

impl TitleBar {
    pub fn new(
        id: impl Into<ElementId>,
        workspace: &Workspace,
        multi_workspace: Option<WeakEntity<MultiWorkspace>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let project = workspace.project().clone();
        let git_store = project.read(cx).git_store().clone();

        let platform_style = PlatformStyle::platform();
        let application_menu = match platform_style {
            PlatformStyle::Mac => {
                if option_env!("ZED_USE_CROSS_PLATFORM_MENU").is_some() {
                    Some(cx.new(|cx| ApplicationMenu::new(window, cx)))
                } else {
                    None
                }
            }
            PlatformStyle::Linux | PlatformStyle::Windows => {
                Some(cx.new(|cx| ApplicationMenu::new(window, cx)))
            }
        };

        let mut subscriptions = Vec::new();
        subscriptions.push(
            cx.observe(&workspace.weak_handle().upgrade().unwrap(), |_, _, cx| {
                cx.notify()
            }),
        );

        subscriptions.push(
            cx.subscribe(&git_store, move |_, _, event, cx| match event {
                GitStoreEvent::ActiveRepositoryChanged(_)
                | GitStoreEvent::RepositoryUpdated(_, _, true) => {
                    cx.notify();
                }
                _ => {}
            }),
        );
        if let Some(workspace_entity) = workspace.weak_handle().upgrade() {
            subscriptions.push(cx.subscribe(
                &workspace_entity,
                |_, _, event: &workspace::Event, cx| {
                    if matches!(event, workspace::Event::WorktreeCreationChanged) {
                        cx.notify();
                    }
                },
            ));
        }
        subscriptions.push(cx.observe_button_layout_changed(window, |_, _, cx| cx.notify()));
        if let Some(trusted_worktrees) = TrustedWorktrees::try_get_global(cx) {
            subscriptions.push(cx.subscribe(&trusted_worktrees, |_, _, _, cx| {
                cx.notify();
            }));
        }

        let platform_titlebar = cx.new(|cx| {
            let mut titlebar = PlatformTitleBar::new(id, cx);
            if let Some(mw) = multi_workspace.clone() {
                titlebar = titlebar.with_multi_workspace(mw);
            }
            titlebar
        });

        Self {
            platform_titlebar,
            application_menu,
            workspace: workspace.weak_handle(),
            multi_workspace,
            project,
            _subscriptions: subscriptions,
        }
    }

    /// Returns the worktree to display in the title bar.
    /// - Prefer the worktree owning the project's active repository
    /// - Fall back to the first visible worktree
    pub fn effective_active_worktree(&self, cx: &App) -> Option<Entity<project::Worktree>> {
        let project = self.project.read(cx);

        if let Some(repo) = project.active_repository(cx) {
            let repo = repo.read(cx);
            let repo_path = &repo.work_directory_abs_path;

            for worktree in project.visible_worktrees(cx) {
                let worktree_path = worktree.read(cx).abs_path();
                if worktree_path == *repo_path || worktree_path.starts_with(repo_path.as_ref()) {
                    return Some(worktree);
                }
            }
        }

        project.visible_worktrees(cx).next()
    }

    fn get_repository_for_worktree(
        &self,
        worktree: &Entity<project::Worktree>,
        cx: &App,
    ) -> Option<Entity<project::git_store::Repository>> {
        let project = self.project.read(cx);
        let git_store = project.git_store().read(cx);
        let worktree_path = worktree.read(cx).abs_path();

        git_store
            .repositories()
            .values()
            .filter(|repo| {
                let repo_path = &repo.read(cx).work_directory_abs_path;
                worktree_path == *repo_path || worktree_path.starts_with(repo_path.as_ref())
            })
            .max_by_key(|repo| repo.read(cx).work_directory_abs_path.as_os_str().len())
            .cloned()
    }

    pub fn render_restricted_mode(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let has_restricted_worktrees =
            TrustedWorktrees::has_restricted_worktrees(&self.project.read(cx).worktree_store(), cx);
        if !has_restricted_worktrees {
            return None;
        }

        let button = Button::new("restricted_mode_trigger", "Restricted Mode")
            .style(ButtonStyle::Tinted(TintColor::Warning))
            .label_size(LabelSize::Small)
            .color(Color::Warning)
            .chrome_region(ui::ChromeRegion::TitleBar)
            .start_icon(
                Icon::new(IconName::Warning)
                    .size(IconSize::Small)
                    .color(Color::Warning),
            )
            .tooltip(|_, cx| {
                Tooltip::with_meta(
                    "You're in Restricted Mode",
                    Some(&ToggleWorktreeSecurity),
                    "Mark this project as trusted and unlock all features",
                    cx,
                )
            })
            .on_click({
                cx.listener(move |this, _, window, cx| {
                    this.workspace
                        .update(cx, |workspace, cx| {
                            workspace.show_worktree_trust_security_modal(true, window, cx)
                        })
                        .log_err();
                })
            });

        if ui::utils::MACOS_SDK_26_OR_LATER {
            // Make up for Tahoe's traffic light buttons having less spacing around them
            Some(div().child(button).ml_0p5().into_any_element())
        } else {
            Some(button.into_any_element())
        }
    }

    fn project_gradient_color() -> Hsla {
        rgb(PROJECT_GRADIENT_COLOR).into()
    }

    /// The icon size, the fixed square side and the corner radius of the title bar icon buttons.
    ///
    /// `IconSize::Medium` is an exact 16 px icon in a 22 px box (`MainToolbar.Icon`). With
    /// `title_bar.icon_size` set there is no fixed side, so `IconButton` scales the icon to
    /// 16·N/14 px and grows the box to at least the icon size plus 8 px.
    fn title_bar_button_metrics(cx: &App) -> (IconSize, Option<Pixels>, Pixels) {
        let radius = match theme::theme_settings(cx).ui_density(cx) {
            UiDensity::Compact => TITLE_BAR_BUTTON_RADIUS_COMPACT,
            UiDensity::Default | UiDensity::Comfortable => TITLE_BAR_BUTTON_RADIUS,
        };
        let icon_size_configured = ui::chrome_icon_scale(ui::ChromeRegion::TitleBar, cx).is_some();
        (
            IconSize::Medium,
            (!icon_size_configured).then_some(TITLE_BAR_BUTTON_SIDE),
            radius,
        )
    }

    fn render_navigation_buttons(
        can_go_back: bool,
        can_go_forward: bool,
        workspace: WeakEntity<Workspace>,
        cx: &App,
    ) -> impl IntoElement {
        let (icon_size, side, radius) = Self::title_bar_button_metrics(cx);
        let backward = IconButton::new("title_bar_navigate_backward", IconName::ArrowLeft)
            .shape(ui::IconButtonShape::Square)
            .style(ButtonStyle::Subtle)
            .chrome_region(ui::ChromeRegion::TitleBar)
            .icon_size(icon_size)
            .when_some(side, |button, side| button.box_size(side))
            .corner_radius(radius)
            .aria_label("Go Back")
            .disabled(!can_go_back)
            .tooltip(Tooltip::for_action_title("Go Back", &workspace::GoBack))
            .on_click({
                let workspace = workspace.clone();
                move |_, window, cx| {
                    if let Some(workspace) = workspace.upgrade() {
                        let pane = workspace.read(cx).active_pane().clone();
                        pane.update(cx, |pane, cx| {
                            pane.navigate_backward(&workspace::GoBack, window, cx)
                        });
                    }
                }
            });
        let forward = IconButton::new("title_bar_navigate_forward", IconName::ArrowRight)
            .shape(ui::IconButtonShape::Square)
            .style(ButtonStyle::Subtle)
            .chrome_region(ui::ChromeRegion::TitleBar)
            .icon_size(icon_size)
            .when_some(side, |button, side| button.box_size(side))
            .corner_radius(radius)
            .aria_label("Go Forward")
            .disabled(!can_go_forward)
            .tooltip(Tooltip::for_action_title(
                "Go Forward",
                &workspace::GoForward,
            ))
            .on_click({
                let workspace = workspace.clone();
                move |_, window, cx| {
                    if let Some(workspace) = workspace.upgrade() {
                        let pane = workspace.read(cx).active_pane().clone();
                        pane.update(cx, |pane, cx| {
                            pane.navigate_forward(&workspace::GoForward, window, cx)
                        });
                    }
                }
            });

        h_flex()
            .debug_selector(|| NAVIGATION_BUTTONS_DEBUG_SELECTOR.into())
            .items_center()
            .gap(px(4.))
            .child(backward)
            .child(forward)
    }

    fn render_project_widget(
        &self,
        project_name: SharedString,
        project_path: Option<Arc<Path>>,
    ) -> impl IntoElement + use<> {
        let initials: SharedString = avatar_initials(&project_name).into();
        let color = Self::project_gradient_color();
        let tooltip_text = project_widget_tooltip(&project_name, project_path.as_deref());
        let workspace = self.workspace.clone();
        let multi_workspace = self.multi_workspace.clone();

        h_flex()
            .debug_selector(|| PROJECT_WIDGET_DEBUG_SELECTOR.into())
            .child(
                PopoverMenu::new("project-widget-menu")
                    .menu(move |window, cx| {
                        let window_project_groups = multi_workspace
                            .as_ref()
                            .and_then(|multi_workspace| multi_workspace.upgrade())
                            .map(|multi_workspace| multi_workspace.read(cx).project_group_keys())
                            .unwrap_or_default();
                        RecentProjects::popover(
                            workspace.clone(),
                            window_project_groups,
                            window,
                            cx,
                        )
                    })
                    .trigger(
                        ToolbarWidget::avatar("project_widget_trigger", initials, color)
                            .label(project_name)
                            .tab_index(0)
                            .tooltip(move |_window, cx| Tooltip::simple(tooltip_text.clone(), cx)),
                    )
                    .anchor(gpui::Anchor::TopLeft)
                    // The default offset lines a context menu's item text up with the trigger;
                    // IntelliJ's `showUnderneathOf` puts the popup's own edge under the widget's.
                    .offset(point(px(0.), px(0.))),
            )
    }

    fn render_version_control_setup() -> impl IntoElement {
        h_flex()
            .debug_selector(|| VERSION_CONTROL_SETUP_DEBUG_SELECTOR.into())
            .child(
                PopoverMenu::new("version-control-setup-menu")
                    .menu(|window, cx| {
                        Some(ContextMenu::build(window, cx, |menu, _, _| {
                            menu.action("Create Git Repository…", Box::new(git::Init))
                        }))
                    })
                    .trigger(
                        ToolbarWidget::text("version_control_setup_trigger", "Version Control")
                            .tab_index(0)
                            .tooltip(|_window, cx| {
                                Tooltip::simple("Configure a version control for the project", cx)
                            }),
                    )
                    .anchor(gpui::Anchor::TopLeft),
            )
    }

    fn render_title_bar_actions(
        show_search_button: bool,
        project_gradient_active: bool,
        cx: &App,
    ) -> impl IntoElement + use<> {
        let (icon_size, side, radius) = Self::title_bar_button_metrics(cx);
        let colors = ToolbarWidgetColors::current(cx);
        // With the project gradient IntelliJ paints a pushed header button in the same
        // translucent hover colour (`getHeaderBackgroundColor`).
        let pressed_background = if project_gradient_active {
            colors.hover
        } else {
            colors.open
        };
        let search_action = zed_actions::search_everywhere::Toggle { tab: None };
        let action_button = move |id: &'static str, icon: IconName| {
            IconButton::new(id, icon)
                .shape(ui::IconButtonShape::Square)
                .style(ButtonStyle::Subtle)
                .chrome_region(ui::ChromeRegion::TitleBar)
                .icon_size(icon_size)
                .when_some(side, |button, side| button.box_size(side))
                .corner_radius(radius)
                .hover_background(colors.hover)
                .active_background(pressed_background)
        };

        h_flex()
            .debug_selector(|| TITLE_BAR_ACTIONS_DEBUG_SELECTOR.into())
            .items_center()
            .gap(TITLE_BAR_ACTION_GAP)
            // Like the widgets on the left, keep a press on these buttons from arming the title
            // bar's window drag: the buttons stop the release, so the drag would stay armed and
            // the next mouse move over the title bar would move the window.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .when(show_search_button, |this| {
                this.child(
                    div()
                        .debug_selector(|| SEARCH_BUTTON_DEBUG_SELECTOR.into())
                        .child(
                            action_button("title_bar_search_button", IconName::MagnifyingGlass)
                                .aria_label("Search Everywhere")
                                .tooltip(Tooltip::for_action_title(
                                    "Search Everywhere",
                                    &search_action,
                                ))
                                .on_click(move |_, window, cx| {
                                    window.dispatch_action(Box::new(search_action.clone()), cx)
                                }),
                        ),
                )
            })
            .child(
                // IntelliJ's gear (`SettingsEntryPointAction`) opens the `SettingsEntryPointGroup`
                // popup rather than the Settings dialog. Its Theme and View Mode entries have no
                // Flint equivalent.
                div()
                    .debug_selector(|| SETTINGS_BUTTON_DEBUG_SELECTOR.into())
                    .child(
                        PopoverMenu::new("title_bar_settings_menu")
                            .menu(|window, cx| {
                                Some(ContextMenu::build(window, cx, |menu, _, _| {
                                    menu.action("Settings…", Box::new(OpenSettings))
                                        .action(
                                            "Plugins…",
                                            Box::new(zed_actions::Extensions::default()),
                                        )
                                        .separator()
                                        .action("Keymap…", Box::new(zed_actions::OpenKeymap))
                                }))
                            })
                            .trigger_with_tooltip(
                                action_button("title_bar_settings_button", IconName::Settings)
                                    .aria_label("Settings"),
                                Tooltip::for_action_title("Settings", &OpenSettings),
                            )
                            .anchor(gpui::Anchor::TopRight),
                    ),
            )
    }

    fn render_worktree_and_branch(
        &self,
        repository: Entity<project::git_store::Repository>,
        linked_worktree_name: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let workspace = self.workspace.upgrade()?;
        let icon_color = toolbar_widget_icon_color(cx);

        let (branch_name, upstream, incoming_outgoing, icon_info, is_detached_head) = {
            let repo = repository.read(cx);

            let is_detached_head = repo.branch.is_none();

            let branch_name = repo
                .branch
                .as_ref()
                .map(|branch| branch.name())
                .map(|name| util::truncate_and_trailoff(name, MAX_BRANCH_NAME_LENGTH))
                .or_else(|| {
                    repo.head_commit.as_ref().map(|commit| {
                        commit
                            .sha
                            .chars()
                            .take(MAX_SHORT_SHA_LENGTH)
                            .collect::<String>()
                    })
                });

            let tracking_status = repo
                .branch
                .as_ref()
                .and_then(|branch| branch.tracking_status());
            let upstream =
                tracking_status.and_then(|status| upstream_tracking(status.ahead, status.behind));
            let incoming_outgoing = tracking_status
                .and_then(|status| incoming_outgoing_tooltip(status.behind, status.ahead));

            let status = repo.status_summary();
            let tracked = status.index + status.worktree;
            let icon_info = if status.conflict > 0 {
                (IconName::Warning, Color::VersionControlConflict)
            } else if tracked.modified > 0 {
                (IconName::SquareDot, Color::VersionControlModified)
            } else if tracked.added > 0 || status.untracked > 0 {
                (IconName::SquarePlus, Color::VersionControlAdded)
            } else if tracked.deleted > 0 {
                (IconName::SquareMinus, Color::VersionControlDeleted)
            } else {
                (IconName::GitBranch, icon_color)
            };

            (
                branch_name,
                upstream,
                incoming_outgoing,
                icon_info,
                is_detached_head,
            )
        };

        let settings = TitleBarSettings::get_global(cx);
        let effective_repository = Some(repository);

        let worktree_label: SharedString =
            linked_worktree_name.unwrap_or_else(|| SharedString::new_static("main"));

        let (creation_in_progress, is_switch) = self
            .workspace
            .upgrade()
            .map(|ws| {
                let creation = ws.read(cx).active_worktree_creation();
                (creation.label.clone(), creation.is_switch)
            })
            .unwrap_or((None, false));
        let is_creating = creation_in_progress.is_some();

        let display_label: SharedString = if let Some(ref name) = creation_in_progress {
            if is_switch {
                format!("Loading {}…", name).into()
            } else {
                format!("Creating {}…", name).into()
            }
        } else {
            worktree_label.clone()
        };

        let worktree_button = settings.show_worktree_name.then(|| {
            let project = self.project.clone();
            let workspace_handle = workspace.downgrade();
            PopoverMenu::new("worktree-picker-menu")
                .menu(move |window, cx| {
                    // When opened from the title bar, focus is on the trigger
                    // button (not a dock), so `focused_dock` is `None`. That's
                    // fine — there's no prior dock focus to restore.
                    Some(cx.new(|cx| {
                        WorktreePicker::new(project.clone(), workspace_handle.clone(), window, cx)
                    }))
                })
                .trigger(
                    ToolbarWidget::new(
                        "worktree_picker_trigger",
                        Icon::new(IconName::GitWorktree).color(icon_color),
                        display_label,
                    )
                    .tab_index(0)
                    .loading(is_creating)
                    .tooltip(move |_window, cx| {
                        Tooltip::with_meta(
                            "Worktree",
                            Some(&zed_actions::git::Worktree),
                            format!("Currently In Use: {}", worktree_label),
                            cx,
                        )
                    }),
                )
                .anchor(gpui::Anchor::TopLeft)
        });

        let branch_picker = branch_name.and_then(|branch_name| {
            settings.show_branch_name.then(|| {
                let branch_tooltip_label = branch_name.clone();
                let (branch_icon, branch_icon_color) = if settings.show_branch_status_icon {
                    icon_info
                } else {
                    (IconName::GitBranch, icon_color)
                };

                let trigger = if is_detached_head {
                    // IntelliJ shows the short hash with a warning icon in detached HEAD
                    // (`GitBranchUtil.getDisplayableBranchText`, `GitCurrentBranchPresenter`).
                    ToolbarWidget::new(
                        "project_branch_trigger",
                        Icon::new(IconName::Warning).color(Color::Warning),
                        branch_name,
                    )
                } else {
                    ToolbarWidget::new(
                        "project_branch_trigger",
                        Icon::new(branch_icon).color(branch_icon_color),
                        branch_name,
                    )
                    .upstream(upstream)
                }
                .tab_index(0)
                .tooltip(move |_window, cx| {
                    if is_detached_head {
                        return Tooltip::for_action(
                            DETACHED_HEAD_TOOLTIP,
                            &zed_actions::git::Branch,
                            cx,
                        );
                    }
                    let title = branch_widget_tooltip_title(&branch_tooltip_label);
                    match &incoming_outgoing {
                        Some(incoming_outgoing) => Tooltip::with_meta(
                            title,
                            Some(&zed_actions::git::Branch),
                            incoming_outgoing.clone(),
                            cx,
                        ),
                        None => Tooltip::for_action(title, &zed_actions::git::Branch, cx),
                    }
                });

                PopoverMenu::new("branch-menu")
                    .menu(move |window, cx| {
                        git_ui_core::build_branch_picker(
                            workspace.downgrade(),
                            effective_repository.clone(),
                            window,
                            cx,
                        )
                    })
                    .trigger(trigger)
                    .anchor(gpui::Anchor::TopLeft)
            })
        });

        if worktree_button.is_none() && branch_picker.is_none() {
            return None;
        }

        let show_separator = worktree_button.is_some() && branch_picker.is_some();

        Some(
            h_flex()
                .debug_selector(|| WORKTREE_AND_BRANCH_DEBUG_SELECTOR.into())
                .gap_1()
                .children(worktree_button)
                .when(show_separator, |this| {
                    this.child(
                        Label::new("/")
                            .size(LabelSize::Small)
                            .color(Color::Muted)
                            .alpha(0.25),
                    )
                })
                .children(branch_picker)
                .into_any_element(),
        )
    }
}

/// Like `ProjectToolbarWidgetAction`, the project widget tooltip is the project folder relative
/// to the home folder.
fn project_widget_tooltip(
    project_name: &SharedString,
    project_path: Option<&Path>,
) -> SharedString {
    project_path
        .map(|path| SharedString::from(path.compact().to_string_lossy().into_owned()))
        .unwrap_or_else(|| project_name.clone())
}

/// IntelliJ's VCS widget tooltip title (`tooltip.branch.widget.vcs.branch.name.text`).
fn branch_widget_tooltip_title(branch_name: &str) -> String {
    format!("Git Branch: {branch_name}")
}

/// The incoming and outgoing line of IntelliJ's VCS widget tooltip
/// (`GitInOutCountersInProject.calcTooltip` for one repository).
fn incoming_outgoing_tooltip(incoming: u32, outgoing: u32) -> Option<String> {
    let commits = |count: u32| if count == 1 { "commit" } else { "commits" };
    match (incoming, outgoing) {
        (0, 0) => None,
        (incoming, 0) => Some(format!("{incoming} incoming {}", commits(incoming))),
        (0, outgoing) => Some(format!("{outgoing} outgoing {}", commits(outgoing))),
        (incoming, outgoing) => Some(format!(
            "{incoming} incoming and {outgoing} outgoing commits"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git::repository::{UpstreamTracking, UpstreamTrackingStatus};
    use gpui::{
        Background, Bounds, Modifiers, Quad, ScaledPixels, TestAppContext, UpdateGlobal as _,
        VisualTestContext, point,
    };
    use settings::{SettingsContent, SettingsStore};
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    use util::paths::PathStyle;
    use workspace::AppState;

    const PROJECT_ROOT: &str = "/flint";
    const DETACHED_HEAD_SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    struct TestRepository {
        branch: Option<&'static str>,
        upstream: Option<UpstreamTrackingStatus>,
        head_sha: Option<&'static str>,
    }

    async fn open_project_title_bar<'a>(
        cx: &'a mut TestAppContext,
        checked_out_branch: Option<&'static str>,
    ) -> (
        Entity<Workspace>,
        Entity<TitleBar>,
        &'a mut VisualTestContext,
    ) {
        open_project_title_bar_with_upstream(cx, checked_out_branch, None).await
    }

    async fn open_project_title_bar_with_upstream<'a>(
        cx: &'a mut TestAppContext,
        checked_out_branch: Option<&'static str>,
        upstream: Option<UpstreamTrackingStatus>,
    ) -> (
        Entity<Workspace>,
        Entity<TitleBar>,
        &'a mut VisualTestContext,
    ) {
        let repository = checked_out_branch.map(|branch| TestRepository {
            branch: Some(branch),
            upstream,
            head_sha: None,
        });
        open_project_title_bar_with_repository(cx, repository).await
    }

    async fn open_project_title_bar_with_repository<'a>(
        cx: &'a mut TestAppContext,
        repository: Option<TestRepository>,
    ) -> (
        Entity<Workspace>,
        Entity<TitleBar>,
        &'a mut VisualTestContext,
    ) {
        let app_state = cx.update(|cx| {
            let app_state = AppState::test(cx);
            PlatformTitleBar::init(cx);
            app_state
        });
        app_state
            .fs
            .create_dir(Path::new(PROJECT_ROOT))
            .await
            .expect("project root should be created");
        if let Some(repository) = repository {
            let dot_git = Path::new(PROJECT_ROOT).join(".git");
            app_state
                .fs
                .create_dir(&dot_git)
                .await
                .expect("git directory should be created");
            let fake_fs = app_state.fs.as_fake();
            if let Some(branch) = repository.branch {
                fake_fs.set_branch_name(&dot_git, Some(branch));
                if let Some(tracking) = repository.upstream {
                    fake_fs.set_upstream_for_repo(
                        &dot_git,
                        branch,
                        "origin/main",
                        UpstreamTracking::Tracked(tracking),
                    );
                }
            }
            if let Some(head_sha) = repository.head_sha {
                fake_fs.set_head_for_repo(&dot_git, &[], head_sha);
            }
        }
        let project = Project::test(app_state.fs.clone(), [Path::new(PROJECT_ROOT)], cx).await;
        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project, window, cx));
        let workspace =
            multi_workspace.read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone());
        let title_bar = workspace.update_in(cx, |workspace, window, cx| {
            let title_bar = cx.new(|cx| TitleBar::new("title-bar", workspace, None, window, cx));
            workspace.set_titlebar_item(title_bar.clone().into(), window, cx);
            title_bar
        });
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        (workspace, title_bar, cx)
    }

    fn update_user_settings(cx: &mut VisualTestContext, update: impl FnOnce(&mut SettingsContent)) {
        cx.update(|window, cx| {
            SettingsStore::update_global(cx, |store, cx| store.update_user_settings(cx, update));
            window.refresh();
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    async fn project_gradient_tints_the_title_bar_from_its_left_edge(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;

        let title_bar_height = cx.update(|window, cx| platform_title_bar_height(window, cx));
        let gradient = cx
            .debug_bounds("project_gradient")
            .expect("a named project should tint the title bar with its gradient");
        let search_button = cx
            .debug_bounds(SEARCH_BUTTON_DEBUG_SELECTOR)
            .expect("the search button should be rendered in the title bar");
        assert_eq!(
            gradient.origin,
            point(px(0.), px(0.)),
            "the gradient starts at the left edge of the title bar"
        );
        assert_eq!(
            gradient.size.height, title_bar_height,
            "the gradient fills the full height of the title bar"
        );
        assert!(
            gradient.right() >= search_button.right(),
            "the gradient spans the title bar behind its widgets"
        );

        update_user_settings(cx, |settings| {
            settings
                .title_bar
                .get_or_insert_default()
                .show_project_gradient = Some(false);
        });

        assert_eq!(
            cx.debug_bounds("project_gradient"),
            None,
            "turning off show_project_gradient removes the tint"
        );
    }

    #[gpui::test]
    async fn project_gradient_is_hidden_while_a_background_image_is_set(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        assert!(cx.debug_bounds("project_gradient").is_some());

        let image = cx
            .update(|_, cx| cx.svg_renderer())
            .render_single_frame(
                br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1"/></svg>"#,
                1.,
            )
            .expect("a one pixel SVG should render");
        let layer = ui::BackgroundImagePaintLayer {
            image,
            mode: ui::BackgroundImageMode::Scale,
            alignment: ui::BackgroundImageAlignment::Center,
        };
        let publish_images = |images: ui::WindowBackgroundImages, cx: &mut VisualTestContext| {
            cx.update(|window, cx| {
                let window_id = window.window_handle().window_id();
                ui::BackgroundImageLayers::set_window(window_id, images, cx);
                window.refresh();
            });
            cx.run_until_parked();
        };

        publish_images(
            ui::WindowBackgroundImages {
                editor_and_tools: Some(layer.clone()),
                empty_frame: None,
            },
            cx,
        );
        assert_eq!(
            cx.debug_bounds("project_gradient"),
            None,
            "an editor image turns the project gradient off, like WebStorm's Islands"
        );

        publish_images(
            ui::WindowBackgroundImages {
                editor_and_tools: None,
                empty_frame: Some(layer),
            },
            cx,
        );
        assert_eq!(
            cx.debug_bounds("project_gradient"),
            None,
            "an empty frame image turns the project gradient off too"
        );

        publish_images(ui::WindowBackgroundImages::default(), cx);
        assert!(
            cx.debug_bounds("project_gradient").is_some(),
            "without background images the gradient comes back"
        );
    }

    #[gpui::test]
    async fn project_gradient_is_intellij_violet_for_every_project(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;

        let violet: Hsla = rgb(0x955AE0).into();
        let painted_quads = cx.update(|window, _| window.painted_quads());

        assert!(
            painted_quads
                .iter()
                .any(|quad| quad.background == ui::project_gradient_background(violet)),
            "the title bar gradient is painted with IntelliJ's violet project color whatever the project name"
        );
    }

    #[gpui::test]
    async fn inactive_window_dims_title_bar_content_but_not_its_background(
        cx: &mut TestAppContext,
    ) {
        let (_workspace, title_bar, cx) = open_project_title_bar(cx, None).await;

        let platform_titlebar =
            title_bar.read_with(cx, |title_bar, _| title_bar.platform_titlebar.clone());
        let title_bar_color = platform_titlebar.update_in(cx, |platform_titlebar, window, cx| {
            platform_titlebar.title_bar_color(window, cx)
        });
        let project_color = cx.update(|_, _| TitleBar::project_gradient_color());
        let inactive_opacity = cx.update(|_, cx| {
            <WorkspaceSettings as settings::Settings>::get_global(cx)
                .islands
                .frame_content_opacity(false)
        });
        let scale_factor = cx.update(|window, _| window.scale_factor());
        let avatar_chip = cx
            .debug_bounds(toolbar_widgets::PROJECT_AVATAR_CHIP_DEBUG_SELECTOR)
            .expect("the project avatar chip should be rendered in the title bar")
            .scale(scale_factor);
        let avatar_chip_fills = |quads: &[Quad]| {
            quads
                .iter()
                .filter(|quad| avatar_chip.contains(&quad.bounds.center()))
                .map(|quad| quad.background)
                .collect::<Vec<_>>()
        };
        assert!(
            inactive_opacity < 1.,
            "inactive windows dim their frame content by default"
        );
        let gradient = ui::project_gradient_background(project_color);
        let avatar_fill = toolbar_widgets::project_avatar_background(project_color);
        assert_ne!(
            avatar_fill,
            Background::from(project_color),
            "the avatar chip is IntelliJ's Avatar.End to Avatar.Start gradient, not a solid fill"
        );

        assert!(cx.update(|window, _| window.is_window_active()));
        let active_quads = cx.update(|window, _| window.painted_quads());
        assert!(
            active_quads.iter().any(|quad| quad.background == gradient),
            "an active window paints the gradient at full strength"
        );
        assert!(
            avatar_chip_fills(active_quads.as_slice()).contains(&avatar_fill),
            "an active window paints the project avatar chip at full strength"
        );

        cx.deactivate_window();
        cx.run_until_parked();

        assert!(!cx.update(|window, _| window.is_window_active()));
        let inactive_quads = cx.update(|window, _| window.painted_quads());
        assert!(
            inactive_quads
                .iter()
                .any(|quad| quad.background == gradient.opacity(inactive_opacity)),
            "an inactive window dims the gradient"
        );
        assert!(
            !inactive_quads
                .iter()
                .any(|quad| quad.background == gradient),
            "no undimmed gradient remains in an inactive window"
        );
        let inactive_avatar_fills = avatar_chip_fills(inactive_quads.as_slice());
        assert!(
            inactive_avatar_fills.contains(&avatar_fill.opacity(inactive_opacity)),
            "an inactive window dims the project avatar chip"
        );
        assert!(
            !inactive_avatar_fills.contains(&avatar_fill),
            "no undimmed project avatar chip remains in an inactive window"
        );
        assert!(
            !inactive_quads.iter().any(|quad| {
                quad.background == Background::from(title_bar_color).opacity(inactive_opacity)
            }),
            "the title bar background is never dimmed"
        );
    }

    #[gpui::test]
    async fn title_bar_actions_sit_inside_the_bar_aligned_with_its_right_edge(
        cx: &mut TestAppContext,
    ) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, Some("main")).await;
        let title_bar_height = cx.update(|window, cx| platform_title_bar_height(window, cx));
        let viewport_width = cx.update(|window, _| window.viewport_size().width);
        let actions = cx
            .debug_bounds(TITLE_BAR_ACTIONS_DEBUG_SELECTOR)
            .expect("the title bar renders its right-hand action buttons");
        let search_button = cx
            .debug_bounds(SEARCH_BUTTON_DEBUG_SELECTOR)
            .expect("the Settings gear and the search button share the right-hand group");
        let settings_button = cx
            .debug_bounds(SETTINGS_BUTTON_DEBUG_SELECTOR)
            .expect("the Settings gear is rendered in the title bar");
        let branch_widget = cx
            .debug_bounds(WORKTREE_AND_BRANCH_DEBUG_SELECTOR)
            .expect("a checked-out branch should show the branch picker");

        assert_visible_inside_title_bar(actions, title_bar_height, "title bar actions");
        assert_visible_inside_title_bar(search_button, title_bar_height, "search button");
        assert_visible_inside_title_bar(settings_button, title_bar_height, "settings button");
        assert_eq!(
            search_button.size, settings_button.size,
            "Search Everywhere and Settings are the same size button"
        );
        assert!(
            search_button.right() <= settings_button.left(),
            "Search Everywhere sits before Settings, like the IntelliJ main toolbar: {search_button:?}, {settings_button:?}"
        );
        assert!(
            viewport_width - settings_button.right() < viewport_width * 0.05,
            "the Settings gear ends at the right edge of the title bar: {settings_button:?}"
        );
        assert!(
            branch_widget.right() < search_button.left(),
            "the left-hand widgets stay left of the right-hand action buttons"
        );
    }

    #[gpui::test]
    async fn branch_widget_follows_the_upstream_tracking_of_the_branch(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar_with_upstream(
            cx,
            Some("main"),
            Some(UpstreamTrackingStatus {
                ahead: 1,
                behind: 2,
            }),
        )
        .await;

        assert!(
            cx.debug_bounds("title_bar_incoming_commits").is_some(),
            "a branch behind its upstream draws the incoming arrow"
        );
        assert!(
            cx.debug_bounds("title_bar_outgoing_commits").is_some(),
            "a branch ahead of its upstream draws the outgoing arrow"
        );
    }

    #[gpui::test]
    async fn branch_widget_draws_no_arrows_for_a_branch_in_sync(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, Some("main")).await;

        assert_eq!(cx.debug_bounds("title_bar_incoming_commits"), None);
        assert_eq!(cx.debug_bounds("title_bar_outgoing_commits"), None);
    }

    fn assert_visible_inside_title_bar(
        widget: gpui::Bounds<Pixels>,
        title_bar_height: Pixels,
        label: &str,
    ) {
        assert!(
            widget.size.width > px(0.) && widget.size.height > px(0.),
            "the {label} is drawn with a visible size, got {widget:?}"
        );
        assert!(
            widget.top() >= px(0.) && widget.bottom() <= title_bar_height,
            "the {label} sits inside the {title_bar_height:?} title bar strip, got {widget:?}"
        );
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    async fn title_bar_widgets_stay_visible_beside_the_traffic_lights_outside_full_screen(
        cx: &mut TestAppContext,
    ) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, Some("main")).await;
        let title_bar_height = cx.update(|window, cx| platform_title_bar_height(window, cx));
        let traffic_light_padding = px(ui::utils::TRAFFIC_LIGHT_PADDING);

        assert!(
            !cx.update(|window, _| window.is_fullscreen()),
            "a new project window opens outside full screen"
        );
        let branch_picker = cx
            .debug_bounds(WORKTREE_AND_BRANCH_DEBUG_SELECTOR)
            .expect("outside full screen the branch picker is shown in the title bar");
        let navigation = cx
            .debug_bounds(NAVIGATION_BUTTONS_DEBUG_SELECTOR)
            .expect("outside full screen the navigation buttons are shown in the title bar");
        let search_button = cx
            .debug_bounds(SEARCH_BUTTON_DEBUG_SELECTOR)
            .expect("outside full screen the search button is shown in the title bar");
        assert_visible_inside_title_bar(
            branch_picker,
            title_bar_height,
            "branch picker outside full screen",
        );
        assert_visible_inside_title_bar(
            navigation,
            title_bar_height,
            "navigation buttons outside full screen",
        );
        assert_visible_inside_title_bar(
            search_button,
            title_bar_height,
            "search button outside full screen",
        );
        assert!(
            navigation.left() >= traffic_light_padding,
            "outside full screen the navigation buttons start after the macOS window buttons, got {navigation:?}"
        );
        assert!(
            branch_picker.left() >= traffic_light_padding,
            "outside full screen the branch picker starts after the macOS window buttons, got {branch_picker:?}"
        );
        let windowed_navigation_left = navigation.left();
        let windowed_branch_picker_left = branch_picker.left();

        cx.update(|window, _| {
            window.toggle_fullscreen();
            window.refresh();
        });
        cx.run_until_parked();

        assert!(
            cx.update(|window, _| window.is_fullscreen()),
            "toggling full screen puts the window in full screen"
        );
        let branch_picker = cx
            .debug_bounds(WORKTREE_AND_BRANCH_DEBUG_SELECTOR)
            .expect("in full screen the branch picker is shown in the title bar");
        let navigation = cx
            .debug_bounds(NAVIGATION_BUTTONS_DEBUG_SELECTOR)
            .expect("in full screen the navigation buttons are shown in the title bar");
        let search_button = cx
            .debug_bounds(SEARCH_BUTTON_DEBUG_SELECTOR)
            .expect("in full screen the search button is shown in the title bar");
        assert_visible_inside_title_bar(
            branch_picker,
            title_bar_height,
            "branch picker in full screen",
        );
        assert_visible_inside_title_bar(
            navigation,
            title_bar_height,
            "navigation buttons in full screen",
        );
        assert_visible_inside_title_bar(
            search_button,
            title_bar_height,
            "search button in full screen",
        );
        assert!(
            navigation.left() < traffic_light_padding,
            "in full screen the navigation buttons move into the space of the hidden macOS window buttons, got {navigation:?}"
        );
        assert!(
            branch_picker.left() < windowed_branch_picker_left,
            "in full screen the branch picker shifts left into the space of the hidden macOS window buttons, got {branch_picker:?} (windowed {windowed_branch_picker_left:?})"
        );
        assert!(
            branch_picker.left() > navigation.right(),
            "in full screen the branch picker stays after the navigation buttons, got {branch_picker:?} navigation {navigation:?}"
        );
        assert!(
            navigation.left() < windowed_navigation_left,
            "in full screen the navigation buttons shift left compared to windowed mode, got {navigation:?} (windowed {windowed_navigation_left:?})"
        );

        cx.update(|window, _| {
            window.toggle_fullscreen();
            window.refresh();
        });
        cx.run_until_parked();

        assert!(
            !cx.update(|window, _| window.is_fullscreen()),
            "toggling full screen again returns the window to windowed mode"
        );
        let branch_picker = cx
            .debug_bounds(WORKTREE_AND_BRANCH_DEBUG_SELECTOR)
            .expect("after leaving full screen the branch picker is shown in the title bar");
        let navigation = cx
            .debug_bounds(NAVIGATION_BUTTONS_DEBUG_SELECTOR)
            .expect("after leaving full screen the navigation buttons are shown in the title bar");
        let search_button = cx
            .debug_bounds(SEARCH_BUTTON_DEBUG_SELECTOR)
            .expect("after leaving full screen the search button is shown in the title bar");
        assert_visible_inside_title_bar(
            branch_picker,
            title_bar_height,
            "branch picker after leaving full screen",
        );
        assert_visible_inside_title_bar(
            navigation,
            title_bar_height,
            "navigation buttons after leaving full screen",
        );
        assert_visible_inside_title_bar(
            search_button,
            title_bar_height,
            "search button after leaving full screen",
        );
        assert!(
            navigation.left() >= traffic_light_padding,
            "after leaving full screen the navigation buttons start after the macOS window buttons again, got {navigation:?}"
        );
        assert!(
            branch_picker.left() >= traffic_light_padding,
            "after leaving full screen the branch picker starts after the macOS window buttons again, got {branch_picker:?}"
        );
    }

    #[gpui::test]
    async fn clicking_the_search_button_opens_search_everywhere_on_its_default_tab(
        cx: &mut TestAppContext,
    ) {
        let (workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        let requested_tabs = Rc::new(RefCell::new(Vec::new()));
        workspace.update_in(cx, |workspace, _, cx| {
            let requested_tabs = requested_tabs.clone();
            workspace.register_action(
                move |_, action: &zed_actions::search_everywhere::Toggle, _, _| {
                    requested_tabs.borrow_mut().push(action.tab);
                },
            );
            cx.notify();
        });
        cx.run_until_parked();

        let search_button = cx
            .debug_bounds(SEARCH_BUTTON_DEBUG_SELECTOR)
            .expect("the search button should be rendered in the title bar");
        cx.simulate_click(search_button.center(), Modifiers::none());
        cx.run_until_parked();

        assert_eq!(
            *requested_tabs.borrow(),
            vec![None],
            "one click opens Search Everywhere once, on its default tab"
        );
    }

    #[gpui::test]
    async fn show_search_button_false_removes_the_search_button(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        assert!(
            cx.debug_bounds(SEARCH_BUTTON_DEBUG_SELECTOR).is_some(),
            "the search button is shown by default"
        );
        assert!(
            cx.debug_bounds(SETTINGS_BUTTON_DEBUG_SELECTOR).is_some(),
            "the Settings button does not depend on show_search_button"
        );

        update_user_settings(cx, |settings| {
            settings
                .title_bar
                .get_or_insert_default()
                .show_search_button = Some(false);
        });

        assert_eq!(
            cx.debug_bounds(SEARCH_BUTTON_DEBUG_SELECTOR),
            None,
            "turning off show_search_button removes the search button"
        );
        assert!(
            cx.debug_bounds(SETTINGS_BUTTON_DEBUG_SELECTOR).is_some(),
            "the Settings button stays when the search button is hidden"
        );
    }

    #[gpui::test]
    async fn back_and_forward_buttons_are_disabled_without_navigation_history(
        cx: &mut TestAppContext,
    ) {
        let (workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;

        let navigation = cx
            .debug_bounds(NAVIGATION_BUTTONS_DEBUG_SELECTOR)
            .expect("the navigation buttons are rendered in the title bar");
        let backward = cx
            .debug_bounds("ICON-ArrowLeft")
            .expect("the back button is rendered in the title bar");
        let forward = cx
            .debug_bounds("ICON-ArrowRight")
            .expect("the forward button is rendered in the title bar");
        assert!(
            backward.right() <= forward.left(),
            "Back sits before Forward, like the IntelliJ main toolbar: {backward:?}, {forward:?}"
        );
        assert!(
            navigation.left() <= backward.left() && forward.right() <= navigation.right(),
            "both navigation buttons sit inside the navigation group: {navigation:?}"
        );

        let can_navigate = workspace.read_with(cx, |workspace, cx| {
            let pane = workspace.active_pane().read(cx);
            (pane.can_navigate_backward(), pane.can_navigate_forward())
        });
        assert_eq!(
            can_navigate,
            (false, false),
            "a fresh project has no navigation history, so both buttons are disabled"
        );
    }

    #[track_caller]
    fn assert_square(bounds: Bounds<Pixels>, expected: f32, label: &str) {
        for side in [bounds.size.width, bounds.size.height] {
            assert!(
                (f32::from(side) - expected).abs() < 0.01,
                "the {label} is {bounds:?}, expected {expected} px square"
            );
        }
    }

    fn quads_inside(cx: &mut VisualTestContext, bounds: Bounds<Pixels>) -> Vec<Quad> {
        let (scale_factor, quads) =
            cx.update(|window, _| (window.scale_factor(), window.painted_quads()));
        let bounds = bounds.scale(scale_factor);
        quads
            .into_iter()
            .filter(|quad| bounds.contains(&quad.bounds.center()))
            .collect()
    }

    #[gpui::test]
    async fn title_bar_icon_size_grows_the_icon_buttons_around_their_icons(
        cx: &mut TestAppContext,
    ) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, Some("main")).await;
        let buttons = [
            "ICON-ArrowLeft",
            "ICON-ArrowRight",
            SEARCH_BUTTON_DEBUG_SELECTOR,
            SETTINGS_BUTTON_DEBUG_SELECTOR,
        ];
        for selector in buttons {
            let button = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} is rendered in the title bar"));
            assert_square(button, 22., selector);
        }

        update_user_settings(cx, |settings| {
            settings.title_bar.get_or_insert_default().icon_size = Some(20);
        });

        let title_bar_height = cx.update(|window, cx| platform_title_bar_height(window, cx));
        for selector in buttons {
            let button = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} is rendered in the title bar"));
            assert_square(button, 28., selector);
            assert_visible_inside_title_bar(button, title_bar_height, selector);
        }
    }

    #[gpui::test]
    async fn title_bar_height_never_cuts_off_the_toolbar_widgets(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, Some("main")).await;

        update_user_settings(cx, |settings| {
            settings.title_bar.get_or_insert_default().height = Some(24);
        });

        let title_bar_height = cx.update(|window, cx| platform_title_bar_height(window, cx));
        assert_eq!(
            title_bar_height,
            px(30.),
            "a 24 px title_bar.height is raised to the 30 px widget pills"
        );
        for selector in [
            PROJECT_WIDGET_DEBUG_SELECTOR,
            WORKTREE_AND_BRANCH_DEBUG_SELECTOR,
        ] {
            let widget = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} is rendered in the title bar"));
            assert_visible_inside_title_bar(widget, title_bar_height, selector);
        }
    }

    #[gpui::test]
    async fn toolbar_widgets_are_intellij_combo_pills_in_every_density(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, Some("main")).await;

        for (density, expected_height) in [
            (settings::UiDensity::Default, 30.),
            (settings::UiDensity::Compact, 26.),
        ] {
            update_user_settings(cx, |settings| {
                settings.theme.ui_density = Some(density);
            });
            for selector in [
                PROJECT_WIDGET_DEBUG_SELECTOR,
                WORKTREE_AND_BRANCH_DEBUG_SELECTOR,
            ] {
                let widget = cx
                    .debug_bounds(selector)
                    .unwrap_or_else(|| panic!("{selector} is rendered in the title bar"));
                assert_eq!(
                    widget.size.height,
                    px(expected_height),
                    "{selector} is a {expected_height} px MainToolbar combo pill in {density:?} density"
                );
            }

            let project_widget = cx
                .debug_bounds(PROJECT_WIDGET_DEBUG_SELECTOR)
                .expect("the project widget is rendered in the title bar");
            cx.simulate_mouse_move(project_widget.center(), None, Modifiers::none());
            cx.run_until_parked();
            let (hover, scale_factor) = cx.update(|window, cx| {
                (
                    ToolbarWidgetColors::current(cx).hover,
                    window.scale_factor(),
                )
            });
            let hovered_pill = quads_inside(cx, project_widget)
                .into_iter()
                .find(|quad| quad.background == Background::from(hover))
                .expect("a hovered widget paints its pill");
            assert_eq!(
                hovered_pill.corner_radii.top_left,
                ScaledPixels(6. * scale_factor),
                "MainToolbar.Dropdown.arc 12 has no compact key, so the pill keeps a 6 px radius in {density:?} density"
            );
            cx.simulate_mouse_move(point(px(1.), px(300.)), None, Modifiers::none());
            cx.run_until_parked();
        }
    }

    #[gpui::test]
    async fn an_open_widget_popup_paints_the_hover_colour_and_no_pressed_fill(
        cx: &mut TestAppContext,
    ) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        let widget = cx
            .debug_bounds("TOOLBAR_WIDGET-Version Control")
            .expect("a project without a repository shows the Version Control widget");

        cx.simulate_click(widget.center(), Modifiers::none());
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("MENU_ITEM-Create Git Repository…")
                .is_some(),
            "clicking the widget opens its popup"
        );
        cx.simulate_mouse_move(point(px(1.), px(300.)), None, Modifiers::none());
        cx.run_until_parked();

        let colors = cx.update(|_, cx| ToolbarWidgetColors::current(cx));
        let fills = quads_inside(cx, widget)
            .into_iter()
            .map(|quad| quad.background)
            .collect::<Vec<_>>();
        assert!(
            fills.contains(&Background::from(colors.hover)),
            "while its popup is shown the widget keeps IntelliJ's hover colour"
        );
        assert!(
            !fills.contains(&Background::from(colors.open)),
            "ToolbarComboButtonUI has no pressed or open colour"
        );
    }

    #[gpui::test]
    async fn pressed_icon_buttons_keep_the_hover_colour_over_the_project_gradient(
        cx: &mut TestAppContext,
    ) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        let colors = cx.update(|_, cx| ToolbarWidgetColors::current(cx));

        for (show_project_gradient, expected_fill) in [(true, colors.hover), (false, colors.open)] {
            update_user_settings(cx, |settings| {
                settings
                    .title_bar
                    .get_or_insert_default()
                    .show_project_gradient = Some(show_project_gradient);
            });
            let search_button = cx
                .debug_bounds(SEARCH_BUTTON_DEBUG_SELECTOR)
                .expect("the search button is rendered in the title bar");
            cx.simulate_mouse_move(search_button.center(), None, Modifiers::none());
            cx.simulate_mouse_down(search_button.center(), MouseButton::Left, Modifiers::none());
            cx.run_until_parked();

            let fills = quads_inside(cx, search_button)
                .into_iter()
                .map(|quad| quad.background)
                .collect::<Vec<_>>();
            assert!(
                fills.contains(&Background::from(expected_fill)),
                "a pressed header button is filled with {expected_fill:?} when the gradient is {show_project_gradient}"
            );
            if show_project_gradient {
                assert!(
                    !fills.contains(&Background::from(colors.open)),
                    "over the project gradient IntelliJ paints a pushed header button in the hover colour"
                );
            }

            cx.simulate_mouse_up(search_button.center(), MouseButton::Left, Modifiers::none());
            cx.simulate_mouse_move(point(px(1.), px(300.)), None, Modifiers::none());
            cx.run_until_parked();
        }
    }

    #[gpui::test]
    async fn clicking_a_title_bar_action_does_not_arm_the_window_drag(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        let search_button = cx
            .debug_bounds(SEARCH_BUTTON_DEBUG_SELECTOR)
            .expect("the search button is rendered in the title bar");

        cx.simulate_mouse_move(search_button.center(), None, Modifiers::none());
        cx.simulate_mouse_down(search_button.center(), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(search_button.center(), MouseButton::Left, Modifiers::none());
        // The test window's `start_window_move` is unimplemented, so a drag left armed by the
        // press would panic on this move across the title bar.
        cx.simulate_mouse_move(
            search_button.center() - point(px(40.), px(0.)),
            None,
            Modifiers::none(),
        );
        cx.run_until_parked();
    }

    #[gpui::test]
    async fn detached_head_shows_the_short_hash_like_intellij(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar_with_repository(
            cx,
            Some(TestRepository {
                branch: None,
                upstream: None,
                head_sha: Some(DETACHED_HEAD_SHA),
            }),
        )
        .await;

        assert!(
            cx.debug_bounds("TOOLBAR_WIDGET-01234567").is_some(),
            "a detached HEAD shows its 8 character hash as the widget text"
        );
        assert_eq!(
            cx.debug_bounds("TOOLBAR_WIDGET-Create Branch"),
            None,
            "the widget does not turn into a Create Branch button"
        );
        assert_eq!(
            cx.debug_bounds(VERSION_CONTROL_SETUP_DEBUG_SELECTOR),
            None,
            "a repository in detached HEAD still has its VCS widget"
        );
    }

    #[gpui::test]
    async fn settings_button_opens_the_settings_entry_point_menu(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        let opened_settings = Rc::new(Cell::new(0));
        cx.update(|_, cx| {
            let opened_settings = opened_settings.clone();
            cx.on_action(move |_: &OpenSettings, _| {
                opened_settings.set(opened_settings.get() + 1);
            });
        });

        let settings_button = cx
            .debug_bounds(SETTINGS_BUTTON_DEBUG_SELECTOR)
            .expect("the Settings gear is rendered in the title bar");
        cx.simulate_click(settings_button.center(), Modifiers::none());
        cx.run_until_parked();

        assert_eq!(
            opened_settings.get(),
            0,
            "the gear opens its menu instead of the Settings window"
        );
        let items = [
            "MENU_ITEM-Settings…",
            "MENU_ITEM-Plugins…",
            "MENU_ITEM-Keymap…",
        ]
        .map(|selector| {
            cx.debug_bounds(selector)
                .unwrap_or_else(|| panic!("the Settings menu has {selector}"))
        });
        assert!(
            items[0].top() < items[1].top() && items[1].top() < items[2].top(),
            "the entries follow SettingsEntryPointGroup: {items:?}"
        );

        cx.simulate_click(items[0].center(), Modifiers::none());
        cx.run_until_parked();
        assert_eq!(
            opened_settings.get(),
            1,
            "Settings… opens the Settings window"
        );
    }

    #[gpui::test]
    async fn project_widget_opens_recent_projects_underneath_it(cx: &mut TestAppContext) {
        let (workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        cx.update(|_, cx| editor::init(cx));
        let project_widget = cx
            .debug_bounds(PROJECT_WIDGET_DEBUG_SELECTOR)
            .expect("the project widget is rendered in the title bar");
        assert_eq!(
            cx.debug_bounds(recent_projects::RECENT_PROJECTS_DEBUG_SELECTOR),
            None
        );

        cx.simulate_click(project_widget.center(), Modifiers::none());
        cx.run_until_parked();

        let popup = cx
            .debug_bounds(recent_projects::RECENT_PROJECTS_DEBUG_SELECTOR)
            .expect("clicking the project widget opens Recent Projects");
        assert!(
            popup.top() >= project_widget.bottom(),
            "the popup opens underneath the widget: {popup:?}, widget {project_widget:?}"
        );
        assert!(
            (popup.left() - project_widget.left()).abs() <= px(8.),
            "the popup is left-aligned with the widget: {popup:?}, widget {project_widget:?}"
        );
        workspace.read_with(cx, |workspace, cx| {
            assert!(
                workspace.active_modal::<RecentProjects>(cx).is_none(),
                "the widget does not open the centred Recent Projects modal"
            );
        });
    }

    // PopoverMenu focuses a menu two frames after it opens, and tests have no frame loop.
    fn deliver_popover_focus(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        for _ in 0..2 {
            cx.update(|window, cx| {
                window.simulate_next_frame(cx);
            });
            cx.run_until_parked();
        }
    }

    #[gpui::test]
    async fn recent_projects_popup_stays_open_while_its_actions_menu_is_open(
        cx: &mut TestAppContext,
    ) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        cx.update(|_, cx| editor::init(cx));
        let project_widget = cx
            .debug_bounds(PROJECT_WIDGET_DEBUG_SELECTOR)
            .expect("the project widget is rendered in the title bar");
        cx.simulate_click(project_widget.center(), Modifiers::none());
        deliver_popover_focus(cx);

        cx.dispatch_action(recent_projects::ToggleActionsMenu);
        deliver_popover_focus(cx);

        assert!(
            cx.debug_bounds("MENU_ITEM-Open Local Folders").is_some(),
            "the Actions menu of Recent Projects is open"
        );
        assert!(
            cx.debug_bounds(recent_projects::RECENT_PROJECTS_DEBUG_SELECTOR)
                .is_some(),
            "moving focus into the Actions menu keeps the popup open"
        );
    }

    #[gpui::test]
    async fn project_without_a_repository_shows_the_version_control_widget(
        cx: &mut TestAppContext,
    ) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        let project_widget = cx
            .debug_bounds(PROJECT_WIDGET_DEBUG_SELECTOR)
            .expect("the project widget is rendered in the title bar");
        let version_control = cx
            .debug_bounds(VERSION_CONTROL_SETUP_DEBUG_SELECTOR)
            .expect("a project without a repository keeps IntelliJ's VCS widget");
        assert!(
            cx.debug_bounds("TOOLBAR_WIDGET-Version Control").is_some(),
            "the widget reads Version Control"
        );
        assert!(
            project_widget.right() <= version_control.left(),
            "the VCS widget follows the project widget, like MainToolbarLeft"
        );

        cx.simulate_click(version_control.center(), Modifiers::none());
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("MENU_ITEM-Create Git Repository…")
                .is_some(),
            "its popup offers to create a Git repository"
        );
    }

    #[gpui::test]
    async fn a_repository_shows_the_branch_widget_instead_of_version_control(
        cx: &mut TestAppContext,
    ) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, Some("main")).await;

        assert!(
            cx.debug_bounds(WORKTREE_AND_BRANCH_DEBUG_SELECTOR)
                .is_some()
        );
        assert_eq!(cx.debug_bounds(VERSION_CONTROL_SETUP_DEBUG_SELECTOR), None);
    }

    #[gpui::test]
    async fn toolbar_widget_icons_keep_intellij_logical_sizes(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar_with_upstream(
            cx,
            Some("main"),
            Some(UpstreamTrackingStatus {
                ahead: 1,
                behind: 2,
            }),
        )
        .await;
        let rem_size = cx.update(|window, _| window.rem_size());
        assert_ne!(
            rem_size,
            px(16.),
            "the default ui_font_size would shrink rem-based icons"
        );

        for (icon_size, scale) in [(None, 1.), (Some(21), 1.5)] {
            update_user_settings(cx, |settings| {
                settings.title_bar.get_or_insert_default().icon_size = icon_size;
            });
            for (selector, side) in [
                (toolbar_widgets::TOOLBAR_WIDGET_ICON_DEBUG_SELECTOR, 16.),
                (toolbar_widgets::TOOLBAR_WIDGET_CHEVRON_DEBUG_SELECTOR, 16.),
                (toolbar_widgets::PROJECT_AVATAR_CHIP_DEBUG_SELECTOR, 16.),
                ("title_bar_incoming_commits", 12.),
                ("title_bar_outgoing_commits", 12.),
            ] {
                let bounds = cx
                    .debug_bounds(selector)
                    .unwrap_or_else(|| panic!("{selector} is rendered in the title bar"));
                assert_square(bounds, side * scale, selector);
            }
        }
    }

    #[test]
    fn project_widget_tooltip_is_the_project_folder_relative_to_home() {
        let project_name = SharedString::new_static("Flint");
        let project_path = util::paths::home_dir().join("IdeaProjects").join("Flint");

        assert_eq!(
            project_widget_tooltip(&project_name, Some(project_path.as_path())),
            "~/IdeaProjects/Flint"
        );
        assert_eq!(
            project_widget_tooltip(&project_name, Some(Path::new("/opt/flint"))),
            "/opt/flint"
        );
        assert_eq!(project_widget_tooltip(&project_name, None), "Flint");
    }

    #[test]
    fn branch_tooltip_lists_incoming_and_outgoing_commits_like_intellij() {
        assert_eq!(branch_widget_tooltip_title("main"), "Git Branch: main");
        assert_eq!(incoming_outgoing_tooltip(0, 0), None);
        assert_eq!(
            incoming_outgoing_tooltip(1, 0).as_deref(),
            Some("1 incoming commit")
        );
        assert_eq!(
            incoming_outgoing_tooltip(0, 3).as_deref(),
            Some("3 outgoing commits")
        );
        assert_eq!(
            incoming_outgoing_tooltip(2, 1).as_deref(),
            Some("2 incoming and 1 outgoing commits")
        );
    }

    #[test]
    fn project_avatar_initials_follow_intellij_camel_case_and_word_rules() {
        assert_eq!(avatar_initials("Flint"), "F");
        assert_eq!(avatar_initials("MyProject"), "MP");
        assert_eq!(avatar_initials("My-Project"), "MP");
        assert_eq!(avatar_initials("John Smith"), "JS");
        assert_eq!(avatar_initials("John-Smith-Harris"), "JH");
        assert_eq!(avatar_initials("VivaQuant.RS1"), "VQ");
        assert_eq!(avatar_initials("  vq.Automation  "), "VA");
        assert_eq!(avatar_initials("MyApp server"), "MA");

        // IntelliJ's `RecentProjectsTest.projectInitialsMakeSense` cases.
        for (name, expected) in [
            ("John Smith", "JS"),
            ("John Smith-Harris", "JS"),
            ("John-Smith-Harris", "JH"),
            ("John-Smith Harris", "JH"),
            ("MyProject", "MP"),
            ("My-Project", "MP"),
            ("My-Project_Strong", "MP"),
            ("My_Project_Strong", "MS"),
            ("One,Two-Four", "OT"),
            ("One.Two.Four", "OF"),
            ("Project_", "P"),
            ("_internal-project", "IP"),
            (".internal-project", "IP"),
            (".internalProject", "IP"),
            ("proj_[ver1]", "PV"),
            ("myProject-0", "MP"),
        ] {
            assert_eq!(avatar_initials(name), expected, "initials of {name:?}");
        }
    }

    #[test]
    fn test_foreign_path_style_does_not_use_repository_identity_as_name_anchor() {
        let (common_dir, foreign_path_style) = match PathStyle::local() {
            PathStyle::Unix => (Path::new(r"C:\repos\zed"), PathStyle::Windows),
            PathStyle::Windows => (Path::new("/repos/zed"), PathStyle::Unix),
        };
        let repository_identity_path = repo_identity_path_if_local(common_dir, foreign_path_style);

        assert_eq!(
            linked_worktree_name_anchor(None, repository_identity_path, true),
            None
        );
    }

    #[test]
    fn test_local_path_style_uses_bare_repository_worktree_name() {
        let (repository_identity_path, work_directory_path) = match PathStyle::local() {
            PathStyle::Unix => (
                Path::new("/repos/zed"),
                Path::new("/worktrees/zed/plum-warbler/zed"),
            ),
            PathStyle::Windows => (
                Path::new(r"C:\repos\zed"),
                Path::new(r"C:\worktrees\zed\plum-warbler\zed"),
            ),
        };

        let repository_identity_path =
            repo_identity_path_if_local(repository_identity_path, PathStyle::local());
        let name_anchor_path = linked_worktree_name_anchor(None, repository_identity_path, true);

        assert_eq!(
            name_anchor_path.and_then(|name_anchor_path| {
                linked_worktree_short_name(name_anchor_path, work_directory_path)
            }),
            Some("plum-warbler".into())
        );
    }
}
