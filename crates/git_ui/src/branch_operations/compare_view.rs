use crate::{branch_refs::RefTarget, commit_view::CommitView};
use anyhow::{Result, anyhow};
use futures::channel::oneshot;
use git::repository::CommitSummary;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Subscription, Task,
    UniformListScrollHandle, WeakEntity, Window, uniform_list,
};
use project::git_store::{Repository, RepositoryEvent};
use std::sync::Arc;
use time::{OffsetDateTime, UtcOffset};
use ui::{Divider, ListItem, ListItemSpacing, WithScrollbar, prelude::*};
use util::ResultExt as _;
use workspace::{
    Workspace,
    item::{Item, ItemEvent},
};

pub const COMPARE_COMMIT_LIMIT: usize = 500;
const SHORT_SHA_LENGTH: usize = 8;
const CURRENT_HEAD_REVISION: &str = "HEAD";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompareRefs {
    pub selected_label: SharedString,
    pub selected_revision: String,
    pub current_label: SharedString,
    pub current_revision: String,
}

impl CompareRefs {
    pub fn new(selected: &RefTarget, current_branch: Option<&str>) -> Self {
        let (current_label, current_revision) = match current_branch {
            Some(name) => (name.to_string(), format!("refs/heads/{name}")),
            None => (
                CURRENT_HEAD_REVISION.to_string(),
                CURRENT_HEAD_REVISION.to_string(),
            ),
        };
        Self {
            selected_label: selected.name.clone(),
            selected_revision: selected.full_ref_name(),
            current_label: current_label.into(),
            current_revision,
        }
    }

    pub fn tab_title(&self) -> SharedString {
        format!("Compare {} and {}", self.selected_label, self.current_label).into()
    }

    pub fn selected_only_heading(&self) -> SharedString {
        format!(
            "Commits that exist in {} but don't exist in {}",
            self.selected_label, self.current_label
        )
        .into()
    }

    pub fn current_only_heading(&self) -> SharedString {
        format!(
            "Commits that exist in {} but don't exist in {}",
            self.current_label, self.selected_label
        )
        .into()
    }

    pub fn selected_only_empty_text(&self) -> SharedString {
        format!(
            "{} contains all commits from {}",
            self.current_label, self.selected_label
        )
        .into()
    }

    pub fn current_only_empty_text(&self) -> SharedString {
        format!(
            "{} contains all commits from {}",
            self.selected_label, self.current_label
        )
        .into()
    }
}

pub fn commit_count_label(count: usize) -> String {
    if count >= COMPARE_COMMIT_LIMIT {
        format!("{COMPARE_COMMIT_LIMIT}+")
    } else {
        count.to_string()
    }
}

pub fn short_sha(sha: &str) -> String {
    sha.chars().take(SHORT_SHA_LENGTH).collect()
}

#[derive(Clone, Debug)]
pub enum CommitList {
    Loading,
    Loaded(Arc<Vec<CommitSummary>>),
    Failed(SharedString),
}

impl CommitList {
    pub fn from_load_result(result: Result<Vec<CommitSummary>>) -> Self {
        match result {
            Ok(commits) => Self::Loaded(Arc::new(commits)),
            Err(error) => Self::Failed(format!("Could not load commits: {error}").into()),
        }
    }

    fn has_rows(&self) -> bool {
        matches!(self, Self::Loaded(commits) if !commits.is_empty())
    }

    fn count_label(&self) -> Option<String> {
        match self {
            Self::Loaded(commits) => Some(commit_count_label(commits.len())),
            Self::Loading | Self::Failed(_) => None,
        }
    }
}

fn flatten_load(
    result: std::result::Result<Result<Vec<CommitSummary>>, oneshot::Canceled>,
) -> Result<Vec<CommitSummary>> {
    match result {
        Ok(inner) => inner,
        Err(_) => Err(anyhow!("loading commits was cancelled")),
    }
}

fn format_commit_time(timestamp: i64) -> String {
    let commit_time = OffsetDateTime::from_unix_timestamp(timestamp)
        .unwrap_or_else(|_| OffsetDateTime::now_utc());
    let local_offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    time_format::format_localized_timestamp(
        commit_time,
        OffsetDateTime::now_utc(),
        local_offset,
        time_format::TimestampFormat::Relative,
    )
}

pub struct CompareView {
    refs: CompareRefs,
    repository: Entity<Repository>,
    workspace: WeakEntity<Workspace>,
    selected_only: CommitList,
    current_only: CommitList,
    selected_only_scroll: UniformListScrollHandle,
    current_only_scroll: UniformListScrollHandle,
    focus_handle: FocusHandle,
    _load_task: Task<()>,
    _repository_subscription: Subscription,
}

impl CompareView {
    pub fn new(
        refs: CompareRefs,
        repository: Entity<Repository>,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let repository_subscription =
            cx.subscribe(&repository, |this, _, event: &RepositoryEvent, cx| {
                if matches!(
                    event,
                    RepositoryEvent::HeadChanged | RepositoryEvent::BranchListChanged
                ) {
                    this.reload(cx);
                }
            });
        let mut view = Self {
            refs,
            repository,
            workspace,
            selected_only: CommitList::Loading,
            current_only: CommitList::Loading,
            selected_only_scroll: UniformListScrollHandle::new(),
            current_only_scroll: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            _load_task: Task::ready(()),
            _repository_subscription: repository_subscription,
        };
        view.reload(cx);
        view
    }

    pub fn refs(&self) -> &CompareRefs {
        &self.refs
    }

    pub fn repository(&self) -> &Entity<Repository> {
        &self.repository
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let selected_revision = self.refs.selected_revision.clone();
        let current_revision = self.refs.current_revision.clone();
        let selected_only = self.repository.update(cx, |repository, _| {
            repository.commits_between(
                current_revision.clone(),
                selected_revision.clone(),
                COMPARE_COMMIT_LIMIT,
            )
        });
        let current_only = self.repository.update(cx, |repository, _| {
            repository.commits_between(selected_revision, current_revision, COMPARE_COMMIT_LIMIT)
        });
        self._load_task = cx.spawn(async move |this, cx| {
            let (selected_only, current_only) = futures::join!(selected_only, current_only);
            let selected_only = CommitList::from_load_result(flatten_load(selected_only));
            let current_only = CommitList::from_load_result(flatten_load(current_only));
            this.update(cx, |this, cx| {
                this.selected_only = selected_only;
                this.current_only = current_only;
                cx.notify();
            })
            .log_err();
        });
    }

    fn render_commit_row(
        id_prefix: &'static str,
        index: usize,
        commit: &CommitSummary,
        repository: &WeakEntity<Repository>,
        workspace: &WeakEntity<Workspace>,
    ) -> ListItem {
        let sha = commit.sha.clone();
        let repository = repository.clone();
        let workspace = workspace.clone();
        let author_and_time = format!(
            "{} {}",
            commit.author_name,
            format_commit_time(commit.commit_timestamp)
        );
        ListItem::new((id_prefix, index))
            .inset(true)
            .spacing(ListItemSpacing::Dense)
            .start_slot(
                Icon::new(IconName::GitCommit)
                    .size(IconSize::Small)
                    .color(Color::Muted),
            )
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .child(
                        Label::new(short_sha(&commit.sha))
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Label::new(commit.subject.clone()).truncate()),
                    )
                    .child(
                        Label::new(author_and_time)
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .on_click(move |_, window, cx| {
                CommitView::open(
                    sha.to_string(),
                    repository.clone(),
                    workspace.clone(),
                    None,
                    None,
                    window,
                    cx,
                );
            })
    }

    fn render_list(
        &self,
        id_prefix: &'static str,
        list: &CommitList,
        empty_text: SharedString,
        scroll_handle: &UniformListScrollHandle,
    ) -> gpui::AnyElement {
        let message = |text: SharedString, color: Color| {
            h_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(Label::new(text).color(color))
                .into_any_element()
        };
        match list {
            CommitList::Loading => message("Loading commits…".into(), Color::Muted),
            CommitList::Failed(error) => message(error.clone(), Color::Error),
            CommitList::Loaded(commits) if commits.is_empty() => message(empty_text, Color::Muted),
            CommitList::Loaded(commits) => {
                let commits = commits.clone();
                let repository = self.repository.downgrade();
                let workspace = self.workspace.clone();
                uniform_list(id_prefix, commits.len(), move |range, _window, _cx| {
                    range
                        .filter_map(|index| {
                            commits.get(index).map(|commit| {
                                Self::render_commit_row(
                                    id_prefix,
                                    index,
                                    commit,
                                    &repository,
                                    &workspace,
                                )
                            })
                        })
                        .collect()
                })
                .size_full()
                .track_scroll(scroll_handle)
                .into_any_element()
            }
        }
    }

    fn render_heading(heading: SharedString, list: &CommitList, cx: &App) -> Div {
        h_flex()
            .w_full()
            .px_2()
            .py_1()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .child(Label::new(heading).size(LabelSize::Small))
            .when_some(list.count_label(), |this, count| {
                this.child(Label::new(count).size(LabelSize::Small).color(Color::Muted))
            })
    }
}

impl EventEmitter<ItemEvent> for CompareView {}

impl Focusable for CompareView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Item for CompareView {
    type Event = ItemEvent;

    fn tab_icon(&self, _window: &Window, _cx: &App) -> Option<Icon> {
        Some(Icon::new(IconName::GitBranch).color(Color::Muted))
    }

    fn tab_content_text(&self, _detail: usize, _cx: &App) -> SharedString {
        self.refs.tab_title()
    }

    fn tab_tooltip_text(&self, _cx: &App) -> Option<SharedString> {
        Some(self.refs.tab_title())
    }

    fn show_toolbar(&self) -> bool {
        false
    }
}

impl Render for CompareView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_only_heading =
            Self::render_heading(self.refs.selected_only_heading(), &self.selected_only, cx);
        let current_only_heading =
            Self::render_heading(self.refs.current_only_heading(), &self.current_only, cx);

        let mut selected_only_body = div()
            .id("compare-selected-only-body")
            .flex_1()
            .min_h_0()
            .child(self.render_list(
                "compare-selected-only-list",
                &self.selected_only,
                self.refs.selected_only_empty_text(),
                &self.selected_only_scroll,
            ));
        if self.selected_only.has_rows() {
            selected_only_body =
                selected_only_body.vertical_scrollbar_for(&self.selected_only_scroll, window, cx);
        }

        let mut current_only_body = div()
            .id("compare-current-only-body")
            .flex_1()
            .min_h_0()
            .child(self.render_list(
                "compare-current-only-list",
                &self.current_only,
                self.refs.current_only_empty_text(),
                &self.current_only_scroll,
            ));
        if self.current_only.has_rows() {
            current_only_body =
                current_only_body.vertical_scrollbar_for(&self.current_only_scroll, window, cx);
        }

        v_flex()
            .id("compare-view")
            .key_context("CompareView")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().colors().editor_background)
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .child(selected_only_heading)
                    .child(selected_only_body),
            )
            .child(Divider::horizontal())
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .child(current_only_heading)
                    .child(current_only_body),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(sha: &str) -> CommitSummary {
        CommitSummary {
            sha: sha.to_string().into(),
            subject: "subject".into(),
            commit_timestamp: 0,
            author_name: "author".into(),
            has_parent: true,
        }
    }

    #[test]
    fn refs_for_a_local_branch_compare_against_the_current_branch_tip() {
        let refs = CompareRefs::new(&RefTarget::local("feature/x"), Some("main"));
        assert_eq!(refs.selected_revision, "refs/heads/feature/x");
        assert_eq!(refs.current_revision, "refs/heads/main");
        assert_eq!(refs.tab_title().as_ref(), "Compare feature/x and main");
    }

    #[test]
    fn refs_for_a_remote_branch_and_a_tag_use_their_full_ref_names() {
        let remote = CompareRefs::new(&RefTarget::remote("origin/main"), Some("main"));
        assert_eq!(remote.selected_revision, "refs/remotes/origin/main");
        assert_eq!(remote.selected_label.as_ref(), "origin/main");

        let tag = CompareRefs::new(&RefTarget::tag("v1.0"), Some("main"));
        assert_eq!(tag.selected_revision, "refs/tags/v1.0");
    }

    #[test]
    fn refs_fall_back_to_head_when_no_branch_is_checked_out() {
        let refs = CompareRefs::new(&RefTarget::local("main"), None);
        assert_eq!(refs.current_revision, "HEAD");
        assert_eq!(refs.current_label.as_ref(), "HEAD");
        assert_eq!(refs.tab_title().as_ref(), "Compare main and HEAD");
    }

    #[test]
    fn headings_and_empty_texts_name_the_side_that_is_missing_commits() {
        let refs = CompareRefs::new(&RefTarget::local("feature"), Some("main"));
        assert_eq!(
            refs.selected_only_heading().as_ref(),
            "Commits that exist in feature but don't exist in main"
        );
        assert_eq!(
            refs.current_only_heading().as_ref(),
            "Commits that exist in main but don't exist in feature"
        );
        assert_eq!(
            refs.selected_only_empty_text().as_ref(),
            "main contains all commits from feature"
        );
        assert_eq!(
            refs.current_only_empty_text().as_ref(),
            "feature contains all commits from main"
        );
    }

    #[test]
    fn commit_count_label_marks_a_truncated_list() {
        assert_eq!(commit_count_label(0), "0");
        assert_eq!(commit_count_label(COMPARE_COMMIT_LIMIT - 1), "499");
        assert_eq!(commit_count_label(COMPARE_COMMIT_LIMIT), "500+");
    }

    #[test]
    fn short_sha_keeps_the_first_eight_characters() {
        assert_eq!(short_sha("0123456789abcdef"), "01234567");
        assert_eq!(short_sha("abc"), "abc");
    }

    #[test]
    fn load_failures_are_reported_in_the_list_instead_of_an_empty_state() {
        let failed = CommitList::from_load_result(Err(anyhow!("bad revision")));
        match failed {
            CommitList::Failed(message) => {
                assert_eq!(message.as_ref(), "Could not load commits: bad revision")
            }
            CommitList::Loading | CommitList::Loaded(_) => panic!("expected a failed list"),
        }
    }

    #[test]
    fn loaded_lists_only_render_rows_when_they_contain_commits() {
        assert!(!CommitList::Loading.has_rows());
        assert!(!CommitList::from_load_result(Ok(Vec::new())).has_rows());
        assert!(CommitList::from_load_result(Ok(vec![commit("abc")])).has_rows());
    }

    #[test]
    fn cancelled_loads_become_failures() {
        let cancelled: std::result::Result<Result<Vec<CommitSummary>>, oneshot::Canceled> =
            Err(oneshot::Canceled);
        assert!(flatten_load(cancelled).is_err());
    }
}
