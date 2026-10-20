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

use git::repository::RepositoryOperation;
use gpui::{
    AnyElement, App, ClickEvent, Context, DismissEvent, Entity, EntityId, Focusable, Hsla,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Point,
    Render, Styled, Subscription, Task, TaskExt as _, WeakEntity, Window, actions, anchored,
    deferred, div, px, rgb,
};
use project::{
    Project,
    git_store::{GitStore, GitStoreEvent, RepositoryEvent, RepositoryId},
    project_settings::ProjectSettings,
    trusted_worktrees::TrustedWorktrees,
};
use settings::Settings as _;
use std::collections::HashMap;

use std::path::Path;
use title_bar_settings::TitleBarSettings;
pub use toolbar_widgets::SEARCH_FIELD_DEBUG_SELECTOR;
use toolbar_widgets::{
    ToolbarSearchField, ToolbarWidget, toolbar_widget_icon_color, upstream_tracking,
};
use ui::{
    ChromeRegion, ContextMenu, ContextMenuEntry, IconButtonShape, PopoverMenu, TintColor, Tooltip,
    prelude::*, utils::platform_title_bar_height,
};
use util::{ResultExt, paths::PathExt as _};
use workspace::{
    AccessibleMode, MultiWorkspace, MultiWorkspaceEvent, ProjectTab, ToggleWorktreeSecurity,
    Workspace, WorkspaceSettings,
};

// `GitBranchPresentation.DEFAULT_MAX_BRANCH_NAME_LENGTH`.
const MAX_BRANCH_NAME_LENGTH: usize = 80;
const MAX_SHORT_SHA_LENGTH: usize = 8;
const PROJECT_TABS_DEBUG_SELECTOR: &str = "title_bar_project_tabs";
// Like IntelliJ's `WindowTabsComponent`, the tabs share the free width equally, but here only
// the row after Search Everywhere, so a tab is never wider than a comfortable title. The close
// button sits on the left in its own slot, mirrored on the right so the title stays centered
// and never touches it.
const PROJECT_TABS_GAP: Pixels = px(10.);
const PROJECT_TAB_MIN_WIDTH: Pixels = px(72.);
const PROJECT_TAB_MAX_WIDTH: Pixels = px(220.);
const PROJECT_TAB_PADDING_X: Pixels = px(6.);
const PROJECT_TAB_CLOSE_SLOT: Pixels = px(20.);
const PROJECT_TAB_CLOSE_GAP: Pixels = px(6.);
const PROJECT_TAB_CLOSE_ICON_SIDE: Pixels = px(14.);
// `IslandsUICustomization.paintProjectTab`: the fill is inset 1 px from the tab's sides, and
// the separator between two plain tabs is inset 5 px from the top and bottom.
const PROJECT_TAB_FILL_INSET: Pixels = px(1.);
const PROJECT_TAB_SEPARATOR_INSET: Pixels = px(5.);
// `mac.window.tabs.close.title`
const PROJECT_TAB_CLOSE_TOOLTIP: &str =
    "Click to close this tab. Option-click to close all tabs except this one";
const WORKTREE_AND_BRANCH_DEBUG_SELECTOR: &str = "title_bar_worktree_and_branch";
const VERSION_CONTROL_SETUP_DEBUG_SELECTOR: &str = "title_bar_version_control_setup";
const DETACHED_HEAD_TOOLTIP: &str = "Git: Detached HEAD doesn't point to any branch";
const REBASE_IN_PROGRESS_TOOLTIP: &str = "Git: Rebase in progress";
const PROJECT_GRADIENT_COLOR: u32 = 0x955AE0;

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
    hovered_project_tab: Option<EntityId>,
    project_tab_menu: Option<ProjectTabMenu>,
    repository_states: HashMap<RepositoryId, RepositoryWidgetState>,
    repository_state_tasks: HashMap<RepositoryId, Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// What the branch widget shows beyond the branch, which git reports only asynchronously.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RepositoryWidgetState {
    operation: Option<RepositoryOperation>,
    head_tag: Option<SharedString>,
    rebased_branch: Option<SharedString>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BranchWidgetIcon {
    /// The branch widget's own icon, which may show the working tree status.
    Branch,
    Warning,
    Tag,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BranchWidgetTooltip {
    Branch,
    DetachedHead,
    RebaseInProgress,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BranchWidgetPresentation {
    text: String,
    icon: BranchWidgetIcon,
    tooltip: BranchWidgetTooltip,
}

/// `GitBranchUtil.getDisplayableBranchText` and `GitCurrentBranchPresenter`: an operation in
/// progress prefixes the branch, and a detached HEAD shows the tag it is on or its short hash.
fn branch_widget_presentation(
    branch: Option<&str>,
    short_sha: Option<&str>,
    state: &RepositoryWidgetState,
) -> Option<BranchWidgetPresentation> {
    let truncate = |name: &str| git_ui_core::truncate_branch_name(name, MAX_BRANCH_NAME_LENGTH);
    if let Some(operation) = state.operation {
        // HEAD is detached while a rebase runs, so the rebased branch comes from git's files.
        let rebased_branch = state
            .rebased_branch
            .as_deref()
            .filter(|_| operation == RepositoryOperation::Rebase);
        let name = branch
            .or(rebased_branch)
            .map(truncate)
            .or_else(|| short_sha.map(str::to_string))?;
        let (text, tooltip) = match operation {
            RepositoryOperation::Merge => (format!("Merging {name}"), BranchWidgetTooltip::Branch),
            RepositoryOperation::Rebase => (
                format!("Rebasing {name}"),
                BranchWidgetTooltip::RebaseInProgress,
            ),
            RepositoryOperation::CherryPick => (
                format!("Cherry-picking in {name}"),
                BranchWidgetTooltip::Branch,
            ),
            RepositoryOperation::Revert => {
                (format!("Reverting in {name}"), BranchWidgetTooltip::Branch)
            }
        };
        let tooltip = if branch.is_none() && tooltip == BranchWidgetTooltip::Branch {
            BranchWidgetTooltip::DetachedHead
        } else {
            tooltip
        };
        return Some(BranchWidgetPresentation {
            text,
            icon: BranchWidgetIcon::Warning,
            tooltip,
        });
    }
    if let Some(branch) = branch {
        return Some(BranchWidgetPresentation {
            text: truncate(branch),
            icon: BranchWidgetIcon::Branch,
            tooltip: BranchWidgetTooltip::Branch,
        });
    }
    if let Some(tag) = &state.head_tag {
        return Some(BranchWidgetPresentation {
            text: truncate(tag),
            icon: BranchWidgetIcon::Tag,
            tooltip: BranchWidgetTooltip::DetachedHead,
        });
    }
    short_sha.map(|short_sha| BranchWidgetPresentation {
        text: short_sha.to_string(),
        icon: BranchWidgetIcon::Warning,
        tooltip: BranchWidgetTooltip::DetachedHead,
    })
}

/// The context menu of a project tab, like the one of IntelliJ's window tabs.
struct ProjectTabMenu {
    workspace_id: EntityId,
    menu: Entity<ContextMenu>,
    position: Point<Pixels>,
    _subscription: Subscription,
}

/// A project tab being dragged to another place in the strip.
#[derive(Clone)]
struct DraggedProjectTab {
    workspace: Entity<Workspace>,
    title: SharedString,
}

impl Render for DraggedProjectTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let corner_radius = WorkspaceSettings::get_global(cx).islands.corner_radius;
        h_flex()
            .h(ui::utils::title_bar_widget_height(cx))
            .min_w(PROJECT_TAB_MIN_WIDTH)
            .px(PROJECT_TAB_PADDING_X + PROJECT_TAB_CLOSE_SLOT + PROJECT_TAB_CLOSE_GAP)
            .justify_center()
            .rounded(corner_radius)
            .bg(cx.theme().colors().panel_background)
            .font_ui(cx)
            .child(Label::new(self.title.clone()))
    }
}

impl Render for TitleBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.multi_workspace.is_none() {
            if let Some(mw) = self
                .workspace
                .upgrade()
                .and_then(|ws| ws.read(cx).multi_workspace().cloned())
            {
                self.observe_multi_workspace(&mw, cx);
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

        // IntelliJ keeps the project gradient over a background image, so the main toolbar
        // stays tinted instead of turning into a plain dark strip.
        let project_gradient = (project_name.is_some() && title_bar_settings.show_project_gradient)
            .then(Self::project_gradient_color);
        // IntelliJ keeps its VCS widget in a project without version control, as a
        // "Version Control" combo that offers to create a repository.
        let show_version_control_setup = project_name.is_some()
            && repository.is_none()
            && is_git_enabled
            && title_bar_settings.show_branch_name;

        let project_tabs = self.render_project_tabs(window, cx);
        let project_items = h_flex()
            .h_full()
            .flex_1()
            // Lets the project tabs give way before the branch widget and the search field.
            .min_w_0()
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
                            .when(title_bar_settings.show_search_button, |title_bar| {
                                title_bar.child(Self::render_search_everywhere_field())
                            })
                            .children(project_tabs)
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

        subscriptions.push(cx.subscribe(
            &git_store,
            move |this, git_store, event, cx| match event {
                GitStoreEvent::ActiveRepositoryChanged(_) | GitStoreEvent::RepositoryAdded => {
                    this.refresh_repository_states(&git_store, cx);
                    cx.notify();
                }
                GitStoreEvent::RepositoryUpdated(
                    id,
                    RepositoryEvent::StatusesChanged | RepositoryEvent::HeadChanged,
                    is_active,
                ) => {
                    this.refresh_repository_state(&git_store, *id, cx);
                    if *is_active {
                        cx.notify();
                    }
                }
                GitStoreEvent::RepositoryUpdated(_, _, true) => {
                    cx.notify();
                }
                GitStoreEvent::RepositoryRemoved(id) => {
                    this.repository_states.remove(id);
                    this.repository_state_tasks.remove(id);
                }
                _ => {}
            },
        ));
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

        let mut this = Self {
            platform_titlebar,
            application_menu,
            workspace: workspace.weak_handle(),
            multi_workspace: multi_workspace.clone(),
            project,
            hovered_project_tab: None,
            project_tab_menu: None,
            repository_states: HashMap::default(),
            repository_state_tasks: HashMap::default(),
            _subscriptions: subscriptions,
        };
        if let Some(multi_workspace) = multi_workspace {
            this.observe_multi_workspace(&multi_workspace, cx);
        }
        this.refresh_repository_states(&git_store, cx);
        this
    }

    fn refresh_repository_states(&mut self, git_store: &Entity<GitStore>, cx: &mut Context<Self>) {
        let ids: Vec<RepositoryId> = git_store.read(cx).repositories().keys().copied().collect();
        for id in ids {
            self.refresh_repository_state(git_store, id, cx);
        }
    }

    /// Reads the operation in progress, and the tag a detached HEAD is on, for the branch widget.
    fn refresh_repository_state(
        &mut self,
        git_store: &Entity<GitStore>,
        id: RepositoryId,
        cx: &mut Context<Self>,
    ) {
        // Remote repositories cannot report an operation in progress.
        if !git_store.read(cx).is_local() {
            return;
        }
        let Some(repository) = git_store.read(cx).repositories().get(&id).cloned() else {
            return;
        };
        let head_sha = repository
            .read(cx)
            .head_commit
            .as_ref()
            .map(|commit| commit.sha.clone());
        let (operation, tags, rebased_branch) = repository.update(cx, |repository, _| {
            let detached = repository.branch.is_none();
            let tags = detached.then(|| repository.tags());
            let rebased_branch = detached.then(|| repository.rebased_branch());
            (repository.operation_in_progress(), tags, rebased_branch)
        });
        let task = cx.spawn(async move |this, cx| {
            let operation = operation
                .await
                .ok()
                .and_then(|result| result.log_err())
                .flatten();
            let head_tag = match tags {
                Some(tags) => tags
                    .await
                    .ok()
                    .and_then(|result| result.log_err())
                    .and_then(|tags| {
                        tags.into_iter()
                            .find(|tag| Some(&tag.commit_sha) == head_sha.as_ref())
                    })
                    .map(|tag| tag.name),
                None => None,
            };
            let rebased_branch = match rebased_branch {
                Some(rebased_branch) if operation == Some(RepositoryOperation::Rebase) => {
                    rebased_branch
                        .await
                        .ok()
                        .and_then(|result| result.log_err())
                        .flatten()
                        .map(SharedString::from)
                }
                _ => None,
            };
            this.update(cx, |this, cx| {
                let state = RepositoryWidgetState {
                    operation,
                    head_tag,
                    rebased_branch,
                };
                if this.repository_states.get(&id) != Some(&state) {
                    this.repository_states.insert(id, state);
                    cx.notify();
                }
            })
            .ok();
        });
        self.repository_state_tasks.insert(id, task);
    }

    /// The project tabs show every project of the window, so they redraw whenever the window
    /// adds, removes, reorders or switches its projects.
    fn observe_multi_workspace(
        &mut self,
        multi_workspace: &WeakEntity<MultiWorkspace>,
        cx: &mut Context<Self>,
    ) {
        if let Some(multi_workspace) = multi_workspace.upgrade() {
            self._subscriptions
                .push(cx.observe(&multi_workspace, |_, _, cx| cx.notify()));
            // While another project is shown this title bar is not drawn, so the mouse-leave
            // of a tab clicked in it never arrives and its hover would stick.
            self._subscriptions.push(cx.subscribe(
                &multi_workspace,
                |this, _, event: &MultiWorkspaceEvent, cx| {
                    if let MultiWorkspaceEvent::ActiveWorkspaceChanged { .. } = event {
                        this.hovered_project_tab = None;
                        this.project_tab_menu = None;
                        cx.notify();
                    }
                },
            ));
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

    /// IntelliJ's Islands window tabs, one per project of the window, in the main toolbar row
    /// left of the VCS widget (`WindowTabsComponent`, `IslandsUICustomization.paintProjectTab`).
    fn render_project_tabs(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let multi_workspace = self.multi_workspace.as_ref()?.upgrade()?;
        let (tabs, active_workspace) = {
            let multi_workspace = multi_workspace.read(cx);
            (
                multi_workspace.project_tabs(cx),
                multi_workspace.workspace().clone(),
            )
        };
        if tabs.is_empty() {
            return None;
        }

        let colors = cx.theme().colors();
        let selected_background = colors.panel_background;
        let hover_background = colors.ghost_element_hover;
        let separator_color = workspace::stripe_separator_color(cx.theme().appearance().is_light());
        let corner_radius = WorkspaceSettings::get_global(cx).islands.corner_radius;
        let height = ui::utils::title_bar_widget_height(cx);
        let icon_scale = ui::chrome_icon_scale(ChromeRegion::TitleBar, cx).unwrap_or(1.);
        let close_slot = PROJECT_TAB_CLOSE_SLOT * icon_scale;
        let close_icon_size =
            ui::exact_icon_size(PROJECT_TAB_CLOSE_ICON_SIDE * icon_scale, window.rem_size());
        let menu_workspace_id = self.project_tab_menu.as_ref().map(|menu| menu.workspace_id);
        let highlighted = |tab: &ProjectTab| {
            let id = tab.workspace.entity_id();
            tab.workspace == active_workspace
                || Some(id) == self.hovered_project_tab
                || Some(id) == menu_workspace_id
        };
        let separator_after: Vec<bool> = tabs
            .iter()
            .zip(tabs.iter().skip(1))
            .map(|(tab, next_tab)| !highlighted(tab) && !highlighted(next_tab))
            .chain(std::iter::once(false))
            .collect();
        let shows_close_button: Vec<bool> = tabs
            .iter()
            .map(|tab| {
                let id = Some(tab.workspace.entity_id());
                id == self.hovered_project_tab || id == menu_workspace_id
            })
            .collect();

        let weak_multi_workspace = multi_workspace.downgrade();
        let tab_elements = tabs
            .into_iter()
            .zip(separator_after)
            .zip(shows_close_button)
            .map(|((tab, separator_after), shows_close_button)| {
                let workspace_id = tab.workspace.entity_id();
                let is_active = tab.workspace == active_workspace;
                let is_hovered = Some(workspace_id) == self.hovered_project_tab
                    || Some(workspace_id) == menu_workspace_id;
                let tooltip = project_widget_tooltip(&tab.title, Some(tab.path.as_ref()));
                let debug_title = tab.title.clone();
                let close_debug_title = tab.title.clone();
                let label_debug_title = tab.title.clone();
                let dragged = DraggedProjectTab {
                    workspace: tab.workspace.clone(),
                    title: tab.title.clone(),
                };

                let close_button = shows_close_button.then(|| {
                    let multi_workspace = weak_multi_workspace.clone();
                    let workspace = tab.workspace.clone();
                    div()
                        .debug_selector(move || format!("PROJECT_TAB_CLOSE-{close_debug_title}"))
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            IconButton::new(
                                ("project_tab_close", workspace_id.as_u64()),
                                IconName::Close,
                            )
                            .shape(IconButtonShape::Square)
                            .size(ButtonSize::Compact)
                            .icon_size(close_icon_size)
                            .icon_color(Color::Muted)
                            .tooltip(Tooltip::text(PROJECT_TAB_CLOSE_TOOLTIP))
                            .on_click(
                                move |event: &ClickEvent, window, cx| {
                                    cx.stop_propagation();
                                    let workspace = workspace.clone();
                                    if event.modifiers().alt {
                                        update_multi_workspace_later(
                                            multi_workspace.clone(),
                                            window,
                                            cx,
                                            move |multi_workspace, window, cx| {
                                                multi_workspace
                                                    .close_other_workspaces(&workspace, window, cx)
                                                    .detach_and_log_err(cx);
                                            },
                                        );
                                    } else {
                                        close_project_tab(
                                            multi_workspace.clone(),
                                            workspace,
                                            window,
                                            cx,
                                        );
                                    }
                                },
                            ),
                        )
                });

                let pill = h_flex()
                    .id(("project_tab", workspace_id.as_u64()))
                    .debug_selector(move || format!("PROJECT_TAB-{debug_title}"))
                    .size_full()
                    .min_w_0()
                    .px(PROJECT_TAB_PADDING_X)
                    .gap(PROJECT_TAB_CLOSE_GAP)
                    .rounded(corner_radius)
                    .when(is_active, |pill| pill.bg(selected_background))
                    .when(!is_active && is_hovered, |pill| pill.bg(hover_background))
                    .cursor_pointer()
                    .tooltip(Tooltip::text(tooltip))
                    .on_hover(cx.listener(move |this, hovered: &bool, _window, cx| {
                        if *hovered {
                            this.hovered_project_tab = Some(workspace_id);
                        } else if this.hovered_project_tab == Some(workspace_id) {
                            this.hovered_project_tab = None;
                        }
                        cx.notify();
                    }))
                    .on_click({
                        let multi_workspace = weak_multi_workspace.clone();
                        let workspace = tab.workspace.clone();
                        move |_, window, cx| {
                            let workspace = workspace.clone();
                            // The click arrives in the title bar of the workspace being left,
                            // so the switch waits until this event is done with it.
                            update_multi_workspace_later(
                                multi_workspace.clone(),
                                window,
                                cx,
                                move |multi_workspace, window, cx| {
                                    multi_workspace.activate(workspace, None, window, cx);
                                },
                            );
                        }
                    })
                    .on_mouse_up(MouseButton::Middle, {
                        let multi_workspace = weak_multi_workspace.clone();
                        let workspace = tab.workspace.clone();
                        move |_, window, cx| {
                            cx.stop_propagation();
                            close_project_tab(
                                multi_workspace.clone(),
                                workspace.clone(),
                                window,
                                cx,
                            );
                        }
                    })
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener({
                            let workspace = tab.workspace.clone();
                            move |this, event: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.deploy_project_tab_menu(
                                    workspace.clone(),
                                    event.position,
                                    window,
                                    cx,
                                );
                            }
                        }),
                    )
                    .on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
                    .drag_over::<DraggedProjectTab>(move |style, _, _, cx| {
                        style.bg(cx.theme().colors().drop_target_background)
                    })
                    .on_drop({
                        let multi_workspace = weak_multi_workspace.clone();
                        let target = tab.workspace.clone();
                        move |dragged: &DraggedProjectTab, _window, cx| {
                            multi_workspace
                                .update(cx, |multi_workspace, cx| {
                                    multi_workspace.move_workspace_to_tab_of(
                                        &dragged.workspace,
                                        &target,
                                        cx,
                                    );
                                })
                                .log_err();
                        }
                    })
                    .child(div().flex_none().size(close_slot).children(close_button))
                    .child(
                        h_flex()
                            .debug_selector(move || {
                                format!("PROJECT_TAB_LABEL-{label_debug_title}")
                            })
                            .flex_1()
                            .min_w_0()
                            .justify_center()
                            .overflow_hidden()
                            .child(
                                Label::new(tab.title)
                                    .color(if is_active {
                                        Color::Default
                                    } else {
                                        Color::Muted
                                    })
                                    .truncate(),
                            ),
                    )
                    // Mirrors the close slot, so the title is centered in the tab.
                    .child(div().flex_none().size(close_slot));

                div()
                    .relative()
                    .flex()
                    .flex_1()
                    .min_w(PROJECT_TAB_MIN_WIDTH)
                    .max_w(PROJECT_TAB_MAX_WIDTH)
                    .h(height)
                    .px(PROJECT_TAB_FILL_INSET)
                    .child(pill)
                    .when(separator_after, |slot| {
                        slot.child(
                            div()
                                .absolute()
                                .right_0()
                                .top(PROJECT_TAB_SEPARATOR_INSET)
                                .bottom(PROJECT_TAB_SEPARATOR_INSET)
                                .w(px(1.))
                                .bg(separator_color),
                        )
                    })
            });

        let menu = self.project_tab_menu.as_ref().map(|menu| {
            deferred(
                anchored()
                    .position(menu.position)
                    .anchor(gpui::Anchor::TopLeft)
                    .child(menu.menu.clone()),
            )
            .with_priority(1)
        });

        Some(
            h_flex()
                .debug_selector(|| PROJECT_TABS_DEBUG_SELECTOR.into())
                .h(height)
                .flex_1()
                .min_w_0()
                .ml(PROJECT_TABS_GAP)
                .overflow_x_hidden()
                .children(tab_elements)
                .children(menu)
                .into_any_element(),
        )
    }

    fn deploy_project_tab_menu(
        &mut self,
        workspace: Entity<Workspace>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(multi_workspace) = self.multi_workspace.clone() else {
            return;
        };
        let workspace_id = workspace.entity_id();
        // Like `MoveProjectToNewWindow` and Open Recent's Open Projects, the actions that need
        // another tab are greyed with a single project, so moving it cannot leave an empty window.
        let has_other_tabs = multi_workspace.upgrade().is_some_and(|multi_workspace| {
            multi_workspace
                .read(cx)
                .project_tabs(cx)
                .iter()
                .any(|tab| tab.workspace != workspace)
        });
        let menu = ContextMenu::build(window, cx, move |menu, _, _| {
            let close_workspace = workspace.clone();
            let close_multi_workspace = multi_workspace.clone();
            let other_workspace = workspace.clone();
            let other_multi_workspace = multi_workspace.clone();
            let move_workspace = workspace;
            let move_multi_workspace = multi_workspace;
            // `mac.window.tabs.close.tab`, `mac.window.tabs.close.other.tabs` and
            // `mac.window.tabs.move.tab`
            menu.entry("Close Tab", None, move |window, cx| {
                close_project_tab(
                    close_multi_workspace.clone(),
                    close_workspace.clone(),
                    window,
                    cx,
                );
            })
            .item(
                ContextMenuEntry::new("Close Other Tabs")
                    .disabled(!has_other_tabs)
                    .handler(move |window, cx| {
                        let workspace = other_workspace.clone();
                        update_multi_workspace_later(
                            other_multi_workspace.clone(),
                            window,
                            cx,
                            move |multi_workspace, window, cx| {
                                multi_workspace
                                    .close_other_workspaces(&workspace, window, cx)
                                    .detach_and_log_err(cx);
                            },
                        );
                    }),
            )
            .item(
                ContextMenuEntry::new("Move Tab to New Window")
                    .disabled(!has_other_tabs)
                    .handler(move |window, cx| {
                        let workspace = move_workspace.clone();
                        update_multi_workspace_later(
                            move_multi_workspace.clone(),
                            window,
                            cx,
                            move |multi_workspace, window, cx| {
                                multi_workspace
                                    .move_workspace_to_new_window(&workspace, window, cx)
                                    .detach_and_log_err(cx);
                            },
                        );
                    }),
            )
        });
        window.focus(&menu.focus_handle(cx), cx);
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.project_tab_menu.take();
            cx.notify();
        });
        self.project_tab_menu = Some(ProjectTabMenu {
            workspace_id,
            menu,
            position,
            _subscription: subscription,
        });
        cx.notify();
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

    fn render_search_everywhere_field() -> impl IntoElement {
        ToolbarSearchField::new(
            "title_bar_search_field",
            // `action.SearchEverywhereToolbar.text`
            "Search everywhere",
            Box::new(zed_actions::search_everywhere::Toggle { tab: None }),
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

        let (presentation, branch_tooltip_name, root_name, upstream, incoming_outgoing, icon_info) = {
            let repo = repository.read(cx);
            let short_sha = repo.head_commit.as_ref().map(|commit| {
                commit
                    .sha
                    .chars()
                    .take(MAX_SHORT_SHA_LENGTH)
                    .collect::<String>()
            });
            let widget_state = self
                .repository_states
                .get(&repo.id)
                .cloned()
                .unwrap_or_default();
            let branch = repo.branch.as_ref().map(|branch| branch.name());
            let presentation =
                branch_widget_presentation(branch, short_sha.as_deref(), &widget_state);
            let branch_tooltip_name = branch.map(|name| name.to_string());
            // IntelliJ names the repository root in the tooltip when the project has several.
            let several_repositories = self
                .project
                .read(cx)
                .git_store()
                .read(cx)
                .repositories()
                .len()
                > 1;
            let root_name = several_repositories.then(|| repo.display_name());

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
                presentation,
                branch_tooltip_name,
                root_name,
                upstream,
                incoming_outgoing,
                icon_info,
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

        let branch_picker = presentation.and_then(|presentation| {
            settings.show_branch_name.then(|| {
                let (branch_icon, branch_icon_color) = if settings.show_branch_status_icon {
                    icon_info
                } else {
                    (IconName::GitBranch, icon_color)
                };
                let tooltip_kind = presentation.tooltip;
                let trigger = match presentation.icon {
                    BranchWidgetIcon::Branch => ToolbarWidget::new(
                        "project_branch_trigger",
                        Icon::new(branch_icon).color(branch_icon_color),
                        presentation.text,
                    )
                    .upstream(upstream),
                    BranchWidgetIcon::Warning => ToolbarWidget::new(
                        "project_branch_trigger",
                        Icon::new(IconName::Warning).color(Color::Warning),
                        presentation.text,
                    ),
                    BranchWidgetIcon::Tag => ToolbarWidget::new(
                        "project_branch_trigger",
                        Icon::new(IconName::TagLabel).color(icon_color),
                        presentation.text,
                    ),
                }
                .tab_index(0)
                .tooltip(move |_window, cx| {
                    let branch = match (tooltip_kind, &branch_tooltip_name) {
                        (BranchWidgetTooltip::DetachedHead, _) | (_, None) => {
                            return Tooltip::for_action(
                                DETACHED_HEAD_TOOLTIP,
                                &zed_actions::git::Branch,
                                cx,
                            );
                        }
                        (BranchWidgetTooltip::RebaseInProgress, _) => {
                            return Tooltip::for_action(
                                REBASE_IN_PROGRESS_TOOLTIP,
                                &zed_actions::git::Branch,
                                cx,
                            );
                        }
                        (BranchWidgetTooltip::Branch, Some(branch)) => branch,
                    };
                    let title = branch_widget_tooltip_title(branch);
                    match branch_widget_tooltip_meta(
                        root_name.as_deref(),
                        incoming_outgoing.as_deref(),
                    ) {
                        Some(meta) => {
                            Tooltip::with_meta(title, Some(&zed_actions::git::Branch), meta, cx)
                        }
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

/// Like `ProjectToolbarWidgetAction`, a project tab's tooltip is the project folder relative to
/// the home folder.
fn project_widget_tooltip(
    project_name: &SharedString,
    project_path: Option<&Path>,
) -> SharedString {
    project_path
        .map(|path| SharedString::from(path.compact().to_string_lossy().into_owned()))
        .unwrap_or_else(|| project_name.clone())
}

/// Updates the window's projects once the current event is handled: a project tab lives in the
/// title bar of one of those projects, which the update may switch away from or close.
fn update_multi_workspace_later(
    multi_workspace: WeakEntity<MultiWorkspace>,
    window: &mut Window,
    cx: &mut App,
    update: impl FnOnce(&mut MultiWorkspace, &mut Window, &mut Context<MultiWorkspace>) + 'static,
) {
    window.defer(cx, move |window, cx| {
        multi_workspace
            .update(cx, |multi_workspace, cx| {
                update(multi_workspace, window, cx)
            })
            .log_err();
    });
}

fn close_project_tab(
    multi_workspace: WeakEntity<MultiWorkspace>,
    workspace: Entity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    update_multi_workspace_later(
        multi_workspace,
        window,
        cx,
        move |multi_workspace, window, cx| {
            multi_workspace
                .close_workspace(&workspace, window, cx)
                .detach_and_log_err(cx);
        },
    );
}

/// IntelliJ's VCS widget tooltip title (`tooltip.branch.widget.vcs.branch.name.text`).
fn branch_widget_tooltip_title(branch_name: &str) -> String {
    format!("Git Branch: {branch_name}")
}

/// The lines under the title of IntelliJ's VCS widget tooltip: the repository root when the
/// project has several (`tooltip.branch.widget.root.name.text`), then the incoming and outgoing
/// commits.
fn branch_widget_tooltip_meta(
    root_name: Option<&str>,
    incoming_outgoing: Option<&str>,
) -> Option<String> {
    let lines: Vec<String> = root_name
        .map(|root_name| format!("Root: {root_name}"))
        .into_iter()
        .chain(incoming_outgoing.map(str::to_string))
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
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
    use std::{cell::RefCell, rc::Rc};
    use toolbar_widgets::ToolbarWidgetColors;
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

    async fn open_project_title_bar_with_repository(
        cx: &mut TestAppContext,
        repository: Option<TestRepository>,
    ) -> (Entity<Workspace>, Entity<TitleBar>, &mut VisualTestContext) {
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

    fn multi_workspace_of(
        workspace: &Entity<Workspace>,
        cx: &mut VisualTestContext,
    ) -> Entity<MultiWorkspace> {
        workspace.read_with(cx, |workspace, _| {
            workspace
                .multi_workspace()
                .and_then(|multi_workspace| multi_workspace.upgrade())
                .expect("the workspace belongs to a window")
        })
    }

    /// Opens another project in the window, with its own title bar like `title_bar::init`
    /// gives every workspace, and makes it the active one.
    async fn add_project_to_window(
        workspace: &Entity<Workspace>,
        root: &'static str,
        cx: &mut VisualTestContext,
    ) -> Entity<Workspace> {
        let fs = workspace.read_with(cx, |workspace, _| workspace.app_state().fs.clone());
        fs.create_dir(Path::new(root))
            .await
            .expect("project root should be created");
        let project = Project::test(fs, [Path::new(root)], cx).await;
        let multi_workspace = multi_workspace_of(workspace, cx);
        let new_workspace = multi_workspace.update_in(cx, |multi_workspace, window, cx| {
            multi_workspace.test_add_workspace(project, window, cx)
        });
        new_workspace.update_in(cx, |workspace, window, cx| {
            let title_bar = cx.new(|cx| TitleBar::new("title-bar", workspace, None, window, cx));
            workspace.set_titlebar_item(title_bar.into(), window, cx);
        });
        cx.run_until_parked();
        new_workspace
    }

    fn project_tab_bounds(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
        cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is rendered in the title bar"))
    }

    fn fills_inside(cx: &mut VisualTestContext, bounds: Bounds<Pixels>) -> Vec<Background> {
        quads_inside(cx, bounds)
            .into_iter()
            .map(|quad| quad.background)
            .collect()
    }

    fn projects_of_window(
        workspace: &Entity<Workspace>,
        cx: &mut VisualTestContext,
    ) -> (Vec<Entity<Workspace>>, Entity<Workspace>) {
        let multi_workspace = multi_workspace_of(workspace, cx);
        multi_workspace.read_with(cx, |multi_workspace, _| {
            (
                multi_workspace.workspaces().cloned().collect(),
                multi_workspace.workspace().clone(),
            )
        })
    }

    #[gpui::test]
    async fn project_tabs_show_every_project_in_order_and_select_the_active_one(
        cx: &mut TestAppContext,
    ) {
        let (workspace, _title_bar, cx) = open_project_title_bar(cx, Some("main")).await;
        add_project_to_window(&workspace, "/zed", cx).await;
        add_project_to_window(&workspace, "/notes", cx).await;

        let flint_tab = project_tab_bounds(cx, "PROJECT_TAB-flint");
        let zed_tab = project_tab_bounds(cx, "PROJECT_TAB-zed");
        let notes_tab = project_tab_bounds(cx, "PROJECT_TAB-notes");
        assert!(
            flint_tab.right() <= zed_tab.left() && zed_tab.right() <= notes_tab.left(),
            "one tab per project, in the order the projects joined the window: {flint_tab:?}, {zed_tab:?}, {notes_tab:?}"
        );

        let (selected_fill, widget_height) = cx.update(|_, cx| {
            (
                Background::from(cx.theme().colors().panel_background),
                ui::utils::title_bar_widget_height(cx),
            )
        });
        assert!(
            fills_inside(cx, notes_tab).contains(&selected_fill),
            "the active project's tab is filled with the island colour"
        );
        for tab in [flint_tab, zed_tab] {
            assert!(
                !fills_inside(cx, tab).contains(&selected_fill),
                "the other tabs are not filled: {tab:?}"
            );
        }
        assert_eq!(
            project_tab_bounds(cx, PROJECT_TABS_DEBUG_SELECTOR)
                .size
                .height,
            widget_height,
            "the tabs are as tall as the main toolbar widgets, like IntelliJ's 30 px window tabs"
        );
    }

    #[gpui::test]
    async fn a_single_project_still_shows_its_name_in_a_project_tab(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;

        let flint_tab = project_tab_bounds(cx, "PROJECT_TAB-flint");
        let search_field = project_tab_bounds(cx, SEARCH_FIELD_DEBUG_SELECTOR);
        assert!(
            search_field.right() <= flint_tab.left(),
            "the project tab follows Search Everywhere: {search_field:?}, {flint_tab:?}"
        );
    }

    #[gpui::test]
    async fn clicking_a_project_tab_switches_to_its_project(cx: &mut TestAppContext) {
        let (workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        let zed = add_project_to_window(&workspace, "/zed", cx).await;
        assert_eq!(projects_of_window(&workspace, cx).1, zed);

        let flint_tab = project_tab_bounds(cx, "PROJECT_TAB-flint");
        cx.simulate_click(flint_tab.center(), Modifiers::none());
        cx.run_until_parked();

        let (projects, active) = projects_of_window(&workspace, cx);
        assert_eq!(active, workspace, "clicking a tab activates its project");
        assert_eq!(projects.len(), 2, "switching keeps every project open");
        assert!(
            cx.debug_bounds("PROJECT_TAB-zed").is_some(),
            "the newly active project's title bar shows the same tabs"
        );
    }

    #[gpui::test]
    async fn project_tabs_follow_the_search_field_and_share_the_free_width(
        cx: &mut TestAppContext,
    ) {
        let (workspace, _title_bar, cx) = open_project_title_bar(cx, Some("main")).await;
        add_project_to_window(&workspace, "/zed", cx).await;
        let multi_workspace = multi_workspace_of(&workspace, cx);
        multi_workspace.update_in(cx, |multi_workspace, window, cx| {
            multi_workspace.activate(workspace.clone(), None, window, cx);
        });
        cx.run_until_parked();

        let search_field = project_tab_bounds(cx, SEARCH_FIELD_DEBUG_SELECTOR);
        let flint_tab = project_tab_bounds(cx, "PROJECT_TAB-flint");
        let zed_tab = project_tab_bounds(cx, "PROJECT_TAB-zed");
        assert!(
            search_field.right() + PROJECT_TABS_GAP <= flint_tab.left()
                && flint_tab.right() <= zed_tab.left(),
            "the tabs come right after Search Everywhere: {search_field:?}, {flint_tab:?}, {zed_tab:?}"
        );
        for tab in [flint_tab, zed_tab] {
            assert_eq!(
                tab.size.width,
                PROJECT_TAB_MAX_WIDTH - PROJECT_TAB_FILL_INSET * 2.,
                "with room to spare every tab gets its full width, not just its title: {tab:?}"
            );
            assert_eq!(
                tab.size.height,
                cx.update(|_, cx| ui::utils::title_bar_widget_height(cx))
            );
        }

        for root in [
            "/a-project-with-a-really-long-folder-name-number-one",
            "/a-project-with-a-really-long-folder-name-number-two",
            "/a-project-with-a-really-long-folder-name-number-three",
            "/a-project-with-a-really-long-folder-name-number-four",
            "/a-project-with-a-really-long-folder-name-number-five",
            "/a-project-with-a-really-long-folder-name-number-six",
            "/a-project-with-a-really-long-folder-name-number-seven",
            "/a-project-with-a-really-long-folder-name-number-eight",
        ] {
            add_project_to_window(&workspace, root, cx).await;
        }
        multi_workspace.update_in(cx, |multi_workspace, window, cx| {
            multi_workspace.activate(workspace.clone(), None, window, cx);
        });
        cx.run_until_parked();

        let title_bar_height = cx.update(|window, cx| platform_title_bar_height(window, cx));
        let viewport_width = cx.update(|window, _| window.viewport_size().width);
        let branch_widget = project_tab_bounds(cx, WORKTREE_AND_BRANCH_DEBUG_SELECTOR);
        let search_field = project_tab_bounds(cx, SEARCH_FIELD_DEBUG_SELECTOR);
        let flint_tab = project_tab_bounds(cx, "PROJECT_TAB-flint");
        assert!(
            branch_widget.right() <= search_field.left()
                && search_field.right() <= flint_tab.left(),
            "many projects keep the order: {branch_widget:?}, {search_field:?}, {flint_tab:?}"
        );
        assert_visible_inside_title_bar(search_field, title_bar_height, "Search Everywhere field");
        assert!(
            search_field.right() <= viewport_width,
            "many projects never push the search field out of the window: {search_field:?}"
        );
        assert!(
            search_field.size.width >= px(200.),
            "the search field keeps its width while the tabs shrink: {search_field:?}"
        );
        assert!(
            flint_tab.size.width < PROJECT_TAB_MAX_WIDTH - PROJECT_TAB_FILL_INSET * 2.
                && flint_tab.size.width >= PROJECT_TAB_MIN_WIDTH - PROJECT_TAB_FILL_INSET * 2.,
            "many tabs share the row equally down to a readable width: {flint_tab:?}"
        );
    }

    #[gpui::test]
    async fn the_close_button_and_a_middle_click_close_only_that_project(cx: &mut TestAppContext) {
        let (workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        let zed = add_project_to_window(&workspace, "/zed", cx).await;
        let notes = add_project_to_window(&workspace, "/notes", cx).await;

        assert_eq!(
            cx.debug_bounds("PROJECT_TAB_CLOSE-zed"),
            None,
            "the close button shows only on hover"
        );
        let zed_tab = project_tab_bounds(cx, "PROJECT_TAB-zed");
        cx.simulate_mouse_move(zed_tab.center(), None, Modifiers::none());
        cx.run_until_parked();
        let close_button = project_tab_bounds(cx, "PROJECT_TAB_CLOSE-zed");
        let zed_tab = project_tab_bounds(cx, "PROJECT_TAB-zed");
        let zed_label = project_tab_bounds(cx, "PROJECT_TAB_LABEL-zed");
        assert!(
            close_button.right() <= zed_tab.center().x,
            "the close button sits on the left of the tab, like IntelliJ's window tabs"
        );
        assert!(
            zed_label.left() - close_button.right() >= PROJECT_TAB_CLOSE_GAP,
            "the title keeps clear of the close button: {close_button:?}, {zed_label:?}"
        );
        assert!(
            close_button.size.width >= PROJECT_TAB_CLOSE_SLOT
                && close_button.size.height >= PROJECT_TAB_CLOSE_SLOT,
            "the close button is an easy target: {close_button:?}"
        );
        assert!(
            (zed_label.center().x - zed_tab.center().x).abs() <= px(1.),
            "the title is centered in the tab: {zed_tab:?}, {zed_label:?}"
        );

        cx.simulate_click(close_button.center(), Modifiers::none());
        cx.run_until_parked();
        let (projects, active) = projects_of_window(&workspace, cx);
        assert_eq!(
            projects,
            vec![workspace.clone(), notes.clone()],
            "the close button closes only its project"
        );
        assert_eq!(
            active, notes,
            "closing another tab keeps the active project"
        );
        assert!(!projects.contains(&zed));

        let flint_tab = project_tab_bounds(cx, "PROJECT_TAB-flint");
        cx.simulate_mouse_down(flint_tab.center(), MouseButton::Middle, Modifiers::none());
        cx.simulate_mouse_up(flint_tab.center(), MouseButton::Middle, Modifiers::none());
        cx.run_until_parked();
        let (projects, active) = projects_of_window(&workspace, cx);
        assert_eq!(
            projects,
            vec![notes.clone()],
            "a middle click closes the tab"
        );
        assert_eq!(active, notes);
    }

    #[gpui::test]
    async fn the_project_tab_menu_offers_intellij_tab_actions(cx: &mut TestAppContext) {
        let (workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        let zed = add_project_to_window(&workspace, "/zed", cx).await;
        add_project_to_window(&workspace, "/notes", cx).await;

        let zed_tab = project_tab_bounds(cx, "PROJECT_TAB-zed");
        cx.simulate_mouse_down(zed_tab.center(), MouseButton::Right, Modifiers::none());
        cx.simulate_mouse_up(zed_tab.center(), MouseButton::Right, Modifiers::none());
        cx.run_until_parked();
        for item in [
            "MENU_ITEM-Close Tab",
            "MENU_ITEM-Close Other Tabs",
            "MENU_ITEM-Move Tab to New Window",
        ] {
            assert!(
                cx.debug_bounds(item).is_some(),
                "the tab menu offers {item}"
            );
        }

        let close_others = project_tab_bounds(cx, "MENU_ITEM-Close Other Tabs");
        cx.simulate_click(close_others.center(), Modifiers::none());
        cx.run_until_parked();
        let (projects, active) = projects_of_window(&workspace, cx);
        assert_eq!(
            projects,
            vec![zed.clone()],
            "Close Other Tabs keeps only that project"
        );
        assert_eq!(active, zed);
    }

    #[gpui::test]
    async fn a_single_project_tab_cannot_be_moved_to_a_new_window(cx: &mut TestAppContext) {
        let (workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;

        let flint_tab = project_tab_bounds(cx, "PROJECT_TAB-flint");
        cx.simulate_mouse_down(flint_tab.center(), MouseButton::Right, Modifiers::none());
        cx.simulate_mouse_up(flint_tab.center(), MouseButton::Right, Modifiers::none());
        cx.run_until_parked();
        let move_tab = project_tab_bounds(cx, "MENU_ITEM-Move Tab to New Window");
        cx.simulate_click(move_tab.center(), Modifiers::none());
        cx.run_until_parked();

        let multi_workspace = multi_workspace_of(&workspace, cx);
        multi_workspace
            .update_in(cx, |multi_workspace, window, cx| {
                multi_workspace.move_workspace_to_new_window(&workspace, window, cx)
            })
            .await
            .expect("moving the only tab is a no-op");
        cx.run_until_parked();

        let (projects, active) = projects_of_window(&workspace, cx);
        assert_eq!(projects, vec![workspace.clone()]);
        assert_eq!(active, workspace);
        assert_eq!(
            cx.update(|_, cx| cx.windows().len()),
            1,
            "the only project stays in its window instead of leaving an empty one behind"
        );
    }

    #[gpui::test]
    async fn switching_projects_clears_the_hover_of_a_tab_clicked_in_another_title_bar(
        cx: &mut TestAppContext,
    ) {
        let (workspace, title_bar, cx) = open_project_title_bar(cx, None).await;
        let zed = add_project_to_window(&workspace, "/zed", cx).await;
        let multi_workspace = multi_workspace_of(&workspace, cx);
        multi_workspace.update_in(cx, |multi_workspace, window, cx| {
            multi_workspace.activate(workspace.clone(), None, window, cx);
        });
        cx.run_until_parked();

        let zed_tab = project_tab_bounds(cx, "PROJECT_TAB-zed");
        cx.simulate_mouse_move(zed_tab.center(), None, Modifiers::none());
        cx.run_until_parked();
        cx.simulate_click(zed_tab.center(), Modifiers::none());
        cx.run_until_parked();
        assert_eq!(projects_of_window(&workspace, cx).1, zed);
        cx.simulate_mouse_move(point(px(1.), px(300.)), None, Modifiers::none());
        cx.run_until_parked();

        multi_workspace.update_in(cx, |multi_workspace, window, cx| {
            multi_workspace.activate(workspace.clone(), None, window, cx);
        });
        cx.run_until_parked();

        assert_eq!(
            title_bar.read_with(cx, |title_bar, _| title_bar.hovered_project_tab),
            None,
            "the tab clicked before the switch is not drawn hovered"
        );
        assert_eq!(
            cx.debug_bounds("PROJECT_TAB_CLOSE-zed"),
            None,
            "and shows no close button"
        );
    }

    #[gpui::test]
    async fn move_tab_to_new_window_moves_only_that_project(cx: &mut TestAppContext) {
        let (workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        let zed = add_project_to_window(&workspace, "/zed", cx).await;

        let flint_tab = project_tab_bounds(cx, "PROJECT_TAB-flint");
        cx.simulate_mouse_down(flint_tab.center(), MouseButton::Right, Modifiers::none());
        cx.simulate_mouse_up(flint_tab.center(), MouseButton::Right, Modifiers::none());
        cx.run_until_parked();
        let move_tab = project_tab_bounds(cx, "MENU_ITEM-Move Tab to New Window");
        cx.simulate_click(move_tab.center(), Modifiers::none());
        cx.run_until_parked();

        let (projects, active) = projects_of_window(&workspace, cx);
        assert_eq!(
            projects,
            vec![zed.clone()],
            "the moved project leaves this window"
        );
        assert_eq!(active, zed);
        assert_eq!(
            cx.update(|_, cx| cx.windows().len()),
            2,
            "the moved project opens in a window of its own"
        );
    }

    #[gpui::test]
    async fn dragging_a_project_tab_onto_another_moves_it_there(cx: &mut TestAppContext) {
        let (workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        let zed = add_project_to_window(&workspace, "/zed", cx).await;
        let notes = add_project_to_window(&workspace, "/notes", cx).await;

        let flint_tab = project_tab_bounds(cx, "PROJECT_TAB-flint");
        let notes_tab = project_tab_bounds(cx, "PROJECT_TAB-notes");
        cx.simulate_mouse_down(flint_tab.center(), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(
            flint_tab.center() + point(px(20.), px(0.)),
            Some(MouseButton::Left),
            Modifiers::none(),
        );
        cx.simulate_mouse_move(
            notes_tab.center(),
            Some(MouseButton::Left),
            Modifiers::none(),
        );
        cx.simulate_mouse_up(notes_tab.center(), MouseButton::Left, Modifiers::none());
        cx.run_until_parked();

        let (projects, active) = projects_of_window(&workspace, cx);
        assert_eq!(
            projects,
            vec![zed, notes.clone(), workspace.clone()],
            "the dragged tab takes the place of the tab it is dropped on"
        );
        assert_eq!(active, notes, "dragging a tab does not switch projects");
        let notes_tab = project_tab_bounds(cx, "PROJECT_TAB-notes");
        let flint_tab = project_tab_bounds(cx, "PROJECT_TAB-flint");
        assert!(notes_tab.right() <= flint_tab.left());
    }

    #[gpui::test]
    async fn project_tabs_tell_projects_with_the_same_name_apart(cx: &mut TestAppContext) {
        let (workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        add_project_to_window(&workspace, "/worktrees/feature/flint", cx).await;

        assert!(
            cx.debug_bounds("PROJECT_TAB-flint [/flint]").is_some(),
            "a project that shares its name gets its folder in brackets, like IntelliJ's frame title"
        );
        assert!(
            cx.debug_bounds("PROJECT_TAB-flint [/worktrees/feature/flint]")
                .is_some()
        );
        assert_eq!(cx.debug_bounds("PROJECT_TAB-flint"), None);
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
            .expect("the Search Everywhere field should be rendered in the title bar");
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
    async fn project_gradient_stays_while_a_background_image_is_set(cx: &mut TestAppContext) {
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
        assert!(
            cx.debug_bounds("project_gradient").is_some(),
            "an editor image keeps the project gradient fill of the main toolbar, like IntelliJ"
        );

        publish_images(
            ui::WindowBackgroundImages {
                editor_and_tools: None,
                empty_frame: Some(layer),
            },
            cx,
        );
        assert!(
            cx.debug_bounds("project_gradient").is_some(),
            "an empty frame image keeps the project gradient too"
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
        let active_tab_fill =
            cx.update(|_, cx| Background::from(cx.theme().colors().panel_background));
        let project_tab = project_tab_bounds(cx, "PROJECT_TAB-flint");
        let project_tab_fills = |cx: &mut VisualTestContext| fills_inside(cx, project_tab);
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
            project_tab_fills(cx).contains(&active_tab_fill),
            "an active window paints the selected project tab at full strength"
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
        let inactive_tab_fills = project_tab_fills(cx);
        assert!(
            inactive_tab_fills.contains(&active_tab_fill.opacity(inactive_opacity)),
            "an inactive window dims the selected project tab"
        );
        assert!(
            !inactive_tab_fills.contains(&active_tab_fill),
            "no undimmed project tab remains in an inactive window"
        );
        // The test theme gives the islands and the frame one colour, so the selected tab's
        // own dimmed fill is left out here.
        let scaled_project_tab = project_tab.scale(cx.update(|window, _| window.scale_factor()));
        assert!(
            !inactive_quads
                .iter()
                .filter(|quad| !scaled_project_tab.contains(&quad.bounds.center()))
                .any(|quad| {
                    quad.background == Background::from(title_bar_color).opacity(inactive_opacity)
                }),
            "the title bar background is never dimmed"
        );
    }

    #[gpui::test]
    async fn search_everywhere_field_follows_the_branch_widget(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, Some("main")).await;
        let title_bar_height = cx.update(|window, cx| platform_title_bar_height(window, cx));
        let viewport_width = cx.update(|window, _| window.viewport_size().width);
        let project_tabs = cx
            .debug_bounds(PROJECT_TABS_DEBUG_SELECTOR)
            .expect("the project tabs are rendered in the title bar");
        let branch_widget = cx
            .debug_bounds(WORKTREE_AND_BRANCH_DEBUG_SELECTOR)
            .expect("a checked-out branch should show the branch picker");
        let search_field = cx
            .debug_bounds(SEARCH_FIELD_DEBUG_SELECTOR)
            .expect("the Search Everywhere field is rendered in the title bar");

        assert_visible_inside_title_bar(search_field, title_bar_height, "Search Everywhere field");
        assert!(
            branch_widget.right() <= search_field.left()
                && search_field.right() <= project_tabs.left(),
            "the title bar reads branch, the search field, then the project tabs: {branch_widget:?}, {search_field:?}, {project_tabs:?}"
        );
        assert!(
            search_field.left() - branch_widget.right() <= px(4.),
            "the search field sits right after the branch widget: {branch_widget:?}, {search_field:?}"
        );
        assert!(
            search_field.size.width >= px(200.),
            "the search field is a wide field, not an icon button: {search_field:?}"
        );
        assert!(
            search_field.right() < viewport_width / 2.,
            "nothing is pinned to the right corner of the title bar: {search_field:?}"
        );
        assert_eq!(
            search_field.size.height, project_tabs.size.height,
            "the search field is as tall as the project tabs and the branch widget"
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
        let widgets = |cx: &mut VisualTestContext, phase: &str| {
            [
                WORKTREE_AND_BRANCH_DEBUG_SELECTOR,
                SEARCH_FIELD_DEBUG_SELECTOR,
                PROJECT_TABS_DEBUG_SELECTOR,
            ]
            .map(|selector| {
                let bounds = cx
                    .debug_bounds(selector)
                    .unwrap_or_else(|| panic!("{phase} the {selector} is shown in the title bar"));
                assert_visible_inside_title_bar(
                    bounds,
                    title_bar_height,
                    &format!("{selector} {phase}"),
                );
                bounds
            })
        };

        assert!(
            !cx.update(|window, _| window.is_fullscreen()),
            "a new project window opens outside full screen"
        );
        let [branch_widget, ..] = widgets(cx, "outside full screen");
        assert!(
            branch_widget.left() >= traffic_light_padding,
            "outside full screen the branch widget starts after the macOS window buttons, got {branch_widget:?}"
        );
        let windowed_branch_widget_left = branch_widget.left();

        cx.update(|window, _| {
            window.toggle_fullscreen();
            window.refresh();
        });
        cx.run_until_parked();

        assert!(
            cx.update(|window, _| window.is_fullscreen()),
            "toggling full screen puts the window in full screen"
        );
        let [branch_widget, search_field, project_tabs] = widgets(cx, "in full screen");
        assert!(
            branch_widget.left() < traffic_light_padding,
            "in full screen the branch widget moves into the space of the hidden macOS window buttons, got {branch_widget:?}"
        );
        assert!(
            branch_widget.left() < windowed_branch_widget_left,
            "in full screen the branch widget shifts left compared to windowed mode, got {branch_widget:?} (windowed {windowed_branch_widget_left:?})"
        );
        assert!(
            branch_widget.right() <= search_field.left()
                && search_field.right() <= project_tabs.left(),
            "in full screen the widgets keep their order: {branch_widget:?}, {search_field:?}, {project_tabs:?}"
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
        let [branch_widget, ..] = widgets(cx, "after leaving full screen");
        assert!(
            branch_widget.left() >= traffic_light_padding,
            "after leaving full screen the branch widget starts after the macOS window buttons again, got {branch_widget:?}"
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
            .expect("the Search Everywhere field should be rendered in the title bar");
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
            "the Search Everywhere field is shown by default"
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
            "turning off show_search_button removes the Search Everywhere field"
        );
        assert!(
            cx.debug_bounds(PROJECT_TABS_DEBUG_SELECTOR).is_some(),
            "the project tabs stay when the search field is hidden"
        );
    }

    #[gpui::test]
    async fn title_bar_has_no_navigation_search_or_settings_icon_buttons(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, Some("main")).await;

        for selector in [
            "ICON-ArrowLeft",
            "ICON-ArrowRight",
            "ICON-MagnifyingGlass",
            "ICON-Settings",
            "title_bar_search_button",
            "title_bar_settings_button",
            "title_bar_navigation_buttons",
        ] {
            assert_eq!(
                cx.debug_bounds(selector),
                None,
                "the title bar keeps only the project, branch and search widgets, but {selector} is rendered"
            );
        }
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
    async fn title_bar_icon_size_grows_the_search_field_with_the_other_widgets(
        cx: &mut TestAppContext,
    ) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, Some("main")).await;
        let default_field = cx
            .debug_bounds(SEARCH_FIELD_DEBUG_SELECTOR)
            .expect("the Search Everywhere field is rendered in the title bar");

        update_user_settings(cx, |settings| {
            settings.title_bar.get_or_insert_default().icon_size = Some(21);
        });

        let title_bar_height = cx.update(|window, cx| platform_title_bar_height(window, cx));
        let branch_widget = cx
            .debug_bounds(WORKTREE_AND_BRANCH_DEBUG_SELECTOR)
            .expect("the branch widget is rendered in the title bar");
        let scaled_field = cx
            .debug_bounds(SEARCH_FIELD_DEBUG_SELECTOR)
            .expect("the Search Everywhere field is rendered in the title bar");
        assert_visible_inside_title_bar(scaled_field, title_bar_height, "scaled search field");
        assert_eq!(
            scaled_field.size.height, branch_widget.size.height,
            "the scaled search field stays as tall as the scaled widgets"
        );
        assert!(
            scaled_field.size.height > default_field.size.height
                && scaled_field.size.width > default_field.size.width,
            "a larger title_bar.icon_size grows the search field: {default_field:?} -> {scaled_field:?}"
        );
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
            PROJECT_TABS_DEBUG_SELECTOR,
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
                PROJECT_TABS_DEBUG_SELECTOR,
                WORKTREE_AND_BRANCH_DEBUG_SELECTOR,
            ] {
                let widget = cx
                    .debug_bounds(selector)
                    .unwrap_or_else(|| panic!("{selector} is rendered in the title bar"));
                assert_eq!(
                    widget.size.height,
                    px(expected_height),
                    "{selector} is {expected_height} px tall like the MainToolbar combo pills in {density:?} density"
                );
            }

            let branch_widgets = cx
                .debug_bounds(WORKTREE_AND_BRANCH_DEBUG_SELECTOR)
                .expect("the worktree and branch widgets are rendered in the title bar");
            let worktree_widget = point(branch_widgets.left() + px(12.), branch_widgets.center().y);
            cx.simulate_mouse_move(worktree_widget, None, Modifiers::none());
            cx.run_until_parked();
            let (hover, scale_factor) = cx.update(|window, cx| {
                (
                    ToolbarWidgetColors::current(cx).hover,
                    window.scale_factor(),
                )
            });
            let hovered_pill = quads_inside(cx, branch_widgets)
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
    async fn search_field_is_filled_and_brightens_on_hover(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        let colors = cx.update(|_, cx| ToolbarWidgetColors::current(cx));
        let search_field = cx
            .debug_bounds(SEARCH_FIELD_DEBUG_SELECTOR)
            .expect("the Search Everywhere field is rendered in the title bar");
        let fills = |cx: &mut VisualTestContext| {
            quads_inside(cx, search_field)
                .into_iter()
                .map(|quad| quad.background)
                .collect::<Vec<_>>()
        };

        assert!(
            fills(cx).contains(&Background::from(colors.hover)),
            "at rest the search field is filled so it reads as a field over the gradient"
        );

        cx.simulate_mouse_move(search_field.center(), None, Modifiers::none());
        cx.run_until_parked();
        assert!(
            fills(cx).contains(&Background::from(colors.open)),
            "hovering the search field brightens its fill"
        );
        cx.simulate_mouse_move(point(px(1.), px(300.)), None, Modifiers::none());
        cx.run_until_parked();
    }

    #[gpui::test]
    async fn clicking_the_search_field_does_not_arm_the_window_drag(cx: &mut TestAppContext) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        let search_field = cx
            .debug_bounds(SEARCH_FIELD_DEBUG_SELECTOR)
            .expect("the Search Everywhere field is rendered in the title bar");
        let empty_title_bar = point(search_field.right() + px(40.), search_field.center().y);

        cx.simulate_mouse_move(search_field.center(), None, Modifiers::none());
        cx.simulate_mouse_down(search_field.center(), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(search_field.center(), MouseButton::Left, Modifiers::none());
        // The test window's `start_window_move` is unimplemented, so a drag left armed by the
        // press would panic on this move across the title bar.
        cx.simulate_mouse_move(empty_title_bar, None, Modifiers::none());
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
    async fn project_without_a_repository_shows_the_version_control_widget(
        cx: &mut TestAppContext,
    ) {
        let (_workspace, _title_bar, cx) = open_project_title_bar(cx, None).await;
        let project_tabs = cx
            .debug_bounds(PROJECT_TABS_DEBUG_SELECTOR)
            .expect("the project tabs are rendered in the title bar");
        let version_control = cx
            .debug_bounds(VERSION_CONTROL_SETUP_DEBUG_SELECTOR)
            .expect("a project without a repository keeps IntelliJ's VCS widget");
        assert!(
            cx.debug_bounds("TOOLBAR_WIDGET-Version Control").is_some(),
            "the widget reads Version Control"
        );
        assert!(
            version_control.right() <= project_tabs.left(),
            "the VCS widget leads the row and the project tabs come after Search Everywhere"
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

    async fn open_title_bar_with_configured_repository(
        cx: &mut TestAppContext,
        configure: impl FnOnce(&project::FakeFs, &Path),
    ) -> &mut VisualTestContext {
        let app_state = cx.update(|cx| {
            let app_state = AppState::test(cx);
            PlatformTitleBar::init(cx);
            app_state
        });
        let dot_git = Path::new(PROJECT_ROOT).join(".git");
        app_state
            .fs
            .create_dir(&dot_git)
            .await
            .expect("git directory should be created");
        configure(&app_state.fs.as_fake(), &dot_git);
        let project = Project::test(app_state.fs.clone(), [Path::new(PROJECT_ROOT)], cx).await;
        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project, window, cx));
        let workspace =
            multi_workspace.read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone());
        workspace.update_in(cx, |workspace, window, cx| {
            let title_bar = cx.new(|cx| TitleBar::new("title-bar", workspace, None, window, cx));
            workspace.set_titlebar_item(title_bar.into(), window, cx);
        });
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        cx
    }

    #[gpui::test]
    async fn an_operation_in_progress_prefixes_the_branch_widget_like_intellij(
        cx: &mut TestAppContext,
    ) {
        let cx = open_title_bar_with_configured_repository(cx, |fs, dot_git| {
            fs.set_branch_name(dot_git, Some("main"));
            fs.set_head_for_repo(dot_git, &[], DETACHED_HEAD_SHA);
            fs.set_operation_in_progress_for_repo(dot_git, Some(RepositoryOperation::Merge));
        })
        .await;
        assert!(
            cx.debug_bounds("TOOLBAR_WIDGET-Merging main").is_some(),
            "a merge in progress shows \"Merging main\""
        );
        assert_eq!(cx.debug_bounds("TOOLBAR_WIDGET-main"), None);
    }

    #[gpui::test]
    async fn a_detached_head_on_a_tag_shows_the_tag_name(cx: &mut TestAppContext) {
        let cx = open_title_bar_with_configured_repository(cx, |fs, dot_git| {
            fs.set_head_for_repo(dot_git, &[], DETACHED_HEAD_SHA);
            fs.set_branch_name(dot_git, None::<String>);
            fs.insert_tags(dot_git, &[("v2.0", DETACHED_HEAD_SHA)]);
        })
        .await;
        assert!(
            cx.debug_bounds("TOOLBAR_WIDGET-v2.0").is_some(),
            "a detached HEAD on a tag shows the tag instead of the hash"
        );
        assert_eq!(cx.debug_bounds("TOOLBAR_WIDGET-01234567"), None);
    }

    #[test]
    fn the_branch_widget_text_follows_intellij_for_operations_and_detached_heads() {
        let state = |operation, head_tag: Option<&str>| RepositoryWidgetState {
            operation,
            head_tag: head_tag.map(|tag| SharedString::from(tag.to_string())),
            rebased_branch: None,
        };
        let present = |branch, operation, head_tag| {
            branch_widget_presentation(branch, Some("0123abcd"), &state(operation, head_tag))
                .expect("the widget has a text")
        };
        for (operation, text) in [
            (RepositoryOperation::Merge, "Merging main"),
            (RepositoryOperation::Rebase, "Rebasing main"),
            (RepositoryOperation::CherryPick, "Cherry-picking in main"),
            (RepositoryOperation::Revert, "Reverting in main"),
        ] {
            let presentation = present(Some("main"), Some(operation), None);
            assert_eq!(presentation.text, text);
            assert_eq!(presentation.icon, BranchWidgetIcon::Warning);
        }
        assert_eq!(
            present(Some("main"), Some(RepositoryOperation::Rebase), None).tooltip,
            BranchWidgetTooltip::RebaseInProgress
        );
        assert_eq!(
            present(Some("main"), Some(RepositoryOperation::Merge), None).tooltip,
            BranchWidgetTooltip::Branch
        );

        let normal = present(Some("main"), None, None);
        assert_eq!(normal.text, "main");
        assert_eq!(normal.icon, BranchWidgetIcon::Branch);

        let tag = present(None, None, Some("v1.0"));
        assert_eq!(tag.text, "v1.0");
        assert_eq!(tag.icon, BranchWidgetIcon::Tag);
        assert_eq!(tag.tooltip, BranchWidgetTooltip::DetachedHead);

        let detached = present(None, None, None);
        assert_eq!(detached.text, "0123abcd");
        assert_eq!(detached.icon, BranchWidgetIcon::Warning);

        let rebasing = |rebased_branch: Option<&str>| {
            branch_widget_presentation(
                None,
                Some("0123abcd"),
                &RepositoryWidgetState {
                    operation: Some(RepositoryOperation::Rebase),
                    head_tag: None,
                    rebased_branch: rebased_branch.map(|name| SharedString::from(name.to_string())),
                },
            )
            .expect("the widget has a text")
        };
        assert_eq!(
            rebasing(Some("feature")).text,
            "Rebasing feature",
            "HEAD is detached during a rebase, so the rebased branch names it"
        );
        assert_eq!(rebasing(None).text, "Rebasing 0123abcd");
    }

    #[test]
    fn long_branch_names_are_cut_in_the_middle_at_eighty_characters() {
        let long_name = format!("feature/{}-12345", "x".repeat(100));
        let presentation =
            branch_widget_presentation(Some(&long_name), None, &RepositoryWidgetState::default())
                .expect("the widget has a text");
        assert_eq!(presentation.text.chars().count(), 79);
        assert!(presentation.text.ends_with("\u{2026}12345"));
        let short = "a".repeat(84);
        assert_eq!(
            branch_widget_presentation(Some(&short), None, &RepositoryWidgetState::default())
                .map(|presentation| presentation.text),
            Some(short)
        );
    }

    #[test]
    fn the_branch_tooltip_names_the_root_only_with_several_repositories() {
        assert_eq!(branch_widget_tooltip_meta(None, None), None);
        assert_eq!(
            branch_widget_tooltip_meta(Some("api"), None).as_deref(),
            Some("Root: api")
        );
        assert_eq!(
            branch_widget_tooltip_meta(Some("api"), Some("1 incoming commit")).as_deref(),
            Some("Root: api\n1 incoming commit")
        );
        assert_eq!(
            branch_widget_tooltip_meta(None, Some("1 incoming commit")).as_deref(),
            Some("1 incoming commit")
        );
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
