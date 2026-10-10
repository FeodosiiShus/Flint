use crate::{
    OpenInTerminal, OpenTerminal, SplitDirection, ToggleFileFinder, Workspace,
    item::{ItemBufferKind, ItemHandle},
    pane::{
        CloseActiveItem, CloseAllItems, CloseItemsToTheLeft, CloseItemsToTheRight, CloseOtherItems,
        Event as PaneEvent, JoinAll, JoinIntoNext, Pane, ReopenClosedItem, SaveIntent,
        SplitAndMoveDown, SplitAndMoveRight, SplitDown, SplitMode, SplitRight, TogglePinTab,
    },
};
use git::{CopyFilePermalink, OpenFilePermalink};
use gpui::{
    Action, App, ClipboardItem, Context, Entity, EntityId, FocusHandle, SharedString, TaskExt,
    WeakEntity, Window,
};
use project::{Project, ProjectPath};
use std::{path::PathBuf, rc::Rc};
use ui::{ContextMenu, ContextMenuEntry};
use zed_actions::search_everywhere;

pub(crate) const TAB_BAR_MORE_MENU_TOOLTIP: &str = "Recent Files, Tab Actions, and More";
const COPY_PATH_SUBMENU_LABEL: &str = "Copy Path/Reference…";
const OPEN_IN_SUBMENU_LABEL: &str = "Open In";
const GIT_SUBMENU_LABEL: &str = "Git";
const CONFIGURE_EDITOR_TABS_SETTING_PATH: &str = "tab_bar.show";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabMenuCommand {
    Close,
    CloseOtherTabs,
    CloseAllTabs,
    CloseAllButPinned,
    CloseTabsToTheLeft,
    CloseTabsToTheRight,
    CopyAbsolutePath,
    CopyFileName,
    CopyContentRootPath,
    CopyRepositoryRootPath,
    SplitRight,
    SplitAndMoveRight,
    SplitDown,
    SplitAndMoveDown,
    Unsplit,
    UnsplitAll,
    PinTab,
    UnpinTab,
    KeepTabOpen,
    ConfigureEditorTabs,
    ReopenClosedTab,
    RevealInFinder,
    OpenInTerminal,
    OpenFilePermalink,
    CopyFilePermalink,
    RecentFiles,
    GoToFile,
}

impl TabMenuCommand {
    fn action(self) -> Option<Box<dyn Action>> {
        match self {
            Self::Close => Some(Box::new(CloseActiveItem {
                save_intent: None,
                close_pinned: true,
            })),
            Self::CloseOtherTabs => Some(Box::new(close_other_tabs_action())),
            Self::CloseAllTabs => Some(Box::new(close_all_tabs_action(true))),
            Self::CloseAllButPinned => Some(Box::new(close_all_tabs_action(false))),
            Self::CloseTabsToTheLeft => Some(Box::new(CloseItemsToTheLeft {
                close_pinned: false,
            })),
            Self::CloseTabsToTheRight => Some(Box::new(CloseItemsToTheRight {
                close_pinned: false,
            })),
            Self::CopyAbsolutePath => Some(Box::new(zed_actions::workspace::CopyPath)),
            Self::CopyContentRootPath => Some(Box::new(zed_actions::workspace::CopyRelativePath)),
            Self::CopyFileName | Self::CopyRepositoryRootPath | Self::KeepTabOpen => None,
            Self::SplitRight => Some(Box::new(SplitRight::default())),
            Self::SplitAndMoveRight => Some(Box::new(SplitAndMoveRight)),
            Self::SplitDown => Some(Box::new(SplitDown::default())),
            Self::SplitAndMoveDown => Some(Box::new(SplitAndMoveDown)),
            Self::Unsplit => Some(Box::new(JoinIntoNext)),
            Self::UnsplitAll => Some(Box::new(JoinAll)),
            Self::PinTab | Self::UnpinTab => Some(Box::new(TogglePinTab)),
            Self::ConfigureEditorTabs => Some(Box::new(configure_editor_tabs_action())),
            Self::ReopenClosedTab => Some(Box::new(ReopenClosedItem)),
            Self::RevealInFinder => Some(Box::new(zed_actions::editor::RevealInFileManager)),
            Self::OpenInTerminal => Some(Box::new(OpenInTerminal)),
            Self::OpenFilePermalink => Some(Box::new(OpenFilePermalink)),
            Self::CopyFilePermalink => Some(Box::new(CopyFilePermalink)),
            Self::RecentFiles => Some(Box::new(ToggleFileFinder::default())),
            Self::GoToFile => Some(Box::new(go_to_file_action())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TabMenuItem {
    Entry {
        label: &'static str,
        command: TabMenuCommand,
        enabled: bool,
    },
    Separator,
    Submenu {
        label: &'static str,
        items: Vec<TabMenuItem>,
    },
}

impl TabMenuItem {
    fn entry(label: &'static str, command: TabMenuCommand) -> Self {
        Self::Entry {
            label,
            command,
            enabled: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TabFileSnapshot {
    has_content_root_path: bool,
    has_repository_path: bool,
    is_local: bool,
    has_parent_directory: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TabMenuSnapshot {
    tab_count: usize,
    pinned_tab_count: usize,
    clicked_index: usize,
    is_preview: bool,
    can_split: bool,
    split_pane_count: usize,
    file: Option<TabFileSnapshot>,
}

pub(crate) fn tab_context_menu_items(snapshot: &TabMenuSnapshot) -> Vec<TabMenuItem> {
    let is_split = snapshot.split_pane_count > 1;
    let pinned_tab_count = snapshot.pinned_tab_count.min(snapshot.tab_count);
    let has_several_tabs = snapshot.tab_count > 1;
    let is_clicked_tab_pinned = snapshot.clicked_index < pinned_tab_count;
    let has_unpinned_tab_to_the_left = snapshot.clicked_index > pinned_tab_count;
    let has_unpinned_tab_to_the_right =
        (snapshot.clicked_index + 1).max(pinned_tab_count) < snapshot.tab_count;

    let mut items = vec![
        TabMenuItem::entry("Close", TabMenuCommand::Close),
        TabMenuItem::Entry {
            label: "Close Other Tabs",
            command: TabMenuCommand::CloseOtherTabs,
            enabled: has_several_tabs,
        },
        TabMenuItem::entry(close_all_tabs_label(is_split), TabMenuCommand::CloseAllTabs),
    ];
    if pinned_tab_count > 0 && pinned_tab_count < snapshot.tab_count {
        items.push(TabMenuItem::entry(
            if is_split {
                "Close All but Pinned In Group"
            } else {
                "Close All but Pinned"
            },
            TabMenuCommand::CloseAllButPinned,
        ));
    }
    if has_unpinned_tab_to_the_left {
        items.push(TabMenuItem::entry(
            "Close Tabs to the Left",
            TabMenuCommand::CloseTabsToTheLeft,
        ));
    }
    if has_unpinned_tab_to_the_right {
        items.push(TabMenuItem::entry(
            "Close Tabs to the Right",
            TabMenuCommand::CloseTabsToTheRight,
        ));
    }

    items.push(TabMenuItem::Separator);
    if let Some(file) = &snapshot.file {
        items.push(TabMenuItem::Submenu {
            label: COPY_PATH_SUBMENU_LABEL,
            items: copy_path_items(file),
        });
    }

    items.push(TabMenuItem::Separator);
    if snapshot.can_split {
        items.push(TabMenuItem::entry(
            "Split Right",
            TabMenuCommand::SplitRight,
        ));
    }
    if has_several_tabs {
        items.push(TabMenuItem::entry(
            "Split and Move Right",
            TabMenuCommand::SplitAndMoveRight,
        ));
    }
    if snapshot.can_split {
        items.push(TabMenuItem::entry("Split Down", TabMenuCommand::SplitDown));
    }
    if has_several_tabs {
        items.push(TabMenuItem::entry(
            "Split and Move Down",
            TabMenuCommand::SplitAndMoveDown,
        ));
    }
    items.extend(unsplit_items(snapshot.split_pane_count));

    items.push(TabMenuItem::Separator);
    items.push(if is_clicked_tab_pinned {
        TabMenuItem::entry("Unpin Tab", TabMenuCommand::UnpinTab)
    } else {
        TabMenuItem::entry("Pin Tab", TabMenuCommand::PinTab)
    });
    if snapshot.is_preview {
        items.push(TabMenuItem::entry(
            "Keep Tab Open",
            TabMenuCommand::KeepTabOpen,
        ));
    }
    items.push(TabMenuItem::entry(
        "Configure Editor Tabs…",
        TabMenuCommand::ConfigureEditorTabs,
    ));

    items.push(TabMenuItem::Separator);
    items.push(TabMenuItem::entry(
        "Reopen Closed Tab",
        TabMenuCommand::ReopenClosedTab,
    ));

    if let Some(file) = &snapshot.file {
        items.push(TabMenuItem::Separator);
        if file.is_local {
            items.push(TabMenuItem::Submenu {
                label: OPEN_IN_SUBMENU_LABEL,
                items: open_in_items(file),
            });
        }
        items.push(TabMenuItem::Separator);
        if file.has_repository_path {
            items.push(TabMenuItem::Submenu {
                label: GIT_SUBMENU_LABEL,
                items: vec![
                    TabMenuItem::entry("Open File Permalink", TabMenuCommand::OpenFilePermalink),
                    TabMenuItem::entry("Copy File Permalink", TabMenuCommand::CopyFilePermalink),
                ],
            });
        }
    }

    normalized(items)
}

pub(crate) fn tab_bar_more_menu_items(split_pane_count: usize) -> Vec<TabMenuItem> {
    let mut items = vec![
        TabMenuItem::entry("Recent Files", TabMenuCommand::RecentFiles),
        TabMenuItem::entry("Go to File…", TabMenuCommand::GoToFile),
        TabMenuItem::Separator,
        TabMenuItem::entry(
            close_all_tabs_label(split_pane_count > 1),
            TabMenuCommand::CloseAllTabs,
        ),
        TabMenuItem::entry("Reopen Closed Tab", TabMenuCommand::ReopenClosedTab),
        TabMenuItem::Separator,
    ];
    items.extend(unsplit_items(split_pane_count));
    items.push(TabMenuItem::Separator);
    items.push(TabMenuItem::entry(
        "Configure Editor Tabs…",
        TabMenuCommand::ConfigureEditorTabs,
    ));
    normalized(items)
}

fn close_all_tabs_label(is_split: bool) -> &'static str {
    if is_split {
        "Close All Tabs In Group"
    } else {
        "Close All Tabs"
    }
}

fn unsplit_items(split_pane_count: usize) -> Vec<TabMenuItem> {
    let mut items = Vec::new();
    if split_pane_count > 1 {
        items.push(TabMenuItem::entry("Unsplit", TabMenuCommand::Unsplit));
    }
    if split_pane_count > 2 {
        items.push(TabMenuItem::entry(
            "Unsplit All",
            TabMenuCommand::UnsplitAll,
        ));
    }
    items
}

fn copy_path_items(file: &TabFileSnapshot) -> Vec<TabMenuItem> {
    let mut items = vec![
        TabMenuItem::entry("Absolute Path", TabMenuCommand::CopyAbsolutePath),
        TabMenuItem::entry("File Name", TabMenuCommand::CopyFileName),
        TabMenuItem::Separator,
    ];
    if file.has_content_root_path {
        items.push(TabMenuItem::entry(
            "Path from Content Root",
            TabMenuCommand::CopyContentRootPath,
        ));
    }
    if file.has_repository_path {
        items.push(TabMenuItem::entry(
            "Path from Repository Root",
            TabMenuCommand::CopyRepositoryRootPath,
        ));
    }
    items
}

fn open_in_items(file: &TabFileSnapshot) -> Vec<TabMenuItem> {
    let mut items = vec![TabMenuItem::entry("Finder", TabMenuCommand::RevealInFinder)];
    if file.has_parent_directory {
        items.push(TabMenuItem::entry(
            "Terminal",
            TabMenuCommand::OpenInTerminal,
        ));
    }
    items
}

fn normalized(items: Vec<TabMenuItem>) -> Vec<TabMenuItem> {
    let mut result: Vec<TabMenuItem> = Vec::with_capacity(items.len());
    for item in items {
        match item {
            TabMenuItem::Separator => {
                if result
                    .last()
                    .is_some_and(|last| !matches!(last, TabMenuItem::Separator))
                {
                    result.push(TabMenuItem::Separator);
                }
            }
            TabMenuItem::Submenu { label, items } => {
                let items = normalized(items);
                if !items.is_empty() {
                    result.push(TabMenuItem::Submenu { label, items });
                }
            }
            entry @ TabMenuItem::Entry { .. } => result.push(entry),
        }
    }
    if matches!(result.last(), Some(TabMenuItem::Separator)) {
        result.pop();
    }
    result
}

fn close_other_tabs_action() -> CloseOtherItems {
    CloseOtherItems {
        save_intent: None,
        close_pinned: false,
    }
}

fn close_all_tabs_action(close_pinned: bool) -> CloseAllItems {
    CloseAllItems {
        save_intent: None,
        close_pinned,
    }
}

fn configure_editor_tabs_action() -> zed_actions::OpenSettingsAt {
    zed_actions::OpenSettingsAt {
        path: CONFIGURE_EDITOR_TABS_SETTING_PATH.to_string(),
        target: None,
    }
}

fn go_to_file_action() -> search_everywhere::Toggle {
    search_everywhere::Toggle {
        tab: Some(search_everywhere::Tab::Files),
    }
}

pub(crate) fn build_tab_context_menu(
    pane: &WeakEntity<Pane>,
    item_id: EntityId,
    menu_context: FocusHandle,
    extra_actions: Vec<(SharedString, Box<dyn Action>)>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ContextMenu> {
    let state = pane
        .upgrade()
        .and_then(|pane| tab_menu_state(&pane, item_id, cx));
    let (items, target) = match state {
        Some((snapshot, target)) => (tab_context_menu_items(&snapshot), Some(Rc::new(target))),
        None => (Vec::new(), None),
    };
    ContextMenu::build(window, cx, move |menu, _, _| {
        let menu = match &target {
            Some(target) => append_menu_items(menu, &items, target),
            None => menu,
        };
        let menu = if extra_actions.is_empty() {
            menu
        } else {
            extra_actions
                .into_iter()
                .fold(menu.separator(), |menu, (label, action)| {
                    menu.action(label, action)
                })
        };
        menu.context(menu_context)
    })
}

pub(crate) fn build_tab_bar_more_menu(
    pane: &WeakEntity<Pane>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<ContextMenu>> {
    let pane_entity = pane.upgrade()?;
    let items = tab_bar_more_menu_items(split_pane_count(&pane_entity, cx));
    let target = Rc::new(TabMenuTarget {
        pane: pane.clone(),
        workspace: pane_entity.read(cx).workspace.clone(),
        item_id: None,
        file: None,
    });
    Some(ContextMenu::build(window, cx, move |menu, _, _| {
        append_menu_items(menu, &items, &target)
    }))
}

fn append_menu_items(
    menu: ContextMenu,
    items: &[TabMenuItem],
    target: &Rc<TabMenuTarget>,
) -> ContextMenu {
    items.iter().fold(menu, |menu, item| match item {
        TabMenuItem::Separator => menu.separator(),
        TabMenuItem::Entry {
            label,
            command,
            enabled,
        } => {
            let command = *command;
            let target = target.clone();
            let entry = ContextMenuEntry::new(*label)
                .disabled(!*enabled)
                .handler(move |window, cx| target.run(command, window, cx));
            menu.item(match command.action() {
                Some(action) => entry.action(action),
                None => entry,
            })
        }
        TabMenuItem::Submenu { label, items } => {
            let items = items.clone();
            let target = target.clone();
            menu.submenu(*label, move |menu, _, _| {
                append_menu_items(menu, &items, &target)
            })
        }
    })
}

fn split_pane_count(pane: &Entity<Pane>, cx: &App) -> usize {
    pane.read(cx).workspace.upgrade().map_or(1, |workspace| {
        let panes = workspace.read(cx).panes();
        if panes.contains(pane) { panes.len() } else { 1 }
    })
}

fn tab_menu_state(
    pane: &Entity<Pane>,
    item_id: EntityId,
    cx: &App,
) -> Option<(TabMenuSnapshot, TabMenuTarget)> {
    let pane_state = pane.read(cx);
    let clicked_index = pane_state
        .items()
        .position(|item| item.item_id() == item_id)?;
    let item = pane_state.item_for_index(clicked_index)?;
    let file = tab_file_target(pane_state, item, cx);
    let tab_count = pane_state.items_len();
    let snapshot = TabMenuSnapshot {
        tab_count,
        pinned_tab_count: pane_state.pinned_count().min(tab_count),
        clicked_index,
        is_preview: pane_state.preview_item_id() == Some(item_id),
        can_split: item.can_split(cx),
        split_pane_count: split_pane_count(pane, cx),
        file: file.as_ref().map(TabFileTarget::snapshot),
    };
    let target = TabMenuTarget {
        pane: pane.downgrade(),
        workspace: pane_state.workspace.clone(),
        item_id: Some(item_id),
        file,
    };
    Some((snapshot, target))
}

fn tab_file_target(pane: &Pane, item: &dyn ItemHandle, cx: &App) -> Option<TabFileTarget> {
    if item.buffer_kind(cx) != ItemBufferKind::Singleton {
        return None;
    }
    let entry_id = item.project_entry_ids(cx).first().copied()?;
    let absolute_path = pane.entry_abs_path(entry_id, cx)?;
    let project_path = item.project_path(cx)?;
    let workspace = pane.workspace.upgrade()?;
    let project = workspace.read(cx).project().read(cx);
    let path_style = project.path_style(cx);
    let content_root_path = project
        .worktree_for_id(project_path.worktree_id, cx)
        .filter(|worktree| {
            worktree
                .read(cx)
                .root_entry()
                .is_some_and(|entry| entry.is_dir())
        })
        .map(|_| project_path.path.display(path_style).into_owned());
    let repository_root_path = project
        .git_store()
        .read(cx)
        .repository_and_path_for_project_path(&project_path, cx)
        .map(|(_, repository_path)| repository_path.display(path_style).into_owned());
    Some(TabFileTarget {
        project_path,
        absolute_path,
        content_root_path,
        repository_root_path,
        is_local: project.is_local(),
    })
}

struct TabFileTarget {
    project_path: ProjectPath,
    absolute_path: PathBuf,
    content_root_path: Option<String>,
    repository_root_path: Option<String>,
    is_local: bool,
}

impl TabFileTarget {
    fn snapshot(&self) -> TabFileSnapshot {
        TabFileSnapshot {
            has_content_root_path: self.content_root_path.is_some(),
            has_repository_path: self.repository_root_path.is_some(),
            is_local: self.is_local,
            has_parent_directory: self.absolute_path.parent().is_some(),
        }
    }
}

struct TabMenuTarget {
    pane: WeakEntity<Pane>,
    workspace: WeakEntity<Workspace>,
    item_id: Option<EntityId>,
    file: Option<TabFileTarget>,
}

impl TabMenuTarget {
    fn run(&self, command: TabMenuCommand, window: &mut Window, cx: &mut App) {
        match command {
            TabMenuCommand::Close => {
                self.update_clicked_item(window, cx, |pane, item_id, window, cx| {
                    pane.close_item_by_id(item_id, SaveIntent::Close, window, cx)
                        .detach_and_log_err(cx);
                })
            }
            TabMenuCommand::CloseOtherTabs => {
                self.update_clicked_item(window, cx, |pane, item_id, window, cx| {
                    pane.close_other_items(&close_other_tabs_action(), Some(item_id), window, cx)
                        .detach_and_log_err(cx);
                })
            }
            TabMenuCommand::CloseAllTabs => self.update_pane(window, cx, |pane, window, cx| {
                pane.close_all_items(&close_all_tabs_action(true), window, cx)
                    .detach_and_log_err(cx);
            }),
            TabMenuCommand::CloseAllButPinned => {
                self.update_pane(window, cx, |pane, window, cx| {
                    pane.close_all_items(&close_all_tabs_action(false), window, cx)
                        .detach_and_log_err(cx);
                })
            }
            TabMenuCommand::CloseTabsToTheLeft => {
                self.update_clicked_item(window, cx, |pane, item_id, window, cx| {
                    pane.close_items_to_the_left_by_id(
                        Some(item_id),
                        &CloseItemsToTheLeft {
                            close_pinned: false,
                        },
                        window,
                        cx,
                    )
                    .detach_and_log_err(cx);
                })
            }
            TabMenuCommand::CloseTabsToTheRight => {
                self.update_clicked_item(window, cx, |pane, item_id, window, cx| {
                    pane.close_items_to_the_right_by_id(
                        Some(item_id),
                        &CloseItemsToTheRight {
                            close_pinned: false,
                        },
                        window,
                        cx,
                    )
                    .detach_and_log_err(cx);
                })
            }
            TabMenuCommand::CopyAbsolutePath => self.copy_file_text(cx, |file| {
                Some(file.absolute_path.to_string_lossy().into_owned())
            }),
            TabMenuCommand::CopyFileName => self.copy_file_text(cx, |file| {
                file.absolute_path
                    .file_name()
                    .map(|file_name| file_name.to_string_lossy().into_owned())
            }),
            TabMenuCommand::CopyContentRootPath => {
                self.copy_file_text(cx, |file| file.content_root_path.clone())
            }
            TabMenuCommand::CopyRepositoryRootPath => {
                self.copy_file_text(cx, |file| file.repository_root_path.clone())
            }
            TabMenuCommand::SplitRight => {
                self.split_clicked_item(SplitDirection::Right, SplitMode::ClonePane, window, cx)
            }
            TabMenuCommand::SplitAndMoveRight => {
                self.split_clicked_item(SplitDirection::Right, SplitMode::MovePane, window, cx)
            }
            TabMenuCommand::SplitDown => {
                self.split_clicked_item(SplitDirection::Down, SplitMode::ClonePane, window, cx)
            }
            TabMenuCommand::SplitAndMoveDown => {
                self.split_clicked_item(SplitDirection::Down, SplitMode::MovePane, window, cx)
            }
            TabMenuCommand::Unsplit => {
                self.update_pane(window, cx, |_, _, cx| cx.emit(PaneEvent::JoinIntoNext))
            }
            TabMenuCommand::UnsplitAll => {
                self.update_pane(window, cx, |_, _, cx| cx.emit(PaneEvent::JoinAll))
            }
            TabMenuCommand::PinTab => {
                self.update_clicked_index(window, cx, |pane, index, window, cx| {
                    pane.pin_tab_at(index, window, cx)
                })
            }
            TabMenuCommand::UnpinTab => {
                self.update_clicked_index(window, cx, |pane, index, window, cx| {
                    pane.unpin_tab_at(index, window, cx)
                })
            }
            TabMenuCommand::KeepTabOpen => {
                self.update_clicked_item(window, cx, |pane, item_id, _, cx| {
                    pane.unpreview_item_if_preview(item_id);
                    cx.notify();
                })
            }
            TabMenuCommand::ReopenClosedTab => {
                if let Some(workspace) = self.workspace.upgrade() {
                    workspace.update(cx, |workspace, cx| {
                        workspace
                            .reopen_closed_item(window, cx)
                            .detach_and_log_err(cx);
                    });
                }
            }
            TabMenuCommand::RevealInFinder => {
                if let Some(file) = &self.file {
                    cx.reveal_path(&file.absolute_path);
                }
            }
            TabMenuCommand::OpenInTerminal => {
                if let Some(directory) = self
                    .file
                    .as_ref()
                    .and_then(|file| file.absolute_path.parent())
                {
                    window.dispatch_action(
                        Box::new(OpenTerminal {
                            working_directory: directory.to_path_buf(),
                            local: false,
                        }),
                        cx,
                    );
                }
            }
            TabMenuCommand::OpenFilePermalink => {
                if let Some((project, project_path)) = self.file_project(cx) {
                    crate::open_file_permalink(
                        project,
                        project_path,
                        self.workspace.clone(),
                        window,
                        cx,
                    );
                }
            }
            TabMenuCommand::CopyFilePermalink => {
                if let Some((project, project_path)) = self.file_project(cx) {
                    crate::copy_file_permalink(
                        project,
                        project_path,
                        self.workspace.clone(),
                        window,
                        cx,
                    );
                }
            }
            TabMenuCommand::ConfigureEditorTabs
            | TabMenuCommand::RecentFiles
            | TabMenuCommand::GoToFile => {
                if let Some(action) = command.action() {
                    window.dispatch_action(action, cx);
                }
            }
        }
    }

    fn update_pane(
        &self,
        window: &mut Window,
        cx: &mut App,
        update: impl FnOnce(&mut Pane, &mut Window, &mut Context<Pane>),
    ) {
        if let Some(pane) = self.pane.upgrade() {
            pane.update(cx, |pane, cx| update(pane, window, cx));
        }
    }

    fn update_clicked_item(
        &self,
        window: &mut Window,
        cx: &mut App,
        update: impl FnOnce(&mut Pane, EntityId, &mut Window, &mut Context<Pane>),
    ) {
        if let Some(item_id) = self.item_id {
            self.update_pane(window, cx, |pane, window, cx| {
                update(pane, item_id, window, cx)
            });
        }
    }

    fn update_clicked_index(
        &self,
        window: &mut Window,
        cx: &mut App,
        update: impl FnOnce(&mut Pane, usize, &mut Window, &mut Context<Pane>),
    ) {
        self.update_clicked_item(window, cx, |pane, item_id, window, cx| {
            if let Some(index) = pane.items().position(|item| item.item_id() == item_id) {
                update(pane, index, window, cx);
            }
        });
    }

    fn split_clicked_item(
        &self,
        direction: SplitDirection,
        mode: SplitMode,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.update_clicked_index(window, cx, |pane, index, window, cx| {
            pane.activate_item(index, true, true, window, cx);
            pane.split(direction, mode, window, cx);
        });
    }

    fn copy_file_text(&self, cx: &mut App, text: impl FnOnce(&TabFileTarget) -> Option<String>) {
        if let Some(text) = self.file.as_ref().and_then(text) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn file_project(&self, cx: &App) -> Option<(Entity<Project>, ProjectPath)> {
        let file = self.file.as_ref()?;
        let workspace = self.workspace.upgrade()?;
        let project = workspace.read(cx).project().clone();
        Some((project, file.project_path.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEPARATOR: &str = "---";

    fn labels(items: &[TabMenuItem]) -> Vec<&'static str> {
        items
            .iter()
            .map(|item| match item {
                TabMenuItem::Entry { label, .. } | TabMenuItem::Submenu { label, .. } => *label,
                TabMenuItem::Separator => SEPARATOR,
            })
            .collect()
    }

    fn submenu<'a>(items: &'a [TabMenuItem], submenu_label: &str) -> Option<&'a [TabMenuItem]> {
        items.iter().find_map(|item| match item {
            TabMenuItem::Submenu { label, items } if *label == submenu_label => {
                Some(items.as_slice())
            }
            _ => None,
        })
    }

    fn entry_enabled(items: &[TabMenuItem], entry_label: &str) -> Option<bool> {
        items.iter().find_map(|item| match item {
            TabMenuItem::Entry { label, enabled, .. } if *label == entry_label => Some(*enabled),
            _ => None,
        })
    }

    fn single_tab() -> TabMenuSnapshot {
        TabMenuSnapshot {
            tab_count: 1,
            pinned_tab_count: 0,
            clicked_index: 0,
            is_preview: false,
            can_split: true,
            split_pane_count: 1,
            file: None,
        }
    }

    fn local_file() -> TabFileSnapshot {
        TabFileSnapshot {
            has_content_root_path: true,
            has_repository_path: false,
            is_local: true,
            has_parent_directory: true,
        }
    }

    fn assert_well_formed(items: &[TabMenuItem]) {
        assert!(
            !matches!(items.first(), Some(TabMenuItem::Separator)),
            "a menu never starts with a separator: {:?}",
            labels(items)
        );
        assert!(
            !matches!(items.last(), Some(TabMenuItem::Separator)),
            "a menu never ends with a separator: {:?}",
            labels(items)
        );
        for pair in items.windows(2) {
            assert!(
                !matches!(pair, [TabMenuItem::Separator, TabMenuItem::Separator]),
                "a menu never shows two separators in a row: {:?}",
                labels(items)
            );
        }
        for item in items {
            if let TabMenuItem::Submenu { items, .. } = item {
                assert!(!items.is_empty(), "a submenu is never empty");
                assert_well_formed(items);
            }
        }
    }

    #[test]
    fn tab_context_menu_single_unpinned_tab_follows_intellij_order() {
        let items = tab_context_menu_items(&single_tab());
        assert_eq!(
            labels(&items),
            vec![
                "Close",
                "Close Other Tabs",
                "Close All Tabs",
                SEPARATOR,
                "Split Right",
                "Split Down",
                SEPARATOR,
                "Pin Tab",
                "Configure Editor Tabs…",
                SEPARATOR,
                "Reopen Closed Tab",
            ]
        );
        assert_eq!(
            entry_enabled(&items, "Close Other Tabs"),
            Some(false),
            "Close Other Tabs stays visible but is disabled with a single tab"
        );
        assert_well_formed(&items);
    }

    #[test]
    fn tab_context_menu_close_other_tabs_is_enabled_with_several_tabs() {
        let items = tab_context_menu_items(&TabMenuSnapshot {
            tab_count: 2,
            ..single_tab()
        });
        assert_eq!(entry_enabled(&items, "Close Other Tabs"), Some(true));
    }

    #[test]
    fn tab_context_menu_shows_close_all_but_pinned_only_with_pinned_and_unpinned_tabs() {
        let mixed = tab_context_menu_items(&TabMenuSnapshot {
            tab_count: 3,
            pinned_tab_count: 1,
            clicked_index: 1,
            ..single_tab()
        });
        assert!(labels(&mixed).contains(&"Close All but Pinned"));

        let all_pinned = tab_context_menu_items(&TabMenuSnapshot {
            tab_count: 2,
            pinned_tab_count: 2,
            ..single_tab()
        });
        assert!(!labels(&all_pinned).contains(&"Close All but Pinned"));

        let none_pinned = tab_context_menu_items(&TabMenuSnapshot {
            tab_count: 2,
            ..single_tab()
        });
        assert!(!labels(&none_pinned).contains(&"Close All but Pinned"));
    }

    #[test]
    fn tab_context_menu_left_and_right_entries_follow_unpinned_neighbors() {
        let first = tab_context_menu_items(&TabMenuSnapshot {
            tab_count: 3,
            clicked_index: 0,
            ..single_tab()
        });
        assert!(!labels(&first).contains(&"Close Tabs to the Left"));
        assert!(labels(&first).contains(&"Close Tabs to the Right"));

        let last = tab_context_menu_items(&TabMenuSnapshot {
            tab_count: 3,
            clicked_index: 2,
            ..single_tab()
        });
        assert!(labels(&last).contains(&"Close Tabs to the Left"));
        assert!(!labels(&last).contains(&"Close Tabs to the Right"));

        let middle = tab_context_menu_items(&TabMenuSnapshot {
            tab_count: 3,
            clicked_index: 1,
            ..single_tab()
        });
        assert_eq!(
            labels(&middle)[..5],
            [
                "Close",
                "Close Other Tabs",
                "Close All Tabs",
                "Close Tabs to the Left",
                "Close Tabs to the Right",
            ]
        );

        let first_unpinned_after_pinned = tab_context_menu_items(&TabMenuSnapshot {
            tab_count: 3,
            pinned_tab_count: 2,
            clicked_index: 2,
            ..single_tab()
        });
        assert!(
            !labels(&first_unpinned_after_pinned).contains(&"Close Tabs to the Left"),
            "only pinned tabs sit to the left, so nothing there can be closed"
        );

        let pinned_before_unpinned = tab_context_menu_items(&TabMenuSnapshot {
            tab_count: 3,
            pinned_tab_count: 2,
            clicked_index: 0,
            ..single_tab()
        });
        assert!(!labels(&pinned_before_unpinned).contains(&"Close Tabs to the Left"));
        assert!(
            labels(&pinned_before_unpinned).contains(&"Close Tabs to the Right"),
            "an unpinned tab sits to the right of the pinned neighbor"
        );

        let last_pinned = tab_context_menu_items(&TabMenuSnapshot {
            tab_count: 2,
            pinned_tab_count: 2,
            clicked_index: 0,
            ..single_tab()
        });
        assert!(!labels(&last_pinned).contains(&"Close Tabs to the Right"));
        assert!(labels(&last_pinned).contains(&"Unpin Tab"));
    }

    #[test]
    fn tab_context_menu_split_entries_follow_split_ability_and_tab_count() {
        let cannot_split = tab_context_menu_items(&TabMenuSnapshot {
            can_split: false,
            ..single_tab()
        });
        for label in [
            "Split Right",
            "Split and Move Right",
            "Split Down",
            "Split and Move Down",
        ] {
            assert!(!labels(&cannot_split).contains(&label));
        }

        let two_tabs = tab_context_menu_items(&TabMenuSnapshot {
            tab_count: 2,
            ..single_tab()
        });
        let two_tab_labels = labels(&two_tabs);
        let split_right = two_tab_labels
            .iter()
            .position(|label| *label == "Split Right")
            .expect("Split Right is shown for a splittable tab");
        assert_eq!(
            two_tab_labels[split_right..split_right + 4],
            [
                "Split Right",
                "Split and Move Right",
                "Split Down",
                "Split and Move Down",
            ]
        );

        let two_tabs_cannot_split = tab_context_menu_items(&TabMenuSnapshot {
            tab_count: 2,
            can_split: false,
            ..single_tab()
        });
        let labels_without_split = labels(&two_tabs_cannot_split);
        assert!(!labels_without_split.contains(&"Split Right"));
        assert!(labels_without_split.contains(&"Split and Move Right"));
        assert!(labels_without_split.contains(&"Split and Move Down"));
    }

    #[test]
    fn tab_context_menu_unsplit_entries_and_group_labels_follow_pane_count() {
        let one_pane = labels(&tab_context_menu_items(&single_tab()));
        assert!(!one_pane.contains(&"Unsplit"));
        assert!(!one_pane.contains(&"Unsplit All"));
        assert!(one_pane.contains(&"Close All Tabs"));

        let two_panes = labels(&tab_context_menu_items(&TabMenuSnapshot {
            split_pane_count: 2,
            tab_count: 2,
            pinned_tab_count: 1,
            clicked_index: 1,
            ..single_tab()
        }));
        assert!(two_panes.contains(&"Unsplit"));
        assert!(!two_panes.contains(&"Unsplit All"));
        assert!(two_panes.contains(&"Close All Tabs In Group"));
        assert!(two_panes.contains(&"Close All but Pinned In Group"));
        assert!(!two_panes.contains(&"Close All Tabs"));

        let three_panes = labels(&tab_context_menu_items(&TabMenuSnapshot {
            split_pane_count: 3,
            ..single_tab()
        }));
        let unsplit = three_panes
            .iter()
            .position(|label| *label == "Unsplit")
            .expect("Unsplit is shown while split");
        assert_eq!(three_panes[unsplit + 1], "Unsplit All");
        assert_eq!(
            three_panes[unsplit - 1],
            "Split Down",
            "Unsplit follows the split entries without a separator"
        );
    }

    #[test]
    fn tab_context_menu_copy_path_submenu_lists_file_references() {
        let with_content_root = tab_context_menu_items(&TabMenuSnapshot {
            file: Some(local_file()),
            ..single_tab()
        });
        let copy_path = submenu(&with_content_root, COPY_PATH_SUBMENU_LABEL)
            .expect("file tabs offer the Copy Path/Reference submenu");
        assert_eq!(
            labels(copy_path),
            vec![
                "Absolute Path",
                "File Name",
                SEPARATOR,
                "Path from Content Root"
            ]
        );
        assert_eq!(
            labels(&with_content_root)[..5],
            [
                "Close",
                "Close Other Tabs",
                "Close All Tabs",
                SEPARATOR,
                COPY_PATH_SUBMENU_LABEL
            ]
        );

        let single_file_worktree = tab_context_menu_items(&TabMenuSnapshot {
            file: Some(TabFileSnapshot {
                has_content_root_path: false,
                ..local_file()
            }),
            ..single_tab()
        });
        let copy_path = submenu(&single_file_worktree, COPY_PATH_SUBMENU_LABEL)
            .expect("file tabs offer the Copy Path/Reference submenu");
        assert_eq!(labels(copy_path), vec!["Absolute Path", "File Name"]);

        let in_repository = tab_context_menu_items(&TabMenuSnapshot {
            file: Some(TabFileSnapshot {
                has_repository_path: true,
                ..local_file()
            }),
            ..single_tab()
        });
        let copy_path = submenu(&in_repository, COPY_PATH_SUBMENU_LABEL)
            .expect("file tabs offer the Copy Path/Reference submenu");
        assert_eq!(
            labels(copy_path),
            vec![
                "Absolute Path",
                "File Name",
                SEPARATOR,
                "Path from Content Root",
                "Path from Repository Root",
            ]
        );

        let without_file = tab_context_menu_items(&single_tab());
        assert!(submenu(&without_file, COPY_PATH_SUBMENU_LABEL).is_none());
    }

    #[test]
    fn tab_context_menu_open_in_and_git_submenus_follow_file_state() {
        let local_in_repository = tab_context_menu_items(&TabMenuSnapshot {
            file: Some(TabFileSnapshot {
                has_repository_path: true,
                ..local_file()
            }),
            ..single_tab()
        });
        let open_in = submenu(&local_in_repository, OPEN_IN_SUBMENU_LABEL)
            .expect("local file tabs offer Open In");
        assert_eq!(labels(open_in), vec!["Finder", "Terminal"]);
        let git = submenu(&local_in_repository, GIT_SUBMENU_LABEL)
            .expect("files in a repository offer the Git submenu");
        assert_eq!(
            labels(git),
            vec!["Open File Permalink", "Copy File Permalink"]
        );
        assert_eq!(
            labels(&local_in_repository)[labels(&local_in_repository).len() - 6..],
            [
                SEPARATOR,
                "Reopen Closed Tab",
                SEPARATOR,
                OPEN_IN_SUBMENU_LABEL,
                SEPARATOR,
                GIT_SUBMENU_LABEL,
            ]
        );

        let local_outside_repository = tab_context_menu_items(&TabMenuSnapshot {
            file: Some(local_file()),
            ..single_tab()
        });
        assert!(submenu(&local_outside_repository, GIT_SUBMENU_LABEL).is_none());
        assert!(submenu(&local_outside_repository, OPEN_IN_SUBMENU_LABEL).is_some());

        let remote_in_repository = tab_context_menu_items(&TabMenuSnapshot {
            file: Some(TabFileSnapshot {
                is_local: false,
                has_repository_path: true,
                ..local_file()
            }),
            ..single_tab()
        });
        assert!(submenu(&remote_in_repository, OPEN_IN_SUBMENU_LABEL).is_none());
        assert!(submenu(&remote_in_repository, GIT_SUBMENU_LABEL).is_some());

        let without_parent = tab_context_menu_items(&TabMenuSnapshot {
            file: Some(TabFileSnapshot {
                has_parent_directory: false,
                ..local_file()
            }),
            ..single_tab()
        });
        let open_in =
            submenu(&without_parent, OPEN_IN_SUBMENU_LABEL).expect("local file tabs offer Open In");
        assert_eq!(labels(open_in), vec!["Finder"]);

        let without_file = tab_context_menu_items(&single_tab());
        assert!(submenu(&without_file, OPEN_IN_SUBMENU_LABEL).is_none());
        assert!(submenu(&without_file, GIT_SUBMENU_LABEL).is_none());
    }

    #[test]
    fn tab_context_menu_preview_tab_offers_keep_tab_open() {
        let preview = labels(&tab_context_menu_items(&TabMenuSnapshot {
            is_preview: true,
            ..single_tab()
        }));
        let pin = preview
            .iter()
            .position(|label| *label == "Pin Tab")
            .expect("Pin Tab is always shown");
        assert_eq!(
            preview[pin..pin + 3],
            ["Pin Tab", "Keep Tab Open", "Configure Editor Tabs…"]
        );

        let regular = labels(&tab_context_menu_items(&single_tab()));
        assert!(!regular.contains(&"Keep Tab Open"));
    }

    #[test]
    fn tab_context_menu_is_well_formed_for_every_state() {
        let files = [
            None,
            Some(local_file()),
            Some(TabFileSnapshot {
                has_content_root_path: false,
                has_repository_path: true,
                is_local: false,
                has_parent_directory: false,
            }),
        ];
        for tab_count in 1..=3 {
            for pinned_tab_count in 0..=tab_count {
                for clicked_index in 0..tab_count {
                    for split_pane_count in 1..=3 {
                        for can_split in [false, true] {
                            for is_preview in [false, true] {
                                for file in files {
                                    let items = tab_context_menu_items(&TabMenuSnapshot {
                                        tab_count,
                                        pinned_tab_count,
                                        clicked_index,
                                        is_preview,
                                        can_split,
                                        split_pane_count,
                                        file,
                                    });
                                    assert_eq!(
                                        items.first(),
                                        Some(&TabMenuItem::entry("Close", TabMenuCommand::Close))
                                    );
                                    assert_well_formed(&items);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn tab_bar_more_menu_lists_intellij_entries() {
        let one_pane = tab_bar_more_menu_items(1);
        assert_eq!(
            labels(&one_pane),
            vec![
                "Recent Files",
                "Go to File…",
                SEPARATOR,
                "Close All Tabs",
                "Reopen Closed Tab",
                SEPARATOR,
                "Configure Editor Tabs…",
            ]
        );
        assert_well_formed(&one_pane);

        let three_panes = tab_bar_more_menu_items(3);
        assert_eq!(
            labels(&three_panes),
            vec![
                "Recent Files",
                "Go to File…",
                SEPARATOR,
                "Close All Tabs In Group",
                "Reopen Closed Tab",
                SEPARATOR,
                "Unsplit",
                "Unsplit All",
                SEPARATOR,
                "Configure Editor Tabs…",
            ]
        );
        assert_well_formed(&three_panes);

        let two_panes = labels(&tab_bar_more_menu_items(2));
        assert!(two_panes.contains(&"Unsplit"));
        assert!(!two_panes.contains(&"Unsplit All"));
    }
}
