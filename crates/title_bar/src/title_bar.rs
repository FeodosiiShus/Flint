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
    ParentElement, Render, Styled, Subscription, WeakEntity, Window, actions, div, px, rgb,
};
use project::{
    Project, git_store::GitStoreEvent, project_settings::ProjectSettings,
    trusted_worktrees::TrustedWorktrees,
};
use settings::Settings as _;

use std::path::Path;
use theme::UiDensity;
use title_bar_settings::TitleBarSettings;
use toolbar_widgets::{ToolbarWidget, ToolbarWidgetColors, avatar_initials, upstream_tracking};
use ui::{
    IconButton, PopoverMenu, TintColor, Tooltip, prelude::*, utils::platform_title_bar_height,
};
use util::ResultExt;
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
        let mut repository = None;
        let mut linked_worktree_name = None;
        if let Some(worktree) = self.effective_active_worktree(cx) {
            repository = self.get_repository_for_worktree(&worktree, cx);
            project_name = worktree
                .read(cx)
                .root_name()
                .file_name()
                .map(|name| SharedString::from(name.to_string()));
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

        let project_gradient = (project_name.is_some() && title_bar_settings.show_project_gradient)
            .then(Self::project_gradient_color);

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
                        window,
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
                                title_bar.child(Self::render_project_widget(project_name))
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
                    })
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .into_any_element();

        let title_bar_actions =
            Self::render_title_bar_actions(title_bar_settings.show_search_button, window, cx);

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

    fn title_bar_button_metrics(window: &Window, cx: &App) -> (IconSize, Pixels, Pixels) {
        let radius = match theme::theme_settings(cx).ui_density(cx) {
            UiDensity::Compact => TITLE_BAR_BUTTON_RADIUS_COMPACT,
            UiDensity::Default | UiDensity::Comfortable => TITLE_BAR_BUTTON_RADIUS,
        };
        (
            ui::exact_icon_size(ui::TOOLBAR_ICON_SIDE, window.rem_size()),
            TITLE_BAR_BUTTON_SIDE,
            radius,
        )
    }

    fn render_navigation_buttons(
        can_go_back: bool,
        can_go_forward: bool,
        workspace: WeakEntity<Workspace>,
        window: &Window,
        cx: &App,
    ) -> impl IntoElement {
        let (icon_size, side, radius) = Self::title_bar_button_metrics(window, cx);
        let backward = IconButton::new("title_bar_navigate_backward", IconName::ArrowLeft)
            .shape(ui::IconButtonShape::Square)
            .style(ButtonStyle::Subtle)
            .chrome_region(ui::ChromeRegion::TitleBar)
            .icon_size(icon_size)
            .box_size(side)
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
            .box_size(side)
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

    fn render_project_widget(project_name: SharedString) -> impl IntoElement {
        let initials: SharedString = avatar_initials(&project_name).into();
        let color = Self::project_gradient_color();
        let tooltip_label = project_name.clone();
        let open_recent = zed_actions::OpenRecent {
            create_new_window: None,
        };
        let tooltip_action = open_recent.clone();

        h_flex()
            .debug_selector(|| PROJECT_WIDGET_DEBUG_SELECTOR.into())
            .child(
                ToolbarWidget::avatar("project_widget_trigger", initials, color)
                    .label(project_name)
                    .tab_index(0)
                    .tooltip(move |_window, cx| {
                        Tooltip::with_meta(
                            "Recent Projects",
                            Some(&tooltip_action),
                            format!("Currently Open: {tooltip_label}"),
                            cx,
                        )
                    })
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(open_recent.clone()), cx)
                    }),
            )
    }

    fn render_title_bar_actions(
        show_search_button: bool,
        window: &Window,
        cx: &App,
    ) -> impl IntoElement {
        let (icon_size, side, radius) = Self::title_bar_button_metrics(window, cx);
        let colors = ToolbarWidgetColors::current(cx);
        let search_action = zed_actions::search_everywhere::Toggle { tab: None };
        let action_button = move |id: &'static str, icon: IconName| {
            IconButton::new(id, icon)
                .shape(ui::IconButtonShape::Square)
                .style(ButtonStyle::Subtle)
                .chrome_region(ui::ChromeRegion::TitleBar)
                .icon_size(icon_size)
                .box_size(side)
                .corner_radius(radius)
                .hover_background(colors.hover)
                .active_background(colors.open)
        };

        h_flex()
            .debug_selector(|| TITLE_BAR_ACTIONS_DEBUG_SELECTOR.into())
            .items_center()
            .gap(TITLE_BAR_ACTION_GAP)
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
                div()
                    .debug_selector(|| SETTINGS_BUTTON_DEBUG_SELECTOR.into())
                    .child(
                        action_button("title_bar_settings_button", IconName::Settings)
                            .aria_label("Settings")
                            .tooltip(Tooltip::for_action_title("Settings", &OpenSettings))
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(OpenSettings), cx)
                            }),
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

        let (branch_name, upstream, icon_info, is_detached_head) = {
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

            let upstream = repo
                .branch
                .as_ref()
                .and_then(|branch| branch.tracking_status())
                .and_then(|status| upstream_tracking(status.ahead, status.behind));

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
                (IconName::GitBranch, Color::Muted)
            };

            (branch_name, upstream, icon_info, is_detached_head)
        };

        let settings = TitleBarSettings::get_global(cx);
        let effective_repository = Some(repository);

        let worktree_label: SharedString = linked_worktree_name.unwrap_or_else(|| "main".into());

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
                        Icon::new(IconName::GitWorktree).color(Color::Muted),
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
                    (IconName::GitBranch, Color::Muted)
                };

                let trigger = if is_detached_head {
                    ToolbarWidget::new(
                        "project_branch_trigger",
                        Icon::new(IconName::GitBranchPlus).color(Color::Muted),
                        "Create Branch",
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
                    let meta = if is_detached_head {
                        format!("Detached HEAD: {}", branch_tooltip_label)
                    } else {
                        format!("Currently Checked Out: {}", branch_tooltip_label)
                    };
                    Tooltip::with_meta("Git Branches", Some(&zed_actions::git::Branch), meta, cx)
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

#[cfg(test)]
mod tests {
    use super::*;
    use git::repository::{UpstreamTracking, UpstreamTrackingStatus};
    use gpui::{
        Background, Modifiers, Quad, TestAppContext, UpdateGlobal as _, VisualTestContext, point,
    };
    use settings::{SettingsContent, SettingsStore};
    use std::{cell::RefCell, rc::Rc};
    use util::paths::PathStyle;
    use workspace::AppState;

    const PROJECT_ROOT: &str = "/flint";

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
        if let Some(branch) = checked_out_branch {
            let dot_git = Path::new(PROJECT_ROOT).join(".git");
            app_state
                .fs
                .create_dir(&dot_git)
                .await
                .expect("git directory should be created");
            let fake_fs = app_state.fs.as_fake();
            fake_fs.set_branch_name(&dot_git, Some(branch));
            if let Some(tracking) = upstream {
                fake_fs.set_upstream_for_repo(
                    &dot_git,
                    branch,
                    "origin/main",
                    UpstreamTracking::Tracked(tracking),
                );
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

        assert!(cx.update(|window, _| window.is_window_active()));
        let active_quads = cx.update(|window, _| window.painted_quads());
        assert!(
            active_quads.iter().any(|quad| quad.background == gradient),
            "an active window paints the gradient at full strength"
        );
        assert!(
            avatar_chip_fills(active_quads.as_slice()).contains(&Background::from(project_color)),
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
            inactive_avatar_fills
                .contains(&Background::from(project_color.opacity(inactive_opacity))),
            "an inactive window dims the project avatar chip"
        );
        assert!(
            !inactive_avatar_fills.contains(&Background::from(project_color)),
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
            branch_picker.left() < traffic_light_padding,
            "in full screen the branch picker moves into the space of the hidden macOS window buttons, got {branch_picker:?}"
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

    #[test]
    fn project_avatar_initials_follow_intellij_camel_case_and_word_rules() {
        assert_eq!(avatar_initials("Flint"), "F");
        assert_eq!(avatar_initials("MyProject"), "MP");
        assert_eq!(avatar_initials("My-Project"), "MP");
        assert_eq!(avatar_initials("John Smith"), "JS");
        assert_eq!(avatar_initials("John-Smith-Harris"), "JH");
        assert_eq!(avatar_initials("VivaQuant.RS1"), "VR");
        assert_eq!(avatar_initials("  vq.Automation  "), "VA");
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
