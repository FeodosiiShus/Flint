use collections::HashMap;
use fs::{FakeFs, FakeGitOperation, Fs};
use git::{
    repository::{
        AskPassDelegate, Branch, CommitSummary, GitFailure, GitFailureKind, GitRepository,
        MergeOutcome, RebaseAction, RebaseOutcome, RepositoryOperation, UpstreamTracking,
        UpstreamTrackingStatus, repo_path,
    },
    status::{FileStatus, StatusCode, TrackedStatus, UnmergedStatus, UnmergedStatusCode},
};
use gpui::{BackgroundExecutor, TestAppContext};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use util::path;

#[gpui::test]
async fn test_fake_worktree_lifecycle(cx: &mut TestAppContext) {
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/project", json!({".git": {}, "file.txt": "content"}))
        .await;
    let repo = fs
        .open_repo(Path::new("/project/.git"), None)
        .expect("should open fake repo");

    // Initially only the main worktree exists
    let worktrees = repo.worktrees().await.unwrap();
    assert_eq!(worktrees.len(), 1);
    assert_eq!(worktrees[0].path, PathBuf::from("/project"));

    fs.create_dir("/my-worktrees".as_ref()).await.unwrap();
    let worktrees_dir = Path::new("/my-worktrees");

    // Create a worktree
    let worktree_1_dir = worktrees_dir.join("feature-branch");
    repo.create_worktree(
        git::repository::CreateWorktreeTarget::NewBranch {
            branch_name: "feature-branch".to_string(),
            base_sha: Some("abc123".to_string()),
        },
        worktree_1_dir.clone(),
    )
    .await
    .unwrap();

    // List worktrees — should have main + one created
    let worktrees = repo.worktrees().await.unwrap();
    assert_eq!(worktrees.len(), 2);
    assert_eq!(worktrees[0].path, PathBuf::from("/project"));
    assert_eq!(worktrees[1].path, worktree_1_dir);
    assert_eq!(
        worktrees[1].ref_name,
        Some("refs/heads/feature-branch".into())
    );
    assert_eq!(worktrees[1].sha.as_ref(), "abc123");

    // Directory should exist in FakeFs after create
    assert!(fs.is_dir(&worktrees_dir.join("feature-branch")).await);

    // Create a second worktree (without explicit commit)
    let worktree_2_dir = worktrees_dir.join("bugfix-branch");
    repo.create_worktree(
        git::repository::CreateWorktreeTarget::NewBranch {
            branch_name: "bugfix-branch".to_string(),
            base_sha: None,
        },
        worktree_2_dir.clone(),
    )
    .await
    .unwrap();

    let worktrees = repo.worktrees().await.unwrap();
    assert_eq!(worktrees.len(), 3);
    assert!(fs.is_dir(&worktree_2_dir).await);

    // Rename the first worktree
    repo.rename_worktree(worktree_1_dir, worktrees_dir.join("renamed-branch"))
        .await
        .unwrap();

    let worktrees = repo.worktrees().await.unwrap();
    assert_eq!(worktrees.len(), 3);
    assert!(
        worktrees
            .iter()
            .any(|w| w.path == worktrees_dir.join("renamed-branch")),
    );
    assert!(
        worktrees
            .iter()
            .all(|w| w.path != worktrees_dir.join("feature-branch")),
    );

    // Directory should be moved in FakeFs after rename
    assert!(!fs.is_dir(&worktrees_dir.join("feature-branch")).await);
    assert!(fs.is_dir(&worktrees_dir.join("renamed-branch")).await);

    // Rename a nonexistent worktree should fail
    let result = repo
        .rename_worktree(PathBuf::from("/nonexistent"), PathBuf::from("/somewhere"))
        .await;
    assert!(result.is_err());

    // Remove a worktree
    repo.remove_worktree(worktrees_dir.join("renamed-branch"), false)
        .await
        .unwrap();

    let worktrees = repo.worktrees().await.unwrap();
    assert_eq!(worktrees.len(), 2);
    assert_eq!(worktrees[0].path, PathBuf::from("/project"));
    assert_eq!(worktrees[1].path, worktree_2_dir);

    // Directory should be removed from FakeFs after remove
    assert!(!fs.is_dir(&worktrees_dir.join("renamed-branch")).await);

    // Remove a nonexistent worktree should fail
    let result = repo
        .remove_worktree(PathBuf::from("/nonexistent"), false)
        .await;
    assert!(result.is_err());

    // Remove the last worktree
    repo.remove_worktree(worktree_2_dir.clone(), false)
        .await
        .unwrap();

    let worktrees = repo.worktrees().await.unwrap();
    assert_eq!(worktrees.len(), 1);
    assert_eq!(worktrees[0].path, PathBuf::from("/project"));
    assert!(!fs.is_dir(&worktree_2_dir).await);
}

#[gpui::test]
async fn test_checkpoints(executor: BackgroundExecutor) {
    let fs = FakeFs::new(executor);
    fs.insert_tree(
        path!("/"),
        json!({
            "bar": {
                "baz": "qux"
            },
            "foo": {
                ".git": {},
                "a": "lorem",
                "b": "ipsum",
            },
        }),
    )
    .await;
    fs.with_git_state(Path::new("/foo/.git"), true, |_git| {})
        .unwrap();
    let repository = fs
        .open_repo(Path::new("/foo/.git"), Some("git".as_ref()))
        .unwrap();

    let checkpoint_1 = repository.checkpoint().await.unwrap();
    fs.write(Path::new("/foo/b"), b"IPSUM").await.unwrap();
    fs.write(Path::new("/foo/c"), b"dolor").await.unwrap();
    let checkpoint_2 = repository.checkpoint().await.unwrap();
    let checkpoint_3 = repository.checkpoint().await.unwrap();

    assert!(
        repository
            .compare_checkpoints(checkpoint_2.clone(), checkpoint_3.clone())
            .await
            .unwrap()
    );
    assert!(
        !repository
            .compare_checkpoints(checkpoint_1.clone(), checkpoint_2.clone())
            .await
            .unwrap()
    );

    repository
        .restore_checkpoint(checkpoint_1.clone())
        .await
        .unwrap();
    assert_eq!(
        fs.files_with_contents(Path::new("")),
        [
            (Path::new(path!("/bar/baz")).into(), b"qux".into()),
            (Path::new(path!("/foo/a")).into(), b"lorem".into()),
            (Path::new(path!("/foo/b")).into(), b"ipsum".into())
        ]
    );

    // diff_checkpoints: identical checkpoints produce empty diff
    let diff = repository
        .diff_checkpoints(checkpoint_2.clone(), checkpoint_3.clone())
        .await
        .unwrap();
    assert!(
        diff.is_empty(),
        "identical checkpoints should produce empty diff"
    );

    // diff_checkpoints: different checkpoints produce non-empty diff
    let diff = repository
        .diff_checkpoints(checkpoint_1.clone(), checkpoint_2.clone())
        .await
        .unwrap();
    assert!(diff.contains("b"), "diff should mention changed file 'b'");
    assert!(diff.contains("c"), "diff should mention added file 'c'");
}

#[gpui::test]
async fn test_fake_conflict_stages(executor: BackgroundExecutor) {
    let fs = FakeFs::new(executor);
    fs.insert_tree(
        path!("/project"),
        json!({
            ".git": {},
            "both_modified.txt": "stale",
            "deleted_by_us.txt": "stale",
        }),
    )
    .await;
    let dot_git = Path::new(path!("/project/.git"));
    fs.set_conflict_for_repo(
        dot_git,
        "both_modified.txt",
        Some("base\n"),
        Some("ours\n"),
        Some("theirs\n"),
    );
    fs.set_conflict_for_repo(
        dot_git,
        "deleted_by_us.txt",
        Some("base\n"),
        None,
        Some("theirs\n"),
    );
    fs.set_conflict_for_repo(
        dot_git,
        "both_added.txt",
        None,
        Some("ours\n"),
        Some("theirs\n"),
    );
    fs.set_merge_message_for_repo(dot_git, Some("Merge branch 'feature'"));
    let repository = fs.open_repo(dot_git, None).unwrap();

    let revisions = repository
        .load_revisions(
            [
                ":1:both_modified.txt",
                ":2:both_modified.txt",
                ":3:both_modified.txt",
                ":1:deleted_by_us.txt",
                ":2:deleted_by_us.txt",
                ":3:deleted_by_us.txt",
                ":1:both_added.txt",
                ":both_modified.txt",
                "HEAD:both_modified.txt",
                ":2:missing.txt",
            ]
            .map(String::from)
            .to_vec(),
        )
        .await
        .unwrap();
    assert_eq!(
        revisions,
        vec![
            Some(b"base\n".to_vec()),
            Some(b"ours\n".to_vec()),
            Some(b"theirs\n".to_vec()),
            Some(b"base\n".to_vec()),
            None,
            Some(b"theirs\n".to_vec()),
            None,
            None,
            Some(b"ours\n".to_vec()),
            None,
        ]
    );

    assert_eq!(
        fs.load(Path::new(path!("/project/both_modified.txt")))
            .await
            .unwrap(),
        "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> MERGE_HEAD\n"
    );
    assert_eq!(
        fs.load(Path::new(path!("/project/deleted_by_us.txt")))
            .await
            .unwrap(),
        "<<<<<<< HEAD\n=======\ntheirs\n>>>>>>> MERGE_HEAD\n"
    );

    let status = repository
        .status(&[
            repo_path("both_added.txt"),
            repo_path("both_modified.txt"),
            repo_path("deleted_by_us.txt"),
        ])
        .await
        .unwrap();
    assert_eq!(
        status.entries.to_vec(),
        vec![
            (
                repo_path("both_added.txt"),
                FileStatus::Unmerged(UnmergedStatus {
                    first_head: UnmergedStatusCode::Added,
                    second_head: UnmergedStatusCode::Added,
                })
            ),
            (
                repo_path("both_modified.txt"),
                FileStatus::Unmerged(UnmergedStatus {
                    first_head: UnmergedStatusCode::Updated,
                    second_head: UnmergedStatusCode::Updated,
                })
            ),
            (
                repo_path("deleted_by_us.txt"),
                FileStatus::Unmerged(UnmergedStatus {
                    first_head: UnmergedStatusCode::Deleted,
                    second_head: UnmergedStatusCode::Updated,
                })
            ),
        ]
    );

    assert!(
        repository
            .revparse_batch(vec!["MERGE_HEAD".to_string()])
            .await
            .unwrap()
            .first()
            .is_some_and(Option::is_some),
        "recording a conflict should start a merge"
    );
    assert_eq!(
        repository.merge_message().await.as_deref(),
        Some("Merge branch 'feature'")
    );
    fs.set_merge_message_for_repo(dot_git, None);
    assert_eq!(repository.merge_message().await, None);
}

#[gpui::test]
async fn test_fake_stage_paths_resolves_only_recorded_conflicts(executor: BackgroundExecutor) {
    let fs = FakeFs::new(executor);
    fs.insert_tree(
        path!("/project"),
        json!({
            ".git": {},
            "recorded.txt": "stale",
            "legacy.txt": "legacy",
            "both_deleted.txt": "stale",
        }),
    )
    .await;
    let dot_git = Path::new(path!("/project/.git"));
    fs.set_conflict_for_repo(
        dot_git,
        "recorded.txt",
        Some("base\n"),
        Some("ours\n"),
        Some("theirs\n"),
    );
    fs.set_conflict_for_repo(dot_git, "both_deleted.txt", Some("base\n"), None, None);
    let legacy_status = UnmergedStatus {
        first_head: UnmergedStatusCode::Updated,
        second_head: UnmergedStatusCode::Updated,
    };
    fs.with_git_state(dot_git, true, |state| {
        state
            .unmerged_paths
            .insert(repo_path("legacy.txt"), legacy_status);
    })
    .unwrap();
    assert!(
        !fs.is_file(Path::new(path!("/project/both_deleted.txt")))
            .await,
        "a conflict without ours and theirs leaves no worktree file"
    );

    let repository = fs.open_repo(dot_git, None).unwrap();
    let paths = vec![
        repo_path("both_deleted.txt"),
        repo_path("legacy.txt"),
        repo_path("recorded.txt"),
    ];
    let status = repository.status(&paths).await.unwrap();
    assert_eq!(
        status.entries.first(),
        Some(&(
            repo_path("both_deleted.txt"),
            FileStatus::Unmerged(UnmergedStatus {
                first_head: UnmergedStatusCode::Deleted,
                second_head: UnmergedStatusCode::Deleted,
            })
        ))
    );

    fs.write(Path::new(path!("/project/recorded.txt")), b"resolved\n")
        .await
        .unwrap();
    repository
        .stage_paths(paths.clone(), Arc::new(HashMap::default()))
        .await
        .unwrap();

    let status = repository.status(&paths).await.unwrap();
    assert_eq!(
        status.entries.to_vec(),
        vec![
            (repo_path("legacy.txt"), FileStatus::Unmerged(legacy_status)),
            (
                repo_path("recorded.txt"),
                FileStatus::Tracked(TrackedStatus {
                    index_status: StatusCode::Modified,
                    worktree_status: StatusCode::Unmodified,
                })
            ),
        ]
    );
    assert_eq!(
        repository
            .load_revisions(vec![
                ":recorded.txt".to_string(),
                ":both_deleted.txt".to_string()
            ])
            .await
            .unwrap(),
        vec![Some(b"resolved\n".to_vec()), None]
    );
}

#[gpui::test]
async fn test_fake_unresolve_paths_restores_conflict(executor: BackgroundExecutor) {
    let fs = FakeFs::new(executor);
    fs.insert_tree(
        path!("/project"),
        json!({
            ".git": {},
            "recorded.txt": "stale",
            "plain.txt": "plain",
        }),
    )
    .await;
    let dot_git = Path::new(path!("/project/.git"));
    fs.set_conflict_for_repo(
        dot_git,
        "recorded.txt",
        Some("base\n"),
        Some("ours\n"),
        None,
    );
    let repository = fs.open_repo(dot_git, None).unwrap();
    let recorded_path = vec![repo_path("recorded.txt")];

    fs.write(Path::new(path!("/project/recorded.txt")), b"resolved\n")
        .await
        .unwrap();
    repository
        .stage_paths(recorded_path.clone(), Arc::new(HashMap::default()))
        .await
        .unwrap();
    let status = repository.status(&recorded_path).await.unwrap();
    assert!(
        status
            .entries
            .iter()
            .all(|(_, status)| !status.is_conflicted()),
        "staging should resolve the recorded conflict, got {:?}",
        status.entries
    );

    repository
        .unresolve_paths(recorded_path.clone(), Arc::new(HashMap::default()))
        .await
        .unwrap();
    let status = repository.status(&recorded_path).await.unwrap();
    assert_eq!(
        status.entries.to_vec(),
        vec![(
            repo_path("recorded.txt"),
            FileStatus::Unmerged(UnmergedStatus {
                first_head: UnmergedStatusCode::Updated,
                second_head: UnmergedStatusCode::Deleted,
            })
        )]
    );
    assert_eq!(
        fs.load(Path::new(path!("/project/recorded.txt")))
            .await
            .unwrap(),
        "<<<<<<< HEAD\nours\n=======\n>>>>>>> MERGE_HEAD\n"
    );
    assert_eq!(
        repository
            .load_revisions(vec![
                ":recorded.txt".to_string(),
                ":2:recorded.txt".to_string()
            ])
            .await
            .unwrap(),
        vec![None, Some(b"ours\n".to_vec())]
    );

    let error = repository
        .unresolve_paths(vec![repo_path("plain.txt")], Arc::new(HashMap::default()))
        .await
        .expect_err("unresolving a path without a recorded conflict should fail");
    assert!(
        error.to_string().contains("has no recorded conflict"),
        "unexpected error: {error}"
    );
    assert_eq!(
        fs.load(Path::new(path!("/project/plain.txt")))
            .await
            .unwrap(),
        "plain"
    );
}

fn dot_git() -> &'static Path {
    Path::new(path!("/project/.git"))
}

fn no_env() -> Arc<HashMap<String, String>> {
    Arc::new(HashMap::default())
}

fn askpass(cx: &mut TestAppContext) -> AskPassDelegate {
    AskPassDelegate::new(&mut cx.to_async(), |_, _, _| {})
}

fn tracked(ahead: u32, behind: u32) -> UpstreamTracking {
    UpstreamTracking::Tracked(UpstreamTrackingStatus { ahead, behind })
}

fn summary(sha: &str) -> CommitSummary {
    CommitSummary {
        sha: sha.to_string().into(),
        subject: format!("commit {sha}").into(),
        commit_timestamp: 0,
        author_name: "author".into(),
        has_parent: true,
    }
}

fn history(shas: &[&str]) -> Vec<CommitSummary> {
    shas.iter().map(|sha| summary(sha)).collect()
}

fn shas(commits: &[CommitSummary]) -> Vec<String> {
    commits
        .iter()
        .map(|commit| commit.sha.to_string())
        .collect()
}

fn failure(error: &anyhow::Error) -> &GitFailure {
    error
        .downcast_ref::<GitFailure>()
        .expect("error should be a GitFailure")
}

async fn open_fake_repository(
    executor: BackgroundExecutor,
) -> (Arc<FakeFs>, Arc<dyn GitRepository>) {
    let fs = FakeFs::new(executor);
    fs.insert_tree(
        path!("/project"),
        json!({
            ".git": {},
            "file.txt": "content",
            "conflicted.txt": "stale",
        }),
    )
    .await;
    let repository = fs.open_repo(dot_git(), None).unwrap();
    (fs, repository)
}

async fn branch_list(repository: &Arc<dyn GitRepository>) -> Vec<Branch> {
    repository.branches().await.unwrap().branches
}

async fn find_branch(repository: &Arc<dyn GitRepository>, ref_name: &str) -> Branch {
    branch_list(repository)
        .await
        .into_iter()
        .find(|branch| &*branch.ref_name == ref_name)
        .unwrap_or_else(|| panic!("branch {ref_name} should be listed"))
}

async fn has_branch(repository: &Arc<dyn GitRepository>, ref_name: &str) -> bool {
    branch_list(repository)
        .await
        .iter()
        .any(|branch| &*branch.ref_name == ref_name)
}

async fn current_branch_name(repository: &Arc<dyn GitRepository>) -> Option<String> {
    branch_list(repository)
        .await
        .into_iter()
        .find(|branch| branch.is_head)
        .map(|branch| branch.name().to_string())
}

async fn has_conflicts(repository: &Arc<dyn GitRepository>) -> bool {
    let status = repository.status(&[repo_path("")]).await.unwrap();
    status
        .entries
        .iter()
        .any(|(_, status)| status.is_conflicted())
}

async fn tag_entries(repository: &Arc<dyn GitRepository>) -> Vec<String> {
    repository
        .tags()
        .await
        .unwrap()
        .into_iter()
        .map(|tag| format!("{}={}", tag.name, tag.commit_sha))
        .collect()
}

async fn recent(repository: &Arc<dyn GitRepository>, limit: usize) -> Vec<String> {
    repository
        .recent_branches(limit)
        .await
        .unwrap()
        .into_iter()
        .map(|name| name.to_string())
        .collect()
}

async fn tip_of(repository: &Arc<dyn GitRepository>, revision: &str) -> Option<String> {
    repository
        .revparse_batch(vec![revision.to_string()])
        .await
        .unwrap()
        .into_iter()
        .next()
        .flatten()
}

async fn commits_between(
    repository: &Arc<dyn GitRepository>,
    base: &str,
    head: &str,
) -> Vec<String> {
    shas(
        &repository
            .commits_between(base.to_string(), head.to_string(), 100)
            .await
            .unwrap(),
    )
}

#[gpui::test]
async fn test_fake_tags_are_listed_newest_first_and_created_tags_are_prepended(
    executor: BackgroundExecutor,
) {
    let (fs, repository) = open_fake_repository(executor).await;
    fs.insert_tags(dot_git(), &[("v2.0", "sha-v2"), ("v1.0", "sha-v1")]);
    assert_eq!(
        tag_entries(&repository).await,
        ["v2.0=sha-v2", "v1.0=sha-v1"]
    );

    repository
        .create_tag("v3.0".into(), "v1.0".into())
        .await
        .unwrap();
    repository
        .create_tag("at-head".into(), "HEAD".into())
        .await
        .unwrap();
    assert_eq!(
        tag_entries(&repository).await,
        ["at-head=abc", "v3.0=sha-v1", "v2.0=sha-v2", "v1.0=sha-v1"]
    );

    let duplicate = repository
        .create_tag("v1.0".into(), "HEAD".into())
        .await
        .unwrap_err();
    assert_eq!(failure(&duplicate).kind, GitFailureKind::Other);
    assert!(
        duplicate.to_string().contains("already exists"),
        "unexpected error: {duplicate}"
    );
    let unresolved = repository
        .create_tag("ghost".into(), "missing".into())
        .await
        .unwrap_err();
    assert!(
        unresolved.to_string().contains("Failed to resolve"),
        "unexpected error: {unresolved}"
    );
    assert_eq!(tag_entries(&repository).await.len(), 4);

    repository.delete_tag("v2.0".into()).await.unwrap();
    assert_eq!(
        tag_entries(&repository).await,
        ["at-head=abc", "v3.0=sha-v1", "v1.0=sha-v1"]
    );
    let missing = repository.delete_tag("v2.0".into()).await.unwrap_err();
    assert!(
        missing.to_string().contains("not found"),
        "unexpected error: {missing}"
    );
}

#[gpui::test]
async fn test_fake_recent_branches_follow_checkouts_and_skip_deleted_branches(
    executor: BackgroundExecutor,
) {
    let (fs, repository) = open_fake_repository(executor).await;
    fs.insert_branches(dot_git(), &["main", "dev", "topic"]);
    assert_eq!(recent(&repository, 10).await, Vec::<String>::new());

    for name in ["dev", "topic", "main", "dev", "dev"] {
        repository.change_branch(name.to_string()).await.unwrap();
    }
    assert_eq!(recent(&repository, 10).await, ["dev", "main", "topic"]);
    assert_eq!(recent(&repository, 2).await, ["dev", "main"]);
    assert_eq!(recent(&repository, 0).await, Vec::<String>::new());

    repository
        .delete_branch(false, "topic".into(), false)
        .await
        .unwrap();
    assert_eq!(recent(&repository, 10).await, ["dev", "main"]);

    fs.set_recent_branches_for_repo(dot_git(), &["ghost", "main", "main", "dev"]);
    assert_eq!(recent(&repository, 10).await, ["main", "dev"]);
}

#[gpui::test]
async fn test_fake_branches_report_upstream_and_head_for_slashed_local_branches(
    executor: BackgroundExecutor,
) {
    let (fs, repository) = open_fake_repository(executor).await;
    fs.insert_branches(
        dot_git(),
        &[
            "main",
            "refs/heads/feature/x",
            "origin/main",
            "origin/other",
        ],
    );
    fs.set_upstream_for_repo(dot_git(), "main", "origin/main", tracked(2, 1));

    let main = find_branch(&repository, "refs/heads/main").await;
    let upstream = main.upstream.expect("main should have an upstream");
    assert_eq!(&*upstream.ref_name, "refs/remotes/origin/main");
    assert_eq!(upstream.tracking, tracked(2, 1));
    assert!(main.is_head);
    assert_eq!(
        find_branch(&repository, "refs/remotes/origin/main")
            .await
            .upstream,
        None
    );
    assert_eq!(
        find_branch(&repository, "refs/heads/feature/x")
            .await
            .upstream,
        None
    );

    repository
        .change_branch("feature/x".to_string())
        .await
        .unwrap();
    assert_eq!(
        current_branch_name(&repository).await.as_deref(),
        Some("feature/x")
    );
    assert!(!find_branch(&repository, "refs/heads/main").await.is_head);

    repository
        .set_upstream("feature/x".into(), Some("origin/other".into()))
        .await
        .unwrap();
    let feature = find_branch(&repository, "refs/heads/feature/x").await;
    let upstream = feature
        .upstream
        .expect("set_upstream should record upstream");
    assert_eq!(&*upstream.ref_name, "refs/remotes/origin/other");
    assert_eq!(upstream.tracking, tracked(0, 0));

    repository.set_upstream("main".into(), None).await.unwrap();
    assert_eq!(
        find_branch(&repository, "refs/heads/main").await.upstream,
        None
    );
    repository.set_upstream("main".into(), None).await.unwrap();

    let unknown_branch = repository
        .set_upstream("nope".into(), Some("origin/main".into()))
        .await
        .unwrap_err();
    assert!(
        unknown_branch.to_string().contains("does not exist"),
        "unexpected error: {unknown_branch}"
    );
    let unknown_upstream = repository
        .set_upstream("main".into(), Some("origin/missing".into()))
        .await
        .unwrap_err();
    assert!(
        unknown_upstream
            .to_string()
            .contains("requested upstream branch"),
        "unexpected error: {unknown_upstream}"
    );

    fs.clear_upstream_for_repo(dot_git(), "feature/x");
    assert_eq!(
        find_branch(&repository, "refs/heads/feature/x")
            .await
            .upstream,
        None
    );
}

#[gpui::test]
async fn test_fake_commits_between_lists_commits_reachable_only_from_head(
    executor: BackgroundExecutor,
) {
    let (fs, repository) = open_fake_repository(executor).await;
    fs.insert_branches(dot_git(), &["main", "feature", "origin/main"]);
    fs.set_ref_history_for_repo(dot_git(), "main", history(&["c3", "c2", "c1"]));
    fs.set_ref_history_for_repo(dot_git(), "feature", history(&["f2", "f1", "c2", "c1"]));
    fs.set_ref_history_for_repo(
        dot_git(),
        "refs/remotes/origin/main",
        history(&["c2", "c1"]),
    );

    assert_eq!(
        commits_between(&repository, "main", "feature").await,
        ["f2", "f1"]
    );
    assert_eq!(
        commits_between(&repository, "feature", "main").await,
        ["c3"]
    );
    assert_eq!(
        commits_between(&repository, "origin/main", "main").await,
        ["c3"]
    );
    assert_eq!(
        commits_between(&repository, "refs/heads/main", "refs/heads/feature").await,
        ["f2", "f1"]
    );
    assert_eq!(
        commits_between(&repository, "main", "HEAD").await,
        Vec::<String>::new()
    );
    assert_eq!(
        commits_between(&repository, "feature", "feature").await,
        Vec::<String>::new()
    );

    let limited = repository
        .commits_between("main".into(), "feature".into(), 1)
        .await
        .unwrap();
    assert_eq!(shas(&limited), ["f2"]);
    assert!(limited[0].has_parent);

    let error = repository
        .commits_between("main".into(), "unknown".into(), 10)
        .await
        .unwrap_err();
    assert_eq!(failure(&error).kind, GitFailureKind::RevisionNotFound);
}

#[gpui::test]
async fn test_fake_operation_in_progress_reflects_markers(executor: BackgroundExecutor) {
    let (fs, repository) = open_fake_repository(executor).await;
    assert_eq!(repository.operation_in_progress().await.unwrap(), None);

    for operation in [
        RepositoryOperation::Merge,
        RepositoryOperation::Rebase,
        RepositoryOperation::CherryPick,
        RepositoryOperation::Revert,
    ] {
        fs.set_operation_in_progress_for_repo(dot_git(), Some(operation));
        assert_eq!(
            repository.operation_in_progress().await.unwrap(),
            Some(operation)
        );
    }

    fs.set_operation_in_progress_for_repo(dot_git(), None);
    assert_eq!(repository.operation_in_progress().await.unwrap(), None);

    fs.set_conflict_for_repo(
        dot_git(),
        "conflicted.txt",
        Some("base\n"),
        Some("ours\n"),
        Some("theirs\n"),
    );
    assert_eq!(
        repository.operation_in_progress().await.unwrap(),
        Some(RepositoryOperation::Merge)
    );
}

#[gpui::test]
async fn test_fake_checkout_detached_resolves_tags_branches_and_rejects_unknown_revisions(
    executor: BackgroundExecutor,
) {
    let (fs, repository) = open_fake_repository(executor).await;
    fs.insert_branches(dot_git(), &["main", "dev"]);
    fs.insert_tags(dot_git(), &[("v1", "tag-sha")]);
    fs.set_ref_for_repo(dot_git(), "refs/heads/dev", "dev-sha");

    let missing = repository
        .checkout_detached("nope".into())
        .await
        .unwrap_err();
    assert_eq!(failure(&missing).kind, GitFailureKind::RevisionNotFound);
    assert_eq!(
        current_branch_name(&repository).await.as_deref(),
        Some("main")
    );

    repository.checkout_detached("v1".into()).await.unwrap();
    assert_eq!(current_branch_name(&repository).await, None);
    assert_eq!(repository.head_sha().await.as_deref(), Some("tag-sha"));
    assert_eq!(recent(&repository, 10).await, Vec::<String>::new());

    repository
        .checkout_detached("refs/heads/dev".into())
        .await
        .unwrap();
    assert_eq!(repository.head_sha().await.as_deref(), Some("dev-sha"));
    assert_eq!(current_branch_name(&repository).await, None);

    repository.change_branch("main".to_string()).await.unwrap();
    assert_eq!(recent(&repository, 10).await, ["main"]);

    repository.checkout_detached("dev^0".into()).await.unwrap();
    assert_eq!(repository.head_sha().await.as_deref(), Some("dev-sha"));

    fs.set_conflict_for_repo(
        dot_git(),
        "conflicted.txt",
        Some("base\n"),
        Some("ours\n"),
        Some("theirs\n"),
    );
    repository.change_branch("main".to_string()).await.unwrap();
    let unmerged = repository.checkout_detached("v1".into()).await.unwrap_err();
    assert_eq!(failure(&unmerged).kind, GitFailureKind::UnmergedFiles);
    assert_eq!(
        current_branch_name(&repository).await.as_deref(),
        Some("main")
    );
}

#[gpui::test]
async fn test_fake_checkout_force_clears_conflicts_and_ignores_simulated_checkout_failures(
    executor: BackgroundExecutor,
) {
    let (fs, repository) = open_fake_repository(executor).await;
    fs.insert_branches(dot_git(), &["main", "dev"]);
    fs.insert_tags(dot_git(), &[("v1", "tag-sha")]);
    fs.set_conflict_for_repo(
        dot_git(),
        "conflicted.txt",
        Some("base\n"),
        Some("ours\n"),
        Some("theirs\n"),
    );
    assert!(has_conflicts(&repository).await);
    fs.set_simulated_git_failure_for_repo(
        dot_git(),
        FakeGitOperation::Checkout,
        Some((
            GitFailureKind::LocalChangesWouldBeOverwritten {
                files: vec!["file.txt".to_string()],
            },
            "error: Your local changes to the following files would be overwritten by checkout:\n\tfile.txt",
        )),
    );

    let blocked = repository
        .change_branch("dev".to_string())
        .await
        .unwrap_err();
    assert_eq!(
        failure(&blocked).kind,
        GitFailureKind::LocalChangesWouldBeOverwritten {
            files: vec!["file.txt".to_string()]
        }
    );
    assert_eq!(
        current_branch_name(&repository).await.as_deref(),
        Some("main")
    );

    repository.checkout_force("dev".into()).await.unwrap();
    assert_eq!(
        current_branch_name(&repository).await.as_deref(),
        Some("dev")
    );
    assert_eq!(repository.operation_in_progress().await.unwrap(), None);
    assert!(!has_conflicts(&repository).await);

    repository.checkout_force("v1".into()).await.unwrap();
    assert_eq!(current_branch_name(&repository).await, None);
    assert_eq!(repository.head_sha().await.as_deref(), Some("tag-sha"));

    repository.checkout_force("dev^0".into()).await.unwrap();
    assert_eq!(current_branch_name(&repository).await, None);

    let unknown = repository.checkout_force("nope".into()).await.unwrap_err();
    assert_eq!(failure(&unknown).kind, GitFailureKind::Other);
    assert!(
        unknown.to_string().contains("did not match"),
        "unexpected error: {unknown}"
    );

    fs.set_simulated_git_failure_for_repo(dot_git(), FakeGitOperation::Checkout, None);
    repository.change_branch("main".to_string()).await.unwrap();
    assert_eq!(
        current_branch_name(&repository).await.as_deref(),
        Some("main")
    );
}

#[gpui::test]
async fn test_fake_create_branch_at_supports_every_checkout_and_overwrite_combination(
    executor: BackgroundExecutor,
) {
    let (fs, repository) = open_fake_repository(executor).await;
    fs.insert_branches(dot_git(), &["main", "origin/dev"]);
    fs.insert_tags(dot_git(), &[("v1", "tag-sha")]);
    fs.set_ref_for_repo(dot_git(), "HEAD", "head-sha");
    fs.set_ref_for_repo(dot_git(), "refs/remotes/origin/dev", "dev-sha");

    repository
        .create_branch_at("topic".into(), "main".into(), true, false)
        .await
        .unwrap();
    assert_eq!(
        current_branch_name(&repository).await.as_deref(),
        Some("topic")
    );
    assert_eq!(repository.head_sha().await.as_deref(), Some("head-sha"));
    assert_eq!(recent(&repository, 10).await, ["topic"]);

    repository
        .create_branch_at("side".into(), "main".into(), false, false)
        .await
        .unwrap();
    assert!(has_branch(&repository, "refs/heads/side").await);
    assert_eq!(
        current_branch_name(&repository).await.as_deref(),
        Some("topic")
    );
    assert_eq!(
        tip_of(&repository, "refs/heads/side").await.as_deref(),
        Some("head-sha")
    );

    let duplicate = repository
        .create_branch_at("side".into(), "v1".into(), false, false)
        .await
        .unwrap_err();
    assert!(
        duplicate.to_string().contains("already exists"),
        "unexpected error: {duplicate}"
    );
    assert_eq!(
        tip_of(&repository, "refs/heads/side").await.as_deref(),
        Some("head-sha")
    );

    repository
        .create_branch_at("side".into(), "v1".into(), false, true)
        .await
        .unwrap();
    assert_eq!(
        tip_of(&repository, "refs/heads/side").await.as_deref(),
        Some("tag-sha")
    );
    assert_eq!(
        current_branch_name(&repository).await.as_deref(),
        Some("topic")
    );

    repository
        .create_branch_at("topic".into(), "v1".into(), false, true)
        .await
        .unwrap();
    assert_eq!(repository.head_sha().await.as_deref(), Some("tag-sha"));
    assert_eq!(
        current_branch_name(&repository).await.as_deref(),
        Some("topic")
    );

    repository
        .create_branch_at("side".into(), "main".into(), true, true)
        .await
        .unwrap();
    assert_eq!(
        current_branch_name(&repository).await.as_deref(),
        Some("side")
    );
    assert_eq!(repository.head_sha().await.as_deref(), Some("head-sha"));

    repository
        .create_branch_at("dev".into(), "origin/dev".into(), false, false)
        .await
        .unwrap();
    let dev = find_branch(&repository, "refs/heads/dev").await;
    let upstream = dev
        .upstream
        .expect("branching from a remote branch should track it");
    assert_eq!(&*upstream.ref_name, "refs/remotes/origin/dev");
    assert_eq!(upstream.tracking, tracked(0, 0));
    assert_eq!(
        tip_of(&repository, "refs/heads/dev").await.as_deref(),
        Some("dev-sha")
    );

    let invalid_start = repository
        .create_branch_at("ghost".into(), "missing".into(), true, false)
        .await
        .unwrap_err();
    assert!(
        invalid_start
            .to_string()
            .contains("not a valid object name"),
        "unexpected error: {invalid_start}"
    );
    assert!(!has_branch(&repository, "refs/heads/ghost").await);
    assert_eq!(
        current_branch_name(&repository).await.as_deref(),
        Some("side")
    );
}

#[gpui::test]
async fn test_fake_merge_reports_up_to_date_fast_forward_and_merge_commit(
    executor: BackgroundExecutor,
) {
    let (fs, repository) = open_fake_repository(executor).await;
    fs.insert_branches(dot_git(), &["main", "feature", "ahead"]);
    fs.set_ref_for_repo(dot_git(), "HEAD", "main-sha");
    fs.set_ref_history_for_repo(dot_git(), "main", history(&["m2", "m1"]));
    fs.set_ref_history_for_repo(dot_git(), "feature", history(&["f1", "m1"]));
    fs.set_ref_history_for_repo(dot_git(), "ahead", history(&["a1", "m2", "m1"]));
    fs.set_ref_for_repo(dot_git(), "refs/heads/ahead", "ahead-sha");

    let outcome = repository.merge("feature".into(), no_env()).await.unwrap();
    assert!(
        matches!(&outcome, MergeOutcome::Merged { output } if output.contains("Merge made")),
        "unexpected outcome: {outcome:?}"
    );
    assert_eq!(
        repository.head_sha().await.as_deref(),
        Some("fake-merge-feature-2")
    );
    let new_commits = repository
        .commits_between("feature".into(), "HEAD".into(), 10)
        .await
        .unwrap();
    assert_eq!(shas(&new_commits), ["fake-merge-feature-2", "m2"]);
    assert_eq!(&*new_commits[0].subject, "Merge branch 'feature'");

    assert_eq!(
        repository.merge("feature".into(), no_env()).await.unwrap(),
        MergeOutcome::AlreadyUpToDate
    );

    fs.set_ref_for_repo(dot_git(), "HEAD", "main-sha");
    fs.set_ref_history_for_repo(dot_git(), "main", history(&["m2", "m1"]));
    let outcome = repository.merge("ahead".into(), no_env()).await.unwrap();
    assert!(
        matches!(&outcome, MergeOutcome::Merged { output } if output.contains("Fast-forward")),
        "unexpected outcome: {outcome:?}"
    );
    assert_eq!(repository.head_sha().await.as_deref(), Some("ahead-sha"));
    assert_eq!(
        commits_between(&repository, "ahead", "HEAD").await,
        Vec::<String>::new()
    );

    let unknown = repository
        .merge("missing".into(), no_env())
        .await
        .unwrap_err();
    assert_eq!(failure(&unknown).kind, GitFailureKind::Other);
}

#[gpui::test]
async fn test_fake_merge_conflict_records_state_until_aborted(executor: BackgroundExecutor) {
    let (fs, repository) = open_fake_repository(executor).await;
    fs.insert_branches(dot_git(), &["main", "feature"]);
    fs.set_ref_history_for_repo(dot_git(), "feature", history(&["f1"]));
    fs.set_merge_conflict_for_repo(
        dot_git(),
        "feature",
        "conflicted.txt",
        Some("base\n"),
        Some("ours\n"),
        Some("theirs\n"),
    );

    let outcome = repository.merge("feature".into(), no_env()).await.unwrap();
    let output = match outcome {
        MergeOutcome::Conflicted { output } => output,
        other => panic!("expected a conflict, got {other:?}"),
    };
    assert!(
        output.contains("CONFLICT (content): Merge conflict in conflicted.txt"),
        "unexpected output: {output}"
    );
    assert_eq!(
        repository.operation_in_progress().await.unwrap(),
        Some(RepositoryOperation::Merge)
    );
    assert!(has_conflicts(&repository).await);
    assert!(
        repository
            .merge_message()
            .await
            .is_some_and(|message| message.starts_with("Merge branch 'feature'")),
    );
    assert_eq!(
        fs.load(Path::new(path!("/project/conflicted.txt")))
            .await
            .unwrap(),
        "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> MERGE_HEAD\n"
    );

    let blocked = repository
        .merge("feature".into(), no_env())
        .await
        .unwrap_err();
    assert_eq!(failure(&blocked).kind, GitFailureKind::UnmergedFiles);

    repository.merge_abort(no_env()).await.unwrap();
    assert_eq!(repository.operation_in_progress().await.unwrap(), None);
    assert!(!has_conflicts(&repository).await);
    assert_eq!(repository.merge_message().await, None);

    let nothing_to_abort = repository.merge_abort(no_env()).await.unwrap_err();
    assert!(
        nothing_to_abort.to_string().contains("no merge to abort"),
        "unexpected error: {nothing_to_abort}"
    );
}

#[gpui::test]
async fn test_fake_merge_surfaces_simulated_failures(executor: BackgroundExecutor) {
    let (fs, repository) = open_fake_repository(executor).await;
    fs.insert_branches(dot_git(), &["main", "feature"]);
    fs.set_simulated_git_failure_for_repo(
        dot_git(),
        FakeGitOperation::Merge,
        Some((
            GitFailureKind::LocalChangesWouldBeOverwritten {
                files: vec!["file.txt".to_string()],
            },
            "error: Your local changes to the following files would be overwritten by merge:\n\tfile.txt",
        )),
    );

    let error = repository
        .merge("feature".into(), no_env())
        .await
        .unwrap_err();
    assert_eq!(
        failure(&error).kind,
        GitFailureKind::LocalChangesWouldBeOverwritten {
            files: vec!["file.txt".to_string()]
        }
    );
    assert!(error.to_string().contains("would be overwritten by merge"));

    fs.set_simulated_git_failure_for_repo(dot_git(), FakeGitOperation::Merge, None);
    repository.merge("feature".into(), no_env()).await.unwrap();
}

#[gpui::test]
async fn test_fake_rebase_replays_unique_commits_onto_upstream(executor: BackgroundExecutor) {
    let (fs, repository) = open_fake_repository(executor).await;
    fs.insert_branches(dot_git(), &["main", "topic"]);
    fs.set_ref_for_repo(dot_git(), "HEAD", "m2");
    fs.set_ref_for_repo(dot_git(), "refs/heads/topic", "t1");
    fs.set_ref_history_for_repo(dot_git(), "main", history(&["m2", "m1"]));
    fs.set_ref_history_for_repo(dot_git(), "topic", history(&["t1", "m1"]));

    let outcome = repository
        .rebase(
            RebaseAction::Start {
                upstream: "main".into(),
                branch: Some("topic".into()),
            },
            no_env(),
        )
        .await
        .unwrap();
    assert!(
        matches!(&outcome, RebaseOutcome::Completed { output } if output.contains("Successfully rebased")),
        "unexpected outcome: {outcome:?}"
    );
    assert_eq!(
        current_branch_name(&repository).await.as_deref(),
        Some("topic")
    );
    assert_eq!(repository.head_sha().await.as_deref(), Some("t1"));
    assert_eq!(commits_between(&repository, "main", "topic").await, ["t1"]);
    assert_eq!(
        commits_between(&repository, "topic", "main").await,
        Vec::<String>::new()
    );
    assert_eq!(repository.operation_in_progress().await.unwrap(), None);

    let outcome = repository
        .rebase(
            RebaseAction::Start {
                upstream: "main".into(),
                branch: Some("topic".into()),
            },
            no_env(),
        )
        .await
        .unwrap();
    assert!(
        matches!(&outcome, RebaseOutcome::Completed { output } if output.contains("up to date")),
        "unexpected outcome: {outcome:?}"
    );

    let unknown_upstream = repository
        .rebase(
            RebaseAction::Start {
                upstream: "missing".into(),
                branch: None,
            },
            no_env(),
        )
        .await
        .unwrap_err();
    assert_eq!(failure(&unknown_upstream).kind, GitFailureKind::Other);

    let no_session = repository
        .rebase(RebaseAction::Continue, no_env())
        .await
        .unwrap_err();
    assert!(
        no_session.to_string().contains("No rebase in progress"),
        "unexpected error: {no_session}"
    );
}

#[gpui::test]
async fn test_fake_rebase_conflict_continue_requires_staged_resolution(
    executor: BackgroundExecutor,
) {
    let (fs, repository) = open_fake_repository(executor).await;
    fs.insert_branches(dot_git(), &["main", "topic"]);
    fs.set_ref_history_for_repo(dot_git(), "main", history(&["m2", "m1"]));
    fs.set_ref_history_for_repo(dot_git(), "topic", history(&["t1", "m1"]));
    fs.set_rebase_conflict_for_repo(
        dot_git(),
        "main",
        "conflicted.txt",
        Some("base\n"),
        Some("ours\n"),
        Some("theirs\n"),
    );

    let start = RebaseAction::Start {
        upstream: "main".into(),
        branch: Some("topic".into()),
    };
    let outcome = repository.rebase(start.clone(), no_env()).await.unwrap();
    assert!(
        matches!(&outcome, RebaseOutcome::Conflicted { output } if output.contains("CONFLICT (content)")),
        "unexpected outcome: {outcome:?}"
    );
    assert_eq!(
        repository.operation_in_progress().await.unwrap(),
        Some(RepositoryOperation::Rebase)
    );
    assert_eq!(current_branch_name(&repository).await, None);
    assert!(has_conflicts(&repository).await);

    let already_running = repository.rebase(start, no_env()).await.unwrap_err();
    assert!(
        already_running.to_string().contains("already a rebase"),
        "unexpected error: {already_running}"
    );

    let unresolved = repository
        .rebase(RebaseAction::Continue, no_env())
        .await
        .unwrap_err();
    assert_eq!(failure(&unresolved).kind, GitFailureKind::UnmergedFiles);
    assert_eq!(
        repository.operation_in_progress().await.unwrap(),
        Some(RepositoryOperation::Rebase)
    );

    repository
        .stage_paths(vec![repo_path("conflicted.txt")], no_env())
        .await
        .unwrap();
    let outcome = repository
        .rebase(RebaseAction::Continue, no_env())
        .await
        .unwrap();
    assert!(
        matches!(&outcome, RebaseOutcome::Completed { .. }),
        "unexpected outcome: {outcome:?}"
    );
    assert_eq!(repository.operation_in_progress().await.unwrap(), None);
    assert_eq!(
        current_branch_name(&repository).await.as_deref(),
        Some("topic")
    );
    assert_eq!(commits_between(&repository, "main", "topic").await, ["t1"]);
}

#[gpui::test]
async fn test_fake_rebase_abort_and_skip_leave_no_operation_in_progress(
    executor: BackgroundExecutor,
) {
    let (fs, repository) = open_fake_repository(executor).await;
    fs.insert_branches(dot_git(), &["main", "topic"]);
    fs.set_rebase_conflict_for_repo(
        dot_git(),
        "main",
        "conflicted.txt",
        Some("base\n"),
        Some("ours\n"),
        Some("theirs\n"),
    );
    let start = RebaseAction::Start {
        upstream: "main".into(),
        branch: Some("topic".into()),
    };

    repository.rebase(start.clone(), no_env()).await.unwrap();
    let aborted = repository
        .rebase(RebaseAction::Abort, no_env())
        .await
        .unwrap();
    assert_eq!(aborted, RebaseOutcome::Aborted);
    assert_eq!(repository.operation_in_progress().await.unwrap(), None);
    assert!(!has_conflicts(&repository).await);
    assert_eq!(
        current_branch_name(&repository).await.as_deref(),
        Some("topic")
    );
    let nothing_to_abort = repository
        .rebase(RebaseAction::Abort, no_env())
        .await
        .unwrap_err();
    assert!(
        nothing_to_abort
            .to_string()
            .contains("No rebase in progress"),
        "unexpected error: {nothing_to_abort}"
    );

    repository.rebase(start, no_env()).await.unwrap();
    let skipped = repository
        .rebase(RebaseAction::Skip, no_env())
        .await
        .unwrap();
    assert!(
        matches!(&skipped, RebaseOutcome::Completed { .. }),
        "unexpected outcome: {skipped:?}"
    );
    assert_eq!(repository.operation_in_progress().await.unwrap(), None);
    assert!(!has_conflicts(&repository).await);

    fs.set_simulated_git_failure_for_repo(
        dot_git(),
        FakeGitOperation::Rebase,
        Some((GitFailureKind::Other, "fatal: simulated rebase failure")),
    );
    fs.clear_conflict_plans_for_repo(dot_git());
    let error = repository
        .rebase(
            RebaseAction::Start {
                upstream: "main".into(),
                branch: None,
            },
            no_env(),
        )
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("simulated rebase failure"),
        "unexpected error: {error}"
    );
}

#[gpui::test]
async fn test_fake_reset_hard_moves_head_to_revision_and_clears_conflicts(
    executor: BackgroundExecutor,
) {
    let (fs, repository) = open_fake_repository(executor).await;
    fs.insert_branches(dot_git(), &["main", "origin/main"]);
    fs.set_ref_for_repo(dot_git(), "HEAD", "m2");
    fs.set_ref_for_repo(dot_git(), "refs/remotes/origin/main", "o2");
    fs.set_ref_history_for_repo(dot_git(), "main", history(&["m2", "m1"]));
    fs.set_ref_history_for_repo(dot_git(), "origin/main", history(&["o2", "m2", "m1"]));
    fs.set_conflict_for_repo(
        dot_git(),
        "conflicted.txt",
        Some("base\n"),
        Some("ours\n"),
        Some("theirs\n"),
    );

    let unknown = repository
        .reset_hard("missing".into(), no_env())
        .await
        .unwrap_err();
    assert_eq!(failure(&unknown).kind, GitFailureKind::RevisionNotFound);
    assert_eq!(repository.head_sha().await.as_deref(), Some("m2"));
    assert!(has_conflicts(&repository).await);

    repository
        .reset_hard("origin/main".into(), no_env())
        .await
        .unwrap();
    assert_eq!(repository.head_sha().await.as_deref(), Some("o2"));
    assert_eq!(
        tip_of(&repository, "refs/heads/main").await.as_deref(),
        Some("o2")
    );
    assert_eq!(
        commits_between(&repository, "origin/main", "HEAD").await,
        Vec::<String>::new()
    );
    assert_eq!(repository.operation_in_progress().await.unwrap(), None);
    assert!(!has_conflicts(&repository).await);

    fs.set_simulated_git_failure_for_repo(
        dot_git(),
        FakeGitOperation::ResetHard,
        Some((GitFailureKind::Other, "fatal: simulated reset failure")),
    );
    let error = repository
        .reset_hard("origin/main".into(), no_env())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("simulated reset failure"));
}

#[gpui::test]
async fn test_fake_delete_branch_cleans_up_state_and_reports_unmerged_branches(
    executor: BackgroundExecutor,
) {
    let (fs, repository) = open_fake_repository(executor).await;
    fs.insert_branches(
        dot_git(),
        &["main", "refs/heads/feature/x", "topic", "origin/main"],
    );
    fs.set_upstream_for_repo(dot_git(), "main", "origin/main", tracked(0, 0));
    fs.with_git_state(dot_git(), false, |state| {
        state
            .branches_requiring_force_delete
            .insert("topic".to_string());
    })
    .unwrap();

    let unmerged = repository
        .delete_branch(false, "topic".into(), false)
        .await
        .unwrap_err();
    assert_eq!(
        failure(&unmerged).kind,
        GitFailureKind::BranchNotFullyMerged
    );
    assert!(
        unmerged.to_string().contains("is not fully merged"),
        "unexpected error: {unmerged}"
    );
    assert!(has_branch(&repository, "refs/heads/topic").await);
    repository
        .delete_branch(false, "topic".into(), true)
        .await
        .unwrap();
    assert!(!has_branch(&repository, "refs/heads/topic").await);

    repository
        .delete_branch(false, "feature/x".into(), false)
        .await
        .unwrap();
    assert!(!has_branch(&repository, "refs/heads/feature/x").await);

    repository
        .delete_branch(true, "origin/main".into(), true)
        .await
        .unwrap();
    assert!(!has_branch(&repository, "refs/remotes/origin/main").await);
    let main = find_branch(&repository, "refs/heads/main").await;
    assert_eq!(
        main.upstream
            .expect("upstream should stay recorded")
            .tracking,
        UpstreamTracking::Gone
    );

    let missing = repository
        .delete_branch(false, "ghost".into(), false)
        .await
        .unwrap_err();
    assert!(missing.to_string().contains("no such branch"));
}

#[gpui::test]
async fn test_fake_rename_branch_moves_upstream_history_and_current_branch(
    executor: BackgroundExecutor,
) {
    let (fs, repository) = open_fake_repository(executor).await;
    fs.insert_branches(dot_git(), &["main", "refs/heads/feature/x", "origin/main"]);
    fs.set_upstream_for_repo(dot_git(), "feature/x", "origin/main", tracked(1, 0));
    fs.set_ref_history_for_repo(dot_git(), "feature/x", history(&["c1"]));
    repository
        .change_branch("feature/x".to_string())
        .await
        .unwrap();

    repository
        .rename_branch("feature/x".into(), "feature/y".into())
        .await
        .unwrap();

    assert!(!has_branch(&repository, "refs/heads/feature/x").await);
    let renamed = find_branch(&repository, "refs/heads/feature/y").await;
    assert!(renamed.is_head);
    assert_eq!(
        renamed
            .upstream
            .expect("upstream should move with the branch")
            .tracking,
        tracked(1, 0)
    );
    assert_eq!(
        commits_between(&repository, "main", "feature/y").await,
        ["c1"]
    );
    assert_eq!(recent(&repository, 10).await, ["feature/y"]);

    let missing = repository
        .rename_branch("ghost".into(), "other".into())
        .await
        .unwrap_err();
    assert!(missing.to_string().contains("no such branch"));
}

#[gpui::test]
async fn test_fake_delete_remote_branch_removes_tracking_ref_and_is_idempotent(
    cx: &mut TestAppContext,
) {
    let (fs, repository) = open_fake_repository(cx.executor()).await;
    fs.insert_branches(dot_git(), &["main", "origin/feature"]);
    fs.set_upstream_for_repo(dot_git(), "main", "origin/feature", tracked(0, 0));

    let output = repository
        .delete_remote_branch(
            "origin".into(),
            "feature".into(),
            askpass(cx),
            no_env(),
            cx.to_async(),
        )
        .await
        .unwrap();
    assert!(
        output.stderr.contains("[deleted]"),
        "unexpected output: {output:?}"
    );
    assert!(!has_branch(&repository, "refs/remotes/origin/feature").await);
    assert_eq!(
        find_branch(&repository, "refs/heads/main")
            .await
            .upstream
            .expect("upstream should stay recorded")
            .tracking,
        UpstreamTracking::Gone
    );

    let output = repository
        .delete_remote_branch(
            "origin".into(),
            "feature".into(),
            askpass(cx),
            no_env(),
            cx.to_async(),
        )
        .await
        .unwrap();
    assert!(output.is_empty(), "unexpected output: {output:?}");

    fs.set_simulated_git_failure_for_repo(
        dot_git(),
        FakeGitOperation::DeleteRemoteBranch,
        Some((GitFailureKind::Other, "fatal: unable to access remote")),
    );
    let error = repository
        .delete_remote_branch(
            "origin".into(),
            "feature".into(),
            askpass(cx),
            no_env(),
            cx.to_async(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("unable to access remote"));
}

#[gpui::test]
async fn test_fake_fast_forward_branch_updates_only_fast_forwardable_branches(
    cx: &mut TestAppContext,
) {
    let (fs, repository) = open_fake_repository(cx.executor()).await;
    fs.insert_branches(dot_git(), &["main", "dev", "origin/dev"]);
    fs.set_ref_for_repo(dot_git(), "refs/heads/dev", "d1");
    fs.set_ref_for_repo(dot_git(), "refs/remotes/origin/dev", "remote-sha");
    fs.set_ref_history_for_repo(dot_git(), "dev", history(&["d1"]));
    fs.set_ref_history_for_repo(dot_git(), "origin/dev", history(&["d2", "d1"]));
    fs.set_upstream_for_repo(dot_git(), "dev", "origin/dev", tracked(0, 1));

    let output = repository
        .fast_forward_branch(
            "origin".into(),
            "dev".into(),
            "dev".into(),
            askpass(cx),
            no_env(),
            cx.to_async(),
        )
        .await
        .unwrap();
    assert!(
        output.stderr.contains("dev -> dev"),
        "unexpected output: {output:?}"
    );
    assert_eq!(
        tip_of(&repository, "refs/heads/dev").await.as_deref(),
        Some("remote-sha")
    );
    assert_eq!(
        commits_between(&repository, "dev", "origin/dev").await,
        Vec::<String>::new()
    );
    assert_eq!(
        find_branch(&repository, "refs/heads/dev")
            .await
            .upstream
            .expect("upstream should stay recorded")
            .tracking,
        tracked(0, 0)
    );

    fs.set_ref_history_for_repo(dot_git(), "dev", history(&["x1", "d2", "d1"]));
    fs.set_ref_for_repo(dot_git(), "refs/heads/dev", "x1");
    let rejected = repository
        .fast_forward_branch(
            "origin".into(),
            "dev".into(),
            "dev".into(),
            askpass(cx),
            no_env(),
            cx.to_async(),
        )
        .await
        .unwrap_err();
    assert!(
        rejected.to_string().contains("non-fast-forward"),
        "unexpected error: {rejected}"
    );
    assert_eq!(
        tip_of(&repository, "refs/heads/dev").await.as_deref(),
        Some("x1")
    );

    let current_branch = repository
        .fast_forward_branch(
            "origin".into(),
            "dev".into(),
            "main".into(),
            askpass(cx),
            no_env(),
            cx.to_async(),
        )
        .await
        .unwrap_err();
    assert!(
        current_branch.to_string().contains("current branch"),
        "unexpected error: {current_branch}"
    );

    let missing_remote_ref = repository
        .fast_forward_branch(
            "origin".into(),
            "ghost".into(),
            "dev".into(),
            askpass(cx),
            no_env(),
            cx.to_async(),
        )
        .await
        .unwrap_err();
    assert!(
        missing_remote_ref
            .to_string()
            .contains("couldn't find remote ref"),
        "unexpected error: {missing_remote_ref}"
    );
}

#[gpui::test]
async fn test_fake_push_tag_records_pushed_tags(cx: &mut TestAppContext) {
    let (fs, repository) = open_fake_repository(cx.executor()).await;
    fs.insert_tags(dot_git(), &[("v1", "tag-sha")]);

    let output = repository
        .push_tag(
            "origin".into(),
            "v1".into(),
            askpass(cx),
            no_env(),
            cx.to_async(),
        )
        .await
        .unwrap();
    assert!(
        output.stderr.contains("[new tag]"),
        "unexpected output: {output:?}"
    );
    assert_eq!(
        fs.pushed_tags_for_repo(dot_git()),
        vec![("origin".to_string(), "v1".to_string())]
    );

    let output = repository
        .push_tag(
            "origin".into(),
            "v1".into(),
            askpass(cx),
            no_env(),
            cx.to_async(),
        )
        .await
        .unwrap();
    assert!(
        output.stderr.contains("up-to-date"),
        "unexpected output: {output:?}"
    );
    assert_eq!(fs.pushed_tags_for_repo(dot_git()).len(), 1);

    let missing = repository
        .push_tag(
            "origin".into(),
            "v2".into(),
            askpass(cx),
            no_env(),
            cx.to_async(),
        )
        .await
        .unwrap_err();
    assert!(
        missing.to_string().contains("does not match any"),
        "unexpected error: {missing}"
    );

    fs.set_simulated_git_failure_for_repo(
        dot_git(),
        FakeGitOperation::PushTag,
        Some((GitFailureKind::Other, "fatal: unable to access remote")),
    );
    let error = repository
        .push_tag(
            "origin".into(),
            "v1".into(),
            askpass(cx),
            no_env(),
            cx.to_async(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("unable to access remote"));
}
