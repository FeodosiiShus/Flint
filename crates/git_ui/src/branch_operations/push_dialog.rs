use std::time::Duration;

use db::kvp::KeyValueStore;
use editor::{Editor, EditorEvent};
use git::repository::{CommitSummary, PushOptions, Remote, Upstream};
use gpui::{
    AnyElement, App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, Render, SharedString, Subscription, Window,
};
use menu::{Cancel, Confirm};
use ui::{
    Checkbox, DialogButton, DialogWidth, GatedSurface as _, Modal, ModalFooter, ModalHeader,
    ReadyGate, Section, Tooltip, prelude::*,
};
use util::ResultExt as _;
use workspace::ModalView;

use super::delete_dialogs::render_commit_list;
use super::integrate::{is_protected_branch_name, protected_branch_patterns};
use super::manage::{notify_failure, settle};
use super::push_result::{PushTarget, push_failed_notice, push_succeeded_notice};
use super::{BranchContext, OperationProgress, PUSH_PROGRESS_TITLE};
use crate::branch_refs::RefTarget;

const REFRESH_DEBOUNCE: Duration = Duration::from_millis(150);
const PUSH_COMMIT_LIMIT: usize = 100;
const FORCE_PUSH_TITLE: &str = "Force Push";
const DONT_WARN_LABEL: &str = "Don't warn about this target";
const SILENT_FORCE_PUSH_KEY_PREFIX: &str = "git_silent_force_push";

pub fn force_push_prohibited_message(target: &str) -> String {
    format!("Force push to {target} is prohibited")
}

pub fn force_push_confirmation_message(target: &str) -> String {
    format!(
        "You're going to force push to '{target}'. It may overwrite commits at the remote. Are you sure you want to proceed?"
    )
}

fn silent_force_push_key(repository: &std::path::Path, target: &str) -> String {
    format!(
        "{SILENT_FORCE_PUSH_KEY_PREFIX}:{}:{target}",
        repository.display()
    )
}

fn is_silent_force_push_allowed(repository: &std::path::Path, target: &str, cx: &App) -> bool {
    KeyValueStore::global(cx)
        .read_kvp(&silent_force_push_key(repository, target))
        .log_err()
        .flatten()
        .is_some()
}

fn save_silent_force_push_target(repository: &std::path::Path, target: &str, cx: &App) {
    let key = silent_force_push_key(repository, target);
    let store = KeyValueStore::global(cx);
    db::write_and_log(cx, move || async move {
        store.write_kvp(key, "true".to_string()).await
    });
}

pub fn needs_upstream(upstream: Option<&Upstream>) -> bool {
    match upstream {
        None => true,
        Some(upstream) => upstream.tracking.is_gone(),
    }
}

pub fn push_options(upstream: Option<&Upstream>, force: bool) -> Option<PushOptions> {
    if force {
        return Some(PushOptions::Force);
    }
    needs_upstream(upstream).then_some(PushOptions::SetUpstream)
}

pub fn choose_default_remote(preferred: &[String], all: &[String]) -> Option<String> {
    if let [only] = preferred {
        return Some(only.clone());
    }
    all.iter()
        .find(|remote| remote.as_str() == "origin")
        .or_else(|| all.first())
        .cloned()
}

pub fn direct_outgoing_base(
    remote: &str,
    remote_branch: &str,
    remote_branches: &[String],
) -> Option<String> {
    let candidate = format!("{remote}/{remote_branch}");
    remote_branches.contains(&candidate).then_some(candidate)
}

pub fn default_outgoing_base(
    remote: &str,
    default_branch: Option<&str>,
    remote_branches: &[String],
) -> Option<String> {
    let default_branch = default_branch?;
    let belongs_to_remote = default_branch
        .strip_prefix(remote)
        .is_some_and(|rest| rest.starts_with('/'));
    (belongs_to_remote
        && remote_branches
            .iter()
            .any(|branch| branch == default_branch))
    .then(|| default_branch.to_string())
}

pub fn commits_to_push_label(count: usize, truncated: bool) -> String {
    let plus = if truncated { "+" } else { "" };
    match count {
        1 => "1 commit to push".to_string(),
        _ => format!("{count}{plus} commits to push"),
    }
}

enum OutgoingCommits {
    Loading,
    NoRemote,
    UnknownBase,
    Loaded {
        commits: Vec<CommitSummary>,
        truncated: bool,
        relative_to_target: bool,
    },
    Failed(SharedString),
}

impl OutgoingCommits {
    fn pushed_commit_count(&self) -> Option<usize> {
        match self {
            OutgoingCommits::Loaded {
                commits,
                truncated: false,
                relative_to_target: true,
            } => Some(commits.len()),
            _ => None,
        }
    }
}

pub struct PushDialog {
    context: BranchContext,
    branch: SharedString,
    upstream: Option<Upstream>,
    remotes: Vec<SharedString>,
    remote: Option<SharedString>,
    remote_branch_editor: Entity<Editor>,
    outgoing: OutgoingCommits,
    refresh_generation: usize,
    force_confirmation: Option<ForcePushConfirmation>,
    confirmation_focus_handle: FocusHandle,
    /// The outgoing commits load after the dialog opens and change its height.
    gate: ReadyGate,
    _editor_subscription: Subscription,
}

struct ForcePushConfirmation {
    target: SharedString,
    dont_warn: bool,
}

impl PushDialog {
    pub fn new(
        context: BranchContext,
        branch: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let upstream = context
            .find_branch(&RefTarget::local(branch.clone()), cx)
            .and_then(|found| found.upstream);
        let initial_remote = upstream
            .as_ref()
            .and_then(|upstream| upstream.remote_name())
            .map(|remote| SharedString::from(remote.to_string()));
        let initial_remote_branch = upstream
            .as_ref()
            .and_then(|upstream| upstream.branch_name())
            .map(str::to_string)
            .unwrap_or_else(|| branch.to_string());

        let remote_branch_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(initial_remote_branch, window, cx);
            editor
        });
        let editor_subscription = cx.subscribe_in(
            &remote_branch_editor,
            window,
            |this, _editor, event: &EditorEvent, _window, cx| {
                if matches!(event, EditorEvent::BufferEdited) {
                    this.refresh_outgoing(REFRESH_DEBOUNCE, cx);
                }
            },
        );

        let repository = context.repository.clone();
        let branch_name = branch.to_string();
        cx.spawn(async move |this, cx| {
            let all =
                settle(repository.update(cx, |repository, _| repository.get_remotes(None, false)))
                    .await
                    .log_err()
                    .unwrap_or_default();
            let preferred = settle(repository.update(cx, |repository, _| {
                repository.get_remotes(Some(branch_name), true)
            }))
            .await
            .log_err()
            .unwrap_or_default();
            this.update(cx, |this, cx| this.set_remotes(all, preferred, cx))
                .log_err();
        })
        .detach();

        Self {
            context,
            branch,
            upstream,
            remotes: Vec::new(),
            remote: initial_remote,
            remote_branch_editor,
            outgoing: OutgoingCommits::Loading,
            refresh_generation: 0,
            force_confirmation: None,
            confirmation_focus_handle: cx.focus_handle(),
            gate: ReadyGate::new(cx),
            _editor_subscription: editor_subscription,
        }
    }

    fn set_remotes(&mut self, all: Vec<Remote>, preferred: Vec<Remote>, cx: &mut Context<Self>) {
        let all_names: Vec<String> = all.iter().map(|remote| remote.name.to_string()).collect();
        let preferred_names: Vec<String> = preferred
            .iter()
            .map(|remote| remote.name.to_string())
            .collect();
        self.remotes = all.into_iter().map(|remote| remote.name).collect();
        if self.remote.is_none() {
            self.remote =
                choose_default_remote(&preferred_names, &all_names).map(SharedString::from);
        }
        self.refresh_outgoing(Duration::ZERO, cx);
    }

    fn remote_branch(&self, cx: &App) -> String {
        self.remote_branch_editor
            .read(cx)
            .text(cx)
            .trim()
            .to_string()
    }

    fn select_remote(&mut self, remote: SharedString, cx: &mut Context<Self>) {
        self.remote = Some(remote);
        self.refresh_outgoing(Duration::ZERO, cx);
    }

    /// Reloads the outgoing commits after `debounce`, which a dialog that has just opened has no
    /// use for: its commits are what it is waiting to show.
    fn refresh_outgoing(&mut self, debounce: Duration, cx: &mut Context<Self>) {
        self.refresh_generation += 1;
        let generation = self.refresh_generation;
        let Some(remote) = self.remote.clone() else {
            self.outgoing = OutgoingCommits::NoRemote;
            cx.notify();
            return;
        };
        self.outgoing = OutgoingCommits::Loading;
        cx.notify();

        let remote = remote.to_string();
        let remote_branch = self.remote_branch(cx);
        let branch = self.branch.to_string();
        let repository = self.context.repository.clone();
        let remote_branches: Vec<String> = self
            .context
            .snapshot(cx)
            .branch_list
            .iter()
            .filter(|candidate| candidate.is_remote())
            .map(|candidate| candidate.name().to_string())
            .collect();

        cx.spawn(async move |this, cx| {
            if !debounce.is_zero() {
                cx.background_executor().timer(debounce).await;
            }
            let is_current = this
                .update(cx, |this, _| this.refresh_generation == generation)
                .unwrap_or(false);
            if !is_current {
                return;
            }

            let direct_base = direct_outgoing_base(&remote, &remote_branch, &remote_branches);
            let relative_to_target = direct_base.is_some();
            let base = match direct_base {
                Some(base) => Some(base),
                None => {
                    let default_branch = settle(
                        repository.update(cx, |repository, _| repository.default_branch(true)),
                    )
                    .await
                    .log_err()
                    .flatten();
                    default_outgoing_base(&remote, default_branch.as_deref(), &remote_branches)
                }
            };

            let outgoing = match base {
                None => OutgoingCommits::UnknownBase,
                Some(base) => {
                    let commits = repository.update(cx, |repository, _| {
                        repository.commits_between(base, branch, PUSH_COMMIT_LIMIT + 1)
                    });
                    match settle(commits).await {
                        Ok(mut commits) => {
                            let truncated = commits.len() > PUSH_COMMIT_LIMIT;
                            commits.truncate(PUSH_COMMIT_LIMIT);
                            OutgoingCommits::Loaded {
                                commits,
                                truncated,
                                relative_to_target,
                            }
                        }
                        Err(error) => OutgoingCommits::Failed(format!("{error:#}").into()),
                    }
                }
            };

            this.update(cx, |this, cx| {
                if this.refresh_generation == generation {
                    this.outgoing = outgoing;
                    cx.notify();
                }
            })
            .log_err();
        })
        .detach();
    }

    fn can_push(&self, cx: &App) -> bool {
        self.remote.is_some() && !self.remote_branch(cx).is_empty()
    }

    fn force_push_target(&self, cx: &App) -> Option<String> {
        let remote = self.remote.as_ref()?;
        let remote_branch = self.remote_branch(cx);
        (!remote_branch.is_empty()).then(|| format!("{remote}/{remote_branch}"))
    }

    /// IntelliJ's `ForcePushAction` refuses targets on protected branches.
    fn force_push_prohibited(&self, cx: &App) -> bool {
        is_protected_branch_name(&self.remote_branch(cx), &protected_branch_patterns(cx))
    }

    pub(crate) fn request_force_push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_push(cx) || self.force_push_prohibited(cx) {
            return;
        }
        let Some(target) = self.force_push_target(cx) else {
            return;
        };
        let repository_path = self
            .context
            .repository
            .read(cx)
            .work_directory_abs_path
            .clone();
        if is_silent_force_push_allowed(&repository_path, &target, cx) {
            self.push(true, window, cx);
            return;
        }
        self.force_confirmation = Some(ForcePushConfirmation {
            target: target.into(),
            dont_warn: false,
        });
        // The confirmation replaces the branch editor, which held the focus, so Enter and
        // Escape need a focused element inside the confirmation to reach this dialog.
        window.focus(&self.confirmation_focus_handle, cx);
        cx.notify();
    }

    fn confirm_force_push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(confirmation) = self.force_confirmation.take() else {
            return;
        };
        if confirmation.dont_warn {
            let repository_path = self
                .context
                .repository
                .read(cx)
                .work_directory_abs_path
                .clone();
            save_silent_force_push_target(&repository_path, &confirmation.target, cx);
        }
        self.push(true, window, cx);
    }

    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.force_confirmation.take().is_some() {
            window.focus(&self.remote_branch_editor.focus_handle(cx), cx);
            cx.notify();
            return;
        }
        cx.emit(DismissEvent);
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.force_confirmation.is_some() {
            self.confirm_force_push(window, cx);
        } else {
            self.push(false, window, cx);
        }
    }

    #[cfg(test)]
    pub(crate) fn click_force_push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.request_force_push(window, cx);
    }

    #[cfg(test)]
    pub(crate) fn force_push_confirmation(&self) -> Option<(SharedString, bool)> {
        self.force_confirmation
            .as_ref()
            .map(|confirmation| (confirmation.target.clone(), confirmation.dont_warn))
    }

    #[cfg(test)]
    pub(crate) fn set_dont_warn(&mut self, dont_warn: bool) {
        if let Some(confirmation) = self.force_confirmation.as_mut() {
            confirmation.dont_warn = dont_warn;
        }
    }

    #[cfg(test)]
    pub(crate) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(&Confirm, window, cx);
    }

    #[cfg(test)]
    pub(crate) fn is_force_push_prohibited(&self, cx: &App) -> bool {
        self.force_push_prohibited(cx)
    }

    fn push(&mut self, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_push(cx) {
            return;
        }
        let Some(remote) = self.remote.clone() else {
            return;
        };
        let target = PushTarget {
            branch: self.branch.clone(),
            remote,
            remote_branch: SharedString::from(self.remote_branch(cx)),
        };
        let pushed_commits = self.outgoing.pushed_commit_count();
        let options = push_options(self.upstream.as_ref(), force);
        let upstream_to_set = (force && needs_upstream(self.upstream.as_ref()))
            .then(|| format!("{}/{}", target.remote, target.remote_branch));
        let context = self.context.clone();

        window
            .spawn(cx, async move |cx| {
                let started = cx
                    .update(|window, cx| {
                        let askpass = context.askpass_delegate(
                            format!("git push {}", target.remote),
                            window,
                            cx,
                        );
                        let (progress, progress_sender) = OperationProgress::start_reporting(
                            &context.workspace,
                            PUSH_PROGRESS_TITLE,
                            cx,
                        );
                        (askpass, progress, progress_sender)
                    })
                    .log_err();
                let Some((askpass, progress, progress_sender)) = started else {
                    return;
                };
                let receiver = context.repository.update(cx, |repository, cx| {
                    repository.push(
                        target.branch.clone(),
                        target.remote_branch.clone(),
                        target.remote.clone(),
                        options,
                        askpass,
                        Some(progress_sender),
                        cx,
                    )
                });
                let result = settle(receiver).await;
                drop(progress);
                let upstream_error = match (&result, upstream_to_set) {
                    (Ok(_), Some(upstream)) => {
                        let set_upstream = context.repository.update(cx, |repository, _| {
                            repository.set_upstream(target.branch.to_string(), Some(upstream))
                        });
                        settle(set_upstream).await.err()
                    }
                    _ => None,
                };
                cx.update(|_window, cx| {
                    let notice = match result {
                        Ok(output) => Some(push_succeeded_notice(&target, pushed_commits, &output)),
                        Err(error) => {
                            push_failed_notice(&target, &error, context.workspace.clone())
                        }
                    };
                    if let Some(notice) = notice {
                        context.notify(notice, cx);
                    }
                    if let Some(error) = upstream_error {
                        notify_failure(&context, "Could not set upstream", error, cx);
                    }
                })
                .log_err();
            })
            .detach();
        cx.emit(DismissEvent);
    }

    fn render_remote_selector(&self, cx: &Context<Self>) -> AnyElement {
        if self.remotes.is_empty() {
            let label = self
                .remote
                .clone()
                .unwrap_or_else(|| SharedString::from("no remote"));
            return Label::new(label).color(Color::Muted).into_any_element();
        }
        h_flex()
            .gap_1()
            .children(self.remotes.iter().enumerate().map(|(index, remote)| {
                let selected = self.remote.as_ref() == Some(remote);
                let remote = remote.clone();
                Button::new(("push-remote", index), remote.clone())
                    .style(ButtonStyle::Subtle)
                    .toggle_state(selected)
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.select_remote(remote.clone(), cx);
                    }))
            }))
            .into_any_element()
    }

    fn render_outgoing(&self) -> AnyElement {
        match &self.outgoing {
            OutgoingCommits::Loading => Label::new("Loading commits\u{2026}")
                .size(LabelSize::Small)
                .color(Color::Muted)
                .into_any_element(),
            OutgoingCommits::NoRemote => {
                Label::new("No remote is configured for this repository. Add a remote to push.")
                    .size(LabelSize::Small)
                    .color(Color::Warning)
                    .into_any_element()
            }
            OutgoingCommits::UnknownBase => Label::new(format!(
                "There is no matching remote branch yet. All commits of {} will be pushed.",
                self.branch
            ))
            .size(LabelSize::Small)
            .color(Color::Muted)
            .into_any_element(),
            OutgoingCommits::Failed(message) => Label::new(message.clone())
                .size(LabelSize::Small)
                .color(Color::Error)
                .into_any_element(),
            OutgoingCommits::Loaded {
                commits, truncated, ..
            } => v_flex()
                .gap_1()
                .child(
                    Label::new(commits_to_push_label(commits.len(), *truncated))
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
                .child(render_commit_list(
                    "push-commits",
                    commits,
                    "There are no commits to push",
                ))
                .into_any_element(),
        }
    }
}

impl EventEmitter<DismissEvent> for PushDialog {}

impl ModalView for PushDialog {
    fn surface_ready(&self) -> bool {
        self.gate.is_revealed()
    }
}

impl Focusable for PushDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.force_confirmation.is_some() {
            return self.confirmation_focus_handle.clone();
        }
        self.remote_branch_editor.focus_handle(cx)
    }
}

impl PushDialog {
    fn render_force_confirmation(
        &self,
        confirmation: &ForcePushConfirmation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .key_context("PushDialog")
            .track_focus(&self.confirmation_focus_handle)
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .dialog_surface(DialogWidth::Medium, window, cx)
            .child(
                Modal::new("force-push-confirmation", None)
                    .header(
                        ModalHeader::new()
                            .icon(
                                Icon::new(IconName::Warning)
                                    .size(IconSize::Small)
                                    .color(Color::Warning),
                            )
                            .headline(FORCE_PUSH_TITLE)
                            .show_dismiss_button(true),
                    )
                    .section(
                        Section::new().child(Label::new(force_push_confirmation_message(
                            &confirmation.target,
                        ))),
                    )
                    .footer(
                        ModalFooter::new()
                            .start_slot(
                                Checkbox::new(
                                    "force-push-dont-warn",
                                    confirmation.dont_warn.into(),
                                )
                                .label(DONT_WARN_LABEL)
                                .on_click(cx.listener(
                                    |this, state: &ToggleState, _, cx| {
                                        if let Some(confirmation) = this.force_confirmation.as_mut()
                                        {
                                            confirmation.dont_warn = state.selected();
                                        }
                                        cx.notify();
                                    },
                                )),
                            )
                            .end_button(
                                DialogButton::new("force-push-cancel", "Cancel")
                                    .default_button(false)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.cancel(&Cancel, window, cx);
                                    })),
                            )
                            .end_button(
                                DialogButton::new("force-push-confirm", FORCE_PUSH_TITLE)
                                    .default_button(true)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm_force_push(window, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for PushDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(confirmation) = self.force_confirmation.as_ref() {
            return self.render_force_confirmation(confirmation, window, cx);
        }
        self.gate
            .reveal_when(!matches!(self.outgoing, OutgoingCommits::Loading));
        let can_push = self.can_push(cx);
        let force_prohibited = can_push && self.force_push_prohibited(cx);
        let force_target = self.force_push_target(cx).unwrap_or_default();
        let remote_selector = self.render_remote_selector(cx);
        let outgoing = self.render_outgoing();
        let colors = cx.theme().colors();

        v_flex()
            .key_context("PushDialog")
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .dialog_surface(DialogWidth::Large, window, cx)
            .hidden_until(self.gate.is_revealed())
            .child(
                Modal::new("push-dialog", None)
                    .header(
                        ModalHeader::new()
                            .icon(
                                Icon::new(IconName::ArrowUp)
                                    .size(IconSize::Small)
                                    .color(Color::Muted),
                            )
                            .headline(format!("Push {}", self.branch))
                            .show_dismiss_button(true),
                    )
                    .section(
                        Section::new()
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        Icon::new(IconName::GitBranch)
                                            .size(IconSize::Small)
                                            .color(Color::Muted),
                                    )
                                    .child(Label::new(self.branch.clone()))
                                    .child(
                                        Icon::new(IconName::ArrowRight)
                                            .size(IconSize::XSmall)
                                            .color(Color::Muted),
                                    )
                                    .child(remote_selector)
                                    .child(Label::new("/").color(Color::Muted))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .px_2()
                                            .py_1()
                                            .rounded(ui::component_corner_radii(cx).input)
                                            .border_1()
                                            .border_color(colors.border)
                                            .bg(colors.editor_background)
                                            .child(self.remote_branch_editor.clone()),
                                    ),
                            )
                            .child(outgoing),
                    )
                    .footer(
                        ModalFooter::new()
                            .end_button(
                                DialogButton::new("push-cancel", "Cancel")
                                    .default_button(false)
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(DismissEvent);
                                    })),
                            )
                            .end_button(
                                DialogButton::new("push-force", FORCE_PUSH_TITLE)
                                    .default_button(false)
                                    .disabled(!can_push || force_prohibited)
                                    .when(force_prohibited, |button| {
                                        button.tooltip(Tooltip::text(
                                            force_push_prohibited_message(&force_target),
                                        ))
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.request_force_push(window, cx);
                                    })),
                            )
                            .end_button(
                                DialogButton::new("push-confirm", "Push")
                                    .default_button(true)
                                    .disabled(!can_push)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.push(false, window, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git::repository::{UpstreamTracking, UpstreamTrackingStatus};

    fn upstream(tracking: UpstreamTracking) -> Upstream {
        Upstream {
            ref_name: SharedString::from("refs/remotes/origin/main"),
            tracking,
        }
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn commits(count: usize) -> Vec<CommitSummary> {
        (0..count)
            .map(|index| CommitSummary {
                sha: SharedString::from(format!("{index:040}")),
                subject: SharedString::from("subject"),
                commit_timestamp: 0,
                author_name: SharedString::from("author"),
                has_parent: true,
            })
            .collect()
    }

    #[test]
    fn force_push_texts_follow_the_intellij_push_dialog() {
        assert_eq!(
            force_push_prohibited_message("origin/main"),
            "Force push to origin/main is prohibited"
        );
        assert_eq!(
            force_push_confirmation_message("origin/feature"),
            "You're going to force push to 'origin/feature'. It may overwrite commits at the remote. Are you sure you want to proceed?"
        );
    }

    #[test]
    fn force_always_wins_over_set_upstream() {
        assert_eq!(push_options(None, true), Some(PushOptions::Force));
    }

    #[test]
    fn branch_without_upstream_sets_one() {
        assert_eq!(push_options(None, false), Some(PushOptions::SetUpstream));
    }

    #[test]
    fn gone_upstream_is_replaced() {
        let gone = upstream(UpstreamTracking::Gone);
        assert_eq!(
            push_options(Some(&gone), false),
            Some(PushOptions::SetUpstream)
        );
    }

    #[test]
    fn tracked_upstream_is_kept() {
        let tracked = upstream(UpstreamTracking::Tracked(UpstreamTrackingStatus {
            ahead: 1,
            behind: 0,
        }));
        assert_eq!(push_options(Some(&tracked), false), None);
    }

    #[test]
    fn upstream_is_needed_when_missing_or_gone_only() {
        let tracked = upstream(UpstreamTracking::Tracked(UpstreamTrackingStatus {
            ahead: 0,
            behind: 0,
        }));
        assert!(needs_upstream(None));
        assert!(needs_upstream(Some(&upstream(UpstreamTracking::Gone))));
        assert!(!needs_upstream(Some(&tracked)));
    }

    #[test]
    fn single_preferred_remote_wins() {
        assert_eq!(
            choose_default_remote(&strings(&["fork"]), &strings(&["origin", "fork"])),
            Some("fork".to_string())
        );
    }

    #[test]
    fn origin_is_preferred_among_several_remotes() {
        assert_eq!(
            choose_default_remote(&strings(&["fork", "origin"]), &strings(&["fork", "origin"])),
            Some("origin".to_string())
        );
    }

    #[test]
    fn first_remote_is_used_when_there_is_no_origin() {
        assert_eq!(
            choose_default_remote(&strings(&["b", "a"]), &strings(&["b", "a"])),
            Some("b".to_string())
        );
    }

    #[test]
    fn no_remotes_means_no_default_remote() {
        assert_eq!(choose_default_remote(&[], &[]), None);
    }

    #[test]
    fn outgoing_base_prefers_the_same_named_remote_branch() {
        let branches = strings(&["origin/feature", "origin/main"]);
        assert_eq!(
            direct_outgoing_base("origin", "feature", &branches),
            Some("origin/feature".to_string())
        );
        assert_eq!(direct_outgoing_base("origin", "other", &branches), None);
        assert_eq!(direct_outgoing_base("fork", "feature", &branches), None);
    }

    #[test]
    fn default_outgoing_base_requires_a_branch_of_the_selected_remote() {
        let branches = strings(&["origin/main", "fork/main"]);
        assert_eq!(
            default_outgoing_base("origin", Some("origin/main"), &branches),
            Some("origin/main".to_string())
        );
        assert_eq!(
            default_outgoing_base("fork", Some("origin/main"), &branches),
            None
        );
        assert_eq!(default_outgoing_base("origin", None, &branches), None);
        assert_eq!(
            default_outgoing_base("origin", Some("origin/gone"), &branches),
            None
        );
    }

    #[test]
    fn default_outgoing_base_does_not_match_remotes_sharing_a_prefix() {
        let branches = strings(&["origin2/main"]);
        assert_eq!(
            default_outgoing_base("origin", Some("origin2/main"), &branches),
            None
        );
    }

    #[test]
    fn commit_label_pluralizes_and_marks_truncation() {
        assert_eq!(commits_to_push_label(1, false), "1 commit to push");
        assert_eq!(commits_to_push_label(0, false), "0 commits to push");
        assert_eq!(commits_to_push_label(3, false), "3 commits to push");
        assert_eq!(commits_to_push_label(100, true), "100+ commits to push");
    }

    #[test]
    fn pushed_commit_count_is_known_only_for_a_complete_list_against_the_target_branch() {
        let against_target = OutgoingCommits::Loaded {
            commits: commits(3),
            truncated: false,
            relative_to_target: true,
        };
        assert_eq!(against_target.pushed_commit_count(), Some(3));

        let truncated = OutgoingCommits::Loaded {
            commits: commits(PUSH_COMMIT_LIMIT),
            truncated: true,
            relative_to_target: true,
        };
        assert_eq!(truncated.pushed_commit_count(), None);

        let against_default_branch = OutgoingCommits::Loaded {
            commits: commits(2),
            truncated: false,
            relative_to_target: false,
        };
        assert_eq!(against_default_branch.pushed_commit_count(), None);

        for unknown in [
            OutgoingCommits::Loading,
            OutgoingCommits::NoRemote,
            OutgoingCommits::UnknownBase,
            OutgoingCommits::Failed(SharedString::from("fatal: bad revision")),
        ] {
            assert_eq!(unknown.pushed_commit_count(), None);
        }
    }
}
