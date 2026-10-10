use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};

use client::proto;
use collections::HashSet;
use editor::Editor;
use gpui::{App, Entity, Subscription, TaskExt, WeakEntity, actions};
use language::{BinaryStatus, Buffer, BufferId, ServerHealth};
use lsp::{LanguageServerId, LanguageServerName, LanguageServerSelector};
use path::PathStyle;
use project::{
    LspStore, LspStoreEvent, Project, Worktree, lsp_store::log_store::GlobalLogStore,
    trusted_worktrees::TrustedWorktrees,
};
use ui::prelude::*;

use util::{ResultExt, paths::PathExt, rel_path::RelPath};
use workspace::Workspace;

use crate::lsp_log_view;

actions!(
    lsp_tool,
    [
        /// Toggles the language server tool menu.
        ToggleFocus
    ]
);

pub(crate) struct LanguageServerState {
    pub(crate) items: Vec<LspMenuItem>,
    pub(crate) workspace: WeakEntity<Workspace>,
    pub(crate) lsp_store: WeakEntity<LspStore>,
    pub(crate) active_editor: Option<ActiveEditor>,
    pub(crate) language_servers: LanguageServers,
    pub(crate) server_metadata: HashMap<LanguageServerId, ServerMetadata>,
    pub(crate) process_memory_cache: Rc<RefCell<ProcessMemoryCache>>,
}

impl std::fmt::Debug for LanguageServerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LanguageServerState")
            .field("items", &self.items)
            .field("workspace", &self.workspace)
            .field("lsp_store", &self.lsp_store)
            .field("active_editor", &self.active_editor)
            .field("language_servers", &self.language_servers)
            .finish_non_exhaustive()
    }
}

const PROCESS_MEMORY_CACHE_DURATION: Duration = Duration::from_secs(5);

pub(crate) struct ProcessMemoryCache {
    system: System,
    memory_usage: HashMap<u32, u64>,
    last_refresh: Option<Instant>,
}

impl ProcessMemoryCache {
    fn new() -> Self {
        Self {
            system: System::new(),
            memory_usage: HashMap::new(),
            last_refresh: None,
        }
    }

    fn get_memory_usage(&mut self, process_id: u32) -> u64 {
        let cache_expired = self
            .last_refresh
            .map(|last| last.elapsed() >= PROCESS_MEMORY_CACHE_DURATION)
            .unwrap_or(true);

        if cache_expired {
            let refresh_kind = RefreshKind::nothing()
                .with_processes(ProcessRefreshKind::nothing().without_tasks().with_memory());
            self.system.refresh_specifics(refresh_kind);
            self.memory_usage.clear();
            self.last_refresh = Some(Instant::now());
        }

        if let Some(&memory) = self.memory_usage.get(&process_id) {
            return memory;
        }

        let root_pid = Pid::from_u32(process_id);

        let parent_map: HashMap<Pid, Pid> = self
            .system
            .processes()
            .iter()
            .filter_map(|(&pid, process)| Some((pid, process.parent()?)))
            .collect();

        let total_memory = self
            .system
            .processes()
            .iter()
            .filter(|(pid, _)| self.is_descendant_of(**pid, root_pid, &parent_map))
            .map(|(_, process)| process.memory())
            .sum();

        self.memory_usage.insert(process_id, total_memory);
        total_memory
    }

    fn is_descendant_of(&self, pid: Pid, root_pid: Pid, parent_map: &HashMap<Pid, Pid>) -> bool {
        let mut current = pid;
        let mut visited = HashSet::default();
        while current != root_pid {
            if !visited.insert(current) {
                return false;
            }
            match parent_map.get(&current) {
                Some(&parent) => current = parent,
                None => return false,
            }
        }
        true
    }
}

pub(crate) struct ActiveEditor {
    pub(crate) editor: WeakEntity<Editor>,
    pub(crate) _editor_subscription: Subscription,
    pub(crate) editor_buffers: HashSet<BufferId>,
}

impl std::fmt::Debug for ActiveEditor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveEditor")
            .field("editor", &self.editor)
            .field("editor_buffers", &self.editor_buffers)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct LanguageServers {
    pub(crate) health_statuses: HashMap<LanguageServerId, LanguageServerHealthStatus>,
    pub(crate) binary_statuses: HashMap<LanguageServerName, LanguageServerBinaryStatus>,
    pub(crate) servers_per_buffer_abs_path: HashMap<PathBuf, ServersForPath>,
}

#[derive(Debug, Clone)]
pub(crate) struct ServersForPath {
    pub(crate) servers: HashMap<LanguageServerId, Option<LanguageServerName>>,
    pub(crate) worktree: Option<WeakEntity<Worktree>>,
}

#[derive(Debug, Clone)]
pub(crate) struct LanguageServerHealthStatus {
    pub(crate) name: LanguageServerName,
    pub(crate) health: Option<(Option<SharedString>, ServerHealth)>,
}

#[derive(Debug, Clone)]
pub(crate) struct LanguageServerBinaryStatus {
    pub(crate) status: BinaryStatus,
    pub(crate) message: Option<SharedString>,
}

#[derive(Debug, Clone)]
pub(crate) struct ServerInfo {
    pub(crate) name: LanguageServerName,
    pub(crate) id: LanguageServerId,
    pub(crate) health: Option<ServerHealth>,
    pub(crate) binary_status: Option<LanguageServerBinaryStatus>,
    pub(crate) message: Option<SharedString>,
}

#[derive(Default, Clone)]
pub(crate) struct ServerMetadata {
    pub(crate) server_version: Option<SharedString>,
    pub(crate) binary_display_path: Option<SharedString>,
    pub(crate) process_id: Option<u32>,
}

impl ServerInfo {
    pub(crate) fn server_selector(&self) -> LanguageServerSelector {
        LanguageServerSelector::Id(self.id)
    }

    pub(crate) fn can_stop(&self) -> bool {
        self.binary_status.as_ref().is_none_or(|status| {
            matches!(status.status, BinaryStatus::None | BinaryStatus::Starting)
        })
    }

    pub(crate) fn status_display(&self) -> (Color, &'static str) {
        self.binary_status
            .as_ref()
            .and_then(|binary_status| match binary_status.status {
                BinaryStatus::None => None,
                BinaryStatus::CheckingForUpdate
                | BinaryStatus::Downloading
                | BinaryStatus::Starting => Some((Color::Modified, "Starting…")),
                BinaryStatus::Stopping | BinaryStatus::Stopped => {
                    Some((Color::Disabled, "Stopped"))
                }
                BinaryStatus::Failed { .. } => Some((Color::Error, "Error")),
            })
            .or_else(|| {
                Some(match self.health? {
                    ServerHealth::Ok => (Color::Success, "Running"),
                    ServerHealth::Warning => (Color::Warning, "Warning"),
                    ServerHealth::Error => (Color::Error, "Error"),
                })
            })
            .unwrap_or((Color::Success, "Running"))
    }

    pub(crate) fn display_message(&self) -> Option<SharedString> {
        self.message
            .as_ref()
            .or_else(|| self.binary_status.as_ref()?.message.as_ref())
            .cloned()
    }
}

impl LanguageServerHealthStatus {
    fn health(&self) -> Option<ServerHealth> {
        self.health.as_ref().map(|(_, health)| *health)
    }

    fn message(&self) -> Option<SharedString> {
        self.health
            .as_ref()
            .and_then(|(message, _)| message.clone())
    }
}

fn tooltip_for_server_binary(
    server_binary: &lsp::LanguageServerBinary,
    path_style: PathStyle,
) -> SharedString {
    let runtime = path_style.file_name(&server_binary.path).and_then(|name| {
        ["node", "python"]
            .into_iter()
            .find(|runtime| name.starts_with(runtime))
    });

    let target_path = runtime
        .and_then(|_runtime| {
            server_binary
                .arguments
                .iter()
                .find(|arg| !arg.to_string_lossy().starts_with('-'))
        })
        .map(Path::new)
        .unwrap_or(&server_binary.path);

    let display_path = path_style.normalize(&target_path.compact().to_string_lossy());

    match runtime {
        Some(runtime) => format!("{display_path} ({runtime})").into(),
        None => display_path.into(),
    }
}

impl LanguageServers {
    fn update_binary_status(
        &mut self,
        binary_status: BinaryStatus,
        message: Option<&str>,
        name: LanguageServerName,
    ) {
        let binary_status_message = message.map(SharedString::new);
        if matches!(
            binary_status,
            BinaryStatus::Stopped | BinaryStatus::Failed { .. }
        ) {
            self.health_statuses.retain(|_, server| server.name != name);
        }
        self.binary_statuses.insert(
            name,
            LanguageServerBinaryStatus {
                status: binary_status,
                message: binary_status_message,
            },
        );
    }

    fn update_server_health(
        &mut self,
        id: LanguageServerId,
        health: ServerHealth,
        message: Option<&str>,
        name: Option<LanguageServerName>,
    ) {
        if let Some(state) = self.health_statuses.get_mut(&id) {
            state.health = Some((message.map(SharedString::new), health));
            if let Some(name) = name {
                state.name = name;
            }
        } else if let Some(name) = name {
            self.health_statuses.insert(
                id,
                LanguageServerHealthStatus {
                    health: Some((message.map(SharedString::new), health)),
                    name,
                },
            );
        }
    }

    /// Drop all id-keyed state for a server that has been removed (stopped or
    /// reaching end-of-life via restart). `binary_statuses` is intentionally
    /// preserved — it is keyed by name and shared across restart cycles to
    /// drive the "Downloading… → Starting…" status UX.
    fn remove_server(&mut self, server_id: LanguageServerId) {
        self.health_statuses.remove(&server_id);
        self.servers_per_buffer_abs_path
            .retain(|_, servers_for_path| {
                servers_for_path.servers.remove(&server_id);
                !servers_for_path.servers.is_empty()
            });
    }
}

#[derive(Debug)]
enum ServerData<'a> {
    WithHealthCheck {
        server_id: LanguageServerId,
        health: &'a LanguageServerHealthStatus,
        binary_status: Option<&'a LanguageServerBinaryStatus>,
    },
    WithBinaryStatus {
        server_id: LanguageServerId,
        server_name: &'a LanguageServerName,
        binary_status: &'a LanguageServerBinaryStatus,
    },
}

#[derive(Debug)]
pub(crate) enum LspMenuItem {
    WithHealthCheck {
        server_id: LanguageServerId,
        health: LanguageServerHealthStatus,
        binary_status: Option<LanguageServerBinaryStatus>,
    },
    WithBinaryStatus {
        server_id: LanguageServerId,
        server_name: LanguageServerName,
        binary_status: LanguageServerBinaryStatus,
    },
    ToggleServersButton {
        restart: bool,
    },
    Header {
        header: Option<SharedString>,
    },
}

impl LspMenuItem {
    pub(crate) fn server_info(&self) -> Option<ServerInfo> {
        match self {
            Self::Header { .. } => None,
            Self::ToggleServersButton { .. } => None,
            Self::WithHealthCheck {
                server_id,
                health,
                binary_status,
                ..
            } => Some(ServerInfo {
                name: health.name.clone(),
                id: *server_id,
                health: health.health(),
                binary_status: binary_status.clone(),
                message: health.message(),
            }),
            Self::WithBinaryStatus {
                server_id,
                server_name,
                binary_status,
                ..
            } => Some(ServerInfo {
                name: server_name.clone(),
                id: *server_id,
                health: None,
                binary_status: Some(binary_status.clone()),
                message: binary_status.message.clone(),
            }),
        }
    }
}

impl ServerData<'_> {
    fn into_lsp_item(self) -> LspMenuItem {
        match self {
            Self::WithHealthCheck {
                server_id,
                health,
                binary_status,
                ..
            } => LspMenuItem::WithHealthCheck {
                server_id,
                health: health.clone(),
                binary_status: binary_status.cloned(),
            },
            Self::WithBinaryStatus {
                server_id,
                server_name,
                binary_status,
                ..
            } => LspMenuItem::WithBinaryStatus {
                server_id,
                server_name: server_name.clone(),
                binary_status: binary_status.clone(),
            },
        }
    }
}

impl LanguageServerState {
    pub(crate) fn new(workspace: &Workspace, cx: &mut Context<Self>) -> Self {
        let lsp_store = workspace.project().read(cx).lsp_store();
        let mut language_servers = LanguageServers::default();
        for (_, status) in lsp_store.read(cx).language_server_statuses() {
            language_servers.binary_statuses.insert(
                status.name.clone(),
                LanguageServerBinaryStatus {
                    status: BinaryStatus::None,
                    message: None,
                },
            );
        }

        Self {
            workspace: workspace.weak_handle(),
            items: Vec::new(),
            lsp_store: lsp_store.downgrade(),
            active_editor: None,
            language_servers,
            server_metadata: HashMap::default(),
            process_memory_cache: Rc::new(RefCell::new(ProcessMemoryCache::new())),
        }
    }

    pub(crate) fn servers_needing_attention(&self) -> usize {
        self.language_servers
            .health_statuses
            .values()
            .filter(|server| {
                matches!(
                    server.health(),
                    Some(ServerHealth::Error | ServerHealth::Warning)
                )
            })
            .count()
    }

    pub(crate) fn is_restricted(&self, cx: &App) -> bool {
        self.workspace
            .upgrade()
            .map(|workspace| {
                let worktree_store = workspace.read(cx).project().read(cx).worktree_store();
                TrustedWorktrees::has_restricted_worktrees(&worktree_store, cx)
            })
            .unwrap_or(false)
    }

    pub(crate) fn metadata_for(&self, server_id: LanguageServerId) -> ServerMetadata {
        self.server_metadata
            .get(&server_id)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn memory_usage(&self, process_id: u32) -> u64 {
        self.process_memory_cache
            .borrow_mut()
            .get_memory_usage(process_id)
    }

    pub(crate) fn has_logs(&self, server_selector: &LanguageServerSelector, cx: &App) -> bool {
        let Some(lsp_logs) = cx
            .try_global::<GlobalLogStore>()
            .map(|lsp_logs| lsp_logs.0.clone())
        else {
            return false;
        };
        let is_remote = self
            .lsp_store
            .read_with(cx, |lsp_store, _| lsp_store.as_remote().is_some())
            .unwrap_or(false);
        is_remote
            || self.workspace.upgrade().is_some_and(|workspace| {
                let project = workspace.read(cx).project();
                lsp_logs.read(cx).has_server_logs(
                    server_selector,
                    &project.downgrade(),
                    &self.lsp_store,
                )
            })
    }

    pub(crate) fn restart_all_servers(&self, cx: &mut App) {
        self.lsp_store
            .update(cx, |lsp_store, cx| {
                lsp_store.restart_all_language_servers(cx)
            })
            .ok();
    }

    pub(crate) fn stop_all_servers(&self, cx: &mut App) {
        self.lsp_store
            .update(cx, |lsp_store, cx| lsp_store.stop_all_language_servers(cx))
            .ok();
    }

    pub(crate) fn stop_server(&self, server_selector: LanguageServerSelector, cx: &mut App) {
        self.lsp_store
            .update(cx, |lsp_store, cx| {
                lsp_store
                    .stop_language_servers_for_buffers(
                        Vec::new(),
                        HashSet::from_iter([server_selector]),
                        cx,
                    )
                    .detach_and_log_err(cx);
            })
            .ok();
    }

    pub(crate) fn view_logs(
        &self,
        server_selector: LanguageServerSelector,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(lsp_logs) = cx
            .try_global::<GlobalLogStore>()
            .map(|lsp_logs| lsp_logs.0.clone())
        else {
            return;
        };
        lsp_log_view::open(
            &lsp_logs,
            self.workspace.clone(),
            server_selector,
            window,
            cx,
        );
    }

    pub(crate) fn view_message(
        &self,
        server_name: LanguageServerName,
        message: SharedString,
        window: &mut Window,
        cx: &mut App,
    ) {
        let workspace = self.workspace.clone();
        let Some(create_buffer) = workspace
            .update(cx, |workspace, cx| {
                workspace
                    .project()
                    .update(cx, |project, cx| project.create_buffer(None, false, cx))
            })
            .ok()
        else {
            return;
        };

        let window_handle = window.window_handle();
        cx.spawn(async move |cx| {
            let buffer = create_buffer.await?;
            buffer.update(cx, |buffer, cx| {
                buffer.edit(
                    [(0..0, format!("Language server {server_name}:\n\n{message}"))],
                    None,
                    cx,
                );
                buffer.set_capability(language::Capability::ReadOnly, cx);
            });

            workspace.update(cx, |workspace, cx| {
                window_handle.update(cx, |_, window, cx| {
                    workspace.add_item_to_active_pane(
                        Box::new(cx.new(|cx| {
                            let mut editor = Editor::for_buffer(buffer, None, window, cx);
                            editor.set_read_only(true);
                            editor
                        })),
                        None,
                        true,
                        window,
                        cx,
                    );
                })
            })??;

            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
    }

    pub(crate) fn restart_server(&self, server_name: &LanguageServerName, cx: &mut App) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let project = workspace.read(cx).project().clone();

        let mut buffers = self.open_buffers(&project, Some(server_name), cx);
        if buffers.is_empty() {
            buffers = self.open_buffers(&project, None, cx);
        }
        if buffers.is_empty() {
            return;
        }

        self.lsp_store
            .update(cx, |lsp_store, cx| {
                lsp_store.restart_language_servers_for_buffers(
                    buffers,
                    HashSet::from_iter([LanguageServerSelector::Name(server_name.clone())]),
                    true,
                    cx,
                );
            })
            .ok();
    }

    fn open_buffers(
        &self,
        project: &Entity<Project>,
        only_for_server: Option<&LanguageServerName>,
        cx: &App,
    ) -> Vec<Entity<Buffer>> {
        let project = project.read(cx);
        let path_style = project.path_style(cx);
        let buffer_store = project.buffer_store().read(cx);

        self.language_servers
            .servers_per_buffer_abs_path
            .iter()
            .filter_map(|(abs_path, servers)| {
                if let Some(server_name) = only_for_server
                    && !servers
                        .servers
                        .values()
                        .any(|name| name.as_ref() == Some(server_name))
                {
                    return None;
                }

                let worktree = servers.worktree.as_ref()?.upgrade()?;
                let worktree = worktree.read(cx);
                let relative_path = abs_path.strip_prefix(&worktree.abs_path()).ok()?;
                let relative_path = RelPath::new(relative_path, path_style).log_err()?;
                let entry = worktree.entry_for_path(&relative_path)?;
                let project_path = project.path_for_entry(entry.id, cx)?;
                buffer_store.get_by_path(&project_path)
            })
            .collect()
    }

    pub(crate) fn apply_lsp_store_event(
        &mut self,
        event: &LspStoreEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        match event {
            LspStoreEvent::LanguageServerUpdate {
                language_server_id,
                name,
                message: proto::update_language_server::Variant::StatusUpdate(status_update),
            } => match &status_update.status {
                Some(proto::status_update::Status::Binary(binary_status)) => {
                    let Some(name) = name.as_ref() else {
                        return false;
                    };
                    let Ok(binary_status) = proto::ServerBinaryStatus::try_from(*binary_status)
                    else {
                        return false;
                    };
                    let binary_status = match binary_status {
                        proto::ServerBinaryStatus::None => BinaryStatus::None,
                        proto::ServerBinaryStatus::CheckingForUpdate => {
                            BinaryStatus::CheckingForUpdate
                        }
                        proto::ServerBinaryStatus::Downloading => BinaryStatus::Downloading,
                        proto::ServerBinaryStatus::Starting => BinaryStatus::Starting,
                        proto::ServerBinaryStatus::Stopping => BinaryStatus::Stopping,
                        proto::ServerBinaryStatus::Stopped => BinaryStatus::Stopped,
                        proto::ServerBinaryStatus::Failed => {
                            let Some(error) = status_update.message.clone() else {
                                return false;
                            };
                            BinaryStatus::Failed { error }
                        }
                    };
                    self.language_servers.update_binary_status(
                        binary_status,
                        status_update.message.as_deref(),
                        name.clone(),
                    );
                    true
                }
                Some(proto::status_update::Status::Health(health_status)) => {
                    let Ok(health) = proto::ServerHealth::try_from(*health_status) else {
                        return false;
                    };
                    let health = match health {
                        proto::ServerHealth::Ok => ServerHealth::Ok,
                        proto::ServerHealth::Warning => ServerHealth::Warning,
                        proto::ServerHealth::Error => ServerHealth::Error,
                    };
                    self.language_servers.update_server_health(
                        *language_server_id,
                        health,
                        status_update.message.as_deref(),
                        name.clone(),
                    );
                    true
                }
                None => false,
            },
            LspStoreEvent::LanguageServerUpdate {
                language_server_id,
                name,
                message: proto::update_language_server::Variant::RegisteredForBuffer(update),
                ..
            } => {
                if let Ok(worktree) = self.workspace.update(cx, |workspace, cx| {
                    workspace
                        .project()
                        .read(cx)
                        .find_worktree(Path::new(&update.buffer_abs_path), cx)
                        .map(|(worktree, _)| worktree.downgrade())
                }) {
                    let entry = self
                        .language_servers
                        .servers_per_buffer_abs_path
                        .entry(PathBuf::from(&update.buffer_abs_path))
                        .or_insert_with(|| ServersForPath {
                            servers: HashMap::default(),
                            worktree: worktree.clone(),
                        });
                    entry.servers.insert(*language_server_id, name.clone());
                    if worktree.is_some() {
                        entry.worktree = worktree;
                    }
                }
                true
            }
            LspStoreEvent::LanguageServerRemoved(server_id) => {
                self.language_servers.remove_server(*server_id);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn regenerate_items(&mut self, cx: &mut Context<Self>) {
        let active_worktrees = self
            .active_editor
            .as_ref()
            .into_iter()
            .flat_map(|active_editor| {
                active_editor
                    .editor
                    .upgrade()
                    .into_iter()
                    .flat_map(|active_editor| {
                        active_editor
                            .read(cx)
                            .buffer()
                            .read(cx)
                            .all_buffers()
                            .into_iter()
                            .filter_map(|buffer| project::File::from_dyn(buffer.read(cx).file()))
                            .map(|buffer_file| buffer_file.worktree.clone())
                    })
            })
            .collect::<HashSet<_>>();

        let mut server_ids_to_worktrees = HashMap::<LanguageServerId, Entity<Worktree>>::default();
        let mut server_names_to_worktrees =
            HashMap::<LanguageServerName, HashSet<(Entity<Worktree>, LanguageServerId)>>::default();
        for servers_for_path in self.language_servers.servers_per_buffer_abs_path.values() {
            if let Some(worktree) = servers_for_path
                .worktree
                .as_ref()
                .and_then(|worktree| worktree.upgrade())
            {
                for (server_id, server_name) in &servers_for_path.servers {
                    server_ids_to_worktrees.insert(*server_id, worktree.clone());
                    if let Some(server_name) = server_name {
                        server_names_to_worktrees
                            .entry(server_name.clone())
                            .or_default()
                            .insert((worktree.clone(), *server_id));
                    }
                }
            }
        }

        let path_style = self
            .workspace
            .upgrade()
            .map(|workspace| workspace.read(cx).path_style(cx))
            .unwrap_or(PathStyle::local());
        let mut server_metadata = HashMap::default();
        self.lsp_store
            .update(cx, |lsp_store, cx| {
                for (server_id, status) in lsp_store.language_server_statuses() {
                    server_metadata.insert(
                        server_id,
                        ServerMetadata {
                            server_version: status.server_readable_version.clone(),
                            binary_display_path: status
                                .binary
                                .as_ref()
                                .map(|binary| tooltip_for_server_binary(binary, path_style)),
                            process_id: status.process_id,
                        },
                    );
                    if let Some(worktree) = status.worktree.and_then(|worktree_id| {
                        lsp_store
                            .worktree_store()
                            .read(cx)
                            .worktree_for_id(worktree_id, cx)
                    }) {
                        server_ids_to_worktrees.insert(server_id, worktree.clone());
                        server_names_to_worktrees
                            .entry(status.name.clone())
                            .or_default()
                            .insert((worktree, server_id));
                    }
                }
            })
            .ok();

        let mut servers_per_worktree = BTreeMap::<SharedString, Vec<ServerData>>::new();
        let mut servers_with_health_checks = HashSet::default();

        for (server_id, health) in &self.language_servers.health_statuses {
            let worktree = server_ids_to_worktrees.get(server_id).or_else(|| {
                let worktrees = server_names_to_worktrees.get(&health.name)?;
                worktrees
                    .iter()
                    .find(|(worktree, _)| active_worktrees.contains(worktree))
                    .or_else(|| worktrees.iter().next())
                    .map(|(worktree, _)| worktree)
            });
            servers_with_health_checks.insert(&health.name);
            let worktree_name =
                worktree.map(|worktree| SharedString::new(worktree.read(cx).root_name_str()));

            let binary_status = self.language_servers.binary_statuses.get(&health.name);
            let server_data = ServerData::WithHealthCheck {
                server_id: *server_id,
                health,
                binary_status,
            };
            if let Some(worktree_name) = worktree_name {
                servers_per_worktree
                    .entry(worktree_name.clone())
                    .or_default()
                    .push(server_data);
            }
        }

        let mut can_stop_all = !self.language_servers.health_statuses.is_empty();
        let mut can_restart_all = self.language_servers.health_statuses.is_empty();
        for (server_name, binary_status) in self
            .language_servers
            .binary_statuses
            .iter()
            .filter(|(name, _)| !servers_with_health_checks.contains(name))
        {
            match binary_status.status {
                BinaryStatus::None => {
                    can_restart_all = false;
                    can_stop_all |= true;
                }
                BinaryStatus::CheckingForUpdate
                | BinaryStatus::Downloading
                | BinaryStatus::Starting
                | BinaryStatus::Stopping => {
                    can_restart_all = false;
                    can_stop_all = false;
                }
                BinaryStatus::Stopped | BinaryStatus::Failed { .. } => {}
            }

            if let Some(worktrees_for_name) = server_names_to_worktrees.get(server_name)
                && let Some((worktree, server_id)) = worktrees_for_name
                    .iter()
                    .find(|(worktree, _)| active_worktrees.contains(worktree))
                    .or_else(|| worktrees_for_name.iter().next())
            {
                let worktree_name = SharedString::new(worktree.read(cx).root_name_str());
                servers_per_worktree
                    .entry(worktree_name.clone())
                    .or_default()
                    .push(ServerData::WithBinaryStatus {
                        server_name,
                        binary_status,
                        server_id: *server_id,
                    });
            }
        }

        let mut new_lsp_items = Vec::with_capacity(servers_per_worktree.len() + 1);
        for (worktree_name, worktree_servers) in servers_per_worktree {
            if worktree_servers.is_empty() {
                continue;
            }
            new_lsp_items.push(LspMenuItem::Header {
                header: Some(worktree_name),
            });
            new_lsp_items.extend(worktree_servers.into_iter().map(ServerData::into_lsp_item));
        }
        if !new_lsp_items.is_empty() {
            if can_stop_all {
                new_lsp_items.push(LspMenuItem::ToggleServersButton { restart: true });
                new_lsp_items.push(LspMenuItem::ToggleServersButton { restart: false });
            } else if can_restart_all {
                new_lsp_items.push(LspMenuItem::ToggleServersButton { restart: true });
            }
        }

        self.items = new_lsp_items;
        self.server_metadata = server_metadata;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server_id(n: usize) -> LanguageServerId {
        LanguageServerId(n)
    }

    fn server_name(s: &str) -> LanguageServerName {
        LanguageServerName(s.into())
    }

    fn health_status(name: &str) -> LanguageServerHealthStatus {
        LanguageServerHealthStatus {
            name: server_name(name),
            health: Some((None, ServerHealth::Ok)),
        }
    }

    fn servers_for_path(servers: &[(LanguageServerId, &str)]) -> ServersForPath {
        ServersForPath {
            servers: servers
                .iter()
                .map(|(id, name)| (*id, Some(server_name(name))))
                .collect(),
            worktree: None,
        }
    }

    /// `remove_server` evicts the id from `health_statuses` so a restarted
    /// server's new id renders without inheriting the old one's stale entry.
    /// This is the regression test for #53627.
    #[test]
    fn remove_server_drops_health_entry_for_id() {
        let mut state = LanguageServers::default();
        state
            .health_statuses
            .insert(server_id(1), health_status("rust-analyzer"));
        state
            .health_statuses
            .insert(server_id(2), health_status("typescript-language-server"));

        state.remove_server(server_id(1));

        assert!(!state.health_statuses.contains_key(&server_id(1)));
        assert!(state.health_statuses.contains_key(&server_id(2)));
    }

    /// `remove_server` evicts the id from each per-buffer entry; entries that
    /// become empty are dropped so the map does not grow unbounded across
    /// many buffer opens/closes.
    #[test]
    fn remove_server_evicts_id_from_per_buffer_entries_and_drops_empty_entries() {
        let mut state = LanguageServers::default();
        let buffer_a = PathBuf::from("/project/a.rs");
        let buffer_b = PathBuf::from("/project/b.rs");

        state.servers_per_buffer_abs_path.insert(
            buffer_a.clone(),
            servers_for_path(&[(server_id(1), "rust-analyzer")]),
        );
        state.servers_per_buffer_abs_path.insert(
            buffer_b.clone(),
            servers_for_path(&[(server_id(1), "rust-analyzer"), (server_id(2), "typos-lsp")]),
        );

        state.remove_server(server_id(1));

        assert!(
            !state.servers_per_buffer_abs_path.contains_key(&buffer_a),
            "buffer_a's entry held only the removed server, so the entry itself should be dropped",
        );
        let buffer_b_entry = state
            .servers_per_buffer_abs_path
            .get(&buffer_b)
            .expect("buffer_b's entry has another server, so it must be retained");
        assert!(!buffer_b_entry.servers.contains_key(&server_id(1)));
        assert!(buffer_b_entry.servers.contains_key(&server_id(2)));
    }

    /// `binary_statuses` is keyed by name and intentionally shared across
    /// restart cycles to drive the "Downloading… → Starting…" UX. Removing a
    /// single server's id must not touch it.
    #[test]
    fn remove_server_does_not_touch_binary_statuses() {
        let mut state = LanguageServers::default();
        state.binary_statuses.insert(
            server_name("rust-analyzer"),
            LanguageServerBinaryStatus {
                status: BinaryStatus::Starting,
                message: None,
            },
        );

        state.remove_server(server_id(1));

        assert!(
            state
                .binary_statuses
                .contains_key(&server_name("rust-analyzer")),
            "binary_statuses is name-keyed and shared across restart cycles",
        );
    }

    /// Simulates the full restart event sequence: remove old id, register
    /// new id with same name, write health for the new id. After restart
    /// only the new id should be visible — no leftover entry from the old
    /// incarnation.
    #[test]
    fn restart_sequence_leaves_only_new_server_id() {
        let mut state = LanguageServers::default();
        let buffer = PathBuf::from("/project/main.rs");
        let name = "rust-analyzer";

        // Pre-restart: server v1 is registered for the buffer with health.
        state
            .servers_per_buffer_abs_path
            .insert(buffer.clone(), servers_for_path(&[(server_id(1), name)]));
        state
            .health_statuses
            .insert(server_id(1), health_status(name));

        // Restart: old id is removed.
        state.remove_server(server_id(1));

        // New id registers for the same buffer.
        let entry = state
            .servers_per_buffer_abs_path
            .entry(buffer.clone())
            .or_insert_with(|| ServersForPath {
                servers: HashMap::default(),
                worktree: None,
            });
        entry.servers.insert(server_id(2), Some(server_name(name)));

        // Health update for the new id arrives.
        state
            .health_statuses
            .insert(server_id(2), health_status(name));

        let entry = state
            .servers_per_buffer_abs_path
            .get(&buffer)
            .expect("buffer must still be tracked");
        assert_eq!(
            entry.servers.keys().copied().collect::<Vec<_>>(),
            vec![server_id(2)],
            "exactly one server for this buffer — the new incarnation",
        );
        assert!(
            !state.health_statuses.contains_key(&server_id(1)),
            "the dead server's health entry must not linger",
        );
        assert!(
            state.health_statuses.contains_key(&server_id(2)),
            "the new server's health entry is present",
        );
    }

    #[test]
    fn tooltip_for_server_binary_handles_runtime_and_standalone_servers() {
        let node_server = lsp::LanguageServerBinary {
            path: "/usr/bin/node".into(),
            arguments: vec![
                "/zed/languages/basedpyright/langserver.index.js".into(),
                "--stdio".into(),
            ],
            env: None,
        };
        assert_eq!(
            tooltip_for_server_binary(&node_server, PathStyle::Unix),
            "/zed/languages/basedpyright/langserver.index.js (node)"
        );

        let node_server_windows = lsp::LanguageServerBinary {
                path: "C:\\Program Files\\nodejs\\node.exe".into(),
                arguments: vec![
                    "C:\\Users\\Zed\\languages\\basedpyright\\node_modules/basedpyright/langserver.index.js".into(),
                    "--stdio".into(),
                ],
                env: None
        };
        assert_eq!(
            tooltip_for_server_binary(&node_server_windows, PathStyle::Windows),
            "C:\\Users\\Zed\\languages\\basedpyright\\node_modules\\basedpyright\\langserver.index.js (node)"
        );

        let python_server = lsp::LanguageServerBinary {
            path: "/usr/bin/python3".into(),
            arguments: vec!["/zed/languages/pylsp/pylsp".into(), "--stdio".into()],
            env: None,
        };
        assert_eq!(
            tooltip_for_server_binary(&python_server, PathStyle::Unix),
            "/zed/languages/pylsp/pylsp (python)"
        );

        let standalone_server = lsp::LanguageServerBinary {
            path: "/usr/bin/ty".into(),
            arguments: vec!["server".into()],
            env: None,
        };
        assert_eq!(
            tooltip_for_server_binary(&standalone_server, PathStyle::Unix),
            "/usr/bin/ty"
        );

        let flagged_node_server = lsp::LanguageServerBinary {
            path: "/usr/bin/node".into(),
            arguments: vec![
                "--max-old-space-size=8192".into(),
                "/zed/languages/eslint/server.js".into(),
                "--stdio".into(),
            ],
            env: None,
        };
        assert_eq!(
            tooltip_for_server_binary(&flagged_node_server, PathStyle::Unix),
            "/zed/languages/eslint/server.js (node)"
        );
    }
}
