#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RichStyle {
    Plain,
    Bold,
    Code,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RichSegment {
    Text { text: String, style: RichStyle },
    LineBreak,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RichText {
    segments: Vec<RichSegment>,
}

impl RichText {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn plain(text: impl Into<String>) -> Self {
        Self::new().with_plain(text)
    }

    pub(crate) fn with_plain(self, text: impl Into<String>) -> Self {
        self.with_text(text, RichStyle::Plain)
    }

    pub(crate) fn with_bold(self, text: impl Into<String>) -> Self {
        self.with_text(text, RichStyle::Bold)
    }

    pub(crate) fn with_code(self, text: impl Into<String>) -> Self {
        self.with_text(text, RichStyle::Code)
    }

    pub(crate) fn with_line_break(mut self) -> Self {
        self.segments.push(RichSegment::LineBreak);
        self
    }

    pub(crate) fn with_text(mut self, text: impl Into<String>, style: RichStyle) -> Self {
        let text = text.into();
        if text.is_empty() {
            return self;
        }
        if let Some(RichSegment::Text {
            text: previous_text,
            style: previous_style,
        }) = self.segments.last_mut()
            && *previous_style == style
        {
            previous_text.push_str(&text);
            return self;
        }
        self.segments.push(RichSegment::Text { text, style });
        self
    }

    pub(crate) fn with_rich(mut self, other: RichText) -> Self {
        for segment in other.segments {
            self = match segment {
                RichSegment::Text { text, style } => self.with_text(text, style),
                RichSegment::LineBreak => self.with_line_break(),
            };
        }
        self
    }

    pub(crate) fn segments(&self) -> &[RichSegment] {
        &self.segments
    }

    pub(crate) fn plain_text(&self) -> String {
        let mut text = String::new();
        for segment in &self.segments {
            match segment {
                RichSegment::Text { text: part, .. } => text.push_str(part),
                RichSegment::LineBreak => text.push('\n'),
            }
        }
        text
    }
}
