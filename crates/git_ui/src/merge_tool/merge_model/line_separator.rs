#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum LineSeparator {
    Lf,
    Crlf,
    Cr,
}

impl LineSeparator {
    pub(crate) fn detect(text: &str) -> Option<Self> {
        let mut remaining = text
            .bytes()
            .skip_while(|byte| !matches!(byte, b'\n' | b'\r'));
        match remaining.next()? {
            b'\r' => Some(if remaining.next() == Some(b'\n') {
                Self::Crlf
            } else {
                Self::Cr
            }),
            _ => Some(Self::Lf),
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::Crlf => "CRLF",
            Self::Cr => "CR",
        }
    }
}

pub(crate) type PaneSeparators = [Option<LineSeparator>; 3];

pub(crate) fn separators_to_display(detected: PaneSeparators) -> PaneSeparators {
    let mut distinct: Vec<LineSeparator> = Vec::new();
    for separator in detected.into_iter().flatten() {
        if !distinct.contains(&separator) {
            distinct.push(separator);
        }
    }
    if distinct.len() < 2 {
        [None; 3]
    } else {
        detected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_uses_the_first_separator_of_the_text() {
        assert_eq!(LineSeparator::detect("a\nb\r\nc"), Some(LineSeparator::Lf));
        assert_eq!(
            LineSeparator::detect("a\r\nb\nc"),
            Some(LineSeparator::Crlf)
        );
        assert_eq!(LineSeparator::detect("a\rb\nc"), Some(LineSeparator::Cr));
    }

    #[test]
    fn detect_returns_nothing_for_text_without_line_breaks() {
        assert_eq!(LineSeparator::detect(""), None);
        assert_eq!(LineSeparator::detect("single line"), None);
    }

    #[test]
    fn detect_treats_a_trailing_carriage_return_as_classic_mac() {
        assert_eq!(LineSeparator::detect("a\r"), Some(LineSeparator::Cr));
    }

    #[test]
    fn labels_match_the_jetbrains_enum_names() {
        assert_eq!(LineSeparator::Lf.label(), "LF");
        assert_eq!(LineSeparator::Crlf.label(), "CRLF");
        assert_eq!(LineSeparator::Cr.label(), "CR");
    }

    #[test]
    fn equal_separators_display_nothing() {
        let detected = [
            Some(LineSeparator::Lf),
            Some(LineSeparator::Lf),
            Some(LineSeparator::Lf),
        ];
        assert_eq!(separators_to_display(detected), [None; 3]);
    }

    #[test]
    fn contents_without_separators_are_ignored_when_comparing() {
        let detected = [Some(LineSeparator::Crlf), None, Some(LineSeparator::Crlf)];
        assert_eq!(separators_to_display(detected), [None; 3]);
        assert_eq!(separators_to_display([None; 3]), [None; 3]);
    }

    #[test]
    fn differing_separators_are_displayed_for_every_pane_that_has_one() {
        let detected = [Some(LineSeparator::Crlf), None, Some(LineSeparator::Lf)];
        assert_eq!(separators_to_display(detected), detected);
    }

    #[test]
    fn merge_texts_record_the_separator_of_every_side_before_normalizing() {
        let texts =
            crate::merge_tool::merge_model::MergeTexts::new("a\r\nb\r\n", "a\nb\n", "a\rb\r");
        assert_eq!(
            texts.line_separators,
            [
                Some(LineSeparator::Crlf),
                Some(LineSeparator::Lf),
                Some(LineSeparator::Cr)
            ]
        );
        assert_eq!(texts.left, "a\nb\n");
        assert_eq!(texts.right, "a\nb\n");
    }
}
