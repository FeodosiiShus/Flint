use crate::repository::{RemoteProgress, RemoteProgressSender};
use smol::io::{AsyncRead, AsyncReadExt};
use std::cmp::Ordering;
use std::io;

const READ_CHUNK_SIZE: usize = 8192;
const OBJECTS_COUNTED_AT_FULL_PROGRESS: f32 = 5000.0;

#[derive(Default)]
pub struct ProgressLineSplitter {
    pending: Vec<u8>,
}

impl ProgressLineSplitter {
    pub fn push(&mut self, bytes: &[u8], mut on_line: impl FnMut(&str)) {
        let mut segments = bytes.split(is_line_terminator).peekable();
        while let Some(segment) = segments.next() {
            if segments.peek().is_some() {
                self.complete_line(segment, &mut on_line);
            } else {
                self.pending.extend_from_slice(segment);
            }
        }
    }

    pub fn finish(&mut self, mut on_line: impl FnMut(&str)) {
        self.complete_line(&[], &mut on_line);
    }

    fn complete_line(&mut self, line_end: &[u8], on_line: &mut impl FnMut(&str)) {
        if self.pending.is_empty() {
            emit_line(line_end, on_line);
        } else {
            self.pending.extend_from_slice(line_end);
            emit_line(&self.pending, on_line);
            self.pending.clear();
        }
    }
}

fn is_line_terminator(byte: &u8) -> bool {
    matches!(*byte, b'\r' | b'\n')
}

fn emit_line(line: &[u8], on_line: &mut impl FnMut(&str)) {
    if !line.is_empty() {
        on_line(&String::from_utf8_lossy(line));
    }
}

pub(crate) async fn read_reporting_progress(
    mut pipe: impl AsyncRead + Unpin,
    progress: RemoteProgressSender,
) -> io::Result<Vec<u8>> {
    let mut contents = Vec::new();
    let mut buffer = [0; READ_CHUNK_SIZE];
    let mut lines = ProgressLineSplitter::default();
    let mut analyzer = RemoteProgressAnalyzer::default();
    loop {
        let bytes_read = pipe.read(&mut buffer).await?;
        if bytes_read == 0 {
            return Ok(contents);
        }
        let chunk = buffer.get(..bytes_read).unwrap_or_default();
        contents.extend_from_slice(chunk);
        lines.push(chunk, |line| {
            if let Some(fraction) = analyzer.analyze(line) {
                let details = line.trim().to_owned();
                let update = RemoteProgress { fraction, details };
                progress.unbounded_send(update).ok();
            }
        });
    }
}

pub(crate) fn without_progress_rewrites(output: &[u8]) -> Vec<u8> {
    let mut cleaned = Vec::with_capacity(output.len());
    for line in output.split_inclusive(|byte| *byte == b'\n') {
        let (content, terminator) = split_line_terminator(line);
        let final_rewrite = content
            .rsplit(|byte| *byte == b'\r')
            .next()
            .unwrap_or_default();
        cleaned.extend_from_slice(final_rewrite);
        cleaned.extend_from_slice(terminator);
    }
    cleaned
}

fn split_line_terminator(line: &[u8]) -> (&[u8], &[u8]) {
    for terminator in [b"\r\n".as_slice(), b"\n".as_slice()] {
        if let Some(content) = line.strip_suffix(terminator) {
            return (content, terminator);
        }
    }
    (line, &[])
}

#[derive(Default)]
struct RemoteProgressAnalyzer {
    stage_progress: [f32; TransferStage::IN_ORDER.len()],
}

impl RemoteProgressAnalyzer {
    fn analyze(&mut self, line: &str) -> Option<f32> {
        let (matched_index, matched_progress) = TransferStage::IN_ORDER
            .into_iter()
            .enumerate()
            .find_map(|(index, stage)| Some((index, stage.progress_in(line)?)))?;
        for (index, progress) in self.stage_progress.iter_mut().enumerate() {
            match index.cmp(&matched_index) {
                Ordering::Less => *progress = 1.0,
                Ordering::Equal => *progress = matched_progress,
                Ordering::Greater => {}
            }
        }
        Some(self.total_fraction())
    }

    fn total_fraction(&self) -> f32 {
        TransferStage::IN_ORDER
            .into_iter()
            .zip(self.stage_progress)
            .map(|(stage, progress)| stage.weight() * progress)
            .sum::<f32>()
            .clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy)]
enum TransferStage {
    CountingObjects,
    CompressingObjects,
    TransferringObjects,
    ResolvingDeltas,
}

impl TransferStage {
    const IN_ORDER: [Self; 4] = [
        Self::CountingObjects,
        Self::CompressingObjects,
        Self::TransferringObjects,
        Self::ResolvingDeltas,
    ];

    fn weight(self) -> f32 {
        match self {
            Self::CountingObjects => 0.05,
            Self::CompressingObjects => 0.1,
            Self::TransferringObjects => 0.8,
            Self::ResolvingDeltas => 0.05,
        }
    }

    fn labels(self) -> &'static [&'static str] {
        match self {
            Self::CountingObjects => &["counting objects:"],
            Self::CompressingObjects => &["compressing objects:"],
            Self::TransferringObjects => &["receiving objects:", "writing objects:"],
            Self::ResolvingDeltas => &["resolving deltas:"],
        }
    }

    fn progress_in(self, line: &str) -> Option<f32> {
        let progress_after_label = match self {
            Self::CountingObjects => counted_objects_progress,
            Self::CompressingObjects | Self::TransferringObjects | Self::ResolvingDeltas => {
                percentage_progress
            }
        };
        rightmost_progress_after(line, self.labels(), progress_after_label)
    }
}

fn rightmost_progress_after(
    line: &str,
    labels: &[&str],
    progress_after_label: fn(&[u8]) -> Option<f32>,
) -> Option<f32> {
    let line = line.as_bytes();
    (0..line.len()).rev().find_map(|start| {
        let after_label = strip_any_label(line.get(start..)?, labels)?;
        progress_after_label(after_label)
    })
}

fn strip_any_label<'a>(text: &'a [u8], labels: &[&str]) -> Option<&'a [u8]> {
    labels.iter().find_map(|label| {
        let (head, tail) = text.split_at_checked(label.len())?;
        let matches_label = head.eq_ignore_ascii_case(label.as_bytes());
        matches_label.then_some(tail)
    })
}

fn counted_objects_progress(text: &[u8]) -> Option<f32> {
    let (digits, _) = spaced_digits(text)?;
    Some((decimal_value(digits) / OBJECTS_COUNTED_AT_FULL_PROGRESS).min(1.0))
}

fn percentage_progress(text: &[u8]) -> Option<f32> {
    let (digits, rest) = spaced_digits(text)?;
    let is_percentage = digits.len() <= 3 && rest.starts_with(b"%");
    is_percentage.then(|| decimal_value(digits) / 100.0)
}

fn spaced_digits(text: &[u8]) -> Option<(&[u8], &[u8])> {
    let space_count = leading_count(text, |byte| *byte == b' ');
    if space_count == 0 {
        return None;
    }
    let number = text.get(space_count..)?;
    let digit_count = leading_count(number, u8::is_ascii_digit);
    if digit_count == 0 {
        return None;
    }
    number.split_at_checked(digit_count)
}

fn leading_count(bytes: &[u8], predicate: impl Fn(&u8) -> bool) -> usize {
    bytes.iter().take_while(|byte| predicate(byte)).count()
}

fn decimal_value(digits: &[u8]) -> f32 {
    digits
        .iter()
        .fold(0.0, |value, digit| value * 10.0 + f32::from(digit - b'0'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{TryStreamExt as _, channel::mpsc, executor::block_on, stream};

    fn rounded(fraction: f32) -> f32 {
        (fraction * 10_000.0).round() / 10_000.0
    }

    fn analyze_lines(lines: &[&str]) -> Vec<Option<f32>> {
        let mut analyzer = RemoteProgressAnalyzer::default();
        lines
            .iter()
            .map(|line| analyzer.analyze(line).map(rounded))
            .collect()
    }

    fn cleaned(output: &str) -> String {
        String::from_utf8(without_progress_rewrites(output.as_bytes())).unwrap()
    }

    #[test]
    fn test_push_progress_weights_stages_and_never_moves_backwards_across_a_push() {
        let fractions = analyze_lines(&[
            "Enumerating objects: 5, done.",
            "Counting objects:  20% (1/5)",
            "Counting objects: 100% (5/5), done.",
            "Delta compression using up to 8 threads",
            "Compressing objects:  50% (1/2)",
            "Compressing objects: 100% (2/2), done.",
            "Writing objects:  33% (1/3)",
            "Writing objects: 100% (3/3), 280 bytes | 280.00 KiB/s, done.",
            "Total 3 (delta 1), reused 0 (delta 0), pack-reused 0",
            "remote: Resolving deltas:   0% (0/1)        ",
            "remote: Resolving deltas: 100% (1/1), completed with 1 local object.        ",
            "To github.com:zed-industries/zed.git",
            "   1a2b3c4..5d6e7f8  main -> main",
        ]);

        assert_eq!(
            fractions,
            [
                None,
                Some(0.0002),
                Some(0.001),
                None,
                Some(0.1),
                Some(0.15),
                Some(0.414),
                Some(0.95),
                None,
                Some(0.95),
                Some(1.0),
                None,
                None,
            ]
        );
        let reported = fractions.iter().flatten().collect::<Vec<_>>();
        assert!(
            reported.windows(2).all(|pair| pair[0] <= pair[1]),
            "progress must never move backwards: {reported:?}"
        );
    }

    #[test]
    fn test_push_progress_scales_counted_objects_and_caps_them() {
        let fractions = analyze_lines(&[
            "Counting objects: 2500",
            "Counting objects: 5000, done.",
            "Counting objects: 123456",
            "Counting objects: 99999999999999999999999999999999999999999999",
        ]);

        assert_eq!(fractions, [Some(0.025), Some(0.05), Some(0.05), Some(0.05)]);
    }

    #[test]
    fn test_push_progress_counts_every_earlier_stage_as_complete() {
        assert_eq!(
            analyze_lines(&["Compressing objects:  10% (1/10)"]),
            [Some(0.06)]
        );
        assert_eq!(
            analyze_lines(&["Writing objects:  50% (5/10)"]),
            [Some(0.55)]
        );
        assert_eq!(
            analyze_lines(&["Resolving deltas:  40% (2/5)"]),
            [Some(0.97)]
        );
    }

    #[test]
    fn test_push_progress_matches_stage_names_case_insensitively() {
        let fractions = analyze_lines(&[
            "WRITING OBJECTS:  50% (1/2)",
            "receiving objects: 100% (2/2)",
            "rEsOlViNg DeLtAs: 100% (1/1)",
        ]);

        assert_eq!(fractions, [Some(0.55), Some(0.95), Some(1.0)]);
    }

    #[test]
    fn test_push_progress_ignores_lines_that_match_no_stage() {
        let fractions = analyze_lines(&[
            "Enumerating objects: 5, done.",
            "Writing objects: done",
            "Writing objects:50% (1/2)",
            "Writing objects:  50 (1/2)",
            "Compressing objects: 1000% (1/1)",
            "Total 3 (delta 1), reused 0 (delta 0), pack-reused 0",
            "",
            "Compressing objects:  50% (1/2)",
        ]);

        assert_eq!(
            fractions,
            [None, None, None, None, None, None, None, Some(0.1)]
        );
    }

    #[test]
    fn test_push_progress_reads_stages_behind_the_remote_prefix() {
        let fractions = analyze_lines(&[
            "remote: Counting objects: 2500        ",
            "remote: Compressing objects: 100% (2/2), done.        ",
            "remote: Resolving deltas:  50% (1/2)        ",
        ]);

        assert_eq!(fractions, [Some(0.025), Some(0.15), Some(0.975)]);
    }

    #[test]
    fn test_push_progress_lines_split_on_carriage_returns_and_newlines_across_chunks() {
        let chunks: [&[u8]; 4] = [
            b"Writing obj",
            b"ects:  33% (1/3)\rWriting objects: 100% (3/3), done.\r\nTo origin\n\n",
            b"remote: \xC3",
            b"\x89criture des objets: 50%\rpartial",
        ];
        let mut splitter = ProgressLineSplitter::default();
        let mut lines = Vec::new();

        for chunk in chunks {
            splitter.push(chunk, |line| lines.push(line.to_owned()));
        }
        splitter.finish(|line| lines.push(line.to_owned()));

        assert_eq!(
            lines,
            [
                "Writing objects:  33% (1/3)",
                "Writing objects: 100% (3/3), done.",
                "To origin",
                "remote: \u{c9}criture des objets: 50%",
                "partial",
            ]
        );
    }

    #[test]
    fn test_push_progress_rewrites_leave_only_the_final_text_of_each_line() {
        let transcript = [
            "Enumerating objects: 5, done.\n",
            "Counting objects:  20% (1/5)\rCounting objects: 100% (5/5), done.\n",
            "remote: Resolving deltas:   0% (0/1)        \r",
            "remote: Resolving deltas: 100% (1/1), done.        \n",
            "To origin\n",
        ]
        .concat();
        let expected = [
            "Enumerating objects: 5, done.\n",
            "Counting objects: 100% (5/5), done.\n",
            "remote: Resolving deltas: 100% (1/1), done.        \n",
            "To origin\n",
        ]
        .concat();

        assert_eq!(cleaned(&transcript), expected);
        assert_eq!(
            cleaned("Writing objects:  33% (1/3)\rWriting objects: 100% (3/3), done.\n"),
            "Writing objects: 100% (3/3), done.\n"
        );
        assert_eq!(cleaned("hook output\r\n"), "hook output\r\n");
        assert_eq!(
            cleaned("Writing objects:  50% (1/2)\rfatal: the remote end hung up"),
            "fatal: the remote end hung up"
        );
        assert_eq!(
            cleaned("Everything up-to-date\nWriting objects:  50% (1/2)\r"),
            "Everything up-to-date\n"
        );
    }

    #[test]
    fn test_push_progress_is_reported_for_each_recognized_line_as_chunks_arrive() {
        let chunks: [&[u8]; 3] = [
            b"Enumerating objects: 5, done.\nCompressing obj",
            b"ects: 100% (2/2), done.\nWriting objects:  50% (1/2)\r",
            b"Writing objects: 100% (2/2), done.\nTo origin\n",
        ];
        let (sender, mut receiver) = mpsc::unbounded();
        let pipe = stream::iter(chunks.map(Ok::<_, io::Error>)).into_async_read();

        let contents = block_on(read_reporting_progress(pipe, sender)).unwrap();

        let reported = std::iter::from_fn(|| receiver.try_recv().ok())
            .map(|update| (rounded(update.fraction), update.details))
            .collect::<Vec<_>>();
        assert_eq!(
            reported,
            [
                (0.15, "Compressing objects: 100% (2/2), done.".to_string()),
                (0.55, "Writing objects:  50% (1/2)".to_string()),
                (0.95, "Writing objects: 100% (2/2), done.".to_string()),
            ]
        );
        assert_eq!(contents, chunks.concat());
    }
}
