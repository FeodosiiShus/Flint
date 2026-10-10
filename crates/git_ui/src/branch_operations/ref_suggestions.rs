use collections::HashSet;
use gpui::{AnyElement, ClickEvent};
use ui::{HighlightedLabel, prelude::*};

use crate::branches_popup::tree::match_query;

pub const MAX_SUGGESTIONS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefSuggestion {
    pub text: String,
    pub highlight_offsets: Vec<usize>,
}

impl RefSuggestion {
    pub fn is_directory(&self) -> bool {
        self.text.ends_with('/')
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HighlightMove {
    Next,
    Previous,
}

pub fn branch_name_candidates<'a>(
    local_branches: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut seen = HashSet::default();
    let mut candidates = Vec::new();
    for name in local_branches {
        for (index, _) in name.match_indices('/') {
            push_unique(&mut candidates, &mut seen, &name[..=index]);
        }
        push_unique(&mut candidates, &mut seen, name);
    }
    candidates
}

pub fn revision_candidates<'a>(
    local_branches: impl IntoIterator<Item = &'a str>,
    remote_branches: impl IntoIterator<Item = &'a str>,
    tags: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut seen = HashSet::default();
    let mut candidates = Vec::new();
    for name in local_branches
        .into_iter()
        .chain(remote_branches)
        .chain(tags)
    {
        push_unique(&mut candidates, &mut seen, name);
    }
    candidates
}

fn push_unique(candidates: &mut Vec<String>, seen: &mut HashSet<String>, name: &str) {
    if !name.is_empty() && seen.insert(name.to_string()) {
        candidates.push(name.to_string());
    }
}

pub fn suggest(query: &str, candidates: &[String], limit: usize) -> Vec<RefSuggestion> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let mut matches: Vec<(i64, &String, Vec<usize>)> = candidates
        .iter()
        .filter(|candidate| candidate.as_str() != query)
        .filter_map(|candidate| {
            match_query(query, candidate).map(|(score, positions)| (score, candidate, positions))
        })
        .collect();
    matches.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.chars().count().cmp(&right.1.chars().count()))
            .then_with(|| left.1.cmp(right.1))
    });
    matches
        .into_iter()
        .take(limit)
        .map(|(_, candidate, positions)| RefSuggestion {
            text: candidate.clone(),
            highlight_offsets: char_positions_to_byte_offsets(candidate, &positions),
        })
        .collect()
}

fn char_positions_to_byte_offsets(text: &str, positions: &[usize]) -> Vec<usize> {
    let offsets: Vec<usize> = text.char_indices().map(|(offset, _)| offset).collect();
    positions
        .iter()
        .filter_map(|position| offsets.get(*position).copied())
        .collect()
}

pub fn move_highlight(
    current: Option<usize>,
    count: usize,
    direction: HighlightMove,
) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match (current, direction) {
        (None, HighlightMove::Next) => 0,
        (None, HighlightMove::Previous) => count - 1,
        (Some(index), HighlightMove::Next) => (index + 1) % count,
        (Some(index), HighlightMove::Previous) => (index + count - 1) % count,
    })
}

pub fn highlighted_suggestion(
    suggestions: &[RefSuggestion],
    highlighted: Option<usize>,
) -> Option<&RefSuggestion> {
    highlighted.and_then(|index| suggestions.get(index))
}

pub fn suggestion_for_tab(
    suggestions: &[RefSuggestion],
    highlighted: Option<usize>,
) -> Option<&RefSuggestion> {
    highlighted_suggestion(suggestions, highlighted).or_else(|| suggestions.first())
}

pub fn render_suggestion_list<V: 'static>(
    suggestions: &[RefSuggestion],
    highlighted: Option<usize>,
    id: &'static str,
    select: fn(&mut V, usize, &mut Window, &mut Context<V>),
    cx: &mut Context<V>,
) -> AnyElement {
    let colors = cx.theme().colors();
    let hover_background = colors.ghost_element_hover;
    let selected_background = colors.ghost_element_selected;
    let border_color = colors.border;
    let surface = colors.elevated_surface_background;
    v_flex()
        .id(id)
        .w_full()
        .py_0p5()
        .rounded_md()
        .border_1()
        .border_color(border_color)
        .bg(surface)
        .children(suggestions.iter().enumerate().map(|(index, suggestion)| {
            h_flex()
                .id((id, index))
                .w_full()
                .px_2()
                .py_0p5()
                .cursor_pointer()
                .when(highlighted == Some(index), |row| {
                    row.bg(selected_background)
                })
                .hover(move |style| style.bg(hover_background))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    select(this, index, window, cx)
                }))
                .child(
                    HighlightedLabel::new(
                        suggestion.text.clone(),
                        suggestion.highlight_offsets.clone(),
                    )
                    .size(LabelSize::Small)
                    .single_line(),
                )
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn texts(suggestions: &[RefSuggestion]) -> Vec<&str> {
        suggestions
            .iter()
            .map(|suggestion| suggestion.text.as_str())
            .collect()
    }

    #[test]
    fn branch_name_candidates_include_every_directory_prefix_once() {
        let candidates =
            branch_name_candidates(["feature/login", "feature/ui/button", "main", "main"]);
        assert_eq!(
            candidates,
            strings(&[
                "feature/",
                "feature/login",
                "feature/ui/",
                "feature/ui/button",
                "main",
            ])
        );
    }

    #[test]
    fn revision_candidates_keep_kind_order_and_drop_duplicates_and_empty_names() {
        let candidates = revision_candidates(
            ["main", "dev"],
            ["origin/main", "main"],
            ["v1.0", "", "dev"],
        );
        assert_eq!(candidates, strings(&["main", "dev", "origin/main", "v1.0"]));
    }

    #[test]
    fn blank_queries_suggest_nothing() {
        let candidates = strings(&["main", "feature/"]);
        assert!(suggest("", &candidates, MAX_SUGGESTIONS).is_empty());
        assert!(suggest("  \t", &candidates, MAX_SUGGESTIONS).is_empty());
    }

    #[test]
    fn matching_is_case_insensitive_and_skips_non_matches() {
        let candidates = strings(&["main", "Maintenance", "develop"]);
        let suggestions = suggest("MAIN", &candidates, MAX_SUGGESTIONS);
        assert_eq!(texts(&suggestions), vec!["Maintenance", "main"]);
        let suggestions = suggest("maint", &candidates, MAX_SUGGESTIONS);
        assert_eq!(texts(&suggestions), vec!["Maintenance"]);
    }

    #[test]
    fn a_candidate_equal_to_the_query_is_not_suggested() {
        let candidates = strings(&["main", "main-old"]);
        let suggestions = suggest("main", &candidates, MAX_SUGGESTIONS);
        assert_eq!(texts(&suggestions), vec!["main-old"]);
    }

    #[test]
    fn matches_at_the_start_rank_before_matches_inside_the_name() {
        let candidates = strings(&["catalog", "feature/login", "login"]);
        let suggestions = suggest("log", &candidates, MAX_SUGGESTIONS);
        assert_eq!(suggestions.len(), 3);
        assert_eq!(suggestions[0].text, "login");
        let position = |name: &str| {
            suggestions
                .iter()
                .position(|suggestion| suggestion.text == name)
        };
        assert!(position("login") < position("catalog"));
    }

    #[test]
    fn equal_scores_prefer_shorter_names_then_alphabetical_order() {
        let candidates = strings(&["acd", "ac", "ab", "abc"]);
        let suggestions = suggest("a", &candidates, MAX_SUGGESTIONS);
        assert_eq!(texts(&suggestions), vec!["ab", "ac", "abc", "acd"]);
    }

    #[test]
    fn the_result_is_limited() {
        let candidates: Vec<String> = (0..20).map(|index| format!("feature/{index:02}")).collect();
        let suggestions = suggest("feature", &candidates, MAX_SUGGESTIONS);
        assert_eq!(suggestions.len(), MAX_SUGGESTIONS);
        assert_eq!(suggest("feature", &candidates, 3).len(), 3);
    }

    #[test]
    fn highlight_offsets_are_utf8_byte_offsets_of_the_matched_characters() {
        let candidates = strings(&["login", "\u{f1}log"]);
        let suggestions = suggest("lg", &candidates, MAX_SUGGESTIONS);
        let login = suggestions
            .iter()
            .find(|suggestion| suggestion.text == "login")
            .map(|suggestion| suggestion.highlight_offsets.clone());
        assert_eq!(login, Some(vec![0, 2]));
        let accented = suggestions
            .iter()
            .find(|suggestion| suggestion.text == "\u{f1}log")
            .map(|suggestion| suggestion.highlight_offsets.clone());
        assert_eq!(accented, Some(vec![2, 4]));
    }

    #[test]
    fn directory_suggestions_are_recognised_by_their_trailing_slash() {
        let candidates = branch_name_candidates(["feature/login"]);
        let suggestions = suggest("feat", &candidates, MAX_SUGGESTIONS);
        let directory = suggestions
            .iter()
            .find(|suggestion| suggestion.text == "feature/")
            .map(RefSuggestion::is_directory);
        let branch = suggestions
            .iter()
            .find(|suggestion| suggestion.text == "feature/login")
            .map(RefSuggestion::is_directory);
        assert_eq!(directory, Some(true));
        assert_eq!(branch, Some(false));
    }

    #[test]
    fn highlight_moves_wrap_around_in_both_directions() {
        assert_eq!(move_highlight(None, 0, HighlightMove::Next), None);
        assert_eq!(move_highlight(None, 3, HighlightMove::Next), Some(0));
        assert_eq!(move_highlight(None, 3, HighlightMove::Previous), Some(2));
        assert_eq!(move_highlight(Some(0), 3, HighlightMove::Next), Some(1));
        assert_eq!(move_highlight(Some(2), 3, HighlightMove::Next), Some(0));
        assert_eq!(move_highlight(Some(0), 3, HighlightMove::Previous), Some(2));
        assert_eq!(move_highlight(Some(2), 3, HighlightMove::Previous), Some(1));
        assert_eq!(move_highlight(Some(5), 3, HighlightMove::Next), Some(0));
    }

    #[test]
    fn enter_only_accepts_a_highlighted_suggestion_while_tab_falls_back_to_the_first() {
        let candidates = strings(&["main", "master"]);
        let suggestions = suggest("ma", &candidates, MAX_SUGGESTIONS);
        assert_eq!(suggestions.len(), 2);
        assert_eq!(highlighted_suggestion(&suggestions, None), None);
        assert_eq!(
            highlighted_suggestion(&suggestions, Some(1)),
            suggestions.get(1)
        );
        assert_eq!(highlighted_suggestion(&suggestions, Some(7)), None);
        assert_eq!(suggestion_for_tab(&suggestions, None), suggestions.first());
        assert_eq!(
            suggestion_for_tab(&suggestions, Some(1)),
            suggestions.get(1)
        );
        assert_eq!(suggestion_for_tab(&[], None), None);
    }
}
