use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use gpui::{AppContext as _, Context, Task};
use merge_diff::{
    CancellationChecker, ComparisonPolicy, MergeInnerDifferences, ThreeSide,
    compare_threeside_inner,
};
use util::ResultExt as _;

use super::{MergeViewer, RediffState};
use crate::merge_tool::merge_model::line_text::LineText;

const INNER_DIFF_DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Default)]
pub(super) struct InnerDiffWorker {
    enabled: bool,
    scheduled: BTreeSet<usize>,
    in_flight: Option<InFlight>,
    _debounce: Option<Task<()>>,
}

struct InFlight {
    cancellation: Arc<AtomicBool>,
    _task: Task<()>,
}

impl InnerDiffWorker {
    fn cancel_in_flight(&mut self) {
        if let Some(in_flight) = self.in_flight.take() {
            in_flight.cancellation.store(true, Ordering::Relaxed);
        }
    }

    fn stop(&mut self) {
        self.cancel_in_flight();
        self.scheduled.clear();
        self._debounce = None;
    }

    pub(super) fn is_running(&self) -> bool {
        self.in_flight.is_some()
    }
}

struct SharedCancellation(Arc<AtomicBool>);

impl CancellationChecker for SharedCancellation {
    fn is_canceled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

struct InnerChunkData {
    index: usize,
    texts: [Option<String>; 3],
}

type InnerResults = Vec<(usize, Option<MergeInnerDifferences>)>;

fn compute_inner_results(
    chunks: Vec<InnerChunkData>,
    policy: ComparisonPolicy,
    cancellation: &SharedCancellation,
) -> InnerResults {
    chunks
        .into_iter()
        .map(|chunk| {
            let texts = [
                chunk.texts[0].as_deref(),
                chunk.texts[1].as_deref(),
                chunk.texts[2].as_deref(),
            ];
            let differences = compare_threeside_inner(texts, policy, cancellation)
                .ok()
                .flatten();
            (chunk.index, differences)
        })
        .collect()
}

impl MergeViewer {
    pub(super) fn is_busy(&self) -> bool {
        self.rediff == RediffState::Running || self.inner_worker.is_running()
    }

    pub(super) fn schedule_inner_diff(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.inner_worker.enabled {
            return;
        }
        let is_resolved = self
            .model
            .read(cx)
            .change(index)
            .is_none_or(|change| change.is_resolved());
        if is_resolved {
            return;
        }
        self.inner_worker.scheduled.insert(index);
        self.schedule_inner_diff_launch(cx);
    }

    fn schedule_inner_diff_launch(&mut self, cx: &mut Context<Self>) {
        if self.inner_worker.in_flight.is_some() || self.inner_worker.scheduled.is_empty() {
            return;
        }
        self.inner_worker._debounce = Some(cx.spawn(async move |viewer, cx| {
            cx.background_executor().timer(INNER_DIFF_DEBOUNCE).await;
            viewer
                .update(cx, |viewer, cx| viewer.launch_inner_diff(cx))
                .log_err();
        }));
    }

    pub(super) fn inner_diff_settings_changed(&mut self, cx: &mut Context<Self>) {
        if self.inner_worker.enabled == self.highlight_by_word {
            return;
        }
        self.inner_worker.enabled = self.highlight_by_word;
        self.rebuild_inner_diffs(cx);
    }

    pub(super) fn inner_diff_everything_changed(&mut self, cx: &mut Context<Self>) {
        self.inner_worker.enabled = self.highlight_by_word;
        self.rebuild_inner_diffs(cx);
    }

    pub(super) fn disable_inner_diff(&mut self) {
        self.inner_worker.enabled = false;
        self.inner_worker.stop();
    }

    fn rebuild_inner_diffs(&mut self, cx: &mut Context<Self>) {
        self.inner_worker.stop();
        if self.inner_worker.enabled {
            let unresolved = self.model.read(cx).unresolved_changes();
            self.inner_worker.scheduled.extend(unresolved);
            self.launch_inner_diff(cx);
        } else {
            for fragments in &mut self.inner_fragments {
                *fragments = None;
            }
            self.refresh_highlights(cx);
        }
    }

    fn collect_chunks(&self, indices: &[usize], cx: &gpui::App) -> Vec<InnerChunkData> {
        let model = self.model.read(cx);
        let left = LineText::new(model.side_text(ThreeSide::Left));
        let right = LineText::new(model.side_text(ThreeSide::Right));
        let result = LineText::new(model.current_result_text(cx));
        indices
            .iter()
            .filter_map(|index| {
                let change = model.change(*index)?;
                let texts = ThreeSide::ALL.map(|side| {
                    let start = model.start_line(*index, side);
                    let end = model.end_line(*index, side);
                    if !change.is_change(side) || change.is_resolved_for(side) || start == end {
                        return None;
                    }
                    let source = match side {
                        ThreeSide::Left => &left,
                        ThreeSide::Base => &result,
                        ThreeSide::Right => &right,
                    };
                    Some(source.lines_content(start, end).to_string())
                });
                Some(InnerChunkData {
                    index: *index,
                    texts,
                })
            })
            .collect()
    }

    fn launch_inner_diff(&mut self, cx: &mut Context<Self>) {
        self.inner_worker._debounce = None;
        self.inner_worker.cancel_in_flight();
        let indices: Vec<usize> = std::mem::take(&mut self.inner_worker.scheduled)
            .into_iter()
            .collect();
        if indices.is_empty() {
            cx.notify();
            return;
        }
        let chunks = self.collect_chunks(&indices, cx);
        let policy = self.model.read(cx).ignore_policy();
        let cancellation = Arc::new(AtomicBool::new(false));
        let background_cancellation = SharedCancellation(cancellation.clone());
        let computation = cx.background_spawn(async move {
            compute_inner_results(chunks, policy, &background_cancellation)
        });
        let task_cancellation = cancellation.clone();
        let task = cx.spawn(async move |viewer, cx| {
            let results = computation.await;
            viewer
                .update(cx, |viewer, cx| {
                    viewer.apply_inner_results(results, &task_cancellation, cx)
                })
                .log_err();
        });
        self.inner_worker.in_flight = Some(InFlight {
            cancellation,
            _task: task,
        });
        cx.notify();
    }

    fn apply_inner_results(
        &mut self,
        results: InnerResults,
        cancellation: &AtomicBool,
        cx: &mut Context<Self>,
    ) {
        if !self.inner_worker.enabled || cancellation.load(Ordering::Relaxed) {
            return;
        }
        self.inner_worker.in_flight = None;
        for (index, differences) in results {
            if self.inner_worker.scheduled.contains(&index) {
                continue;
            }
            if let Some(slot) = self.inner_fragments.get_mut(index) {
                *slot = differences;
            }
        }
        self.refresh_highlights(cx);
        if !self.inner_worker.scheduled.is_empty() {
            self.launch_inner_diff(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(
        index: usize,
        left: Option<&str>,
        base: Option<&str>,
        right: Option<&str>,
    ) -> InnerChunkData {
        InnerChunkData {
            index,
            texts: [left, base, right].map(|text| text.map(str::to_string)),
        }
    }

    fn never_canceled() -> SharedCancellation {
        SharedCancellation(Arc::new(AtomicBool::new(false)))
    }

    fn already_canceled() -> SharedCancellation {
        SharedCancellation(Arc::new(AtomicBool::new(true)))
    }

    #[test]
    fn merge_tool_inner_diff_all_three_sides_report_word_ranges_per_side() {
        let chunks = vec![chunk(
            3,
            Some("alpha beta\n"),
            Some("alpha gamma\n"),
            Some("alpha delta\n"),
        )];
        let results = compute_inner_results(chunks, ComparisonPolicy::Default, &never_canceled());
        assert_eq!(
            results,
            vec![(
                3,
                Some(MergeInnerDifferences {
                    left: Some(vec![6..10]),
                    base: Some(vec![6..11, 6..11]),
                    right: Some(vec![6..11]),
                })
            )]
        );
    }

    #[test]
    fn merge_tool_inner_diff_missing_base_compares_left_against_right() {
        let chunks = vec![chunk(5, Some("alpha beta\n"), None, Some("alpha gamma\n"))];
        let results = compute_inner_results(chunks, ComparisonPolicy::Default, &never_canceled());
        assert_eq!(
            results,
            vec![(
                5,
                Some(MergeInnerDifferences {
                    left: Some(vec![6..10]),
                    base: None,
                    right: Some(vec![6..11]),
                })
            )]
        );
    }

    #[test]
    fn merge_tool_inner_diff_two_missing_sides_produce_no_differences() {
        let chunks = vec![chunk(4, Some("only left\n"), None, None)];
        let results = compute_inner_results(chunks, ComparisonPolicy::Default, &never_canceled());
        assert_eq!(results, vec![(4, None)]);
    }

    #[test]
    fn merge_tool_inner_diff_policy_decides_whether_whitespace_changes_are_reported() {
        let whitespace_only = || vec![chunk(0, Some("a  b\n"), Some("a b\n"), Some("ab\n"))];
        let ignoring = compute_inner_results(
            whitespace_only(),
            ComparisonPolicy::IgnoreWhitespaces,
            &never_canceled(),
        );
        let comparing = compute_inner_results(
            whitespace_only(),
            ComparisonPolicy::Default,
            &never_canceled(),
        );
        assert_eq!(
            ignoring,
            vec![(
                0,
                Some(MergeInnerDifferences {
                    left: Some(vec![]),
                    base: Some(vec![]),
                    right: Some(vec![]),
                })
            )]
        );
        assert_eq!(
            comparing,
            vec![(
                0,
                Some(MergeInnerDifferences {
                    left: Some(vec![1..2]),
                    base: Some(vec![1..1, 0..3]),
                    right: Some(vec![0..2]),
                })
            )]
        );
    }

    #[test]
    fn merge_tool_inner_diff_canceled_computation_yields_no_differences() {
        let chunks = vec![chunk(
            6,
            Some("alpha beta\n"),
            Some("alpha gamma\n"),
            Some("alpha gamma delta\n"),
        )];
        let results = compute_inner_results(chunks, ComparisonPolicy::Default, &already_canceled());
        assert_eq!(results, vec![(6, None)]);
    }
}
