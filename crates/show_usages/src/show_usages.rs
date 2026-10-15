mod collect;
mod find_usages_dialog;
mod hint;
mod model;
mod popup;
#[cfg(test)]
mod show_usages_tests;
mod symbol;

use std::{any::TypeId, future::Future, ops::Range, pin::pin, time::Duration};

use db::kvp::KeyValueStore;
use editor::{
    Editor, MultibufferSelectionMode, Navigated,
    actions::ShowUsages,
    hover_links::{ShowUsagesOfLocations, UsageLocations, UsageLocationsKind},
};
use futures::{FutureExt as _, future::Fuse};
use gpui::{
    Action, App, AsyncWindowContext, Context, DispatchPhase, Entity, EntityId, FocusHandle,
    Focusable, Global, KeyBinding, Modifiers, Pixels, Point, SharedString, Task, TaskExt as _,
    WeakEntity, Window, actions,
};
use language::{Bias, Buffer};
use lsp::LanguageServerId;
use project::{Location, Project};
use settings::Settings as _;
use util::ResultExt as _;
use workspace::{PreviewTabsSettings, Workspace, item::ItemBufferKind};

use crate::{
    collect::{AccessOutcome, UsageContext},
    find_usages_dialog::FindUsagesDialog,
    hint::{HintContent, UsagesHint},
    model::{
        RawUsage, ShowUsagesState, UsageAccess, UsageScope, UsagesModel,
        all_usages_in_line_message, no_usages_message, only_usage_message, out_of_scope_message,
        second_invocation_hint,
    },
    popup::ShowUsagesPopup,
    symbol::{SymbolDescription, SymbolTitle},
};

actions!(
    show_usages,
    [
        ToggleReadAccess,
        ToggleWriteAccess,
        ToggleImports,
        ToggleComments,
        TogglePreview,
        OpenInFindWindow,
        ShowSettings,
        SelectPageUp,
        SelectPageDown
    ]
);

const POPUP_DELAY: Duration = Duration::from_millis(300);
const ACCESS_WAIT_LIMIT: Duration = Duration::from_millis(1500);
const STATE_KEY: &str = "show_usages_state";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UsagesKind {
    References,
    Implementations,
}

#[derive(Clone)]
pub(crate) enum UsagesTarget {
    Symbol {
        buffer: Entity<Buffer>,
        position: text::Anchor,
    },
    Locations {
        locations: Vec<Location>,
    },
}

#[derive(Clone)]
pub(crate) struct UsagesSession {
    pub(crate) workspace: WeakEntity<Workspace>,
    pub(crate) project: Entity<Project>,
    pub(crate) origin_editor: WeakEntity<Editor>,
    pub(crate) target: UsagesTarget,
    pub(crate) kind: UsagesKind,
    pub(crate) symbol: SymbolDescription,
    pub(crate) scope: UsageScope,
    pub(crate) popup_position: Point<Pixels>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UsagesTitle {
    Symbol(SymbolTitle),
    Implementations(SharedString),
}

impl UsagesTitle {
    pub(crate) fn text(&self) -> SharedString {
        match self {
            UsagesTitle::Symbol(title) => title.text(),
            UsagesTitle::Implementations(text) => text.clone(),
        }
    }
}

impl UsagesSession {
    pub(crate) fn title(&self) -> UsagesTitle {
        match self.kind {
            UsagesKind::References => UsagesTitle::Symbol(self.symbol.title()),
            UsagesKind::Implementations => UsagesTitle::Implementations(SharedString::from(
                format!("Implementations of '{}'", self.symbol.name),
            )),
        }
    }
}

#[derive(Default)]
struct ActiveSearches(collections::HashMap<EntityId, Task<()>>);

impl Global for ActiveSearches {}

struct CachedState(ShowUsagesState);

impl Global for CachedState {}

struct PendingQuery {
    references: Task<anyhow::Result<Vec<Location>>>,
    declarations: Task<(Vec<Location>, Option<anyhow::Error>)>,
}

enum Finish {
    Hint {
        message: String,
        warning: bool,
        target: Option<Location>,
    },
    Popup,
}

pub fn init(cx: &mut App) {
    cx.observe_new(register).detach();
}

fn register(editor: &mut Editor, _window: Option<&mut Window>, cx: &mut Context<Editor>) {
    if !editor.mode().is_full() {
        return;
    }
    let handle = cx.entity().downgrade();
    editor
        .register_action_renderer(move |editor, window, _| {
            if editor.lsp_data_enabled() {
                window.on_action(TypeId::of::<ShowUsages>(), {
                    let handle = handle.clone();
                    move |action, phase, window, cx| {
                        if phase == DispatchPhase::Bubble
                            && let Some(action) = action.downcast_ref::<ShowUsages>()
                        {
                            show_usages_at_caret(&handle, action.at_mouse_position, window, cx);
                        }
                    }
                });
                window.on_action(TypeId::of::<ShowSettings>(), {
                    let handle = handle.clone();
                    move |action, phase, window, cx| {
                        if phase == DispatchPhase::Bubble && action.is::<ShowSettings>() {
                            show_find_usages_options(&handle, window, cx);
                        }
                    }
                });
                window.on_action(
                    TypeId::of::<ShowUsagesOfLocations>(),
                    |action, phase, window, cx| {
                        if phase == DispatchPhase::Bubble
                            && let Some(action) = action.downcast_ref::<ShowUsagesOfLocations>()
                        {
                            show_usages_of_locations(&action.0, window, cx);
                        }
                    },
                );
            }
        })
        .detach();
}

fn show_usages_at_caret(
    editor: &WeakEntity<Editor>,
    at_mouse_position: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(editor) = editor.upgrade() else {
        return;
    };
    let Some(workspace) = editor.read(cx).workspace() else {
        cx.propagate();
        return;
    };
    if continue_in_maximal_scope(&workspace, window, cx) {
        return;
    }
    match caret_session(&editor, &workspace, at_mouse_position, window, cx) {
        Some(session) => run(session, window, cx),
        None => cx.propagate(),
    }
}

fn show_find_usages_options(editor: &WeakEntity<Editor>, window: &mut Window, cx: &mut App) {
    let Some(editor) = editor.upgrade() else {
        return;
    };
    let Some(workspace) = editor.read(cx).workspace() else {
        cx.propagate();
        return;
    };
    let active_hint = workspace.read(cx).active_modal::<UsagesHint>(cx);
    let session = match active_hint {
        Some(hint) => hint.read(cx).session().clone(),
        None => match caret_session(&editor, &workspace, false, window, cx) {
            Some(session) => session,
            None => {
                cx.propagate();
                return;
            }
        },
    };
    show_find_usages_dialog(session, window, cx);
}

fn caret_session(
    editor: &Entity<Editor>,
    workspace: &Entity<Workspace>,
    at_mouse_position: bool,
    window: &mut Window,
    cx: &mut App,
) -> Option<UsagesSession> {
    let (buffer, position) = caret_position(editor, cx)?;
    let project = workspace.read(cx).project().clone();
    let symbol = symbol_at(&buffer, position, &project, cx);
    let popup_position = if at_mouse_position {
        window.mouse_position()
    } else {
        popup_position_below_caret(editor, window, cx)
    };
    Some(UsagesSession {
        workspace: workspace.downgrade(),
        project,
        origin_editor: editor.downgrade(),
        target: UsagesTarget::Symbol { buffer, position },
        kind: UsagesKind::References,
        symbol,
        scope: cached_state(cx).scope,
        popup_position,
    })
}

fn show_usages_of_locations(usage_locations: &UsageLocations, window: &mut Window, cx: &mut App) {
    let Some(editor) = usage_locations.editor.upgrade() else {
        return;
    };
    let Some(workspace) = editor.read(cx).workspace() else {
        return;
    };
    let project = workspace.read(cx).project().clone();
    let multi_buffer = editor.read(cx).buffer().read(cx);
    let symbol = if multi_buffer
        .snapshot(cx)
        .can_resolve(&usage_locations.symbol_position)
    {
        multi_buffer
            .text_anchor_for_position(usage_locations.symbol_position, cx)
            .map(|(buffer, position)| declaration_at(&buffer, position, &project, cx))
            .unwrap_or_default()
    } else {
        SymbolDescription::default()
    };
    let kind = match usage_locations.kind {
        UsageLocationsKind::References => UsagesKind::References,
        UsageLocationsKind::Implementations => UsagesKind::Implementations,
    };
    let workspace_id = workspace.entity_id();
    let workspace_handle = workspace.downgrade();
    let origin_editor = editor.downgrade();
    let scope = cached_state(cx).scope;
    let popup_position = usage_locations.popup_position;
    let server_id = usage_locations.server_id;
    let lsp_locations = usage_locations.locations.clone();
    let task = window.spawn(cx, async move |cx| {
        let (locations, error) =
            resolve_lsp_locations(&project, server_id, lsp_locations, cx).await;
        if let Some(error) = error {
            report_error(&workspace_handle, error, cx);
        }
        let session = UsagesSession {
            workspace: workspace_handle,
            project,
            origin_editor,
            target: UsagesTarget::Locations { locations },
            kind,
            symbol,
            scope,
            popup_position,
        };
        search(session, cx).await;
    });
    track_search(workspace_id, task, cx);
}

pub(crate) fn restart(
    session: UsagesSession,
    scope: UsageScope,
    window: &mut Window,
    cx: &mut App,
) {
    window.defer(cx, move |window, cx| {
        dismiss_usages_modals(&session.workspace, window, cx);
        run(UsagesSession { scope, ..session }, window, cx);
    });
}

pub(crate) fn navigate_to_usage(
    session: &UsagesSession,
    location: Location,
    window: &mut Window,
    cx: &mut App,
) {
    if let Some(navigation) = start_navigation(session, location, window, cx) {
        navigation.detach_and_log_err(cx);
    }
}

pub(crate) fn open_in_find_window(
    session: &UsagesSession,
    locations: Vec<Location>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(workspace) = session.workspace.upgrade() else {
        return;
    };
    let mut ranges_by_buffer: std::collections::HashMap<
        Entity<Buffer>,
        Vec<Range<language::Point>>,
    > = std::collections::HashMap::default();
    for location in locations {
        let buffer = location.buffer.read(cx);
        let start = buffer.summary_for_anchor::<language::Point>(&location.range.start);
        let end = buffer.summary_for_anchor::<language::Point>(&location.range.end);
        ranges_by_buffer
            .entry(location.buffer)
            .or_default()
            .push(start.min(end)..start.max(end));
    }
    if ranges_by_buffer.is_empty() {
        return;
    }
    let title = format!("References to {}", session.symbol.name);
    workspace.update(cx, |workspace, cx| {
        let allow_preview =
            PreviewTabsSettings::get_global(cx).enable_preview_multibuffer_from_code_navigation;
        Editor::open_locations_in_multibuffer(
            workspace,
            ranges_by_buffer,
            title,
            false,
            allow_preview,
            MultibufferSelectionMode::First,
            window,
            cx,
        );
    });
}

pub(crate) fn show_find_usages_dialog(session: UsagesSession, window: &mut Window, cx: &mut App) {
    window.defer(cx, move |window, cx| {
        cancel_search(session.workspace.entity_id(), cx);
        let Some(workspace) = session.workspace.upgrade() else {
            return;
        };
        workspace.update(cx, |workspace, cx| {
            if workspace.active_modal::<FindUsagesDialog>(cx).is_some() {
                return;
            }
            workspace.toggle_modal(window, cx, |window, cx| {
                FindUsagesDialog::new(session, window, cx)
            });
        });
    });
}

pub(crate) fn saved_state(cx: &App) -> ShowUsagesState {
    match cx.try_global::<CachedState>() {
        Some(cached) => cached.0.clone(),
        None => stored_state(cx),
    }
}

pub(crate) fn save_state(state: &ShowUsagesState, cx: &mut App) {
    cx.set_global(CachedState(state.clone()));
    let Some(json) = serde_json::to_string(state).log_err() else {
        return;
    };
    let store = KeyValueStore::global(cx);
    db::write_and_log(cx, move || async move {
        store.write_kvp(STATE_KEY.to_string(), json).await
    });
}

pub(crate) fn action_keystroke_text(
    action: &dyn Action,
    focus_handle: &FocusHandle,
    window: &Window,
) -> Option<SharedString> {
    window
        .highest_precedence_binding_for_action_in(action, focus_handle)
        .map(|binding| binding_text(&binding))
}

fn stored_state(cx: &App) -> ShowUsagesState {
    KeyValueStore::global(cx)
        .read_kvp(STATE_KEY)
        .log_err()
        .flatten()
        .and_then(|json| serde_json::from_str::<ShowUsagesState>(&json).log_err())
        .unwrap_or_default()
}

fn cached_state(cx: &mut App) -> ShowUsagesState {
    if let Some(cached) = cx.try_global::<CachedState>() {
        return cached.0.clone();
    }
    let state = stored_state(cx);
    cx.set_global(CachedState(state.clone()));
    state
}

fn run(session: UsagesSession, window: &mut Window, cx: &mut App) {
    cached_state(cx);
    let workspace_id = session.workspace.entity_id();
    let task = window.spawn(cx, async move |cx| search(session, cx).await);
    track_search(workspace_id, task, cx);
}

fn track_search(workspace_id: EntityId, task: Task<()>, cx: &mut App) {
    let searches = &mut cx.default_global::<ActiveSearches>().0;
    searches.retain(|_, search| !search.is_ready());
    searches.insert(workspace_id, task);
}

fn cancel_search(workspace_id: EntityId, cx: &mut App) {
    cx.default_global::<ActiveSearches>()
        .0
        .remove(&workspace_id);
}

async fn search(mut session: UsagesSession, cx: &mut AsyncWindowContext) {
    let mut popup: Option<WeakEntity<ShowUsagesPopup>> = None;
    let mut popup_timer = cx.background_executor().timer(POPUP_DELAY).fuse();

    let PendingQuery {
        references,
        declarations,
    } = start_query(&session, cx);
    let (declarations, declarations_error) =
        wait_with_popup(declarations, &session, &mut popup_timer, &mut popup, cx).await;
    if popup_dismissed(&session, popup.as_ref(), cx) {
        return;
    }
    if let Some(declaration) = declarations.first() {
        let Ok(parsing) =
            cx.update(|_, cx| collect::parsing_settled(std::slice::from_ref(declaration), cx))
        else {
            return;
        };
        wait_with_popup(parsing, &session, &mut popup_timer, &mut popup, cx).await;
        if popup_dismissed(&session, popup.as_ref(), cx) {
            return;
        }
        if let Ok(Some(symbol)) =
            cx.update(|_, cx| describe_location(declaration, &session.project, cx))
        {
            session.symbol = symbol;
            publish_symbol(&session, popup.as_ref(), cx);
        }
    }

    let references = wait_with_popup(references, &session, &mut popup_timer, &mut popup, cx).await;
    if popup_dismissed(&session, popup.as_ref(), cx) {
        return;
    }
    let references = match references {
        Ok(references) => references,
        Err(error) => {
            cx.update(|window, cx| close_popup(&session, popup.as_ref(), window, cx))
                .log_err();
            report_error(&session.workspace, error, cx);
            return;
        }
    };
    if let Some(error) = declarations_error {
        report_error(&session.workspace, error, cx);
    }

    let Ok(locations) =
        cx.update(|_, cx| collect::exclude_declarations(references, &declarations, cx))
    else {
        return;
    };
    if session.symbol.name.is_empty()
        && let Ok(Some(symbol_name)) =
            cx.update(|_, cx| fallback_symbol_name(&session, &locations, cx))
    {
        session.symbol.name = symbol_name;
        publish_symbol(&session, popup.as_ref(), cx);
    }

    let Ok(parsing) = cx.update(|_, cx| collect::parsing_settled(&locations, cx)) else {
        return;
    };
    wait_with_popup(parsing, &session, &mut popup_timer, &mut popup, cx).await;
    if popup_dismissed(&session, popup.as_ref(), cx) {
        return;
    }

    let Ok(mut raw_usages) = cx.update(|_, cx| {
        let context = usage_context(&session, cx);
        collect::raw_usages(locations, &context, session.project.read(cx), cx)
    }) else {
        return;
    };

    let mut access = None;
    if session.kind == UsagesKind::References && !raw_usages.is_empty() {
        let Ok(requests) =
            cx.update(|_, cx| collect::request_access(&session.project, &raw_usages, cx))
        else {
            return;
        };
        access = Some(
            cx.spawn(async move |cx| collect::resolve_access(requests, cx).await)
                .fuse(),
        );
    }

    let Ok(filters) = cx.update(|_, cx| saved_state(cx).filters) else {
        return;
    };
    if !filters.show_read_access || !filters.show_write_access {
        let mut delivered = None;
        if let Some(access_task) = access.as_mut() {
            let mut access_timeout = cx.background_executor().timer(ACCESS_WAIT_LIMIT).fuse();
            delivered = wait_with_popup(
                async {
                    futures::select_biased! {
                        outcome = &mut *access_task => Some(outcome),
                        () = access_timeout => None,
                    }
                },
                &session,
                &mut popup_timer,
                &mut popup,
                cx,
            )
            .await;
        }
        if let Some(outcome) = delivered {
            access = None;
            let AccessOutcome { accesses, error } = outcome;
            apply_access(&mut raw_usages, &accesses);
            if let Some(error) = error {
                report_error(&session.workspace, error, cx);
            }
        }
        if popup_dismissed(&session, popup.as_ref(), cx) {
            return;
        }
    }

    let Ok(filters) = cx.update(|_, cx| saved_state(cx).filters) else {
        return;
    };
    let model = UsagesModel::new(raw_usages, session.scope, filters);
    match finish_for(&model) {
        Finish::Hint {
            message,
            warning,
            target,
        } => {
            let navigation = cx.update(|window, cx| {
                close_popup(&session, popup.as_ref(), window, cx);
                target.and_then(|target| start_navigation(&session, target, window, cx))
            });
            if let Ok(Some(navigation)) = navigation {
                navigation.await.log_err();
            }
            cx.update(|window, cx| show_hint(&session, message, warning, window, cx))
                .log_err();
        }
        Finish::Popup => {
            let popup = match popup {
                Some(popup) => popup,
                None => match open_popup(&session, cx) {
                    Some(popup) => popup,
                    None => return,
                },
            };
            let model_applied = cx.update(|window, cx| {
                if let Some(popup) = popup.upgrade() {
                    popup.update(cx, |popup, cx| popup.set_model(model, window, cx));
                }
            });
            if model_applied.is_err() {
                return;
            }
            if let Some(access_task) = access {
                deliver_access(&session, &popup, access_task.await, cx);
            }
        }
    }
}

async fn wait_with_popup<T>(
    future: impl Future<Output = T>,
    session: &UsagesSession,
    popup_timer: &mut Fuse<Task<()>>,
    popup: &mut Option<WeakEntity<ShowUsagesPopup>>,
    cx: &mut AsyncWindowContext,
) -> T {
    let mut future = pin!(future.fuse());
    loop {
        futures::select_biased! {
            output = future => return output,
            () = &mut *popup_timer => *popup = open_popup(session, cx),
        }
    }
}

fn start_query(session: &UsagesSession, cx: &mut AsyncWindowContext) -> PendingQuery {
    let (buffer, position) = match &session.target {
        UsagesTarget::Locations { locations } => {
            return PendingQuery {
                references: Task::ready(Ok(locations.clone())),
                declarations: Task::ready((Vec::new(), None)),
            };
        }
        UsagesTarget::Symbol { buffer, position } => (buffer, *position),
    };
    match session.kind {
        UsagesKind::References => {
            let references = session
                .project
                .update(cx, |project, cx| project.references(buffer, position, cx));
            let definitions = session
                .project
                .update(cx, |project, cx| project.definitions(buffer, position, cx));
            PendingQuery {
                references: cx
                    .spawn(async move |_| anyhow::Ok(references.await?.unwrap_or_default())),
                declarations: cx.spawn(async move |_| match definitions.await {
                    Ok(links) => (
                        links
                            .unwrap_or_default()
                            .into_iter()
                            .map(|link| link.target)
                            .collect(),
                        None,
                    ),
                    Err(error) => (Vec::new(), Some(error)),
                }),
            }
        }
        UsagesKind::Implementations => {
            let implementations = session.project.update(cx, |project, cx| {
                project.implementations(buffer, position, cx)
            });
            PendingQuery {
                references: cx.spawn(async move |_| {
                    let links = implementations.await?.unwrap_or_default();
                    anyhow::Ok(links.into_iter().map(|link| link.target).collect())
                }),
                declarations: Task::ready((Vec::new(), None)),
            }
        }
    }
}

async fn resolve_lsp_locations(
    project: &Entity<Project>,
    server_id: LanguageServerId,
    lsp_locations: Vec<lsp::Location>,
    cx: &mut AsyncWindowContext,
) -> (Vec<Location>, Option<anyhow::Error>) {
    let mut opening = Vec::with_capacity(lsp_locations.len());
    for lsp_location in lsp_locations {
        let buffer = project.update(cx, |project, cx| {
            project.open_local_buffer_via_lsp(lsp_location.uri, server_id, cx)
        });
        opening.push((buffer, lsp_location.range));
    }
    let mut locations = Vec::with_capacity(opening.len());
    let mut first_error = None;
    for (buffer, lsp_range) in opening {
        match buffer.await {
            Ok(buffer) => {
                let range = buffer.read_with(cx, |buffer, _| {
                    let range = language::range_from_lsp(lsp_range);
                    let start = buffer.clip_point_utf16(range.start, Bias::Left);
                    let end = buffer.clip_point_utf16(range.end, Bias::Left);
                    buffer.anchor_after(start)..buffer.anchor_before(end)
                });
                locations.push(Location { buffer, range });
            }
            Err(error) => {
                log::error!("Show Usages failed to open a usage location: {error:#}");
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    (locations, first_error)
}

fn finish_for(model: &UsagesModel<Location>) -> Finish {
    let scope = model.scope();
    if model.in_scope_count() == 0 {
        if model.out_of_scope_count() == 0 {
            return Finish::Hint {
                message: no_usages_message(scope),
                warning: false,
                target: None,
            };
        }
        let out_of_scope_usages = (0..)
            .map_while(|row_index| model.row(row_index))
            .map(|row| row.raw_indices.len())
            .sum();
        return Finish::Hint {
            message: out_of_scope_message(out_of_scope_usages, scope),
            warning: true,
            target: None,
        };
    }
    if model.visible_count() == 1
        && model.out_of_scope_count() == 0
        && model.filtered_out_count() == 0
        && let Some(target) = model.visible_locations().into_iter().next()
    {
        let usage_count = model.visible_usage_count();
        let message = if usage_count > 1 {
            all_usages_in_line_message(usage_count, scope)
        } else {
            only_usage_message(scope)
        };
        return Finish::Hint {
            message,
            warning: false,
            target: Some(target),
        };
    }
    Finish::Popup
}

fn apply_access(raw_usages: &mut [RawUsage<Location>], accesses: &[(usize, UsageAccess)]) {
    for (raw_index, access) in accesses {
        if let Some(usage) = raw_usages.get_mut(*raw_index) {
            usage.access = *access;
        }
    }
}

fn deliver_access(
    session: &UsagesSession,
    popup: &WeakEntity<ShowUsagesPopup>,
    outcome: AccessOutcome,
    cx: &mut AsyncWindowContext,
) {
    let Some(popup) = popup.upgrade() else {
        return;
    };
    let AccessOutcome { accesses, error } = outcome;
    if !accesses.is_empty() {
        popup.update(cx, |popup, cx| popup.set_accesses(accesses, cx));
    }
    if let Some(error) = error {
        report_error(&session.workspace, error, cx);
    }
}

fn report_error(
    workspace: &WeakEntity<Workspace>,
    error: anyhow::Error,
    cx: &mut AsyncWindowContext,
) {
    log::error!("Show Usages failed: {error:#}");
    workspace
        .update(cx, |workspace, cx| workspace.show_error(error, cx))
        .log_err();
}

fn open_popup(
    session: &UsagesSession,
    cx: &mut AsyncWindowContext,
) -> Option<WeakEntity<ShowUsagesPopup>> {
    let workspace = session.workspace.upgrade()?;
    let session = session.clone();
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            if workspace.active_modal::<ShowUsagesPopup>(cx).is_some() {
                workspace.hide_modal(window, cx);
            }
            workspace.toggle_modal(window, cx, |window, cx| {
                ShowUsagesPopup::new(session, window, cx)
            });
            workspace
                .active_modal::<ShowUsagesPopup>(cx)
                .map(|popup| popup.downgrade())
        })
    })
    .ok()
    .flatten()
}

fn popup_dismissed(
    session: &UsagesSession,
    popup: Option<&WeakEntity<ShowUsagesPopup>>,
    cx: &AsyncWindowContext,
) -> bool {
    let Some(popup) = popup else {
        return false;
    };
    let Some(workspace) = session.workspace.upgrade() else {
        return true;
    };
    !workspace.read_with(cx, |workspace, cx| {
        workspace
            .active_modal::<ShowUsagesPopup>(cx)
            .is_some_and(|active| active.entity_id() == popup.entity_id())
    })
}

fn close_popup(
    session: &UsagesSession,
    popup: Option<&WeakEntity<ShowUsagesPopup>>,
    window: &mut Window,
    cx: &mut App,
) {
    let (Some(popup), Some(workspace)) = (popup, session.workspace.upgrade()) else {
        return;
    };
    workspace.update(cx, |workspace, cx| {
        if workspace
            .active_modal::<ShowUsagesPopup>(cx)
            .is_some_and(|active| active.entity_id() == popup.entity_id())
        {
            workspace.hide_modal(window, cx);
        }
    });
}

fn dismiss_usages_modals(workspace: &WeakEntity<Workspace>, window: &mut Window, cx: &mut App) {
    let Some(workspace) = workspace.upgrade() else {
        return;
    };
    workspace.update(cx, |workspace, cx| {
        let usages_modal_active = workspace.active_modal::<ShowUsagesPopup>(cx).is_some()
            || workspace.active_modal::<UsagesHint>(cx).is_some()
            || workspace.active_modal::<FindUsagesDialog>(cx).is_some();
        if usages_modal_active {
            workspace.hide_modal(window, cx);
        }
    });
}

fn show_hint(
    session: &UsagesSession,
    message: String,
    warning: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(workspace) = session.workspace.upgrade() else {
        return;
    };
    let origin_editor = session.origin_editor.upgrade();
    let Some(editor) = workspace
        .read(cx)
        .active_item_as::<Editor>(cx)
        .or_else(|| origin_editor.clone())
    else {
        return;
    };
    let editor_focus_handles = [Some(&editor), origin_editor.as_ref()]
        .into_iter()
        .flatten()
        .map(|candidate| candidate.focus_handle(cx))
        .collect::<Vec<_>>();
    let editor_shortcut = |action: &dyn Action| {
        editor_focus_handles
            .iter()
            .find_map(|focus_handle| action_keystroke_text(action, focus_handle, window))
    };
    let second_invocation = editor_shortcut(&ShowUsages::default())
        .and_then(|shortcut| second_invocation_hint(&shortcut, session.scope))
        .map(SharedString::from);
    let find_options_shortcut = editor_shortcut(&ShowSettings);
    let content = HintContent {
        message: SharedString::from(message),
        warning,
        second_invocation,
        find_options_shortcut,
        session: session.clone(),
    };
    let position = session.popup_position;
    workspace.update(cx, |workspace, cx| {
        if workspace.active_modal::<UsagesHint>(cx).is_some() {
            workspace.hide_modal(window, cx);
        }
        workspace.toggle_modal(window, cx, |window, cx| {
            UsagesHint::new(content, position, editor, window, cx)
        });
    });
}

fn continue_in_maximal_scope(
    workspace: &Entity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let Some(hint) = workspace.read(cx).active_modal::<UsagesHint>(cx) else {
        return false;
    };
    let session = hint.read(cx).session().clone();
    restart(session, UsageScope::maximal(), window, cx);
    true
}

fn start_navigation(
    session: &UsagesSession,
    location: Location,
    window: &mut Window,
    cx: &mut App,
) -> Option<Task<anyhow::Result<Navigated>>> {
    if let Some(editor) = session.origin_editor.upgrade() {
        let same_buffer =
            editor.read(cx).buffer().read(cx).as_singleton().as_ref() == Some(&location.buffer);
        if same_buffer {
            window.focus(&editor.focus_handle(cx), cx);
        }
        return Some(editor.update(cx, |editor, cx| {
            editor.open_location(location, false, window, cx)
        }));
    }
    let workspace = session.workspace.upgrade()?;
    workspace.update(cx, |workspace, cx| {
        let preview_tabs = PreviewTabsSettings::get_global(cx);
        let keep_old_preview = preview_tabs.enable_keep_preview_on_code_navigation;
        let allow_new_preview = preview_tabs.enable_preview_file_from_code_navigation;
        let buffer = location.buffer.read(cx);
        let start = buffer.summary_for_anchor::<language::Point>(&location.range.start);
        let end = buffer.summary_for_anchor::<language::Point>(&location.range.end);
        let range = start.min(end)..start.max(end);
        let editor = workspace.open_project_item::<Editor>(
            None,
            location.buffer,
            true,
            true,
            keep_old_preview,
            allow_new_preview,
            window,
            cx,
        );
        editor.update(cx, |editor, cx| {
            editor.go_to_singleton_buffer_range(range, window, cx)
        });
    });
    None
}

fn usage_context(session: &UsagesSession, cx: &App) -> UsageContext {
    let (origin, current_buffer) = match &session.target {
        UsagesTarget::Symbol { buffer, position } => {
            (Some((buffer.clone(), *position)), Some(buffer.clone()))
        }
        UsagesTarget::Locations { .. } => (
            None,
            session
                .origin_editor
                .upgrade()
                .and_then(|editor| caret_position(&editor, cx))
                .map(|(buffer, _)| buffer),
        ),
    };
    let open_buffers: collections::HashSet<EntityId> = session
        .workspace
        .upgrade()
        .map(|workspace| {
            workspace
                .read(cx)
                .items(cx)
                .filter(|item| item.buffer_kind(cx) == ItemBufferKind::Singleton)
                .flat_map(|item| item.project_item_model_ids(cx))
                .collect()
        })
        .unwrap_or_default();
    let visible_worktrees: Vec<project::WorktreeId> = session
        .project
        .read(cx)
        .visible_worktrees(cx)
        .map(|worktree| worktree.read(cx).id())
        .collect();
    UsageContext {
        origin,
        current_buffer,
        open_buffers,
        visible_worktrees,
    }
}

fn fallback_symbol_name(
    session: &UsagesSession,
    locations: &[Location],
    cx: &App,
) -> Option<SharedString> {
    let location = match &session.target {
        UsagesTarget::Symbol { buffer, position } => {
            let buffer_state = buffer.read(cx);
            let caret = buffer_state.summary_for_anchor::<usize>(position);
            locations.iter().find(|location| {
                &location.buffer == buffer && {
                    let start = buffer_state.summary_for_anchor::<usize>(&location.range.start);
                    let end = buffer_state.summary_for_anchor::<usize>(&location.range.end);
                    start.min(end) <= caret && caret <= start.max(end)
                }
            })
        }
        UsagesTarget::Locations { .. } => locations.first(),
    }?;
    let text = location
        .buffer
        .read(cx)
        .text_for_range(location.range.clone())
        .collect::<String>();
    let text = text.trim();
    (!text.is_empty() && !text.contains('\n')).then(|| SharedString::from(text))
}

fn caret_position(editor: &Entity<Editor>, cx: &App) -> Option<(Entity<Buffer>, text::Anchor)> {
    let editor = editor.read(cx);
    let head = editor.selections.newest_anchor().head();
    editor.buffer().read(cx).text_anchor_for_position(head, cx)
}

fn popup_position_below_caret(
    editor: &Entity<Editor>,
    window: &mut Window,
    cx: &mut App,
) -> Point<Pixels> {
    editor
        .update(cx, |editor, cx| {
            editor.popup_origin_below_cursor(window, cx)
        })
        .or_else(|| editor.read(cx).last_bounds().map(|bounds| bounds.center()))
        .unwrap_or_else(|| window.viewport_size().center())
}

fn symbol_at(
    buffer: &Entity<Buffer>,
    position: text::Anchor,
    project: &Entity<Project>,
    cx: &App,
) -> SymbolDescription {
    let buffer = buffer.read(cx);
    let snapshot = buffer.snapshot();
    let offset = snapshot.summary_for_anchor::<usize>(&position);
    let path = collect::display_path(buffer, project.read(cx), cx);
    symbol::describe_symbol_at(&snapshot, offset, path)
}

fn declaration_at(
    buffer: &Entity<Buffer>,
    position: text::Anchor,
    project: &Entity<Project>,
    cx: &App,
) -> SymbolDescription {
    let buffer = buffer.read(cx);
    let snapshot = buffer.snapshot();
    let offset = snapshot.summary_for_anchor::<usize>(&position);
    match collect::display_path(buffer, project.read(cx), cx) {
        Some(path) => symbol::describe_declaration(&snapshot, offset..offset, path),
        None => symbol::describe_symbol_at(&snapshot, offset, None),
    }
}

fn describe_location(
    location: &Location,
    project: &Entity<Project>,
    cx: &App,
) -> Option<SymbolDescription> {
    let buffer = location.buffer.read(cx);
    let path = collect::display_path(buffer, project.read(cx), cx)?;
    let snapshot = buffer.snapshot();
    let start = snapshot.summary_for_anchor::<usize>(&location.range.start);
    let end = snapshot.summary_for_anchor::<usize>(&location.range.end);
    Some(symbol::describe_declaration(
        &snapshot,
        start.min(end)..start.max(end),
        path,
    ))
}

fn publish_symbol(
    session: &UsagesSession,
    popup: Option<&WeakEntity<ShowUsagesPopup>>,
    cx: &mut AsyncWindowContext,
) {
    if let Some(popup) = popup.and_then(WeakEntity::upgrade) {
        let symbol = session.symbol.clone();
        popup.update(cx, |popup, cx| popup.set_symbol(symbol, cx));
    }
}

fn binding_text(binding: &KeyBinding) -> SharedString {
    let keystrokes = binding
        .keystrokes()
        .iter()
        .map(|keystroke| keystroke_text(keystroke.modifiers(), keystroke.key()))
        .collect::<Vec<_>>();
    SharedString::from(keystrokes.join(" "))
}

fn keystroke_text(modifiers: &Modifiers, key: &str) -> String {
    let mut text = String::new();
    if modifiers.function {
        text.push_str("Fn");
    }
    if modifiers.control {
        text.push('⌃');
    }
    if modifiers.alt {
        text.push('⌥');
    }
    if modifiers.shift {
        text.push('⇧');
    }
    if modifiers.platform {
        text.push('⌘');
    }
    text.push_str(&key_label(key));
    text
}

fn key_label(key: &str) -> String {
    let glyph = match key {
        "left" => "←",
        "right" => "→",
        "up" => "↑",
        "down" => "↓",
        "backspace" => "⌫",
        "delete" => "⌦",
        "enter" | "return" => "↩",
        "tab" => "⇥",
        "escape" => "⎋",
        "space" => "Space",
        "pageup" => "⇞",
        "pagedown" => "⇟",
        "home" => "↖",
        "end" => "↘",
        _ => {
            let mut characters = key.chars();
            return match characters.next() {
                Some(first) => first.to_uppercase().chain(characters).collect(),
                None => String::new(),
            };
        }
    };
    glyph.to_string()
}
