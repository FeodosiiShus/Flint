use std::{sync::Arc, time::Duration};

use anyhow::Result;
use editor::{Editor, EditorEvent};
use fs::Fs;
use gpui::{
    Action, AsyncWindowContext, Entity, EventEmitter, FocusHandle, Focusable, Subscription, Task,
    WeakEntity,
};
use project::{
    LspStoreEvent,
    trusted_worktrees::{TrustedWorktrees, TrustedWorktreesEvent},
};
use settings::{IntoGpui, RegisterSetting, Settings};
use ui::{IconButtonShape, Tooltip, prelude::*};
use workspace::{
    HideStatusItem, ItemHandle, ToggleWorktreeSecurity, Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

use crate::lsp_button::{
    ActiveEditor, LanguageServerState, LspMenuItem, ServerInfo, ServerMetadata, ToggleFocus,
};

const LANGUAGE_SERVICES_PANEL_KEY: &str = "LanguageServicesPanel";
const LANGUAGE_SERVICES_PANEL_ACTIVATION_PRIORITY: u32 = 6;
const ITEM_REFRESH_DEBOUNCE: Duration = Duration::from_millis(30);
const MEMORY_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const BYTES_PER_MEGABYTE: f64 = 1024.0 * 1024.0;
const BYTES_PER_GIGABYTE: f64 = 1024.0 * 1024.0 * 1024.0;

#[derive(Debug, Clone, PartialEq, RegisterSetting)]
pub struct LanguageServicesPanelSettings {
    pub button: bool,
    pub dock: DockPosition,
    pub default_width: Pixels,
}

impl Settings for LanguageServicesPanelSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let language_services_panel = content.language_services_panel.clone().unwrap();
        Self {
            button: language_services_panel.button.unwrap(),
            dock: language_services_panel.dock.unwrap().into(),
            default_width: language_services_panel.default_width.unwrap().into_gpui(),
        }
    }
}

pub(crate) fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _window, _cx| {
        workspace.register_action(|workspace, _: &ToggleFocus, window, cx| {
            workspace.toggle_panel_focus::<LanguageServicesPanel>(window, cx);
        });
    })
    .detach();
}

#[derive(Debug, Clone)]
pub(crate) enum PanelRow {
    Header(SharedString),
    ToggleAllServers { restart: bool },
    Server(ServerRow),
}

#[derive(Debug, Clone)]
pub(crate) struct ServerRow {
    pub(crate) info: ServerInfo,
    pub(crate) status_color: Color,
    pub(crate) status_label: &'static str,
    pub(crate) message: Option<SharedString>,
    pub(crate) version: Option<String>,
    pub(crate) binary_path: Option<SharedString>,
    pub(crate) process_id: Option<u32>,
}

impl ServerRow {
    fn new(info: ServerInfo, metadata: ServerMetadata) -> Self {
        let (status_color, status_label) = info.status_display();
        let message = info.display_message();
        let ServerMetadata {
            server_version,
            binary_display_path,
            process_id,
        } = metadata;
        Self {
            info,
            status_color,
            status_label,
            message,
            version: server_version.map(|version| format!("v{version}")),
            binary_path: binary_display_path,
            process_id,
        }
    }
}

fn format_memory(bytes: u64) -> String {
    let bytes = bytes as f64;
    if bytes >= BYTES_PER_GIGABYTE {
        format!("{:.1} GB", bytes / BYTES_PER_GIGABYTE)
    } else {
        format!("{:.1} MB", bytes / BYTES_PER_MEGABYTE)
    }
}

pub struct LanguageServicesPanel {
    focus_handle: FocusHandle,
    fs: Arc<dyn Fs>,
    pub(crate) server_state: Entity<LanguageServerState>,
    refresh_task: Task<()>,
    memory_refresh_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl LanguageServicesPanel {
    pub async fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        workspace.update_in(&mut cx, |workspace, window, cx| {
            cx.new(|cx| Self::new(workspace, window, cx))
        })
    }

    pub(crate) fn new(workspace: &Workspace, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let server_state = cx.new(|cx| LanguageServerState::new(workspace, cx));
        let lsp_store = workspace.project().read(cx).lsp_store();

        let mut subscriptions =
            vec![
                cx.subscribe(&lsp_store, |this, _, event: &LspStoreEvent, cx| {
                    let updated = this
                        .server_state
                        .update(cx, |state, cx| state.apply_lsp_store_event(event, cx));
                    if updated {
                        this.refresh_items(cx);
                    }
                }),
            ];
        if let Some(workspace_entity) = workspace.weak_handle().upgrade() {
            subscriptions.push(cx.subscribe_in(
                &workspace_entity,
                window,
                |this, workspace, event: &workspace::Event, _, cx| {
                    if matches!(event, workspace::Event::ActiveItemChanged) {
                        let active_item = workspace.read(cx).active_item(cx);
                        this.set_active_item(active_item, cx);
                    }
                },
            ));
        }
        if let Some(trusted_worktrees) = TrustedWorktrees::try_get_global(cx) {
            subscriptions.push(
                cx.subscribe(&trusted_worktrees, |_, _, _: &TrustedWorktreesEvent, cx| {
                    cx.notify()
                }),
            );
        }

        let mut panel = Self {
            focus_handle: cx.focus_handle(),
            fs: workspace.app_state().fs.clone(),
            server_state,
            refresh_task: Task::ready(()),
            memory_refresh_task: None,
            _subscriptions: subscriptions,
        };
        panel.set_active_item(workspace.active_item(cx), cx);
        panel.refresh_items(cx);
        panel
    }

    pub(crate) fn rows(&self, cx: &App) -> Vec<PanelRow> {
        let state = self.server_state.read(cx);
        state
            .items
            .iter()
            .filter_map(|item| match item {
                LspMenuItem::Header { header, .. } => header.clone().map(PanelRow::Header),
                LspMenuItem::ToggleServersButton { restart } => {
                    Some(PanelRow::ToggleAllServers { restart: *restart })
                }
                LspMenuItem::WithHealthCheck { .. } | LspMenuItem::WithBinaryStatus { .. } => {
                    item.server_info().map(|info| {
                        let metadata = state.metadata_for(info.id);
                        PanelRow::Server(ServerRow::new(info, metadata))
                    })
                }
            })
            .collect()
    }

    fn refresh_items(&mut self, cx: &mut Context<Self>) {
        self.refresh_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ITEM_REFRESH_DEBOUNCE).await;
            this.update(cx, |this, cx| {
                this.server_state
                    .update(cx, |state, cx| state.regenerate_items(cx));
                cx.notify();
            })
            .ok();
        });
    }

    fn set_active_item(
        &mut self,
        active_item: Option<Box<dyn ItemHandle>>,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = active_item.and_then(|item| item.downcast::<Editor>()) else {
            if self.server_state.read(cx).active_editor.is_some() {
                self.server_state
                    .update(cx, |state, _| state.active_editor = None);
                self.refresh_items(cx);
            }
            return;
        };

        let is_already_tracked = self
            .server_state
            .read(cx)
            .active_editor
            .as_ref()
            .and_then(|active_editor| active_editor.editor.upgrade())
            .is_some_and(|active_editor| active_editor == editor);
        if is_already_tracked {
            return;
        }

        let editor_buffers = editor
            .read(cx)
            .buffer()
            .read(cx)
            .snapshot(cx)
            .excerpts()
            .map(|excerpt| excerpt.context.start.buffer_id)
            .collect();
        let editor_subscription =
            cx.subscribe(&editor, |this, _, event: &EditorEvent, cx| match event {
                EditorEvent::BufferRangesUpdated { buffer, .. } => {
                    let buffer_id = buffer.read(cx).remote_id();
                    let updated = this.server_state.update(cx, |state, _| {
                        state.active_editor.as_mut().is_some_and(|active_editor| {
                            active_editor.editor_buffers.insert(buffer_id)
                        })
                    });
                    if updated {
                        this.refresh_items(cx);
                    }
                }
                EditorEvent::BuffersRemoved { removed_buffer_ids } => {
                    let removed = this.server_state.update(cx, |state, _| {
                        let Some(active_editor) = state.active_editor.as_mut() else {
                            return false;
                        };
                        let buffers_before = active_editor.editor_buffers.len();
                        active_editor
                            .editor_buffers
                            .retain(|buffer_id| !removed_buffer_ids.contains(buffer_id));
                        active_editor.editor_buffers.len() != buffers_before
                    });
                    if removed {
                        this.refresh_items(cx);
                    }
                }
                _ => {}
            });
        self.server_state.update(cx, |state, _| {
            state.active_editor = Some(ActiveEditor {
                editor: editor.downgrade(),
                _editor_subscription: editor_subscription,
                editor_buffers,
            });
        });
        self.refresh_items(cx);
    }

    fn render_restricted_banner(cx: &mut Context<Self>) -> impl IntoElement {
        let hover_background = cx.theme().colors().ghost_element_hover;
        v_flex()
            .id("language-services-restricted-banner")
            .w_full()
            .px_2()
            .py_1()
            .gap_0p5()
            .cursor_pointer()
            .hover(move |style| style.bg(hover_background))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Icon::new(IconName::Warning)
                            .color(Color::Warning)
                            .size(IconSize::XSmall),
                    )
                    .child(Label::new("Project is in Restricted Mode").size(LabelSize::Small)),
            )
            .child(
                Label::new("Language Servers can't run until you trust this project.")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .on_click(|_, window, cx| {
                window.dispatch_action(ToggleWorktreeSecurity.boxed_clone(), cx);
            })
    }

    fn render_header_row(name: SharedString) -> impl IntoElement {
        div()
            .w_full()
            .px_2()
            .pt_1p5()
            .pb_0p5()
            .child(Label::new(name).size(LabelSize::Small).color(Color::Muted))
    }

    fn render_toggle_all_row(restart: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let (id, label, icon) = if restart {
            (
                "language-services-restart-all",
                "Restart All Servers",
                IconName::RotateCw,
            )
        } else {
            (
                "language-services-stop-all",
                "Stop All Servers",
                IconName::Stop,
            )
        };
        Button::new(id, label)
            .full_width()
            .style(ButtonStyle::Subtle)
            .label_size(LabelSize::Small)
            .start_icon(Icon::new(icon).size(IconSize::Small))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.server_state.update(cx, |state, cx| {
                    if restart {
                        state.restart_all_servers(cx);
                    } else {
                        state.stop_all_servers(cx);
                    }
                });
            }))
    }

    fn render_server_row(&self, row: ServerRow, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.server_state.read(cx);
        let server_selector = row.info.server_selector();
        let has_logs = state.has_logs(&server_selector, cx);
        let memory_label = row
            .process_id
            .map(|process_id| format_memory(state.memory_usage(process_id)));

        let server_id = row.info.id.0;
        let server_name = row.info.name.clone();
        let can_stop = row.info.can_stop();

        let details = [
            Some(row.status_label.to_string()),
            row.version.clone(),
            memory_label,
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");

        let view_message_button = row.message.clone().map(|message| {
            let server_name = server_name.clone();
            IconButton::new(("language-server-view-message", server_id), IconName::Info)
                .shape(IconButtonShape::Square)
                .icon_size(IconSize::Small)
                .tooltip(Tooltip::text("View Message"))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.server_state.update(cx, |state, cx| {
                        state.view_message(server_name.clone(), message.clone(), window, cx);
                    });
                }))
        });

        let view_logs_button = has_logs.then(|| {
            let server_selector = server_selector.clone();
            IconButton::new(
                ("language-server-view-logs", server_id),
                IconName::FileTextOutlined,
            )
            .shape(IconButtonShape::Square)
            .icon_size(IconSize::Small)
            .tooltip(Tooltip::text("View Logs"))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.server_state.update(cx, |state, cx| {
                    state.view_logs(server_selector.clone(), window, cx);
                });
            }))
        });

        let restart_button =
            IconButton::new(("language-server-restart", server_id), IconName::RotateCw)
                .shape(IconButtonShape::Square)
                .icon_size(IconSize::Small)
                .tooltip(Tooltip::text("Restart Server"))
                .on_click(cx.listener({
                    let server_name = server_name.clone();
                    move |this, _, _, cx| {
                        this.server_state
                            .update(cx, |state, cx| state.restart_server(&server_name, cx));
                    }
                }));

        let stop_button = can_stop.then(|| {
            let server_selector = server_selector.clone();
            IconButton::new(("language-server-stop", server_id), IconName::Stop)
                .shape(IconButtonShape::Square)
                .icon_size(IconSize::Small)
                .tooltip(Tooltip::text("Stop Server"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.server_state.update(cx, |state, cx| {
                        state.stop_server(server_selector.clone(), cx);
                    });
                }))
        });

        h_flex()
            .id(("language-server-row", server_id))
            .w_full()
            .px_2()
            .py_1()
            .gap_2()
            .when_some(row.binary_path.clone(), |this, binary_path| {
                this.tooltip(Tooltip::text(binary_path))
            })
            .child(
                Icon::new(IconName::Circle)
                    .color(row.status_color)
                    .size(IconSize::XSmall),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(Label::new(server_name.0.clone()).truncate())
                    .child(
                        Label::new(details)
                            .size(LabelSize::Small)
                            .color(Color::Muted)
                            .truncate(),
                    )
                    .when_some(row.message, |this, message| {
                        this.child(
                            Label::new(message)
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                    }),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_0p5()
                    .children(view_message_button)
                    .children(view_logs_button)
                    .child(restart_button)
                    .children(stop_button),
            )
    }

    fn render_empty_state() -> impl IntoElement {
        div()
            .w_full()
            .px_2()
            .py_2()
            .child(Label::new("No language servers").color(Color::Muted))
    }
}

impl EventEmitter<PanelEvent> for LanguageServicesPanel {}

impl Focusable for LanguageServicesPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for LanguageServicesPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_restricted = self.server_state.read(cx).is_restricted(cx);
        let rows = self.rows(cx);
        let is_empty = rows.is_empty();

        let mut row_elements = Vec::with_capacity(rows.len());
        for row in rows {
            row_elements.push(match row {
                PanelRow::Header(name) => Self::render_header_row(name).into_any_element(),
                PanelRow::ToggleAllServers { restart } => {
                    Self::render_toggle_all_row(restart, cx).into_any_element()
                }
                PanelRow::Server(server_row) => {
                    self.render_server_row(server_row, cx).into_any_element()
                }
            });
        }

        v_flex()
            .key_context("LanguageServicesPanel")
            .track_focus(&self.focus_handle)
            .size_full()
            .child(
                v_flex()
                    .id("language-services-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(is_restricted, |this| {
                        this.child(Self::render_restricted_banner(cx))
                    })
                    .when(is_empty, |this| this.child(Self::render_empty_state()))
                    .children(row_elements),
            )
    }
}

impl Panel for LanguageServicesPanel {
    fn persistent_name() -> &'static str {
        LANGUAGE_SERVICES_PANEL_KEY
    }

    fn panel_key() -> &'static str {
        LANGUAGE_SERVICES_PANEL_KEY
    }

    fn position(&self, _: &Window, cx: &App) -> DockPosition {
        LanguageServicesPanelSettings::get_global(cx).dock
    }

    fn position_is_valid(&self, _: DockPosition) -> bool {
        true
    }

    fn set_position(&mut self, position: DockPosition, _: &mut Window, cx: &mut Context<Self>) {
        settings::update_settings_file(self.fs.clone(), cx, move |settings, _| {
            settings
                .language_services_panel
                .get_or_insert_default()
                .dock = Some(position.into())
        });
    }

    fn default_size(&self, _: &Window, cx: &App) -> Pixels {
        LanguageServicesPanelSettings::get_global(cx).default_width
    }

    fn icon(&self, _: &Window, cx: &App) -> Option<IconName> {
        LanguageServicesPanelSettings::get_global(cx)
            .button
            .then_some(IconName::BoltOutlined)
    }

    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<&'static str> {
        Some("Language Services Panel")
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(ToggleFocus)
    }

    fn icon_label(&self, _: &Window, cx: &App) -> Option<String> {
        let servers_needing_attention = self.server_state.read(cx).servers_needing_attention();
        (servers_needing_attention > 0).then(|| servers_needing_attention.to_string())
    }

    fn activation_priority(&self) -> u32 {
        LANGUAGE_SERVICES_PANEL_ACTIVATION_PRIORITY
    }

    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.memory_refresh_task = active.then(|| {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(MEMORY_REFRESH_INTERVAL)
                        .await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            })
        });
        if active {
            self.refresh_items(cx);
        }
    }

    fn hide_button_setting(&self, _: &App) -> Option<HideStatusItem> {
        Some(HideStatusItem::new(|settings| {
            settings
                .language_services_panel
                .get_or_insert_default()
                .button = Some(false);
        }))
    }
}
