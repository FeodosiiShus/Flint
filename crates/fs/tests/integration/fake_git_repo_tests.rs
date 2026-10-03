use collections::HashMap;
use fs::{FakeFs, Fs};
use git::{
    repository::repo_path,
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
