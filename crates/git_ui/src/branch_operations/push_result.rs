use git::repository::{Branch, RemoteCommandOutput, UpstreamTracking};
use gpui::{App, SharedString, WeakEntity, Window};
use util::ResultExt as _;
use workspace::Workspace;

use super::{BranchNotice, failure_text};
use crate::git_panel::GitPanel;
use crate::remote_output::extract_pull_request_link;

const EVERYTHING_UP_TO_DATE: &str = "Everything is up to date";
const PUSH_REJECTED_TITLE: &str = "Push rejected";
const PUSH_FAILED_TITLE: &str = "Push failed";
const SHOW_DETAILS_IN_CONSOLE: &str = "Show details in console";
const CREATE_PULL_REQUEST: &str = "Create Pull Request";
const GIT_EVERYTHING_UP_TO_DATE: &str = "Everything up-to-date";
const REJECTED_SUMMARY: &str = "[rejected]";
const STALE_INFO_REASON: &str = "stale info";
const NON_FAST_FORWARD_REASONS: [&str; 2] = ["non-fast-forward", "fetch first"];
const REMOTE_LINE_PREFIX: &str = "remote:";
const PROGRESS_LINE_PREFIXES: [&str; 8] = [
    "Enumerating objects:",
    "Counting objects:",
    "Delta compression using up to",
    "Compressing objects:",
    "Writing objects:",
    "Receiving objects:",
    "Resolving deltas:",
    "Total ",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushTarget {
    pub branch: SharedString,
    pub remote: SharedString,
    pub remote_branch: SharedString,
}

impl PushTarget {
    fn destination(&self) -> String {
        format!("{}/{}", self.remote, self.remote_branch)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PushedUpdate {
    FastForward,
    NewBranch,
    Forced,
    UpToDate,
    Unrecognized,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rejection {
    NotFastForward,
    StaleInfo,
    ByRemote,
}

struct RefStatus<'a> {
    flag: char,
    summary: &'a str,
    reason: Option<&'a str>,
}

pub fn push_succeeded_notice(
    target: &PushTarget,
    pushed_commits: Option<usize>,
    output: &RemoteCommandOutput,
) -> BranchNotice {
    let title = pushed_title(target, pushed_update(&output.stderr), pushed_commits);
    let notice = BranchNotice::info(SharedString::default()).title(title);
    match extract_pull_request_link(output) {
        Some((label, url)) => notice.action(label, move |_window, cx| cx.open_url(&url)),
        None => notice.action(CREATE_PULL_REQUEST, |window, cx| {
            window.dispatch_action(Box::new(zed_actions::git::CreatePullRequest), cx);
        }),
    }
}

pub fn push_failed_notice(
    target: &PushTarget,
    error: &anyhow::Error,
    workspace: WeakEntity<Workspace>,
) -> Option<BranchNotice> {
    let text = failure_text(error)?;
    let notice = match rejection(&text) {
        Some(rejection) => {
            BranchNotice::warning(rejection_message(target, rejection)).title(PUSH_REJECTED_TITLE)
        }
        None => BranchNotice::error(error_details(&text)).title(PUSH_FAILED_TITLE),
    };
    Some(notice.action(SHOW_DETAILS_IN_CONSOLE, move |window, cx| {
        show_git_console(&workspace, window, cx)
    }))
}

pub fn push_not_started_notice(error: &anyhow::Error) -> Option<BranchNotice> {
    let message = failure_text(error)?.trim().to_string();
    Some(BranchNotice::error(message).title(PUSH_FAILED_TITLE))
}

pub fn upstream_ahead_count(branch: &Branch, target: &PushTarget) -> Option<usize> {
    let upstream = branch.upstream.as_ref()?;
    let UpstreamTracking::Tracked(status) = upstream.tracking else {
        return None;
    };
    let tracks_target = upstream.remote_name() == Some(&*target.remote)
        && upstream.branch_name() == Some(&*target.remote_branch);
    tracks_target.then_some(status.ahead as usize)
}

fn show_git_console(workspace: &WeakEntity<Workspace>, window: &mut Window, cx: &mut App) {
    workspace
        .update(cx, |workspace, cx| {
            GitPanel::show_console(workspace, window, cx)
        })
        .log_err();
}

fn pushed_title(
    target: &PushTarget,
    update: PushedUpdate,
    pushed_commits: Option<usize>,
) -> String {
    let destination = target.destination();
    match (update, pushed_commits) {
        (PushedUpdate::UpToDate, _) => EVERYTHING_UP_TO_DATE.to_string(),
        (PushedUpdate::NewBranch, _) => {
            format!("Pushed {} to new branch {destination}", target.branch)
        }
        (PushedUpdate::Forced, _) => format!("Force pushed {} to {destination}", target.branch),
        (PushedUpdate::FastForward, Some(1)) => format!("Pushed 1 commit to {destination}"),
        (PushedUpdate::FastForward, Some(count)) if count > 1 => {
            format!("Pushed {count} commits to {destination}")
        }
        (PushedUpdate::FastForward | PushedUpdate::Unrecognized, _) => {
            format!("Pushed {} to {destination}", target.branch)
        }
    }
}

fn rejection_message(target: &PushTarget, rejection: Rejection) -> String {
    let destination = target.destination();
    match rejection {
        Rejection::NotFastForward => format!("Push to {destination} was rejected"),
        Rejection::ByRemote => format!(
            "Push {} to {destination} was rejected by the remote",
            target.branch
        ),
        Rejection::StaleInfo => format!(
            "Force-with-lease push {} to {destination} was rejected",
            target.branch
        ),
    }
}

fn pushed_update(stderr: &str) -> PushedUpdate {
    let reported = ref_statuses(stderr).find_map(|status| match status.flag {
        ' ' if status.summary.contains("..") => Some(PushedUpdate::FastForward),
        '+' if status.summary.contains("...") => Some(PushedUpdate::Forced),
        '*' => Some(PushedUpdate::NewBranch),
        '=' => Some(PushedUpdate::UpToDate),
        _ => None,
    });
    match reported {
        Some(update) => update,
        None if stderr
            .lines()
            .any(|line| line.trim() == GIT_EVERYTHING_UP_TO_DATE) =>
        {
            PushedUpdate::UpToDate
        }
        None => PushedUpdate::Unrecognized,
    }
}

fn rejection(text: &str) -> Option<Rejection> {
    let status = ref_statuses(text).find(|status| status.flag == '!')?;
    let rejection = match (status.summary, status.reason) {
        (REJECTED_SUMMARY, Some(STALE_INFO_REASON)) => Rejection::StaleInfo,
        (REJECTED_SUMMARY, Some(reason)) if NON_FAST_FORWARD_REASONS.contains(&reason) => {
            Rejection::NotFastForward
        }
        _ => Rejection::ByRemote,
    };
    Some(rejection)
}

fn ref_statuses(text: &str) -> impl Iterator<Item = RefStatus<'_>> {
    text.lines().filter_map(parse_ref_status)
}

fn parse_ref_status(line: &str) -> Option<RefStatus<'_>> {
    let mut characters = line.strip_prefix(' ')?.chars();
    let flag = characters.next()?;
    let rest = characters.as_str().strip_prefix(' ')?;
    let summary_end = if rest.starts_with('[') {
        rest.find(']')? + 1
    } else {
        rest.find(' ')?
    };
    let summary = rest.get(..summary_end)?;
    let refs = rest.get(summary_end..)?.trim();
    let destination = refs
        .split_once(" -> ")
        .map_or(refs, |(_, destination)| destination);
    let reason = destination
        .split_once(" (")
        .and_then(|(_, reason)| reason.strip_suffix(')'));
    Some(RefStatus {
        flag,
        summary,
        reason,
    })
}

fn error_details(text: &str) -> String {
    let details = text
        .lines()
        .filter(|line| !is_progress_line(line))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    if details.is_empty() {
        text.trim().to_string()
    } else {
        details
    }
}

fn is_progress_line(line: &str) -> bool {
    let line = line.trim_start();
    let line = line
        .strip_prefix(REMOTE_LINE_PREFIX)
        .map_or(line, str::trim_start);
    PROGRESS_LINE_PREFIXES
        .iter()
        .any(|prefix| line.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use git::repository::{GitFailure, GitFailureKind, Upstream, UpstreamTrackingStatus};
    use indoc::indoc;

    use super::*;
    use crate::branch_operations::NoticeSeverity;

    fn target(branch: &str, remote: &str, remote_branch: &str) -> PushTarget {
        PushTarget {
            branch: SharedString::from(branch.to_string()),
            remote: SharedString::from(remote.to_string()),
            remote_branch: SharedString::from(remote_branch.to_string()),
        }
    }

    fn output(stderr: &str) -> RemoteCommandOutput {
        RemoteCommandOutput {
            stdout: String::new(),
            stderr: stderr.to_string(),
        }
    }

    fn labels(notice: &BranchNotice) -> Vec<String> {
        notice
            .actions
            .iter()
            .map(|action| action.label.to_string())
            .collect()
    }

    fn failed(stderr: &str) -> anyhow::Error {
        anyhow::anyhow!("{stderr}")
    }

    fn failed_notice(target: &PushTarget, error: &anyhow::Error) -> BranchNotice {
        push_failed_notice(target, error, WeakEntity::new_invalid())
            .expect("a notice for a push that was not cancelled")
    }

    fn branch_tracking(upstream: &str, tracking: UpstreamTracking) -> Branch {
        Branch {
            is_head: true,
            ref_name: SharedString::from("refs/heads/main"),
            upstream: Some(Upstream {
                ref_name: SharedString::from(format!("refs/remotes/{upstream}")),
                tracking,
            }),
            most_recent_commit: None,
        }
    }

    const FAST_FORWARD: &str = indoc! {"
        Enumerating objects: 5, done.
        Counting objects: 100% (5/5), done.
        Delta compression using up to 10 threads
        Compressing objects: 100% (3/3), done.
        Writing objects: 100% (3/3), 312 bytes | 312.00 KiB/s, done.
        Total 3 (delta 2), reused 0 (delta 0), pack-reused 0
        To github.com:octo/repo.git
           1a2b3c4..5d6e7f8  main -> main
    "};

    const NEW_BRANCH: &str = indoc! {"
        Total 0 (delta 0), reused 0 (delta 0), pack-reused 0
        remote:
        remote: Create a pull request for 'feature' on GitHub by visiting:
        remote:      https://github.com/octo/repo/pull/new/feature
        remote:
        To github.com:octo/repo.git
         * [new branch]      feature -> feature
    "};

    const FORCED: &str = indoc! {"
        To github.com:octo/repo.git
         + 1a2b3c4...9f8e7d6 main -> main (forced update)
    "};

    const UP_TO_DATE: &str = "Everything up-to-date\n";

    const FETCH_FIRST: &str = indoc! {"
        To github.com:octo/repo.git
         ! [rejected]        main -> main (fetch first)
        error: failed to push some refs to 'github.com:octo/repo.git'
        hint: Updates were rejected because the remote contains work that you do not
        hint: have locally. This is usually caused by another repository pushing to
        hint: the same ref. If you want to integrate the remote changes, use
        hint: 'git pull' before pushing again.
    "};

    const NON_FAST_FORWARD: &str = indoc! {"
        To github.com:octo/repo.git
         ! [rejected]        main -> main (non-fast-forward)
        error: failed to push some refs to 'github.com:octo/repo.git'
        hint: Updates were rejected because the tip of your current branch is behind
        hint: its remote counterpart.
    "};

    const REMOTE_HOOK_REJECTION: &str = indoc! {"
        Enumerating objects: 3, done.
        Writing objects: 100% (3/3), 280 bytes | 280.00 KiB/s, done.
        remote: error: GH006: Protected branch update failed for refs/heads/main.
        remote: error: Changes must be made through a pull request.
        To github.com:octo/repo.git
         ! [remote rejected] main -> main (protected branch hook declined)
        error: failed to push some refs to 'github.com:octo/repo.git'
    "};

    const STALE_INFO: &str = indoc! {"
        To github.com:octo/repo.git
         ! [rejected]        main -> main (stale info)
        error: failed to push some refs to 'github.com:octo/repo.git'
    "};

    const UNREACHABLE_HOST: &str = indoc! {"
        fatal: unable to access 'https://github.com/octo/repo.git/': Could not resolve host: github.com
    "};

    const DROPPED_CONNECTION: &str = indoc! {"
        Enumerating objects: 2048, done.
        Counting objects: 100% (2048/2048), done.
        Writing objects: 100% (2048/2048), 48.00 MiB | 9.00 MiB/s, done.
        error: RPC failed; HTTP 413 curl 22 The requested URL returned error: 413
        send-pack: unexpected disconnect while reading sideband packet
        fatal: the remote end hung up unexpectedly
    "};

    #[test]
    fn a_fast_forward_of_one_commit_is_reported_as_one_commit() {
        let notice = push_succeeded_notice(
            &target("main", "origin", "main"),
            Some(1),
            &output(FAST_FORWARD),
        );
        assert_eq!(notice.severity, NoticeSeverity::Info);
        assert_eq!(
            notice.title.as_deref(),
            Some("Pushed 1 commit to origin/main")
        );
        assert!(
            notice.message.is_empty(),
            "a success balloon has a title only"
        );
    }

    #[test]
    fn a_fast_forward_of_several_commits_is_reported_with_the_count() {
        let notice = push_succeeded_notice(
            &target("main", "origin", "main"),
            Some(3),
            &output(FAST_FORWARD),
        );
        assert_eq!(
            notice.title.as_deref(),
            Some("Pushed 3 commits to origin/main")
        );
    }

    #[test]
    fn a_fast_forward_without_a_known_count_names_the_branch() {
        let target = target("main", "origin", "main");
        for unknown in [None, Some(0)] {
            let notice = push_succeeded_notice(&target, unknown, &output(FAST_FORWARD));
            assert_eq!(notice.title.as_deref(), Some("Pushed main to origin/main"));
        }
    }

    #[test]
    fn a_new_branch_push_names_the_new_remote_branch() {
        let notice = push_succeeded_notice(
            &target("feature", "origin", "feature"),
            Some(4),
            &output(NEW_BRANCH),
        );
        assert_eq!(
            notice.title.as_deref(),
            Some("Pushed feature to new branch origin/feature")
        );
    }

    #[test]
    fn a_forced_update_is_reported_as_a_force_push() {
        let notice =
            push_succeeded_notice(&target("main", "origin", "main"), Some(2), &output(FORCED));
        assert_eq!(
            notice.title.as_deref(),
            Some("Force pushed main to origin/main")
        );
    }

    #[test]
    fn a_push_with_nothing_to_send_is_up_to_date() {
        let notice = push_succeeded_notice(
            &target("main", "origin", "main"),
            Some(0),
            &output(UP_TO_DATE),
        );
        assert_eq!(notice.title.as_deref(), Some("Everything is up to date"));
        assert!(notice.message.is_empty());
    }

    #[test]
    fn an_unrecognized_success_output_falls_back_to_the_branch_name() {
        let notice =
            push_succeeded_notice(&target("main", "fork", "release"), Some(5), &output(""));
        assert_eq!(notice.title.as_deref(), Some("Pushed main to fork/release"));
    }

    #[test]
    fn a_success_offers_the_pull_request_link_printed_by_the_remote() {
        let notice = push_succeeded_notice(
            &target("feature", "origin", "feature"),
            None,
            &output(NEW_BRANCH),
        );
        assert_eq!(labels(&notice), vec!["Create Pull Request".to_string()]);

        let merge_request = indoc! {"
            remote:
            remote: View merge request for feature:
            remote:    https://gitlab.example.com/octo/repo/-/merge_requests/7
            remote:
            To gitlab.example.com:octo/repo.git
               1a2b3c4..5d6e7f8  feature -> feature
        "};
        let notice = push_succeeded_notice(
            &target("feature", "origin", "feature"),
            Some(1),
            &output(merge_request),
        );
        assert_eq!(labels(&notice), vec!["View Merge Request".to_string()]);
    }

    #[test]
    fn a_success_without_a_link_still_offers_to_create_a_pull_request() {
        for stderr in [FAST_FORWARD, FORCED, UP_TO_DATE] {
            let notice =
                push_succeeded_notice(&target("main", "origin", "main"), Some(1), &output(stderr));
            assert_eq!(labels(&notice), vec!["Create Pull Request".to_string()]);
        }
    }

    #[test]
    fn a_fetch_first_rejection_is_a_warning_about_the_target() {
        let notice = failed_notice(&target("main", "origin", "main"), &failed(FETCH_FIRST));
        assert_eq!(notice.severity, NoticeSeverity::Warning);
        assert_eq!(notice.title.as_deref(), Some("Push rejected"));
        assert_eq!(&*notice.message, "Push to origin/main was rejected");
    }

    #[test]
    fn a_non_fast_forward_rejection_is_a_warning_about_the_target() {
        let notice = failed_notice(&target("main", "origin", "main"), &failed(NON_FAST_FORWARD));
        assert_eq!(notice.severity, NoticeSeverity::Warning);
        assert_eq!(notice.title.as_deref(), Some("Push rejected"));
        assert_eq!(&*notice.message, "Push to origin/main was rejected");
    }

    #[test]
    fn a_hook_rejection_is_reported_as_rejected_by_the_remote() {
        let notice = failed_notice(
            &target("main", "origin", "main"),
            &failed(REMOTE_HOOK_REJECTION),
        );
        assert_eq!(notice.severity, NoticeSeverity::Warning);
        assert_eq!(notice.title.as_deref(), Some("Push rejected"));
        assert_eq!(
            &*notice.message,
            "Push main to origin/main was rejected by the remote"
        );
    }

    #[test]
    fn a_stale_lease_is_reported_as_a_rejected_force_with_lease_push() {
        let notice = failed_notice(&target("main", "origin", "main"), &failed(STALE_INFO));
        assert_eq!(notice.severity, NoticeSeverity::Warning);
        assert_eq!(notice.title.as_deref(), Some("Push rejected"));
        assert_eq!(
            &*notice.message,
            "Force-with-lease push main to origin/main was rejected"
        );
    }

    #[test]
    fn a_rejection_reported_through_a_git_failure_is_still_classified() {
        let error = anyhow::Error::new(GitFailure {
            kind: GitFailureKind::Other,
            message: FETCH_FIRST.to_string(),
        })
        .context("pushing main");
        let notice = failed_notice(&target("main", "origin", "main"), &error);
        assert_eq!(&*notice.message, "Push to origin/main was rejected");
    }

    #[test]
    fn other_failures_are_errors_showing_the_git_message() {
        let notice = failed_notice(&target("main", "origin", "main"), &failed(UNREACHABLE_HOST));
        assert_eq!(notice.severity, NoticeSeverity::Error);
        assert_eq!(notice.title.as_deref(), Some("Push failed"));
        assert_eq!(
            &*notice.message,
            "fatal: unable to access 'https://github.com/octo/repo.git/': Could not resolve host: github.com"
        );
    }

    #[test]
    fn failure_messages_leave_out_git_progress_lines() {
        let notice = failed_notice(
            &target("main", "origin", "main"),
            &failed(DROPPED_CONNECTION),
        );
        assert_eq!(
            &*notice.message,
            indoc! {"
                error: RPC failed; HTTP 413 curl 22 The requested URL returned error: 413
                send-pack: unexpected disconnect while reading sideband packet
                fatal: the remote end hung up unexpectedly"}
        );
    }

    #[test]
    fn rejected_and_failed_pushes_offer_the_console_details() {
        let target = target("main", "origin", "main");
        for stderr in [
            FETCH_FIRST,
            NON_FAST_FORWARD,
            REMOTE_HOOK_REJECTION,
            STALE_INFO,
            UNREACHABLE_HOST,
        ] {
            let notice = failed_notice(&target, &failed(stderr));
            assert_eq!(labels(&notice), vec!["Show details in console".to_string()]);
        }
    }

    #[test]
    fn a_cancelled_push_shows_nothing() {
        let error = anyhow::anyhow!(git::repository::REMOTE_CANCELLED_BY_USER);
        assert!(
            push_failed_notice(
                &target("main", "origin", "main"),
                &error,
                WeakEntity::new_invalid()
            )
            .is_none()
        );
        assert!(push_not_started_notice(&error).is_none());
    }

    #[test]
    fn a_push_that_could_not_start_is_an_error_without_console_details() {
        let notice = push_not_started_notice(&anyhow::anyhow!(
            "No remote available to push to. Add a remote to be able to publish changes.\n"
        ))
        .expect("a notice for a push that could not start");
        assert_eq!(notice.severity, NoticeSeverity::Error);
        assert_eq!(notice.title.as_deref(), Some("Push failed"));
        assert_eq!(
            &*notice.message,
            "No remote available to push to. Add a remote to be able to publish changes."
        );
        assert!(notice.actions.is_empty());
    }

    #[test]
    fn the_ahead_count_is_used_only_for_the_tracked_upstream() {
        let tracked = UpstreamTracking::Tracked(UpstreamTrackingStatus {
            ahead: 3,
            behind: 1,
        });
        let branch = branch_tracking("origin/main", tracked);
        assert_eq!(
            upstream_ahead_count(&branch, &target("main", "origin", "main")),
            Some(3)
        );
        assert_eq!(
            upstream_ahead_count(&branch, &target("main", "fork", "main")),
            None
        );
        assert_eq!(
            upstream_ahead_count(&branch, &target("main", "origin", "release")),
            None
        );

        let gone = branch_tracking("origin/main", UpstreamTracking::Gone);
        assert_eq!(
            upstream_ahead_count(&gone, &target("main", "origin", "main")),
            None
        );

        let untracked = Branch {
            upstream: None,
            ..branch
        };
        assert_eq!(
            upstream_ahead_count(&untracked, &target("main", "origin", "main")),
            None
        );
    }
}
