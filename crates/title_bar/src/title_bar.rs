mod application_menu;
mod title_bar_settings;
mod toolbar_widgets;

use crate::application_menu::{ApplicationMenu, show_menus};
use agent_settings::{AgentSettings, WindowLayout};
use arrayvec::ArrayVec;
use git_ui_core::worktree_picker::WorktreePicker;
pub use platform_title_bar::{
    self, DraggedWindowTab, MergeAllWindows, MoveTabToNewWindow, PlatformTitleBar,
    ShowNextWindowTab, ShowPreviousWindowTab,
};
use project::{linked_worktree_short_name, repo_identity_path, repo_identity_path_if_local};

#[cfg(not(target_os = "macos"))]
use crate::application_menu::{
    ActivateDirection, ActivateMenuLeft, ActivateMenuRight, OpenApplicationMenu,
};

use command_palette_hooks::CommandPaletteFilter;

use gpui::{
    AnyElement, App, Context, Entity, Focusable, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, Styled, Subscription, WeakEntity, Window, actions, div,
};
use project::{
    Project, git_store::GitStoreEvent, project_settings::ProjectSettings,
    trusted_worktrees::TrustedWorktrees,
};
use remote::RemoteConnectionOptions;
use settings::{Settings as _, SettingsStore};

use std::any::TypeId;
use std::path::Path;
use theme::ActiveTheme;
use title_bar_settings::TitleBarSettings;
use toolbar_widgets::{
    badge_text_color, project_accent_index, project_initials, upstream_tracking_label,
};
use ui::{
    ButtonLike, IconButtonShape, IconWithIndicator, Indicator, PopoverMenu, TintColor, Tooltip,
    prelude::*, utils::platform_title_bar_height,
};
use util::ResultExt;
use workspace::{AccessibleMode, MultiWorkspace, ToggleWorktreeSecurity, Workspace};

use zed_actions::OpenRemote;

const MAX_PROJECT_NAME_LENGTH: usize = 40;
const MAX_BRANCH_NAME_LENGTH: usize = 40;
const MAX_SHORT_SHA_LENGTH: usize = 8;
const MAX_TASK_LABEL_LENGTH: usize = 30;
const PROJECT_BADGE_SIZE: f32 = 20.;
const PROJECT_BADGE_CORNER_RADIUS: f32 = 5.;

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
        /// Toggles the project menu dropdown.
        ToggleProjectMenu,
        /// Switches to a different git branch.
        SwitchBranch,
    ]
);

actions!(
    workspace,
    [
        /// Switches to the classic, editor-focused panel layout.
        UseClassicLayout,
        /// Switches to the agentic panel layout.
        UseAgenticLayout,
    ]
);

pub fn init(cx: &mut App) {
    platform_title_bar::PlatformTitleBar::init(cx);

    update_layout_action_filter(cx);

    cx.observe_global::<SettingsStore>(update_layout_action_filter)
        .detach();

    cx.observe_new(|workspace: &mut Workspace, window, cx| {
        let Some(window) = window else {
            return;
        };
        let multi_workspace = workspace.multi_workspace().cloned();
        let item = cx.new(|cx| TitleBar::new("title-bar", workspace, multi_workspace, window, cx));
        workspace.set_titlebar_item(item.into(), window, cx);

        workspace.register_action(|_workspace, _: &UseClassicLayout, _window, cx| {
            set_window_layout(WindowLayout::Editor(None), cx);
        });

        workspace.register_action(|_workspace, _: &UseAgenticLayout, _window, cx| {
            set_window_layout(WindowLayout::Agent(None), cx);
        });

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

/// Hides or shows the panel layout actions in the command palette based on
/// whether AI is currently disabled.
fn update_layout_action_filter(cx: &mut App) {
    let disable_ai = project::DisableAiSettings::get_global(cx).disable_ai;
    let layout_actions = [
        TypeId::of::<UseClassicLayout>(),
        TypeId::of::<UseAgenticLayout>(),
    ];
    CommandPaletteFilter::update_global(cx, |filter, _| {
        if disable_ai {
            filter.hide_action_types(&layout_actions);
        } else {
            filter.show_action_types(layout_actions.iter());
        }
    });
}

fn set_window_layout(layout: WindowLayout, cx: &App) {
    let fs = <dyn fs::Fs>::global(cx);
    drop(AgentSettings::set_layout(layout, fs, cx));
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

        let mut children = <ArrayVec<_, 5>>::new();

        let mut project_name = None;
        let mut repository = None;
        let mut linked_worktree_name = None;
        if let Some(worktree) = self.effective_active_worktree(cx) {
            repository = self.get_repository_for_worktree(&worktree, cx);
            let worktree_abs_path = worktree.read(cx).abs_path();
            project_name = worktree
                .read(cx)
                .root_name()
                .file_name()
                .map(|name| SharedString::from(name.to_string()));
            if let Some(repo) = &repository {
                let repo = repo.read(cx);
                let identity = repo_identity_path(&repo.common_dir_abs_path, repo.path_style);
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

                let display_name = if identity.extension() == Some(std::ffi::OsStr::new("git")) {
                    identity.file_stem().and_then(|n| n.to_str())
                } else {
                    repo.path_style.file_name(identity)
                };

                if let Some(repo_name) = display_name {
                    let visible_worktrees_in_repo = self.visible_worktrees_in_repository(repo, cx);
                    let name = if visible_worktrees_in_repo == 1 {
                        if let Ok(relative) =
                            worktree_abs_path.strip_prefix(&*repo.work_directory_abs_path)
                        {
                            if relative.as_os_str().is_empty() {
                                repo_name.to_string()
                            } else {
                                format!("{}/{}", repo_name, relative.display())
                            }
                        } else {
                            repo_name.to_string()
                        }
                    } else {
                        repo_name.to_string()
                    };
                    project_name = Some(SharedString::from(name));
                }
            }
        }

        children.push(
            h_flex()
                .h_full()
                .gap_0p5()
                .map(|title_bar| {
                    let mut render_project_items = title_bar_settings.show_branch_name
                        || title_bar_settings.show_project_items;
                    title_bar
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
                                .when(title_bar_settings.show_project_items, |title_bar| {
                                    title_bar
                                        .children(self.render_project_host(cx))
                                        .child(self.render_project_name(project_name, window, cx))
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
                                .when(title_bar_settings.show_run_widget, |title_bar| {
                                    title_bar.child(self.render_run_widget(cx))
                                })
                        })
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .into_any_element(),
        );

        children.extend(Self::render_right_buttons(&title_bar_settings));

        if show_menus {
            self.platform_titlebar.update(cx, |this, _| {
                this.set_button_layout(button_layout);
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
                        .h(height)
                        .pl_2()
                        .justify_between()
                        .w_full()
                        .children(children),
                )
                .into_any_element()
        } else {
            self.platform_titlebar.update(cx, |this, _| {
                this.set_button_layout(button_layout);
                this.set_children(children);
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

    fn visible_worktrees_in_repository(
        &self,
        repository: &project::git_store::Repository,
        cx: &App,
    ) -> usize {
        let repo_path = &repository.work_directory_abs_path;
        self.project
            .read(cx)
            .visible_worktrees(cx)
            .filter(|worktree| {
                let worktree_path = worktree.read(cx).abs_path();
                worktree_path == *repo_path || worktree_path.starts_with(repo_path.as_ref())
            })
            .count()
    }

    fn render_remote_project_connection(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let workspace = self.workspace.clone();

        let options = self.project.read(cx).remote_connection_options(cx)?;
        let host: SharedString = options.display_name().into();

        let (nickname, tooltip_title, icon) = match options {
            RemoteConnectionOptions::Ssh(options) => (
                options.nickname.map(|nick| nick.into()),
                "Remote Project",
                IconName::Server,
            ),
            RemoteConnectionOptions::Wsl(_) => (None, "Remote Project", IconName::Linux),
            RemoteConnectionOptions::Docker(_dev_container_connection) => {
                (None, "Dev Container", IconName::Box)
            }
            #[cfg(any(test, feature = "test-support"))]
            RemoteConnectionOptions::Mock(_) => (None, "Mock Remote Project", IconName::Server),
        };

        let nickname = nickname.unwrap_or_else(|| host.clone());

        let (indicator_color, meta) = match self.project.read(cx).remote_connection_state(cx)? {
            remote::ConnectionState::Connecting => (Color::Info, format!("Connecting to: {host}")),
            remote::ConnectionState::Connected => (Color::Success, format!("Connected to: {host}")),
            remote::ConnectionState::HeartbeatMissed => (
                Color::Warning,
                format!("Connection attempt to {host} missed. Retrying..."),
            ),
            remote::ConnectionState::Reconnecting => (
                Color::Warning,
                format!("Lost connection to {host}. Reconnecting..."),
            ),
            remote::ConnectionState::Disconnected => {
                (Color::Error, format!("Disconnected from {host}"))
            }
        };

        let icon_color = match self.project.read(cx).remote_connection_state(cx)? {
            remote::ConnectionState::Connecting => Color::Info,
            remote::ConnectionState::Connected => Color::Default,
            remote::ConnectionState::HeartbeatMissed => Color::Warning,
            remote::ConnectionState::Reconnecting => Color::Warning,
            remote::ConnectionState::Disconnected => Color::Error,
        };

        let meta = SharedString::from(meta);
        let region = ui::ChromeRegion::TitleBar;
        let icon_size = ui::chrome_icon_size(region, IconSize::Small, cx);

        Some(
            PopoverMenu::new("remote-project-menu")
                .menu(move |window, cx| {
                    let workspace_entity = workspace.upgrade()?;
                    let fs = workspace_entity.read(cx).project().read(cx).fs().clone();
                    Some(recent_projects::RemoteServerProjects::popover(
                        fs,
                        workspace.clone(),
                        None,
                        window,
                        cx,
                    ))
                })
                .trigger_with_tooltip(
                    ButtonLike::new("remote_project")
                        .selected_style(ButtonStyle::Tinted(TintColor::Accent))
                        .chrome_region(region)
                        .child(
                            h_flex()
                                .gap_2()
                                .max_w_32()
                                .child(
                                    IconWithIndicator::new(
                                        Icon::new(icon).size(icon_size).color(icon_color),
                                        Some(Indicator::dot().color(indicator_color)),
                                    )
                                    .indicator_border_color(Some(
                                        cx.theme().colors().title_bar_background,
                                    ))
                                    .into_any_element(),
                                )
                                .child(Label::new(nickname).size(LabelSize::Small).truncate()),
                        ),
                    move |_window, cx| {
                        Tooltip::with_meta(
                            tooltip_title,
                            Some(&OpenRemote::default()),
                            meta.clone(),
                            cx,
                        )
                    },
                )
                .anchor(gpui::Anchor::TopLeft)
                .into_any_element(),
        )
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

    pub fn render_project_host(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.project.read(cx).is_via_remote_server() {
            return self.render_remote_project_connection(cx);
        }

        None
    }

    fn render_project_name(
        &self,
        name: Option<SharedString>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let workspace = self.workspace.clone();

        let display_name = if let Some(name) = &name {
            util::truncate_and_trailoff(name, MAX_PROJECT_NAME_LENGTH)
        } else {
            "Open Recent Project".to_string()
        };

        let is_sidebar_open = self
            .multi_workspace
            .as_ref()
            .and_then(|mw| mw.upgrade())
            .map(|mw| mw.read(cx).sidebar_open())
            .unwrap_or(false)
            && PlatformTitleBar::is_multi_workspace_enabled(cx);

        let is_threads_list_view_active = self
            .multi_workspace
            .as_ref()
            .and_then(|mw| mw.upgrade())
            .map(|mw| mw.read(cx).is_threads_list_view_active(cx))
            .unwrap_or(false);

        if is_sidebar_open && is_threads_list_view_active {
            return self
                .render_recent_projects_popover(name.as_ref(), display_name, cx)
                .into_any_element();
        }

        let focus_handle = workspace
            .upgrade()
            .map(|w| w.read(cx).focus_handle(cx))
            .unwrap_or_else(|| cx.focus_handle());

        let window_project_groups: Vec<_> = self
            .multi_workspace
            .as_ref()
            .and_then(|mw| mw.upgrade())
            .map(|mw| mw.read(cx).project_group_keys())
            .unwrap_or_default();

        PopoverMenu::new("recent-projects-menu")
            .menu(move |window, cx| {
                Some(recent_projects::RecentProjects::popover(
                    workspace.clone(),
                    window_project_groups.clone(),
                    None,
                    focus_handle.clone(),
                    window,
                    cx,
                ))
            })
            .trigger_with_tooltip(
                Self::render_project_trigger(name.as_ref(), display_name, cx),
                move |_window, cx| {
                    Tooltip::for_action("Recent Projects", &zed_actions::OpenRecent::default(), cx)
                },
            )
            .anchor(gpui::Anchor::TopLeft)
            .into_any_element()
    }

    fn render_recent_projects_popover(
        &self,
        name: Option<&SharedString>,
        display_name: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let workspace = self.workspace.clone();

        let focus_handle = workspace
            .upgrade()
            .map(|w| w.read(cx).focus_handle(cx))
            .unwrap_or_else(|| cx.focus_handle());

        let window_project_groups: Vec<_> = self
            .multi_workspace
            .as_ref()
            .and_then(|mw| mw.upgrade())
            .map(|mw| mw.read(cx).project_group_keys())
            .unwrap_or_default();

        PopoverMenu::new("sidebar-title-recent-projects-menu")
            .menu(move |window, cx| {
                Some(recent_projects::RecentProjects::popover(
                    workspace.clone(),
                    window_project_groups.clone(),
                    None,
                    focus_handle.clone(),
                    window,
                    cx,
                ))
            })
            .trigger_with_tooltip(
                Self::render_project_trigger(name, display_name, cx),
                move |_window, cx| {
                    Tooltip::for_action("Recent Projects", &zed_actions::OpenRecent::default(), cx)
                },
            )
            .anchor(gpui::Anchor::TopLeft)
    }

    fn render_project_trigger(
        name: Option<&SharedString>,
        display_name: String,
        cx: &App,
    ) -> ButtonLike {
        let region = ui::ChromeRegion::TitleBar;
        let is_project_selected = name.is_some();
        let badge = name
            .filter(|_| TitleBarSettings::get_global(cx).show_project_badge)
            .and_then(|name| Self::render_project_badge(name, cx));
        let chevron_size = ui::chrome_icon_size(region, IconSize::XSmall, cx);

        ButtonLike::new("project_name_trigger")
            .selected_style(ButtonStyle::Tinted(TintColor::Accent))
            .chrome_region(region)
            .tab_index(0isize)
            .aria_label(display_name.clone())
            .child(
                h_flex()
                    .gap_1()
                    .children(badge)
                    .child(
                        Label::new(display_name)
                            .size(LabelSize::Small)
                            .when(!is_project_selected, |label| label.color(Color::Muted)),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .size(chevron_size)
                            .color(Color::Muted),
                    ),
            )
    }

    fn render_project_badge(project_name: &str, cx: &App) -> Option<AnyElement> {
        let initials = project_initials(project_name)?;
        let accents = &cx.theme().accents().0;
        let background = project_accent_index(project_name, accents.len())
            .and_then(|index| accents.get(index).copied())
            .unwrap_or(cx.theme().colors().text_accent);

        Some(
            h_flex()
                .flex_none()
                .justify_center()
                .size(px(PROJECT_BADGE_SIZE))
                .rounded(px(PROJECT_BADGE_CORNER_RADIUS))
                .bg(background)
                .child(
                    Label::new(initials)
                        .size(LabelSize::XSmall)
                        .weight(FontWeight::SEMIBOLD)
                        .color(Color::Custom(badge_text_color(background))),
                )
                .into_any_element(),
        )
    }

    fn last_scheduled_task_label(&self, cx: &App) -> Option<SharedString> {
        let (_, task) = self
            .project
            .read(cx)
            .task_store()
            .read(cx)
            .task_inventory()?
            .read(cx)
            .last_scheduled_task(None)?;
        Some(util::truncate_and_trailoff(task.display_label(), MAX_TASK_LABEL_LENGTH).into())
    }

    fn render_run_widget(&self, cx: &App) -> impl IntoElement {
        let region = ui::ChromeRegion::TitleBar;
        let task_label = self
            .last_scheduled_task_label(cx)
            .unwrap_or_else(|| "Run…".into());

        h_flex()
            .h_full()
            .ml_2()
            .gap_0p5()
            .child(
                Button::new("run_widget_task_picker", task_label)
                    .label_size(LabelSize::Small)
                    .chrome_region(region)
                    .tab_index(0isize)
                    .end_icon(
                        Icon::new(IconName::ChevronDown)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    )
                    .tooltip(Tooltip::for_action_title(
                        "Run Task…",
                        &zed_actions::Spawn::modal(),
                    ))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(zed_actions::Spawn::modal()), cx)
                    }),
            )
            .child(
                IconButton::new("run_widget_rerun", IconName::PlayFilled)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Small)
                    .icon_color(Color::Created)
                    .chrome_region(region)
                    .tab_index(0isize)
                    .aria_label("Rerun Last Task")
                    .tooltip(Tooltip::for_action_title(
                        "Rerun Last Task",
                        &zed_actions::Rerun::default(),
                    ))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(zed_actions::Rerun::default()), cx)
                    }),
            )
            .child(
                IconButton::new("run_widget_debug", IconName::Debug)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Small)
                    .icon_color(Color::Created)
                    .chrome_region(region)
                    .tab_index(0isize)
                    .aria_label("Start Debugging")
                    .tooltip(Tooltip::for_action_title(
                        "Start Debugging",
                        &debugger_ui::Start,
                    ))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(debugger_ui::Start), cx)
                    }),
            )
    }

    fn render_right_buttons(settings: &TitleBarSettings) -> Option<AnyElement> {
        if !settings.show_search_button && !settings.show_settings_button {
            return None;
        }
        let region = ui::ChromeRegion::TitleBar;

        Some(
            h_flex()
                .h_full()
                .pr_1()
                .gap_0p5()
                .when(settings.show_search_button, |this| {
                    this.child(
                        IconButton::new("title_bar_search_everywhere", IconName::MagnifyingGlass)
                            .shape(IconButtonShape::Square)
                            .icon_size(IconSize::Small)
                            .chrome_region(region)
                            .tab_index(0isize)
                            .aria_label("Search Everywhere")
                            .tooltip(Tooltip::for_action_title(
                                "Search Everywhere",
                                &zed_actions::search_everywhere::Toggle { tab: None },
                            ))
                            .on_click(|_, window, cx| {
                                window.dispatch_action(
                                    Box::new(zed_actions::search_everywhere::Toggle { tab: None }),
                                    cx,
                                )
                            }),
                    )
                })
                .when(settings.show_settings_button, |this| {
                    this.child(
                        IconButton::new("title_bar_settings", IconName::Settings)
                            .shape(IconButtonShape::Square)
                            .icon_size(IconSize::Small)
                            .chrome_region(region)
                            .tab_index(0isize)
                            .aria_label("Settings")
                            .tooltip(Tooltip::for_action_title(
                                "Settings",
                                &zed_actions::OpenSettings,
                            ))
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(zed_actions::OpenSettings), cx)
                            }),
                    )
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .into_any_element(),
        )
    }

    fn render_worktree_and_branch(
        &self,
        repository: Entity<project::git_store::Repository>,
        linked_worktree_name: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let workspace = self.workspace.upgrade()?;

        let (branch_name, upstream_label, icon_info, is_detached_head) = {
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

            let upstream_label = repo
                .branch
                .as_ref()
                .and_then(|branch| branch.tracking_status())
                .and_then(|status| upstream_tracking_label(status.ahead, status.behind));

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

            (branch_name, upstream_label, icon_info, is_detached_head)
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
                .trigger_with_tooltip(
                    Button::new("worktree_picker_trigger", display_label)
                        .selected_style(ButtonStyle::Tinted(TintColor::Accent))
                        .label_size(LabelSize::Small)
                        .chrome_region(ui::ChromeRegion::TitleBar)
                        .color(Color::Muted)
                        .tab_index(0isize)
                        .loading(is_creating)
                        .start_icon(
                            Icon::new(IconName::GitWorktree)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        ),
                    move |_window, cx| {
                        Tooltip::with_meta(
                            "Worktree",
                            Some(&zed_actions::git::Worktree),
                            format!("Currently In Use: {}", worktree_label),
                            cx,
                        )
                    },
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

                let chevron = Icon::new(IconName::ChevronDown)
                    .size(IconSize::XSmall)
                    .color(Color::Muted);

                let trigger = if is_detached_head {
                    Button::new("project_branch_trigger", "Create Branch")
                        .selected_style(ButtonStyle::Tinted(TintColor::Accent))
                        .label_size(LabelSize::Small)
                        .chrome_region(ui::ChromeRegion::TitleBar)
                        .tab_index(0isize)
                        .start_icon(
                            Icon::new(IconName::GitBranchPlus)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .end_icon(chevron)
                } else {
                    let branch_label: SharedString = match &upstream_label {
                        Some(upstream) => format!("{branch_name}  {upstream}").into(),
                        None => branch_name.into(),
                    };
                    Button::new("project_branch_trigger", branch_label)
                        .selected_style(ButtonStyle::Tinted(TintColor::Accent))
                        .label_size(LabelSize::Small)
                        .chrome_region(ui::ChromeRegion::TitleBar)
                        .color(Color::Muted)
                        .tab_index(0isize)
                        .start_icon(
                            Icon::new(branch_icon)
                                .size(IconSize::XSmall)
                                .color(branch_icon_color),
                        )
                        .end_icon(chevron)
                };

                PopoverMenu::new("branch-menu")
                    .menu(move |window, cx| {
                        git_ui_core::build_branch_picker(
                            workspace.downgrade(),
                            effective_repository.clone(),
                            window,
                            cx,
                        )
                    })
                    .trigger_with_tooltip(trigger, move |_window, cx| {
                        let meta = if is_detached_head {
                            format!("Detached HEAD: {}", branch_tooltip_label)
                        } else {
                            format!("Currently Checked Out: {}", branch_tooltip_label)
                        };
                        Tooltip::with_meta(
                            "Branch & Stash",
                            Some(&zed_actions::git::Branch),
                            meta,
                            cx,
                        )
                    })
                    .anchor(gpui::Anchor::TopLeft)
            })
        });

        if worktree_button.is_none() && branch_picker.is_none() {
            return None;
        }

        let show_separator = worktree_button.is_some() && branch_picker.is_some();

        Some(
            h_flex()
                .gap_px()
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
    use gpui::{Modifiers, TestAppContext};
    use std::{cell::Cell, rc::Rc};
    use util::paths::PathStyle;
    use workspace::AppState;

    #[gpui::test]
    async fn test_search_and_settings_buttons_dispatch_their_actions(cx: &mut TestAppContext) {
        let app_state = cx.update(|cx| {
            let app_state = AppState::test(cx);
            PlatformTitleBar::init(cx);
            app_state
        });
        let project = Project::test(app_state.fs.clone(), [], cx).await;
        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project, window, cx));
        let workspace =
            multi_workspace.read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone());

        let search_dispatches = Rc::new(Cell::new(0));
        let settings_dispatches = Rc::new(Cell::new(0));
        workspace.update_in(cx, |workspace, window, cx| {
            let title_bar = cx.new(|cx| TitleBar::new("title-bar", workspace, None, window, cx));
            workspace.set_titlebar_item(title_bar.into(), window, cx);

            let search_dispatches = search_dispatches.clone();
            workspace.register_action(
                move |_, action: &zed_actions::search_everywhere::Toggle, _, _| {
                    assert_eq!(action.tab, None, "the title bar opens the default tab");
                    search_dispatches.set(search_dispatches.get() + 1);
                },
            );
            let settings_dispatches = settings_dispatches.clone();
            workspace.register_action(move |_, _: &zed_actions::OpenSettings, _, _| {
                settings_dispatches.set(settings_dispatches.get() + 1);
            });
        });
        cx.run_until_parked();

        let search_button = cx
            .debug_bounds("ICON-MagnifyingGlass")
            .expect("search button should be rendered at the right edge of the title bar");
        cx.simulate_click(search_button.center(), Modifiers::none());
        cx.run_until_parked();
        assert_eq!(search_dispatches.get(), 1);
        assert_eq!(settings_dispatches.get(), 0);

        let settings_button = cx
            .debug_bounds("ICON-Settings")
            .expect("settings button should be rendered at the right edge of the title bar");
        cx.simulate_click(settings_button.center(), Modifiers::none());
        cx.run_until_parked();
        assert_eq!(search_dispatches.get(), 1);
        assert_eq!(settings_dispatches.get(), 1);
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
