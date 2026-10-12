mod presentation;

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use collections::{HashMap, HashSet};
use git::Oid;
use git::repository::CommitData;
use gpui::{
    AnyElement, App, AsyncApp, Context, Entity, FocusHandle, Focusable, FontWeight, HighlightStyle,
    Hsla, InteractiveText, Pixels, ScrollHandle, SharedString, StyledText, Subscription, Task,
    TextStyleRefinement, WeakEntity, Window, canvas, px, rgb,
};
use markdown::{Markdown, MarkdownElement, MarkdownStyle};
use project::git_store::{CommitDataState, Repository, RepositoryEvent};
use release_channel::ReleaseChannel;
use settings::Settings as _;
use theme_settings::ThemeSettings;
use ui::{CommonAnimationExt as _, TextSize, WithScrollbar, prelude::*};
use workspace::Workspace;

use self::presentation::{
    BRANCHES_LOADING_TEXT, BRANCHES_TAIL_PREFIX, BranchesLine, HIDE_LINK, MAX_COMMITS_TO_LOAD,
    MetadataInput, MetadataPart, NOT_IN_ANY_BRANCH_TEXT, RefSource, RefSourceBranch, ReferenceLine,
    SHOW_ALL_LINK, branches_line, message_presentation, metadata, reference_line, refs_for_commit,
    showing_selected_text, split_references,
};
use super::log_dates::{format_date, format_time};
use super::ref_labels::{label_icon, ref_label_color};

const KEY_CONTEXT: &str = "GitLogDetails";
const COMMIT_DETAILS_TEXT: &str = "Commit details";
const NO_COMMITS_SELECTED_TEXT: &str = "No commits selected";
const ERROR_LOADING_DETAILS_TEXT: &str = "Error loading commit details";
const LOADING_TEXT: &str = "Loading…";
const LOADING_DELAY: Duration = Duration::from_millis(300);
const SIDE_BORDER: Pixels = px(14.);
const INTERNAL_BORDER: Pixels = px(10.);
const EXTERNAL_BORDER: Pixels = px(14.);
const REFERENCES_GAP: Pixels = px(4.);
const REFERENCE_ICON_TEXT_GAP: Pixels = px(2.);
const COMMITTER_GRAY: u32 = 0x808080;

struct CommitEntry {
    data: Arc<CommitData>,
    subject: Option<Entity<Markdown>>,
    body: Option<Entity<Markdown>>,
}

pub(crate) struct CommitDetailsPanel {
    _workspace: WeakEntity<Workspace>,
    repository: Option<Entity<Repository>>,
    commits: Vec<Oid>,
    focus_handle: FocusHandle,
    scroll_handle: ScrollHandle,
    status_text: SharedString,
    entries: Vec<CommitEntry>,
    requested_count: usize,
    loading: bool,
    loading_visible: bool,
    containing_branches: HashMap<Oid, Vec<SharedString>>,
    expanded_branches: HashSet<Oid>,
    tags: Vec<(SharedString, SharedString)>,
    content_width: Rc<Cell<Pixels>>,
    load_task: Task<()>,
    loading_indicator_task: Task<()>,
    branches_task: Task<()>,
    tags_task: Task<()>,
    repository_subscription: Option<Subscription>,
}

impl CommitDetailsPanel {
    pub(crate) fn new(
        workspace: WeakEntity<Workspace>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            _workspace: workspace,
            repository: None,
            commits: Vec::new(),
            focus_handle: cx.focus_handle(),
            scroll_handle: ScrollHandle::new(),
            status_text: COMMIT_DETAILS_TEXT.into(),
            entries: Vec::new(),
            requested_count: 0,
            loading: false,
            loading_visible: false,
            containing_branches: HashMap::default(),
            expanded_branches: HashSet::default(),
            tags: Vec::new(),
            content_width: Rc::new(Cell::new(px(0.))),
            load_task: Task::ready(()),
            loading_indicator_task: Task::ready(()),
            branches_task: Task::ready(()),
            tags_task: Task::ready(()),
            repository_subscription: None,
        }
    }

    pub(crate) fn set_commits(
        &mut self,
        repository: Option<Entity<Repository>>,
        commits: Vec<Oid>,
        cx: &mut Context<Self>,
    ) {
        let same_repository = match (&self.repository, &repository) {
            (Some(current), Some(new)) => current == new,
            (None, None) => true,
            _ => false,
        };
        if !same_repository {
            self.repository = repository;
            self.containing_branches.clear();
            self.tags.clear();
            self.repository_subscription = self.repository.as_ref().map(|repository| {
                cx.subscribe(repository, |this, _, event: &RepositoryEvent, cx| {
                    if matches!(
                        event,
                        RepositoryEvent::HeadChanged | RepositoryEvent::BranchListChanged
                    ) {
                        this.containing_branches.clear();
                        this.load_tags(cx);
                        this.load_containing_branches(cx);
                        cx.notify();
                    }
                })
            });
            self.load_tags(cx);
        } else if self.commits == commits && !self.entries.is_empty() {
            return;
        }
        self.commits = commits;
        self.expanded_branches.clear();
        self.load(cx);
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let repository = self
            .repository
            .as_ref()
            .filter(|_| !self.commits.is_empty())
            .map(Entity::downgrade);
        let Some(repository) = repository else {
            self.load_task = Task::ready(());
            self.loading_indicator_task = Task::ready(());
            self.branches_task = Task::ready(());
            self.loading = false;
            self.loading_visible = false;
            self.status_text = NO_COMMITS_SELECTED_TEXT.into();
            self.entries.clear();
            self.requested_count = 0;
            cx.notify();
            return;
        };
        self.loading = true;
        self.loading_visible = false;
        self.loading_indicator_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LOADING_DELAY).await;
            this.update(cx, |this, cx| {
                if this.loading {
                    this.loading_visible = true;
                    cx.notify();
                }
            })
            .ok();
        });
        let requested_count = self.commits.len();
        let commits: Vec<Oid> = self
            .commits
            .iter()
            .take(MAX_COMMITS_TO_LOAD)
            .copied()
            .collect();
        self.load_task = cx.spawn(async move |this, cx| {
            let mut loaded = Vec::with_capacity(commits.len());
            let mut failure = None;
            for sha in commits {
                match commit_data(&repository, sha, cx).await {
                    Ok(data) => loaded.push(data),
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                }
            }
            this.update(cx, |this, cx| {
                this.loading = false;
                this.loading_visible = false;
                match failure {
                    None => {
                        let entries: Vec<CommitEntry> = loaded
                            .into_iter()
                            .map(|data| Self::create_entry(data, cx))
                            .collect();
                        this.entries = entries;
                        this.requested_count = requested_count;
                        this.load_containing_branches(cx);
                    }
                    Some(error) => {
                        log::error!("error loading details of the selected commits: {error:#}");
                        this.status_text = ERROR_LOADING_DETAILS_TEXT.into();
                        this.entries.clear();
                        this.requested_count = 0;
                    }
                }
                cx.notify();
            })
            .ok();
        });
    }

    fn create_entry(data: Arc<CommitData>, cx: &mut Context<Self>) -> CommitEntry {
        let product_name = ReleaseChannel::try_global(cx)
            .map(|channel| channel.display_name())
            .unwrap_or_default();
        let message = message_presentation(&data.message, product_name);
        let subject = message
            .subject
            .map(|subject| cx.new(|cx| Markdown::new_text(subject.into(), cx)));
        let body = (!message.body.is_empty())
            .then(|| cx.new(|cx| Markdown::new_text(message.body.into(), cx)));
        CommitEntry {
            data,
            subject,
            body,
        }
    }

    fn load_tags(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.clone() else {
            self.tags.clear();
            self.tags_task = Task::ready(());
            return;
        };
        let receiver = repository.update(cx, |repository, _| repository.tags());
        self.tags_task = cx.spawn(async move |this, cx| match receiver.await {
            Ok(Ok(tags)) => {
                this.update(cx, |this, cx| {
                    this.tags = tags
                        .into_iter()
                        .map(|tag| (tag.name, tag.commit_sha))
                        .collect();
                    cx.notify();
                })
                .ok();
            }
            Ok(Err(error)) => log::error!("error loading tags for commit details: {error:#}"),
            Err(_) => log::debug!("tag loading for commit details was canceled"),
        });
    }

    fn load_containing_branches(&mut self, cx: &mut Context<Self>) {
        let Some(repository) = self.repository.as_ref().map(Entity::downgrade) else {
            return;
        };
        let pending: Vec<Oid> = self
            .entries
            .iter()
            .map(|entry| entry.data.sha)
            .filter(|sha| !self.containing_branches.contains_key(sha))
            .collect();
        self.branches_task = cx.spawn(async move |this, cx| {
            for sha in pending {
                let Ok(task) = repository.update(cx, |repository, cx| {
                    repository.branches_containing_commit(sha, cx)
                }) else {
                    return;
                };
                match task.await {
                    Ok(branches) => {
                        this.update(cx, |this, cx| {
                            this.containing_branches.insert(sha, branches);
                            cx.notify();
                        })
                        .ok();
                    }
                    Err(error) => {
                        log::error!("error loading branches containing {sha}: {error:#}")
                    }
                }
            }
        });
    }

    fn toggle_branches(&mut self, sha: Oid, cx: &mut Context<Self>) {
        if !self.expanded_branches.remove(&sha) {
            self.expanded_branches.insert(sha);
        }
        cx.notify();
    }

    fn ref_source(&self, cx: &App) -> RefSource {
        let Some(repository) = self.repository.as_ref() else {
            return RefSource::default();
        };
        let snapshot = repository.read(cx);
        RefSource {
            head_sha: snapshot
                .head_commit
                .as_ref()
                .map(|commit| commit.sha.clone()),
            current_branch: snapshot
                .branch
                .as_ref()
                .map(|branch| SharedString::from(branch.name())),
            branches: snapshot
                .branch_list
                .iter()
                .filter_map(|branch| {
                    let sha = branch.most_recent_commit.as_ref()?.sha.clone();
                    let name = branch.name();
                    if branch.is_remote() && name.ends_with("/HEAD") {
                        return None;
                    }
                    Some(RefSourceBranch {
                        name: name.into(),
                        is_remote: branch.is_remote(),
                        sha,
                    })
                })
                .collect(),
            tags: self.tags.clone(),
        }
    }

    fn render_entry(
        &self,
        index: usize,
        entry: &CommitEntry,
        source: &RefSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors();
        let background = colors.panel_background;
        let link_color = colors.text_accent;
        let font_size = TextSize::Default.rems(cx).to_pixels(window.rem_size());
        let text_style = window.text_style();
        let text_system = window.text_system();
        let measure = |text: &str| {
            f32::from(
                text_system
                    .shape_line(
                        SharedString::from(text),
                        font_size,
                        &[text_style.to_run(text.len())],
                        None,
                    )
                    .width,
            )
        };
        let probe = text_system.shape_line(
            SharedString::new_static("Gl"),
            font_size,
            &[text_style.to_run(2)],
            None,
        );
        let icon_height = probe.ascent + probe.descent;

        let data = &entry.data;
        let sha = data.sha;
        let hash = sha.to_string();
        let metadata = metadata(
            &MetadataInput {
                hash: &hash,
                author_name: &data.author_name,
                author_email: &data.author_email,
                author_time: data.author_timestamp,
                committer_name: &data.committer_name,
                committer_email: &data.committer_email,
                commit_time: data.committer_timestamp,
            },
            |timestamp| {
                (
                    format_date(timestamp).to_string(),
                    format_time(timestamp).to_string(),
                )
            },
        );
        let author_line = metadata_text(
            ("git-log-details-author", index),
            &metadata.author,
            None,
            link_color,
        );
        let committer_line = metadata.committer.as_ref().map(|parts| {
            metadata_text(
                ("git-log-details-committer", index),
                parts,
                Some(Hsla::from(rgb(COMMITTER_GRAY))),
                link_color,
            )
        });

        let (branches, tags) = split_references(refs_for_commit(source, &hash));
        let branches_line_refs = reference_line(&branches);
        let tags_line_refs = reference_line(&tags);
        let tags_visible = !tags_line_refs.entries.is_empty();
        let branch_references = render_references(
            branches_line_refs,
            !tags_visible,
            icon_height,
            background,
            cx,
        );
        let tag_references = render_references(tags_line_refs, true, icon_height, background, cx);

        let any_branches_loaded = self
            .entries
            .iter()
            .any(|entry| self.containing_branches.contains_key(&entry.data.sha));
        let containing = self.containing_branches.get(&sha);
        let containing_line = (containing.is_some() || any_branches_loaded).then(|| {
            let available = f32::from(self.content_width.get() - SIDE_BORDER - SIDE_BORDER).max(0.);
            let line = branches_line(
                containing.map(Vec::as_slice),
                self.expanded_branches.contains(&sha),
                available,
                measure,
            );
            self.render_branches_line(index, sha, line, cx)
        });

        let message_style = message_style(false, window, cx);
        let subject_style = message_style_with_weight(message_style.clone(), FontWeight::BOLD);
        v_flex()
            .w_full()
            .pr(SIDE_BORDER)
            .child(
                v_flex()
                    .w_full()
                    .pt(EXTERNAL_BORDER)
                    .pl(SIDE_BORDER)
                    .pb(INTERNAL_BORDER)
                    .children(
                        entry
                            .subject
                            .clone()
                            .map(|subject| MarkdownElement::new(subject, subject_style)),
                    )
                    .children(
                        entry
                            .body
                            .clone()
                            .map(|body| MarkdownElement::new(body, message_style)),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .pt(INTERNAL_BORDER)
                    .pl(SIDE_BORDER)
                    .pb(INTERNAL_BORDER)
                    .text_size(TextSize::Default.rems(cx))
                    .child(author_line)
                    .children(committer_line),
            )
            .children(branch_references)
            .children(tag_references)
            .children(containing_line)
            .into_any_element()
    }

    fn render_branches_line(
        &self,
        index: usize,
        sha: Oid,
        line: BranchesLine,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let link = |text: &'static str, cx: &mut Context<Self>| {
            div()
                .id(("git-log-details-branches-link", index))
                .flex_none()
                .cursor_pointer()
                .child(Label::new(text).color(Color::Accent).single_line())
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_branches(sha, cx)))
        };
        let content = match line {
            BranchesLine::Loading => Label::new(BRANCHES_LOADING_TEXT).into_any_element(),
            BranchesLine::Empty => Label::new(NOT_IN_ANY_BRANCH_TEXT).into_any_element(),
            BranchesLine::Collapsed {
                head,
                shown,
                show_all,
            } => h_flex()
                .w_full()
                .overflow_hidden()
                .child(Label::new(format!("{head}{shown}")).single_line())
                .when(show_all, |this| {
                    this.child(Label::new(BRANCHES_TAIL_PREFIX).single_line())
                        .child(link(SHOW_ALL_LINK, cx))
                })
                .into_any_element(),
            BranchesLine::Expanded { head, branches } => v_flex()
                .w_full()
                .child(
                    h_flex()
                        .child(Label::new(head).single_line())
                        .child(link(HIDE_LINK, cx)),
                )
                .children(branches.into_iter().map(Label::new))
                .into_any_element(),
        };
        v_flex()
            .w_full()
            .pl(SIDE_BORDER)
            .pb(EXTERNAL_BORDER)
            .child(content)
            .into_any_element()
    }

    fn render_status(&self) -> Option<AnyElement> {
        (self.entries.is_empty() && !self.loading_visible).then(|| {
            v_flex()
                .absolute()
                .size_full()
                .items_center()
                .child(div().flex_1())
                .child(Label::new(self.status_text.clone()).color(Color::Muted))
                .child(div().flex_1())
                .child(div().flex_1())
                .into_any_element()
        })
    }

    fn render_loading(&self) -> Option<AnyElement> {
        self.loading_visible.then(|| {
            h_flex()
                .absolute()
                .size_full()
                .justify_center()
                .items_center()
                .gap_1()
                .child(
                    Icon::new(IconName::ArrowCircle)
                        .size(IconSize::Medium)
                        .color(Color::Muted)
                        .with_rotate_animation(2),
                )
                .child(Label::new(LOADING_TEXT).color(Color::Muted))
                .into_any_element()
        })
    }
}

fn message_style(bold: bool, window: &Window, cx: &App) -> MarkdownStyle {
    let settings = ThemeSettings::get_global(cx);
    let colors = cx.theme().colors();
    let mut base_text_style = window.text_style();
    base_text_style.font_family = settings.buffer_font.family.clone();
    base_text_style.font_features = settings.buffer_font.features.clone();
    base_text_style.font_fallbacks = settings.buffer_font.fallbacks.clone();
    base_text_style.font_size = settings.buffer_font_size(cx).into();
    base_text_style.color = colors.text;
    if bold {
        base_text_style.font_weight = FontWeight::BOLD;
    }
    MarkdownStyle {
        base_text_style,
        link: TextStyleRefinement {
            color: Some(colors.text_accent),
            ..Default::default()
        },
        selection_background_color: colors.element_selection_background,
        ..Default::default()
    }
}

fn message_style_with_weight(mut style: MarkdownStyle, weight: FontWeight) -> MarkdownStyle {
    style.base_text_style.font_weight = weight;
    style
}

fn metadata_text(
    id: (&'static str, usize),
    parts: &[MetadataPart],
    base_color: Option<Hsla>,
    link_color: Hsla,
) -> AnyElement {
    let mut text = String::new();
    let mut highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    let mut email_ranges = Vec::new();
    let mut emails = Vec::new();
    for part in parts {
        let start = text.len();
        match part {
            MetadataPart::Text(value) => {
                text.push_str(value);
                if let Some(color) = base_color {
                    highlights.push((
                        start..text.len(),
                        HighlightStyle {
                            color: Some(color),
                            ..Default::default()
                        },
                    ));
                }
            }
            MetadataPart::Email(email) => {
                text.push('<');
                text.push_str(email);
                text.push('>');
                highlights.push((
                    start..text.len(),
                    HighlightStyle {
                        color: Some(link_color),
                        ..Default::default()
                    },
                ));
                email_ranges.push(start..text.len());
                emails.push(email.clone());
            }
        }
    }
    InteractiveText::new(id, StyledText::new(text).with_highlights(highlights))
        .on_click(email_ranges, move |index, _, cx| {
            if let Some(email) = emails.get(index) {
                cx.open_url(&format!("mailto:{email}"));
            }
        })
        .into_any_element()
}

fn render_references(
    line: ReferenceLine,
    bottom_padding: bool,
    icon_height: Pixels,
    background: Hsla,
    cx: &App,
) -> Option<AnyElement> {
    if line.entries.is_empty() {
        return None;
    }
    let hidden = line.hidden;
    Some(
        h_flex()
            .w_full()
            .flex_wrap()
            .pl(SIDE_BORDER - REFERENCES_GAP)
            .when(bottom_padding, |this| this.pb(INTERNAL_BORDER))
            .children(line.entries.into_iter().map(|entry| {
                let icon = entry.icon.map(|(kind, doubled)| {
                    let color = ref_label_color(kind, cx);
                    let colors = if doubled {
                        vec![color, color]
                    } else {
                        vec![color]
                    };
                    label_icon(colors, icon_height, background)
                });
                h_flex()
                    .ml(REFERENCES_GAP)
                    .gap(REFERENCE_ICON_TEXT_GAP)
                    .items_center()
                    .children(icon)
                    .child(Label::new(entry.text).single_line())
            }))
            .when(hidden > 0, |this| {
                this.child(
                    div()
                        .ml(REFERENCES_GAP)
                        .child(Label::new(format!("... {hidden} more")).single_line()),
                )
            })
            .into_any_element(),
    )
}

async fn commit_data(
    repository: &WeakEntity<Repository>,
    sha: Oid,
    cx: &mut AsyncApp,
) -> Result<Arc<CommitData>> {
    let state = repository.update(cx, |repository, cx| {
        repository.fetch_commit_data(sha, true, cx).clone()
    })?;
    match state {
        CommitDataState::Loaded(data) => Ok(data),
        CommitDataState::Loading(Some(receiver)) => receiver
            .await
            .map_err(|_| anyhow!("commit data for {sha} was not loaded")),
        CommitDataState::Loading(None) => Err(anyhow!("commit data for {sha} was not requested")),
    }
}

impl Focusable for CommitDetailsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CommitDetailsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let background = colors.panel_background;
        let separator = colors.border;
        let source = self.ref_source(cx);
        let mut content = Vec::new();
        for index in 0..self.entries.len() {
            if index > 0 {
                content.push(div().w_full().h(px(1.)).bg(separator).into_any_element());
            }
            if let Some(entry) = self.entries.get(index) {
                content.push(self.render_entry(index, entry, &source, window, cx));
            }
        }
        if self.requested_count > MAX_COMMITS_TO_LOAD && !self.entries.is_empty() {
            content.push(div().w_full().h(px(1.)).bg(separator).into_any_element());
            content.push(
                div()
                    .pl(SIDE_BORDER)
                    .child(Label::new(showing_selected_text(
                        MAX_COMMITS_TO_LOAD,
                        self.requested_count,
                    )))
                    .into_any_element(),
            );
        }
        let width = self.content_width.clone();
        let panel = cx.weak_entity();
        v_flex()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .bg(background)
            .child(
                div()
                    .relative()
                    .size_full()
                    .child(
                        div()
                            .id("git-log-details-scroll")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll_handle)
                            .child(v_flex().w_full().children(content)),
                    )
                    .child(
                        canvas(
                            move |bounds, window, _| {
                                if width.get() != bounds.size.width {
                                    width.set(bounds.size.width);
                                    window.on_next_frame(move |_, cx| {
                                        panel.update(cx, |_, cx| cx.notify()).ok();
                                    });
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .vertical_scrollbar_for(&self.scroll_handle, window, cx),
            )
            .children(self.render_status())
            .children(self.render_loading())
    }
}
