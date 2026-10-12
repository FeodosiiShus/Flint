use anyhow::Result;
use db::kvp::KeyValueStore;
use git::Oid;
use git::repository::{CommitData, GitFailure, ResetMode};
use gpui::{
    AsyncWindowContext, DismissEvent, EventEmitter, FocusHandle, Focusable, FontWeight,
    PromptLevel, Role,
};
use menu::{Cancel, Confirm};
use ui::{ChoiceCard, DialogButton, prelude::*};
use util::ResultExt as _;
use workspace::ModalView;

use super::context_menu::MenuCommit;
use crate::branch_operations::integrate::is_protected_remote_reference;
use crate::branch_operations::{BranchContext, BranchNotice};
use crate::git_panel::GitPanel;
use crate::merge_tool::conflict_resolution::labels::short_hash;

const RESET_MODE_KEY: &str = "git_reset_mode";
const DIALOG_WIDTH: f32 = 34.;
const DIALOG_TITLE: &str = "Git Reset";
const DIALOG_DESCRIPTION: &str = "This will reset the current branch head to the selected commit,\nand update the working tree and the index according to the selected mode:";
const RESET_BUTTON: &str = "Reset";
const CANCEL_BUTTON: &str = "Cancel";
const OK_BUTTON: &str = "OK";
const HEAD: &str = "HEAD";
const SOURCE_TARGET_SEPARATOR: &str = " -> ";
const SOURCE_IN_REPOSITORY_SEPARATOR: &str = " in ";
const BY_AUTHOR_SEPARATOR: &str = " by ";
const MAX_TARGET_TEXT_LENGTH: usize = 40;
const SHORTENED_TEXT_SYMBOL: &str = "...";
const RESET_FAILURE_PREFIX: &str = "Failed to reset:";
const CANNOT_UNDO_COMMIT_TITLE: &str = "Cannot Undo Commit";

const RESET_PRESENTATION: ResetPresentation = ResetPresentation {
    success_message: "Reset successful",
    failure_title: "Reset failed",
};

const UNDO_COMMIT_PRESENTATION: ResetPresentation = ResetPresentation {
    success_message: "Undo commit successful",
    failure_title: "Undo commit failed",
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResetPresentation {
    success_message: &'static str,
    failure_title: &'static str,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum GitResetMode {
    Soft,
    #[default]
    Mixed,
    Hard,
    Keep,
}

impl GitResetMode {
    const ALL: [Self; 4] = [Self::Soft, Self::Mixed, Self::Hard, Self::Keep];

    fn name(self) -> &'static str {
        match self {
            Self::Soft => "Soft",
            Self::Mixed => "Mixed",
            Self::Hard => "Hard",
            Self::Keep => "Keep",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Soft => "Files won't change, differences will be staged for commit.",
            Self::Mixed => "Files won't change, differences won't be staged.",
            Self::Hard => {
                "Files will be reverted to the state of the selected commit.\nWarning: any local changes will be lost."
            }
            Self::Keep => {
                "Files will be reverted to the state of the selected commit,\nbut local changes will be kept intact."
            }
        }
    }

    fn element_id(self) -> &'static str {
        match self {
            Self::Soft => "git-reset-mode-soft",
            Self::Mixed => "git-reset-mode-mixed",
            Self::Hard => "git-reset-mode-hard",
            Self::Keep => "git-reset-mode-keep",
        }
    }

    fn stored_value(self) -> &'static str {
        match self {
            Self::Soft => "SOFT",
            Self::Mixed => "MIXED",
            Self::Hard => "HARD",
            Self::Keep => "KEEP",
        }
    }

    fn from_stored(value: Option<&str>) -> Self {
        let value = value.map(str::trim);
        Self::ALL
            .into_iter()
            .find(|mode| Some(mode.stored_value()) == value)
            .unwrap_or_default()
    }

    fn repository_mode(self) -> ResetMode {
        match self {
            Self::Soft => ResetMode::Soft,
            Self::Mixed => ResetMode::Mixed,
            Self::Hard => ResetMode::Hard,
            Self::Keep => ResetMode::Keep,
        }
    }
}

fn saved_reset_mode(cx: &App) -> GitResetMode {
    let stored = KeyValueStore::global(cx)
        .read_kvp(RESET_MODE_KEY)
        .log_err()
        .flatten();
    GitResetMode::from_stored(stored.as_deref())
}

fn save_reset_mode(mode: GitResetMode, cx: &App) {
    let store = KeyValueStore::global(cx);
    let value = mode.stored_value().to_string();
    db::write_and_log(cx, move || async move {
        store.write_kvp(RESET_MODE_KEY.to_string(), value).await
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SegmentStyle {
    Plain,
    Bold,
    Code,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DescriptionSegment {
    text: SharedString,
    style: SegmentStyle,
}

impl DescriptionSegment {
    fn new(text: impl Into<SharedString>, style: SegmentStyle) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TargetCommit {
    sha: Oid,
    subject: SharedString,
    author_name: SharedString,
    author_email: SharedString,
    message: SharedString,
}

impl TargetCommit {
    fn from_data(data: &CommitData) -> Self {
        Self {
            sha: data.sha,
            subject: data.subject.clone(),
            author_name: data.author_name.clone(),
            author_email: data.author_email.clone(),
            message: data.message.clone(),
        }
    }
}

fn shorten_text_with_ellipsis(text: &str, max_length: usize) -> String {
    if text.chars().count() <= max_length {
        return text.to_string();
    }
    let prefix_length = max_length.saturating_sub(SHORTENED_TEXT_SYMBOL.chars().count());
    let prefix: String = text.chars().take(prefix_length).collect();
    format!("{prefix}{SHORTENED_TEXT_SYMBOL}")
}

fn user_name(name: &str, email: &str) -> String {
    if !name.is_empty() {
        return name.to_string();
    }
    match email.find('@') {
        Some(at) if at > 0 => email[..at].to_string(),
        _ => email.to_string(),
    }
}

fn subject_of(message: &str) -> String {
    message
        .lines()
        .skip_while(|line| line.trim().is_empty())
        .take_while(|line| !line.trim().is_empty())
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ")
}

fn source_text(current_branch: Option<&str>, current_revision: Option<&str>) -> String {
    match (current_branch, current_revision) {
        (Some(branch), _) => branch.to_string(),
        (None, Some(revision)) => format!("{HEAD} ({})", short_hash(revision)),
        (None, None) => HEAD.to_string(),
    }
}

fn target_segments(target: &TargetCommit) -> Vec<DescriptionSegment> {
    let subject = shorten_text_with_ellipsis(&target.subject, MAX_TARGET_TEXT_LENGTH);
    let author = shorten_text_with_ellipsis(
        &user_name(&target.author_name, &target.author_email),
        MAX_TARGET_TEXT_LENGTH,
    );
    vec![
        DescriptionSegment::new(short_hash(&target.sha.to_string()), SegmentStyle::Bold),
        DescriptionSegment::new(format!(" \"{subject}\""), SegmentStyle::Plain),
        DescriptionSegment::new(BY_AUTHOR_SEPARATOR, SegmentStyle::Plain),
        DescriptionSegment::new(author, SegmentStyle::Code),
    ]
}

fn description_segments(
    source: &str,
    repository_name: Option<&str>,
    target: &TargetCommit,
) -> Vec<DescriptionSegment> {
    let mut segments = vec![DescriptionSegment::new(
        source.to_string(),
        SegmentStyle::Bold,
    )];
    if let Some(repository_name) = repository_name {
        segments.push(DescriptionSegment::new(
            SOURCE_IN_REPOSITORY_SEPARATOR,
            SegmentStyle::Plain,
        ));
        segments.push(DescriptionSegment::new(
            repository_name.to_string(),
            SegmentStyle::Plain,
        ));
    }
    segments.push(DescriptionSegment::new(
        SOURCE_TARGET_SEPARATOR,
        SegmentStyle::Plain,
    ));
    segments.extend(target_segments(target));
    segments
}

fn reset_description(
    context: &BranchContext,
    target: &TargetCommit,
    cx: &App,
) -> Vec<DescriptionSegment> {
    let repository = context.repository.read(cx);
    let current_branch = repository
        .branch
        .as_ref()
        .map(|branch| branch.name().to_string());
    let current_revision = repository
        .head_commit
        .as_ref()
        .map(|commit| commit.sha.to_string());
    let source = source_text(current_branch.as_deref(), current_revision.as_deref());
    let is_multi_repository = repository
        .git_store()
        .is_some_and(|git_store| git_store.read(cx).repositories().len() > 1);
    let repository_name = is_multi_repository.then(|| repository.display_name());
    description_segments(&source, repository_name.as_deref(), target)
}

fn reset_error_output(error: &anyhow::Error) -> String {
    if let Some(failure) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<GitFailure>())
    {
        return failure.message.trim().to_string();
    }
    let text = format!("{error:#}");
    let trimmed = text.trim();
    trimmed
        .strip_prefix(RESET_FAILURE_PREFIX)
        .unwrap_or(trimmed)
        .trim()
        .to_string()
}

fn root_or_merge_message(sha: Oid, parent_count: usize) -> String {
    format!("Commit {sha} has {parent_count} parents")
}

fn pushed_to_protected_branch_message(branch: &str) -> String {
    format!("The commit is already pushed to protected branch '{branch}'")
}

fn find_protected_remote_branch(
    remote_branches: &[SharedString],
    containing_branches: &[SharedString],
) -> Option<SharedString> {
    remote_branches
        .iter()
        .filter(|branch| is_protected_remote_reference(branch))
        .find(|branch| containing_branches.contains(branch))
        .cloned()
}

fn remote_branch_names(context: &BranchContext, cx: &App) -> Vec<SharedString> {
    context
        .repository
        .read(cx)
        .branch_list
        .iter()
        .filter(|branch| branch.is_remote())
        .map(|branch| SharedString::from(branch.name().to_string()))
        .collect()
}

fn notify(context: &BranchContext, notice: BranchNotice, cx: &mut AsyncWindowContext) {
    cx.update(|_, app| context.notify(notice, app)).log_err();
}

fn failure_notice(presentation: ResetPresentation, error: &anyhow::Error) -> BranchNotice {
    BranchNotice::error(reset_error_output(error)).title(presentation.failure_title)
}

async fn load_target_commit(
    context: &BranchContext,
    commit: &MenuCommit,
    cx: &mut AsyncWindowContext,
) -> Result<TargetCommit> {
    if let Some(data) = commit.data.as_ref() {
        return Ok(TargetCommit::from_data(data));
    }
    let sha = commit.sha;
    let receiver = context
        .repository
        .update(cx, |repository, _| repository.show(sha.to_string()));
    let details = receiver.await??;
    Ok(TargetCommit {
        sha,
        subject: subject_of(&details.message).into(),
        author_name: details.author_name,
        author_email: details.author_email,
        message: details.message,
    })
}

async fn reset(
    context: &BranchContext,
    target: String,
    mode: GitResetMode,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let receiver = context.repository.update(cx, |repository, cx| {
        repository.reset(target, mode.repository_mode(), cx)
    });
    receiver.await?
}

async fn execute_reset(
    context: &BranchContext,
    target: String,
    mode: GitResetMode,
    presentation: ResetPresentation,
    cx: &mut AsyncWindowContext,
) {
    match reset(context, target, mode, cx).await {
        Ok(()) => notify(
            context,
            BranchNotice::info(presentation.success_message),
            cx,
        ),
        Err(error) => notify(context, failure_notice(presentation, &error), cx),
    }
}

pub(crate) fn reset_current_branch_to(
    context: BranchContext,
    commit: MenuCommit,
    window: &mut Window,
    cx: &mut App,
) {
    window
        .spawn(cx, async move |cx| {
            let target = match load_target_commit(&context, &commit, cx).await {
                Ok(target) => target,
                Err(error) => {
                    notify(&context, failure_notice(RESET_PRESENTATION, &error), cx);
                    return;
                }
            };
            cx.update(|window, app| {
                let description = reset_description(&context, &target, app);
                let mode = saved_reset_mode(app);
                let dialog_context = context.clone();
                let target_sha = target.sha;
                context.open_modal(window, app, move |_, cx| {
                    GitResetDialog::new(dialog_context, target_sha, description, mode, cx)
                });
            })
            .log_err();
        })
        .detach();
}

pub(crate) fn undo_commit(
    context: BranchContext,
    commit: MenuCommit,
    window: &mut Window,
    cx: &mut App,
) {
    window
        .spawn(cx, async move |cx| {
            if let Some(description) = undo_commit_check(&context, &commit, cx).await {
                let answer = cx.prompt(
                    PromptLevel::Critical,
                    CANNOT_UNDO_COMMIT_TITLE,
                    Some(description.as_str()),
                    &[OK_BUTTON],
                );
                answer.await.ok();
                return;
            }
            let Some(parent) = commit.parents.first().copied() else {
                return;
            };
            let message = load_target_commit(&context, &commit, cx)
                .await
                .log_err()
                .map(|target| target.message);
            execute_reset(
                &context,
                parent.to_string(),
                GitResetMode::Soft,
                UNDO_COMMIT_PRESENTATION,
                cx,
            )
            .await;
            if let Some(message) = message {
                set_commit_message(&context, message, cx);
            }
        })
        .detach();
}

async fn undo_commit_check(
    context: &BranchContext,
    commit: &MenuCommit,
    cx: &mut AsyncWindowContext,
) -> Option<String> {
    if commit.parents.len() != 1 {
        return Some(root_or_merge_message(commit.sha, commit.parents.len()));
    }
    let sha = commit.sha;
    let containing_branches = match cx.update(|_, app| {
        context
            .repository
            .read(app)
            .branches_containing_commit(sha, app)
    }) {
        Ok(task) => task.await,
        Err(error) => Err(error),
    };
    let containing_branches = match containing_branches {
        Ok(branches) => branches,
        Err(error) => return Some(format!("{error:#}")),
    };
    let remote_branches = cx
        .update(|_, app| remote_branch_names(context, app))
        .log_err()?;
    find_protected_remote_branch(&remote_branches, &containing_branches)
        .map(|branch| pushed_to_protected_branch_message(&branch))
}

fn set_commit_message(context: &BranchContext, message: SharedString, cx: &mut AsyncWindowContext) {
    let workspace = context.workspace.clone();
    cx.update(|_, app| {
        let Some(workspace) = workspace.upgrade() else {
            return;
        };
        let Some(git_panel) = workspace.read(app).panel::<GitPanel>(app) else {
            return;
        };
        let buffer = git_panel.read(app).commit_message_buffer(app);
        buffer.update(app, |buffer, cx| {
            buffer.set_text(message.to_string(), cx);
        });
    })
    .log_err();
}

struct GitResetDialog {
    context: BranchContext,
    target: Oid,
    description: Vec<DescriptionSegment>,
    mode: GitResetMode,
    focus_handle: FocusHandle,
}

impl GitResetDialog {
    fn new(
        context: BranchContext,
        target: Oid,
        description: Vec<DescriptionSegment>,
        mode: GitResetMode,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            context,
            target,
            description,
            mode,
            focus_handle: cx.focus_handle(),
        }
    }

    fn select_mode(&mut self, mode: GitResetMode, cx: &mut Context<Self>) {
        self.mode = mode;
        cx.notify();
    }

    fn cancel(&mut self, _: &Cancel, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        let context = self.context.clone();
        let target = self.target.to_string();
        let mode = self.mode;
        save_reset_mode(mode, cx);
        cx.emit(DismissEvent);
        window
            .spawn(cx, async move |cx| {
                execute_reset(&context, target, mode, RESET_PRESENTATION, cx).await;
            })
            .detach();
    }

    fn render_segment(segment: &DescriptionSegment, cx: &App) -> Label {
        let label = Label::new(segment.text.clone());
        match segment.style {
            SegmentStyle::Plain => label,
            SegmentStyle::Bold => label.weight(FontWeight::BOLD),
            SegmentStyle::Code => label.buffer_font(cx),
        }
    }
}

impl EventEmitter<DismissEvent> for GitResetDialog {}
impl ModalView for GitResetDialog {}

impl Focusable for GitResetDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GitResetDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_mode = self.mode;
        let description = h_flex().flex_wrap().children(
            self.description
                .iter()
                .map(|segment| Self::render_segment(segment, cx)),
        );
        let mode_cards: Vec<ChoiceCard> = GitResetMode::ALL
            .into_iter()
            .map(|mode| {
                ChoiceCard::radio(mode.element_id(), mode.name(), mode == selected_mode)
                    .description(mode.description())
                    .on_click(cx.listener(move |this, _, _, cx| this.select_mode(mode, cx)))
            })
            .collect();

        v_flex()
            .key_context("GitResetDialog")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .elevation_3(cx)
            .w(rems(DIALOG_WIDTH))
            .child(
                h_flex()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .w_full()
                    .child(Headline::new(DIALOG_TITLE).size(HeadlineSize::XSmall)),
            )
            .child(
                v_flex()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .child(description)
                    .child(Label::new(DIALOG_DESCRIPTION)),
            )
            .child(
                v_flex()
                    .id("git-reset-mode")
                    .role(Role::RadioGroup)
                    .px_3()
                    .py_1()
                    .gap_1()
                    .children(mode_cards),
            )
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .pt_1()
                    .pb_2()
                    .gap_2()
                    .justify_end()
                    .child(
                        DialogButton::new("git-reset-cancel", CANCEL_BUTTON)
                            .default_button(false)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.cancel(&Cancel, window, cx)),
                            ),
                    )
                    .child(
                        DialogButton::new("git-reset-confirm", RESET_BUTTON)
                            .default_button(true)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm(&Confirm, window, cx)
                            })),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn oid() -> Oid {
        Oid::try_from(SHA).expect("valid object id")
    }

    fn target(subject: &str, author_name: &str, author_email: &str) -> TargetCommit {
        TargetCommit {
            sha: oid(),
            subject: subject.to_string().into(),
            author_name: author_name.to_string().into(),
            author_email: author_email.to_string().into(),
            message: subject.to_string().into(),
        }
    }

    fn texts(segments: &[DescriptionSegment]) -> String {
        segments
            .iter()
            .map(|segment| segment.text.to_string())
            .collect()
    }

    #[test]
    fn modes_are_listed_in_the_git_reset_mode_enum_order_and_default_to_mixed() {
        assert_eq!(
            GitResetMode::ALL,
            [
                GitResetMode::Soft,
                GitResetMode::Mixed,
                GitResetMode::Hard,
                GitResetMode::Keep
            ]
        );
        assert_eq!(GitResetMode::default(), GitResetMode::Mixed);
        let names: Vec<&str> = GitResetMode::ALL.iter().map(|mode| mode.name()).collect();
        assert_eq!(names, vec!["Soft", "Mixed", "Hard", "Keep"]);
    }

    #[test]
    fn remembered_mode_round_trips_and_falls_back_to_mixed() {
        for mode in GitResetMode::ALL {
            assert_eq!(GitResetMode::from_stored(Some(mode.stored_value())), mode);
        }
        assert_eq!(GitResetMode::from_stored(None), GitResetMode::Mixed);
        assert_eq!(
            GitResetMode::from_stored(Some("bogus")),
            GitResetMode::Mixed
        );
    }

    #[test]
    fn mode_descriptions_keep_the_bundle_line_breaks_without_html() {
        assert_eq!(
            GitResetMode::Soft.description(),
            "Files won't change, differences will be staged for commit."
        );
        assert_eq!(
            GitResetMode::Hard.description(),
            "Files will be reverted to the state of the selected commit.\nWarning: any local changes will be lost."
        );
        assert!(
            GitResetMode::ALL
                .iter()
                .all(|mode| !mode.description().contains('<'))
        );
    }

    #[test]
    fn long_texts_are_cut_to_forty_characters_with_three_dots() {
        let exact = "a".repeat(MAX_TARGET_TEXT_LENGTH);
        assert_eq!(
            shorten_text_with_ellipsis(&exact, MAX_TARGET_TEXT_LENGTH),
            exact
        );
        let longer = "b".repeat(MAX_TARGET_TEXT_LENGTH + 1);
        let shortened = shorten_text_with_ellipsis(&longer, MAX_TARGET_TEXT_LENGTH);
        assert_eq!(
            shortened,
            format!("{}...", "b".repeat(MAX_TARGET_TEXT_LENGTH - 3))
        );
        assert_eq!(shortened.chars().count(), MAX_TARGET_TEXT_LENGTH);
    }

    #[test]
    fn author_falls_back_to_the_email_name_part_then_the_email() {
        assert_eq!(user_name("Jane Doe", "jane@example.com"), "Jane Doe");
        assert_eq!(user_name("", "jane@example.com"), "jane");
        assert_eq!(user_name("", "@example.com"), "@example.com");
        assert_eq!(user_name("", "jane"), "jane");
    }

    #[test]
    fn source_is_the_branch_then_detached_head_with_its_short_hash_then_head() {
        assert_eq!(source_text(Some("main"), Some(SHA)), "main");
        assert_eq!(source_text(None, Some(SHA)), "HEAD (01234567)");
        assert_eq!(source_text(None, None), "HEAD");
    }

    #[test]
    fn single_repository_description_points_the_source_at_the_target_commit() {
        let segments = description_segments("main", None, &target("Fix bug", "Jane", "j@x.io"));
        assert_eq!(texts(&segments), "main -> 01234567 \"Fix bug\" by Jane");
        assert_eq!(segments[0].style, SegmentStyle::Bold);
        assert_eq!(segments[2].style, SegmentStyle::Bold);
        assert_eq!(
            segments.last().map(|segment| segment.style),
            Some(SegmentStyle::Code)
        );
    }

    #[test]
    fn multi_repository_description_names_the_repository_of_the_source() {
        let segments =
            description_segments("main", Some("flint"), &target("Fix bug", "", "jane@x.io"));
        assert_eq!(
            texts(&segments),
            "main in flint -> 01234567 \"Fix bug\" by jane"
        );
    }

    #[test]
    fn subject_of_a_loaded_message_is_its_first_paragraph() {
        assert_eq!(subject_of("Fix bug\n\nBody text"), "Fix bug");
        assert_eq!(
            subject_of("\nFirst line\nsecond line\n\nBody"),
            "First line second line"
        );
        assert_eq!(subject_of(""), "");
    }

    #[test]
    fn reset_error_output_shows_the_git_output_without_the_backend_prefix() {
        let error = anyhow::anyhow!(
            "Failed to reset:\nerror: Entry 'a.rs' not uptodate. Cannot merge.\nfatal: Could not reset index file to revision 'abc'.\n"
        );
        assert_eq!(
            reset_error_output(&error),
            "error: Entry 'a.rs' not uptodate. Cannot merge.\nfatal: Could not reset index file to revision 'abc'."
        );
        assert_eq!(
            reset_error_output(&anyhow::anyhow!("connection lost")),
            "connection lost"
        );
    }

    #[test]
    fn notices_use_the_reset_and_undo_commit_bundle_texts() {
        let error = anyhow::anyhow!("Failed to reset:\nfatal: bad");
        let reset = failure_notice(RESET_PRESENTATION, &error);
        assert_eq!(reset.title.as_deref(), Some("Reset failed"));
        assert_eq!(reset.message.as_ref(), "fatal: bad");
        let undo = failure_notice(UNDO_COMMIT_PRESENTATION, &error);
        assert_eq!(undo.title.as_deref(), Some("Undo commit failed"));
        assert_eq!(RESET_PRESENTATION.success_message, "Reset successful");
        assert_eq!(
            UNDO_COMMIT_PRESENTATION.success_message,
            "Undo commit successful"
        );
    }

    #[test]
    fn root_and_merge_commits_cannot_be_undone() {
        assert_eq!(
            root_or_merge_message(oid(), 0),
            format!("Commit {SHA} has 0 parents")
        );
        assert_eq!(
            root_or_merge_message(oid(), 2),
            format!("Commit {SHA} has 2 parents")
        );
    }

    #[test]
    fn the_first_protected_remote_branch_containing_the_commit_blocks_undo() {
        let remote_branches: [SharedString; 3] = [
            "origin/feature".into(),
            "upstream/main".into(),
            "origin/main".into(),
        ];
        let containing: [SharedString; 3] =
            ["main".into(), "origin/main".into(), "upstream/main".into()];
        let found = find_protected_remote_branch(&remote_branches, &containing);
        assert_eq!(found.as_deref(), Some("upstream/main"));
        assert_eq!(
            pushed_to_protected_branch_message("upstream/main"),
            "The commit is already pushed to protected branch 'upstream/main'"
        );
        let unpublished: [SharedString; 2] = ["main".into(), "origin/feature".into()];
        assert_eq!(
            find_protected_remote_branch(&remote_branches, &unpublished),
            None
        );
    }
}
