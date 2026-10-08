use std::sync::Arc;

use anyhow::Result;
use git::repository::RepositoryOperation;
use gpui::{App, AsyncApp, Entity, SharedString, Task};
use project::Fs;
use project::git_store::Repository;
use util::ResultExt as _;

use super::dialog_texts::{DialogTexts, PaneTitleText, PaneTitleTexts, ShowDetails};
use super::labels::{
    CHERRY_PICK_HEAD_REVISION, CommitFact, HEAD_REVISION, MergeLabels, REBASE_HEAD_REVISION,
    RepositoryFacts, column_title, current_branch_presentable, load_repository_facts,
    resolve_merge_branch, resolve_rebase_onto_branch, short_hash,
};
use super::rich_text::RichText;

const DIALOG_TITLE: &str = "Conflicts";
const RESULT_TITLE: &str = "Result";
const DEFAULT_LEFT_TITLE: &str = "Your version";
const DEFAULT_RIGHT_TITLE: &str = "Changes from server";
const DEFAULT_LEFT_TITLE_FOR_BRANCH_PREFIX: &str = "Your version, branch ";
const DEFAULT_RIGHT_TITLE_WITH_BRANCH_PREFIX: &str = "Changes from branch ";
const DEFAULT_RIGHT_TITLE_WITHOUT_ONTO_INFO: &str = "Changes from diverging branches";
const CHANGES_FROM_PREFIX: &str = "Changes from ";
const CHERRY_PICK_LEFT_TITLE: &str = "Local Changes";
const CHERRY_PICK_RIGHT_TITLE_PREFIX: &str = "Changes from cherry-pick ";
const REBASE_RIGHT_TITLE_WITH_BRANCH_PREFIX: &str = "Already rebased commits and commits from ";
const REBASE_RIGHT_TITLE: &str = "Already rebased commits";
const UNSTASH_LEFT_TITLE: &str = "Local changes";
const UNSTASH_RIGHT_TITLE: &str = "Changes from stash";
const RESTORE_LEFT_STASH_TITLE: &str = "Uncommitted changes from the stash";
const RESTORE_LEFT_SHELF_TITLE: &str = "Uncommitted changes from the shelf";
const PREVIEW_MAX_LINES: usize = 3;
const PREVIEW_CUT_MARKER: &str = "...";
const FULL_HASH_LENGTHS: [usize; 2] = [40, 64];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RebaseUpstream {
    Commit(String),
    Reference(String),
}

impl RebaseUpstream {
    pub(crate) fn from_ref_string(reference: &str) -> Self {
        if reference.is_empty() {
            log::error!("Empty rebase upstream specified");
            return Self::Reference(HEAD_REVISION.to_string());
        }
        if is_possible_full_hash(reference) {
            Self::Commit(reference.to_string())
        } else {
            Self::Reference(reference.to_string())
        }
    }

    fn reference(&self) -> &str {
        match self {
            Self::Commit(reference) | Self::Reference(reference) => reference.as_str(),
        }
    }
}

fn is_possible_full_hash(reference: &str) -> bool {
    FULL_HASH_LENGTHS.contains(&reference.len())
        && reference.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RebaseCustomizerSpec {
    pub(crate) upstream: Option<RebaseUpstream>,
    pub(crate) branch: Option<String>,
    pub(crate) initial_branch: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnstashSpec {
    pub(crate) stash: SharedString,
    pub(crate) message: SharedString,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SmartRestoreSpec {
    pub(crate) operation_title: SharedString,
    pub(crate) destination_name: SharedString,
    pub(crate) stash: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CustomizerKind {
    GitDefault,
    DescriptionOverride(SharedString),
    Rebase(RebaseCustomizerSpec),
    Unstash(UnstashSpec),
    SmartRestore(SmartRestoreSpec),
}

struct ResolvedRebase {
    upstream: RebaseUpstream,
    branch: String,
}

impl ResolvedRebase {
    fn new(spec: &RebaseCustomizerSpec) -> Option<Self> {
        let initial_branch = spec
            .initial_branch
            .as_deref()
            .filter(|branch| !branch.trim().is_empty());
        let upstream = match spec.upstream.as_ref()? {
            RebaseUpstream::Reference(reference) if reference == HEAD_REVISION => {
                RebaseUpstream::from_ref_string(initial_branch?)
            }
            upstream => upstream.clone(),
        };
        let branch = spec
            .branch
            .as_deref()
            .filter(|branch| !branch.trim().is_empty())
            .or(initial_branch)?;
        Some(Self {
            upstream,
            branch: branch.to_string(),
        })
    }

    fn base_branch(&self) -> Option<&str> {
        match &self.upstream {
            RebaseUpstream::Commit(_) => None,
            RebaseUpstream::Reference(reference) => Some(reference.as_str()),
        }
    }

    fn base_hash(&self) -> Option<&str> {
        match &self.upstream {
            RebaseUpstream::Commit(commit) => Some(commit.as_str()),
            RebaseUpstream::Reference(_) => None,
        }
    }

    fn base_presentable(&self) -> String {
        match &self.upstream {
            RebaseUpstream::Commit(commit) => short_hash(commit),
            RebaseUpstream::Reference(reference) => reference.clone(),
        }
    }
}

pub(super) struct RebaseDescription<'a> {
    pub(super) rebasing_branch: Option<&'a str>,
    pub(super) base_branch: Option<&'a str>,
    pub(super) base_hash: Option<&'a str>,
    pub(super) conflict_commit: Option<&'a CommitFact>,
}

pub(super) fn default_description(files_count: usize) -> RichText {
    if files_count >= 2 {
        RichText::plain("The following files have conflicts:")
    } else {
        RichText::plain("The following file has conflicts:")
    }
}

pub(super) fn merge_description(
    merge_branch_count: usize,
    first_merge_branch: &str,
    current_branch_count: usize,
    first_current_branch: &str,
) -> RichText {
    RichText::plain("Merging ")
        .with_rich(branch_phrase(merge_branch_count, first_merge_branch))
        .with_plain(" into ")
        .with_rich(branch_phrase(current_branch_count, first_current_branch))
}

fn branch_phrase(branch_count: usize, first_branch: &str) -> RichText {
    if branch_count >= 2 {
        RichText::plain("diverging branches")
    } else {
        RichText::plain("branch ").with_bold(first_branch)
    }
}

pub(super) fn rebase_description(parts: &RebaseDescription) -> RichText {
    let mut text = RichText::plain("Rebasing");
    if let Some(rebasing_branch) = parts.rebasing_branch {
        text = text.with_plain(" branch ").with_bold(rebasing_branch);
    }
    text = match (parts.base_branch, parts.base_hash) {
        (Some(base_branch), base_hash) => {
            let text = text.with_plain(" onto branch ").with_bold(base_branch);
            match base_hash {
                Some(base_hash) => text.with_plain(format!(", revision {}", short_hash(base_hash))),
                None => text,
            }
        }
        (None, Some(base_hash)) => text.with_plain(" onto ").with_bold(short_hash(base_hash)),
        (None, None) => text.with_plain(" onto diverging branches"),
    };
    match parts.conflict_commit {
        Some(commit) => text
            .with_plain(". Current commit ")
            .with_code(short_hash(&commit.sha))
            .with_plain(format!(" made by {}:", commit.author_name))
            .with_line_break()
            .with_rich(commit_message_preview(&commit.message)),
        None => text,
    }
}

pub(super) fn cherry_pick_description(
    commit_count: usize,
    commit_short_hash: &str,
    made_by: Option<(&str, &str)>,
) -> RichText {
    let mut text = RichText::plain("Conflicts during cherry-picking ");
    text = if commit_count >= 2 {
        text.with_plain("multiple commits")
    } else {
        text.with_plain("commit ").with_code(commit_short_hash)
    };
    match made_by {
        Some((author, message)) => text
            .with_plain(format!(" made by {author}"))
            .with_line_break()
            .with_rich(commit_message_preview(message)),
        None => text,
    }
}

fn unstash_description(spec: &UnstashSpec) -> RichText {
    RichText::plain("Conflicts during unstashing ")
        .with_code(format!("{}\"{}\"", spec.stash, spec.message))
}

fn restore_description(spec: &SmartRestoreSpec) -> RichText {
    RichText::plain(format!(
        "Uncommitted changes that were saved before {} have conflicts with files from ",
        spec.operation_title
    ))
    .with_code(spec.destination_name.to_string())
}

pub(super) fn commit_message_preview(message: &str) -> RichText {
    let mut preview = RichText::new();
    for (index, line) in trimmed_preview(message).split('\n').enumerate() {
        if index > 0 {
            preview = preview.with_line_break();
        }
        preview = preview.with_code(line.trim_end_matches('\r'));
    }
    preview
}

fn trimmed_preview(message: &str) -> String {
    let lines = split_lines(message);
    if lines.len() <= PREVIEW_MAX_LINES {
        return message.to_string();
    }
    let mut kept: Vec<&str> = lines.into_iter().take(PREVIEW_MAX_LINES).collect();
    while kept.last().is_some_and(|line| line.trim().is_empty()) {
        kept.pop();
    }
    format!("{}{PREVIEW_CUT_MARKER}", kept.join("\n"))
}

fn split_lines(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut line_start = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' => {
                lines.push(&text[line_start..index]);
                index += 1;
                line_start = index;
            }
            b'\r' => {
                lines.push(&text[line_start..index]);
                index += 1;
                if bytes.get(index) == Some(&b'\n') {
                    index += 1;
                }
                line_start = index;
            }
            _ => index += 1,
        }
    }
    lines.push(&text[line_start..]);
    lines
}

fn commit_title(label: RichText, sha: &str) -> PaneTitleText {
    let dialog_title = SharedString::from(label.plain_text());
    PaneTitleText {
        label,
        show_details: Some(ShowDetails::Commit {
            sha: SharedString::from(sha.to_string()),
            dialog_title,
        }),
    }
}

fn ranged_title(label: RichText, first: &str, second: &str) -> PaneTitleText {
    let dialog_title = SharedString::from(label.plain_text());
    PaneTitleText {
        label,
        show_details: Some(ShowDetails::CommitRange {
            first: SharedString::from(first.to_string()),
            second: SharedString::from(second.to_string()),
            dialog_title,
        }),
    }
}

fn unadorned_title(label: RichText) -> PaneTitleText {
    PaneTitleText {
        label,
        show_details: None,
    }
}

fn default_left_title() -> PaneTitleText {
    PaneTitleText::plain(DEFAULT_LEFT_TITLE)
}

pub(super) fn default_right_title(revision: Option<&str>) -> PaneTitleText {
    match revision {
        Some(revision) => {
            PaneTitleText::plain(format!("{DEFAULT_RIGHT_TITLE} (revision {revision})"))
        }
        None => PaneTitleText::plain(DEFAULT_RIGHT_TITLE),
    }
}

fn default_left_title_for_branch(branch: &str) -> PaneTitleText {
    unadorned_title(RichText::plain(DEFAULT_LEFT_TITLE_FOR_BRANCH_PREFIX).with_bold(branch))
}

fn default_right_title_for_branch(
    branch_name: Option<&str>,
    revision: Option<&str>,
) -> PaneTitleText {
    let label = match (branch_name, revision) {
        (Some(branch_name), revision) => {
            let label =
                RichText::plain(DEFAULT_RIGHT_TITLE_WITH_BRANCH_PREFIX).with_bold(branch_name);
            match revision {
                Some(revision) => label.with_plain(format!(", revision {}", short_hash(revision))),
                None => label,
            }
        }
        (None, Some(revision)) => {
            RichText::plain(CHANGES_FROM_PREFIX).with_bold(short_hash(revision))
        }
        (None, None) => RichText::plain(DEFAULT_RIGHT_TITLE_WITHOUT_ONTO_INFO),
    };
    unadorned_title(label)
}

pub(super) fn last_revision(facts: &RepositoryFacts, reverse: bool) -> Option<String> {
    if reverse {
        return facts.head.clone();
    }
    facts
        .merge_head
        .as_ref()
        .map(|commit| commit.sha.clone())
        .or_else(|| {
            facts
                .cherry_pick_head
                .as_ref()
                .map(|commit| commit.sha.clone())
        })
        .or_else(|| facts.rebase_current_commit.clone())
}

fn merge_titles(
    facts: &RepositoryFacts,
    merge_base: Option<&str>,
) -> Option<(PaneTitleText, PaneTitleText)> {
    let head = facts.head.as_deref()?;
    let current_branch = facts.branch.clone().unwrap_or_else(|| short_hash(head));
    let merge_branch = resolve_merge_branch(facts)?;
    let merge_base = merge_base?;
    let left = ranged_title(
        RichText::plain(CHANGES_FROM_PREFIX).with_bold(current_branch),
        merge_base,
        head,
    );
    let right = ranged_title(
        RichText::plain(CHANGES_FROM_PREFIX).with_bold(merge_branch.presentable()),
        merge_base,
        &merge_branch.hash,
    );
    Some((left, right))
}

fn rebase_state_titles(
    facts: &RepositoryFacts,
    merge_base: Option<&str>,
) -> Option<(PaneTitleText, PaneTitleText)> {
    let head = facts.head.as_deref()?;
    let rebasing_branch = facts.branch.clone().unwrap_or_else(|| short_hash(head));
    let onto = resolve_rebase_onto_branch(facts)?;
    let rebase_head = facts.rebase_head.as_ref()?;
    let merge_base = merge_base?;
    let left = commit_title(
        RichText::plain(format!("Rebasing {} from ", short_hash(&rebase_head.sha)))
            .with_bold(rebasing_branch),
        &rebase_head.sha,
    );
    let right_label = match &onto.branch_name {
        Some(branch_name) => {
            RichText::plain(REBASE_RIGHT_TITLE_WITH_BRANCH_PREFIX).with_bold(branch_name.as_str())
        }
        None => RichText::plain(REBASE_RIGHT_TITLE),
    };
    let right = ranged_title(right_label, merge_base, HEAD_REVISION);
    Some((left, right))
}

fn cherry_pick_titles(
    facts: &RepositoryFacts,
    merge_base: Option<&str>,
) -> Option<(PaneTitleText, PaneTitleText)> {
    let cherry_pick_head = facts.cherry_pick_head.as_ref()?;
    let merge_base = merge_base?;
    let left = ranged_title(
        RichText::plain(CHERRY_PICK_LEFT_TITLE),
        merge_base,
        HEAD_REVISION,
    );
    let right = commit_title(
        RichText::plain(format!(
            "{CHERRY_PICK_RIGHT_TITLE_PREFIX}{}",
            short_hash(&cherry_pick_head.sha)
        )),
        &cherry_pick_head.sha,
    );
    Some((left, right))
}

fn git_default_pane_titles(
    facts: &RepositoryFacts,
    merge_base: Option<&str>,
    reverse: bool,
) -> PaneTitleTexts {
    let customized = match facts.state() {
        Some(RepositoryOperation::Merge) => merge_titles(facts, merge_base),
        Some(RepositoryOperation::Rebase) => rebase_state_titles(facts, merge_base),
        Some(RepositoryOperation::CherryPick) => cherry_pick_titles(facts, merge_base),
        Some(RepositoryOperation::Revert) | None => None,
    };
    let (left, right) = customized.unwrap_or_else(|| {
        (
            default_left_title(),
            default_right_title(last_revision(facts, reverse).as_deref()),
        )
    });
    PaneTitleTexts {
        left,
        result: PaneTitleText::plain(RESULT_TITLE),
        right,
    }
}

fn texts_with(labels: &MergeLabels, pane_titles: PaneTitleTexts) -> DialogTexts {
    DialogTexts {
        dialog_title: DIALOG_TITLE.into(),
        yours_column: labels.yours_column().into(),
        theirs_column: labels.theirs_column().into(),
        pane_titles,
    }
}

fn fixed_pane_titles(left: PaneTitleText, right: PaneTitleText) -> PaneTitleTexts {
    PaneTitleTexts {
        left,
        result: PaneTitleText::plain(RESULT_TITLE),
        right,
    }
}

fn rebase_process_texts(
    rebase: &ResolvedRebase,
    facts: &RepositoryFacts,
    merge_base: Option<&str>,
    reverse: bool,
) -> DialogTexts {
    let left = match &facts.rebase_head {
        Some(commit) => commit_title(
            RichText::plain(format!("Rebasing {} from ", short_hash(&commit.sha)))
                .with_bold(rebase.branch.as_str()),
            &commit.sha,
        ),
        None => default_left_title_for_branch(&rebase.branch),
    };
    let right = match merge_base {
        Some(merge_base) => {
            let label = match rebase.base_branch() {
                Some(base_branch) => {
                    RichText::plain(REBASE_RIGHT_TITLE_WITH_BRANCH_PREFIX).with_bold(base_branch)
                }
                None => RichText::plain(REBASE_RIGHT_TITLE),
            };
            ranged_title(label, merge_base, HEAD_REVISION)
        }
        None => {
            let revision = last_revision(facts, reverse);
            default_right_title_for_branch(
                rebase.base_branch(),
                revision.as_deref().or(rebase.base_hash()),
            )
        }
    };
    let base_presentable = rebase.base_presentable();
    DialogTexts {
        dialog_title: DIALOG_TITLE.into(),
        yours_column: column_title(false, Some(rebase.branch.as_str())).into(),
        theirs_column: column_title(true, Some(base_presentable.as_str())).into(),
        pane_titles: fixed_pane_titles(left, right),
    }
}

pub(crate) fn build_dialog_texts(
    kind: &CustomizerKind,
    facts: &RepositoryFacts,
    merge_base: Option<&str>,
    reverse: bool,
) -> DialogTexts {
    let labels = MergeLabels::for_facts(facts);
    match kind {
        CustomizerKind::Rebase(spec) => match ResolvedRebase::new(spec) {
            Some(rebase) => rebase_process_texts(&rebase, facts, merge_base, reverse),
            None => texts_with(&labels, git_default_pane_titles(facts, merge_base, reverse)),
        },
        CustomizerKind::GitDefault | CustomizerKind::DescriptionOverride(_) => {
            texts_with(&labels, git_default_pane_titles(facts, merge_base, reverse))
        }
        CustomizerKind::Unstash(_) => texts_with(
            &labels,
            fixed_pane_titles(
                PaneTitleText::plain(UNSTASH_LEFT_TITLE),
                PaneTitleText::plain(UNSTASH_RIGHT_TITLE),
            ),
        ),
        CustomizerKind::SmartRestore(spec) => {
            let left_title = if spec.stash {
                RESTORE_LEFT_STASH_TITLE
            } else {
                RESTORE_LEFT_SHELF_TITLE
            };
            texts_with(
                &labels,
                fixed_pane_titles(
                    PaneTitleText::plain(left_title),
                    unadorned_title(
                        RichText::plain(CHANGES_FROM_PREFIX)
                            .with_bold(spec.destination_name.to_string()),
                    ),
                ),
            )
        }
    }
}

fn git_default_description(facts: &RepositoryFacts, files_count: usize) -> RichText {
    if let Some(merge_branch) = resolve_merge_branch(facts)
        && let Some(current_branch) = current_branch_presentable(facts)
    {
        return merge_description(1, &merge_branch.presentable(), 1, &current_branch);
    }
    if let Some(onto) = resolve_rebase_onto_branch(facts) {
        return rebase_description(&RebaseDescription {
            rebasing_branch: current_branch_presentable(facts).as_deref(),
            base_branch: onto.branch_name.as_deref(),
            base_hash: Some(onto.hash.as_str()),
            conflict_commit: facts.rebase_head.as_ref(),
        });
    }
    if let Some(commit) = &facts.cherry_pick_head {
        return cherry_pick_description(
            1,
            &short_hash(&commit.sha),
            Some((commit.author_name.as_str(), commit.message.as_str())),
        );
    }
    default_description(files_count)
}

pub(crate) fn build_description(
    kind: &CustomizerKind,
    facts: &RepositoryFacts,
    files_count: usize,
) -> RichText {
    match kind {
        CustomizerKind::GitDefault => git_default_description(facts, files_count),
        CustomizerKind::DescriptionOverride(description) => {
            if description.is_empty() {
                git_default_description(facts, files_count)
            } else {
                RichText::plain(description.to_string())
            }
        }
        CustomizerKind::Rebase(spec) => match ResolvedRebase::new(spec) {
            Some(rebase) => rebase_description(&RebaseDescription {
                rebasing_branch: Some(rebase.branch.as_str()),
                base_branch: rebase.base_branch(),
                base_hash: rebase.base_hash(),
                conflict_commit: facts.rebase_head.as_ref(),
            }),
            None => git_default_description(facts, files_count),
        },
        CustomizerKind::Unstash(spec) => unstash_description(spec),
        CustomizerKind::SmartRestore(spec) => restore_description(spec),
    }
}

fn state_merge_base_request(facts: &RepositoryFacts) -> Option<(String, String)> {
    match facts.state()? {
        RepositoryOperation::Merge => {
            Some((facts.head.clone()?, facts.merge_head.as_ref()?.sha.clone()))
        }
        RepositoryOperation::Rebase => {
            Some((REBASE_HEAD_REVISION.to_string(), facts.rebase_onto.clone()?))
        }
        RepositoryOperation::CherryPick => Some((
            CHERRY_PICK_HEAD_REVISION.to_string(),
            HEAD_REVISION.to_string(),
        )),
        RepositoryOperation::Revert => None,
    }
}

pub(super) fn merge_base_request(
    kind: &CustomizerKind,
    facts: &RepositoryFacts,
) -> Option<(String, String)> {
    match kind {
        CustomizerKind::Rebase(spec) => match ResolvedRebase::new(spec) {
            Some(rebase) => Some((rebase.upstream.reference().to_string(), rebase.branch)),
            None => state_merge_base_request(facts),
        },
        CustomizerKind::GitDefault | CustomizerKind::DescriptionOverride(_) => {
            state_merge_base_request(facts)
        }
        CustomizerKind::Unstash(_) | CustomizerKind::SmartRestore(_) => None,
    }
}

async fn load_merge_base(
    repository: &Entity<Repository>,
    request: Option<(String, String)>,
    cx: &mut AsyncApp,
) -> Option<String> {
    let (first, second) = request?;
    let merge_base = repository.update(cx, |repository, cx| {
        repository.merge_base(first, second, cx)
    });
    merge_base.await.warn_on_err().flatten()
}

pub(crate) fn load_dialog_texts(
    repository: &Entity<Repository>,
    fs: &Arc<dyn Fs>,
    kind: &CustomizerKind,
    reverse: bool,
    cx: &mut App,
) -> Task<Result<DialogTexts>> {
    let repository = repository.clone();
    let fs = fs.clone();
    let kind = kind.clone();
    cx.spawn(async move |cx| {
        let facts = load_repository_facts(&repository, &fs, cx).await?;
        let merge_base = load_merge_base(&repository, merge_base_request(&kind, &facts), cx).await;
        anyhow::Ok(build_dialog_texts(
            &kind,
            &facts,
            merge_base.as_deref(),
            reverse,
        ))
    })
}

pub(crate) fn load_description(
    repository: &Entity<Repository>,
    fs: &Arc<dyn Fs>,
    kind: &CustomizerKind,
    files_count: usize,
    cx: &mut App,
) -> Task<RichText> {
    let repository = repository.clone();
    let fs = fs.clone();
    let kind = kind.clone();
    cx.spawn(async move |cx| {
        let facts = load_repository_facts(&repository, &fs, cx)
            .await
            .warn_on_err()
            .unwrap_or_default();
        build_description(&kind, &facts, files_count)
    })
}
