use gpui::{Hsla, SharedString};

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;
const LIGHT_BACKGROUND_LUMINANCE: f32 = 0.6;

pub(crate) fn project_initials(project_name: &str) -> Option<SharedString> {
    let mut words = project_name
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty());
    let first_word = words.next()?;
    let first_initial = first_word.chars().next()?;
    let second_initial = match words.next() {
        Some(second_word) => second_word.chars().next(),
        None => camel_case_second_initial(first_word),
    };
    let initials: String = std::iter::once(first_initial)
        .chain(second_initial)
        .filter_map(|initial| initial.to_uppercase().next())
        .collect();
    Some(initials.into())
}

fn camel_case_second_initial(word: &str) -> Option<char> {
    word.chars()
        .zip(word.chars().skip(1))
        .find(|(previous, current)| previous.is_lowercase() && current.is_uppercase())
        .map(|(_, current)| current)
}

pub(crate) fn project_accent_index(project_name: &str, palette_size: usize) -> Option<usize> {
    if palette_size == 0 {
        return None;
    }
    let hash = project_name.bytes().fold(FNV_OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
    });
    let palette_size = u64::try_from(palette_size).ok()?;
    usize::try_from(hash % palette_size).ok()
}

pub(crate) fn badge_text_color(background: Hsla) -> Hsla {
    let rgb = background.to_rgb();
    let luminance = 0.2126 * rgb.r + 0.7152 * rgb.g + 0.0722 * rgb.b;
    if luminance > LIGHT_BACKGROUND_LUMINANCE {
        gpui::black()
    } else {
        gpui::white()
    }
}

pub(crate) fn upstream_tracking_label(ahead: u32, behind: u32) -> Option<SharedString> {
    match (behind, ahead) {
        (0, 0) => None,
        (behind, 0) => Some(format!("↓{behind}").into()),
        (0, ahead) => Some(format!("↑{ahead}").into()),
        (behind, ahead) => Some(format!("↓{behind} ↑{ahead}").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::hsla;

    fn initials(project_name: &str) -> Option<String> {
        project_initials(project_name).map(|initials| initials.to_string())
    }

    #[test]
    fn test_single_word_project_uses_one_uppercase_initial() {
        assert_eq!(initials("flint").as_deref(), Some("F"));
        assert_eq!(initials("Zed").as_deref(), Some("Z"));
        assert_eq!(initials("project2").as_deref(), Some("P"));
    }

    #[test]
    fn test_multiple_words_use_the_first_two_initials() {
        assert_eq!(initials("my project").as_deref(), Some("MP"));
        assert_eq!(initials("web storm ide").as_deref(), Some("WS"));
        assert_eq!(initials("2048 game").as_deref(), Some("2G"));
    }

    #[test]
    fn test_camel_case_single_word_uses_its_inner_capital() {
        assert_eq!(initials("NewTask").as_deref(), Some("NT"));
        assert_eq!(initials("myWebApp").as_deref(), Some("MW"));
        assert_eq!(initials("HTMLParser").as_deref(), Some("H"));
        assert_eq!(initials("iOS-app").as_deref(), Some("IA"));
    }

    #[test]
    fn test_punctuation_separates_words_and_is_never_an_initial() {
        assert_eq!(initials("web-storm_ide").as_deref(), Some("WS"));
        assert_eq!(initials("--zed--").as_deref(), Some("Z"));
        assert_eq!(initials("my.app").as_deref(), Some("MA"));
        assert_eq!(initials("(flint)").as_deref(), Some("F"));
    }

    #[test]
    fn test_non_ascii_names_produce_uppercase_initials() {
        assert_eq!(initials("проект тест").as_deref(), Some("ПТ"));
        assert_eq!(initials("émile").as_deref(), Some("É"));
        assert_eq!(initials("日本語").as_deref(), Some("日"));
        assert_eq!(initials("straße").as_deref(), Some("S"));
    }

    #[test]
    fn test_names_without_alphanumerics_have_no_initials() {
        assert_eq!(initials(""), None);
        assert_eq!(initials("   "), None);
        assert_eq!(initials("...!-_"), None);
    }

    #[test]
    fn test_accent_index_is_deterministic_for_the_same_name() {
        let first = project_accent_index("flint", 13);
        let second = project_accent_index("flint", 13);
        assert_eq!(first, second);
        assert!(first.is_some());
    }

    #[test]
    fn test_accent_index_stays_within_the_palette() {
        let names = ["", "a", "flint", "zed", "проект", "my-web-app", "日本語"];
        for palette_size in 1..=16 {
            for name in names {
                let index = project_accent_index(name, palette_size)
                    .expect("non-empty palette always yields an index");
                assert!(
                    index < palette_size,
                    "{name:?} mapped outside {palette_size}"
                );
            }
        }
    }

    #[test]
    fn test_accent_index_with_single_color_palette_is_zero() {
        assert_eq!(project_accent_index("anything", 1), Some(0));
    }

    #[test]
    fn test_empty_palette_has_no_accent_index() {
        assert_eq!(project_accent_index("flint", 0), None);
    }

    #[test]
    fn test_accent_index_spreads_different_names_across_the_palette() {
        let distinct_indices: std::collections::HashSet<usize> = (0..32)
            .filter_map(|number| project_accent_index(&format!("project-{number}"), 8))
            .collect();
        assert!(
            distinct_indices.len() > 1,
            "different project names should not all share one color"
        );
    }

    #[test]
    fn test_badge_text_contrasts_with_its_background() {
        assert_eq!(badge_text_color(hsla(0.0, 0.0, 1.0, 1.0)), gpui::black());
        assert_eq!(badge_text_color(hsla(0.0, 0.0, 0.0, 1.0)), gpui::white());
        assert_eq!(
            badge_text_color(hsla(60.0 / 360.0, 1.0, 0.5, 1.0)),
            gpui::black()
        );
        assert_eq!(
            badge_text_color(hsla(240.0 / 360.0, 1.0, 0.5, 1.0)),
            gpui::white()
        );
    }

    #[test]
    fn test_upstream_label_is_hidden_when_in_sync() {
        assert_eq!(upstream_tracking_label(0, 0), None);
    }

    #[test]
    fn test_upstream_label_shows_only_nonzero_directions() {
        assert_eq!(
            upstream_tracking_label(0, 2).map(|label| label.to_string()),
            Some("↓2".to_string())
        );
        assert_eq!(
            upstream_tracking_label(3, 0).map(|label| label.to_string()),
            Some("↑3".to_string())
        );
    }

    #[test]
    fn test_upstream_label_shows_incoming_before_outgoing() {
        assert_eq!(
            upstream_tracking_label(3, 2).map(|label| label.to_string()),
            Some("↓2 ↑3".to_string())
        );
        assert_eq!(
            upstream_tracking_label(u32::MAX, 1).map(|label| label.to_string()),
            Some(format!("↓1 ↑{}", u32::MAX))
        );
    }
}
