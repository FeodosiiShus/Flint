use merge_diff::{
    ComparisonError, ComparisonPolicy, ConflictKind, DiffFragment, InnerFragmentsPolicy,
    LineFragment, LineTexts, MergeConflictType, MergeInnerDifferences, MergeRange, NeverCanceled,
    ResolutionStrategy, Utf16Offsets, compare_chars, compare_lines, compare_lines_inner,
    compare_lines_three_way, compare_lines_with_inner_policy, compare_threeside_inner,
    compare_words, get_inner_chunks, line_merge_type, line_three_way_diff_type, merge_lines,
    try_greedy_resolve, try_resolve, try_resolve_conflict, word_merge,
};
use serde_json::{Map, Value, json};
use std::ops::Range;

const POLICIES: [(&str, ComparisonPolicy); 3] = [
    ("DEFAULT", ComparisonPolicy::Default),
    ("TRIM_WHITESPACES", ComparisonPolicy::TrimWhitespaces),
    ("IGNORE_WHITESPACES", ComparisonPolicy::IgnoreWhitespaces),
];

const MAX_REPORTED_FAILURES: usize = 8;
const MAX_CLIPPED_CHARACTERS: usize = 400;

type CaseRunner = fn(&Value) -> Result<Value, String>;
type FragmentComparer = fn(
    &str,
    &str,
    ComparisonPolicy,
    &dyn merge_diff::CancellationChecker,
) -> Result<Vec<DiffFragment>, ComparisonError>;

fn clip(value: &Value) -> String {
    let text = value.to_string();
    let total = text.chars().count();
    if total <= MAX_CLIPPED_CHARACTERS {
        return text;
    }
    let head: String = text.chars().take(MAX_CLIPPED_CHARACTERS).collect();
    format!("{head}...({total} chars)")
}

fn text_field<'a>(case: &'a Value, key: &str) -> Result<&'a str, String> {
    case.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("case has no string field `{key}`"))
}

fn utf16(offsets: &Utf16Offsets, byte_offset: usize) -> Result<usize, String> {
    offsets
        .to_utf16(byte_offset)
        .ok_or_else(|| format!("byte offset {byte_offset} is not a character boundary"))
}

fn relative_utf16(
    offsets: &Utf16Offsets,
    fragment_start: usize,
    relative_offset: usize,
) -> Result<usize, String> {
    Ok(utf16(offsets, fragment_start + relative_offset)? - utf16(offsets, fragment_start)?)
}

fn comparison_error(policy_name: &str, error: ComparisonError) -> String {
    format!("{policy_name}: {error}")
}

fn line_fragment_offsets(
    fragment: &LineFragment,
    offsets1: &Utf16Offsets,
    offsets2: &Utf16Offsets,
) -> Result<Value, String> {
    Ok(json!([
        utf16(offsets1, fragment.start_offset1)?,
        utf16(offsets1, fragment.end_offset1)?,
        utf16(offsets2, fragment.start_offset2)?,
        utf16(offsets2, fragment.end_offset2)?,
    ]))
}

fn line_fragment_lines(fragment: &LineFragment) -> Value {
    json!([
        fragment.start_line1,
        fragment.end_line1,
        fragment.start_line2,
        fragment.end_line2,
    ])
}

fn diff_fragment_offsets(
    fragment: &DiffFragment,
    offsets1: &Utf16Offsets,
    offsets2: &Utf16Offsets,
) -> Result<Value, String> {
    Ok(json!([
        utf16(offsets1, fragment.start_offset1)?,
        utf16(offsets1, fragment.end_offset1)?,
        utf16(offsets2, fragment.start_offset2)?,
        utf16(offsets2, fragment.end_offset2)?,
    ]))
}

fn run_lines2(case: &Value) -> Result<Value, String> {
    let left = text_field(case, "left")?;
    let right = text_field(case, "right")?;
    let offsets1 = Utf16Offsets::new(left);
    let offsets2 = Utf16Offsets::new(right);
    let mut results = Map::new();
    for (policy_name, policy) in POLICIES {
        let fragments = compare_lines(left, right, policy, &NeverCanceled)
            .map_err(|error| comparison_error(policy_name, error))?;
        let mut offsets = Vec::new();
        for fragment in &fragments {
            offsets.push(line_fragment_offsets(fragment, &offsets1, &offsets2)?);
        }
        let changes: Vec<Value> = fragments.iter().map(line_fragment_lines).collect();
        results.insert(
            policy_name.to_string(),
            json!({ "changes": changes, "offsets": offsets }),
        );
    }
    Ok(Value::Object(results))
}

fn inner_fragment_json(
    fragment: &LineFragment,
    offsets1: &Utf16Offsets,
    offsets2: &Utf16Offsets,
) -> Result<Value, String> {
    let inner = match &fragment.inner_fragments {
        None => Value::Null,
        Some(inner_fragments) => {
            let mut rows = Vec::new();
            for inner in inner_fragments {
                rows.push(json!([
                    relative_utf16(offsets1, fragment.start_offset1, inner.start_offset1)?,
                    relative_utf16(offsets1, fragment.start_offset1, inner.end_offset1)?,
                    relative_utf16(offsets2, fragment.start_offset2, inner.start_offset2)?,
                    relative_utf16(offsets2, fragment.start_offset2, inner.end_offset2)?,
                ]));
            }
            Value::Array(rows)
        }
    };
    Ok(json!({
        "lines": line_fragment_lines(fragment),
        "offsets": line_fragment_offsets(fragment, offsets1, offsets2)?,
        "inner": inner,
    }))
}

fn inner_fragments_json(
    fragments: &[LineFragment],
    offsets1: &Utf16Offsets,
    offsets2: &Utf16Offsets,
) -> Result<Value, String> {
    let mut rows = Vec::new();
    for fragment in fragments {
        rows.push(inner_fragment_json(fragment, offsets1, offsets2)?);
    }
    Ok(Value::Array(rows))
}

fn run_lines2_inner(case: &Value) -> Result<Value, String> {
    let left = text_field(case, "left")?;
    let right = text_field(case, "right")?;
    let offsets1 = Utf16Offsets::new(left);
    let offsets2 = Utf16Offsets::new(right);
    let mut results = Map::new();
    for (policy_name, policy) in POLICIES {
        let words = compare_lines_inner(left, right, policy, &NeverCanceled)
            .map_err(|error| comparison_error(policy_name, error))?;
        let chars = compare_lines_with_inner_policy(
            left,
            right,
            policy,
            InnerFragmentsPolicy::Chars,
            &NeverCanceled,
        )
        .map_err(|error| comparison_error(policy_name, error))?;
        results.insert(
            policy_name.to_string(),
            json!({
                "WORDS": inner_fragments_json(&words, &offsets1, &offsets2)?,
                "CHARS": inner_fragments_json(&chars, &offsets1, &offsets2)?,
            }),
        );
    }
    Ok(Value::Object(results))
}

fn run_diff_fragments(case: &Value, compare: FragmentComparer) -> Result<Value, String> {
    let left = text_field(case, "left")?;
    let right = text_field(case, "right")?;
    let offsets1 = Utf16Offsets::new(left);
    let offsets2 = Utf16Offsets::new(right);
    let mut results = Map::new();
    for (policy_name, policy) in POLICIES {
        let fragments = compare(left, right, policy, &NeverCanceled)
            .map_err(|error| comparison_error(policy_name, error))?;
        let mut rows = Vec::new();
        for fragment in &fragments {
            rows.push(diff_fragment_offsets(fragment, &offsets1, &offsets2)?);
        }
        results.insert(policy_name.to_string(), Value::Array(rows));
    }
    Ok(Value::Object(results))
}

fn run_words(case: &Value) -> Result<Value, String> {
    run_diff_fragments(case, compare_words)
}

fn run_chars(case: &Value) -> Result<Value, String> {
    run_diff_fragments(case, compare_chars)
}

fn kind_name(kind: ConflictKind) -> &'static str {
    match kind {
        ConflictKind::Inserted => "INSERTED",
        ConflictKind::Deleted => "DELETED",
        ConflictKind::Modified => "MODIFIED",
        ConflictKind::Conflict => "CONFLICT",
    }
}

fn resolution_json(resolution: Option<ResolutionStrategy>) -> Value {
    match resolution {
        None => Value::Null,
        Some(ResolutionStrategy::Default) => json!("DEFAULT"),
        Some(ResolutionStrategy::Text) => json!("TEXT"),
    }
}

fn type_json(merge_type: &MergeConflictType) -> Value {
    json!([
        kind_name(merge_type.kind),
        merge_type.left_change,
        merge_type.right_change,
        resolution_json(merge_type.resolution),
    ])
}

fn merge_lines_json(range: &MergeRange) -> Value {
    json!([
        range.left.start,
        range.left.end,
        range.base.start,
        range.base.end,
        range.right.start,
        range.right.end,
    ])
}

fn side_ranges_json(
    ranges: Option<&Vec<Range<usize>>>,
    chunk: Option<&str>,
) -> Result<Value, String> {
    let Some(ranges) = ranges else {
        return Ok(Value::Null);
    };
    let offsets = Utf16Offsets::new(chunk.unwrap_or_default());
    let mut rows = Vec::new();
    for range in ranges {
        rows.push(json!([
            utf16(&offsets, range.start)?,
            utf16(&offsets, range.end)?
        ]));
    }
    Ok(Value::Array(rows))
}

fn inner_differences_json(
    inner: Option<MergeInnerDifferences>,
    chunks: [Option<&str>; 3],
) -> Result<Value, String> {
    let Some(inner) = inner else {
        return Ok(Value::Null);
    };
    Ok(json!({
        "left": side_ranges_json(inner.left.as_ref(), chunks[0])?,
        "base": side_ranges_json(inner.base.as_ref(), chunks[1])?,
        "right": side_ranges_json(inner.right.as_ref(), chunks[2])?,
    }))
}

fn merge_type_of(
    range: &MergeRange,
    texts: [&LineTexts; 3],
    policy: ComparisonPolicy,
) -> Result<MergeConflictType, String> {
    line_merge_type(range, texts, policy)
        .ok_or_else(|| format!("no merge type for range {}", merge_lines_json(range)))
}

fn run_merge3(case: &Value) -> Result<Value, String> {
    let left = text_field(case, "left")?;
    let base = text_field(case, "base")?;
    let right = text_field(case, "right")?;
    let left_texts = LineTexts::new(left);
    let base_texts = LineTexts::new(base);
    let right_texts = LineTexts::new(right);
    let texts = [&left_texts, &base_texts, &right_texts];
    let mut results = Map::new();
    for (policy_name, policy) in POLICIES {
        let compared = compare_lines_three_way(left, base, right, policy, &NeverCanceled)
            .map_err(|error| comparison_error(policy_name, error))?;
        let mut compare_rows = Vec::new();
        for range in &compared {
            let merge_type = merge_type_of(range, texts, policy)?;
            let three_way_type =
                line_three_way_diff_type(range, texts, policy).ok_or_else(|| {
                    format!("no three way type for range {}", merge_lines_json(range))
                })?;
            compare_rows.push(json!({
                "lines": merge_lines_json(range),
                "type": type_json(&merge_type),
                "type3": type_json(&three_way_type),
            }));
        }
        let merged = merge_lines(left, base, right, policy, &NeverCanceled)
            .map_err(|error| comparison_error(policy_name, error))?;
        let mut merge_rows = Vec::new();
        for range in &merged {
            let merge_type = merge_type_of(range, texts, policy)?;
            let chunks = get_inner_chunks(range, texts, &merge_type);
            let inner = compare_threeside_inner(chunks, policy, &NeverCanceled)
                .map_err(|error| comparison_error(policy_name, error))?;
            merge_rows.push(json!({
                "lines": merge_lines_json(range),
                "type": type_json(&merge_type),
                "inner": inner_differences_json(inner, chunks)?,
            }));
        }
        results.insert(
            policy_name.to_string(),
            json!({ "compare": compare_rows, "merge": merge_rows }),
        );
    }
    Ok(Value::Object(results))
}

fn word_merge_row(range: &MergeRange, offsets: [&Utf16Offsets; 3]) -> Result<Value, String> {
    Ok(json!([
        utf16(offsets[0], range.left.start)?,
        utf16(offsets[0], range.left.end)?,
        utf16(offsets[1], range.base.start)?,
        utf16(offsets[1], range.base.end)?,
        utf16(offsets[2], range.right.start)?,
        utf16(offsets[2], range.right.end)?,
    ]))
}

fn run_resolve(case: &Value) -> Result<Value, String> {
    let left = text_field(case, "left")?;
    let base = text_field(case, "base")?;
    let right = text_field(case, "right")?;
    let left_offsets = Utf16Offsets::new(left);
    let base_offsets = Utf16Offsets::new(base);
    let right_offsets = Utf16Offsets::new(right);
    let mut word_merges = Map::new();
    for (policy_name, policy) in POLICIES {
        let ranges = word_merge(left, base, right, policy, &NeverCanceled)
            .map_err(|error| comparison_error(policy_name, error))?;
        let mut rows = Vec::new();
        for range in &ranges {
            rows.push(word_merge_row(
                range,
                [&left_offsets, &base_offsets, &right_offsets],
            )?);
        }
        word_merges.insert(policy_name.to_string(), Value::Array(rows));
    }
    Ok(json!({
        "tryResolve": try_resolve(left, base, right),
        "tryGreedyResolve": try_greedy_resolve(left, base, right),
        "tryResolveConflict": try_resolve_conflict(left, base, right),
        "wordMerge": word_merges,
    }))
}

fn first_difference(
    expected: &Value,
    actual: &Value,
    path: &str,
) -> Option<(String, Value, Value)> {
    match (expected, actual) {
        (Value::Object(expected_map), Value::Object(actual_map)) => {
            for (key, expected_value) in expected_map {
                let child_path = format!("{path}/{key}");
                match actual_map.get(key) {
                    None => {
                        return Some((
                            child_path,
                            expected_value.clone(),
                            Value::String("<missing>".to_string()),
                        ));
                    }
                    Some(actual_value) => {
                        if let Some(difference) =
                            first_difference(expected_value, actual_value, &child_path)
                        {
                            return Some(difference);
                        }
                    }
                }
            }
            actual_map
                .iter()
                .find(|(key, _)| !expected_map.contains_key(*key))
                .map(|(key, actual_value)| {
                    (
                        format!("{path}/{key}"),
                        Value::String("<missing>".to_string()),
                        actual_value.clone(),
                    )
                })
        }
        (Value::Array(expected_items), Value::Array(actual_items))
            if expected_items.len() == actual_items.len() =>
        {
            expected_items
                .iter()
                .zip(actual_items)
                .enumerate()
                .find_map(|(index, (expected_item, actual_item))| {
                    first_difference(expected_item, actual_item, &format!("{path}/{index}"))
                })
        }
        _ => (expected != actual).then(|| (path.to_string(), expected.clone(), actual.clone())),
    }
}

fn describe_inputs(case: &Value) -> String {
    ["left", "base", "right"]
        .iter()
        .filter_map(|key| case.get(*key).map(|value| format!("{key}={}", clip(value))))
        .collect::<Vec<_>>()
        .join("\n    ")
}

fn check_fixture(fixture: &str, raw: &str, expected_case_count: usize, run: CaseRunner) {
    let document: Value = match serde_json::from_str(raw) {
        Ok(document) => document,
        Err(error) => panic!("{fixture}: fixture is not valid JSON: {error}"),
    };
    let Some(cases) = document.get("cases").and_then(Value::as_array) else {
        panic!("{fixture}: fixture has no `cases` array");
    };
    assert_eq!(
        cases.len(),
        expected_case_count,
        "{fixture}: committed corpus size changed"
    );
    let null = Value::Null;
    let mut failures = Vec::new();
    for case in cases {
        let name = case
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("<unnamed>");
        let expected = case.get("results").unwrap_or(&null);
        match run(case) {
            Err(message) => failures.push(format!(
                "--- {fixture} :: {name}\n  error: {message}\n  {}",
                describe_inputs(case)
            )),
            Ok(actual) => {
                if let Some((path, expected_value, actual_value)) =
                    first_difference(expected, &actual, "")
                {
                    failures.push(format!(
                        "--- {fixture} :: {name}\n  at {path}\n  {}\n  expected {}\n  actual   {}",
                        describe_inputs(case),
                        clip(&expected_value),
                        clip(&actual_value)
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} {fixture} cases differ from IntelliJ:\n{}",
        failures.len(),
        cases.len(),
        failures
            .iter()
            .take(MAX_REPORTED_FAILURES)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn lines2_fixtures_match_intellij() {
    check_fixture(
        "lines2",
        include_str!("fixtures/lines2.json"),
        349,
        run_lines2,
    );
}

#[test]
fn lines2_inner_fixtures_match_intellij() {
    check_fixture(
        "lines2_inner",
        include_str!("fixtures/lines2_inner.json"),
        299,
        run_lines2_inner,
    );
}

#[test]
fn words_fixtures_match_intellij() {
    check_fixture("words", include_str!("fixtures/words.json"), 118, run_words);
}

#[test]
fn chars_fixtures_match_intellij() {
    check_fixture("chars", include_str!("fixtures/chars.json"), 118, run_chars);
}

#[test]
fn merge3_fixtures_match_intellij() {
    check_fixture(
        "merge3",
        include_str!("fixtures/merge3.json"),
        169,
        run_merge3,
    );
}

#[test]
fn merge3_random_fixtures_match_intellij() {
    check_fixture(
        "merge3_random",
        include_str!("fixtures/merge3_random.json"),
        50,
        run_merge3,
    );
}

#[test]
fn resolve_fixtures_match_intellij() {
    check_fixture(
        "resolve",
        include_str!("fixtures/resolve.json"),
        223,
        run_resolve,
    );
}
