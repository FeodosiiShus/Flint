pub(crate) mod actions;
mod change_view;
mod context_menu;
mod editors;
mod fold_sync;
mod folding;
mod inner_diff;
mod movement;
mod navigation;
mod operations;
pub(crate) mod panes;
mod rediff;
mod scroll;
mod scroll_sync;
mod selection;
#[cfg(test)]
mod tests;
mod toolbar;
pub(crate) mod viewer_settings;

use editor::{Anchor, Editor, EditorEvent};
use gpui::{
    AnyWindowHandle, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Pixels, Point,
    Size, Subscription, Window, point,
};
use merge_diff::{ComparisonPolicy, MergeInnerDifferences, ThreeSide};
use ui::{ContextMenu, PopoverMenuHandle};
use util::ResultExt as _;

use self::folding::FoldState;
use self::inner_diff::InnerDiffWorker;
use self::viewer_settings::ViewerSettings;
use super::merge_model::{MergeConflictModel, MergeModelEvent};
use super::merge_window::{MergePaneTitle, MergeResult, layout::Proportions};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MergeViewerEvent {
    FinishRequested(MergeResult),
    RestartConfirmationRequested(ComparisonPolicy),
    RevertConfirmationRequested,
    RediffCanceled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RediffState {
    Idle,
    Running,
    Finished,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewerNotification {
    DiffTooBig,
    ReadOnlyFile,
}

impl ViewerNotification {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::DiffTooBig => {
                "Unable to calculate diff. File is too big and there are too many changes."
            }
            Self::ReadOnlyFile => "Cannot resolve conflicts in a read-only file",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Appearance {
    pub soft_wrap: bool,
    pub indent_guides: bool,
    pub show_whitespaces: bool,
    pub line_numbers: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            soft_wrap: false,
            indent_guides: false,
            show_whitespaces: false,
            line_numbers: true,
        }
    }
}

pub(crate) struct MergeViewer {
    model: Entity<MergeConflictModel>,
    editors: [Entity<Editor>; 3],
    titles: [MergePaneTitle; 3],
    current_side: ThreeSide,
    rediff: RediffState,
    rediff_generation: u64,
    error_content: bool,
    notification: Option<ViewerNotification>,
    notification_hidden: bool,
    inner_fragments: Vec<Option<MergeInnerDifferences>>,
    inner_worker: InnerDiffWorker,
    sync_scroll_enabled: bool,
    expand_by_default: bool,
    ignore_policy: ComparisonPolicy,
    highlight_by_word: bool,
    appearance: Appearance,
    folds: Option<FoldState>,
    fold_anchors: Vec<[Option<Anchor>; 3]>,
    last_scroll_positions: [Point<f64>; 3],
    programmatic_scrolls: [usize; 3],
    proportions: Proportions,
    balloon_visible: bool,
    balloon_shown_at_keystroke: u64,
    keystrokes_seen: u64,
    viewport_size: Size<Pixels>,
    highlight_refresh_pending: bool,
    window_handle: AnyWindowHandle,
    settings_menu: PopoverMenuHandle<ContextMenu>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<MergeViewerEvent> for MergeViewer {}

impl Focusable for MergeViewer {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editors[ThreeSide::Base.index()].focus_handle(cx)
    }
}

impl MergeViewer {
    pub(crate) fn new(
        model: Entity<MergeConflictModel>,
        titles: [MergePaneTitle; 3],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let stored_settings = viewer_settings::load(cx);
        let ignore_policy = Self::initial_ignore_policy(&model, &stored_settings, cx);
        let editors = editors::create_pane_editors(&model, stored_settings.appearance, window, cx);
        let mut viewer = Self {
            model,
            editors,
            titles,
            current_side: ThreeSide::Base,
            rediff: RediffState::Idle,
            error_content: false,
            notification: None,
            notification_hidden: false,
            inner_fragments: Vec::new(),
            inner_worker: InnerDiffWorker::default(),
            sync_scroll_enabled: true,
            expand_by_default: stored_settings.expand_by_default,
            ignore_policy,
            highlight_by_word: stored_settings.highlight_by_word,
            appearance: stored_settings.appearance,
            folds: None,
            fold_anchors: Vec::new(),
            rediff_generation: 0,
            last_scroll_positions: [point(0.0, 0.0); 3],
            programmatic_scrolls: [0; 3],
            proportions: Proportions::default(),
            balloon_visible: false,
            balloon_shown_at_keystroke: 0,
            keystrokes_seen: 0,
            viewport_size: window.viewport_size(),
            highlight_refresh_pending: false,
            window_handle: window.window_handle(),
            settings_menu: PopoverMenuHandle::default(),
            _subscriptions: Vec::new(),
        };
        viewer._subscriptions = viewer.subscribe_to_children(window, cx);
        context_menu::install_context_menus(&viewer, cx);
        viewer.start_rediff(window, cx);
        viewer
    }

    fn initial_ignore_policy(
        model: &Entity<MergeConflictModel>,
        stored_settings: &ViewerSettings,
        cx: &App,
    ) -> ComparisonPolicy {
        let model = model.read(cx);
        if !model.is_initialized() {
            return stored_settings.ignore_policy;
        }
        let cached_policy = model.ignore_policy();
        if cached_policy != stored_settings.ignore_policy {
            viewer_settings::store_ignore_policy(cached_policy, cx);
        }
        cached_policy
    }

    pub(crate) fn editor(&self, side: ThreeSide) -> &Entity<Editor> {
        &self.editors[side.index()]
    }

    pub(crate) fn result_editor(&self) -> &Entity<Editor> {
        self.editor(ThreeSide::Base)
    }

    pub(crate) fn rediff_state(&self) -> RediffState {
        self.rediff
    }

    pub(crate) fn appearance(&self) -> Appearance {
        self.appearance
    }

    fn subscribe_to_children(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        let mut subscriptions = vec![
            cx.subscribe(
                &self.model,
                |viewer, _model, event: &MergeModelEvent, cx| viewer.on_model_event(event, cx),
            ),
            cx.observe_keystrokes(|viewer, _event, window, cx| {
                viewer.on_keystroke(window, cx);
            }),
            cx.observe_window_bounds(window, |viewer, window, cx| {
                viewer.on_window_bounds_changed(window, cx);
            }),
        ];
        for side in ThreeSide::ALL {
            let editor = self.editor(side).clone();
            subscriptions.push(cx.subscribe_in(
                &editor,
                window,
                move |viewer, _editor, event: &EditorEvent, window, cx| {
                    viewer.on_editor_event(side, event, window, cx);
                },
            ));
            let focus_handle = editor.focus_handle(cx);
            subscriptions.push(cx.on_focus_in(&focus_handle, window, move |viewer, _, cx| {
                viewer.current_side = side;
                cx.notify();
            }));
        }
        subscriptions
    }

    fn on_editor_event(
        &mut self,
        side: ThreeSide,
        event: &EditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            EditorEvent::ScrollPositionChanged { local: true, .. } => {
                self.on_pane_scrolled(side, window, cx);
            }
            EditorEvent::SelectionsChanged { .. } => cx.notify(),
            _ => {}
        }
    }

    fn on_model_event(&mut self, event: &MergeModelEvent, cx: &mut Context<Self>) {
        match event {
            MergeModelEvent::ChangeResolved(_) => {
                let all_processed = self
                    .model
                    .read(cx)
                    .counters()
                    .is_some_and(|counters| counters.is_all_resolved());
                if all_processed && !self.balloon_visible {
                    self.balloon_shown_at_keystroke = self.keystrokes_seen;
                }
                self.balloon_visible = all_processed;
            }
            MergeModelEvent::ChangeProcessed(index) => self.schedule_inner_diff(*index, cx),
            MergeModelEvent::ChangeReset(_)
            | MergeModelEvent::ChangeSideResolved { .. }
            | MergeModelEvent::Rediffed
            | MergeModelEvent::BulkProcessingFinished
            | MergeModelEvent::ContentModifiedChanged(_) => {}
        }
        self.request_highlight_refresh(cx);
    }

    fn request_highlight_refresh(&mut self, cx: &mut Context<Self>) {
        if self.highlight_refresh_pending {
            return;
        }
        self.highlight_refresh_pending = true;
        cx.spawn(async move |viewer, cx| {
            viewer
                .update(cx, |viewer, cx| {
                    viewer.highlight_refresh_pending = false;
                    viewer.refresh_highlights(cx);
                })
                .log_err();
        })
        .detach();
    }

    fn on_keystroke(&mut self, window: &Window, cx: &mut Context<Self>) {
        if window.window_handle() != self.window_handle {
            return;
        }
        let shown_by_this_keystroke =
            self.balloon_visible && self.balloon_shown_at_keystroke == self.keystrokes_seen;
        self.keystrokes_seen += 1;
        if !shown_by_this_keystroke {
            self.dismiss_balloon(cx);
        }
    }

    fn on_window_bounds_changed(&mut self, window: &Window, cx: &mut Context<Self>) {
        let viewport_size = window.viewport_size();
        if viewport_size != self.viewport_size {
            self.viewport_size = viewport_size;
            self.dismiss_balloon(cx);
        }
    }

    pub(crate) fn dismiss_balloon(&mut self, cx: &mut Context<Self>) {
        if self.balloon_visible {
            self.balloon_visible = false;
            cx.notify();
        }
    }

    pub(crate) fn request_finish_from_balloon(&mut self, cx: &mut Context<Self>) {
        if self.rediff != RediffState::Finished {
            return;
        }
        self.balloon_visible = false;
        cx.emit(MergeViewerEvent::FinishRequested(MergeResult::Resolved));
    }
}
