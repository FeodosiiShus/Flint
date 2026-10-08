use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LineText {
    text: String,
    line_starts: Vec<usize>,
}

impl Default for LineText {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl LineText {
    pub(crate) fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let line_starts = compute_line_starts(&text);
        Self { text, line_starts }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn len(&self) -> usize {
        self.text.len()
    }

    pub(crate) fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    pub(crate) fn line_start(&self, line: usize) -> usize {
        let line = line.min(self.line_starts.len() - 1);
        self.line_starts[line]
    }

    pub(crate) fn line_end(&self, line: usize) -> usize {
        let line = line.min(self.line_starts.len() - 1);
        match self.line_starts.get(line + 1) {
            Some(next_line_start) => next_line_start - 1,
            None => self.text.len(),
        }
    }

    pub(crate) fn line_of_offset(&self, offset: usize) -> usize {
        self.line_starts
            .partition_point(|line_start| *line_start <= offset)
            .saturating_sub(1)
    }

    pub(crate) fn lines_range(
        &self,
        start_line: usize,
        end_line: usize,
        include_newline: bool,
    ) -> Range<usize> {
        if start_line == end_line {
            let offset = if start_line < self.line_count() {
                self.line_start(start_line)
            } else {
                self.text.len()
            };
            offset..offset
        } else {
            let start = self.line_start(start_line);
            let mut end = self.line_end(end_line.saturating_sub(1));
            if include_newline && end < self.text.len() {
                end += 1;
            }
            start..end.max(start)
        }
    }

    pub(crate) fn lines_content(&self, start_line: usize, end_line: usize) -> &str {
        let range = self.lines_range(start_line, end_line, false);
        self.text.get(range).unwrap_or_default()
    }

    pub(crate) fn lines(&self, start_line: usize, end_line: usize) -> Vec<String> {
        let end_line = end_line.min(self.line_count());
        (start_line..end_line)
            .filter_map(|line| {
                self.text
                    .get(self.line_start(line)..self.line_end(line))
                    .map(str::to_string)
            })
            .collect()
    }

    pub(crate) fn replace_range(&mut self, range: Range<usize>, new_text: &str) {
        self.text.replace_range(range, new_text);
        self.line_starts = compute_line_starts(&self.text);
    }

    pub(crate) fn is_valid_range(&self, range: &Range<usize>) -> bool {
        range.start <= range.end
            && self.text.is_char_boundary(range.start)
            && self.text.is_char_boundary(range.end)
    }
}

fn compute_line_starts(text: &str) -> Vec<usize> {
    let mut line_starts = vec![0];
    line_starts.extend(
        text.bytes()
            .enumerate()
            .filter(|(_, byte)| *byte == b'\n')
            .map(|(index, _)| index + 1),
    );
    line_starts
}

pub(crate) fn count_newlines(text: &str) -> usize {
    text.bytes().filter(|byte| *byte == b'\n').count()
}

pub(crate) fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

pub(crate) fn tokenize_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let body = text.strip_suffix('\n').unwrap_or(text);
    body.split('\n').map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_has_one_empty_line() {
        let document = LineText::new("");
        assert_eq!(document.line_count(), 1);
        assert_eq!(document.line_start(0), 0);
        assert_eq!(document.line_end(0), 0);
        assert_eq!(document.lines_range(0, 0, false), 0..0);
        assert_eq!(document.lines(0, 1), vec![String::new()]);
    }

    #[test]
    fn trailing_newline_adds_an_empty_last_line() {
        let document = LineText::new("a\nb\n");
        assert_eq!(document.line_count(), 3);
        assert_eq!(document.line_start(2), 4);
        assert_eq!(document.line_end(2), 4);
        assert_eq!(document.line_of_offset(4), 2);
        assert_eq!(document.line_of_offset(3), 1);
        assert_eq!(document.line_of_offset(2), 1);
    }

    #[test]
    fn lines_range_excludes_the_closing_newline_unless_requested() {
        let document = LineText::new("a\nbb\nc");
        assert_eq!(document.lines_range(1, 2, false), 2..4);
        assert_eq!(document.lines_range(1, 2, true), 2..5);
        assert_eq!(document.lines_range(0, 3, true), 0..6);
        assert_eq!(document.lines_range(3, 3, false), 6..6);
        assert_eq!(document.lines_range(1, 1, false), 2..2);
        assert_eq!(document.lines_content(0, 2), "a\nbb");
    }

    #[test]
    fn lines_returns_each_line_without_separators() {
        let document = LineText::new("a\n\nc");
        assert_eq!(
            document.lines(0, 3),
            vec!["a".to_string(), String::new(), "c".to_string()]
        );
        assert!(document.lines(2, 2).is_empty());
    }

    #[test]
    fn tokenize_skips_only_the_last_empty_line() {
        assert!(tokenize_lines("").is_empty());
        assert_eq!(tokenize_lines("a"), vec!["a".to_string()]);
        assert_eq!(tokenize_lines("a\n"), vec!["a".to_string()]);
        assert_eq!(
            tokenize_lines("a\n\n"),
            vec!["a".to_string(), String::new()]
        );
        assert_eq!(tokenize_lines("\n"), vec![String::new()]);
        assert_eq!(
            tokenize_lines("a\nb"),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn crlf_and_lone_cr_are_normalized_to_lf() {
        assert_eq!(normalize_line_endings("a\r\nb\r\n"), "a\nb\n");
        assert_eq!(normalize_line_endings("a\rb\r\r\nc"), "a\nb\n\nc");
    }
}
