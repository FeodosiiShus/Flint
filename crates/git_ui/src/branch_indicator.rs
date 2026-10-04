use gpui::{Anchor, Empty, Entity, Subscription, WeakEntity};
use project::{
    Project,
    git_store::{GitStore, GitStoreEvent},
};
use ui::{ChromeRegion, PopoverMenu, TintColor, Tooltip, prelude::*};
use workspace::{HideStatusItem, ItemHandle, StatusItemView, Workspace};

const MAX_BRANCH_NAME_LENGTH: usize = 40;
const SHORT_SHA_LENGTH: usize = 8;
const POPOVER_OFFSET_Y: f32 = -2.;

pub struct BranchIndicator {
    workspace: WeakEntity<Workspace>,
    project: Entity<Project>,
    _git_store_subscription: Subscription,
}

impl BranchIndicator {
    pub fn new(workspace: &Workspace, cx: &mut Context<Self>) -> Self {
        let project = workspace.project().clone();
        let git_store = project.read(cx).git_store().clone();
        let git_store_subscription = cx.subscribe(&git_store, Self::on_git_store_event);

        Self {
            workspace: workspace.weak_handle(),
            project,
            _git_store_subscription: git_store_subscription,
        }
    }

    fn on_git_store_event(
        &mut self,
        _git_store: Entity<GitStore>,
        event: &GitStoreEvent,
        cx: &mut Context<Self>,
    ) {
        if matches!(
            event,
            GitStoreEvent::ActiveRepositoryChanged(_)
                | GitStoreEvent::RepositoryUpdated(_, _, true)
        ) {
            cx.notify();
        }
    }
}

impl Render for BranchIndicator {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(repository) = self.project.read(cx).active_repository(cx) else {
            return Empty.into_any_element();
        };

        let (label, is_detached_head) = {
            let repository = repository.read(cx);
            match (repository.branch.as_ref(), repository.head_commit.as_ref()) {
                (Some(branch), _) => (
                    util::truncate_and_trailoff(branch.name(), MAX_BRANCH_NAME_LENGTH),
                    false,
                ),
                (None, Some(commit)) => (
                    commit
                        .sha
                        .chars()
                        .take(SHORT_SHA_LENGTH)
                        .collect::<String>(),
                    true,
                ),
                (None, None) => return Empty.into_any_element(),
            }
        };

        let region = ChromeRegion::StatusBar;
        let icon_size = ui::chrome_icon_size(region, IconSize::Small, cx);
        let workspace = self.workspace.clone();
        let tooltip_meta = if is_detached_head {
            format!("Detached HEAD: {label}")
        } else {
            format!("Currently Checked Out: {label}")
        };

        PopoverMenu::new("status-bar-branch-menu")
            .menu(move |window, cx| {
                git_ui_core::build_branch_picker(
                    workspace.clone(),
                    Some(repository.clone()),
                    window,
                    cx,
                )
            })
            .trigger_with_tooltip(
                Button::new("status-bar-branch-trigger", label)
                    .selected_style(ButtonStyle::Tinted(TintColor::Accent))
                    .label_size(LabelSize::Small)
                    .chrome_region(region)
                    .color(Color::Muted)
                    .tab_index(0isize)
                    .start_icon(
                        Icon::new(IconName::GitBranch)
                            .size(icon_size)
                            .color(Color::Muted),
                    ),
                move |_window, cx| {
                    Tooltip::with_meta(
                        "Branch & Stash",
                        Some(&zed_actions::git::Branch),
                        tooltip_meta.clone(),
                        cx,
                    )
                },
            )
            .anchor(Anchor::BottomLeft)
            .offset(gpui::point(px(0.), px(POPOVER_OFFSET_Y)))
            .into_any_element()
    }
}

impl StatusItemView for BranchIndicator {
    fn set_active_pane_item(
        &mut self,
        _: Option<&dyn ItemHandle>,
        _window: &mut Window,
        _: &mut Context<Self>,
    ) {
    }

    fn hide_setting(&self, _: &App) -> Option<HideStatusItem> {
        None
    }
}
