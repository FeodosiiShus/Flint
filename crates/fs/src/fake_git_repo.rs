use std::path::Path;

use crate::{FakeFs, FakeFsEntry, Fs, RemoveOptions, RenameOptions};
use anyhow::{Context as _, Result, bail};
use async_channel::Sender;
use collections::{HashMap, HashSet};
use futures::FutureExt as _;
use futures::channel::oneshot;
use futures::future::{self, BoxFuture, join_all};
use git::repository::GitCommitTemplate;
use git::{
    Oid, RunHook,
    blame::Blame,
    repository::{
        AskPassDelegate, Branch, CommitData, CommitDataReader, CommitDetails, CommitOptions,
        CommitSummary, ConflictSide, CreateWorktreeTarget, FetchOptions,
        FileHistoryChangedFileSets, GRAPH_CHUNK_SIZE, GitFailure, GitFailureKind, GitRepository,
        GitRepositoryCheckpoint, InitialGraphCommitData, LogOrder, LogSource, MergeOutcome,
        PushOptions, RebaseAction, RebaseOutcome, RefEdit, Remote, RemoteCommandOutput, RepoPath,
        RepositoryOperation, ResetMode, SearchCommitArgs, Tag, UnmergedEntry,
        UnmergedStagePresence, Upstream, UpstreamTracking, UpstreamTrackingStatus, Worktree,
        commit_hash_search_query,
    },
    stash::GitStash,
    status::{
        DiffTreeType, FileStatus, GitStatus, StatusCode, TrackedStatus, TreeDiff, TreeDiffStatus,
        UnmergedStatus, UnmergedStatusCode,
    },
};
use gpui::{AsyncApp, BackgroundExecutor, SharedString, Task};
use ignore::gitignore::GitignoreBuilder;
use parking_lot::Mutex;
use rope::Rope;
use std::{path::PathBuf, sync::Arc, sync::atomic::AtomicBool, time::SystemTime};
use text::LineEnding;
use util::{paths::PathStyle, rel_path::RelPath};

#[derive(Clone)]
pub struct FakeGitRepository {
    pub(crate) fs: Arc<FakeFs>,
    pub(crate) checkpoints: Arc<Mutex<HashMap<Oid, FakeFsEntry>>>,
    pub(crate) executor: BackgroundExecutor,
    pub(crate) dot_git_path: PathBuf,
    pub(crate) repository_dir_path: PathBuf,
    pub(crate) common_dir_path: PathBuf,
    pub(crate) is_trusted: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
pub struct FakeCommitSnapshot {
    pub head_contents: HashMap<RepoPath, Vec<u8>>,
    pub index_contents: HashMap<RepoPath, Vec<u8>>,
    pub sha: String,
}

#[derive(Debug, Clone)]
pub enum FakeCommitDataEntry {
    Success(CommitData),
    Fail(CommitData),
}

#[derive(Debug, Clone)]
pub struct FakeGitRepositoryState {
    pub commit_history: Vec<FakeCommitSnapshot>,
    pub event_emitter: async_channel::Sender<PathBuf>,
    pub unmerged_paths: HashMap<RepoPath, UnmergedStatus>,
    pub conflict_stages: HashMap<RepoPath, FakeConflictStages>,
    pub merge_message: Option<String>,
    pub head_contents: HashMap<RepoPath, Vec<u8>>,
    pub index_contents: HashMap<RepoPath, Vec<u8>>,
    // everything in commit contents is in oids
    pub merge_base_contents: HashMap<RepoPath, Oid>,
    pub merge_base_commits: HashMap<(String, String), String>,
    pub oids: HashMap<Oid, Vec<u8>>,
    pub blames: HashMap<RepoPath, Blame>,
    pub blames_at_revision: HashMap<(RepoPath, Oid), Blame>,
    pub current_branch_name: Option<String>,
    pub branches: HashSet<String>,
    /// List of remotes, keys are names and values are URLs
    pub remotes: HashMap<String, String>,
    pub simulated_index_write_error_message: Option<String>,
    pub simulated_create_worktree_error: Option<String>,
    pub simulated_graph_error: Option<String>,
    pub branches_requiring_force_delete: HashSet<String>,
    pub worktrees_requiring_force_delete: HashSet<PathBuf>,
    pub refs: HashMap<String, String>,
    pub graph_commits: Vec<Arc<InitialGraphCommitData>>,
    pub commit_data: HashMap<Oid, FakeCommitDataEntry>,
    pub stash_entries: GitStash,
    pub commit_template: Option<GitCommitTemplate>,
    pub blob_read_gate: Option<FakeBlobReadGate>,
    pub tags: Vec<Tag>,
    pub upstreams: HashMap<String, Upstream>,
    pub recent_branches: Vec<String>,
    pub ref_histories: HashMap<String, Vec<CommitSummary>>,
    pub merge_conflicts: HashMap<String, Vec<(RepoPath, FakeConflictStages)>>,
    pub rebase_conflicts: HashMap<String, Vec<(RepoPath, FakeConflictStages)>>,
    pub rebase_session: Option<FakeRebaseSession>,
    pub simulated_failures: HashMap<FakeGitOperation, FakeGitFailure>,
    pub pushed_tags: Vec<(String, String)>,
}

impl FakeGitRepositoryState {
    pub fn new(event_emitter: async_channel::Sender<PathBuf>) -> Self {
        FakeGitRepositoryState {
            event_emitter,
            blob_read_gate: None,
            head_contents: Default::default(),
            index_contents: Default::default(),
            unmerged_paths: Default::default(),
            conflict_stages: Default::default(),
            merge_message: None,
            blames: Default::default(),
            blames_at_revision: Default::default(),
            current_branch_name: Default::default(),
            branches: Default::default(),
            simulated_index_write_error_message: Default::default(),
            simulated_create_worktree_error: Default::default(),
            simulated_graph_error: None,
            branches_requiring_force_delete: Default::default(),
            worktrees_requiring_force_delete: Default::default(),
            refs: HashMap::from_iter([("HEAD".into(), "abc".into())]),
            merge_base_contents: Default::default(),
            merge_base_commits: Default::default(),
            oids: Default::default(),
            remotes: HashMap::default(),
            graph_commits: Vec::new(),
            commit_data: Default::default(),
            commit_history: Vec::new(),
            stash_entries: Default::default(),
            commit_template: None,
            tags: Vec::new(),
            upstreams: Default::default(),
            recent_branches: Vec::new(),
            ref_histories: Default::default(),
            merge_conflicts: Default::default(),
            rebase_conflicts: Default::default(),
            rebase_session: None,
            simulated_failures: Default::default(),
            pushed_tags: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FakeConflictStages {
    pub base: Option<Vec<u8>>,
    pub ours: Option<Vec<u8>>,
    pub theirs: Option<Vec<u8>>,
}

impl FakeConflictStages {
    pub fn stage(&self, stage: &str) -> Option<&Vec<u8>> {
        match stage {
            "1" => self.base.as_ref(),
            "2" => self.ours.as_ref(),
            "3" => self.theirs.as_ref(),
            _ => None,
        }
    }

    pub fn unmerged_status(&self) -> UnmergedStatus {
        let (first_head, second_head) = match (
            self.base.is_some(),
            self.ours.is_some(),
            self.theirs.is_some(),
        ) {
            (true, true, true) => (UnmergedStatusCode::Updated, UnmergedStatusCode::Updated),
            (false, true, true) => (UnmergedStatusCode::Added, UnmergedStatusCode::Added),
            (true, true, false) => (UnmergedStatusCode::Updated, UnmergedStatusCode::Deleted),
            (true, false, true) => (UnmergedStatusCode::Deleted, UnmergedStatusCode::Updated),
            (false, true, false) => (UnmergedStatusCode::Added, UnmergedStatusCode::Updated),
            (false, false, true) => (UnmergedStatusCode::Updated, UnmergedStatusCode::Added),
            (_, false, false) => (UnmergedStatusCode::Deleted, UnmergedStatusCode::Deleted),
        };
        UnmergedStatus {
            first_head,
            second_head,
        }
    }

    pub fn conflict_text(&self) -> Option<Vec<u8>> {
        if self.ours.is_none() && self.theirs.is_none() {
            return None;
        }
        let mut text = b"<<<<<<< HEAD\n".to_vec();
        text.extend_from_slice(self.ours.as_deref().unwrap_or_default());
        text.extend_from_slice(b"=======\n");
        text.extend_from_slice(self.theirs.as_deref().unwrap_or_default());
        text.extend_from_slice(b">>>>>>> MERGE_HEAD\n");
        Some(text)
    }
}

fn parse_stage_revision(revision: &str) -> Option<(&str, &str)> {
    let (stage, path) = revision.strip_prefix(':')?.split_once(':')?;
    matches!(stage, "1" | "2" | "3").then_some((stage, path))
}

fn stage_presence_for_status(status: UnmergedStatus) -> UnmergedStagePresence {
    let (base, ours, theirs) = match (status.first_head, status.second_head) {
        (UnmergedStatusCode::Updated, UnmergedStatusCode::Updated) => (true, true, true),
        (UnmergedStatusCode::Added, UnmergedStatusCode::Added) => (false, true, true),
        (UnmergedStatusCode::Updated, UnmergedStatusCode::Deleted) => (true, true, false),
        (UnmergedStatusCode::Deleted, UnmergedStatusCode::Updated) => (true, false, true),
        (UnmergedStatusCode::Added, UnmergedStatusCode::Updated)
        | (UnmergedStatusCode::Added, UnmergedStatusCode::Deleted) => (false, true, false),
        (UnmergedStatusCode::Updated, UnmergedStatusCode::Added)
        | (UnmergedStatusCode::Deleted, UnmergedStatusCode::Added) => (false, false, true),
        (UnmergedStatusCode::Deleted, UnmergedStatusCode::Deleted) => (true, false, false),
    };
    UnmergedStagePresence { base, ours, theirs }
}

fn unmerged_stage_presence(
    state: &FakeGitRepositoryState,
    path: &RepoPath,
) -> Option<UnmergedStagePresence> {
    let status = *state.unmerged_paths.get(path)?;
    Some(match state.conflict_stages.get(path) {
        Some(stages) => UnmergedStagePresence {
            base: stages.base.is_some(),
            ours: stages.ours.is_some(),
            theirs: stages.theirs.is_some(),
        },
        None => stage_presence_for_status(status),
    })
}

fn revision_bytes(state: &FakeGitRepositoryState, revision: &str) -> Option<Vec<u8>> {
    if let Some((stage, path)) = parse_stage_revision(revision) {
        let repo_path = RepoPath::new(path).ok()?;
        return state
            .conflict_stages
            .get(&repo_path)
            .and_then(|stages| stages.stage(stage))
            .cloned();
    }
    let (prefix, path) = revision.split_once(':')?;
    let repo_path = RepoPath::new(path).ok()?;
    match prefix {
        "" => state.index_contents.get(&repo_path).cloned(),
        "HEAD" => state.head_contents.get(&repo_path).cloned(),
        _ if state.refs.get("HEAD").map(String::as_str) == Some(prefix) => {
            state.head_contents.get(&repo_path).cloned()
        }
        _ => state
            .commit_history
            .iter()
            .find(|snapshot| snapshot.sha == prefix)
            .and_then(|snapshot| snapshot.head_contents.get(&repo_path).cloned()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FakeGitOperation {
    Checkout,
    CreateBranch,
    Merge,
    Rebase,
    ResetHard,
    DeleteRemoteBranch,
    FastForwardBranch,
    PushTag,
}

#[derive(Debug, Clone)]
pub struct FakeGitFailure {
    pub kind: GitFailureKind,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct FakeRebaseSession {
    pub original_branch: Option<String>,
    pub upstream: Option<String>,
    pub onto: Option<String>,
}

type ConflictFiles = Vec<(RepoPath, Option<Vec<u8>>)>;

const FAKE_BLOB_MODE: u32 = 0o100644;

fn git_failure(kind: GitFailureKind, message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(GitFailure {
        kind,
        message: message.into(),
    })
}

fn simulated_failure(state: &FakeGitRepositoryState, operation: FakeGitOperation) -> Result<()> {
    match state.simulated_failures.get(&operation) {
        Some(failure) => Err(git_failure(failure.kind.clone(), failure.message.clone())),
        None => Ok(()),
    }
}

fn ensure_no_unmerged_paths(state: &FakeGitRepositoryState, message: &str) -> Result<()> {
    if state.unmerged_paths.is_empty() {
        Ok(())
    } else {
        Err(git_failure(GitFailureKind::UnmergedFiles, message))
    }
}

fn unmatched_pathspec(path: &RepoPath) -> anyhow::Error {
    git_failure(
        GitFailureKind::Other,
        format!(
            "error: pathspec '{}' did not match any file(s) known to git",
            path.as_unix_str()
        ),
    )
}

fn is_tracked(state: &FakeGitRepositoryState, path: &RepoPath) -> bool {
    state.unmerged_paths.contains_key(path)
        || state.index_contents.contains_key(path)
        || state.head_contents.contains_key(path)
}

fn conflict_side_content(
    state: &FakeGitRepositoryState,
    path: &RepoPath,
    side: ConflictSide,
) -> Result<Option<Vec<u8>>> {
    if state.unmerged_paths.contains_key(path) {
        let stages = state
            .conflict_stages
            .get(path)
            .with_context(|| format!("{path:?} has no recorded conflict"))?;
        let (content, version) = match side {
            ConflictSide::Ours => (stages.ours.clone(), "our"),
            ConflictSide::Theirs => (stages.theirs.clone(), "their"),
        };
        return match content {
            Some(content) => Ok(Some(content)),
            None => Err(git_failure(
                GitFailureKind::Other,
                format!(
                    "error: path '{}' does not have {version} version",
                    path.as_unix_str()
                ),
            )),
        };
    }
    if is_tracked(state, path) {
        Ok(None)
    } else {
        Err(unmatched_pathspec(path))
    }
}

pub(crate) fn strip_ref_prefix(revision: &str) -> &str {
    revision
        .strip_prefix("refs/heads/")
        .or_else(|| revision.strip_prefix("refs/remotes/"))
        .or_else(|| revision.strip_prefix("refs/tags/"))
        .unwrap_or(revision)
}

pub(crate) fn full_ref_name(branch_key: &str) -> String {
    if branch_key.starts_with("refs/") {
        branch_key.to_string()
    } else if branch_key.contains('/') {
        format!("refs/remotes/{branch_key}")
    } else {
        format!("refs/heads/{branch_key}")
    }
}

fn local_branch_storage_key(name: &str) -> String {
    if name.contains('/') {
        format!("refs/heads/{name}")
    } else {
        name.to_string()
    }
}

fn local_branch_key(state: &FakeGitRepositoryState, name: &str) -> Option<String> {
    let full = format!("refs/heads/{name}");
    if state.branches.contains(&full) {
        return Some(full);
    }
    (!name.contains('/') && !name.starts_with("refs/") && state.branches.contains(name))
        .then(|| name.to_string())
}

fn remote_branch_key(state: &FakeGitRepositoryState, name: &str) -> Option<String> {
    let full = format!("refs/remotes/{name}");
    if state.branches.contains(&full) {
        return Some(full);
    }
    (name.contains('/') && !name.starts_with("refs/") && state.branches.contains(name))
        .then(|| name.to_string())
}

fn tag_sha(state: &FakeGitRepositoryState, name: &str) -> Option<String> {
    state
        .tags
        .iter()
        .find(|tag| &*tag.name == name)
        .map(|tag| tag.commit_sha.to_string())
}

fn local_branch_tip(state: &FakeGitRepositoryState, name: &str) -> String {
    if state.current_branch_name.as_deref() == Some(name)
        && let Some(sha) = state.refs.get("HEAD")
    {
        return sha.clone();
    }
    let full = format!("refs/heads/{name}");
    state
        .refs
        .get(&full)
        .cloned()
        .unwrap_or_else(|| format!("fake-tip-{full}"))
}

fn remote_branch_tip(state: &FakeGitRepositoryState, name: &str) -> String {
    let full = format!("refs/remotes/{name}");
    state
        .refs
        .get(&full)
        .cloned()
        .unwrap_or_else(|| format!("fake-tip-{full}"))
}

fn peel_revision(revision: &str) -> &str {
    revision
        .strip_suffix("^{commit}")
        .or_else(|| revision.strip_suffix("^0"))
        .unwrap_or(revision)
}

fn revision_sha(state: &FakeGitRepositoryState, revision: &str) -> Option<String> {
    let revision = peel_revision(revision);
    if let Some(sha) = state.refs.get(revision) {
        return Some(sha.clone());
    }
    if let Some(name) = revision.strip_prefix("refs/heads/") {
        return local_branch_key(state, name).map(|_| local_branch_tip(state, name));
    }
    if let Some(name) = revision.strip_prefix("refs/remotes/") {
        return remote_branch_key(state, name).map(|_| remote_branch_tip(state, name));
    }
    if let Some(name) = revision.strip_prefix("refs/tags/") {
        return tag_sha(state, name);
    }
    if let Some(sha) = tag_sha(state, revision) {
        return Some(sha);
    }
    if local_branch_key(state, revision).is_some() {
        return Some(local_branch_tip(state, revision));
    }
    if remote_branch_key(state, revision).is_some() {
        return Some(remote_branch_tip(state, revision));
    }
    let is_known_commit = state
        .commit_history
        .iter()
        .any(|snapshot| snapshot.sha == revision)
        || state
            .graph_commits
            .iter()
            .any(|commit| commit.sha.to_string() == revision)
        || state
            .ref_histories
            .values()
            .flatten()
            .any(|commit| &*commit.sha == revision);
    is_known_commit.then(|| revision.to_string())
}

fn current_history_key(state: &FakeGitRepositoryState) -> String {
    state
        .current_branch_name
        .clone()
        .unwrap_or_else(|| "HEAD".to_string())
}

fn ref_history(state: &FakeGitRepositoryState, revision: &str) -> Vec<CommitSummary> {
    let revision = peel_revision(revision);
    let key = if revision == "HEAD" {
        current_history_key(state)
    } else {
        strip_ref_prefix(revision).to_string()
    };
    state.ref_histories.get(&key).cloned().unwrap_or_default()
}

fn set_current_history(state: &mut FakeGitRepositoryState, history: Vec<CommitSummary>) {
    let key = current_history_key(state);
    state.ref_histories.insert(key, history);
}

fn set_current_tip(state: &mut FakeGitRepositoryState, sha: String) {
    if let Some(branch) = state.current_branch_name.clone() {
        state
            .refs
            .insert(format!("refs/heads/{branch}"), sha.clone());
    }
    state.refs.insert("HEAD".into(), sha);
}

fn short_sha(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

fn note_checkout(state: &mut FakeGitRepositoryState, destination: Option<&str>) {
    if let Some(previous) = state.current_branch_name.clone()
        && destination != Some(previous.as_str())
        && let Some(head) = state.refs.get("HEAD").cloned()
    {
        state.refs.insert(format!("refs/heads/{previous}"), head);
    }
    if let Some(destination) = destination {
        state.recent_branches.retain(|name| name != destination);
        state.recent_branches.insert(0, destination.to_string());
    }
}

fn clear_merge_markers(state: &mut FakeGitRepositoryState) {
    for marker in ["MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"] {
        state.refs.remove(marker);
    }
    state.merge_message = None;
}

fn switch_to_branch(state: &mut FakeGitRepositoryState, name: &str) {
    let tip = local_branch_tip(state, name);
    note_checkout(state, Some(name));
    state.current_branch_name = Some(name.to_string());
    state.refs.insert("HEAD".into(), tip);
    clear_merge_markers(state);
}

fn detach_head_at(state: &mut FakeGitRepositoryState, revision: &str, sha: String) {
    let history = ref_history(state, revision);
    note_checkout(state, None);
    state.current_branch_name = None;
    state.ref_histories.insert("HEAD".to_string(), history);
    state.refs.insert("HEAD".into(), sha);
    clear_merge_markers(state);
}

fn forget_branch(state: &mut FakeGitRepositoryState, key: &str) {
    let full = full_ref_name(key);
    let short = strip_ref_prefix(&full).to_string();
    state.refs.remove(&full);
    state.ref_histories.remove(&short);
    if full.starts_with("refs/remotes/") {
        for upstream in state.upstreams.values_mut() {
            if &*upstream.ref_name == full.as_str() {
                upstream.tracking = UpstreamTracking::Gone;
            }
        }
    } else {
        state.upstreams.remove(&short);
        state.recent_branches.retain(|name| *name != short);
    }
}

fn apply_conflicts(
    state: &mut FakeGitRepositoryState,
    conflicts: Vec<(RepoPath, FakeConflictStages)>,
) -> ConflictFiles {
    let mut files = Vec::with_capacity(conflicts.len());
    for (path, stages) in conflicts {
        state
            .unmerged_paths
            .insert(path.clone(), stages.unmerged_status());
        state.index_contents.remove(&path);
        match &stages.ours {
            Some(ours) => state.head_contents.insert(path.clone(), ours.clone()),
            None => state.head_contents.remove(&path),
        };
        files.push((path.clone(), stages.conflict_text()));
        state.conflict_stages.insert(path, stages);
    }
    files
}

fn conflict_summary(paths: &[RepoPath], trailer: &str) -> String {
    let mut output = String::new();
    for path in paths {
        let path = path.as_unix_str();
        output.push_str(&format!(
            "Auto-merging {path}\nCONFLICT (content): Merge conflict in {path}\n"
        ));
    }
    output.push_str(trailer);
    output
}

fn merge_subject(state: &FakeGitRepositoryState, reference: &str) -> String {
    let name = strip_ref_prefix(reference);
    let is_remote_tracking = local_branch_key(state, name).is_none()
        && tag_sha(state, name).is_none()
        && remote_branch_key(state, name).is_some();
    if is_remote_tracking {
        format!("Merge remote-tracking branch '{name}'")
    } else {
        format!("Merge branch '{name}'")
    }
}

fn rewind_commit_history(
    state: &mut FakeGitRepositoryState,
    commit: &str,
) -> Result<FakeCommitSnapshot> {
    let pop_count = if commit == "HEAD~" || commit == "HEAD^" {
        1
    } else if let Some(suffix) = commit.strip_prefix("HEAD~") {
        suffix
            .parse::<usize>()
            .with_context(|| format!("Invalid HEAD~ offset: {commit}"))?
    } else {
        match state
            .commit_history
            .iter()
            .rposition(|entry| entry.sha == commit)
        {
            Some(index) => state.commit_history.len() - index,
            None => anyhow::bail!("Unknown commit ref: {commit}"),
        }
    };

    if pop_count == 0 || pop_count > state.commit_history.len() {
        anyhow::bail!(
            "Cannot reset {pop_count} commit(s): only {} in history",
            state.commit_history.len()
        );
    }

    let target_index = state.commit_history.len() - pop_count;
    let snapshot = state.commit_history[target_index].clone();
    state.commit_history.truncate(target_index);
    Ok(snapshot)
}

fn fake_merge(
    state: &mut FakeGitRepositoryState,
    reference: &str,
) -> Result<(MergeOutcome, ConflictFiles)> {
    if state.refs.contains_key("MERGE_HEAD") {
        return Err(git_failure(
            GitFailureKind::UnmergedFiles,
            "fatal: You have not concluded your merge (MERGE_HEAD exists).\nPlease, commit your changes before you merge.",
        ));
    }
    ensure_no_unmerged_paths(
        state,
        "error: Merging is not possible because you have unmerged files.\nhint: Fix them up in the work tree, and then use 'git add/rm <file>'\nhint: as appropriate to mark resolution and make a commit.\nfatal: Exiting because of an unresolved conflict.",
    )?;
    let Some(target_sha) = revision_sha(state, reference) else {
        return Err(git_failure(
            GitFailureKind::Other,
            format!("merge: {reference} - not something we can merge"),
        ));
    };
    simulated_failure(state, FakeGitOperation::Merge)?;

    if let Some(conflicts) = state
        .merge_conflicts
        .get(strip_ref_prefix(reference))
        .cloned()
    {
        let paths = conflicts
            .iter()
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        let subject = merge_subject(state, reference);
        let files = apply_conflicts(state, conflicts);
        state.refs.insert("MERGE_HEAD".into(), target_sha);
        let conflict_lines = paths
            .iter()
            .map(|path| format!("#\t{}\n", path.as_unix_str()))
            .collect::<String>();
        state.merge_message = Some(format!("{subject}\n\n# Conflicts:\n{conflict_lines}"));
        let output = conflict_summary(
            &paths,
            "Automatic merge failed; fix conflicts and then commit the result.\n",
        );
        return Ok((MergeOutcome::Conflicted { output }, files));
    }

    let head_sha = state.refs.get("HEAD").cloned().unwrap_or_default();
    let head_history = ref_history(state, "HEAD");
    let target_history = ref_history(state, reference);
    let head_ids = head_history
        .iter()
        .map(|commit| commit.sha.clone())
        .collect::<HashSet<SharedString>>();
    let missing = target_history
        .iter()
        .filter(|commit| !head_ids.contains(&commit.sha))
        .cloned()
        .collect::<Vec<_>>();
    let already_up_to_date = if target_history.is_empty() {
        target_sha == head_sha
    } else {
        missing.is_empty()
    };
    if already_up_to_date {
        return Ok((MergeOutcome::AlreadyUpToDate, Vec::new()));
    }

    let target_ids = target_history
        .iter()
        .map(|commit| commit.sha.clone())
        .collect::<HashSet<SharedString>>();
    let is_fast_forward = head_history
        .iter()
        .all(|commit| target_ids.contains(&commit.sha));
    if is_fast_forward {
        let output = format!(
            "Updating {}..{}\nFast-forward\n",
            short_sha(&head_sha),
            short_sha(&target_sha)
        );
        set_current_history(state, target_history);
        set_current_tip(state, target_sha);
        return Ok((MergeOutcome::Merged { output }, Vec::new()));
    }

    let merge_commit = CommitSummary {
        sha: format!(
            "fake-merge-{}-{}",
            strip_ref_prefix(reference),
            head_history.len()
        )
        .into(),
        subject: merge_subject(state, reference).into(),
        commit_timestamp: head_history
            .first()
            .map_or(0, |commit| commit.commit_timestamp),
        author_name: SharedString::default(),
        has_parent: true,
    };
    let merge_sha = merge_commit.sha.to_string();
    let mut merged_history = vec![merge_commit];
    merged_history.extend(missing);
    merged_history.extend(head_history);
    set_current_history(state, merged_history);
    set_current_tip(state, merge_sha);
    Ok((
        MergeOutcome::Merged {
            output: "Merge made by the 'ort' strategy.\n".to_string(),
        },
        Vec::new(),
    ))
}

fn complete_rebase(state: &mut FakeGitRepositoryState, session: &FakeRebaseSession) -> String {
    if let Some(branch) = &session.original_branch {
        state.current_branch_name = Some(branch.clone());
    }
    state.rebase_session = None;
    state.refs.remove("REBASE_HEAD");
    let target = match &session.original_branch {
        Some(branch) => format!("refs/heads/{branch}"),
        None => "detached HEAD".to_string(),
    };
    let Some(upstream) = &session.upstream else {
        return format!("Successfully rebased and updated {target}.\n");
    };

    let head_history = ref_history(state, "HEAD");
    let upstream_history = ref_history(state, upstream);
    let head_ids = head_history
        .iter()
        .map(|commit| commit.sha.clone())
        .collect::<HashSet<SharedString>>();
    let upstream_ids = upstream_history
        .iter()
        .map(|commit| commit.sha.clone())
        .collect::<HashSet<SharedString>>();
    let is_up_to_date = if upstream_history.is_empty() {
        revision_sha(state, upstream).as_deref() == state.refs.get("HEAD").map(String::as_str)
    } else {
        upstream_history
            .iter()
            .all(|commit| head_ids.contains(&commit.sha))
    };
    if is_up_to_date {
        return match &session.original_branch {
            Some(branch) => format!("Current branch {branch} is up to date.\n"),
            None => "HEAD is up to date.\n".to_string(),
        };
    }

    let replayed = head_history
        .into_iter()
        .filter(|commit| !upstream_ids.contains(&commit.sha))
        .collect::<Vec<_>>();
    let tip = replayed
        .first()
        .map(|commit| commit.sha.to_string())
        .or_else(|| revision_sha(state, upstream));
    let mut rebased_history = replayed;
    rebased_history.extend(upstream_history);
    set_current_history(state, rebased_history);
    if let Some(tip) = tip {
        set_current_tip(state, tip);
    }
    format!("Successfully rebased and updated {target}.\n")
}

fn continue_rebase(
    state: &mut FakeGitRepositoryState,
    require_resolved_paths: bool,
) -> Result<(RebaseOutcome, ConflictFiles)> {
    let Some(session) = state.rebase_session.clone() else {
        return Err(git_failure(
            GitFailureKind::Other,
            "fatal: No rebase in progress?",
        ));
    };
    if require_resolved_paths {
        ensure_no_unmerged_paths(
            state,
            "error: Committing is not possible because you have unmerged files.\nhint: Fix them up in the work tree, and then use 'git add/rm <file>'\nhint: as appropriate to mark resolution and make a commit.\nfatal: Exiting because of an unresolved conflict.",
        )?;
    }
    simulated_failure(state, FakeGitOperation::Rebase)?;
    state.unmerged_paths.clear();
    state.conflict_stages.clear();
    let output = complete_rebase(state, &session);
    Ok((RebaseOutcome::Completed { output }, Vec::new()))
}

fn fake_rebase(
    state: &mut FakeGitRepositoryState,
    action: RebaseAction,
) -> Result<(RebaseOutcome, ConflictFiles)> {
    match action {
        RebaseAction::Start { upstream, branch } => {
            let upstream = if upstream == "HEAD" {
                state.current_branch_name.clone().unwrap_or(upstream)
            } else {
                upstream
            };
            if state.rebase_session.is_some() {
                return Err(git_failure(
                    GitFailureKind::Other,
                    "fatal: It seems that there is already a rebase-merge directory, and\nI wonder if you are in the middle of another rebase.",
                ));
            }
            ensure_no_unmerged_paths(
                state,
                "error: Rebasing is not possible because you have unmerged files.",
            )?;
            let Some(upstream_sha) = revision_sha(state, &upstream) else {
                return Err(git_failure(
                    GitFailureKind::Other,
                    format!("fatal: invalid upstream '{upstream}'"),
                ));
            };
            if let Some(branch) = &branch
                && local_branch_key(state, branch).is_none()
            {
                return Err(git_failure(
                    GitFailureKind::Other,
                    format!("fatal: invalid reference: {branch}"),
                ));
            }
            simulated_failure(state, FakeGitOperation::Rebase)?;
            if let Some(branch) = &branch {
                switch_to_branch(state, branch);
            }
            let session = FakeRebaseSession {
                original_branch: state.current_branch_name.clone(),
                upstream: Some(upstream.clone()),
                onto: Some(upstream_sha.clone()),
            };
            let Some(conflicts) = state
                .rebase_conflicts
                .get(strip_ref_prefix(&upstream))
                .cloned()
            else {
                let output = complete_rebase(state, &session);
                return Ok((RebaseOutcome::Completed { output }, Vec::new()));
            };
            let paths = conflicts
                .iter()
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>();
            let files = apply_conflicts(state, conflicts);
            let history = ref_history(state, "HEAD");
            state.ref_histories.insert("HEAD".to_string(), history);
            state.current_branch_name = None;
            let rebase_head = state.refs.get("HEAD").cloned().unwrap_or(upstream_sha);
            state.refs.insert("REBASE_HEAD".into(), rebase_head.clone());
            state.rebase_session = Some(session);
            let trailer = format!(
                "error: could not apply {}\nhint: Resolve all conflicts manually, mark them as resolved with\nhint: \"git add/rm <conflicted_files>\", then run \"git rebase --continue\".\nhint: You can instead skip this commit: run \"git rebase --skip\".\nhint: To abort and get back to the state before \"git rebase\", run \"git rebase --abort\".\n",
                short_sha(&rebase_head)
            );
            let output = conflict_summary(&paths, &trailer);
            Ok((RebaseOutcome::Conflicted { output }, files))
        }
        RebaseAction::Continue => continue_rebase(state, true),
        RebaseAction::Skip => continue_rebase(state, false),
        RebaseAction::Abort => {
            let Some(session) = state.rebase_session.take() else {
                return Err(git_failure(
                    GitFailureKind::Other,
                    "fatal: No rebase in progress?",
                ));
            };
            if let Some(branch) = session.original_branch {
                state.current_branch_name = Some(branch);
            }
            state.unmerged_paths.clear();
            state.conflict_stages.clear();
            state.refs.remove("REBASE_HEAD");
            Ok((RebaseOutcome::Aborted, Vec::new()))
        }
    }
}

#[derive(Clone, Default, Debug)]
pub struct FakeBlobReadGate(Arc<Mutex<BlobReadGateState>>);

#[derive(Default, Debug)]
struct BlobReadGateState {
    open: bool,
    peak: usize,
    next_id: u64,
    waiters: Vec<Waiter>,
}

#[derive(Debug)]
struct Waiter {
    id: u64,
    oid: Oid,
    sender: oneshot::Sender<()>,
}

impl FakeBlobReadGate {
    async fn wait(&self, oid: Oid) {
        let (_guard, receiver) = {
            let mut inner = self.0.lock();
            if inner.open {
                return;
            }
            let id = inner.next_id;
            inner.next_id += 1;
            let (sender, receiver) = oneshot::channel();
            inner.waiters.push(Waiter { id, oid, sender });
            inner.peak = inner.peak.max(inner.waiters.len());
            (
                WaiterGuard {
                    state: self.0.clone(),
                    id,
                },
                receiver,
            )
        };
        receiver.await.ok();
    }

    pub fn peak_concurrent(&self) -> usize {
        self.0.lock().peak
    }

    pub fn waiting(&self) -> usize {
        self.0.lock().waiters.len()
    }

    pub fn is_waiting(&self, oid: Oid) -> bool {
        self.0.lock().waiters.iter().any(|waiter| waiter.oid == oid)
    }

    pub fn release(&self, oid: Oid) -> bool {
        let mut inner = self.0.lock();
        let Some(position) = inner.waiters.iter().position(|waiter| waiter.oid == oid) else {
            return false;
        };
        let waiter = inner.waiters.remove(position);
        drop(inner);
        waiter.sender.send(()).ok();
        true
    }

    pub fn open(&self) {
        let waiters = {
            let mut inner = self.0.lock();
            inner.open = true;
            std::mem::take(&mut inner.waiters)
        };
        for waiter in waiters {
            waiter.sender.send(()).ok();
        }
    }
}

struct WaiterGuard {
    state: Arc<Mutex<BlobReadGateState>>,
    id: u64,
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        self.state
            .lock()
            .waiters
            .retain(|waiter| waiter.id != self.id);
    }
}

impl FakeGitRepository {
    fn with_state_async<F, T>(&self, write: bool, f: F) -> BoxFuture<'static, Result<T>>
    where
        F: 'static + Send + FnOnce(&mut FakeGitRepositoryState) -> Result<T>,
        T: Send,
    {
        let fs = self.fs.clone();
        let executor = self.executor.clone();
        let dot_git_path = self.dot_git_path.clone();
        async move {
            executor.simulate_random_delay().await;
            fs.with_git_state(&dot_git_path, write, f)?
        }
        .boxed()
    }

    fn edit_ref(&self, edit: RefEdit) -> BoxFuture<'_, Result<()>> {
        self.with_state_async(true, move |state| {
            match edit {
                RefEdit::Update { ref_name, commit } => {
                    state.refs.insert(ref_name, commit);
                }
                RefEdit::Delete { ref_name } => {
                    state.refs.remove(&ref_name);
                }
            }
            Ok(())
        })
    }

    async fn write_conflict_files(&self, files: ConflictFiles) -> Result<()> {
        let working_directory = self
            .dot_git_path
            .parent()
            .context("repository has no working directory")?
            .to_path_buf();
        for (path, conflict_text) in files {
            let absolute_path = working_directory.join(path.as_std_path());
            match conflict_text {
                Some(conflict_text) => {
                    self.fs
                        .write_file_internal(&absolute_path, conflict_text, false)?
                }
                None => {
                    self.fs
                        .remove_file(
                            &absolute_path,
                            RemoveOptions {
                                recursive: false,
                                ignore_if_not_exists: true,
                            },
                        )
                        .await?
                }
            }
        }
        Ok(())
    }

    /// Scans `.git/worktrees/*/gitdir` to find the admin entry directory for a
    /// worktree at the given checkout path. Used when the working tree directory
    /// has already been deleted and we can't read its `.git` pointer file.
    async fn find_worktree_entry_dir_by_path(&self, path: &Path) -> Option<PathBuf> {
        use futures::StreamExt;

        let worktrees_dir = self.common_dir_path.join("worktrees");
        let mut entries = self.fs.read_dir(&worktrees_dir).await.ok()?;
        while let Some(Ok(entry_path)) = entries.next().await {
            if let Ok(gitdir_content) = self.fs.load(&entry_path.join("gitdir")).await {
                let worktree_path = PathBuf::from(gitdir_content.trim())
                    .parent()
                    .map(PathBuf::from)
                    .unwrap_or_default();
                if worktree_path == path {
                    return Some(entry_path);
                }
            }
        }
        None
    }
}

impl GitRepository for FakeGitRepository {
    fn load_commit_template(&self) -> BoxFuture<'_, Result<Option<GitCommitTemplate>>> {
        self.with_state_async(false, |state| Ok(state.commit_template.clone()))
    }

    fn load_blob_content(&self, oid: git::Oid) -> BoxFuture<'_, Result<Vec<u8>>> {
        let content_and_gate = self.with_state_async(false, move |state| {
            Ok((state.oids.get(&oid).cloned(), state.blob_read_gate.clone()))
        });
        async move {
            let (content, gate) = content_and_gate.await?;
            if let Some(gate) = gate {
                gate.wait(oid).await;
            }
            content.context("oid does not exist")
        }
        .boxed()
    }

    fn load_commit(
        &self,
        _commit: String,
        _ignore_shallow_boundary: bool,
        _cx: AsyncApp,
    ) -> BoxFuture<'_, Result<git::repository::CommitDiff>> {
        async {
            Ok(git::repository::CommitDiff {
                files: Vec::new(),
                is_shallow_boundary: false,
            })
        }
        .boxed()
    }

    fn set_index_text(
        &self,
        path: RepoPath,
        content: Option<Vec<u8>>,
        _env: Arc<HashMap<String, String>>,
        _is_executable: bool,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        self.with_state_async(true, move |state| {
            if let Some(message) = &state.simulated_index_write_error_message {
                anyhow::bail!("{message}");
            } else if let Some(content) = content {
                state.index_contents.insert(path, content);
            } else {
                state.index_contents.remove(&path);
            }
            Ok(())
        })
    }

    fn remote_urls(&self) -> BoxFuture<'_, HashMap<String, String>> {
        let fut = self.with_state_async(false, |state| Ok(state.remotes.clone()));
        async move { fut.await.unwrap_or_default() }.boxed()
    }

    fn diff_tree(&self, request: DiffTreeType) -> BoxFuture<'_, Result<TreeDiff>> {
        let worktree_contents =
            matches!(request, DiffTreeType::MergeBaseWithWorktree { .. }).then(|| {
                let workdir_path = self.dot_git_path.parent().unwrap();
                self.fs
                    .files()
                    .iter()
                    .filter_map(|path| {
                        let path_in_repo = path.strip_prefix(workdir_path).ok()?;
                        let path_in_repo = RelPath::new(path_in_repo, PathStyle::local()).ok()?;
                        let content = self.fs.read_file_sync(path).ok()?;
                        Some((RepoPath::from_rel_path(&path_in_repo), content))
                    })
                    .collect::<HashMap<_, _>>()
            });
        self.with_state_async(false, move |state| {
            let contents = worktree_contents.as_ref().unwrap_or(&state.head_contents);
            let tracked_paths = state
                .merge_base_contents
                .keys()
                .chain(state.head_contents.keys())
                .chain(state.index_contents.keys())
                .collect::<HashSet<_>>();
            let mut entries = HashMap::default();
            for path in tracked_paths {
                let status = match (state.merge_base_contents.get(path), contents.get(path)) {
                    (Some(oid), Some(content)) => {
                        if state.oids.get(oid).context("merge-base blob is missing")? == content {
                            continue;
                        }
                        TreeDiffStatus::Modified { old: *oid }
                    }
                    (Some(oid), None) => TreeDiffStatus::Deleted { old: *oid },
                    (None, Some(_)) => TreeDiffStatus::Added,
                    (None, None) => continue,
                };
                entries.insert(path.clone(), status);
            }
            Ok(TreeDiff { entries })
        })
        .boxed()
    }

    fn revparse_batch(&self, revs: Vec<String>) -> BoxFuture<'_, Result<Vec<Option<String>>>> {
        self.with_state_async(false, |state| {
            Ok(revs
                .into_iter()
                .map(|rev| state.refs.get(&rev).cloned())
                .collect())
        })
    }

    fn load_revisions(
        &self,
        revisions: Vec<String>,
    ) -> BoxFuture<'_, Result<Vec<Option<Vec<u8>>>>> {
        let fut = self.with_state_async(false, move |state| {
            Ok(revisions
                .iter()
                .map(|revision| revision_bytes(state, revision))
                .collect())
        });
        self.executor.spawn(fut).boxed()
    }

    fn load_revisions_filtered(
        &self,
        revisions: Vec<String>,
    ) -> BoxFuture<'_, Result<Vec<Option<Vec<u8>>>>> {
        self.load_revisions(revisions)
    }

    fn unmerged_entries(&self) -> BoxFuture<'_, Result<Vec<UnmergedEntry>>> {
        self.with_state_async(false, |state| {
            let mut entries = state
                .unmerged_paths
                .keys()
                .filter_map(|path| {
                    let stages = unmerged_stage_presence(state, path)?;
                    Some(UnmergedEntry {
                        path: path.clone(),
                        stages,
                        base_oid: None,
                        ours_oid: None,
                        theirs_oid: None,
                        ours_mode: stages.ours.then_some(FAKE_BLOB_MODE),
                        theirs_mode: stages.theirs.then_some(FAKE_BLOB_MODE),
                    })
                })
                .collect::<Vec<_>>();
            entries.sort_by(|left, right| left.path.cmp(&right.path));
            Ok(entries)
        })
    }

    fn has_unmerged_paths(&self) -> BoxFuture<'_, Result<bool>> {
        self.with_state_async(false, |state| Ok(!state.unmerged_paths.is_empty()))
    }

    fn merge_base(&self, first: String, second: String) -> BoxFuture<'_, Result<Option<String>>> {
        self.with_state_async(false, move |state| {
            anyhow::ensure!(
                !first.starts_with('-') && !second.starts_with('-'),
                "merge-base revisions must not start with a dash"
            );
            let planned = state
                .merge_base_commits
                .get(&(first.clone(), second.clone()))
                .or_else(|| {
                    state
                        .merge_base_commits
                        .get(&(second.clone(), first.clone()))
                })
                .cloned();
            if planned.is_some() {
                return Ok(planned);
            }
            let first_sha = revision_sha(state, &first);
            let second_sha = revision_sha(state, &second);
            Ok(first_sha.filter(|sha| Some(sha) == second_sha.as_ref()))
        })
    }

    fn rebase_onto(&self) -> BoxFuture<'_, Option<String>> {
        let onto = self.with_state_async(false, |state| {
            Ok(state
                .rebase_session
                .as_ref()
                .and_then(|session| session.onto.clone()))
        });
        async move { onto.await.ok().flatten() }.boxed()
    }

    fn rebase_current_commit(&self) -> BoxFuture<'_, Option<String>> {
        let current_commit = self.with_state_async(false, |state| {
            Ok(state
                .rebase_session
                .as_ref()
                .and_then(|_| state.refs.get("REBASE_HEAD").cloned()))
        });
        async move { current_commit.await.ok().flatten() }.boxed()
    }

    fn show(&self, commit: String) -> BoxFuture<'_, Result<CommitDetails>> {
        self.with_state_async(false, move |state| {
            let sha = match state.refs.get(&commit) {
                Some(sha) => sha.clone(),
                // Real git fails to show an unresolvable revision (e.g. HEAD on an
                // unborn branch), so only fall back to treating the input as a sha.
                None => {
                    anyhow::ensure!(
                        commit.parse::<Oid>().is_ok(),
                        "unable to resolve revision: {commit}"
                    );
                    commit
                }
            };
            Ok(CommitDetails {
                sha: sha.into(),
                message: "initial commit".into(),
                ..Default::default()
            })
        })
    }

    fn reset(
        &self,
        commit: String,
        mode: ResetMode,
        _env: Arc<HashMap<String, String>>,
    ) -> BoxFuture<'_, Result<()>> {
        self.with_state_async(true, move |state| {
            let snapshot = rewind_commit_history(state, &commit)?;

            match mode {
                ResetMode::Soft => {
                    state.head_contents = snapshot.head_contents;
                }
                ResetMode::Mixed => {
                    state.head_contents = snapshot.head_contents;
                    state.index_contents = state.head_contents.clone();
                }
            }

            state.refs.insert("HEAD".into(), snapshot.sha);
            Ok(())
        })
    }

    fn checkout_files(
        &self,
        _commit: String,
        _paths: Vec<RepoPath>,
        _env: Arc<HashMap<String, String>>,
    ) -> BoxFuture<'_, Result<()>> {
        unimplemented!()
    }

    fn path(&self) -> PathBuf {
        self.repository_dir_path.clone()
    }

    fn main_repository_path(&self) -> PathBuf {
        self.common_dir_path.clone()
    }

    fn merge_message(&self) -> BoxFuture<'_, Option<String>> {
        let merge_message = self.with_state_async(false, |state| Ok(state.merge_message.clone()));
        async move { merge_message.await.ok().flatten() }.boxed()
    }

    fn status(&self, path_prefixes: &[RepoPath]) -> Task<Result<GitStatus>> {
        let workdir_path = self.dot_git_path.parent().unwrap();

        // Load gitignores
        let ignores = workdir_path
            .ancestors()
            .filter_map(|dir| {
                let ignore_path = dir.join(".gitignore");
                let content = self.fs.read_file_sync(ignore_path).ok()?;
                let content = String::from_utf8(content).ok()?;
                let mut builder = GitignoreBuilder::new(dir);
                for line in content.lines() {
                    builder.add_line(Some(dir.into()), line).ok()?;
                }
                builder.build().ok()
            })
            .collect::<Vec<_>>();

        // Load working copy files.
        let git_files: HashMap<RepoPath, (Vec<u8>, bool)> = self
            .fs
            .files()
            .iter()
            .filter_map(|path| {
                // TODO better simulate git status output in the case of submodules and worktrees
                let repo_path = path.strip_prefix(workdir_path).ok()?;
                let mut is_ignored = repo_path.starts_with(".git");
                for ignore in &ignores {
                    match ignore.matched_path_or_any_parents(path, false) {
                        ignore::Match::None => {}
                        ignore::Match::Ignore(_) => is_ignored = true,
                        ignore::Match::Whitelist(_) => break,
                    }
                }
                let content = self.fs.read_file_sync(path).ok()?;
                let repo_path = RelPath::new(repo_path, PathStyle::local()).ok()?;
                Some((RepoPath::from_rel_path(&repo_path), (content, is_ignored)))
            })
            .collect();

        let result = self.fs.with_git_state(&self.dot_git_path, false, |state| {
            let mut entries = Vec::new();
            let paths = state
                .head_contents
                .keys()
                .chain(state.index_contents.keys())
                .chain(state.unmerged_paths.keys())
                .chain(git_files.keys())
                .collect::<HashSet<_>>();
            for path in paths {
                if !path_prefixes.iter().any(|prefix| path.starts_with(prefix)) {
                    continue;
                }

                let head = state.head_contents.get(path);
                let index = state.index_contents.get(path);
                let unmerged = state.unmerged_paths.get(path);
                let fs = git_files.get(path);
                let status = match (unmerged, head, index, fs) {
                    (Some(unmerged), _, _, _) => FileStatus::Unmerged(*unmerged),
                    (_, Some(head), Some(index), Some((fs, _))) => {
                        FileStatus::Tracked(TrackedStatus {
                            index_status: if head == index {
                                StatusCode::Unmodified
                            } else {
                                StatusCode::Modified
                            },
                            worktree_status: if fs == index {
                                StatusCode::Unmodified
                            } else {
                                StatusCode::Modified
                            },
                        })
                    }
                    (_, Some(head), Some(index), None) => FileStatus::Tracked(TrackedStatus {
                        index_status: if head == index {
                            StatusCode::Unmodified
                        } else {
                            StatusCode::Modified
                        },
                        worktree_status: StatusCode::Deleted,
                    }),
                    (_, Some(_), None, Some(_)) => FileStatus::Tracked(TrackedStatus {
                        index_status: StatusCode::Deleted,
                        worktree_status: StatusCode::Added,
                    }),
                    (_, Some(_), None, None) => FileStatus::Tracked(TrackedStatus {
                        index_status: StatusCode::Deleted,
                        worktree_status: StatusCode::Deleted,
                    }),
                    (_, None, Some(index), Some((fs, _))) => FileStatus::Tracked(TrackedStatus {
                        index_status: StatusCode::Added,
                        worktree_status: if fs == index {
                            StatusCode::Unmodified
                        } else {
                            StatusCode::Modified
                        },
                    }),
                    (_, None, Some(_), None) => FileStatus::Tracked(TrackedStatus {
                        index_status: StatusCode::Added,
                        worktree_status: StatusCode::Deleted,
                    }),
                    (_, None, None, Some((_, is_ignored))) => {
                        if *is_ignored {
                            continue;
                        }
                        FileStatus::Untracked
                    }
                    (_, None, None, None) => {
                        unreachable!();
                    }
                };
                if status
                    != FileStatus::Tracked(TrackedStatus {
                        index_status: StatusCode::Unmodified,
                        worktree_status: StatusCode::Unmodified,
                    })
                {
                    entries.push((path.clone(), status));
                }
            }
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            anyhow::Ok(GitStatus {
                entries: entries.into(),
            })
        });
        Task::ready(match result {
            Ok(result) => result,
            Err(e) => Err(e),
        })
    }

    fn stash_entries(&self) -> BoxFuture<'static, Result<git::stash::GitStash>> {
        self.with_state_async(false, |state| Ok(state.stash_entries.clone()))
    }

    fn branches(&self) -> BoxFuture<'_, Result<git::repository::BranchesScanResult>> {
        self.with_state_async(false, move |state| {
            let current_branch = &state.current_branch_name;
            let mut branches = state
                .branches
                .iter()
                .map(|branch_name| {
                    let ref_name = if branch_name.starts_with("refs/") {
                        branch_name.into()
                    } else if branch_name.contains('/') {
                        format!("refs/remotes/{branch_name}").into()
                    } else {
                        format!("refs/heads/{branch_name}").into()
                    };
                    let local_name = branch_name
                        .strip_prefix("refs/heads/")
                        .or_else(|| (!branch_name.contains('/')).then_some(branch_name.as_str()));
                    let is_head = current_branch.as_deref().is_some_and(|current| {
                        local_name == Some(current) || branch_name == current
                    });
                    Branch {
                        is_head,
                        ref_name,
                        most_recent_commit: None,
                        upstream: local_name
                            .and_then(|local_name| state.upstreams.get(local_name))
                            .cloned(),
                    }
                })
                .collect::<Vec<_>>();
            // compute snapshot expects these to be sorted by ref_name
            // because that's what git itself does
            branches.sort_by(|a, b| a.ref_name.cmp(&b.ref_name));
            Ok(branches.into())
        })
    }

    fn worktrees(&self) -> BoxFuture<'_, Result<Vec<Worktree>>> {
        let fs = self.fs.clone();
        let common_dir_path = self.common_dir_path.clone();
        let executor = self.executor.clone();

        async move {
            executor.simulate_random_delay().await;

            let (main_worktree, refs) = fs.with_git_state(&common_dir_path, false, |state| {
                let work_dir = common_dir_path
                    .parent()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| common_dir_path.clone());
                let head_sha = state
                    .refs
                    .get("HEAD")
                    .cloned()
                    .unwrap_or_else(|| "0000000".to_string());
                let branch_ref = state
                    .current_branch_name
                    .as_ref()
                    .map(|name| format!("refs/heads/{name}"))
                    .unwrap_or_else(|| "refs/heads/main".to_string());
                let main_wt = Worktree {
                    path: work_dir,
                    ref_name: Some(branch_ref.into()),
                    sha: head_sha.into(),
                    is_main: true,
                    is_bare: false,
                };
                (main_wt, state.refs.clone())
            })?;

            let mut all = vec![main_worktree];

            let worktrees_dir = common_dir_path.join("worktrees");
            if let Ok(mut entries) = fs.read_dir(&worktrees_dir).await {
                use futures::StreamExt;
                while let Some(Ok(entry_path)) = entries.next().await {
                    let head_content = match fs.load(&entry_path.join("HEAD")).await {
                        Ok(content) => content,
                        Err(_) => continue,
                    };
                    let gitdir_content = match fs.load(&entry_path.join("gitdir")).await {
                        Ok(content) => content,
                        Err(_) => continue,
                    };

                    let ref_name = head_content
                        .strip_prefix("ref: ")
                        .map(|s| s.trim().to_string());
                    let sha = ref_name
                        .as_ref()
                        .and_then(|r| refs.get(r))
                        .cloned()
                        .unwrap_or_else(|| head_content.trim().to_string());

                    let worktree_path = PathBuf::from(gitdir_content.trim())
                        .parent()
                        .map(PathBuf::from)
                        .unwrap_or_default();

                    all.push(Worktree {
                        path: worktree_path,
                        ref_name: ref_name.map(Into::into),
                        sha: sha.into(),
                        is_main: false,
                        is_bare: false,
                    });
                }
            }

            Ok(all)
        }
        .boxed()
    }

    fn worktree_created_at(
        &self,
        worktree_path: PathBuf,
    ) -> BoxFuture<'_, Result<Option<SystemTime>>> {
        let fs = self.fs.clone();
        async move {
            if fs.metadata(&worktree_path).await?.is_none() {
                return Ok(None);
            }
            let dot_git_path = worktree_path.join(".git");
            let git_file = fs
                .load(&dot_git_path)
                .await
                .with_context(|| format!("failed to read {}", dot_git_path.display()))?;
            let git_dir = git_file
                .strip_prefix("gitdir:")
                .context("worktree .git file missing gitdir pointer")?
                .trim();
            let git_dir = worktree_path.join(git_dir);
            let metadata = fs
                .metadata(&git_dir)
                .await?
                .with_context(|| format!("missing worktree git dir {}", git_dir.display()))?;
            // FakeFs assigns directories a monotonically increasing mtime at
            // creation and never updates it afterwards, so a directory's
            // mtime doubles as its creation time.
            Ok(Some(metadata.mtime.0))
        }
        .boxed()
    }

    fn create_worktree(
        &self,
        target: CreateWorktreeTarget,
        path: PathBuf,
    ) -> BoxFuture<'_, Result<()>> {
        let fs = self.fs.clone();
        let executor = self.executor.clone();
        let dot_git_path = self.dot_git_path.clone();
        let common_dir_path = self.common_dir_path.clone();
        async move {
            executor.simulate_random_delay().await;

            let branch_name = target.branch_name().map(ToOwned::to_owned);
            let create_branch_ref = matches!(target, CreateWorktreeTarget::NewBranch { .. });

            // Check for simulated error and validate branch state before any side effects.
            fs.with_git_state(&dot_git_path, false, {
                let branch_name = branch_name.clone();
                move |state| {
                    if let Some(message) = &state.simulated_create_worktree_error {
                        anyhow::bail!("{message}");
                    }

                    match (create_branch_ref, branch_name.as_ref()) {
                        (true, Some(branch_name)) => {
                            if state.branches.contains(branch_name) {
                                bail!("a branch named '{}' already exists", branch_name);
                            }
                        }
                        (false, Some(branch_name)) => {
                            if !state.branches.contains(branch_name) {
                                bail!("no branch named '{}' exists", branch_name);
                            }
                        }
                        (false, None) => {}
                        (true, None) => bail!("branch name is required to create a branch"),
                    }

                    Ok(())
                }
            })??;

            let (branch_name, sha, create_branch_ref) = match target {
                CreateWorktreeTarget::ExistingBranch { branch_name } => {
                    let ref_name = format!("refs/heads/{branch_name}");
                    let sha = fs.with_git_state(&dot_git_path, false, {
                        move |state| {
                            Ok::<_, anyhow::Error>(
                                state
                                    .refs
                                    .get(&ref_name)
                                    .cloned()
                                    .unwrap_or_else(|| "fake-sha".to_string()),
                            )
                        }
                    })??;
                    (Some(branch_name), sha, false)
                }
                CreateWorktreeTarget::NewBranch {
                    branch_name,
                    base_sha: start_point,
                } => (
                    Some(branch_name),
                    start_point.unwrap_or_else(|| "fake-sha".to_string()),
                    true,
                ),
                CreateWorktreeTarget::Detached {
                    base_sha: start_point,
                } => (
                    None,
                    start_point.unwrap_or_else(|| "fake-sha".to_string()),
                    false,
                ),
            };

            // Create the worktree checkout directory.
            fs.create_dir(&path).await?;

            // Create .git/worktrees/<name>/ directory with HEAD, commondir, gitdir.
            let worktree_entry_name = branch_name.as_deref().unwrap_or_else(|| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("detached")
            });
            let worktrees_entry_dir = common_dir_path.join("worktrees").join(worktree_entry_name);
            fs.create_dir(&worktrees_entry_dir).await?;

            let head_content = if let Some(ref branch_name) = branch_name {
                let ref_name = format!("refs/heads/{branch_name}");
                format!("ref: {ref_name}")
            } else {
                sha.clone()
            };
            fs.write_file_internal(
                worktrees_entry_dir.join("HEAD"),
                head_content.into_bytes(),
                false,
            )?;
            fs.write_file_internal(
                worktrees_entry_dir.join("commondir"),
                common_dir_path.to_string_lossy().into_owned().into_bytes(),
                false,
            )?;
            let worktree_dot_git = path.join(".git");
            fs.write_file_internal(
                worktrees_entry_dir.join("gitdir"),
                worktree_dot_git.to_string_lossy().into_owned().into_bytes(),
                false,
            )?;

            // Create .git file in the worktree checkout.
            fs.write_file_internal(
                &worktree_dot_git,
                format!("gitdir: {}", worktrees_entry_dir.display()).into_bytes(),
                false,
            )?;

            // Update git state for newly created branches.
            if create_branch_ref {
                fs.with_git_state(&dot_git_path, true, {
                    let branch_name = branch_name.clone();
                    let sha = sha.clone();
                    move |state| {
                        if let Some(branch_name) = branch_name {
                            let ref_name = format!("refs/heads/{branch_name}");
                            state.refs.insert(ref_name, sha);
                            state.branches.insert(branch_name);
                        }
                        Ok::<(), anyhow::Error>(())
                    }
                })??;
            }

            Ok(())
        }
        .boxed()
    }

    fn remove_worktree(&self, path: PathBuf, force: bool) -> BoxFuture<'_, Result<()>> {
        let fs = self.fs.clone();
        let executor = self.executor.clone();
        let common_dir_path = self.common_dir_path.clone();
        async move {
            executor.simulate_random_delay().await;

            if !force {
                fs.with_git_state(&common_dir_path, false, |state| {
                    if state.worktrees_requiring_force_delete.contains(&path) {
                        bail!(
                            "fatal: '{}' contains modified or untracked files, use --force to delete it",
                            path.display()
                        );
                    }
                    Ok::<(), anyhow::Error>(())
                })??;
            }

            // Try to read the worktree's .git file to find its entry
            // directory. If the working tree is already gone (e.g. the
            // caller deleted it before asking git to clean up), fall back
            // to scanning `.git/worktrees/*/gitdir` for a matching path,
            // mirroring real git's behavior with `--force`.
            let dot_git_file = path.join(".git");
            let worktree_entry_dir = if let Ok(content) = fs.load(&dot_git_file).await {
                let gitdir = content
                    .strip_prefix("gitdir:")
                    .context("invalid .git file in worktree")?
                    .trim();
                PathBuf::from(gitdir)
            } else {
                self.find_worktree_entry_dir_by_path(&path)
                    .await
                    .with_context(|| format!("no worktree found at path: {}", path.display()))?
            };

            // Remove the worktree checkout directory if it still exists.
            fs.remove_dir(
                &path,
                RemoveOptions {
                    recursive: true,
                    ignore_if_not_exists: true,
                },
            )
            .await?;

            // Remove the .git/worktrees/<name>/ directory.
            fs.remove_dir(
                &worktree_entry_dir,
                RemoveOptions {
                    recursive: true,
                    ignore_if_not_exists: false,
                },
            )
            .await?;

            // Emit a git event on the main .git directory so the scanner
            // notices the change.
            fs.with_git_state(&common_dir_path, true, |state| {
                state.worktrees_requiring_force_delete.remove(&path);
                Ok::<(), anyhow::Error>(())
            })??;

            Ok(())
        }
        .boxed()
    }

    fn rename_worktree(&self, old_path: PathBuf, new_path: PathBuf) -> BoxFuture<'_, Result<()>> {
        let fs = self.fs.clone();
        let executor = self.executor.clone();
        let common_dir_path = self.common_dir_path.clone();
        async move {
            executor.simulate_random_delay().await;

            // Read the worktree's .git file to find its entry directory.
            let dot_git_file = old_path.join(".git");
            let content = fs
                .load(&dot_git_file)
                .await
                .with_context(|| format!("no worktree found at path: {}", old_path.display()))?;
            let gitdir = content
                .strip_prefix("gitdir:")
                .context("invalid .git file in worktree")?
                .trim();
            let worktree_entry_dir = PathBuf::from(gitdir);

            // Move the worktree checkout directory.
            fs.rename(
                &old_path,
                &new_path,
                RenameOptions {
                    overwrite: false,
                    ignore_if_exists: false,
                    create_parents: true,
                },
            )
            .await?;

            // Update the gitdir file in .git/worktrees/<name>/ to point to the
            // new location.
            let new_dot_git = new_path.join(".git");
            fs.write_file_internal(
                worktree_entry_dir.join("gitdir"),
                new_dot_git.to_string_lossy().into_owned().into_bytes(),
                false,
            )?;

            // Update the .git file in the moved worktree checkout.
            fs.write_file_internal(
                &new_dot_git,
                format!("gitdir: {}", worktree_entry_dir.display()).into_bytes(),
                false,
            )?;

            // Emit a git event on the main .git directory so the scanner
            // notices the change.
            fs.with_git_state(&common_dir_path, true, |_| {})?;

            Ok(())
        }
        .boxed()
    }

    fn checkout_branch_in_worktree(
        &self,
        _branch_name: String,
        _worktree_path: PathBuf,
        _create: bool,
    ) -> BoxFuture<'_, Result<()>> {
        async { Ok(()) }.boxed()
    }

    fn change_branch(&self, name: String) -> BoxFuture<'_, Result<()>> {
        self.with_state_async(true, |state| {
            simulated_failure(state, FakeGitOperation::Checkout)?;
            note_checkout(state, Some(&name));
            if let Some(tip) = state.refs.get(&format!("refs/heads/{name}")).cloned() {
                state.refs.insert("HEAD".into(), tip);
            }
            state.current_branch_name = Some(name);
            Ok(())
        })
    }

    fn create_branch(
        &self,
        name: String,
        _base_branch: Option<String>,
    ) -> BoxFuture<'_, Result<()>> {
        self.with_state_async(true, move |state| {
            if let Some((remote, _)) = name.split_once('/')
                && !state.remotes.contains_key(remote)
            {
                state.remotes.insert(remote.to_owned(), "".to_owned());
            }
            state.branches.insert(name);
            Ok(())
        })
    }

    fn rename_branch(&self, branch: String, new_name: String) -> BoxFuture<'_, Result<()>> {
        self.with_state_async(true, move |state| {
            let key = if state.branches.contains(&branch) {
                Some(branch.clone())
            } else {
                local_branch_key(state, &branch)
            };
            let Some(key) = key else {
                bail!("no such branch: {branch}");
            };
            state.branches.remove(&key);
            let new_key = if key.starts_with("refs/heads/") {
                local_branch_storage_key(&new_name)
            } else {
                new_name.clone()
            };
            state.branches.insert(new_key.clone());
            let old_full = full_ref_name(&key);
            if let Some(sha) = state.refs.remove(&old_full) {
                state.refs.insert(full_ref_name(&new_key), sha);
            }
            if let Some(history) = state.ref_histories.remove(&branch) {
                state.ref_histories.insert(new_name.clone(), history);
            }
            if let Some(upstream) = state.upstreams.remove(&branch) {
                state.upstreams.insert(new_name.clone(), upstream);
            }
            for recent in &mut state.recent_branches {
                if *recent == branch {
                    *recent = new_name.clone();
                }
            }
            if state.current_branch_name == Some(branch) {
                state.current_branch_name = Some(new_name);
            }
            Ok(())
        })
    }

    fn delete_branch(
        &self,
        is_remote: bool,
        name: String,
        force: bool,
    ) -> BoxFuture<'_, Result<()>> {
        self.with_state_async(true, move |state| {
            if !force && state.branches_requiring_force_delete.contains(&name) {
                return Err(git_failure(
                    GitFailureKind::BranchNotFullyMerged,
                    format!(
                        "error: The branch '{name}' is not fully merged.\nIf you are sure you want to delete it, run 'git branch -D {name}'."
                    ),
                ));
            }
            let key = if state.branches.contains(&name) {
                Some(name.clone())
            } else if is_remote {
                remote_branch_key(state, &name)
            } else {
                local_branch_key(state, &name)
            };
            let Some(key) = key else {
                bail!("no such branch: {name}");
            };
            state.branches.remove(&key);
            forget_branch(state, &key);
            state.branches_requiring_force_delete.remove(&name);
            Ok(())
        })
    }

    fn tags(&self) -> BoxFuture<'_, Result<Vec<Tag>>> {
        self.with_state_async(false, |state| Ok(state.tags.clone()))
    }

    fn recent_branches(&self, limit: usize) -> BoxFuture<'_, Result<Vec<SharedString>>> {
        self.with_state_async(false, move |state| {
            let state = &*state;
            let mut recent = Vec::<SharedString>::new();
            for name in &state.recent_branches {
                if recent.len() >= limit {
                    break;
                }
                let already_listed = recent.iter().any(|listed| &**listed == name.as_str());
                if !already_listed && local_branch_key(state, name).is_some() {
                    recent.push(name.clone().into());
                }
            }
            Ok(recent)
        })
    }

    fn commits_between(
        &self,
        base: String,
        head: String,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<CommitSummary>>> {
        self.with_state_async(false, move |state| {
            let state = &*state;
            for revision in [&base, &head] {
                if revision_sha(state, revision).is_none() {
                    return Err(git_failure(
                        GitFailureKind::RevisionNotFound,
                        format!("fatal: bad revision '{revision}'"),
                    ));
                }
            }
            let base_ids = ref_history(state, &base)
                .into_iter()
                .map(|commit| commit.sha)
                .collect::<HashSet<SharedString>>();
            Ok(ref_history(state, &head)
                .into_iter()
                .filter(|commit| !base_ids.contains(&commit.sha))
                .take(limit)
                .collect())
        })
    }

    fn operation_in_progress(&self) -> BoxFuture<'_, Result<Option<RepositoryOperation>>> {
        self.with_state_async(false, |state| {
            let operation = if state.rebase_session.is_some() {
                Some(RepositoryOperation::Rebase)
            } else if state.refs.contains_key("MERGE_HEAD") {
                Some(RepositoryOperation::Merge)
            } else if state.refs.contains_key("CHERRY_PICK_HEAD") {
                Some(RepositoryOperation::CherryPick)
            } else if state.refs.contains_key("REVERT_HEAD") {
                Some(RepositoryOperation::Revert)
            } else {
                None
            };
            Ok(operation)
        })
    }

    fn checkout_detached(&self, revision: String) -> BoxFuture<'_, Result<()>> {
        self.with_state_async(true, move |state| {
            let Some(sha) = revision_sha(state, &revision) else {
                return Err(git_failure(
                    GitFailureKind::RevisionNotFound,
                    format!("fatal: invalid reference: {revision}"),
                ));
            };
            ensure_no_unmerged_paths(state, "error: you need to resolve your current index first")?;
            simulated_failure(state, FakeGitOperation::Checkout)?;
            detach_head_at(state, &revision, sha);
            Ok(())
        })
    }

    fn checkout_force(&self, name: String) -> BoxFuture<'_, Result<()>> {
        self.with_state_async(true, move |state| {
            if local_branch_key(state, &name).is_some() {
                switch_to_branch(state, &name);
            } else if let Some(sha) = revision_sha(state, &name) {
                detach_head_at(state, &name, sha);
            } else {
                return Err(git_failure(
                    GitFailureKind::Other,
                    format!("error: pathspec '{name}' did not match any file(s) known to git"),
                ));
            }
            state.index_contents = state.head_contents.clone();
            state.unmerged_paths.clear();
            state.conflict_stages.clear();
            Ok(())
        })
    }

    fn create_branch_at(
        &self,
        name: String,
        start_point: String,
        checkout: bool,
        overwrite: bool,
    ) -> BoxFuture<'_, Result<()>> {
        self.with_state_async(true, move |state| {
            simulated_failure(state, FakeGitOperation::CreateBranch)?;
            if checkout {
                simulated_failure(state, FakeGitOperation::Checkout)?;
            }
            let existing_key = local_branch_key(state, &name);
            if existing_key.is_some() && !overwrite {
                return Err(git_failure(
                    GitFailureKind::Other,
                    format!("fatal: a branch named '{name}' already exists"),
                ));
            }
            let Some(start_sha) = revision_sha(state, &start_point) else {
                return Err(git_failure(
                    GitFailureKind::Other,
                    format!("fatal: not a valid object name: '{start_point}'"),
                ));
            };
            let is_current = state.current_branch_name.as_deref() == Some(name.as_str());
            let checkout = checkout || (overwrite && is_current);
            let tracked_remote = if local_branch_key(state, &start_point).is_none()
                && tag_sha(state, &start_point).is_none()
            {
                remote_branch_key(state, &start_point).map(|key| full_ref_name(&key))
            } else {
                None
            };
            let history = ref_history(state, &start_point);

            if existing_key.is_none() {
                state.branches.insert(local_branch_storage_key(&name));
            }
            state
                .refs
                .insert(format!("refs/heads/{name}"), start_sha.clone());
            state.ref_histories.insert(name.clone(), history);
            if let Some(remote_ref) = tracked_remote {
                state.upstreams.insert(
                    name.clone(),
                    Upstream {
                        ref_name: remote_ref.into(),
                        tracking: UpstreamTracking::Tracked(UpstreamTrackingStatus {
                            ahead: 0,
                            behind: 0,
                        }),
                    },
                );
            }
            if checkout {
                note_checkout(state, Some(&name));
                state.current_branch_name = Some(name);
                state.refs.insert("HEAD".into(), start_sha);
                clear_merge_markers(state);
            }
            Ok(())
        })
    }

    fn set_upstream(&self, branch: String, upstream: Option<String>) -> BoxFuture<'_, Result<()>> {
        self.with_state_async(true, move |state| {
            if local_branch_key(state, &branch).is_none() {
                return Err(git_failure(
                    GitFailureKind::Other,
                    format!("fatal: branch '{branch}' does not exist"),
                ));
            }
            let Some(upstream) = upstream else {
                state.upstreams.remove(&branch);
                return Ok(());
            };
            let ref_name = if let Some(key) = remote_branch_key(state, &upstream) {
                full_ref_name(&key)
            } else if local_branch_key(state, &upstream).is_some() {
                format!("refs/heads/{upstream}")
            } else {
                return Err(git_failure(
                    GitFailureKind::Other,
                    format!("fatal: the requested upstream branch '{upstream}' does not exist"),
                ));
            };
            state.upstreams.insert(
                branch,
                Upstream {
                    ref_name: ref_name.into(),
                    tracking: UpstreamTracking::Tracked(UpstreamTrackingStatus {
                        ahead: 0,
                        behind: 0,
                    }),
                },
            );
            Ok(())
        })
    }

    fn merge(
        &self,
        reference: String,
        _env: Arc<HashMap<String, String>>,
    ) -> BoxFuture<'_, Result<MergeOutcome>> {
        async move {
            let (outcome, conflict_files) = self
                .with_state_async(true, move |state| fake_merge(state, &reference))
                .await?;
            self.write_conflict_files(conflict_files).await?;
            Ok(outcome)
        }
        .boxed()
    }

    fn merge_abort(&self, _env: Arc<HashMap<String, String>>) -> BoxFuture<'_, Result<()>> {
        self.with_state_async(true, |state| {
            if !state.refs.contains_key("MERGE_HEAD") {
                return Err(git_failure(
                    GitFailureKind::Other,
                    "fatal: There is no merge to abort (MERGE_HEAD missing).",
                ));
            }
            clear_merge_markers(state);
            state.unmerged_paths.clear();
            state.conflict_stages.clear();
            Ok(())
        })
    }

    fn rebase(
        &self,
        action: RebaseAction,
        _env: Arc<HashMap<String, String>>,
    ) -> BoxFuture<'_, Result<RebaseOutcome>> {
        async move {
            let (outcome, conflict_files) = self
                .with_state_async(true, move |state| fake_rebase(state, action))
                .await?;
            self.write_conflict_files(conflict_files).await?;
            Ok(outcome)
        }
        .boxed()
    }

    fn reset_hard(
        &self,
        commit: String,
        _env: Arc<HashMap<String, String>>,
    ) -> BoxFuture<'_, Result<()>> {
        self.with_state_async(true, move |state| {
            simulated_failure(state, FakeGitOperation::ResetHard)?;
            let rewinds_history = commit.starts_with("HEAD~")
                || commit == "HEAD^"
                || state.commit_history.iter().any(|entry| entry.sha == commit);
            if rewinds_history {
                let snapshot = rewind_commit_history(state, &commit)?;
                state.head_contents = snapshot.head_contents;
                set_current_tip(state, snapshot.sha);
            } else {
                let Some(sha) = revision_sha(state, &commit) else {
                    return Err(git_failure(
                        GitFailureKind::RevisionNotFound,
                        format!(
                            "fatal: ambiguous argument '{commit}': unknown revision or path not in the working tree."
                        ),
                    ));
                };
                let history = ref_history(state, &commit);
                set_current_history(state, history);
                set_current_tip(state, sha);
            }
            state.index_contents = state.head_contents.clone();
            state.unmerged_paths.clear();
            state.conflict_stages.clear();
            clear_merge_markers(state);
            Ok(())
        })
    }

    fn delete_remote_branch(
        &self,
        remote: String,
        branch: String,
        _askpass: AskPassDelegate,
        _env: Arc<HashMap<String, String>>,
        _cx: AsyncApp,
    ) -> BoxFuture<'_, Result<RemoteCommandOutput>> {
        self.with_state_async(true, move |state| {
            simulated_failure(state, FakeGitOperation::DeleteRemoteBranch)?;
            let tracking_name = format!("{remote}/{branch}");
            let Some(key) = remote_branch_key(state, &tracking_name) else {
                return Ok(RemoteCommandOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                });
            };
            state.branches.remove(&key);
            forget_branch(state, &key);
            Ok(RemoteCommandOutput {
                stdout: String::new(),
                stderr: format!("To {remote}\n - [deleted]         {branch}\n"),
            })
        })
    }

    fn fast_forward_branch(
        &self,
        remote: String,
        remote_branch: String,
        local_branch: String,
        _askpass: AskPassDelegate,
        _env: Arc<HashMap<String, String>>,
        _cx: AsyncApp,
    ) -> BoxFuture<'_, Result<RemoteCommandOutput>> {
        self.with_state_async(true, move |state| {
            simulated_failure(state, FakeGitOperation::FastForwardBranch)?;
            let tracking_name = format!("{remote}/{remote_branch}");
            let Some(remote_key) = remote_branch_key(state, &tracking_name) else {
                return Err(git_failure(
                    GitFailureKind::Other,
                    format!("fatal: couldn't find remote ref {remote_branch}"),
                ));
            };
            if state.current_branch_name.as_deref() == Some(local_branch.as_str()) {
                return Err(git_failure(
                    GitFailureKind::Other,
                    format!(
                        "fatal: refusing to fetch into current branch refs/heads/{local_branch} of non-bare repository"
                    ),
                ));
            }
            let local_exists = local_branch_key(state, &local_branch).is_some();
            let local_sha = local_exists.then(|| local_branch_tip(state, &local_branch));
            let remote_sha = remote_branch_tip(state, &tracking_name);
            let local_history = ref_history(state, &local_branch);
            let remote_history = ref_history(state, &tracking_name);
            let remote_ids = remote_history
                .iter()
                .map(|commit| commit.sha.clone())
                .collect::<HashSet<SharedString>>();
            let is_fast_forward = local_history
                .iter()
                .all(|commit| remote_ids.contains(&commit.sha));
            if !is_fast_forward {
                return Err(git_failure(
                    GitFailureKind::Other,
                    format!(
                        " ! [rejected]        {remote_branch} -> {local_branch}  (non-fast-forward)\nerror: some local refs could not be updated"
                    ),
                ));
            }

            if !local_exists {
                state
                    .branches
                    .insert(local_branch_storage_key(&local_branch));
            }
            state
                .refs
                .insert(format!("refs/heads/{local_branch}"), remote_sha.clone());
            state
                .ref_histories
                .insert(local_branch.clone(), remote_history);
            let remote_ref = full_ref_name(&remote_key);
            if let Some(upstream) = state.upstreams.get_mut(&local_branch)
                && &*upstream.ref_name == remote_ref.as_str()
            {
                upstream.tracking = UpstreamTracking::Tracked(UpstreamTrackingStatus {
                    ahead: 0,
                    behind: 0,
                });
            }
            let stderr = match local_sha {
                Some(local_sha) => format!(
                    "   {}..{}  {remote_branch} -> {local_branch}\n",
                    short_sha(&local_sha),
                    short_sha(&remote_sha)
                ),
                None => format!(" * [new branch]      {remote_branch} -> {local_branch}\n"),
            };
            Ok(RemoteCommandOutput {
                stdout: String::new(),
                stderr,
            })
        })
    }

    fn delete_tag(&self, name: String) -> BoxFuture<'_, Result<()>> {
        self.with_state_async(true, move |state| {
            let Some(position) = state.tags.iter().position(|tag| *tag.name == *name) else {
                return Err(git_failure(
                    GitFailureKind::Other,
                    format!("error: tag '{name}' not found."),
                ));
            };
            state.tags.remove(position);
            Ok(())
        })
    }

    fn create_tag(&self, name: String, target: String) -> BoxFuture<'_, Result<()>> {
        self.with_state_async(true, move |state| {
            if state.tags.iter().any(|tag| *tag.name == *name) {
                return Err(git_failure(
                    GitFailureKind::Other,
                    format!("fatal: tag '{name}' already exists"),
                ));
            }
            let Some(sha) = revision_sha(state, &target) else {
                return Err(git_failure(
                    GitFailureKind::Other,
                    format!("fatal: Failed to resolve '{target}' as a valid ref."),
                ));
            };
            let history = ref_history(state, &target);
            state.ref_histories.insert(name.clone(), history);
            state.tags.insert(
                0,
                Tag {
                    name: name.into(),
                    commit_sha: sha.into(),
                },
            );
            Ok(())
        })
    }

    fn push_tag(
        &self,
        remote: String,
        tag: String,
        _askpass: AskPassDelegate,
        _env: Arc<HashMap<String, String>>,
        _cx: AsyncApp,
    ) -> BoxFuture<'_, Result<RemoteCommandOutput>> {
        self.with_state_async(true, move |state| {
            simulated_failure(state, FakeGitOperation::PushTag)?;
            if !state.tags.iter().any(|existing| *existing.name == *tag) {
                return Err(git_failure(
                    GitFailureKind::Other,
                    format!("error: src refspec refs/tags/{tag} does not match any"),
                ));
            }
            let entry = (remote.clone(), tag.clone());
            let stderr = if state.pushed_tags.contains(&entry) {
                "Everything up-to-date\n".to_string()
            } else {
                state.pushed_tags.push(entry);
                format!("To {remote}\n * [new tag]         {tag} -> {tag}\n")
            };
            Ok(RemoteCommandOutput {
                stdout: String::new(),
                stderr,
            })
        })
    }

    fn blame(
        &self,
        path: RepoPath,
        _content: Rope,
        _line_ending: LineEnding,
    ) -> BoxFuture<'_, Result<git::blame::Blame>> {
        self.with_state_async(false, move |state| {
            state
                .blames
                .get(&path)
                .with_context(|| format!("failed to get blame for {:?}", path))
                .cloned()
        })
    }

    fn blame_at_revision(
        &self,
        path: RepoPath,
        revision: Oid,
    ) -> BoxFuture<'_, Result<git::blame::Blame>> {
        self.with_state_async(false, move |state| {
            state
                .blames_at_revision
                .get(&(path.clone(), revision))
                .with_context(|| format!("failed to get blame for {path:?} at {revision}"))
                .cloned()
        })
    }

    fn stage_paths(
        &self,
        paths: Vec<RepoPath>,
        _env: Arc<HashMap<String, String>>,
    ) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let contents = paths
                .into_iter()
                .map(|path| {
                    let abs_path = self
                        .dot_git_path
                        .parent()
                        .unwrap()
                        .join(&path.as_std_path());
                    Box::pin(
                        async move { (path.clone(), self.fs.load_bytes(&abs_path).await.ok()) },
                    )
                })
                .collect::<Vec<_>>();
            let contents = join_all(contents).await;
            self.with_state_async(true, move |state| {
                for (path, content) in contents {
                    if let Some(content) = content {
                        state.index_contents.insert(path.clone(), content);
                    } else {
                        state.index_contents.remove(&path);
                    }
                    if state.conflict_stages.contains_key(&path) {
                        state.unmerged_paths.remove(&path);
                    }
                }
                Ok(())
            })
            .await
        })
    }

    fn unstage_paths(
        &self,
        paths: Vec<RepoPath>,
        _env: Arc<HashMap<String, String>>,
    ) -> BoxFuture<'_, Result<()>> {
        self.with_state_async(true, move |state| {
            for path in paths {
                match state.head_contents.get(&path) {
                    Some(content) => state.index_contents.insert(path, content.clone()),
                    None => state.index_contents.remove(&path),
                };
            }
            Ok(())
        })
    }

    fn unresolve_paths(
        &self,
        paths: Vec<RepoPath>,
        _env: Arc<HashMap<String, String>>,
    ) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let working_directory = self
                .dot_git_path
                .parent()
                .context("repository has no working directory")?
                .to_path_buf();
            let requested_paths = paths.clone();
            let conflicts = self
                .with_state_async(false, move |state| {
                    requested_paths
                        .into_iter()
                        .map(|path| {
                            let stages = state
                                .conflict_stages
                                .get(&path)
                                .with_context(|| format!("{path:?} has no recorded conflict"))?;
                            Ok((path, stages.conflict_text()))
                        })
                        .collect::<Result<Vec<_>>>()
                })
                .await?;
            for (path, conflict_text) in conflicts {
                let absolute_path = working_directory.join(path.as_std_path());
                match conflict_text {
                    Some(conflict_text) => {
                        self.fs
                            .write_file_internal(&absolute_path, conflict_text, false)?
                    }
                    None => {
                        self.fs
                            .remove_file(
                                &absolute_path,
                                RemoveOptions {
                                    recursive: false,
                                    ignore_if_not_exists: true,
                                },
                            )
                            .await?
                    }
                }
            }
            self.with_state_async(true, move |state| {
                for path in paths {
                    if let Some(stages) = state.conflict_stages.get(&path) {
                        let unmerged_status = stages.unmerged_status();
                        state.unmerged_paths.insert(path.clone(), unmerged_status);
                        state.index_contents.remove(&path);
                    }
                }
                Ok(())
            })
            .await
        })
    }

    fn checkout_conflict_side(
        &self,
        paths: Vec<RepoPath>,
        side: ConflictSide,
        _env: Arc<HashMap<String, String>>,
    ) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            if paths.is_empty() {
                return anyhow::Ok(());
            }
            let working_directory = self
                .dot_git_path
                .parent()
                .context("repository has no working directory")?
                .to_path_buf();
            let side_contents = self
                .with_state_async(false, move |state| {
                    let mut side_contents = Vec::new();
                    for path in paths {
                        if let Some(content) = conflict_side_content(state, &path, side)? {
                            side_contents.push((path, content));
                        }
                    }
                    Ok(side_contents)
                })
                .await?;
            for (path, content) in side_contents {
                self.fs.write_file_internal(
                    &working_directory.join(path.as_std_path()),
                    content,
                    false,
                )?;
            }
            anyhow::Ok(())
        })
    }

    fn mark_conflicts_resolved(
        &self,
        to_add: Vec<RepoPath>,
        to_remove: Vec<RepoPath>,
        _env: Arc<HashMap<String, String>>,
    ) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            if to_add.is_empty() && to_remove.is_empty() {
                return anyhow::Ok(());
            }
            let working_directory = self
                .dot_git_path
                .parent()
                .context("repository has no working directory")?
                .to_path_buf();
            let worktree_contents = join_all(to_add.iter().map(|path| {
                let absolute_path = working_directory.join(path.as_std_path());
                async move { self.fs.load_bytes(&absolute_path).await.ok() }
            }))
            .await;
            let removed_absolute_paths = to_remove
                .iter()
                .map(|path| working_directory.join(path.as_std_path()))
                .collect::<Vec<_>>();
            self.with_state_async(true, move |state| {
                for (path, content) in to_add.iter().zip(&worktree_contents) {
                    if content.is_none() && !is_tracked(state, path) {
                        return Err(unmatched_pathspec(path));
                    }
                }
                if let Some(path) = to_remove.iter().find(|path| !is_tracked(state, path)) {
                    return Err(git_failure(
                        GitFailureKind::Other,
                        format!(
                            "fatal: pathspec '{}' did not match any files",
                            path.as_unix_str()
                        ),
                    ));
                }
                for (path, content) in to_add.into_iter().zip(worktree_contents) {
                    match content {
                        Some(content) => state.index_contents.insert(path.clone(), content),
                        None => state.index_contents.remove(&path),
                    };
                    state.unmerged_paths.remove(&path);
                }
                for path in to_remove {
                    state.index_contents.remove(&path);
                    state.unmerged_paths.remove(&path);
                }
                Ok(())
            })
            .await?;
            for absolute_path in removed_absolute_paths {
                self.fs
                    .remove_file(
                        &absolute_path,
                        RemoveOptions {
                            recursive: false,
                            ignore_if_not_exists: true,
                        },
                    )
                    .await?;
            }
            anyhow::Ok(())
        })
    }

    fn commit_merge(&self, _env: Arc<HashMap<String, String>>) -> BoxFuture<'_, Result<()>> {
        let working_directory = self
            .dot_git_path
            .parent()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        self.with_state_async(true, move |state| {
            ensure_no_unmerged_paths(
                state,
                "error: Committing is not possible because you have unmerged files.\nhint: Fix them up in the work tree, and then use 'git add/rm <file>'\nhint: as appropriate to mark resolution and make a commit.\nfatal: Exiting because of an unresolved conflict.",
            )?;
            let subject = match state.merge_message.as_deref() {
                Some(message) => message
                    .lines()
                    .find(|line| !line.trim().is_empty() && !line.starts_with('#'))
                    .unwrap_or_default()
                    .to_string(),
                None => format!(
                    "Merge branch '{}' of {working_directory} with conflicts.",
                    state.current_branch_name.clone().unwrap_or_default()
                ),
            };
            let previous_sha = state.refs.get("HEAD").cloned().unwrap_or_default();
            state.commit_history.push(FakeCommitSnapshot {
                head_contents: state.head_contents.clone(),
                index_contents: state.index_contents.clone(),
                sha: previous_sha,
            });
            state.head_contents = state.index_contents.clone();
            let merge_sha = format!("fake-commit-{}", state.commit_history.len());
            let history = ref_history(state, "HEAD");
            if let Some(newest) = history.first() {
                let merge_commit = CommitSummary {
                    sha: merge_sha.clone().into(),
                    subject: subject.into(),
                    commit_timestamp: newest.commit_timestamp,
                    author_name: SharedString::default(),
                    has_parent: true,
                };
                let mut merged_history = vec![merge_commit];
                merged_history.extend(history);
                set_current_history(state, merged_history);
            }
            set_current_tip(state, merge_sha);
            state.conflict_stages.clear();
            clear_merge_markers(state);
            Ok(())
        })
    }

    fn stash_paths(
        &self,
        _paths: Vec<RepoPath>,
        _message: Option<String>,
        _env: Arc<HashMap<String, String>>,
    ) -> BoxFuture<'_, Result<()>> {
        unimplemented!()
    }

    fn stash_staged(
        &self,
        _message: Option<String>,
        _env: Arc<HashMap<String, String>>,
    ) -> BoxFuture<'_, Result<()>> {
        unimplemented!()
    }

    fn stash_pop(
        &self,
        _index: Option<usize>,
        _env: Arc<HashMap<String, String>>,
    ) -> BoxFuture<'_, Result<()>> {
        unimplemented!()
    }

    fn stash_apply(
        &self,
        _index: Option<usize>,
        _env: Arc<HashMap<String, String>>,
    ) -> BoxFuture<'_, Result<()>> {
        unimplemented!()
    }

    fn stash_drop(
        &self,
        _index: Option<usize>,
        _env: Arc<HashMap<String, String>>,
    ) -> BoxFuture<'_, Result<()>> {
        unimplemented!()
    }

    fn commit(
        &self,
        _message: gpui::SharedString,
        _name_and_email: Option<(gpui::SharedString, gpui::SharedString)>,
        options: CommitOptions,
        _askpass: AskPassDelegate,
        _env: Arc<HashMap<String, String>>,
    ) -> BoxFuture<'_, Result<()>> {
        self.with_state_async(true, move |state| {
            if !options.allow_empty && !options.amend && state.index_contents == state.head_contents
            {
                anyhow::bail!("nothing to commit (use allow_empty to create an empty commit)");
            }

            let old_sha = state.refs.get("HEAD").cloned().unwrap_or_default();
            state.commit_history.push(FakeCommitSnapshot {
                head_contents: state.head_contents.clone(),
                index_contents: state.index_contents.clone(),
                sha: old_sha,
            });

            state.head_contents = state.index_contents.clone();

            let new_sha = format!("fake-commit-{}", state.commit_history.len());
            state.refs.insert("HEAD".into(), new_sha);

            Ok(())
        })
    }

    fn run_hook(
        &self,
        _hook: RunHook,
        _env: Arc<HashMap<String, String>>,
    ) -> BoxFuture<'_, Result<()>> {
        async { Ok(()) }.boxed()
    }

    fn push(
        &self,
        _branch: String,
        _remote_branch: String,
        _remote: String,
        _options: Option<PushOptions>,
        _askpass: AskPassDelegate,
        _env: Arc<HashMap<String, String>>,
        _cx: AsyncApp,
    ) -> BoxFuture<'_, Result<git::repository::RemoteCommandOutput>> {
        unimplemented!()
    }

    fn pull(
        &self,
        _branch: Option<String>,
        _remote: String,
        _rebase: bool,
        _askpass: AskPassDelegate,
        _env: Arc<HashMap<String, String>>,
        _cx: AsyncApp,
    ) -> BoxFuture<'_, Result<git::repository::RemoteCommandOutput>> {
        unimplemented!()
    }

    fn fetch(
        &self,
        _fetch_options: FetchOptions,
        _askpass: AskPassDelegate,
        _env: Arc<HashMap<String, String>>,
        _cx: AsyncApp,
    ) -> BoxFuture<'_, Result<git::repository::RemoteCommandOutput>> {
        unimplemented!()
    }

    fn get_all_remotes(&self) -> BoxFuture<'_, Result<Vec<Remote>>> {
        self.with_state_async(false, move |state| {
            let remotes = state
                .remotes
                .keys()
                .map(|r| Remote {
                    name: r.clone().into(),
                })
                .collect::<Vec<_>>();
            Ok(remotes)
        })
    }

    fn get_push_remote(&self, _branch: String) -> BoxFuture<'_, Result<Option<Remote>>> {
        unimplemented!()
    }

    fn get_branch_remote(&self, _branch: String) -> BoxFuture<'_, Result<Option<Remote>>> {
        unimplemented!()
    }

    fn check_for_pushed_commit(&self) -> BoxFuture<'_, Result<Vec<gpui::SharedString>>> {
        future::ready(Ok(Vec::new())).boxed()
    }

    fn diff(&self, _diff: git::repository::DiffType) -> BoxFuture<'_, Result<String>> {
        future::ready(Ok(String::new())).boxed()
    }

    fn diff_stat(
        &self,
        diff: git::repository::DiffStatType,
        path_prefixes: &[RepoPath],
    ) -> BoxFuture<'static, Result<git::status::GitDiffStat>> {
        fn count_lines(bytes: &[u8]) -> u32 {
            if bytes.is_empty() {
                0
            } else {
                String::from_utf8_lossy(bytes).lines().count() as u32
            }
        }

        fn matches_prefixes(path: &RepoPath, prefixes: &[RepoPath]) -> bool {
            if prefixes.is_empty() {
                return true;
            }
            prefixes.iter().any(|prefix| {
                let prefix_str = prefix.as_unix_str();
                if prefix_str == "." {
                    return true;
                }
                path == prefix || path.starts_with(&prefix)
            })
        }

        let path_prefixes = path_prefixes.to_vec();

        let workdir_path = self.dot_git_path.parent().unwrap().to_path_buf();
        let worktree_files: HashMap<RepoPath, Vec<u8>> = self
            .fs
            .files()
            .iter()
            .filter_map(|path| {
                let repo_path = path.strip_prefix(&workdir_path).ok()?;
                if repo_path.starts_with(".git") {
                    return None;
                }
                let content = self.fs.read_file_sync(path).ok()?;
                let repo_path = RelPath::new(repo_path, PathStyle::local()).ok()?;
                Some((RepoPath::from_rel_path(&repo_path), content))
            })
            .collect();

        self.with_state_async(false, move |state| {
            let mut entries = Vec::new();
            let (old_files, new_files) = match diff {
                git::repository::DiffStatType::HeadToIndex => {
                    (&state.head_contents, &state.index_contents)
                }
                git::repository::DiffStatType::HeadToWorktree => {
                    (&state.head_contents, &worktree_files)
                }
                git::repository::DiffStatType::IndexToWorktree => {
                    (&state.index_contents, &worktree_files)
                }
            };
            let all_paths: HashSet<&RepoPath> = match diff {
                git::repository::DiffStatType::HeadToIndex => state
                    .head_contents
                    .keys()
                    .chain(state.index_contents.keys())
                    .collect(),
                git::repository::DiffStatType::HeadToWorktree => state
                    .head_contents
                    .keys()
                    .chain(
                        worktree_files
                            .keys()
                            .filter(|path| state.index_contents.contains_key(*path)),
                    )
                    .collect(),
                git::repository::DiffStatType::IndexToWorktree => {
                    state.index_contents.keys().collect()
                }
            };
            for path in all_paths {
                if !matches_prefixes(path, &path_prefixes) {
                    continue;
                }
                let old_file = old_files.get(path);
                let new_file = new_files.get(path);
                match (old_file, new_file) {
                    (Some(old), Some(new)) if old != new => {
                        entries.push((
                            path.clone(),
                            git::status::DiffStat {
                                added: count_lines(new),
                                deleted: count_lines(old),
                            },
                        ));
                    }
                    (Some(old), None) => {
                        entries.push((
                            path.clone(),
                            git::status::DiffStat {
                                added: 0,
                                deleted: count_lines(old),
                            },
                        ));
                    }
                    (None, Some(new)) => {
                        entries.push((
                            path.clone(),
                            git::status::DiffStat {
                                added: count_lines(new),
                                deleted: 0,
                            },
                        ));
                    }
                    _ => {}
                }
            }
            entries.sort_by(|(a, _), (b, _)| a.cmp(b));
            Ok(git::status::GitDiffStat {
                entries: entries.into(),
            })
        })
        .boxed()
    }

    fn checkpoint(&self) -> BoxFuture<'static, Result<GitRepositoryCheckpoint>> {
        let executor = self.executor.clone();
        let fs = self.fs.clone();
        let checkpoints = self.checkpoints.clone();
        let repository_dir_path = self.repository_dir_path.parent().unwrap().to_path_buf();
        async move {
            executor.simulate_random_delay().await;
            let oid = git::Oid::random(&mut *executor.rng().lock());
            let entry = fs.entry(&repository_dir_path)?;
            checkpoints.lock().insert(oid, entry);
            Ok(GitRepositoryCheckpoint { commit_sha: oid })
        }
        .boxed()
    }

    fn restore_checkpoint(&self, checkpoint: GitRepositoryCheckpoint) -> BoxFuture<'_, Result<()>> {
        let executor = self.executor.clone();
        let fs = self.fs.clone();
        let checkpoints = self.checkpoints.clone();
        let repository_dir_path = self.repository_dir_path.parent().unwrap().to_path_buf();
        async move {
            executor.simulate_random_delay().await;
            let checkpoints = checkpoints.lock();
            let entry = checkpoints
                .get(&checkpoint.commit_sha)
                .context(format!("invalid checkpoint: {}", checkpoint.commit_sha))?;
            fs.insert_entry(&repository_dir_path, entry.clone())?;
            Ok(())
        }
        .boxed()
    }

    fn create_archive_checkpoint(&self) -> BoxFuture<'_, Result<(String, String)>> {
        let executor = self.executor.clone();
        let fs = self.fs.clone();
        let checkpoints = self.checkpoints.clone();
        let repository_dir_path = self.repository_dir_path.parent().unwrap().to_path_buf();
        async move {
            executor.simulate_random_delay().await;
            let staged_oid = git::Oid::random(&mut *executor.rng().lock());
            let unstaged_oid = git::Oid::random(&mut *executor.rng().lock());
            let entry = fs.entry(&repository_dir_path)?;
            checkpoints.lock().insert(staged_oid, entry.clone());
            checkpoints.lock().insert(unstaged_oid, entry);
            Ok((staged_oid.to_string(), unstaged_oid.to_string()))
        }
        .boxed()
    }

    fn restore_archive_checkpoint(
        &self,
        // The fake filesystem doesn't model a separate index, so only the
        // unstaged (full working directory) snapshot is restored.
        _staged_sha: String,
        unstaged_sha: String,
    ) -> BoxFuture<'_, Result<()>> {
        match unstaged_sha.parse() {
            Ok(commit_sha) => self.restore_checkpoint(GitRepositoryCheckpoint { commit_sha }),
            Err(error) => async move {
                Err(anyhow::anyhow!(error).context("failed to parse unstaged SHA as Oid"))
            }
            .boxed(),
        }
    }

    fn compare_checkpoints(
        &self,
        left: GitRepositoryCheckpoint,
        right: GitRepositoryCheckpoint,
    ) -> BoxFuture<'_, Result<bool>> {
        let executor = self.executor.clone();
        let checkpoints = self.checkpoints.clone();
        async move {
            executor.simulate_random_delay().await;
            let checkpoints = checkpoints.lock();
            let left = checkpoints
                .get(&left.commit_sha)
                .context(format!("invalid left checkpoint: {}", left.commit_sha))?;
            let right = checkpoints
                .get(&right.commit_sha)
                .context(format!("invalid right checkpoint: {}", right.commit_sha))?;

            Ok(left == right)
        }
        .boxed()
    }

    fn diff_checkpoints(
        &self,
        base_checkpoint: GitRepositoryCheckpoint,
        target_checkpoint: GitRepositoryCheckpoint,
    ) -> BoxFuture<'_, Result<String>> {
        let executor = self.executor.clone();
        let checkpoints = self.checkpoints.clone();
        async move {
            executor.simulate_random_delay().await;
            let checkpoints = checkpoints.lock();
            let base = checkpoints
                .get(&base_checkpoint.commit_sha)
                .context(format!(
                    "invalid base checkpoint: {}",
                    base_checkpoint.commit_sha
                ))?;
            let target = checkpoints
                .get(&target_checkpoint.commit_sha)
                .context(format!(
                    "invalid target checkpoint: {}",
                    target_checkpoint.commit_sha
                ))?;

            fn collect_files(
                entry: &FakeFsEntry,
                prefix: String,
                out: &mut std::collections::BTreeMap<String, String>,
            ) {
                match entry {
                    FakeFsEntry::File { content, .. } => {
                        out.insert(prefix, String::from_utf8_lossy(content).into_owned());
                    }
                    FakeFsEntry::Dir { entries, .. } => {
                        for (name, child) in entries {
                            let path = if prefix.is_empty() {
                                name.clone()
                            } else {
                                format!("{prefix}/{name}")
                            };
                            collect_files(child, path, out);
                        }
                    }
                    FakeFsEntry::Symlink { .. } => {}
                }
            }

            let mut base_files = std::collections::BTreeMap::new();
            let mut target_files = std::collections::BTreeMap::new();
            collect_files(base, String::new(), &mut base_files);
            collect_files(target, String::new(), &mut target_files);

            let all_paths: std::collections::BTreeSet<&String> =
                base_files.keys().chain(target_files.keys()).collect();

            let mut diff = String::new();
            for path in all_paths {
                match (base_files.get(path), target_files.get(path)) {
                    (Some(base_content), Some(target_content))
                        if base_content != target_content =>
                    {
                        diff.push_str(&format!("diff --git a/{path} b/{path}\n"));
                        diff.push_str(&format!("--- a/{path}\n"));
                        diff.push_str(&format!("+++ b/{path}\n"));
                        for line in base_content.lines() {
                            diff.push_str(&format!("-{line}\n"));
                        }
                        for line in target_content.lines() {
                            diff.push_str(&format!("+{line}\n"));
                        }
                    }
                    (Some(_), None) => {
                        diff.push_str(&format!("diff --git a/{path} /dev/null\n"));
                        diff.push_str("deleted file\n");
                    }
                    (None, Some(_)) => {
                        diff.push_str(&format!("diff --git /dev/null b/{path}\n"));
                        diff.push_str("new file\n");
                    }
                    _ => {}
                }
            }
            Ok(diff)
        }
        .boxed()
    }

    fn default_branch(
        &self,
        include_remote_name: bool,
    ) -> BoxFuture<'_, Result<Option<SharedString>>> {
        async move {
            Ok(Some(if include_remote_name {
                "origin/main".into()
            } else {
                "main".into()
            }))
        }
        .boxed()
    }

    fn create_remote(&self, name: String, url: String) -> BoxFuture<'_, Result<()>> {
        self.with_state_async(true, move |state| {
            state.remotes.insert(name, url);
            Ok(())
        })
    }

    fn remove_remote(&self, name: String) -> BoxFuture<'_, Result<()>> {
        self.with_state_async(true, move |state| {
            state.branches.retain(|branch| {
                branch
                    .split_once('/')
                    .is_none_or(|(remote, _)| remote != name)
            });
            state.remotes.remove(&name);
            Ok(())
        })
    }

    fn initial_graph_data(
        &self,
        _log_source: LogSource,
        _log_order: LogOrder,
        request_tx: Sender<Vec<Arc<InitialGraphCommitData>>>,
    ) -> BoxFuture<'_, Result<()>> {
        let fs = self.fs.clone();
        let dot_git_path = self.dot_git_path.clone();
        async move {
            let (graph_commits, simulated_error) =
                fs.with_git_state(&dot_git_path, false, |state| {
                    (
                        state.graph_commits.clone(),
                        state.simulated_graph_error.clone(),
                    )
                })?;

            if let Some(error) = simulated_error {
                anyhow::bail!("{}", error);
            }

            for chunk in graph_commits.chunks(GRAPH_CHUNK_SIZE) {
                request_tx.send(chunk.to_vec()).await.ok();
            }
            Ok(())
        }
        .boxed()
    }

    fn search_commits(
        &self,
        _log_source: LogSource,
        search_args: SearchCommitArgs,
        request_tx: Sender<Oid>,
    ) -> BoxFuture<'_, Result<()>> {
        async move {
            let hash_query = commit_hash_search_query(search_args.query.as_str())
                .map(|query| query.to_ascii_lowercase());
            let message_query = if search_args.case_sensitive {
                search_args.query.to_string()
            } else {
                search_args.query.to_lowercase()
            };

            let matching_shas = self.fs.with_git_state(&self.dot_git_path, false, |state| {
                state
                    .commit_data
                    .iter()
                    .filter_map(|(sha, entry)| {
                        let FakeCommitDataEntry::Success(commit_data) = entry else {
                            return None;
                        };
                        if let Some(hash_query) = hash_query.as_ref() {
                            return sha
                                .to_string()
                                .to_ascii_lowercase()
                                .starts_with(hash_query)
                                .then_some(*sha);
                        }
                        let message = if search_args.case_sensitive {
                            commit_data.message.to_string()
                        } else {
                            commit_data.message.to_lowercase()
                        };
                        message.contains(&message_query).then_some(*sha)
                    })
                    .collect::<Vec<_>>()
            })?;

            for sha in matching_shas {
                if request_tx.send(sha).await.is_err() {
                    break;
                }
            }

            Ok(())
        }
        .boxed()
    }

    fn file_history_changed_files(
        &self,
        paths: Vec<RepoPath>,
        _commit_limit: usize,
    ) -> BoxFuture<'_, Result<Vec<FileHistoryChangedFileSets>>> {
        async move { Ok(vec![FileHistoryChangedFileSets::default(); paths.len()]) }.boxed()
    }

    fn commit_data_reader(&self) -> Result<CommitDataReader> {
        let fs = self.fs.clone();
        let dot_git_path = self.dot_git_path.clone();
        let executor = self.executor.clone();
        Ok(CommitDataReader::for_test(executor, move |sha| {
            fs.with_git_state(&dot_git_path, false, |state| {
                let commit = state
                    .commit_data
                    .get(&sha)
                    .context(format!("graph commit data not found for {sha}"))?;

                match commit {
                    FakeCommitDataEntry::Success(data) => Ok(data.clone()),
                    FakeCommitDataEntry::Fail(_) => {
                        bail!("simulated commit data read failure for {sha}")
                    }
                }
            })?
        }))
    }

    fn update_ref(&self, ref_name: String, commit: String) -> BoxFuture<'_, Result<()>> {
        self.edit_ref(RefEdit::Update { ref_name, commit })
    }

    fn delete_ref(&self, ref_name: String) -> BoxFuture<'_, Result<()>> {
        self.edit_ref(RefEdit::Delete { ref_name })
    }

    fn repair_worktrees(&self) -> BoxFuture<'_, Result<()>> {
        async { Ok(()) }.boxed()
    }

    fn set_trusted(&self, trusted: bool) {
        self.is_trusted
            .store(trusted, std::sync::atomic::Ordering::Release);
    }

    fn is_trusted(&self) -> bool {
        self.is_trusted.load(std::sync::atomic::Ordering::Acquire)
    }
}
