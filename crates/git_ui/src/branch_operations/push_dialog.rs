use std::time::Duration;

use editor::{Editor, EditorEvent};
use git::repository::{CommitSummary, PushOptions, Remote, Upstream};
use gpui::{
    AnyElement, App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, Render, SharedString, Subscription, Window, rems,
};
use menu::{Cancel, Confirm};
use ui::{Modal, ModalFooter, ModalHeader, Section, TintColor, prelude::*};
use util::ResultExt as _;
use workspace::ModalView;

use super::delete_dialogs::render_commit_list;
use super::manage::{notify_failure, push_result_notice, settle};
use super::{BranchContext, opaque_elevated_surface};
use crate::branch_refs::RefTarget;

const DIALOG_WIDTH: f32 = 44.;
const REFRESH_DEBOUNCE: Duration = Duration::from_millis(150);
const PUSH_COMMIT_LIMIT: usize = 100;

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
    remote_branches
        .iter()
        .any(|branch| *branch == candidate)
        .then_some(candidate)
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
    },
    Failed(SharedString),
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
    _editor_subscription: Subscription,
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
                    this.refresh_outgoing(cx);
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
        self.refresh_outgoing(cx);
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
        self.refresh_outgoing(cx);
    }

    fn refresh_outgoing(&mut self, cx: &mut Context<Self>) {
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
            cx.background_executor().timer(REFRESH_DEBOUNCE).await;
            let is_current = this
                .update(cx, |this, _| this.refresh_generation == generation)
                .unwrap_or(false);
            if !is_current {
                return;
            }

            let base = match direct_outgoing_base(&remote, &remote_branch, &remote_branches) {
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
                            OutgoingCommits::Loaded { commits, truncated }
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

    fn cancel(&mut self, _: &Cancel, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        self.push(false, window, cx);
    }

    fn push(&mut self, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_push(cx) {
            return;
        }
        let Some(remote) = self.remote.clone() else {
            return;
        };
        let remote_branch = SharedString::from(self.remote_branch(cx));
        let options = push_options(self.upstream.as_ref(), force);
        let upstream_to_set = (force && needs_upstream(self.upstream.as_ref()))
            .then(|| format!("{remote}/{remote_branch}"));
        let context = self.context.clone();
        let branch = self.branch.clone();

        window
            .spawn(cx, async move |cx| {
                let askpass = cx
                    .update(|window, cx| {
                        context.askpass_delegate(format!("git push {remote}"), window, cx)
                    })
                    .log_err();
                let Some(askpass) = askpass else {
                    return;
                };
                let receiver = context.repository.update(cx, |repository, cx| {
                    repository.push(
                        branch.clone(),
                        remote_branch,
                        remote.clone(),
                        options,
                        askpass,
                        cx,
                    )
                });
                let result = settle(receiver).await;
                let upstream_error = match (&result, upstream_to_set) {
                    (Ok(_), Some(upstream)) => {
                        let set_upstream = context.repository.update(cx, |repository, _| {
                            repository.set_upstream(branch.to_string(), Some(upstream))
                        });
                        settle(set_upstream).await.err()
                    }
                    _ => None,
                };
                cx.update(|_window, cx| {
                    match result {
                        Ok(output) => {
                            let notice = push_result_notice(
                                context.workspace.clone(),
                                branch.clone(),
                                Remote {
                                    name: remote.clone(),
                                },
                                output,
                            );
                            context.notify(notice, cx);
                        }
                        Err(error) => notify_failure(&context, "Push failed", error, cx),
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
            OutgoingCommits::Loaded { commits, truncated } => v_flex()
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

impl ModalView for PushDialog {}

impl Focusable for PushDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.remote_branch_editor.focus_handle(cx)
    }
}

impl Render for PushDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can_push = self.can_push(cx);
        let remote_selector = self.render_remote_selector(cx);
        let outgoing = self.render_outgoing();
        let colors = cx.theme().colors();

        v_flex()
            .key_context("PushDialog")
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .w(rems(DIALOG_WIDTH))
            .elevation_3(cx)
            .bg(opaque_elevated_surface(cx))
            .overflow_hidden()
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
                                            .rounded_sm()
                                            .border_1()
                                            .border_color(colors.border)
                                            .bg(colors.editor_background)
                                            .child(self.remote_branch_editor.clone()),
                                    ),
                            )
                            .child(outgoing),
                    )
                    .footer(
                        ModalFooter::new().end_slot(
                            h_flex()
                                .gap_1()
                                .child(Button::new("push-cancel", "Cancel").on_click(cx.listener(
                                    |_, _, _, cx| {
                                        cx.emit(DismissEvent);
                                    },
                                )))
                                .child(
                                    Button::new("push-force", "Force Push")
                                        .style(ButtonStyle::Tinted(TintColor::Warning))
                                        .disabled(!can_push)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.push(true, window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("push-confirm", "Push")
                                        .style(ButtonStyle::Filled)
                                        .disabled(!can_push)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.push(false, window, cx);
                                        })),
                                ),
                        ),
                    ),
            )
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
}
