use gpui::{Hsla, SharedString};

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

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

pub(crate) fn project_color(project_name: &str, palette: &[Hsla], fallback: Hsla) -> Hsla {
    project_accent_index(project_name, palette.len())
        .and_then(|index| palette.get(index).copied())
        .unwrap_or(fallback)
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
    fn project_gradient_color_is_the_palette_entry_for_the_project_name() {
        let palette = [
            hsla(0.0, 0.7, 0.5, 1.0),
            hsla(0.25, 0.7, 0.5, 1.0),
            hsla(0.5, 0.7, 0.5, 1.0),
            hsla(0.75, 0.7, 0.5, 1.0),
        ];
        let fallback = hsla(0.1, 0.1, 0.1, 1.0);
        for name in ["flint", "zed", "my-web-app", "проект"] {
            let accent_index = project_accent_index(name, palette.len())
                .expect("non-empty palette always yields an index");
            assert_eq!(
                project_color(name, &palette, fallback),
                palette[accent_index],
                "{name:?} should tint its header with its palette entry"
            );
        }
    }

    #[test]
    fn project_gradient_color_falls_back_when_the_theme_has_no_accents() {
        let fallback = hsla(0.1, 0.1, 0.1, 1.0);
        assert_eq!(project_color("flint", &[], fallback), fallback);
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
