use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use gpui::{Context, Entity, Task, Window};
use picker::Picker;
use project::{Project, search::SearchResult};
use search::{SearchOptions, text_finder::process_search_result};
use util::{ResultExt, paths::PathMatcher};

use crate::delegate::{SearchEverywhereDelegate, Source};

const MAX_TEXT_MATCHES: usize = 100;

pub(crate) fn search(
    project: Entity<Project>,
    query: &str,
    include_non_project_items: bool,
    search_id: usize,
    cancel_flag: Arc<AtomicBool>,
    window: &mut Window,
    cx: &mut Context<Picker<SearchEverywhereDelegate>>,
) -> Task<()> {
    let mut options = SearchOptions::NONE;
    options.set(SearchOptions::INCLUDE_IGNORED, include_non_project_items);
    let match_full_paths = project.read(cx).visible_worktrees(cx).count() > 1;
    let Some(search_query) = options
        .build_query(
            query,
            PathMatcher::default(),
            PathMatcher::default(),
            match_full_paths,
            None,
        )
        .log_err()
    else {
        return Task::ready(());
    };
    let search = project.update(cx, |project, cx| project.search(search_query, cx));
    cx.spawn_in(window, async move |picker, cx| {
        let mut found = 0;
        let mut replace_previous = true;
        while found < MAX_TEXT_MATCHES {
            let Ok(result) = search.rx.recv().await else {
                break;
            };
            if cancel_flag.load(Ordering::Acquire) {
                return;
            }
            match result {
                SearchResult::Buffer { buffer, ranges } => {
                    let take = ranges.len().min(MAX_TEXT_MATCHES - found);
                    let matches =
                        process_search_result(&buffer, ranges.get(..take).unwrap_or(&[]), cx);
                    found += matches.len();
                    let replace = std::mem::replace(&mut replace_previous, false);
                    picker
                        .update(cx, |picker, cx| {
                            picker
                                .delegate
                                .apply_results(search_id, Source::Text, cx, |results| {
                                    if replace {
                                        results.text = matches;
                                    } else {
                                        results.text.extend(matches);
                                    }
                                });
                        })
                        .ok();
                }
                SearchResult::LimitReached => break,
                SearchResult::WaitingForScan | SearchResult::Searching => {}
            }
        }
        drop(search);
        if replace_previous {
            picker
                .update(cx, |picker, cx| {
                    picker
                        .delegate
                        .apply_results(search_id, Source::Text, cx, |results| results.text.clear());
                })
                .ok();
        }
    })
}
