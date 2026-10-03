use fuzzy::StringMatchCandidate;
use gpui::{Context, Entity, Task, Window};
use language::SymbolKind;
use picker::Picker;
use project::{Project, Symbol};
use project_symbols::{match_symbol_candidates, partition_symbol_candidates, symbol_query_filter};
use util::ResultExt;

use crate::delegate::{SearchEverywhereDelegate, Source};

const MAX_SYMBOL_MATCHES: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SymbolScope {
    ClassesAndOthers,
    ClassesOnly,
    Everything,
}

pub(crate) fn is_class_kind(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Class | SymbolKind::Interface | SymbolKind::Enum | SymbolKind::Struct
    )
}

fn partition_by_class(
    candidates: Vec<StringMatchCandidate>,
    symbols: &[Symbol],
) -> (Vec<StringMatchCandidate>, Vec<StringMatchCandidate>) {
    candidates.into_iter().partition(|candidate| {
        symbols
            .get(candidate.id)
            .is_some_and(|symbol| is_class_kind(symbol.kind))
    })
}

pub(crate) fn search(
    project: Entity<Project>,
    query: String,
    scope: SymbolScope,
    include_non_project_items: bool,
    search_id: usize,
    window: &mut Window,
    cx: &mut Context<Picker<SearchEverywhereDelegate>>,
) -> Task<()> {
    let filter_query = symbol_query_filter(&query).to_owned();
    let symbols_task = project.update(cx, |project, cx| project.symbols(&query, cx));
    cx.spawn_in(window, async move |picker, cx| {
        let symbols = symbols_task.await.log_err().unwrap_or_default();
        let (visible, external) = project.read_with(cx, |project, cx| {
            partition_symbol_candidates(&symbols, project, cx)
        });
        let external = if include_non_project_items {
            external
        } else {
            Vec::new()
        };
        let executor = cx.background_executor().clone();

        let (classes, others) = match scope {
            SymbolScope::Everything => (
                Vec::new(),
                match_symbol_candidates(
                    &symbols,
                    &visible,
                    &external,
                    &filter_query,
                    MAX_SYMBOL_MATCHES,
                    executor,
                )
                .await,
            ),
            SymbolScope::ClassesAndOthers | SymbolScope::ClassesOnly => {
                let (visible_classes, visible_others) = partition_by_class(visible, &symbols);
                let (external_classes, external_others) = partition_by_class(external, &symbols);
                let classes = match_symbol_candidates(
                    &symbols,
                    &visible_classes,
                    &external_classes,
                    &filter_query,
                    MAX_SYMBOL_MATCHES,
                    executor.clone(),
                )
                .await;
                let others = if scope == SymbolScope::ClassesAndOthers {
                    match_symbol_candidates(
                        &symbols,
                        &visible_others,
                        &external_others,
                        &filter_query,
                        MAX_SYMBOL_MATCHES,
                        executor,
                    )
                    .await
                } else {
                    Vec::new()
                };
                (classes, others)
            }
        };

        picker
            .update(cx, |picker, cx| {
                picker
                    .delegate
                    .apply_results(search_id, Source::Symbols, cx, |results| {
                        results.symbols = symbols;
                        results.classes = classes;
                        results.other_symbols = others;
                    });
            })
            .ok();
    })
}
