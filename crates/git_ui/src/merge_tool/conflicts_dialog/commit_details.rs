use std::collections::HashMap;

use anyhow::Result;
use git::repository::{CommitFileStatus, CommitSummary, RepoPath};
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, Render, SharedString, Task, Window, px, size,
};
use project::git_store::{CommitDiff, Repository};
use ui::{Checkbox, prelude::*};
use util::ResultExt as _;

use super::messages;
use crate::merge_tool::conflict_resolution::dialog_texts::ShowDetails;
use crate::merge_tool::dialog_window::{
    CloseDecision, CloseRequest, DialogOwner, DialogWindowContent, DialogWindowSpec,
    close_dialog_window, open_dialog_window,
};

const COMMIT_LIMIT: usize = 500;
const BOUNDS_KEY: &str = "MergeConflictCommitDetails";
const INITIAL_WIDTH: f32 = 760.;
const INITIAL_HEIGHT: f32 = 520.;
const MIN_WIDTH: f32 = 480.;
const MIN_HEIGHT: f32 = 320.;
const SHORT_HASH_LENGTH: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

impl ChangeKind {
    fn label(self) -> &'static str {
        match self {
            ChangeKind::Added => "Added",
            ChangeKind::Modified => "Modified",
            ChangeKind::Deleted => "Deleted",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ChangedFile {
    path: RepoPath,
    kind: ChangeKind,
}

fn short_hash(sha: &str) -> &str {
    sha.get(..SHORT_HASH_LENGTH).unwrap_or(sha)
}

fn changed_files(diff: &CommitDiff) -> Vec<ChangedFile> {
    diff.files
        .iter()
        .map(|file| ChangedFile {
            path: file.path.clone(),
            kind: match file.status() {
                CommitFileStatus::Added => ChangeKind::Added,
                CommitFileStatus::Modified => ChangeKind::Modified,
                CommitFileStatus::Deleted => ChangeKind::Deleted,
            },
        })
        .collect()
}

fn visible_commit_indices(
    commits: &[CommitSummary],
    changes: &HashMap<SharedString, Vec<ChangedFile>>,
    file: &RepoPath,
    filter_by_file: bool,
) -> Vec<usize> {
    commits
        .iter()
        .enumerate()
        .filter(|(_, commit)| {
            if !filter_by_file {
                return true;
            }
            changes
                .get(&commit.sha)
                .is_none_or(|files| files.iter().any(|changed| &changed.path == file))
        })
        .map(|(index, _)| index)
        .collect()
}

fn effective_selection(
    commits: &[CommitSummary],
    visible: &[usize],
    selected: Option<&SharedString>,
) -> Option<SharedString> {
    let selected_and_visible = selected.filter(|selected| {
        visible
            .iter()
            .any(|index| &commits[*index].sha == *selected)
    });
    selected_and_visible
        .cloned()
        .or_else(|| visible.first().map(|index| commits[*index].sha.clone()))
}

pub(super) fn open_commit_details(
    details: ShowDetails,
    repository: Entity<Repository>,
    file: RepoPath,
    window: &mut Window,
    cx: &mut App,
) {
    let owner = DialogOwner::dialog_window(window.window_handle());
    let title = match &details {
        ShowDetails::Commit { dialog_title, .. }
        | ShowDetails::CommitRange { dialog_title, .. } => dialog_title.clone(),
    };
    let spec = DialogWindowSpec {
        title,
        initial_size: size(px(INITIAL_WIDTH), px(INITIAL_HEIGHT)),
        min_size: size(px(MIN_WIDTH), px(MIN_HEIGHT)),
        bounds_key: BOUNDS_KEY,
    };
    open_dialog_window(
        spec,
        owner,
        cx,
        move |_, cx| cx.new(|cx| CommitDetailsDialog::new(details, repository, file, cx)),
        |opened, _| {
            if let Err(error) = opened {
                log::error!("could not open the commit details window: {error:#}");
            }
        },
    );
}

struct CommitDetailsDialog {
    repository: Entity<Repository>,
    file: RepoPath,
    is_range: bool,
    commits: Vec<CommitSummary>,
    changes: HashMap<SharedString, Vec<ChangedFile>>,
    filter_by_file: bool,
    selected: Option<SharedString>,
    loading: bool,
    error: Option<SharedString>,
    focus_handle: FocusHandle,
    _tasks: Vec<Task<()>>,
}

impl CommitDetailsDialog {
    fn new(
        details: ShowDetails,
        repository: Entity<Repository>,
        file: RepoPath,
        cx: &mut Context<Self>,
    ) -> Self {
        let is_range = matches!(details, ShowDetails::CommitRange { .. });
        let mut dialog = Self {
            repository,
            file,
            is_range,
            commits: Vec::new(),
            changes: HashMap::new(),
            filter_by_file: true,
            selected: None,
            loading: true,
            error: None,
            focus_handle: cx.focus_handle(),
            _tasks: Vec::new(),
        };
        let load = dialog.load_commits(details, cx);
        dialog._tasks.push(load);
        dialog
    }

    fn load_commits(&self, details: ShowDetails, cx: &mut Context<Self>) -> Task<()> {
        let repository = self.repository.clone();
        cx.spawn(async move |this, cx| {
            let commits: Result<Vec<CommitSummary>> = match details {
                ShowDetails::Commit { sha, .. } => {
                    let task = repository.read_with(cx, |repository, cx| {
                        repository.show_commit(sha.to_string(), cx)
                    });
                    task.await.map(|details| {
                        let subject = details
                            .message
                            .lines()
                            .next()
                            .unwrap_or_default()
                            .to_string();
                        vec![CommitSummary {
                            sha: details.sha,
                            subject: SharedString::from(subject),
                            commit_timestamp: details.commit_timestamp,
                            author_name: details.author_name,
                            has_parent: true,
                        }]
                    })
                }
                ShowDetails::CommitRange { first, second, .. } => {
                    let receiver = repository.update(cx, |repository, _| {
                        repository.commits_between(
                            first.to_string(),
                            second.to_string(),
                            COMMIT_LIMIT,
                        )
                    });
                    match receiver.await {
                        Ok(result) => result,
                        Err(error) => Err(anyhow::Error::new(error)),
                    }
                }
            };
            let shas = this
                .update(cx, |this, cx| {
                    this.loading = false;
                    match commits {
                        Ok(commits) => {
                            this.selected = commits.first().map(|commit| commit.sha.clone());
                            this.commits = commits;
                        }
                        Err(error) => {
                            this.error = Some(SharedString::from(format!("{error:#}")));
                        }
                    }
                    cx.notify();
                    this.commits
                        .iter()
                        .map(|commit| commit.sha.clone())
                        .collect::<Vec<_>>()
                })
                .log_err()
                .unwrap_or_default();
            for sha in shas {
                let diff = this
                    .update(cx, |this, cx| {
                        this.repository
                            .read(cx)
                            .load_commit_diff(sha.to_string(), false, cx)
                    })
                    .log_err();
                let Some(diff) = diff else {
                    return;
                };
                if let Some(diff) = diff.await.log_err() {
                    let files = changed_files(&diff);
                    this.update(cx, |this, cx| {
                        this.changes.insert(sha, files);
                        cx.notify();
                    })
                    .log_err();
                }
            }
        })
    }

    fn visible(&self) -> Vec<usize> {
        visible_commit_indices(
            &self.commits,
            &self.changes,
            &self.file,
            self.filter_by_file && self.is_range,
        )
    }

    fn render_commit_row(
        &self,
        index: usize,
        selection: Option<&SharedString>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let commit = &self.commits[index];
        let selected = selection == Some(&commit.sha);
        let sha = commit.sha.clone();
        let colors = cx.theme().colors();
        h_flex()
            .id(("commit-details-row", index))
            .w_full()
            .px_2()
            .py_1()
            .gap_2()
            .when(selected, |this| this.bg(colors.element_selected))
            .child(
                Label::new(short_hash(&commit.sha).to_string())
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(Label::new(commit.subject.clone()).truncate())
            .child(
                Label::new(commit.author_name.clone())
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = Some(sha.clone());
                cx.notify();
            }))
            .into_any_element()
    }

    fn render_files(&self, selection: Option<&SharedString>, cx: &App) -> AnyElement {
        let colors = cx.theme().colors();
        let files = selection
            .and_then(|sha| self.changes.get(sha))
            .cloned()
            .unwrap_or_default();
        v_flex()
            .id("commit-details-files")
            .w_full()
            .flex_1()
            .min_h(px(60.))
            .overflow_y_scroll()
            .children(files.into_iter().map(|changed| {
                let is_conflicted_file = changed.path == self.file;
                h_flex()
                    .w_full()
                    .px_2()
                    .py_0p5()
                    .gap_2()
                    .when(is_conflicted_file, |this| this.bg(colors.element_selected))
                    .child(
                        Label::new(changed.kind.label())
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(Label::new(changed.path.as_unix_str().to_string()).truncate())
            }))
            .into_any_element()
    }
}

impl Focusable for CommitDetailsDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl DialogWindowContent for CommitDetailsDialog {
    fn close_requested(
        &mut self,
        _request: CloseRequest,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> CloseDecision {
        CloseDecision::Close
    }
}

impl Render for CommitDetailsDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = self.visible();
        let selection = effective_selection(&self.commits, &visible, self.selected.as_ref());
        let rows: Vec<AnyElement> = visible
            .into_iter()
            .map(|index| self.render_commit_row(index, selection.as_ref(), cx))
            .collect();
        let colors = cx.theme().colors();
        v_flex()
            .id("commit-details-dialog")
            .key_context("CommitDetailsDialog")
            .track_focus(&self.focus_handle)
            .size_full()
            .p_3()
            .gap_2()
            .when(self.loading, |this| {
                this.child(Label::new(messages::COLLECTING_COMMIT_DETAILS).color(Color::Muted))
            })
            .when_some(self.error.clone(), |this, error| {
                this.child(Label::new(error).color(Color::Error))
            })
            .child(
                v_flex()
                    .id("commit-details-commits")
                    .w_full()
                    .flex_1()
                    .min_h(px(60.))
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(colors.border)
                    .children(rows),
            )
            .child(
                v_flex()
                    .w_full()
                    .flex_1()
                    .min_h(px(60.))
                    .border_1()
                    .border_color(colors.border)
                    .child(self.render_files(selection.as_ref(), cx)),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(div().when(self.is_range, |this| {
                        this.child(
                            Checkbox::new("commit-details-filter", self.filter_by_file.into())
                                .label(messages::FILTER_BY_CONFLICTED_FILE)
                                .on_click(cx.listener(|this, state: &ToggleState, _, cx| {
                                    this.filter_by_file = state.selected();
                                    cx.notify();
                                })),
                        )
                    }))
                    .child(
                        Button::new("commit-details-close", messages::CLOSE).on_click(cx.listener(
                            |_, _, window, _| {
                                close_dialog_window(window);
                            },
                        )),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use project::git_store::CommitFile;

    use super::*;

    fn commit(sha: &str) -> CommitSummary {
        CommitSummary {
            sha: SharedString::from(sha.to_string()),
            subject: SharedString::from("subject".to_string()),
            commit_timestamp: 0,
            author_name: SharedString::from("author".to_string()),
            has_parent: true,
        }
    }

    fn changed(path: &str) -> ChangedFile {
        ChangedFile {
            path: RepoPath::new(path).expect("valid repo path"),
            kind: ChangeKind::Modified,
        }
    }

    #[test]
    fn merge_tool_short_hash_is_a_strict_prefix_of_a_full_sha_and_keeps_short_input() {
        let full_sha = "0123456789abcdef0123456789abcdef01234567";
        let shortened = short_hash(full_sha);
        assert!(shortened.len() < full_sha.len());
        assert!(full_sha.starts_with(shortened));
        assert_eq!(short_hash("abc"), "abc");
        assert_eq!(short_hash(""), "");
    }

    #[test]
    fn merge_tool_commit_filter_keeps_commits_touching_the_file_or_not_yet_loaded() {
        let commits = vec![commit("aaaaaaa1"), commit("bbbbbbb2"), commit("ccccccc3")];
        let mut changes = HashMap::new();
        changes.insert(
            SharedString::from("aaaaaaa1".to_string()),
            vec![changed("x.rs")],
        );
        changes.insert(
            SharedString::from("bbbbbbb2".to_string()),
            vec![changed("y.rs")],
        );
        let file = RepoPath::new("x.rs").expect("valid repo path");
        assert_eq!(
            visible_commit_indices(&commits, &changes, &file, true),
            vec![0, 2]
        );
        assert_eq!(
            visible_commit_indices(&commits, &changes, &file, false),
            vec![0, 1, 2]
        );
    }

    fn commit_file(path: &str, old_text: Option<&str>, new_text: Option<&str>) -> CommitFile {
        CommitFile {
            path: RepoPath::new(path).expect("valid repo path"),
            old_text: old_text.map(str::to_string),
            new_text: new_text.map(str::to_string),
            is_binary: false,
        }
    }

    #[test]
    fn merge_tool_changed_files_classify_added_modified_and_deleted_paths() {
        let diff = CommitDiff {
            files: vec![
                commit_file("added.rs", None, Some("new")),
                commit_file("modified.rs", Some("old"), Some("new")),
                commit_file("deleted.rs", Some("old"), None),
            ],
            is_shallow_boundary: false,
        };
        let summary: Vec<(String, &str)> = changed_files(&diff)
            .into_iter()
            .map(|changed| (changed.path.as_unix_str().to_string(), changed.kind.label()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("added.rs".to_string(), "Added"),
                ("modified.rs".to_string(), "Modified"),
                ("deleted.rs".to_string(), "Deleted"),
            ]
        );
    }

    #[test]
    fn merge_tool_selection_falls_back_to_the_first_visible_commit() {
        let commits = vec![commit("aaaaaaa1"), commit("bbbbbbb2"), commit("ccccccc3")];
        let first = commits[0].sha.clone();
        let second = commits[1].sha.clone();
        let third = commits[2].sha.clone();
        assert_eq!(
            effective_selection(&commits, &[1, 2], Some(&first)),
            Some(second.clone())
        );
        assert_eq!(
            effective_selection(&commits, &[1, 2], Some(&third)),
            Some(third)
        );
        assert_eq!(effective_selection(&commits, &[1, 2], None), Some(second));
        assert_eq!(effective_selection(&commits, &[], Some(&first)), None);
    }
}
