use std::time::Duration;

use gpui::{Animation, AnimationExt, Entity, EntityId, Hsla, Subscription, WeakEntity};
use ui::{Tooltip, prelude::*};

use crate::Workspace;

const PROGRESS_BAR_WIDTH: Pixels = px(104.);
const PROGRESS_BAR_HEIGHT: Pixels = px(4.);
const INDETERMINATE_SEGMENT_FRACTION: f32 = 0.3;
const INDETERMINATE_CYCLE: Duration = Duration::from_millis(1200);
const PILL_CORNER_RADIUS: Pixels = px(6.);
const PILL_VERTICAL_PADDING: Pixels = px(6.);
const PILL_HORIZONTAL_PADDING: Pixels = px(10.);
const PILL_CONTENT_GAP: Pixels = px(10.);

pub struct BackgroundTaskProgress {
    title: SharedString,
    fraction: Option<f32>,
    details: Option<SharedString>,
}

impl BackgroundTaskProgress {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            fraction: None,
            details: None,
        }
    }

    pub fn title(&self) -> &SharedString {
        &self.title
    }

    pub fn fraction(&self) -> Option<f32> {
        self.fraction
    }

    pub fn details(&self) -> Option<&SharedString> {
        self.details.as_ref()
    }

    pub fn set_progress(
        &mut self,
        fraction: f32,
        details: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.fraction = Some(clamp_fraction(fraction));
        self.details = Some(details.into());
        cx.notify();
    }
}

fn clamp_fraction(fraction: f32) -> f32 {
    if fraction.is_nan() {
        0.0
    } else {
        fraction.clamp(0.0, 1.0)
    }
}

#[derive(Default)]
pub(crate) struct BackgroundTasks {
    entries: Vec<BackgroundTaskEntry>,
}

struct BackgroundTaskEntry {
    task_id: EntityId,
    indicator: Entity<BackgroundTaskIndicator>,
    _release_subscription: Subscription,
}

impl Workspace {
    pub fn show_background_task(
        &mut self,
        task: &Entity<BackgroundTaskProgress>,
        cx: &mut Context<Self>,
    ) {
        let task_id = task.entity_id();
        if self
            .background_tasks
            .entries
            .iter()
            .any(|entry| entry.task_id == task_id)
        {
            return;
        }

        let indicator = cx.new(|cx| BackgroundTaskIndicator::new(task, cx));
        let release_subscription = cx.observe_release(task, move |workspace, _, cx| {
            workspace
                .background_tasks
                .entries
                .retain(|entry| entry.task_id != task_id);
            cx.notify();
        });
        self.background_tasks.entries.push(BackgroundTaskEntry {
            task_id,
            indicator,
            _release_subscription: release_subscription,
        });
        cx.notify();
    }

    pub(crate) fn render_background_tasks(&self) -> Option<Div> {
        let entries = &self.background_tasks.entries;
        if entries.is_empty() {
            return None;
        }

        let indicators = entries.iter().enumerate().map(render_background_task_entry);
        Some(v_flex().items_end().gap_2().children(indicators))
    }
}

fn render_background_task_entry((index, entry): (usize, &BackgroundTaskEntry)) -> Div {
    div()
        .debug_selector(move || format!("background_task_progress_{index}"))
        .child(entry.indicator.clone())
}

struct BackgroundTaskIndicator {
    task: WeakEntity<BackgroundTaskProgress>,
    _task_observation: Subscription,
}

impl BackgroundTaskIndicator {
    fn new(task: &Entity<BackgroundTaskProgress>, cx: &mut Context<Self>) -> Self {
        Self {
            task: task.downgrade(),
            _task_observation: cx.observe(task, |_, _, cx| cx.notify()),
        }
    }
}

impl Render for BackgroundTaskIndicator {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(task) = self.task.upgrade() else {
            return div().into_any_element();
        };
        let progress = task.read(cx);
        let colors = cx.theme().colors();
        let indicator_id = cx.entity_id();
        let accent = colors.text_accent;

        let progress_bar = div()
            .relative()
            .flex_none()
            .w(PROGRESS_BAR_WIDTH)
            .h(PROGRESS_BAR_HEIGHT)
            .rounded_full()
            .overflow_hidden()
            .bg(colors.border)
            .child(match progress.fraction() {
                Some(fraction) => determinate_fill(fraction, accent).into_any_element(),
                None => indeterminate_segment(indicator_id, accent).into_any_element(),
            });

        h_flex()
            .id(("background_task_progress", indicator_id))
            .occlude()
            .gap(PILL_CONTENT_GAP)
            .py(PILL_VERTICAL_PADDING)
            .px(PILL_HORIZONTAL_PADDING)
            .rounded(PILL_CORNER_RADIUS)
            .border_1()
            .border_color(colors.border_variant)
            .bg(colors.elevated_surface_background)
            .shadow_sm()
            .child(Label::new(progress.title().clone()).size(LabelSize::Small))
            .child(progress_bar)
            .when_some(progress.details().cloned(), |this, details| {
                this.tooltip(Tooltip::text(details))
            })
            .into_any_element()
    }
}

fn determinate_fill(fraction: f32, accent: Hsla) -> Div {
    div()
        .debug_selector(|| "background_task_progress_fill".into())
        .h_full()
        .w(relative(fraction))
        .rounded_full()
        .bg(accent)
}

fn indeterminate_segment(indicator_id: EntityId, accent: Hsla) -> impl IntoElement {
    div()
        .debug_selector(|| "background_task_progress_indeterminate".into())
        .absolute()
        .top_0()
        .h_full()
        .w(relative(INDETERMINATE_SEGMENT_FRACTION))
        .rounded_full()
        .bg(accent)
        .with_animation(
            ("background_task_progress_indeterminate", indicator_id),
            Animation::new(INDETERMINATE_CYCLE).repeat(),
            |segment, delta| segment.left(relative(indeterminate_segment_offset(delta))),
        )
}

fn indeterminate_segment_offset(delta: f32) -> f32 {
    delta * (1.0 + INDETERMINATE_SEGMENT_FRACTION) - INDETERMINATE_SEGMENT_FRACTION
}

#[cfg(test)]
mod tests {
    use fs::FakeFs;
    use gpui::{TestAppContext, VisualTestContext};
    use project::Project;

    use super::*;
    use crate::tests::init_test;

    async fn workspace_window(
        cx: &mut TestAppContext,
    ) -> (Entity<Workspace>, &mut VisualTestContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        cx.add_window_view(|window, cx| Workspace::test_new(project, window, cx))
    }

    fn tracked_task_ids(
        workspace: &Entity<Workspace>,
        cx: &mut VisualTestContext,
    ) -> Vec<EntityId> {
        workspace.read_with(cx, |workspace, _| {
            workspace
                .background_tasks
                .entries
                .iter()
                .map(|entry| entry.task_id)
                .collect()
        })
    }

    #[gpui::test]
    async fn test_background_task_is_shown_until_its_last_handle_is_dropped(
        cx: &mut TestAppContext,
    ) {
        let (workspace, cx) = workspace_window(cx).await;
        assert_eq!(
            cx.debug_bounds("background_task_progress_0"),
            None,
            "no indicator is rendered before a task is shown"
        );

        let task = cx.new(|_| BackgroundTaskProgress::new("Pushing…"));
        workspace.update(cx, |workspace, cx| {
            workspace.show_background_task(&task, cx)
        });
        cx.run_until_parked();

        assert_eq!(tracked_task_ids(&workspace, cx), vec![task.entity_id()]);
        assert!(
            cx.debug_bounds("background_task_progress_0").is_some(),
            "a shown task renders its indicator"
        );

        let second_handle = task.clone();
        cx.update(|_, _| drop(task));
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("background_task_progress_0").is_some(),
            "the indicator stays while any strong handle is alive"
        );

        cx.update(|_, _| drop(second_handle));
        cx.run_until_parked();
        assert!(
            tracked_task_ids(&workspace, cx).is_empty(),
            "dropping the last handle forgets the task"
        );
        assert_eq!(
            cx.debug_bounds("background_task_progress_0"),
            None,
            "the workspace re-renders without the indicator once the task is released"
        );
    }

    #[gpui::test]
    fn test_background_task_set_progress_clamps_fraction_and_stores_details(
        cx: &mut TestAppContext,
    ) {
        let task = cx.new(|_| BackgroundTaskProgress::new("Pushing…"));
        task.read_with(cx, |task, _| {
            assert_eq!(task.title().as_str(), "Pushing…");
            assert_eq!(task.fraction(), None, "a new task is indeterminate");
            assert_eq!(task.details(), None);
        });

        task.update(cx, |task, cx| {
            task.set_progress(1.7, "Writing objects: 100% (3/3)", cx)
        });
        task.read_with(cx, |task, _| {
            assert_eq!(task.fraction(), Some(1.0));
            assert_eq!(
                task.details().map(SharedString::as_str),
                Some("Writing objects: 100% (3/3)")
            );
        });

        task.update(cx, |task, cx| {
            task.set_progress(-0.5, "Counting objects: 1", cx)
        });
        task.read_with(cx, |task, _| assert_eq!(task.fraction(), Some(0.0)));

        task.update(cx, |task, cx| {
            task.set_progress(f32::NAN, "Resolving deltas", cx)
        });
        task.read_with(cx, |task, _| assert_eq!(task.fraction(), Some(0.0)));

        task.update(cx, |task, cx| {
            task.set_progress(0.4, "Writing objects:  33% (1/3)", cx)
        });
        task.read_with(cx, |task, _| {
            assert_eq!(task.fraction(), Some(0.4));
            assert_eq!(
                task.details().map(SharedString::as_str),
                Some("Writing objects:  33% (1/3)")
            );
        });
    }

    #[gpui::test]
    async fn test_background_task_becomes_determinate_after_progress(cx: &mut TestAppContext) {
        let (workspace, cx) = workspace_window(cx).await;
        let task = cx.new(|_| BackgroundTaskProgress::new("Pushing…"));
        workspace.update(cx, |workspace, cx| {
            workspace.show_background_task(&task, cx)
        });
        cx.run_until_parked();

        assert!(
            cx.debug_bounds("background_task_progress_indeterminate")
                .is_some(),
            "the bar is indeterminate before the first progress update"
        );
        assert_eq!(cx.debug_bounds("background_task_progress_fill"), None);

        task.update(cx, |task, cx| {
            task.set_progress(0.4, "Writing objects:  33% (1/3)", cx)
        });
        cx.run_until_parked();

        assert_eq!(
            cx.debug_bounds("background_task_progress_indeterminate"),
            None,
            "a progress update replaces the indeterminate segment"
        );
        let fill = cx
            .debug_bounds("background_task_progress_fill")
            .expect("a determinate bar renders its fill");
        assert!(
            (fill.size.width - PROGRESS_BAR_WIDTH * 0.4).abs() < px(0.5),
            "the fill covers the reported fraction of the bar, got {:?}",
            fill.size.width
        );
    }

    #[gpui::test]
    async fn test_background_tasks_run_concurrently_and_dropping_one_keeps_the_other(
        cx: &mut TestAppContext,
    ) {
        let (workspace, cx) = workspace_window(cx).await;
        let pushing = cx.new(|_| BackgroundTaskProgress::new("Pushing…"));
        let fetching = cx.new(|_| BackgroundTaskProgress::new("Fetching…"));
        workspace.update(cx, |workspace, cx| {
            workspace.show_background_task(&pushing, cx);
            workspace.show_background_task(&fetching, cx);
            workspace.show_background_task(&pushing, cx);
        });
        cx.run_until_parked();

        assert_eq!(
            tracked_task_ids(&workspace, cx),
            vec![pushing.entity_id(), fetching.entity_id()],
            "each task is tracked once even when shown twice"
        );
        assert!(cx.debug_bounds("background_task_progress_0").is_some());
        assert!(cx.debug_bounds("background_task_progress_1").is_some());

        cx.update(|_, _| drop(pushing));
        cx.run_until_parked();

        assert_eq!(
            tracked_task_ids(&workspace, cx),
            vec![fetching.entity_id()],
            "releasing one task keeps the other"
        );
        assert!(cx.debug_bounds("background_task_progress_0").is_some());
        assert_eq!(cx.debug_bounds("background_task_progress_1"), None);
    }
}
