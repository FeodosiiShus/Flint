mod application_menu;
mod title_bar_settings;
mod toolbar_widgets;

use crate::application_menu::{ApplicationMenu, show_menus};
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

use gpui::{
    AnyElement, App, Context, Entity, Hsla, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Render, Styled, Subscription, WeakEntity, Window, actions, div,
};
use project::{
    Project, git_store::GitStoreEvent, project_settings::ProjectSettings,
    trusted_worktrees::TrustedWorktrees,
};
use remote::RemoteConnectionOptions;
use settings::Settings as _;

use std::path::Path;
use theme::ActiveTheme;
use title_bar_settings::TitleBarSettings;
use toolbar_widgets::{project_color, upstream_tracking_label};
use ui::{
    ButtonLike, IconWithIndicator, Indicator, KeyBinding, PopoverMenu, TintColor, Tooltip,
    prelude::*, utils::platform_title_bar_height,
};
use util::ResultExt;
use workspace::{
    AccessibleMode, MultiWorkspace, ToggleWorktreeSecurity, Workspace, WorkspaceSettings,
};

use zed_actions::OpenRemote;

const MAX_BRANCH_NAME_LENGTH: usize = 40;
const MAX_SHORT_SHA_LENGTH: usize = 8;
const SEARCH_FIELD_WIDTH: f32 = 240.;
const SEARCH_FIELD_DEBUG_SELECTOR: &str = "title_bar_search_field";
const WORKTREE_AND_BRANCH_DEBUG_SELECTOR: &str = "title_bar_worktree_and_branch";

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

        let project_gradient = project_name
            .as_ref()
            .filter(|_| title_bar_settings.show_project_gradient)
            .map(|name| Self::themed_project_color(name, cx));

        let project_items = h_flex()
            .h_full()
            .gap_0p5()
            .map(|title_bar| {
                let mut render_project_items = true;
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
                            .children(self.render_project_host(cx))
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
                            .when(title_bar_settings.show_search_button, |title_bar| {
                                title_bar.child(Self::render_search_field(cx))
                            })
                    })
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .into_any_element();

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
                                .child(project_items),
                        ),
                )
                .into_any_element()
        } else {
            self.platform_titlebar.update(cx, |this, _| {
                this.set_button_layout(button_layout);
                this.set_project_gradient(project_gradient);
                this.set_children([project_items]);
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

    fn themed_project_color(project_name: &str, cx: &App) -> Hsla {
        let theme = cx.theme();
        project_color(project_name, &theme.accents().0, theme.colors().text_accent)
    }

    fn render_search_field(cx: &App) -> impl IntoElement {
        let region = ui::ChromeRegion::TitleBar;
        let action = zed_actions::search_everywhere::Toggle { tab: None };

        h_flex()
            .debug_selector(|| SEARCH_FIELD_DEBUG_SELECTOR.into())
            .child(
                ButtonLike::new("title_bar_search_field")
                    .style(ButtonStyle::Outlined)
                    .chrome_region(region)
                    .width(px(SEARCH_FIELD_WIDTH))
                    .tab_index(0isize)
                    .aria_label("Search Everywhere")
                    .tooltip(Tooltip::for_action_title("Search Everywhere", &action))
                    .child(
                        h_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                Icon::new(IconName::MagnifyingGlass)
                                    .size(ui::chrome_icon_size(region, IconSize::Small, cx))
                                    .color(Color::Muted),
                            )
                            .child(
                                Label::new("Search Everywhere")
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            )
                            .child(
                                h_flex()
                                    .flex_1()
                                    .justify_end()
                                    .child(KeyBinding::for_action(&action, cx)),
                            ),
                    )
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(action.clone()), cx)
                    }),
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
                            "Git Branches",
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
                .debug_selector(|| WORKTREE_AND_BRANCH_DEBUG_SELECTOR.into())
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
    use crate::toolbar_widgets::project_accent_index;
    use gpui::{
        Background, Modifiers, Quad, TestAppContext, UpdateGlobal as _, VisualTestContext, point,
    };
    use settings::{SettingsContent, SettingsStore};
    use std::{cell::RefCell, rc::Rc};
    use util::paths::PathStyle;
    use workspace::AppState;

    const PROJECT_ROOT: &str = "/flint";
    const PROJECT_NAME: &str = "flint";

    async fn open_project_title_bar<'a>(
        cx: &'a mut TestAppContext,
        checked_out_branch: Option<&'static str>,
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
            app_state
                .fs
                .as_fake()
                .set_branch_name(&dot_git, Some(branch));
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
        let search_field = cx
            .debug_bounds(SEARCH_FIELD_DEBUG_SELECTOR)
            .expect("the search field should be rendered in the title bar");
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
            gradient.right() >= search_field.right(),
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
    async fn project_gradient_is_painted_with_the_accent_picked_for_the_project_name(
        cx: &mut TestAppContext,
    ) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;

        let project_accent = cx.update(|_, cx| {
            let accents = &cx.theme().accents().0;
            let accent_index = project_accent_index(PROJECT_NAME, accents.len())
                .expect("the test theme should define accent colors");
            accents[accent_index]
        });
        let painted_quads = cx.update(|window, _| window.painted_quads());

        assert!(
            painted_quads
                .iter()
                .any(|quad| quad.background == ui::project_gradient_background(project_accent)),
            "the title bar gradient is painted with the theme accent picked for the project name"
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
        let (project_color, inactive_opacity) = cx.update(|_, cx| {
            (
                TitleBar::themed_project_color(PROJECT_NAME, cx),
                <WorkspaceSettings as settings::Settings>::get_global(cx)
                    .islands
                    .frame_content_opacity(false),
            )
        });
        let search_field_border = cx.update(|_, cx| cx.theme().colors().border_variant);
        let scale_factor = cx.update(|window, _| window.scale_factor());
        let search_field = cx
            .debug_bounds(SEARCH_FIELD_DEBUG_SELECTOR)
            .expect("the search field should be rendered in the title bar")
            .scale(scale_factor);
        let search_field_border_colors = |quads: &[Quad]| {
            quads
                .iter()
                .filter(|quad| {
                    quad.border_widths.any(|width| width.0 > 0.)
                        && search_field.contains(&quad.bounds.center())
                })
                .map(|quad| quad.border_color)
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
            search_field_border_colors(active_quads.as_slice()).contains(&search_field_border),
            "an active window paints the search field border at full strength"
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
        let inactive_search_field_borders = search_field_border_colors(inactive_quads.as_slice());
        assert!(
            inactive_search_field_borders.contains(&search_field_border.opacity(inactive_opacity)),
            "an inactive window dims the search field border"
        );
        assert!(
            !inactive_search_field_borders.contains(&search_field_border),
            "no undimmed search field border remains in an inactive window"
        );
        assert!(
            !inactive_quads.iter().any(|quad| {
                quad.background == Background::from(title_bar_color).opacity(inactive_opacity)
            }),
            "the title bar background is never dimmed"
        );
    }

    #[gpui::test]
    async fn search_field_sits_right_after_the_branch_picker(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, Some("main")).await;

        let branch_picker = cx
            .debug_bounds(WORKTREE_AND_BRANCH_DEBUG_SELECTOR)
            .expect("a checked-out branch should show the branch picker");
        let search_field = cx
            .debug_bounds(SEARCH_FIELD_DEBUG_SELECTOR)
            .expect("the search field should be rendered in the title bar");
        let window_width = cx.update(|window, _| window.viewport_size().width);

        assert!(
            search_field.left() >= branch_picker.right(),
            "the search field follows the branch picker"
        );
        assert!(
            search_field.left() - branch_picker.right() < search_field.size.height,
            "the search field sits right next to the branch picker"
        );
        assert!(
            search_field.right() <= window_width * 0.5,
            "the search field stays in the left half of the title bar"
        );
    }

    #[gpui::test]
    async fn clicking_the_search_field_opens_search_everywhere_on_its_default_tab(
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

        let search_field = cx
            .debug_bounds(SEARCH_FIELD_DEBUG_SELECTOR)
            .expect("the search field should be rendered in the title bar");
        cx.simulate_click(search_field.center(), Modifiers::none());
        cx.run_until_parked();

        assert_eq!(
            *requested_tabs.borrow(),
            vec![None],
            "one click opens Search Everywhere once, on its default tab"
        );
    }

    #[gpui::test]
    async fn show_search_button_false_removes_the_search_field(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        assert!(
            cx.debug_bounds(SEARCH_FIELD_DEBUG_SELECTOR).is_some(),
            "the search field is shown by default"
        );

        update_user_settings(cx, |settings| {
            settings
                .title_bar
                .get_or_insert_default()
                .show_search_button = Some(false);
        });

        assert_eq!(
            cx.debug_bounds(SEARCH_FIELD_DEBUG_SELECTOR),
            None,
            "turning off show_search_button removes the search field"
        );
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
