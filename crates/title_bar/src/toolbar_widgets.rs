use gpui::SharedString;

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
