use gpui::SharedString;

use super::super::ref_labels::{RefLabel, RefLabelKind, compare_ref_labels};

pub(crate) const SHORT_HASH_LENGTH: usize = 8;
pub(crate) const MAX_COMMITS_TO_LOAD: usize = 50;
pub(crate) const MAX_REFERENCES_SHOWN: usize = 100;
const MAX_COMMIT_MESSAGE_LENGTH: usize = 50_000;
const MAX_COMMIT_MESSAGE_LINES: usize = 3_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MessagePresentation {
    pub subject: Option<String>,
    pub body: String,
}

pub(crate) fn trim_message_to_sane_size(message: &str, product_name: &str) -> String {
    let truncated = |end: usize| {
        format!(
            "{}\n\n... The commit message is too long and was truncated by {product_name}…",
            &message[..end]
        )
    };
    let nth_line = message
        .match_indices('\n')
        .nth(MAX_COMMIT_MESSAGE_LINES - 1)
        .map(|(index, _)| index);
    if let Some(index) = nth_line
        && index < MAX_COMMIT_MESSAGE_LENGTH
    {
        return truncated(index);
    }
    if message.len() > MAX_COMMIT_MESSAGE_LENGTH + 50 {
        let mut end = MAX_COMMIT_MESSAGE_LENGTH;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        return truncated(end);
    }
    message.to_string()
}

pub(crate) fn message_presentation(message: &str, product_name: &str) -> MessagePresentation {
    let message = trim_message_to_sane_size(message, product_name).replace('\t', "    ");
    let subject_end = message.find("\n\n").filter(|index| *index > 0);
    let subject = subject_end.map_or(message.as_str(), |index| &message[..index]);
    if subject.contains('\n') {
        return MessagePresentation {
            subject: None,
            body: message,
        };
    }
    let description = &message[subject.len()..];
    MessagePresentation {
        subject: Some(subject.to_string()),
        body: description
            .strip_prefix('\n')
            .unwrap_or(description)
            .to_string(),
    }
}

pub(crate) fn short_presentation(name: &str, email: &str) -> String {
    if !name.is_empty() {
        return name.to_string();
    }
    match email.find('@') {
        Some(index) if index > 0 => email[..index].to_string(),
        _ => email.to_string(),
    }
}

fn is_word_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

fn first_and_last_name(name: &str) -> Option<(&str, &str)> {
    let separator = name.find(|character: char| !is_word_char(character))?;
    let first = &name[..separator];
    let separator_char = name[separator..].chars().next()?;
    let last = &name[separator + separator_char.len_utf8()..];
    let valid_separator = separator_char.is_ascii_punctuation() || separator_char.is_whitespace();
    (!first.is_empty() && !last.is_empty() && valid_separator && last.chars().all(is_word_char))
        .then_some((first, last))
}

fn name_in_standard_form(name: &str) -> String {
    if let Some((first, last)) = first_and_last_name(name) {
        return format!("{} {}", first.to_lowercase(), last.to_lowercase());
    }
    if name
        .chars()
        .all(|character| (' '..='~').contains(&character))
    {
        name.to_lowercase()
    } else {
        name.to_string()
    }
}

pub(crate) fn is_same_person(
    first_name: &str,
    first_email: &str,
    second_name: &str,
    second_email: &str,
) -> bool {
    name_in_standard_form(&short_presentation(first_name, first_email))
        == name_in_standard_form(&short_presentation(second_name, second_email))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MetadataPart {
    Text(String),
    Email(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Metadata {
    pub author: Vec<MetadataPart>,
    pub committer: Option<Vec<MetadataPart>>,
}

pub(crate) struct MetadataInput<'a> {
    pub hash: &'a str,
    pub author_name: &'a str,
    pub author_email: &'a str,
    pub author_time: i64,
    pub committer_name: &'a str,
    pub committer_email: &'a str,
    pub commit_time: i64,
}

pub(crate) fn metadata(
    input: &MetadataInput<'_>,
    date_and_time: impl Fn(i64) -> (String, String),
) -> Metadata {
    let short_hash = input.hash.get(..SHORT_HASH_LENGTH).unwrap_or(input.hash);
    let (author_date, author_clock) = date_and_time(input.author_time);
    let mut author = vec![MetadataPart::Text(format!(
        "{short_hash} {}",
        short_presentation(input.author_name, input.author_email)
    ))];
    if !input.author_email.is_empty() {
        author.push(MetadataPart::Text(" ".to_string()));
        author.push(MetadataPart::Email(input.author_email.to_string()));
    }
    author.push(MetadataPart::Text(format!(
        " on {author_date} at {author_clock}"
    )));

    let same_person = is_same_person(
        input.author_name,
        input.author_email,
        input.committer_name,
        input.committer_email,
    );
    let time_differs = input.commit_time != input.author_time;
    let committer = if !same_person {
        let mut parts = vec![MetadataPart::Text(format!(
            "committed by {}",
            short_presentation(input.committer_name, input.committer_email)
        ))];
        if !input.committer_email.is_empty() {
            parts.push(MetadataPart::Text(" ".to_string()));
            parts.push(MetadataPart::Email(input.committer_email.to_string()));
        }
        if time_differs {
            let (date, clock) = date_and_time(input.commit_time);
            parts.push(MetadataPart::Text(format!(" on {date} at {clock}")));
        }
        Some(parts)
    } else if time_differs {
        let (date, clock) = date_and_time(input.commit_time);
        Some(vec![MetadataPart::Text(format!(
            "committed on {date} at {clock}"
        ))])
    } else {
        None
    };
    Metadata { author, committer }
}

pub(crate) const BRANCHES_LOADING_TEXT: &str = "In branches: loading…";
pub(crate) const NOT_IN_ANY_BRANCH_TEXT: &str = "Not in any branch";
pub(crate) const SHOW_ALL_LINK: &str = "Show all";
pub(crate) const HIDE_LINK: &str = "Hide";
pub(crate) const BRANCHES_TAIL_PREFIX: &str = "… ";

pub(crate) fn branches_head(count: usize) -> String {
    if count == 1 {
        "In 1 branch: ".to_string()
    } else {
        format!("In {count} branches: ")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BranchesLine {
    Loading,
    Empty,
    Expanded {
        head: String,
        branches: Vec<SharedString>,
    },
    Collapsed {
        head: String,
        shown: String,
        show_all: bool,
    },
}

pub(crate) fn branches_line(
    branches: Option<&[SharedString]>,
    expanded: bool,
    available_width: f32,
    measure: impl Fn(&str) -> f32,
) -> BranchesLine {
    let Some(branches) = branches else {
        return BranchesLine::Loading;
    };
    if branches.is_empty() {
        return BranchesLine::Empty;
    }
    let head = branches_head(branches.len());
    if expanded {
        return BranchesLine::Expanded {
            head,
            branches: branches.to_vec(),
        };
    }
    let tail = format!("{BRANCHES_TAIL_PREFIX}{SHOW_ALL_LINK}");
    let head_and_tail_width = measure(&head) + measure(&tail);
    if available_width <= head_and_tail_width {
        return BranchesLine::Collapsed {
            head,
            shown: String::new(),
            show_all: true,
        };
    }
    let mut available = available_width - head_and_tail_width;
    let mut shown = String::new();
    for (index, branch) in branches.iter().enumerate() {
        let separator = if index + 1 == branches.len() {
            ""
        } else {
            ", "
        };
        let text = format!("{branch}{separator}");
        let width = measure(&text);
        if width < available {
            shown.push_str(&text);
            available -= width;
        } else {
            let mut shortened = String::new();
            for character in text.chars() {
                let mut candidate = shortened.clone();
                candidate.push(character);
                if measure(&candidate) >= available {
                    break;
                }
                shortened = candidate;
            }
            shown.push_str(&shortened);
            return BranchesLine::Collapsed {
                head,
                shown,
                show_all: true,
            };
        }
    }
    BranchesLine::Collapsed {
        head,
        shown,
        show_all: false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RefSourceBranch {
    pub name: SharedString,
    pub is_remote: bool,
    pub sha: SharedString,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RefSource {
    pub head_sha: Option<SharedString>,
    pub current_branch: Option<SharedString>,
    pub branches: Vec<RefSourceBranch>,
    pub tags: Vec<(SharedString, SharedString)>,
}

pub(crate) fn refs_for_commit(source: &RefSource, sha: &str) -> Vec<RefLabel> {
    let mut labels = Vec::new();
    if source
        .head_sha
        .as_ref()
        .is_some_and(|head| head.as_ref() == sha)
    {
        labels.push(RefLabel {
            name: "HEAD".into(),
            kind: RefLabelKind::Head,
        });
    }
    for branch in &source.branches {
        if branch.sha.as_ref() == sha {
            labels.push(RefLabel {
                name: branch.name.clone(),
                kind: if branch.is_remote {
                    RefLabelKind::RemoteBranch
                } else {
                    RefLabelKind::LocalBranch
                },
            });
        }
    }
    for (name, tag_sha) in &source.tags {
        if tag_sha.as_ref() == sha {
            labels.push(RefLabel {
                name: name.clone(),
                kind: RefLabelKind::Tag,
            });
        }
    }
    let current_branch = source.current_branch.as_deref();
    labels.sort_by(|left, right| compare_ref_labels(left, right, current_branch));
    labels
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReferenceEntry {
    pub text: SharedString,
    pub icon: Option<(RefLabelKind, bool)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ReferenceLine {
    pub entries: Vec<ReferenceEntry>,
    pub hidden: usize,
}

pub(crate) fn reference_line(refs: &[RefLabel]) -> ReferenceLine {
    let visible = &refs[..refs.len().min(MAX_REFERENCES_SHOWN)];
    let mut kinds: Vec<RefLabelKind> = Vec::new();
    for label in visible {
        if !kinds.contains(&label.kind) {
            kinds.push(label.kind);
        }
    }
    let mut entries = Vec::new();
    for kind in kinds {
        let group: Vec<&RefLabel> = visible.iter().filter(|label| label.kind == kind).collect();
        for (index, label) in group.iter().enumerate() {
            let ending = if index + 1 == group.len() { "" } else { "," };
            entries.push(ReferenceEntry {
                text: format!("{}{ending}", label.name).into(),
                icon: (index == 0).then_some((kind, group.len() > 1)),
            });
        }
    }
    ReferenceLine {
        entries,
        hidden: refs.len() - visible.len(),
    }
}

pub(crate) fn split_references(refs: Vec<RefLabel>) -> (Vec<RefLabel>, Vec<RefLabel>) {
    refs.into_iter().partition(|label| {
        matches!(
            label.kind,
            RefLabelKind::Head | RefLabelKind::LocalBranch | RefLabelKind::RemoteBranch
        )
    })
}

pub(crate) fn showing_selected_text(shown: usize, requested: usize) -> String {
    format!("(showing {shown} of {requested} selected commits)")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_date_and_time(timestamp: i64) -> (String, String) {
        (format!("D{timestamp}"), format!("T{timestamp}"))
    }

    fn render(parts: &[MetadataPart]) -> String {
        parts
            .iter()
            .map(|part| match part {
                MetadataPart::Text(text) => text.clone(),
                MetadataPart::Email(email) => format!("<{email}>"),
            })
            .collect()
    }

    fn input<'a>(
        committer_name: &'a str,
        committer_email: &'a str,
        commit_time: i64,
    ) -> MetadataInput<'a> {
        MetadataInput {
            hash: "0123456789abcdef",
            author_name: "Jane Doe",
            author_email: "jane@example.com",
            author_time: 10,
            committer_name,
            committer_email,
            commit_time,
        }
    }

    #[test]
    fn metadata_line_names_the_committer_only_when_it_differs() {
        let same = metadata(&input("jane.doe", "jane@work.com", 10), fake_date_and_time);
        assert_eq!(
            render(&same.author),
            "01234567 Jane Doe <jane@example.com> on D10 at T10"
        );
        assert_eq!(
            same.committer, None,
            "VcsUserUtil treats 'Jane Doe' and 'jane.doe' as the same person"
        );

        let later = metadata(
            &input("Jane Doe", "jane@example.com", 20),
            fake_date_and_time,
        );
        assert_eq!(
            later.committer.as_deref().map(render),
            Some("committed on D20 at T20".to_string())
        );

        let other = metadata(&input("Bot", "", 10), fake_date_and_time);
        assert_eq!(
            other.committer.as_deref().map(render),
            Some("committed by Bot".to_string())
        );

        let other_later = metadata(&input("Bot", "bot@ci", 30), fake_date_and_time);
        assert_eq!(
            other_later.committer.as_deref().map(render),
            Some("committed by Bot <bot@ci> on D30 at T30".to_string())
        );
    }

    #[test]
    fn branches_line_collapses_to_the_available_width_and_expands_fully() {
        let branches: Vec<SharedString> = vec!["main".into(), "feature".into(), "release".into()];
        let measure = |text: &str| text.chars().count() as f32;
        assert_eq!(
            branches_line(None, false, 100., measure),
            BranchesLine::Loading
        );
        assert_eq!(
            branches_line(Some(&[]), false, 100., measure),
            BranchesLine::Empty
        );
        assert_eq!(
            branches_line(Some(&branches), false, 1000., measure),
            BranchesLine::Collapsed {
                head: "In 3 branches: ".to_string(),
                shown: "main, feature, release".to_string(),
                show_all: false,
            }
        );
        assert_eq!(
            branches_line(Some(&branches), false, 35., measure),
            BranchesLine::Collapsed {
                head: "In 3 branches: ".to_string(),
                shown: "main, fea".to_string(),
                show_all: true,
            }
        );
        assert_eq!(
            branches_line(Some(&branches), true, 10., measure),
            BranchesLine::Expanded {
                head: "In 3 branches: ".to_string(),
                branches,
            }
        );
        assert_eq!(branches_head(1), "In 1 branch: ");
    }

    #[test]
    fn subject_is_bold_only_when_it_is_a_single_line() {
        assert_eq!(
            message_presentation("Fix bug\n\nDetails\tmore", "Flint"),
            MessagePresentation {
                subject: Some("Fix bug".to_string()),
                body: "\nDetails    more".to_string(),
            }
        );
        assert_eq!(
            message_presentation("first\nsecond", "Flint"),
            MessagePresentation {
                subject: None,
                body: "first\nsecond".to_string(),
            }
        );
    }

    #[test]
    fn references_are_ordered_like_git_label_comparator_and_grouped_by_type() {
        let source = RefSource {
            head_sha: Some("abc".into()),
            current_branch: Some("feature".into()),
            branches: vec![
                RefSourceBranch {
                    name: "zeta".into(),
                    is_remote: false,
                    sha: "abc".into(),
                },
                RefSourceBranch {
                    name: "origin/main".into(),
                    is_remote: true,
                    sha: "abc".into(),
                },
                RefSourceBranch {
                    name: "feature".into(),
                    is_remote: false,
                    sha: "abc".into(),
                },
            ],
            tags: vec![("v1".into(), "abc".into())],
        };
        let (branches, tags) = split_references(refs_for_commit(&source, "abc"));
        let line = reference_line(&branches);
        let texts: Vec<&str> = line
            .entries
            .iter()
            .map(|entry| entry.text.as_ref())
            .collect();
        assert_eq!(texts, vec!["HEAD", "feature,", "zeta", "origin/main"]);
        assert_eq!(
            line.entries[1].icon,
            Some((RefLabelKind::LocalBranch, true))
        );
        assert_eq!(line.entries[2].icon, None);
        assert_eq!(tags.len(), 1);
    }
}
