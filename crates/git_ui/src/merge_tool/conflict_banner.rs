#[cfg(test)]
mod tests;

use editor::Editor;
use git::repository::RepoPath;
use git::status::{FileStatus, UnmergedStatus, UnmergedStatusCode};
use gpui::{AnyElement, App, Context, Entity, EventEmitter, Subscription, WeakEntity, Window, px};
use project::{
    Project, ProjectPath,
    git_store::{GitStoreEvent, Repository},
};
use ui::prelude::*;
use workspace::{
    ItemHandle, Pane, ToolbarItemEvent, ToolbarItemLocation, ToolbarItemView, Workspace,
};

use crate::merge_tool::is_available;
use crate::merge_tool::merge_viewer::panes::{
    NOTIFICATION_BACKGROUND_DARK, NOTIFICATION_BACKGROUND_LIGHT, NOTIFICATION_BORDER_DARK,
    NOTIFICATION_BORDER_LIGHT, NOTIFICATION_FOREGROUND_DARK, NOTIFICATION_FOREGROUND_LIGHT,
    NOTIFICATION_HORIZONTAL_INSET, NOTIFICATION_ICON_GAP, NOTIFICATION_ICON_SIZE,
    NOTIFICATION_TEXT_TRAILING_GAP, NOTIFICATION_VERTICAL_INSET,
};
use crate::merge_tool::merge_window::chrome::{ThemedImage, themed_image};
use crate::merge_tool::palette::{current_palette, hex_color};
use crate::merge_tool::single_file_merge::{
    cancel_active_merge_window, has_active_merge_window, observe_active_merge_windows,
    open_single_file_merge, show_active_merge_window,
};

const RESOLVE_SUGGESTED_TEXT: &str = "File has unresolved merge conflicts";
const RESOLVE_IN_PROGRESS_TEXT: &str = "Resolving merge conflicts is in progress";
const RESOLVE_CONFLICTS_LABEL: &str = "Resolve conflicts\u{2026}";
const SHOW_WINDOW_LABEL: &str = "Show resolve conflicts window";
const CANCEL_RESOLVE_LABEL: &str = "Cancel resolve";
const NOTIFICATION_LINK_GAP: f32 = 16.0;
const INFO_BACKGROUND_DARK: u32 = 0x233558;
const INFO_BORDER_DARK: u32 = 0x2E4D89;
const INFO_BACKGROUND_LIGHT: u32 = 0xF7F8FF;
const INFO_BORDER_LIGHT: u32 = 0xBDD3FF;

pub(crate) fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, window, cx| {
        let Some(window) = window else {
            return;
        };
        if !is_available(workspace.project().read(cx), cx) {
            return;
        }
        for pane in workspace.panes().to_vec() {
            add_banner_to_pane(workspace, &pane, window, cx);
        }
        let workspace_entity = cx.entity();
        cx.subscribe_in(
            &workspace_entity,
            window,
            |workspace, _, event, window, cx| {
                if let workspace::Event::PaneAdded(pane) = event {
                    add_banner_to_pane(workspace, pane, window, cx);
                }
            },
        )
        .detach();
    })
    .detach();
}

fn add_banner_to_pane(
    workspace: &Workspace,
    pane: &Entity<Pane>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let workspace_handle = workspace.weak_handle();
    let project = workspace.project().clone();
    pane.update(cx, |pane, cx| {
        pane.toolbar().update(cx, |toolbar, cx| {
            let banner = cx.new(|cx| ConflictBanner::new(workspace_handle, project, cx));
            toolbar.add_item(banner, window, cx);
        });
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BannerKind {
    ResolveSuggested,
    ResolveInProgress,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BannerLink {
    ResolveConflicts,
    ShowWindow,
    CancelResolve,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BannerColors {
    pub background: u32,
    pub border: u32,
    pub foreground: u32,
}

impl BannerKind {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::ResolveSuggested => RESOLVE_SUGGESTED_TEXT,
            Self::ResolveInProgress => RESOLVE_IN_PROGRESS_TEXT,
        }
    }

    pub(crate) fn links(self) -> &'static [BannerLink] {
        match self {
            Self::ResolveSuggested => &[BannerLink::ResolveConflicts],
            Self::ResolveInProgress => &[BannerLink::ShowWindow, BannerLink::CancelResolve],
        }
    }

    pub(crate) fn colors(self, dark: bool) -> BannerColors {
        match (self, dark) {
            (Self::ResolveSuggested, true) => BannerColors {
                background: NOTIFICATION_BACKGROUND_DARK,
                border: NOTIFICATION_BORDER_DARK,
                foreground: NOTIFICATION_FOREGROUND_DARK,
            },
            (Self::ResolveSuggested, false) => BannerColors {
                background: NOTIFICATION_BACKGROUND_LIGHT,
                border: NOTIFICATION_BORDER_LIGHT,
                foreground: NOTIFICATION_FOREGROUND_LIGHT,
            },
            (Self::ResolveInProgress, true) => BannerColors {
                background: INFO_BACKGROUND_DARK,
                border: INFO_BORDER_DARK,
                foreground: NOTIFICATION_FOREGROUND_DARK,
            },
            (Self::ResolveInProgress, false) => BannerColors {
                background: INFO_BACKGROUND_LIGHT,
                border: INFO_BORDER_LIGHT,
                foreground: NOTIFICATION_FOREGROUND_LIGHT,
            },
        }
    }

    fn image(self) -> ThemedImage {
        match self {
            Self::ResolveSuggested => ThemedImage::BannerWarning,
            Self::ResolveInProgress => ThemedImage::BannerInfo,
        }
    }
}

impl BannerLink {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::ResolveConflicts => RESOLVE_CONFLICTS_LABEL,
            Self::ShowWindow => SHOW_WINDOW_LABEL,
            Self::CancelResolve => CANCEL_RESOLVE_LABEL,
        }
    }

    fn element_id(self) -> &'static str {
        match self {
            Self::ResolveConflicts => "merge-conflict-banner-resolve",
            Self::ShowWindow => "merge-conflict-banner-show-window",
            Self::CancelResolve => "merge-conflict-banner-cancel-resolve",
        }
    }
}

pub(crate) fn can_resolve_conflict(status: FileStatus) -> bool {
    match status {
        FileStatus::Unmerged(UnmergedStatus {
            first_head: UnmergedStatusCode::Deleted,
            second_head: UnmergedStatusCode::Deleted,
        }) => false,
        FileStatus::Unmerged(_) => true,
        FileStatus::Untracked | FileStatus::Ignored | FileStatus::Tracked(_) => false,
    }
}

pub(crate) fn banner_kind(
    has_active_merge_window: bool,
    status: Option<FileStatus>,
) -> Option<BannerKind> {
    if has_active_merge_window {
        return Some(BannerKind::ResolveInProgress);
    }
    status
        .is_some_and(can_resolve_conflict)
        .then_some(BannerKind::ResolveSuggested)
}

fn location_for(kind: Option<BannerKind>) -> ToolbarItemLocation {
    match kind {
        Some(_) => ToolbarItemLocation::Secondary,
        None => ToolbarItemLocation::Hidden,
    }
}

#[derive(Clone)]
struct ConflictPanel {
    kind: BannerKind,
    repository: Entity<Repository>,
    repo_path: RepoPath,
}

pub(crate) struct ConflictBanner {
    workspace: WeakEntity<Workspace>,
    project: Entity<Project>,
    project_path: Option<ProjectPath>,
    panel: Option<ConflictPanel>,
    _subscriptions: Vec<Subscription>,
}

impl ConflictBanner {
    pub(crate) fn new(
        workspace: WeakEntity<Workspace>,
        project: Entity<Project>,
        cx: &mut Context<Self>,
    ) -> Self {
        let git_store = project.read(cx).git_store().clone();
        let subscriptions = vec![
            cx.subscribe(&git_store, |banner, _, _: &GitStoreEvent, cx| {
                banner.refresh(cx);
            }),
            observe_active_merge_windows(cx, |banner, cx| banner.refresh(cx)),
        ];
        Self {
            workspace,
            project,
            project_path: None,
            panel: None,
            _subscriptions: subscriptions,
        }
    }

    #[cfg(test)]
    pub(crate) fn kind(&self) -> Option<BannerKind> {
        self.panel.as_ref().map(|panel| panel.kind)
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let location = self.update_panel(cx);
        cx.emit(ToolbarItemEvent::ChangeLocation(location));
        cx.notify();
    }

    fn update_panel(&mut self, cx: &App) -> ToolbarItemLocation {
        self.panel = self.resolve_panel(cx);
        location_for(self.panel.as_ref().map(|panel| panel.kind))
    }

    fn resolve_panel(&self, cx: &App) -> Option<ConflictPanel> {
        let project_path = self.project_path.as_ref()?;
        let project = self.project.read(cx);
        let (repository, repo_path) = project
            .git_store()
            .read(cx)
            .repository_and_path_for_project_path(project_path, cx)?;
        let repository_state = repository.read(cx);
        let status = repository_state
            .status_for_path(&repo_path)
            .map(|entry| entry.status);
        let has_active_window = has_active_merge_window(repository_state.id, &repo_path, cx);
        let kind = banner_kind(has_active_window, status)?;
        Some(ConflictPanel {
            kind,
            repository,
            repo_path,
        })
    }

    fn activate(&mut self, link: BannerLink, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = self.panel.clone() else {
            return;
        };
        let repository_id = panel.repository.read(cx).id;
        match link {
            BannerLink::ResolveConflicts => open_single_file_merge(
                self.workspace.clone(),
                panel.repository,
                panel.repo_path,
                window,
                cx,
            ),
            BannerLink::ShowWindow => show_active_merge_window(repository_id, &panel.repo_path, cx),
            BannerLink::CancelResolve => {
                cancel_active_merge_window(repository_id, &panel.repo_path, cx)
            }
        }
    }

    fn render_link(&self, link: BannerLink, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id(link.element_id())
            .flex_none()
            .cursor_pointer()
            .text_color(cx.theme().colors().text_accent)
            .child(link.label())
            .on_click(cx.listener(move |banner, _, window, cx| {
                banner.activate(link, window, cx);
            }))
            .into_any_element()
    }
}

fn single_file_editor_path(
    item: &dyn ItemHandle,
    project: &Project,
    cx: &App,
) -> Option<ProjectPath> {
    let editor = item.act_as::<Editor>(cx)?;
    if !editor.read(cx).buffer().read(cx).is_singleton() {
        return None;
    }
    let project_path = item.project_path(cx)?;
    project
        .entry_for_path(&project_path, cx)
        .is_some_and(|entry| entry.is_file())
        .then_some(project_path)
}

impl EventEmitter<ToolbarItemEvent> for ConflictBanner {}

impl ToolbarItemView for ConflictBanner {
    fn set_active_pane_item(
        &mut self,
        active_pane_item: Option<&dyn ItemHandle>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ToolbarItemLocation {
        let project = self.project.clone();
        self.project_path =
            active_pane_item.and_then(|item| single_file_editor_path(item, project.read(cx), cx));
        self.update_panel(cx)
    }
}

impl Render for ConflictBanner {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(kind) = self.panel.as_ref().map(|panel| panel.kind) else {
            return div().into_any_element();
        };
        let dark = current_palette(cx).kind.is_dark();
        let colors = kind.colors(dark);
        let links = kind
            .links()
            .iter()
            .map(|link| self.render_link(*link, cx))
            .collect::<Vec<_>>();
        h_flex()
            .w_full()
            .flex_none()
            .items_center()
            .px(px(NOTIFICATION_HORIZONTAL_INSET))
            .py(px(NOTIFICATION_VERTICAL_INSET))
            .bg(hex_color(colors.background))
            .border_t_1()
            .border_b_1()
            .border_color(hex_color(colors.border))
            .text_color(hex_color(colors.foreground))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(NOTIFICATION_ICON_GAP))
                    .pr(px(NOTIFICATION_TEXT_TRAILING_GAP))
                    .items_center()
                    .child(themed_image(kind.image(), NOTIFICATION_ICON_SIZE, cx))
                    .child(
                        Label::new(kind.text()).color(Color::Custom(hex_color(colors.foreground))),
                    ),
            )
            .child(h_flex().gap(px(NOTIFICATION_LINK_GAP)).children(links))
            .into_any_element()
    }
}
