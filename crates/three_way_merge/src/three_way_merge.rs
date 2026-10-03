use std::{borrow::Cow, ops::Range};

use imara_diff::{Algorithm, Diff, InternedInput};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn opposite(self) -> Self {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

    pub fn revision(self) -> Revision {
        match self {
            Side::Left => Revision::Left,
            Side::Right => Revision::Right,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Revision {
    Left,
    Base,
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum IgnorePolicy {
    #[default]
    None,
    TrimWhitespaces,
    IgnoreWhitespaces,
    IgnoreWhitespacesAndEmptyLines,
}

impl IgnorePolicy {
    pub const ALL: [IgnorePolicy; 4] = [
        IgnorePolicy::None,
        IgnorePolicy::TrimWhitespaces,
        IgnorePolicy::IgnoreWhitespaces,
        IgnorePolicy::IgnoreWhitespacesAndEmptyLines,
    ];

    pub fn label(self) -> &'static str {
        match self {
            IgnorePolicy::None => "Do not ignore",
            IgnorePolicy::TrimWhitespaces => "Trim whitespaces",
            IgnorePolicy::IgnoreWhitespaces => "Ignore whitespaces",
            IgnorePolicy::IgnoreWhitespacesAndEmptyLines => "Ignore whitespaces and empty lines",
        }
    }

    fn line_key(self, line: &str) -> Cow<'_, str> {
        match self {
            IgnorePolicy::None => Cow::Borrowed(line),
            IgnorePolicy::TrimWhitespaces => Cow::Borrowed(line.trim()),
            IgnorePolicy::IgnoreWhitespaces | IgnorePolicy::IgnoreWhitespacesAndEmptyLines => {
                Cow::Owned(
                    line.chars()
                        .filter(|character| !character.is_whitespace())
                        .collect(),
                )
            }
        }
    }

    fn ignores_empty_lines(self) -> bool {
        self == IgnorePolicy::IgnoreWhitespacesAndEmptyLines
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChangeType {
    Added,
    Deleted,
    Modified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChunkKind {
    LeftChange,
    RightChange,
    BothChangedEqually,
    Conflict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub base: Range<u32>,
    pub left: Range<u32>,
    pub right: Range<u32>,
    pub kind: ChunkKind,
}

impl Chunk {
    pub fn is_conflict(&self) -> bool {
        self.kind == ChunkKind::Conflict
    }

    pub fn changes(&self, side: Side) -> bool {
        match self.kind {
            ChunkKind::LeftChange => side == Side::Left,
            ChunkKind::RightChange => side == Side::Right,
            ChunkKind::BothChangedEqually | ChunkKind::Conflict => true,
        }
    }

    pub fn change_type(&self, side: Side) -> Option<ChangeType> {
        if !self.changes(side) {
            return None;
        }
        let side_lines = self.lines(side.revision());
        Some(if self.base.is_empty() {
            ChangeType::Added
        } else if side_lines.is_empty() {
            ChangeType::Deleted
        } else {
            ChangeType::Modified
        })
    }

    pub fn lines(&self, revision: Revision) -> Range<u32> {
        match revision {
            Revision::Left => self.left.clone(),
            Revision::Base => self.base.clone(),
            Revision::Right => self.right.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SideState {
    #[default]
    Pending,
    Applied,
    Ignored,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ChunkState {
    pub left: SideState,
    pub right: SideState,
}

impl ChunkState {
    pub fn side(&self, side: Side) -> SideState {
        match side {
            Side::Left => self.left,
            Side::Right => self.right,
        }
    }

    fn set(&mut self, side: Side, state: SideState) {
        match side {
            Side::Left => self.left = state,
            Side::Right => self.right = state,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ApplyMode {
    Replace,
    Append,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NonConflictingScope {
    Left,
    Right,
    All,
}

impl NonConflictingScope {
    fn includes(self, side: Side) -> bool {
        match self {
            NonConflictingScope::Left => side == Side::Left,
            NonConflictingScope::Right => side == Side::Right,
            NonConflictingScope::All => true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkEdit {
    pub chunk_index: usize,
    pub new_text: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Counters {
    pub changes: usize,
    pub conflicts: usize,
    pub total: usize,
}

impl Counters {
    pub fn remaining(&self) -> usize {
        self.changes + self.conflicts
    }

    pub fn resolved(&self) -> usize {
        self.total.saturating_sub(self.remaining())
    }

    pub fn is_complete(&self) -> bool {
        self.remaining() == 0
    }

    pub fn status_text(&self) -> String {
        if self.is_complete() {
            return "All changes have been processed.".to_string();
        }
        format!(
            "{} {}. {} {}.",
            self.changes,
            plural(self.changes, "change", "changes"),
            self.conflicts,
            plural(self.conflicts, "conflict", "conflicts"),
        )
    }
}

fn plural(count: usize, singular: &'static str, multiple: &'static str) -> &'static str {
    if count == 1 { singular } else { multiple }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            text.bytes()
                .enumerate()
                .filter(|(_, byte)| *byte == b'\n')
                .map(|(offset, _)| offset + 1),
        );
        if starts.last() != Some(&text.len()) {
            starts.push(text.len());
        }
        Self { starts }
    }

    fn line_count(&self) -> u32 {
        u32::try_from(self.starts.len().saturating_sub(1)).unwrap_or(u32::MAX)
    }

    fn offset(&self, line: u32) -> usize {
        self.starts
            .get(line as usize)
            .or_else(|| self.starts.last())
            .copied()
            .unwrap_or(0)
    }

    fn byte_range(&self, lines: Range<u32>) -> Range<usize> {
        let start = self.offset(lines.start);
        let end = self.offset(lines.end).max(start);
        start..end
    }
}

#[derive(Clone, Debug)]
struct SideHunk {
    base: Range<u32>,
    side: Range<u32>,
    ignorable: bool,
}

impl SideHunk {
    fn delta(&self) -> i64 {
        i64::from(self.side.end) - i64::from(self.side.start) - i64::from(self.base.end)
            + i64::from(self.base.start)
    }
}

fn line_keys(text: &str, policy: IgnorePolicy) -> Vec<Cow<'_, str>> {
    text.split_inclusive('\n')
        .map(|line| policy.line_key(line))
        .collect()
}

fn side_hunks(
    base_keys: &[Cow<'_, str>],
    side_keys: &[Cow<'_, str>],
    policy: IgnorePolicy,
) -> Vec<SideHunk> {
    let mut input: InternedInput<&str> = InternedInput::default();
    input.update_before(base_keys.iter().map(|key| key.as_ref()));
    input.update_after(side_keys.iter().map(|key| key.as_ref()));
    let diff = Diff::compute(Algorithm::Histogram, &input);
    diff.hunks()
        .map(|hunk| {
            let ignorable = policy.ignores_empty_lines()
                && keys_in(base_keys, hunk.before.clone()).all(str::is_empty)
                && keys_in(side_keys, hunk.after.clone()).all(str::is_empty);
            SideHunk {
                base: hunk.before,
                side: hunk.after,
                ignorable,
            }
        })
        .collect()
}

fn keys_in<'a>(keys: &'a [Cow<'_, str>], lines: Range<u32>) -> impl Iterator<Item = &'a str> {
    keys.get(lines.start as usize..lines.end as usize)
        .unwrap_or(&[])
        .iter()
        .map(|key| key.as_ref())
}

fn same_content(
    left_keys: &[Cow<'_, str>],
    left_lines: Range<u32>,
    right_keys: &[Cow<'_, str>],
    right_lines: Range<u32>,
    policy: IgnorePolicy,
) -> bool {
    let keep = |key: &&str| !(policy.ignores_empty_lines() && key.is_empty());
    keys_in(left_keys, left_lines)
        .filter(keep)
        .eq(keys_in(right_keys, right_lines).filter(keep))
}

fn to_line(value: i64) -> u32 {
    u32::try_from(value.max(0)).unwrap_or(u32::MAX)
}

fn group_side_range(base: &Range<u32>, delta_before: i64, group: &[SideHunk]) -> Range<u32> {
    let group_delta: i64 = group.iter().map(SideHunk::delta).sum();
    let start = to_line(i64::from(base.start) + delta_before);
    let end = to_line(i64::from(base.end) + delta_before + group_delta);
    start..end.max(start)
}

fn compute_chunks(base: &str, left: &str, right: &str, policy: IgnorePolicy) -> Vec<Chunk> {
    let base_keys = line_keys(base, policy);
    let left_keys = line_keys(left, policy);
    let right_keys = line_keys(right, policy);
    let left_hunks = side_hunks(&base_keys, &left_keys, policy);
    let right_hunks = side_hunks(&base_keys, &right_keys, policy);

    let mut chunks = Vec::new();
    let mut left_index = 0;
    let mut right_index = 0;
    let mut left_delta = 0i64;
    let mut right_delta = 0i64;

    loop {
        let group_start = match (left_hunks.get(left_index), right_hunks.get(right_index)) {
            (Some(left_hunk), Some(right_hunk)) => left_hunk.base.start.min(right_hunk.base.start),
            (Some(left_hunk), None) => left_hunk.base.start,
            (None, Some(right_hunk)) => right_hunk.base.start,
            (None, None) => break,
        };
        let mut group_end = group_start;
        let left_first = left_index;
        let right_first = right_index;
        loop {
            let mut consumed = false;
            while let Some(hunk) = left_hunks.get(left_index)
                && hunk.base.start <= group_end
            {
                group_end = group_end.max(hunk.base.end);
                left_index += 1;
                consumed = true;
            }
            while let Some(hunk) = right_hunks.get(right_index)
                && hunk.base.start <= group_end
            {
                group_end = group_end.max(hunk.base.end);
                right_index += 1;
                consumed = true;
            }
            if !consumed {
                break;
            }
        }

        let base_range = group_start..group_end;
        let left_group = left_hunks.get(left_first..left_index).unwrap_or(&[]);
        let right_group = right_hunks.get(right_first..right_index).unwrap_or(&[]);
        let left_range = group_side_range(&base_range, left_delta, left_group);
        let right_range = group_side_range(&base_range, right_delta, right_group);
        left_delta += left_group.iter().map(SideHunk::delta).sum::<i64>();
        right_delta += right_group.iter().map(SideHunk::delta).sum::<i64>();

        let left_changes = left_group.iter().any(|hunk| !hunk.ignorable);
        let right_changes = right_group.iter().any(|hunk| !hunk.ignorable);
        let kind = match (left_changes, right_changes) {
            (false, false) => continue,
            (true, false) => ChunkKind::LeftChange,
            (false, true) => ChunkKind::RightChange,
            (true, true) => {
                if same_content(
                    &left_keys,
                    left_range.clone(),
                    &right_keys,
                    right_range.clone(),
                    policy,
                ) {
                    ChunkKind::BothChangedEqually
                } else {
                    ChunkKind::Conflict
                }
            }
        };
        chunks.push(Chunk {
            base: base_range,
            left: left_range,
            right: right_range,
            kind,
        });
    }
    chunks
}

#[derive(Clone, Debug)]
pub struct ThreeWayMerge {
    base: String,
    left: String,
    right: String,
    base_lines: LineIndex,
    left_lines: LineIndex,
    right_lines: LineIndex,
    policy: IgnorePolicy,
    chunks: Vec<Chunk>,
}

impl ThreeWayMerge {
    pub fn new(
        base: impl Into<String>,
        left: impl Into<String>,
        right: impl Into<String>,
        policy: IgnorePolicy,
    ) -> Self {
        let base = base.into();
        let left = left.into();
        let right = right.into();
        let chunks = compute_chunks(&base, &left, &right, policy);
        Self {
            base_lines: LineIndex::new(&base),
            left_lines: LineIndex::new(&left),
            right_lines: LineIndex::new(&right),
            base,
            left,
            right,
            policy,
            chunks,
        }
    }

    pub fn text(&self, revision: Revision) -> &str {
        match revision {
            Revision::Left => &self.left,
            Revision::Base => &self.base,
            Revision::Right => &self.right,
        }
    }

    pub fn policy(&self) -> IgnorePolicy {
        self.policy
    }

    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }

    pub fn line_count(&self, revision: Revision) -> u32 {
        self.line_index(revision).line_count()
    }

    pub fn byte_range(&self, revision: Revision, lines: Range<u32>) -> Range<usize> {
        self.line_index(revision).byte_range(lines)
    }

    pub fn chunk_text(&self, chunk_index: usize, revision: Revision) -> &str {
        let Some(chunk) = self.chunks.get(chunk_index) else {
            return "";
        };
        self.text_for_lines(revision, chunk.lines(revision))
    }

    pub fn has_conflicts(&self) -> bool {
        self.chunks.iter().any(Chunk::is_conflict)
    }

    pub fn clean_merge(&self) -> Option<String> {
        if self.has_conflicts() {
            return None;
        }
        let mut merged = String::with_capacity(self.base.len());
        let mut base_line = 0;
        for chunk in &self.chunks {
            merged.push_str(self.text_for_lines(Revision::Base, base_line..chunk.base.start));
            let revision = match chunk.kind {
                ChunkKind::RightChange => Revision::Right,
                ChunkKind::LeftChange | ChunkKind::BothChangedEqually | ChunkKind::Conflict => {
                    Revision::Left
                }
            };
            merged.push_str(self.text_for_lines(revision, chunk.lines(revision)));
            base_line = chunk.base.end;
        }
        merged
            .push_str(self.text_for_lines(Revision::Base, base_line..self.base_lines.line_count()));
        Some(merged)
    }

    fn line_index(&self, revision: Revision) -> &LineIndex {
        match revision {
            Revision::Left => &self.left_lines,
            Revision::Base => &self.base_lines,
            Revision::Right => &self.right_lines,
        }
    }

    fn text_for_lines(&self, revision: Revision, lines: Range<u32>) -> &str {
        let range = self.byte_range(revision, lines);
        self.text(revision).get(range).unwrap_or("")
    }

    fn line_ending(&self) -> &'static str {
        let uses_crlf = [&self.base, &self.left, &self.right]
            .iter()
            .find_map(|text| {
                let newline = text.find('\n')?;
                Some(newline > 0 && text.as_bytes().get(newline - 1) == Some(&b'\r'))
            })
            .unwrap_or(false);
        if uses_crlf { "\r\n" } else { "\n" }
    }
}

#[derive(Clone, Debug)]
pub struct MergeModel {
    merge: ThreeWayMerge,
    states: Vec<ChunkState>,
}

impl MergeModel {
    pub fn new(merge: ThreeWayMerge) -> Self {
        let states = vec![ChunkState::default(); merge.chunks.len()];
        Self { merge, states }
    }

    pub fn merge(&self) -> &ThreeWayMerge {
        &self.merge
    }

    pub fn chunks(&self) -> &[Chunk] {
        &self.merge.chunks
    }

    pub fn chunk_state(&self, chunk_index: usize) -> Option<ChunkState> {
        self.states.get(chunk_index).copied()
    }

    pub fn is_pending(&self, chunk_index: usize, side: Side) -> bool {
        match (
            self.merge.chunks.get(chunk_index),
            self.states.get(chunk_index),
        ) {
            (Some(chunk), Some(state)) => {
                chunk.changes(side) && state.side(side) == SideState::Pending
            }
            _ => false,
        }
    }

    pub fn is_resolved(&self, chunk_index: usize) -> bool {
        !self.is_pending(chunk_index, Side::Left) && !self.is_pending(chunk_index, Side::Right)
    }

    pub fn apply(
        &mut self,
        chunk_index: usize,
        side: Side,
        mode: ApplyMode,
        current_text: &str,
    ) -> Option<ChunkEdit> {
        if !self.is_pending(chunk_index, side) {
            return None;
        }
        let kind = self.merge.chunks.get(chunk_index)?.kind;
        let side_text = self.merge.chunk_text(chunk_index, side.revision());
        let state = self.states.get_mut(chunk_index)?;
        let effective_mode =
            if kind == ChunkKind::Conflict && state.side(side.opposite()) == SideState::Applied {
                ApplyMode::Append
            } else {
                mode
            };
        let new_text = match effective_mode {
            ApplyMode::Replace => side_text.to_string(),
            ApplyMode::Append => append_text(current_text, side_text, self.merge.line_ending()),
        };
        state.set(side, SideState::Applied);
        if kind == ChunkKind::BothChangedEqually {
            state.set(side.opposite(), SideState::Applied);
        }
        Some(ChunkEdit {
            chunk_index,
            new_text,
        })
    }

    pub fn ignore(&mut self, chunk_index: usize, side: Side) -> bool {
        if !self.is_pending(chunk_index, side) {
            return false;
        }
        let both_equal = self
            .merge
            .chunks
            .get(chunk_index)
            .is_some_and(|chunk| chunk.kind == ChunkKind::BothChangedEqually);
        let Some(state) = self.states.get_mut(chunk_index) else {
            return false;
        };
        state.set(side, SideState::Ignored);
        if both_equal {
            state.set(side.opposite(), SideState::Ignored);
        }
        true
    }

    pub fn resolve_using(&mut self, chunk_index: usize, side: Side) -> Option<ChunkEdit> {
        if self.is_resolved(chunk_index) {
            return None;
        }
        let chunk = self.merge.chunks.get(chunk_index)?;
        let left_changes = chunk.changes(Side::Left);
        let right_changes = chunk.changes(Side::Right);
        let new_text = self
            .merge
            .chunk_text(chunk_index, side.revision())
            .to_string();
        let state = self.states.get_mut(chunk_index)?;
        for (candidate, changes) in [(Side::Left, left_changes), (Side::Right, right_changes)] {
            if changes {
                state.set(
                    candidate,
                    if candidate == side {
                        SideState::Applied
                    } else {
                        SideState::Ignored
                    },
                );
            }
        }
        Some(ChunkEdit {
            chunk_index,
            new_text,
        })
    }

    pub fn apply_non_conflicting(&mut self, scope: NonConflictingScope) -> Vec<ChunkEdit> {
        let mut edits = Vec::new();
        for chunk_index in 0..self.merge.chunks.len() {
            let is_conflict = self
                .merge
                .chunks
                .get(chunk_index)
                .is_none_or(Chunk::is_conflict);
            if is_conflict {
                continue;
            }
            for side in [Side::Left, Side::Right] {
                if scope.includes(side)
                    && self.is_pending(chunk_index, side)
                    && let Some(edit) = self.apply(chunk_index, side, ApplyMode::Replace, "")
                {
                    edits.push(edit);
                }
            }
        }
        edits
    }

    pub fn simple_resolution(&self, chunk_index: usize) -> Option<String> {
        let chunk = self.merge.chunks.get(chunk_index)?;
        if !chunk.is_conflict()
            || !self.is_pending(chunk_index, Side::Left)
            || !self.is_pending(chunk_index, Side::Right)
        {
            return None;
        }
        merge_tokens(
            self.merge.chunk_text(chunk_index, Revision::Base),
            self.merge.chunk_text(chunk_index, Revision::Left),
            self.merge.chunk_text(chunk_index, Revision::Right),
        )
    }

    pub fn is_simple_conflict(&self, chunk_index: usize) -> bool {
        self.simple_resolution(chunk_index).is_some()
    }

    pub fn resolve_simple(&mut self, chunk_index: usize) -> Option<ChunkEdit> {
        let new_text = self.simple_resolution(chunk_index)?;
        let state = self.states.get_mut(chunk_index)?;
        state.left = SideState::Applied;
        state.right = SideState::Applied;
        Some(ChunkEdit {
            chunk_index,
            new_text,
        })
    }

    pub fn resolve_simple_conflicts(&mut self) -> Vec<ChunkEdit> {
        (0..self.merge.chunks.len())
            .filter_map(|chunk_index| self.resolve_simple(chunk_index))
            .collect()
    }

    pub fn counters(&self) -> Counters {
        let mut counters = Counters {
            total: self.merge.chunks.len(),
            ..Counters::default()
        };
        for (chunk_index, chunk) in self.merge.chunks.iter().enumerate() {
            if self.is_resolved(chunk_index) {
                continue;
            }
            if chunk.is_conflict() {
                counters.conflicts += 1;
            } else {
                counters.changes += 1;
            }
        }
        counters
    }

    pub fn next_unresolved(&self, result_lines: &[Range<u32>], cursor_line: u32) -> Option<usize> {
        (0..self.merge.chunks.len()).find(|&chunk_index| {
            !self.is_resolved(chunk_index)
                && result_lines
                    .get(chunk_index)
                    .is_some_and(|lines| lines.start > cursor_line)
        })
    }

    pub fn previous_unresolved(
        &self,
        result_lines: &[Range<u32>],
        cursor_line: u32,
    ) -> Option<usize> {
        (0..self.merge.chunks.len()).rev().find(|&chunk_index| {
            !self.is_resolved(chunk_index)
                && result_lines
                    .get(chunk_index)
                    .is_some_and(|lines| lines.start < cursor_line)
        })
    }

    pub fn chunk_states(&self) -> Vec<ChunkState> {
        self.states.clone()
    }

    pub fn restore(&mut self, states: Vec<ChunkState>) {
        if states.len() == self.states.len() {
            self.states = states;
        }
    }
}

fn append_text(current: &str, addition: &str, line_ending: &str) -> String {
    let mut text = String::with_capacity(current.len() + addition.len() + line_ending.len());
    text.push_str(current);
    if !current.is_empty() && !addition.is_empty() && !current.ends_with('\n') {
        text.push_str(line_ending);
    }
    text.push_str(addition);
    text
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokenClass {
    Word,
    Space,
    Newline,
    Punctuation,
}

fn token_class(character: char) -> TokenClass {
    if character == '\n' || character == '\r' {
        TokenClass::Newline
    } else if character.is_whitespace() {
        TokenClass::Space
    } else if character.is_alphanumeric() || character == '_' {
        TokenClass::Word
    } else {
        TokenClass::Punctuation
    }
}

fn tokenize(text: &str) -> Vec<Range<usize>> {
    let mut tokens: Vec<Range<usize>> = Vec::new();
    let mut previous: Option<(TokenClass, char)> = None;
    for (offset, character) in text.char_indices() {
        let class = token_class(character);
        let extends = match previous {
            Some((previous_class, previous_character)) => match class {
                TokenClass::Word | TokenClass::Space => previous_class == class,
                TokenClass::Newline => previous_character == '\r' && character == '\n',
                TokenClass::Punctuation => false,
            },
            None => false,
        };
        let end = offset + character.len_utf8();
        match tokens.last_mut() {
            Some(last) if extends => last.end = end,
            _ => tokens.push(offset..end),
        }
        previous = Some((class, character));
    }
    tokens
}

#[derive(Clone, Debug)]
struct TokenEdit<'a> {
    side: Side,
    base: Range<usize>,
    replacement: &'a str,
}

fn token_edits<'a>(
    base: &str,
    base_tokens: &[Range<usize>],
    side_text: &'a str,
    side: Side,
) -> Vec<TokenEdit<'a>> {
    let side_tokens = tokenize(side_text);
    let mut input: InternedInput<&str> = InternedInput::default();
    input.update_before(
        base_tokens
            .iter()
            .map(|token| base.get(token.clone()).unwrap_or("")),
    );
    input.update_after(
        side_tokens
            .iter()
            .map(|token| side_text.get(token.clone()).unwrap_or("")),
    );
    let diff = Diff::compute(Algorithm::Histogram, &input);
    diff.hunks()
        .map(|hunk| {
            let base_range = token_byte_range(base_tokens, hunk.before, base.len());
            let side_range = token_byte_range(&side_tokens, hunk.after, side_text.len());
            TokenEdit {
                side,
                base: base_range,
                replacement: side_text.get(side_range).unwrap_or(""),
            }
        })
        .collect()
}

fn token_byte_range(
    tokens: &[Range<usize>],
    token_range: Range<u32>,
    text_len: usize,
) -> Range<usize> {
    let start = tokens
        .get(token_range.start as usize)
        .map_or(text_len, |token| token.start);
    if token_range.is_empty() {
        return start..start;
    }
    let end = token_range
        .end
        .checked_sub(1)
        .and_then(|last| tokens.get(last as usize))
        .map_or(text_len, |token| token.end);
    start..end.max(start)
}

fn ranges_overlap(first: &Range<usize>, second: &Range<usize>) -> bool {
    first.start < second.end && second.start < first.end
}

fn merge_tokens(base: &str, left: &str, right: &str) -> Option<String> {
    let base_tokens = tokenize(base);
    let mut edits = token_edits(base, &base_tokens, left, Side::Left);
    edits.extend(token_edits(base, &base_tokens, right, Side::Right));
    edits.sort_by_key(|edit| (edit.base.start, !edit.base.is_empty(), edit.base.end));

    let mut kept: Vec<TokenEdit<'_>> = Vec::with_capacity(edits.len());
    let mut furthest: Option<TokenEdit<'_>> = None;
    for edit in edits {
        if let Some(previous) = &furthest
            && previous.side != edit.side
        {
            if previous.base == edit.base && previous.replacement == edit.replacement {
                continue;
            }
            let same_insertion_point = previous.base.is_empty()
                && edit.base.is_empty()
                && previous.base.start == edit.base.start;
            if same_insertion_point || ranges_overlap(&previous.base, &edit.base) {
                return None;
            }
        }
        let extends_further = furthest
            .as_ref()
            .is_none_or(|previous| edit.base.end >= previous.base.end);
        if extends_further {
            furthest = Some(edit.clone());
        }
        kept.push(edit);
    }

    let mut merged = String::with_capacity(base.len() + left.len().max(right.len()));
    let mut cursor = 0;
    for edit in &kept {
        merged.push_str(base.get(cursor..edit.base.start).unwrap_or(""));
        merged.push_str(edit.replacement);
        cursor = edit.base.end.max(cursor);
    }
    merged.push_str(base.get(cursor..).unwrap_or(""));
    Some(merged)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultTracker {
    text: String,
    chunk_ranges: Vec<Range<usize>>,
}

impl ResultTracker {
    pub fn new(merge: &ThreeWayMerge) -> Self {
        let chunk_ranges = merge
            .chunks
            .iter()
            .map(|chunk| merge.byte_range(Revision::Base, chunk.base.clone()))
            .collect();
        Self {
            text: merge.base.clone(),
            chunk_ranges,
        }
    }

    pub fn from_parts(text: String, chunk_ranges: Vec<Range<usize>>) -> Self {
        Self { text, chunk_ranges }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn chunk_ranges(&self) -> &[Range<usize>] {
        &self.chunk_ranges
    }

    pub fn chunk_text(&self, chunk_index: usize) -> &str {
        self.chunk_ranges
            .get(chunk_index)
            .and_then(|range| self.text.get(range.clone()))
            .unwrap_or("")
    }

    pub fn apply(&mut self, edit: &ChunkEdit) {
        if let Some(range) = self.chunk_ranges.get(edit.chunk_index).cloned() {
            self.edit(range, &edit.new_text);
        }
    }

    pub fn edit(&mut self, range: Range<usize>, new_text: &str) {
        let start = range.start.min(self.text.len());
        let end = range.end.clamp(start, self.text.len());
        if !self.text.is_char_boundary(start) || !self.text.is_char_boundary(end) {
            return;
        }
        self.text.replace_range(start..end, new_text);
        let inserted = new_text.len();
        let shift = |offset: usize| offset - (end - start) + inserted;
        for chunk_range in &mut self.chunk_ranges {
            let chunk_start = if chunk_range.start <= start {
                chunk_range.start
            } else if chunk_range.start <= end {
                start + inserted
            } else {
                shift(chunk_range.start)
            };
            let chunk_end = if chunk_range.end < start {
                chunk_range.end
            } else if chunk_range.end <= end {
                start + inserted
            } else {
                shift(chunk_range.end)
            };
            *chunk_range = chunk_start..chunk_end.max(chunk_start);
        }
    }

    pub fn chunk_line_ranges(&self) -> Vec<Range<u32>> {
        self.chunk_ranges
            .iter()
            .map(|range| line_range_for_bytes(&self.text, range.clone()))
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct MergeDocument {
    model: MergeModel,
    result: ResultTracker,
}

impl MergeDocument {
    pub fn new(merge: ThreeWayMerge) -> Self {
        let result = ResultTracker::new(&merge);
        Self {
            model: MergeModel::new(merge),
            result,
        }
    }

    pub fn from_parts(model: MergeModel, result: ResultTracker) -> Self {
        Self { model, result }
    }

    pub fn into_parts(self) -> (MergeModel, ResultTracker) {
        (self.model, self.result)
    }

    pub fn model(&self) -> &MergeModel {
        &self.model
    }

    pub fn result(&self) -> &ResultTracker {
        &self.result
    }

    pub fn apply(&mut self, chunk_index: usize, side: Side, mode: ApplyMode) -> bool {
        let current_text = self.result.chunk_text(chunk_index).to_string();
        let edit = self.model.apply(chunk_index, side, mode, &current_text);
        self.apply_edits(edit)
    }

    pub fn ignore(&mut self, chunk_index: usize, side: Side) -> bool {
        self.model.ignore(chunk_index, side)
    }

    pub fn resolve_using(&mut self, chunk_index: usize, side: Side) -> bool {
        let edit = self.model.resolve_using(chunk_index, side);
        self.apply_edits(edit)
    }

    pub fn resolve_simple(&mut self, chunk_index: usize) -> bool {
        let edit = self.model.resolve_simple(chunk_index);
        self.apply_edits(edit)
    }

    pub fn apply_non_conflicting(&mut self, scope: NonConflictingScope) -> usize {
        let edits = self.model.apply_non_conflicting(scope);
        self.apply_all(edits)
    }

    pub fn resolve_simple_conflicts(&mut self) -> usize {
        let edits = self.model.resolve_simple_conflicts();
        self.apply_all(edits)
    }

    pub fn edit(&mut self, range: Range<usize>, new_text: &str) {
        self.result.edit(range, new_text);
    }

    fn apply_edits(&mut self, edits: impl IntoIterator<Item = ChunkEdit>) -> bool {
        self.apply_all(edits) > 0
    }

    fn apply_all(&mut self, edits: impl IntoIterator<Item = ChunkEdit>) -> usize {
        let mut count = 0;
        for edit in edits {
            self.result.apply(&edit);
            count += 1;
        }
        count
    }
}

pub fn map_line(
    source_chunks: &[Range<u32>],
    target_chunks: &[Range<u32>],
    source_line: f64,
) -> f64 {
    let mut source_end = 0f64;
    let mut target_end = 0f64;
    for (source, target) in source_chunks.iter().zip(target_chunks) {
        let source_start = f64::from(source.start);
        let target_start = f64::from(target.start);
        if source_line < source_start {
            return (target_end + (source_line - source_end)).min(target_start);
        }
        let source_stop = f64::from(source.end);
        let target_stop = f64::from(target.end);
        if source_line < source_stop {
            let fraction = (source_line - source_start) / (source_stop - source_start);
            return target_start + fraction * (target_stop - target_start);
        }
        source_end = source_stop;
        target_end = target_stop;
    }
    target_end + (source_line - source_end)
}

pub fn line_range_for_bytes(text: &str, byte_range: Range<usize>) -> Range<u32> {
    let bytes = text.as_bytes();
    let start = byte_range.start.min(bytes.len());
    let end = byte_range.end.clamp(start, bytes.len());
    let count_newlines = |slice: &[u8]| {
        u32::try_from(slice.iter().filter(|byte| **byte == b'\n').count()).unwrap_or(u32::MAX)
    };
    let start_line = count_newlines(bytes.get(..start).unwrap_or(&[]));
    let range_bytes = bytes.get(start..end).unwrap_or(&[]);
    if range_bytes.is_empty() {
        return start_line..start_line;
    }
    let partial_last_line = u32::from(range_bytes.last() != Some(&b'\n'));
    start_line..start_line + count_newlines(range_bytes) + partial_last_line
}
