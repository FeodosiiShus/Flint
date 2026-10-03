use std::sync::{Arc, atomic::AtomicBool};

use fuzzy_nucleo::StringMatchCandidate;
use gpui::{Context, Task, Window};
use picker::Picker;

use crate::delegate::{SearchEverywhereDelegate, Source};

const MAX_ACTION_MATCHES: usize = 200;

pub(crate) fn search(
    query: String,
    candidates: Arc<[StringMatchCandidate]>,
    search_id: usize,
    cancel_flag: Arc<AtomicBool>,
    window: &mut Window,
    cx: &mut Context<Picker<SearchEverywhereDelegate>>,
) -> Task<()> {
    let query =
        command_palette::normalize_action_query(&command_palette::resolve_command_alias(query, cx));
    cx.spawn_in(window, async move |picker, cx| {
        let matches = fuzzy_nucleo::match_strings_async(
            &candidates,
            &query,
            fuzzy_nucleo::Case::Smart,
            fuzzy_nucleo::LengthPenalty::On,
            MAX_ACTION_MATCHES,
            &cancel_flag,
            cx.background_executor().clone(),
        )
        .await;
        picker
            .update(cx, |picker, cx| {
                picker
                    .delegate
                    .apply_results(search_id, Source::Actions, cx, |results| {
                        results.actions = matches
                    });
            })
            .ok();
    })
}
