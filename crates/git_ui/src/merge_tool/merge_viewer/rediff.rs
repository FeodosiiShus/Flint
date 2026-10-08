use gpui::{Context, Window};
use language::Point;
use merge_diff::{ComparisonPolicy, ThreeSide};
use project::project_settings::ProjectSettings;
use settings::Settings as _;
use util::ResultExt as _;

use super::{MergeViewer, MergeViewerEvent, RediffState, ViewerNotification};
use crate::merge_tool::merge_model::MergeModelError;

impl MergeViewer {
    pub(super) fn start_rediff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let policy = self.ignore_policy;
        let task = self.model.update(cx, |model, cx| model.rediff(policy, cx));
        self.begin_rediff(task, window, cx);
    }

    pub(crate) fn restart_merge(
        &mut self,
        policy: ComparisonPolicy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ignore_policy = policy;
        super::viewer_settings::store_ignore_policy(policy, cx);
        self.remove_folds(cx);
        let task = self
            .model
            .update(cx, |model, cx| model.restart_with_policy(policy, cx));
        self.begin_rediff(task, window, cx);
    }

    fn begin_rediff(
        &mut self,
        task: gpui::Task<Result<(), MergeModelError>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rediff_generation += 1;
        let generation = self.rediff_generation;
        self.rediff = RediffState::Running;
        self.error_content = false;
        self.notification = None;
        self.notification_hidden = false;
        self.balloon_visible = false;
        self.disable_inner_diff();
        self.set_result_editable(false, cx);
        cx.spawn_in(window, async move |viewer, cx| {
            let outcome = task.await;
            viewer
                .update_in(cx, |viewer, window, cx| {
                    if viewer.rediff_generation == generation {
                        viewer.finish_rediff(outcome, window, cx);
                    }
                })
                .log_err();
        })
        .detach();
        cx.notify();
    }

    fn finish_rediff(
        &mut self,
        outcome: Result<(), MergeModelError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match outcome {
            Ok(()) => self.complete_rediff(window, cx),
            Err(MergeModelError::Canceled) => {
                self.rediff = RediffState::Failed;
                cx.emit(MergeViewerEvent::RediffCanceled);
            }
            Err(MergeModelError::DiffTooBig) => {
                self.fail_rediff(Some(ViewerNotification::DiffTooBig), false, cx)
            }
            Err(MergeModelError::ReadOnlyResult) => {
                self.fail_rediff(Some(ViewerNotification::ReadOnlyFile), false, cx)
            }
            Err(MergeModelError::InconsistentFragments) => self.fail_rediff(None, true, cx),
        }
        cx.notify();
    }

    fn any_pane_was_scrolled(&self, cx: &mut Context<Self>) -> bool {
        ThreeSide::ALL.into_iter().any(|side| {
            let position = self.pane_scroll_position(side, cx);
            let caret = self.caret_point(side, cx);
            position.x != 0.0 || position.y != 0.0 || caret != Point::new(0, 0)
        })
    }

    fn complete_rediff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let is_initial_rediff = self.rediff_generation == 1;
        self.rediff = RediffState::Finished;
        self.set_result_editable(true, cx);
        let change_count = self.model.read(cx).changes().len();
        self.inner_fragments = vec![None; change_count];
        self.install_folds(window, cx);
        self.refresh_highlights(cx);
        if is_initial_rediff {
            cx.on_next_frame(window, |viewer, window, cx| {
                if !viewer.any_pane_was_scrolled(cx) {
                    viewer.scroll_to_first_unresolved_change(window, cx);
                }
            });
        }
        self.inner_diff_everything_changed(cx);
        if ProjectSettings::get_global(cx)
            .git
            .merge_tool
            .auto_apply_non_conflicting
        {
            self.apply_non_conflicting(ThreeSide::Base, window, cx);
        }
    }

    fn fail_rediff(
        &mut self,
        notification: Option<ViewerNotification>,
        error_content: bool,
        cx: &mut Context<Self>,
    ) {
        self.rediff = RediffState::Failed;
        self.notification = notification;
        self.error_content = error_content;
        self.remove_folds(cx);
        self.inner_fragments.clear();
        self.refresh_highlights(cx);
    }

    pub(crate) fn visible_notification(&self) -> Option<ViewerNotification> {
        if self.notification_hidden {
            None
        } else {
            self.notification
        }
    }

    pub(crate) fn hide_notification(&mut self, cx: &mut Context<Self>) {
        self.notification_hidden = true;
        cx.notify();
    }

    pub(crate) fn has_error_content(&self) -> bool {
        self.error_content
    }
}
