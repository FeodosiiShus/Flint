use std::pin::pin;
use std::time::Duration;

use anyhow::{Result, anyhow};
use futures::channel::oneshot;
use futures::future;
use git::repository::{Branch, GitFailure, GitFailureKind, RebaseAction, RebaseOutcome};
use gpui::{App, AsyncWindowContext, Entity, PromptButton, PromptLevel, Window};
use project::git_store::{Repository, RepositoryEvent};
use util::ResultExt as _;

use crate::branch_operations::checkout_revision_dialog;
use crate::branch_operations::integrate::{
    resolve_unmerged_files, unmerged_files_error_notice, update_branch,
};
use crate::branch_operations::new_branch_dialog::{
    self, NewBranchDialogOptions, NewBranchRequest, initial_branch_name,
};
use crate::branch_operations::ongoing::{RebaseReport, conflicts_remain, rebase_conflict_notice};
use crate::branch_operations::smart_operation::{
    RestoreLabels, SmartChoice, SmartOperation, confirm_smart_operation, restore_local_changes,
    stash_local_changes,
};
use crate::branch_operations::{BranchContext, BranchNotice};
use crate::branch_refs::{RefKind, RefTarget};
use crate::merge_tool::conflict_resolution::{
    ConflictParams, RebaseCustomizerSpec, RebaseUpstream, ResolveMode, ResolverBehavior,
};

const HEAD_REFERENCE: &str = "HEAD";
const BRANCH_SWITCH_TIMEOUT: Duration = Duration::from_secs(5);
const NEW_BRANCH_TITLE: &str = "Create New Branch";
const CREATE_LABEL: &str = "Create";
const CHECKOUT_LABEL: &str = "Checkout";
const EMPTY_REPOSITORY_MESSAGE: &str =
    "Cannot create new branch in empty repository. Make initial commit first";
const CHECKOUT_AND_REBASE_FAILED: &str = "Checkout and Rebase failed";
const REBASE_FAILED: &str = "Rebase failed";

pub fn checkout(context: BranchContext, target: RefTarget, window: &mut Window, cx: &mut App) {
    match target.kind {
        RefKind::Local => {
            if is_current_branch(&context, &target.name, cx) {
                return;
            }
            spawn_step(context, StepRun::switch_branch(&target.name), window, cx);
        }
        RefKind::Remote => checkout_remote(context, target, window, cx),
        RefKind::Tag => {
            let reference = target.full_ref_name();
            spawn_step(context, StepRun::detach(&reference), window, cx);
        }
    }
}

pub fn checkout_and_rebase_onto_current(
    context: BranchContext,
    target: RefTarget,
    window: &mut Window,
    cx: &mut App,
) {
    match target.kind {
        RefKind::Local => {
            if is_current_branch(&context, &target.name, cx) {
                return;
            }
            let base = current_base_label(&context, cx);
            let run = StepRun::rebase(
                &target.name,
                HEAD_REFERENCE,
                &base,
                CHECKOUT_AND_REBASE_FAILED,
            );
            spawn_step(context, run, window, cx);
        }
        RefKind::Remote => checkout_and_rebase_remote(context, target, window, cx),
        RefKind::Tag => {}
    }
}

pub fn checkout_and_update(
    context: BranchContext,
    target: RefTarget,
    window: &mut Window,
    cx: &mut App,
) {
    if target.kind != RefKind::Local {
        checkout(context, target, window, cx);
        return;
    }
    let name = target.name.to_string();
    let already_current = is_current_branch(&context, &name, cx);
    window
        .spawn(cx, async move |cx| {
            if !already_current {
                let switched =
                    run_step_async(context.clone(), StepRun::switch_branch(&name), cx).await;
                if !switched {
                    return;
                }
                wait_for_current_branch(&context, &name, cx).await;
            }
            cx.update(|window, cx| {
                update_branch(context.clone(), RefTarget::local(name.clone()), window, cx)
            })
            .log_err();
        })
        .detach();
}

pub fn new_branch_from(
    context: BranchContext,
    start: RefTarget,
    window: &mut Window,
    cx: &mut App,
) {
    let options = NewBranchDialogOptions {
        title: format!("Create Branch from {}", start.name).into(),
        confirm_label: CREATE_LABEL.into(),
        initial_name: initial_branch_name(&start),
        show_checkout_option: true,
    };
    let start_point = start.name.to_string();
    let submit_context = context.clone();
    new_branch_dialog::open(
        &context,
        options,
        move |request, window, cx| {
            let request = CreateBranchRequest::from_dialog(request, start_point, true);
            create_branch_from(submit_context, request, window, cx);
        },
        window,
        cx,
    );
}

pub fn new_branch(context: BranchContext, window: &mut Window, cx: &mut App) {
    if is_fresh_repository(context.repository.read(cx)) {
        context.notify(BranchNotice::warning(EMPTY_REPOSITORY_MESSAGE), cx);
        return;
    }
    let options = NewBranchDialogOptions {
        title: NEW_BRANCH_TITLE.into(),
        confirm_label: CREATE_LABEL.into(),
        initial_name: context
            .current_branch_name(cx)
            .map(|name| name.to_string())
            .unwrap_or_default(),
        show_checkout_option: true,
    };
    let submit_context = context.clone();
    new_branch_dialog::open(
        &context,
        options,
        move |request, window, cx| {
            let request =
                CreateBranchRequest::from_dialog(request, HEAD_REFERENCE.to_string(), true);
            create_branch_from(submit_context, request, window, cx);
        },
        window,
        cx,
    );
}

pub fn checkout_tag_or_revision(context: BranchContext, window: &mut Window, cx: &mut App) {
    let submit_context = context.clone();
    checkout_revision_dialog::open(
        &context,
        move |revision, window, cx| {
            spawn_step(submit_context, StepRun::detach(&revision), window, cx);
        },
        window,
        cx,
    );
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CreateBranchRequest {
    name: String,
    start: String,
    checkout: bool,
    overwrite: bool,
    peel_start: bool,
}

impl CreateBranchRequest {
    fn from_dialog(request: NewBranchRequest, start: String, peel_start: bool) -> Self {
        Self {
            name: request.name,
            start,
            checkout: request.checkout,
            overwrite: request.overwrite,
            peel_start,
        }
    }

    fn git_start_point(&self) -> String {
        if self.peel_start {
            format!("{}^0", self.start)
        } else {
            self.start.clone()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CheckoutStep {
    SwitchBranch {
        name: String,
    },
    Detach {
        revision: String,
    },
    CreateBranch {
        name: String,
        start_point: String,
        checkout: bool,
        overwrite: bool,
    },
    RebaseOnto {
        upstream: String,
        branch: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Completion {
    CheckedOut { label: String },
    CheckedOutNewBranch { name: String, start: String },
    BranchCreated { name: String },
    RebasedOnto { branch: String, base: String },
    Silent,
}

impl Completion {
    fn notice(&self) -> Option<BranchNotice> {
        match self {
            Completion::CheckedOut { label } => {
                Some(BranchNotice::info(format!("Checked out {label}")))
            }
            Completion::CheckedOutNewBranch { name, start } => Some(BranchNotice::info(format!(
                "Checked out new branch {name} from {start}"
            ))),
            Completion::BranchCreated { name } => {
                Some(BranchNotice::info(format!("Branch {name} was created")))
            }
            Completion::RebasedOnto { branch, base } => Some(
                BranchNotice::info(format!("Checked out {branch} and rebased it on {base}"))
                    .title("Rebase successful"),
            ),
            Completion::Silent => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StepOutcome {
    Completed,
    RebaseStopped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StepRun {
    step: CheckoutStep,
    completion: Completion,
    failure_title: String,
    operation: SmartOperation,
    ref_name: String,
}

impl StepRun {
    fn switch_branch(name: &str) -> Self {
        Self {
            step: CheckoutStep::SwitchBranch {
                name: name.to_string(),
            },
            completion: Completion::CheckedOut {
                label: name.to_string(),
            },
            failure_title: format!("Could not checkout {name}"),
            operation: SmartOperation::Checkout,
            ref_name: name.to_string(),
        }
    }

    fn detach(revision: &str) -> Self {
        Self {
            step: CheckoutStep::Detach {
                revision: revision.to_string(),
            },
            completion: Completion::CheckedOut {
                label: revision.to_string(),
            },
            failure_title: format!("Could not checkout {revision}"),
            operation: SmartOperation::Checkout,
            ref_name: revision.to_string(),
        }
    }

    fn create_branch(request: &CreateBranchRequest) -> Self {
        let completion = if request.checkout {
            Completion::CheckedOutNewBranch {
                name: request.name.clone(),
                start: request.start.clone(),
            }
        } else {
            Completion::BranchCreated {
                name: request.name.clone(),
            }
        };
        Self {
            step: CheckoutStep::CreateBranch {
                name: request.name.clone(),
                start_point: request.git_start_point(),
                checkout: request.checkout,
                overwrite: request.overwrite,
            },
            completion,
            failure_title: format!("Could not create new branch {}", request.name),
            operation: SmartOperation::Checkout,
            ref_name: request.name.clone(),
        }
    }

    fn create_branch_before_rebase(name: &str, start: &str, overwrite: bool) -> Self {
        Self {
            step: CheckoutStep::CreateBranch {
                name: name.to_string(),
                start_point: start.to_string(),
                checkout: false,
                overwrite,
            },
            completion: Completion::Silent,
            failure_title: CHECKOUT_AND_REBASE_FAILED.to_string(),
            operation: SmartOperation::Rebase,
            ref_name: name.to_string(),
        }
    }

    fn rebase(branch: &str, upstream: &str, base_label: &str, failure_title: &str) -> Self {
        Self {
            step: CheckoutStep::RebaseOnto {
                upstream: upstream.to_string(),
                branch: branch.to_string(),
            },
            completion: Completion::RebasedOnto {
                branch: branch.to_string(),
                base: base_label.to_string(),
            },
            failure_title: failure_title.to_string(),
            operation: SmartOperation::Rebase,
            ref_name: branch.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocalCommitsAnswer {
    Rebase,
    Drop,
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LocalCommitsPrompt {
    title: String,
    detail: String,
    answers: Vec<(&'static str, LocalCommitsAnswer)>,
}

impl LocalCommitsPrompt {
    fn rebase_or_drop(local: &str, start: &str) -> Self {
        Self {
            title: format!("Checkout {start}"),
            detail: format!(
                "Local branch '{local}' has commits that do not exist in '{start}'. Rebase \
                 '{local}' onto '{start}', or drop local commits?"
            ),
            answers: vec![
                ("Rebase onto Remote", LocalCommitsAnswer::Rebase),
                ("Drop Local Commits", LocalCommitsAnswer::Drop),
                ("Cancel", LocalCommitsAnswer::Cancel),
            ],
        }
    }

    fn drop_only(title: String, local: &str, start: &str) -> Self {
        Self {
            title,
            detail: format!(
                "Local branch '{local}' has commits that do not exist in '{start}'. Do you want \
                 to drop local commits?"
            ),
            answers: vec![
                ("Drop Local Commits", LocalCommitsAnswer::Drop),
                ("Cancel", LocalCommitsAnswer::Cancel),
            ],
        }
    }

    fn for_new_branch(request: &CreateBranchRequest) -> Self {
        if request.checkout {
            Self::rebase_or_drop(&request.name, &request.start)
        } else {
            Self::drop_only(
                format!("Create Branch from {}", request.start),
                &request.name,
                &request.start,
            )
        }
    }
}

fn spawn_step(context: BranchContext, run: StepRun, window: &mut Window, cx: &mut App) {
    window
        .spawn(cx, async move |cx| {
            run_step_async(context, run, cx).await;
        })
        .detach();
}

fn checkout_remote(context: BranchContext, target: RefTarget, window: &mut Window, cx: &mut App) {
    let remote = target.name.to_string();
    let local_name = target.name_without_remote().to_string();
    let existing = local_branch(&context, &local_name, cx);
    let collides = existing
        .as_ref()
        .is_some_and(|branch| tracks_different_remote(upstream_ref_name(branch), &remote));
    if collides {
        let options = NewBranchDialogOptions {
            title: format!("Checkout {remote}").into(),
            confirm_label: CHECKOUT_LABEL.into(),
            initial_name: local_name,
            show_checkout_option: false,
        };
        let submit_context = context.clone();
        new_branch_dialog::open(
            &context,
            options,
            move |request, window, cx| {
                let request = CreateBranchRequest::from_dialog(request, remote, false);
                create_branch_from(submit_context, request, window, cx);
            },
            window,
            cx,
        );
    } else {
        let request = CreateBranchRequest {
            name: local_name,
            start: remote,
            checkout: true,
            overwrite: existing.is_some(),
            peel_start: false,
        };
        create_branch_from(context, request, window, cx);
    }
}

fn checkout_and_rebase_remote(
    context: BranchContext,
    target: RefTarget,
    window: &mut Window,
    cx: &mut App,
) {
    let remote = target.name.to_string();
    let local_name = target.name_without_remote().to_string();
    let existing = local_branch(&context, &local_name, cx);
    let collides = existing
        .as_ref()
        .is_some_and(|branch| tracks_different_remote(upstream_ref_name(branch), &remote));
    if collides {
        let options = NewBranchDialogOptions {
            title: format!("Checkout {remote}").into(),
            confirm_label: CHECKOUT_LABEL.into(),
            initial_name: local_name,
            show_checkout_option: false,
        };
        let submit_context = context.clone();
        new_branch_dialog::open(
            &context,
            options,
            move |request, window, cx| {
                rebase_remote_into_local(
                    submit_context,
                    remote,
                    request.name,
                    request.overwrite,
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    } else {
        rebase_remote_into_local(context, remote, local_name, existing.is_some(), window, cx);
    }
}

fn rebase_remote_into_local(
    context: BranchContext,
    remote: String,
    local: String,
    reset: bool,
    window: &mut Window,
    cx: &mut App,
) {
    window
        .spawn(cx, async move |cx| {
            rebase_remote_flow(context, remote, local, reset, cx).await;
        })
        .detach();
}

async fn rebase_remote_flow(
    context: BranchContext,
    remote: String,
    local: String,
    reset: bool,
    cx: &mut AsyncWindowContext,
) {
    let is_current = cx
        .update(|_, cx| is_current_branch(&context, &local, cx))
        .unwrap_or(false);
    if is_current {
        let notice = BranchNotice::error(format!("Cannot overwrite current branch {local}"))
            .title(CHECKOUT_AND_REBASE_FAILED);
        cx.update(|_, cx| context.notify(notice, cx)).log_err();
        return;
    }
    if reset {
        match local_commits_missing_from(&context, &local, &remote, cx).await {
            Ok(false) => {}
            Ok(true) => {
                let prompt =
                    LocalCommitsPrompt::drop_only(format!("Checkout {remote}"), &local, &remote);
                if ask_local_commits(&prompt, cx).await != LocalCommitsAnswer::Drop {
                    return;
                }
            }
            Err(error) => {
                notify_failure(&context, "log", error, cx);
                return;
            }
        }
    }
    let base = cx
        .update(|_, cx| current_base_label(&context, cx))
        .unwrap_or_else(|_| HEAD_REFERENCE.to_string());
    let created = run_step_async(
        context.clone(),
        StepRun::create_branch_before_rebase(&local, &remote, reset),
        cx,
    )
    .await;
    if !created {
        return;
    }
    let run = StepRun::rebase(&local, HEAD_REFERENCE, &base, CHECKOUT_AND_REBASE_FAILED);
    run_step_async(context, run, cx).await;
}

fn create_branch_from(
    context: BranchContext,
    request: CreateBranchRequest,
    window: &mut Window,
    cx: &mut App,
) {
    let local_exists = local_branch(&context, &request.name, cx).is_some();
    window
        .spawn(cx, async move |cx| {
            create_branch_flow(context, request, local_exists, cx).await;
        })
        .detach();
}

async fn create_branch_flow(
    context: BranchContext,
    request: CreateBranchRequest,
    local_exists: bool,
    cx: &mut AsyncWindowContext,
) {
    if local_exists && request.overwrite {
        match local_commits_missing_from(&context, &request.name, &request.start, cx).await {
            Ok(false) => {}
            Ok(true) => {
                let prompt = LocalCommitsPrompt::for_new_branch(&request);
                match ask_local_commits(&prompt, cx).await {
                    LocalCommitsAnswer::Cancel => return,
                    LocalCommitsAnswer::Drop => {}
                    LocalCommitsAnswer::Rebase => {
                        let run = StepRun::rebase(
                            &request.name,
                            &request.start,
                            &request.start,
                            REBASE_FAILED,
                        );
                        run_step_async(context, run, cx).await;
                        return;
                    }
                }
            }
            Err(error) => {
                notify_failure(&context, "log", error, cx);
                return;
            }
        }
    }
    run_step_async(context, StepRun::create_branch(&request), cx).await;
}

async fn local_commits_missing_from(
    context: &BranchContext,
    local: &str,
    start: &str,
    cx: &mut AsyncWindowContext,
) -> Result<bool> {
    let receiver = context.repository.update(cx, |repository, _| {
        repository.commits_between(start.to_string(), local.to_string(), 1)
    });
    Ok(!receive(receiver).await?.is_empty())
}

async fn ask_local_commits(
    prompt: &LocalCommitsPrompt,
    cx: &mut AsyncWindowContext,
) -> LocalCommitsAnswer {
    let buttons: Vec<PromptButton> = prompt
        .answers
        .iter()
        .map(|(label, answer)| {
            if *answer == LocalCommitsAnswer::Cancel {
                PromptButton::cancel(*label)
            } else {
                PromptButton::new(*label)
            }
        })
        .collect();
    let receiver = cx.prompt(
        PromptLevel::Warning,
        &prompt.title,
        Some(prompt.detail.as_str()),
        buttons.as_slice(),
    );
    match receiver.await {
        Ok(index) => prompt
            .answers
            .get(index)
            .map_or(LocalCommitsAnswer::Cancel, |(_, answer)| *answer),
        Err(_) => LocalCommitsAnswer::Cancel,
    }
}

async fn run_step_async(context: BranchContext, run: StepRun, cx: &mut AsyncWindowContext) -> bool {
    if stashes_local_changes_first(&run.step) {
        return stash_and_retry(&context, &run, cx).await;
    }
    match execute_step(&context.repository, &run.step, false, cx).await {
        Ok(outcome) => {
            announce(&context, &run, outcome, None, false, cx).await == StepOutcome::Completed
        }
        Err(error) => recover(context, run, error, cx).await,
    }
}

fn stashes_local_changes_first(step: &CheckoutStep) -> bool {
    matches!(step, CheckoutStep::RebaseOnto { .. })
}

fn force_resets_current_branch(name: &str, overwrite: bool, current: Option<&str>) -> bool {
    overwrite && current == Some(name)
}

async fn execute_step(
    repository: &Entity<Repository>,
    step: &CheckoutStep,
    force: bool,
    cx: &mut AsyncWindowContext,
) -> Result<StepOutcome> {
    match step {
        CheckoutStep::SwitchBranch { name } => {
            let receiver = if force {
                repository.update(cx, |repository, _| repository.checkout_force(name.clone()))
            } else {
                repository.update(cx, |repository, _| repository.change_branch(name.clone()))
            };
            receive(receiver).await?;
            Ok(StepOutcome::Completed)
        }
        CheckoutStep::Detach { revision } => {
            let receiver = if force {
                repository.update(cx, |repository, _| {
                    repository.checkout_force(format!("{revision}^0"))
                })
            } else {
                repository.update(cx, |repository, _| {
                    repository.checkout_detached(revision.clone())
                })
            };
            receive(receiver).await?;
            Ok(StepOutcome::Completed)
        }
        CheckoutStep::CreateBranch {
            name,
            start_point,
            checkout,
            overwrite,
        } => {
            if force && *checkout {
                let current = repository.read_with(cx, |repository, _| {
                    repository
                        .branch
                        .as_ref()
                        .map(|branch| branch.name().to_string())
                });
                if force_resets_current_branch(name, *overwrite, current.as_deref()) {
                    let reset = repository.update(cx, |repository, _| {
                        repository.reset_hard(start_point.clone())
                    });
                    receive(reset).await?;
                } else {
                    let created = repository.update(cx, |repository, _| {
                        repository.create_branch_at(
                            name.clone(),
                            start_point.clone(),
                            false,
                            *overwrite,
                        )
                    });
                    receive(created).await?;
                    let switched = repository
                        .update(cx, |repository, _| repository.checkout_force(name.clone()));
                    receive(switched).await?;
                }
            } else {
                let receiver = repository.update(cx, |repository, _| {
                    repository.create_branch_at(
                        name.clone(),
                        start_point.clone(),
                        *checkout,
                        *overwrite,
                    )
                });
                receive(receiver).await?;
            }
            Ok(StepOutcome::Completed)
        }
        CheckoutStep::RebaseOnto { upstream, branch } => {
            let receiver = repository.update(cx, |repository, _| {
                repository.rebase(RebaseAction::Start {
                    upstream: upstream.clone(),
                    branch: Some(branch.clone()),
                })
            });
            match receive(receiver).await? {
                RebaseOutcome::Completed { .. } => Ok(StepOutcome::Completed),
                RebaseOutcome::Conflicted { .. } => Ok(StepOutcome::RebaseStopped),
                RebaseOutcome::Aborted => Err(anyhow!("The rebase was aborted")),
            }
        }
    }
}

async fn receive<T>(receiver: oneshot::Receiver<Result<T>>) -> Result<T> {
    receiver
        .await
        .map_err(|_| anyhow!("The git operation was canceled"))?
}

async fn announce(
    context: &BranchContext,
    run: &StepRun,
    outcome: StepOutcome,
    initial_branch: Option<String>,
    stashed: bool,
    cx: &mut AsyncWindowContext,
) -> StepOutcome {
    match outcome {
        StepOutcome::Completed => {
            notify_completion(context, run, cx);
            StepOutcome::Completed
        }
        StepOutcome::RebaseStopped => {
            resolve_stopped_checkout_rebase(context, run, initial_branch, stashed, cx).await
        }
    }
}

fn notify_completion(context: &BranchContext, run: &StepRun, cx: &mut AsyncWindowContext) {
    if let Some(notice) = run.completion.notice() {
        cx.update(|_, app| context.notify(notice, app)).log_err();
    }
}

async fn resolve_stopped_checkout_rebase(
    context: &BranchContext,
    run: &StepRun,
    initial_branch: Option<String>,
    stashed: bool,
    cx: &mut AsyncWindowContext,
) -> StepOutcome {
    let CheckoutStep::RebaseOnto { upstream, branch } = &run.step else {
        return StepOutcome::RebaseStopped;
    };
    let params = ConflictParams::for_rebase_process(RebaseCustomizerSpec {
        upstream: Some(RebaseUpstream::from_ref_string(upstream)),
        branch: Some(branch.clone()),
        initial_branch,
    });
    let resolution = cx.update(|window, app| {
        context.resolve_conflicts(
            params,
            ResolverBehavior::ContinueRebase,
            ResolveMode::Initial,
            window,
            app,
        )
    });
    let proceeded = match resolution {
        Ok(task) => task.await.proceed,
        Err(error) => {
            log::error!("could not start the conflict resolution: {error:#}");
            false
        }
    };
    if proceeded {
        notify_completion(context, run, cx);
        return StepOutcome::Completed;
    }
    if conflicts_remain(context, cx).await {
        let report = RebaseReport {
            branch: Some(branch.clone()),
            onto: Some(upstream.clone()),
            stashed,
            ..RebaseReport::default()
        };
        cx.update(|_, app| context.notify(rebase_conflict_notice(context, &report), app))
            .log_err();
    }
    StepOutcome::RebaseStopped
}

async fn recover(
    context: BranchContext,
    run: StepRun,
    error: anyhow::Error,
    cx: &mut AsyncWindowContext,
) -> bool {
    let overwritten_files = match git_failure(&error).map(|failure| &failure.kind) {
        Some(GitFailureKind::LocalChangesWouldBeOverwritten { files }) => files.clone(),
        _ => {
            report_failure(&context, &run, &error, cx);
            return false;
        }
    };
    let operation = run.operation;
    let Ok(decision) = cx.update(|window, cx| {
        confirm_smart_operation(&context, operation, overwritten_files, window, cx)
    }) else {
        return false;
    };
    match decision.await {
        SmartChoice::Cancel => false,
        SmartChoice::Force => force_step(&context, &run, cx).await,
        SmartChoice::Smart => stash_and_retry(&context, &run, cx).await,
    }
}

async fn force_step(context: &BranchContext, run: &StepRun, cx: &mut AsyncWindowContext) -> bool {
    if run.operation != SmartOperation::Checkout {
        return false;
    }
    match execute_step(&context.repository, &run.step, true, cx).await {
        Ok(outcome) => {
            announce(context, run, outcome, None, false, cx).await == StepOutcome::Completed
        }
        Err(error) => {
            report_failure(context, run, &error, cx);
            false
        }
    }
}

async fn stash_and_retry(
    context: &BranchContext,
    run: &StepRun,
    cx: &mut AsyncWindowContext,
) -> bool {
    let Ok(stashing) = cx.update(|_, cx| stash_local_changes(context, cx)) else {
        return false;
    };
    let stashed = match stashing.await {
        Ok(stashed) => stashed,
        Err(error) => {
            notify_failure(context, "stash", error, cx);
            return false;
        }
    };
    let initial_branch = cx
        .update(|_, app| context.current_branch_name(app))
        .log_err()
        .flatten()
        .map(|name| name.to_string());
    let outcome = match execute_step(&context.repository, &run.step, false, cx).await {
        Ok(outcome) => Some(announce(context, run, outcome, initial_branch, stashed, cx).await),
        Err(error) => {
            report_failure(context, run, &error, cx);
            None
        }
    };
    if outcome != Some(StepOutcome::RebaseStopped) {
        let labels = RestoreLabels::new(run.operation.verb(), run.ref_name.clone());
        restore_stashed_changes(context, stashed, labels, cx).await;
    }
    outcome == Some(StepOutcome::Completed)
}

async fn restore_stashed_changes(
    context: &BranchContext,
    stashed: bool,
    labels: RestoreLabels,
    cx: &mut AsyncWindowContext,
) {
    let Ok(restoring) =
        cx.update(|window, cx| restore_local_changes(context, stashed, labels, window, cx))
    else {
        return;
    };
    if let Err(error) = restoring.await {
        notify_failure(context, "stash pop", error, cx);
    }
}

fn notify_failure(
    context: &BranchContext,
    operation: &'static str,
    error: anyhow::Error,
    cx: &mut AsyncWindowContext,
) {
    cx.update(|_, cx| context.notify_error(operation, error, cx))
        .log_err();
}

fn report_failure(
    context: &BranchContext,
    run: &StepRun,
    error: &anyhow::Error,
    cx: &mut AsyncWindowContext,
) {
    cx.update(|_, cx| {
        let notice = failure_notice(
            run,
            error,
            resolve_unmerged_files(context.clone(), run.operation.verb()),
        );
        context.notify(notice, cx);
    })
    .log_err();
}

fn git_failure(error: &anyhow::Error) -> Option<&GitFailure> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<GitFailure>())
}

fn failure_message(error: &anyhow::Error) -> String {
    match git_failure(error) {
        Some(failure) => failure.message.trim().to_string(),
        None => format!("{error:#}").trim().to_string(),
    }
}

fn failure_notice(
    run: &StepRun,
    error: &anyhow::Error,
    resolve_conflicts: impl Fn(&mut Window, &mut App) + 'static,
) -> BranchNotice {
    let verb = run.operation.verb();
    match git_failure(error).map(|failure| &failure.kind) {
        Some(GitFailureKind::UnmergedFiles) => unmerged_files_error_notice(verb, resolve_conflicts),
        Some(GitFailureKind::UntrackedFilesWouldBeOverwritten { files }) => BranchNotice::error(
            format!("Move or commit them before {verb}\n{}", files.join("\n")),
        )
        .title(format!(
            "Untracked Files Prevent {}",
            run.operation.title_case()
        )),
        Some(GitFailureKind::BranchCheckedOutInWorktree { path }) => BranchNotice::error(format!(
            "Branch '{}' is already checked out in a worktree at:\n{path}",
            run.ref_name
        ))
        .title("Worktree Conflict"),
        Some(GitFailureKind::RevisionNotFound) => {
            BranchNotice::error("Revision not found").title(run.failure_title.clone())
        }
        _ => BranchNotice::error(failure_message(error)).title(run.failure_title.clone()),
    }
}

async fn wait_for_current_branch(
    context: &BranchContext,
    expected: &str,
    cx: &mut AsyncWindowContext,
) {
    let (sender, receiver) = async_channel::unbounded::<()>();
    let expected_name = expected.to_string();
    let subscribed = cx.update(|_, cx| {
        let notify_sender = sender.clone();
        let subscription = cx.subscribe(
            &context.repository,
            move |repository, _: &RepositoryEvent, cx| {
                if is_on_branch(repository.read(cx), &expected_name) {
                    notify_sender.try_send(()).log_err();
                }
            },
        );
        if is_on_branch(context.repository.read(cx), expected) {
            sender.try_send(()).log_err();
        }
        subscription
    });
    let Ok(subscription) = subscribed else {
        return;
    };
    let received = pin!(receiver.recv());
    let timeout = pin!(cx.background_executor().timer(BRANCH_SWITCH_TIMEOUT));
    future::select(received, timeout).await;
    drop(subscription);
}

fn is_on_branch(repository: &Repository, name: &str) -> bool {
    repository
        .branch
        .as_ref()
        .is_some_and(|branch| branch.name() == name)
}

fn is_current_branch(context: &BranchContext, name: &str, cx: &App) -> bool {
    context
        .current_branch_name(cx)
        .is_some_and(|current| &*current == name)
}

fn current_base_label(context: &BranchContext, cx: &App) -> String {
    context
        .current_branch_name(cx)
        .map(|name| name.to_string())
        .unwrap_or_else(|| HEAD_REFERENCE.to_string())
}

fn local_branch(context: &BranchContext, name: &str, cx: &App) -> Option<Branch> {
    context.find_branch(&RefTarget::local(name.to_string()), cx)
}

fn upstream_ref_name(branch: &Branch) -> Option<&str> {
    branch
        .upstream
        .as_ref()
        .and_then(|upstream| upstream.stripped_ref_name())
}

fn tracks_different_remote(upstream: Option<&str>, remote: &str) -> bool {
    upstream.is_some_and(|upstream| upstream != remote)
}

fn is_fresh_repository(repository: &Repository) -> bool {
    repository.head_commit.is_none()
        && repository
            .branch
            .as_ref()
            .is_none_or(|branch| branch.most_recent_commit.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(kind: GitFailureKind, message: &str) -> anyhow::Error {
        anyhow::Error::new(GitFailure {
            kind,
            message: message.to_string(),
        })
    }

    fn notice_title(notice: &BranchNotice) -> Option<&str> {
        notice.title.as_deref()
    }

    fn action_labels(notice: &BranchNotice) -> Vec<&str> {
        notice.actions.iter().map(|action| &*action.label).collect()
    }

    #[test]
    fn completion_notices_use_the_ide_wording() {
        let checked_out = Completion::CheckedOut {
            label: "main".to_string(),
        }
        .notice()
        .expect("notice");
        assert_eq!(&*checked_out.message, "Checked out main");
        assert_eq!(notice_title(&checked_out), None);

        let new_branch = Completion::CheckedOutNewBranch {
            name: "feature".to_string(),
            start: "origin/main".to_string(),
        }
        .notice()
        .expect("notice");
        assert_eq!(
            &*new_branch.message,
            "Checked out new branch feature from origin/main"
        );

        let created = Completion::BranchCreated {
            name: "feature".to_string(),
        }
        .notice()
        .expect("notice");
        assert_eq!(&*created.message, "Branch feature was created");

        let rebased = Completion::RebasedOnto {
            branch: "feature".to_string(),
            base: "main".to_string(),
        }
        .notice()
        .expect("notice");
        assert_eq!(
            &*rebased.message,
            "Checked out feature and rebased it on main"
        );
        assert_eq!(notice_title(&rebased), Some("Rebase successful"));

        assert!(Completion::Silent.notice().is_none());
    }

    #[test]
    fn unhandled_local_changes_failure_falls_back_to_the_generic_checkout_failure_notice() {
        let run = StepRun::switch_branch("feature");
        let error = failure(
            GitFailureKind::LocalChangesWouldBeOverwritten {
                files: vec!["a.rs".to_string()],
            },
            "error: Your local changes would be overwritten",
        );
        let notice = failure_notice(&run, &error, |_, _| {});
        assert_eq!(notice_title(&notice), Some("Could not checkout feature"));
        assert_eq!(
            &*notice.message,
            "error: Your local changes would be overwritten"
        );
    }

    #[test]
    fn unmerged_files_failure_offers_to_resolve_conflicts() {
        let run = StepRun::switch_branch("feature");
        let error = failure(GitFailureKind::UnmergedFiles, "needs merge");
        let notice = failure_notice(&run, &error, |_, _| {});
        assert_eq!(
            notice_title(&notice),
            Some("Cannot checkout because of unmerged files")
        );
        assert!(
            notice
                .message
                .starts_with("You need to resolve all merge conflicts before checkout.")
        );
        assert_eq!(action_labels(&notice), vec!["Resolve conflicts…"]);
    }

    #[test]
    fn untracked_files_failure_lists_the_files_and_names_the_operation() {
        let run = StepRun::rebase("feature", "HEAD", "main", CHECKOUT_AND_REBASE_FAILED);
        let error = failure(
            GitFailureKind::UntrackedFilesWouldBeOverwritten {
                files: vec!["a.rs".to_string(), "b.rs".to_string()],
            },
            "untracked",
        );
        let notice = failure_notice(&run, &error, |_, _| {});
        assert_eq!(
            notice_title(&notice),
            Some("Untracked Files Prevent Rebase")
        );
        assert_eq!(
            &*notice.message,
            "Move or commit them before rebase\na.rs\nb.rs"
        );
        assert!(notice.actions.is_empty());
    }

    #[test]
    fn worktree_failure_names_the_branch_and_the_path() {
        let run = StepRun::switch_branch("feature");
        let error = failure(
            GitFailureKind::BranchCheckedOutInWorktree {
                path: "/work/feature".to_string(),
            },
            "already checked out",
        );
        let notice = failure_notice(&run, &error, |_, _| {});
        assert_eq!(notice_title(&notice), Some("Worktree Conflict"));
        assert_eq!(
            &*notice.message,
            "Branch 'feature' is already checked out in a worktree at:\n/work/feature"
        );
    }

    #[test]
    fn missing_revision_is_reported_under_the_checkout_title() {
        let run = StepRun::detach("deadbeef");
        let error = failure(GitFailureKind::RevisionNotFound, "not a valid object name");
        let notice = failure_notice(&run, &error, |_, _| {});
        assert_eq!(notice_title(&notice), Some("Could not checkout deadbeef"));
        assert_eq!(&*notice.message, "Revision not found");
    }

    #[test]
    fn other_failures_show_the_git_message_even_through_error_context() {
        let run = StepRun::switch_branch("feature");
        let error = failure(GitFailureKind::Other, "  fatal: boom \n").context("while switching");
        let notice = failure_notice(&run, &error, |_, _| {});
        assert_eq!(&*notice.message, "fatal: boom");
        assert_eq!(notice_title(&notice), Some("Could not checkout feature"));
    }

    #[test]
    fn non_git_failures_show_the_whole_error_chain() {
        let run = StepRun::switch_branch("feature");
        let error = anyhow!("disk full").context("could not write");
        let notice = failure_notice(&run, &error, |_, _| {});
        assert_eq!(&*notice.message, "could not write: disk full");
    }

    #[test]
    fn creating_a_branch_with_checkout_reports_a_checked_out_new_branch() {
        let request = CreateBranchRequest {
            name: "feature".to_string(),
            start: "origin/main".to_string(),
            checkout: true,
            overwrite: true,
            peel_start: false,
        };
        let run = StepRun::create_branch(&request);
        assert_eq!(
            run.step,
            CheckoutStep::CreateBranch {
                name: "feature".to_string(),
                start_point: "origin/main".to_string(),
                checkout: true,
                overwrite: true,
            }
        );
        assert_eq!(
            run.completion,
            Completion::CheckedOutNewBranch {
                name: "feature".to_string(),
                start: "origin/main".to_string(),
            }
        );
        assert_eq!(run.failure_title, "Could not create new branch feature");
    }

    #[test]
    fn creating_a_branch_without_checkout_reports_a_created_branch() {
        let request = CreateBranchRequest {
            name: "feature".to_string(),
            start: "HEAD".to_string(),
            checkout: false,
            overwrite: false,
            peel_start: false,
        };
        let run = StepRun::create_branch(&request);
        assert_eq!(
            run.completion,
            Completion::BranchCreated {
                name: "feature".to_string()
            }
        );
    }

    #[test]
    fn the_branch_created_before_a_remote_rebase_is_silent_and_never_checked_out() {
        let run = StepRun::create_branch_before_rebase("feature", "origin/feature", true);
        assert_eq!(
            run.step,
            CheckoutStep::CreateBranch {
                name: "feature".to_string(),
                start_point: "origin/feature".to_string(),
                checkout: false,
                overwrite: true,
            }
        );
        assert_eq!(run.completion, Completion::Silent);
        assert_eq!(run.operation, SmartOperation::Rebase);
    }

    #[test]
    fn dialog_requests_keep_the_start_point_and_the_user_choices() {
        let request = CreateBranchRequest::from_dialog(
            NewBranchRequest {
                name: "feature".to_string(),
                checkout: false,
                overwrite: true,
            },
            "main".to_string(),
            false,
        );
        assert_eq!(
            request,
            CreateBranchRequest {
                name: "feature".to_string(),
                start: "main".to_string(),
                checkout: false,
                overwrite: true,
                peel_start: false,
            }
        );
    }

    #[test]
    fn a_peeled_start_point_never_names_a_remote_ref_so_git_does_not_set_tracking() {
        let request = CreateBranchRequest::from_dialog(
            NewBranchRequest {
                name: "feature".to_string(),
                checkout: true,
                overwrite: false,
            },
            "origin/main".to_string(),
            true,
        );
        assert_eq!(request.git_start_point(), "origin/main^0");
        let run = StepRun::create_branch(&request);
        assert_eq!(
            run.step,
            CheckoutStep::CreateBranch {
                name: "feature".to_string(),
                start_point: "origin/main^0".to_string(),
                checkout: true,
                overwrite: false,
            }
        );
        assert_eq!(
            run.completion,
            Completion::CheckedOutNewBranch {
                name: "feature".to_string(),
                start: "origin/main".to_string(),
            }
        );
    }

    #[test]
    fn checking_out_a_remote_branch_keeps_the_plain_start_point_so_tracking_is_set() {
        let request = CreateBranchRequest::from_dialog(
            NewBranchRequest {
                name: "main".to_string(),
                checkout: true,
                overwrite: false,
            },
            "origin/main".to_string(),
            false,
        );
        assert_eq!(request.git_start_point(), "origin/main");
    }

    #[test]
    fn only_rebase_steps_stash_local_changes_before_running() {
        assert!(stashes_local_changes_first(&CheckoutStep::RebaseOnto {
            upstream: "HEAD".to_string(),
            branch: "feature".to_string(),
        }));
        assert!(!stashes_local_changes_first(&CheckoutStep::SwitchBranch {
            name: "feature".to_string(),
        }));
        assert!(!stashes_local_changes_first(&CheckoutStep::Detach {
            revision: "v1".to_string(),
        }));
        assert!(!stashes_local_changes_first(&CheckoutStep::CreateBranch {
            name: "feature".to_string(),
            start_point: "main".to_string(),
            checkout: false,
            overwrite: false,
        }));
    }

    #[test]
    fn forcing_a_branch_creation_resets_only_when_it_overwrites_the_current_branch() {
        assert!(force_resets_current_branch("main", true, Some("main")));
        assert!(!force_resets_current_branch("main", false, Some("main")));
        assert!(!force_resets_current_branch("main", true, Some("other")));
        assert!(!force_resets_current_branch("main", true, None));
    }

    #[test]
    fn only_an_existing_different_upstream_counts_as_tracking_a_different_remote() {
        assert!(tracks_different_remote(Some("origin/other"), "origin/main"));
        assert!(!tracks_different_remote(Some("origin/main"), "origin/main"));
        assert!(!tracks_different_remote(None, "origin/main"));
    }

    #[test]
    fn checkout_prompt_offers_rebase_drop_and_cancel_in_that_order() {
        let prompt = LocalCommitsPrompt::rebase_or_drop("main", "origin/main");
        assert_eq!(prompt.title, "Checkout origin/main");
        assert_eq!(
            prompt.detail,
            "Local branch 'main' has commits that do not exist in 'origin/main'. Rebase 'main' \
             onto 'origin/main', or drop local commits?"
        );
        assert_eq!(
            prompt.answers,
            vec![
                ("Rebase onto Remote", LocalCommitsAnswer::Rebase),
                ("Drop Local Commits", LocalCommitsAnswer::Drop),
                ("Cancel", LocalCommitsAnswer::Cancel),
            ]
        );
    }

    #[test]
    fn creating_without_checkout_only_offers_dropping_local_commits() {
        let request = CreateBranchRequest {
            name: "main".to_string(),
            start: "origin/main".to_string(),
            checkout: false,
            overwrite: true,
            peel_start: false,
        };
        let prompt = LocalCommitsPrompt::for_new_branch(&request);
        assert_eq!(prompt.title, "Create Branch from origin/main");
        assert_eq!(
            prompt.detail,
            "Local branch 'main' has commits that do not exist in 'origin/main'. Do you want to \
             drop local commits?"
        );
        assert_eq!(
            prompt.answers,
            vec![
                ("Drop Local Commits", LocalCommitsAnswer::Drop),
                ("Cancel", LocalCommitsAnswer::Cancel),
            ]
        );
    }

    #[test]
    fn creating_with_checkout_offers_rebase_as_well() {
        let request = CreateBranchRequest {
            name: "main".to_string(),
            start: "origin/main".to_string(),
            checkout: true,
            overwrite: true,
            peel_start: false,
        };
        let prompt = LocalCommitsPrompt::for_new_branch(&request);
        assert_eq!(prompt.answers.len(), 3);
        assert_eq!(prompt.answers[0].1, LocalCommitsAnswer::Rebase);
    }
}
