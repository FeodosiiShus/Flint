use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use git::repository::{Branch, BranchesScanResult, CommitDetails, RepositoryOperation};
use gpui::{AsyncApp, Entity, SharedString, Task};
use project::Fs;
use project::git_store::Repository;
use util::ResultExt as _;

pub(crate) const HEAD_REVISION: &str = "HEAD";
pub(crate) const MERGE_HEAD_REVISION: &str = "MERGE_HEAD";
pub(crate) const CHERRY_PICK_HEAD_REVISION: &str = "CHERRY_PICK_HEAD";
pub(crate) const REBASE_HEAD_REVISION: &str = "REBASE_HEAD";

const SHORT_HASH_LENGTH: usize = 8;
const CHERRY_PICK_LABEL: &str = "cherry-pick";
const REMOTE_ORIGIN_HEAD: &str = "origin/HEAD";
const DETACHED_HEAD_MARKER: &str = "detached HEAD";
const LOCAL_BRANCH_PREFIX: &str = "refs/heads/";
const MERGE_MESSAGE_FILE: &str = "MERGE_MSG";
const REBASE_APPLY_DIRECTORY: &str = "rebase-apply";
const REBASE_MERGE_DIRECTORY: &str = "rebase-merge";
const REBASE_HEAD_NAME_FILE: &str = "head-name";

pub(crate) fn short_hash(sha: &str) -> String {
    sha.chars().take(SHORT_HASH_LENGTH).collect()
}

pub(crate) fn column_title(is_theirs: bool, branch: Option<&str>) -> String {
    match (is_theirs, branch) {
        (true, Some(branch)) => format!("Theirs ({branch})"),
        (true, None) => "Theirs".to_string(),
        (false, Some(branch)) => format!("Yours ({branch})"),
        (false, None) => "Yours".to_string(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CommitFact {
    pub(crate) sha: String,
    pub(crate) author_name: String,
    pub(crate) message: String,
}

impl CommitFact {
    fn from_details(details: &CommitDetails) -> Self {
        Self {
            sha: details.sha.to_string(),
            author_name: details.author_name.to_string(),
            message: details.message.trim_end().to_string(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct BranchTip {
    pub(crate) name: String,
    pub(crate) sha: String,
    pub(crate) is_remote: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RefInfo {
    pub(crate) hash: String,
    pub(crate) branch_name: Option<String>,
}

impl RefInfo {
    pub(crate) fn presentable(&self) -> String {
        self.branch_name
            .clone()
            .unwrap_or_else(|| short_hash(&self.hash))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RepositoryFacts {
    pub(crate) operation: Option<RepositoryOperation>,
    pub(crate) head: Option<String>,
    pub(crate) branch: Option<String>,
    pub(crate) branches: Vec<BranchTip>,
    pub(crate) merge_head: Option<CommitFact>,
    pub(crate) cherry_pick_head: Option<CommitFact>,
    pub(crate) rebase_head: Option<CommitFact>,
    pub(crate) rebase_onto: Option<String>,
    pub(crate) rebase_current_commit: Option<String>,
    pub(crate) merge_message: Option<String>,
}

impl RepositoryFacts {
    fn merge_message_ref_name(&self) -> Option<&str> {
        let first_line = self.merge_message.as_deref()?.split(['\n', '\r']).next()?;
        quoted_ref_name(first_line)
    }

    pub(crate) fn state(&self) -> Option<RepositoryOperation> {
        if self.merge_head.is_some() {
            return Some(RepositoryOperation::Merge);
        }
        match self.operation {
            Some(RepositoryOperation::Rebase) => Some(RepositoryOperation::Rebase),
            Some(RepositoryOperation::CherryPick) if self.branch.is_some() => {
                Some(RepositoryOperation::CherryPick)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MergeLabels {
    pub(crate) yours: Option<SharedString>,
    pub(crate) theirs: Option<SharedString>,
}

impl MergeLabels {
    pub(crate) fn for_facts(facts: &RepositoryFacts) -> Self {
        Self {
            yours: current_branch_presentable(facts).map(SharedString::from),
            theirs: merge_branch_or_cherry_pick(facts).map(SharedString::from),
        }
    }

    pub(crate) fn yours_column(&self) -> String {
        column_title(false, self.yours.as_deref())
    }

    pub(crate) fn theirs_column(&self) -> String {
        column_title(true, self.theirs.as_deref())
    }
}

pub(crate) fn quoted_ref_name(line: &str) -> Option<&str> {
    let mut search_start = 0;
    while let Some(offset) = line[search_start..].find('\'') {
        let opening = search_start + offset;
        let after_opening = &line[opening + 1..];
        let closing = after_opening.find('\'')?;
        if closing > 0 {
            return Some(&after_opening[..closing]);
        }
        search_start = opening + 1;
    }
    None
}

pub(crate) fn resolve_branch_name(
    branches: &[BranchTip],
    hash: &str,
    preferred_name: Option<&str>,
) -> RefInfo {
    let at_hash = |is_remote: bool| {
        branches
            .iter()
            .filter(move |branch| {
                branch.is_remote == is_remote
                    && branch.sha.eq_ignore_ascii_case(hash)
                    && !(is_remote && branch.name == REMOTE_ORIGIN_HEAD)
            })
            .collect::<Vec<_>>()
    };
    let mut candidates = at_hash(false);
    if candidates.is_empty() {
        candidates = at_hash(true);
    }
    let branch_name = match candidates.as_slice() {
        [only] => Some(only.name.clone()),
        _ => preferred_name.and_then(|preferred| {
            candidates
                .iter()
                .find(|branch| branch.name == preferred)
                .map(|branch| branch.name.clone())
        }),
    };
    RefInfo {
        hash: hash.to_string(),
        branch_name,
    }
}

pub(crate) fn current_branch_presentable(facts: &RepositoryFacts) -> Option<String> {
    facts
        .branch
        .clone()
        .or_else(|| facts.head.as_deref().map(short_hash))
}

pub(crate) fn resolve_merge_branch(facts: &RepositoryFacts) -> Option<RefInfo> {
    let merge_head = facts.merge_head.as_ref()?;
    Some(resolve_branch_name(
        &facts.branches,
        &merge_head.sha,
        facts.merge_message_ref_name(),
    ))
}

pub(crate) fn resolve_rebase_onto_branch(facts: &RepositoryFacts) -> Option<RefInfo> {
    let onto = facts.rebase_onto.as_deref()?;
    Some(resolve_branch_name(&facts.branches, onto, None))
}

pub(crate) fn merge_branch_or_cherry_pick(facts: &RepositoryFacts) -> Option<String> {
    resolve_merge_branch(facts)
        .map(|merge_branch| merge_branch.presentable())
        .or_else(|| resolve_rebase_onto_branch(facts).map(|onto| onto.presentable()))
        .or_else(|| {
            facts
                .cherry_pick_head
                .as_ref()
                .map(|_| CHERRY_PICK_LABEL.to_string())
        })
}

pub(crate) fn rebase_branch_name(head_name: &str) -> Option<String> {
    let head_name = head_name.trim();
    if head_name.is_empty() || head_name == DETACHED_HEAD_MARKER {
        return None;
    }
    Some(
        head_name
            .strip_prefix(LOCAL_BRANCH_PREFIX)
            .unwrap_or(head_name)
            .to_string(),
    )
}

fn branch_tip(branch: &Branch) -> Option<BranchTip> {
    let commit = branch.most_recent_commit.as_ref()?;
    Some(BranchTip {
        name: branch.name().to_string(),
        sha: commit.sha.to_string(),
        is_remote: branch.is_remote(),
    })
}

fn current_branch_name(scan: &BranchesScanResult) -> Option<String> {
    scan.branches
        .iter()
        .find(|branch| branch.is_head && !branch.is_remote())
        .map(|branch| branch.name().to_string())
}

fn start_show(
    repository: &Entity<Repository>,
    revision: &str,
    cx: &mut AsyncApp,
) -> Task<Result<CommitDetails>> {
    let revision = revision.to_string();
    repository.read_with(cx, |repository, app| repository.show_commit(revision, app))
}

async fn resolved_commit(task: Task<Result<CommitDetails>>, revision: &str) -> Option<CommitFact> {
    match task.await {
        Ok(details) => Some(CommitFact::from_details(&details)),
        Err(error) => {
            log::debug!("{revision} does not resolve to a commit: {error:#}");
            None
        }
    }
}

async fn read_git_file(
    fs: &Arc<dyn Fs>,
    git_directory: &Path,
    components: &[&str],
) -> Option<String> {
    let path = components
        .iter()
        .fold(git_directory.to_path_buf(), |path, component| {
            path.join(component)
        });
    if !fs.is_file(&path).await {
        return None;
    }
    fs.load(&path).await.log_err()
}

async fn read_rebase_branch(fs: &Arc<dyn Fs>, git_directory: &Path) -> Option<String> {
    let head_name = match read_git_file(
        fs,
        git_directory,
        &[REBASE_APPLY_DIRECTORY, REBASE_HEAD_NAME_FILE],
    )
    .await
    {
        Some(head_name) => head_name,
        None => {
            read_git_file(
                fs,
                git_directory,
                &[REBASE_MERGE_DIRECTORY, REBASE_HEAD_NAME_FILE],
            )
            .await?
        }
    };
    rebase_branch_name(&head_name)
}

pub(crate) async fn load_repository_facts(
    repository: &Entity<Repository>,
    fs: &Arc<dyn Fs>,
    cx: &mut AsyncApp,
) -> Result<RepositoryFacts> {
    let operation = repository.update(cx, |repository, _| repository.operation_in_progress());
    let head = repository.update(cx, |repository, _| repository.head_sha());
    let scan = repository.update(cx, |repository, _| repository.branches());
    let rebase_onto = repository.update(cx, |repository, cx| repository.rebase_onto(cx));
    let rebase_current_commit =
        repository.update(cx, |repository, cx| repository.rebase_current_commit(cx));
    let merge_head = start_show(repository, MERGE_HEAD_REVISION, cx);
    let cherry_pick_head = start_show(repository, CHERRY_PICK_HEAD_REVISION, cx);
    let rebase_head = start_show(repository, REBASE_HEAD_REVISION, cx);
    let git_directory = repository.read_with(cx, |repository, _| {
        repository.repository_dir_abs_path.clone()
    });

    let operation = operation.await?.warn_on_err().flatten();
    let head = head.await?.warn_on_err().flatten();
    let scan = scan.await?.warn_on_err();
    let rebase_onto = rebase_onto.await.warn_on_err().flatten();
    let rebase_current_commit = rebase_current_commit.await.warn_on_err().flatten();
    let merge_head = resolved_commit(merge_head, MERGE_HEAD_REVISION).await;
    let cherry_pick_head = resolved_commit(cherry_pick_head, CHERRY_PICK_HEAD_REVISION).await;
    let rebase_head = resolved_commit(rebase_head, REBASE_HEAD_REVISION).await;

    let branch = if operation == Some(RepositoryOperation::Rebase) {
        read_rebase_branch(fs, &git_directory).await
    } else {
        scan.as_ref().and_then(current_branch_name)
    };
    let branches: Vec<BranchTip> = scan
        .as_ref()
        .map(|scan| scan.branches.iter().filter_map(branch_tip).collect())
        .unwrap_or_default();
    let merge_message = read_git_file(fs, &git_directory, &[MERGE_MESSAGE_FILE]).await;

    Ok(RepositoryFacts {
        operation,
        head,
        branch,
        branches,
        merge_head,
        cherry_pick_head,
        rebase_head,
        rebase_onto,
        rebase_current_commit,
        merge_message,
    })
}
