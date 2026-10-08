use crate::by_word::trim_spaces_corrector;
use crate::diff::{ChangeBuilder, FairDiff, diff};
use crate::range::DiffRange;
use crate::text::{
    ComparisonPolicy, expand_whitespaces_forward, is_punctuation, is_white_space_code_point,
};
use crate::{CancellationChecker, ComparisonError};

pub(crate) fn compare_punctuation(
    text1: &[u8],
    text2: &[u8],
    cancel: &dyn CancellationChecker,
) -> Result<FairDiff, ComparisonError> {
    let punctuation1 = punctuation_positions(text1);
    let punctuation2 = punctuation_positions(text2);
    let codes1: Vec<u32> = punctuation1
        .iter()
        .map(|&position| u32::from(text1[position]))
        .collect();
    let codes2: Vec<u32> = punctuation2
        .iter()
        .map(|&position| u32::from(text2[position]))
        .collect();
    let changes = diff(&codes1, &codes2, cancel)?;
    let mut builder = ChangeBuilder::new(text1.len(), text2.len());
    for unchanged in changes.unchanged() {
        for offset in 0..unchanged.end1 - unchanged.start1 {
            builder.mark_equal(
                punctuation1[unchanged.start1 + offset],
                punctuation2[unchanged.start2 + offset],
                1,
            );
        }
    }
    Ok(builder.finish())
}

fn punctuation_positions(text: &[u8]) -> Vec<usize> {
    text.iter()
        .enumerate()
        .filter(|(_, byte)| is_punctuation(**byte))
        .map(|(position, _)| position)
        .collect()
}

struct CodePointsOffsets {
    code_points: Vec<u32>,
    starts: Vec<usize>,
    ends: Vec<usize>,
}

fn all_code_points_with_boundaries(text: &str) -> (Vec<u32>, Vec<usize>) {
    let mut code_points = Vec::new();
    let mut boundaries = Vec::new();
    for (offset, character) in text.char_indices() {
        code_points.push(character as u32);
        boundaries.push(offset);
    }
    boundaries.push(text.len());
    (code_points, boundaries)
}

fn non_space_code_points(text: &str) -> CodePointsOffsets {
    let mut code_points = Vec::new();
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    for (offset, character) in text.char_indices() {
        let code_point = character as u32;
        if !is_white_space_code_point(code_point) {
            code_points.push(code_point);
            starts.push(offset);
            ends.push(offset + character.len_utf8());
        }
    }
    CodePointsOffsets {
        code_points,
        starts,
        ends,
    }
}

fn compare_all_code_points(
    text1: &str,
    text2: &str,
    cancel: &dyn CancellationChecker,
) -> Result<FairDiff, ComparisonError> {
    let (code_points1, boundaries1) = all_code_points_with_boundaries(text1);
    let (code_points2, boundaries2) = all_code_points_with_boundaries(text2);
    let iterable = diff(&code_points1, &code_points2, cancel)?;
    let mut builder = ChangeBuilder::new(text1.len(), text2.len());
    for unchanged in iterable.unchanged() {
        builder.mark_equal_ranges(
            boundaries1[unchanged.start1],
            boundaries2[unchanged.start2],
            boundaries1[unchanged.end1],
            boundaries2[unchanged.end2],
        );
    }
    Ok(builder.finish())
}

fn match_gap(
    builder: &mut ChangeBuilder,
    text1: &str,
    text2: &str,
    range: &DiffRange,
    cancel: &dyn CancellationChecker,
) -> Result<(), ComparisonError> {
    let inner_changes = compare_all_code_points(
        &text1[range.start1..range.end1],
        &text2[range.start2..range.end2],
        cancel,
    )?;
    for unchanged in inner_changes.unchanged() {
        builder.mark_equal_ranges(
            range.start1 + unchanged.start1,
            range.start2 + unchanged.start2,
            range.start1 + unchanged.end1,
            range.start2 + unchanged.end2,
        );
    }
    Ok(())
}

fn default_char_change_corrector(
    code_points1: &CodePointsOffsets,
    code_points2: &CodePointsOffsets,
    text1: &str,
    text2: &str,
    changes: &FairDiff,
    cancel: &dyn CancellationChecker,
) -> Result<FairDiff, ComparisonError> {
    let mut builder = ChangeBuilder::new(text1.len(), text2.len());
    let mut last1 = 0;
    let mut last2 = 0;
    for unchanged in changes.unchanged() {
        for offset in 0..unchanged.end1 - unchanged.start1 {
            let start1 = code_points1.starts[unchanged.start1 + offset];
            let start2 = code_points2.starts[unchanged.start2 + offset];
            let end1 = code_points1.ends[unchanged.start1 + offset];
            let end2 = code_points2.ends[unchanged.start2 + offset];
            match_gap(
                &mut builder,
                text1,
                text2,
                &DiffRange::new(last1, start1, last2, start2),
                cancel,
            )?;
            builder.mark_equal_ranges(start1, start2, end1, end2);
            last1 = end1;
            last2 = end2;
        }
    }
    match_gap(
        &mut builder,
        text1,
        text2,
        &DiffRange::new(last1, text1.len(), last2, text2.len()),
        cancel,
    )?;
    Ok(builder.finish())
}

fn compare_two_step(
    text1: &str,
    text2: &str,
    cancel: &dyn CancellationChecker,
) -> Result<FairDiff, ComparisonError> {
    let code_points1 = non_space_code_points(text1);
    let code_points2 = non_space_code_points(text2);
    let non_space_changes = diff(&code_points1.code_points, &code_points2.code_points, cancel)?;
    default_char_change_corrector(
        &code_points1,
        &code_points2,
        text1,
        text2,
        &non_space_changes,
        cancel,
    )
}

fn expand_forward_whitespaces(
    code_points1: &CodePointsOffsets,
    code_points2: &CodePointsOffsets,
    text1: &str,
    text2: &str,
    change: &DiffRange,
    left: bool,
) -> usize {
    let offset1 = if change.start1 == 0 {
        0
    } else {
        code_points1.ends[change.start1 - 1]
    };
    let offset2 = if change.start2 == 0 {
        0
    } else {
        code_points2.ends[change.start2 - 1]
    };
    let start = if left { offset1 } else { offset2 };
    start
        + expand_whitespaces_forward(
            text1.as_bytes(),
            text2.as_bytes(),
            offset1,
            offset2,
            text1.len(),
            text2.len(),
        )
}

fn compare_ignore_whitespaces(
    text1: &str,
    text2: &str,
    cancel: &dyn CancellationChecker,
) -> Result<FairDiff, ComparisonError> {
    let code_points1 = non_space_code_points(text1);
    let code_points2 = non_space_code_points(text2);
    let changes = diff(&code_points1.code_points, &code_points2.code_points, cancel)?;
    let mut ranges = Vec::new();
    for change in &changes.changes {
        let (start_offset1, end_offset1) = if change.start1 == change.end1 {
            let offset = expand_forward_whitespaces(
                &code_points1,
                &code_points2,
                text1,
                text2,
                change,
                true,
            );
            (offset, offset)
        } else {
            (
                code_points1.starts[change.start1],
                code_points1.ends[change.end1 - 1],
            )
        };
        let (start_offset2, end_offset2) = if change.start2 == change.end2 {
            let offset = expand_forward_whitespaces(
                &code_points1,
                &code_points2,
                text1,
                text2,
                change,
                false,
            );
            (offset, offset)
        } else {
            (
                code_points2.starts[change.start2],
                code_points2.ends[change.end2 - 1],
            )
        };
        ranges.push(DiffRange::new(
            start_offset1,
            end_offset1,
            start_offset2,
            end_offset2,
        ));
    }
    Ok(FairDiff::new(ranges, text1.len(), text2.len()))
}

pub(crate) fn compare_chars(
    text1: &str,
    text2: &str,
    policy: ComparisonPolicy,
    cancel: &dyn CancellationChecker,
) -> Result<Vec<DiffRange>, ComparisonError> {
    let iterable = match policy {
        ComparisonPolicy::Default => compare_two_step(text1, text2, cancel)?,
        ComparisonPolicy::TrimWhitespaces => {
            let iterable = compare_two_step(text1, text2, cancel)?;
            trim_spaces_corrector(&iterable, text1.as_bytes(), text2.as_bytes())
        }
        ComparisonPolicy::IgnoreWhitespaces => compare_ignore_whitespaces(text1, text2, cancel)?,
    };
    Ok(iterable.changes)
}
