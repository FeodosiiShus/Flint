use std::sync::{Arc, atomic::AtomicBool};

use gpui::{Context, Entity, Task, Window};
use picker::Picker;
use project::{Candidates, Project};

use crate::delegate::{SearchEverywhereDelegate, Source};

const MAX_FILE_MATCHES: usize = 100;

pub(crate) fn search(
    project: Entity<Project>,
    path_query: String,
    include_non_project_items: bool,
    search_id: usize,
    cancel_flag: Arc<AtomicBool>,
    window: &mut Window,
    cx: &mut Context<Picker<SearchEverywhereDelegate>>,
) -> Task<()> {
    let candidate_sets = file_finder::path_candidate_sets(
        project.read(cx),
        include_non_project_items.then_some(true),
        || Candidates::Entries,
        cx,
    );
    cx.spawn_in(window, async move |picker, cx| {
        let matches = fuzzy_nucleo::match_path_sets(
            candidate_sets.as_slice(),
            &path_query,
            &None,
            fuzzy_nucleo::Case::Ignore,
            MAX_FILE_MATCHES,
            &cancel_flag,
            cx.background_executor().clone(),
        )
        .await
        .into_iter()
        .filter(|path_match| !(path_match.is_dir && path_match.path.is_empty()))
        .collect::<Vec<_>>();
        picker
            .update(cx, |picker, cx| {
                picker
                    .delegate
                    .apply_results(search_id, Source::Files, cx, |results| {
                        results.files = matches
                    });
            })
            .ok();
    })
}
