use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use editor::Editor;
use file_finder::{FileSearchQuery, FoundPath};
use file_icons::FileIcons;
use futures::future::join_all;
use fuzzy_nucleo::{PathMatch, StringMatchCandidate};
use gpui::{
    Action, AnyElement, App, Context, DismissEvent, Div, Entity, FocusHandle, Task, WeakEntity,
    Window,
};
use language::Point;
use picker::{Direction, ErasedEditor, Picker, PickerDelegate, PreviewUpdate};
use project::{Project, ProjectPath, Symbol, WorktreeId};
use search::text_finder::{SearchMatch, TextFinder};
use settings::Settings;
use ui::{
    Checkbox, Divider, HighlightedLabel, KeyBinding, LabelLike, ListItem, ListItemSpacing,
    ToggleButtonGroup, ToggleButtonGroupStyle, ToggleButtonSimple, Tooltip, prelude::*,
};
use util::{ResultExt, paths::PathMatcher, post_inc};
use workspace::{
    Workspace,
    item::{ItemSettings, PreviewTabsSettings},
};
use zed_actions::search_everywhere::{Tab, ToggleNonProjectItems};

use crate::{
    SearchEverywhere, actions, files,
    symbols::{self, SymbolScope},
    text,
};

pub(crate) const SEARCH_DEBOUNCE: Duration = Duration::from_millis(100);
const ALL_TAB_CLASS_LIMIT: usize = 4;
const ALL_TAB_FILE_LIMIT: usize = 6;
const ALL_TAB_SYMBOL_LIMIT: usize = 6;
const ALL_TAB_ACTION_LIMIT: usize = 6;
const ALL_TAB_TEXT_LIMIT: usize = 6;

pub(crate) const TABS: [Tab; 6] = [
    Tab::All,
    Tab::Classes,
    Tab::Files,
    Tab::Symbols,
    Tab::Actions,
    Tab::Text,
];

pub(crate) fn tab_label(tab: Tab) -> &'static str {
    match tab {
        Tab::All => "All",
        Tab::Classes => "Classes",
        Tab::Files => "Files",
        Tab::Symbols => "Symbols",
        Tab::Actions => "Actions",
        Tab::Text => "Text",
    }
}

pub(crate) fn adjacent_tab(tab: Tab, forward: bool) -> Tab {
    let current = TABS
        .iter()
        .position(|candidate| *candidate == tab)
        .unwrap_or_default();
    let next = if forward {
        (current + 1) % TABS.len()
    } else {
        (current + TABS.len() - 1) % TABS.len()
    };
    TABS.get(next).copied().unwrap_or_default()
}

pub(crate) fn activate_tab(
    picker: &mut Picker<SearchEverywhereDelegate>,
    tab: Tab,
    window: &mut Window,
    cx: &mut Context<Picker<SearchEverywhereDelegate>>,
) {
    picker.delegate.set_tab(tab);
    picker.refresh_placeholder(window, cx);
    picker.refresh(window, cx);
}

fn searches_files(tab: Tab) -> bool {
    matches!(tab, Tab::All | Tab::Files)
}

fn searches_actions(tab: Tab) -> bool {
    matches!(tab, Tab::All | Tab::Actions)
}

fn searches_text(tab: Tab) -> bool {
    matches!(tab, Tab::All | Tab::Text)
}

fn symbol_scope(tab: Tab) -> Option<SymbolScope> {
    match tab {
        Tab::All => Some(SymbolScope::ClassesAndOthers),
        Tab::Classes => Some(SymbolScope::ClassesOnly),
        Tab::Symbols => Some(SymbolScope::Everything),
        Tab::Files | Tab::Actions | Tab::Text => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Section {
    RecentFiles,
    Classes,
    Files,
    Symbols,
    Actions,
    Text,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Section::RecentFiles => "Recent Files",
            Section::Classes => "Classes",
            Section::Files => "Files",
            Section::Symbols => "Symbols",
            Section::Actions => "Actions",
            Section::Text => "Text",
        }
    }

    pub(crate) fn tab(self) -> Tab {
        match self {
            Section::RecentFiles | Section::Files => Tab::Files,
            Section::Classes => Tab::Classes,
            Section::Symbols => Tab::Symbols,
            Section::Actions => Tab::Actions,
            Section::Text => Tab::Text,
        }
    }

    fn all_tab_limit(self) -> usize {
        match self {
            Section::RecentFiles => usize::MAX,
            Section::Classes => ALL_TAB_CLASS_LIMIT,
            Section::Files => ALL_TAB_FILE_LIMIT,
            Section::Symbols => ALL_TAB_SYMBOL_LIMIT,
            Section::Actions => ALL_TAB_ACTION_LIMIT,
            Section::Text => ALL_TAB_TEXT_LIMIT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    Header(Section),
    Item(Section, usize),
    More(Section),
    OpenInTextFinder,
}

impl Entry {
    fn section(self) -> Section {
        match self {
            Entry::Header(section) | Entry::Item(section, _) | Entry::More(section) => section,
            Entry::OpenInTextFinder => Section::Text,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Files,
    Symbols,
    Actions,
    Text,
}

impl Source {
    fn is_fast(self) -> bool {
        matches!(self, Source::Files | Source::Actions)
    }
}

pub(crate) struct Command {
    pub(crate) name: SharedString,
    pub(crate) action: Box<dyn Action>,
}

#[derive(Default)]
pub(crate) struct Results {
    pub(crate) files: Vec<PathMatch>,
    pub(crate) symbols: Vec<Symbol>,
    pub(crate) classes: Vec<fuzzy::StringMatch>,
    pub(crate) other_symbols: Vec<fuzzy::StringMatch>,
    pub(crate) actions: Vec<fuzzy_nucleo::StringMatch>,
    pub(crate) text: Vec<SearchMatch>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Presentation {
    Modal,
    Panel,
}

#[derive(Clone)]
pub(crate) struct TextScope {
    pub(crate) directory: SharedString,
    pub(crate) matcher: PathMatcher,
}

pub(crate) struct SearchEverywhereDelegate {
    pub(crate) search_everywhere: WeakEntity<SearchEverywhere>,
    pub(crate) presentation: Presentation,
    pub(crate) workspace: WeakEntity<Workspace>,
    pub(crate) project: Entity<Project>,
    pub(crate) previous_focus_handle: FocusHandle,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) tab: Tab,
    pub(crate) include_non_project_items: bool,
    pub(crate) text_scope: Option<TextScope>,
    pub(crate) query: String,
    pub(crate) file_query: Option<FileSearchQuery>,
    pub(crate) recent_files: Vec<FoundPath>,
    pub(crate) commands: Vec<Command>,
    pub(crate) action_candidates: Arc<[StringMatchCandidate]>,
    pub(crate) results: Results,
    pub(crate) entries: Vec<Entry>,
    pub(crate) selected_index: usize,
    pub(crate) selection_moved: bool,
    pub(crate) search_count: usize,
    pub(crate) latest_search_id: usize,
    pub(crate) latest_search_has_results: bool,
    pub(crate) pending_fast_sources: usize,
    pub(crate) cancel_flag: Arc<AtomicBool>,
    pub(crate) search_in_flight: Arc<AtomicBool>,
}

impl SearchEverywhereDelegate {
    pub(crate) fn new(
        search_everywhere: WeakEntity<SearchEverywhere>,
        presentation: Presentation,
        workspace: WeakEntity<Workspace>,
        project: Entity<Project>,
        previous_focus_handle: FocusHandle,
        recent_files: Vec<FoundPath>,
        actions: Vec<(SharedString, Box<dyn Action>)>,
        tab: Tab,
        focus_handle: FocusHandle,
    ) -> Self {
        let (commands, action_candidates) = Self::build_commands(actions);
        Self {
            search_everywhere,
            presentation,
            workspace,
            project,
            previous_focus_handle,
            focus_handle,
            tab,
            include_non_project_items: false,
            text_scope: None,
            query: String::new(),
            file_query: None,
            recent_files,
            commands,
            action_candidates,
            results: Results::default(),
            entries: Vec::new(),
            selected_index: 0,
            selection_moved: false,
            search_count: 0,
            latest_search_id: 0,
            latest_search_has_results: false,
            pending_fast_sources: 0,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            search_in_flight: Arc::new(AtomicBool::new(false)),
        }
    }

    fn build_commands(
        actions: Vec<(SharedString, Box<dyn Action>)>,
    ) -> (Vec<Command>, Arc<[StringMatchCandidate]>) {
        let mut commands = actions
            .into_iter()
            .map(|(name, action)| Command { name, action })
            .collect::<Vec<_>>();
        commands.sort_by(|left, right| left.name.cmp(&right.name));
        let action_candidates = commands
            .iter()
            .enumerate()
            .map(|(index, command)| StringMatchCandidate::new(index, command.name.clone()))
            .collect();
        (commands, action_candidates)
    }

    pub(crate) fn set_focus_handle(&mut self, focus_handle: FocusHandle) {
        self.focus_handle = focus_handle;
    }

    pub(crate) fn set_previous_focus_handle(&mut self, previous_focus_handle: FocusHandle) {
        self.previous_focus_handle = previous_focus_handle;
    }

    pub(crate) fn set_actions(&mut self, actions: Vec<(SharedString, Box<dyn Action>)>) -> bool {
        let (commands, action_candidates) = Self::build_commands(actions);
        let unchanged = commands.len() == self.commands.len()
            && commands
                .iter()
                .zip(&self.commands)
                .all(|(new, old)| new.name == old.name);
        if unchanged {
            return false;
        }
        self.commands = commands;
        self.action_candidates = action_candidates;
        true
    }

    pub(crate) fn set_recent_files(&mut self, recent_files: Vec<FoundPath>) -> bool {
        if self.recent_files == recent_files {
            return false;
        }
        self.recent_files = recent_files;
        true
    }

    pub(crate) fn set_text_scope(&mut self, text_scope: Option<TextScope>) {
        self.text_scope = text_scope;
    }

    pub(crate) fn tab(&self) -> Tab {
        self.tab
    }

    pub(crate) fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.selection_moved = false;
    }

    pub(crate) fn toggle_non_project_items(&mut self) {
        self.include_non_project_items = !self.include_non_project_items;
        self.selection_moved = false;
    }

    pub(crate) fn apply_results(
        &mut self,
        search_id: usize,
        source: Source,
        cx: &mut Context<Picker<Self>>,
        update: impl FnOnce(&mut Results),
    ) {
        if search_id != self.latest_search_id {
            return;
        }
        update(&mut self.results);
        if source.is_fast() {
            self.pending_fast_sources = self.pending_fast_sources.saturating_sub(1);
        }
        self.latest_search_has_results = true;
        self.rebuild_entries();
        cx.notify();
    }

    pub(crate) fn section_len(&self, section: Section) -> usize {
        match section {
            Section::RecentFiles => self.recent_files.len(),
            Section::Classes => self.results.classes.len(),
            Section::Files => self.results.files.len(),
            Section::Symbols => self.results.other_symbols.len(),
            Section::Actions => self.results.actions.len(),
            Section::Text => self.results.text.len(),
        }
    }

    fn visible_sections(&self) -> &'static [Section] {
        const ALL_TAB_SECTIONS: [Section; 5] = [
            Section::Classes,
            Section::Files,
            Section::Symbols,
            Section::Actions,
            Section::Text,
        ];
        match self.tab {
            Tab::All | Tab::Files if self.query.is_empty() => &[Section::RecentFiles],
            Tab::All => &ALL_TAB_SECTIONS,
            Tab::Classes => &[Section::Classes],
            Tab::Files => &[Section::Files],
            Tab::Symbols => &[Section::Symbols],
            Tab::Actions => &[Section::Actions],
            Tab::Text => &[Section::Text],
        }
    }

    pub(crate) fn rebuild_entries(&mut self) {
        let previously_selected = if self.selection_moved {
            self.entries.get(self.selected_index).copied()
        } else {
            None
        };
        let grouped = self.tab == Tab::All;

        let mut entries = Vec::new();
        for &section in self.visible_sections() {
            let len = self.section_len(section);
            if len == 0 {
                continue;
            }
            if grouped {
                entries.push(Entry::Header(section));
            }
            let shown = if grouped {
                len.min(section.all_tab_limit())
            } else {
                len
            };
            entries.extend((0..shown).map(|index| Entry::Item(section, index)));
            if shown < len {
                entries.push(Entry::More(section));
            }
        }
        if self.tab == Tab::Text && !self.query.is_empty() {
            entries.push(Entry::OpenInTextFinder);
        }
        self.entries = entries;

        self.selected_index = previously_selected
            .and_then(|selected| self.entries.iter().position(|entry| *entry == selected))
            .or_else(|| self.first_selectable_index())
            .unwrap_or_default();
    }

    fn first_selectable_index(&self) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| !matches!(entry, Entry::Header(_)))
    }

    pub(crate) fn section_start(&self, direction: Direction) -> Option<usize> {
        let mut section_starts: Vec<(Section, usize)> = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if matches!(entry, Entry::Header(_)) {
                continue;
            }
            if section_starts
                .last()
                .is_none_or(|(section, _)| *section != entry.section())
            {
                section_starts.push((entry.section(), index));
            }
        }
        let current_section = self.entries.get(self.selected_index)?.section();
        let current = section_starts
            .iter()
            .position(|(section, _)| *section == current_section)?;

        if section_starts.len() == 1 {
            return match direction {
                Direction::Down => self
                    .entries
                    .iter()
                    .rposition(|entry| !matches!(entry, Entry::Header(_))),
                Direction::Up => self.first_selectable_index(),
            };
        }

        let target = match direction {
            Direction::Down => (current + 1) % section_starts.len(),
            Direction::Up => (current + section_starts.len() - 1) % section_starts.len(),
        };
        section_starts.get(target).map(|(_, index)| *index)
    }

    fn clear_inactive_results(&mut self) {
        if !searches_files(self.tab) {
            self.results.files.clear();
        }
        if symbol_scope(self.tab).is_none() {
            self.results.symbols.clear();
            self.results.classes.clear();
            self.results.other_symbols.clear();
        }
        if !searches_actions(self.tab) {
            self.results.actions.clear();
        }
        if !searches_text(self.tab) {
            self.results.text.clear();
        }
    }

    fn all_actions(&self) -> Vec<fuzzy_nucleo::StringMatch> {
        self.commands
            .iter()
            .enumerate()
            .map(|(index, command)| fuzzy_nucleo::StringMatch {
                candidate_id: index,
                score: 0.,
                positions: Vec::new(),
                string: command.name.clone(),
            })
            .collect()
    }

    fn spawn_sources(
        &mut self,
        search_id: usize,
        cancel_flag: Arc<AtomicBool>,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Vec<Task<()>> {
        let mut tasks = Vec::new();
        let include_non_project_items = self.include_non_project_items;
        if searches_files(self.tab)
            && let Some(file_query) = &self.file_query
        {
            tasks.push(files::search(
                self.project.clone(),
                file_query.path_query().to_string(),
                include_non_project_items,
                search_id,
                cancel_flag.clone(),
                window,
                cx,
            ));
        }
        if let Some(scope) = symbol_scope(self.tab) {
            tasks.push(symbols::search(
                self.project.clone(),
                self.query.clone(),
                scope,
                include_non_project_items,
                search_id,
                window,
                cx,
            ));
        }
        if searches_actions(self.tab) {
            tasks.push(actions::search(
                self.query.clone(),
                self.action_candidates.clone(),
                search_id,
                cancel_flag.clone(),
                window,
                cx,
            ));
        }
        if searches_text(self.tab) {
            tasks.push(text::search(
                self.project.clone(),
                &self.query,
                self.text_scope.as_ref().map(|scope| scope.matcher.clone()),
                include_non_project_items,
                search_id,
                cancel_flag,
                window,
                cx,
            ));
        }
        tasks
    }

    fn dismiss(&self, cx: &mut App) {
        self.search_everywhere
            .update(cx, |_, cx| cx.emit(DismissEvent))
            .ok();
    }

    fn selected_entry(&self) -> Option<Entry> {
        self.entries.get(self.selected_index).copied()
    }

    fn selected_symbol(&self, section: Section, index: usize) -> Option<&Symbol> {
        let string_match = match section {
            Section::Classes => self.results.classes.get(index),
            Section::Symbols => self.results.other_symbols.get(index),
            Section::RecentFiles | Section::Files | Section::Actions | Section::Text => None,
        }?;
        self.results.symbols.get(string_match.candidate_id)
    }

    fn selected_command(&self, index: usize) -> Option<&Command> {
        let action_match = self.results.actions.get(index)?;
        self.commands.get(action_match.candidate_id)
    }

    fn abs_path_for_file_match(
        &self,
        path_match: &PathMatch,
        cx: &App,
    ) -> Option<std::path::PathBuf> {
        Some(
            self.project
                .read(cx)
                .worktree_for_id(WorktreeId::from_usize(path_match.worktree_id), cx)?
                .read(cx)
                .absolutize(&path_match.path),
        )
    }

    fn open_path(
        &self,
        project_path: ProjectPath,
        secondary: bool,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let allow_preview = PreviewTabsSettings::get_global(cx).enable_preview_from_file_finder;
        let open_task = workspace.update(cx, |workspace, cx| {
            if secondary {
                workspace.split_path_preview(project_path, allow_preview, None, window, cx)
            } else {
                workspace.open_path_preview(
                    project_path,
                    None,
                    true,
                    allow_preview,
                    true,
                    window,
                    cx,
                )
            }
        });
        let file_query = self.file_query.clone();
        cx.spawn_in(window, async move |_, cx| {
            let item = open_task.await.log_err()?;
            let editor = item.downcast::<Editor>()?;
            let file_query = file_query?;
            editor
                .downgrade()
                .update_in(cx, |editor, window, cx| {
                    let buffer = editor.buffer().read(cx).as_singleton()?;
                    let snapshot = buffer.read(cx).snapshot();
                    let range = file_query.selection_range(&snapshot)?;
                    editor.go_to_singleton_buffer_range(range, window, cx);
                    Some(())
                })
                .log_err();
            Some(())
        })
        .detach();
        self.dismiss(cx);
    }

    fn open_recent_file(
        &self,
        found_path: &FoundPath,
        secondary: bool,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) {
        let worktree_exists = self
            .project
            .read(cx)
            .worktree_for_id(found_path.project.worktree_id, cx)
            .is_some();
        if worktree_exists {
            self.open_path(found_path.project.clone(), secondary, window, cx);
            return;
        }
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let absolute = found_path.absolute.clone();
        workspace.update(cx, |workspace, cx| {
            workspace
                .open_abs_path(
                    absolute,
                    workspace::OpenOptions {
                        visible: Some(workspace::OpenVisible::None),
                        ..Default::default()
                    },
                    window,
                    cx,
                )
                .detach_and_log_err(cx);
        });
        self.dismiss(cx);
    }

    fn reveal_folder(&self, project_path: &ProjectPath, cx: &mut Context<Picker<Self>>) {
        let entry_id = self
            .project
            .read(cx)
            .entry_for_path(project_path, cx)
            .map(|entry| entry.id);
        self.dismiss(cx);
        if let Some(entry_id) = entry_id {
            self.project.update(cx, |_, cx| {
                cx.emit(project::Event::RevealInProjectPanel(entry_id));
            });
        }
    }

    fn open_text_match(
        &self,
        search_match: &SearchMatch,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let point = Point::new(
            search_match.line_number.saturating_sub(1),
            search_match.match_start_byte_column,
        );
        let path = search_match.path.clone();
        let open_task = workspace.update(cx, |workspace, cx| {
            workspace.open_path_preview(path, None, true, false, true, window, cx)
        });
        cx.spawn_in(window, async move |_, cx| {
            let item = open_task.await.log_err()?;
            let editor = item.downcast::<Editor>()?;
            editor
                .downgrade()
                .update_in(cx, |editor, window, cx| {
                    editor.go_to_singleton_buffer_point(point, window, cx);
                })
                .log_err();
            Some(())
        })
        .detach();
        self.dismiss(cx);
    }

    fn run_action(
        &self,
        index: usize,
        secondary: bool,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) {
        let Some(command) = self.selected_command(index) else {
            return;
        };
        if secondary {
            let change_keybinding = Box::new(zed_actions::ChangeKeybinding {
                action: command.action.name().to_string(),
            });
            window.dispatch_action(change_keybinding, cx);
            self.dismiss(cx);
            return;
        }
        let action = command.action.boxed_clone();
        window.focus(&self.previous_focus_handle, cx);
        self.dismiss(cx);
        window.dispatch_action(action, cx);
    }

    fn open_in_text_finder(&self, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        self.dismiss(cx);
        let query = self.query.clone();
        workspace.update(cx, |workspace, cx| {
            TextFinder::open_with_query(workspace, query, window, cx).detach();
        });
    }

    fn render_tab_bar(&self, cx: &mut Context<Picker<Self>>) -> impl IntoElement {
        let buttons = TABS.map(|tab| {
            ToggleButtonSimple::new(
                tab_label(tab),
                cx.listener(move |picker, _, window, cx| activate_tab(picker, tab, window, cx)),
            )
        });
        let selected_index = TABS
            .iter()
            .position(|tab| *tab == self.tab)
            .unwrap_or_default();
        let group = ToggleButtonGroup::single_row("search-everywhere-tabs", buttons)
            .label_size(LabelSize::Small)
            .style(ToggleButtonGroupStyle::Transparent)
            .selected_index(selected_index);
        let group = match self.presentation {
            Presentation::Modal => group.auto_width(),
            Presentation::Panel => group,
        };
        h_flex().px_2().pt_2().pb_1().w_full().child(group)
    }

    fn render_text_scope(&self, cx: &mut Context<Picker<Self>>) -> Option<AnyElement> {
        let scope = self.text_scope.as_ref()?;
        Some(
            h_flex()
                .min_w_0()
                .gap_1()
                .child(
                    Label::new(format!("Text in {}", scope.directory))
                        .size(LabelSize::Small)
                        .color(Color::Accent)
                        .single_line()
                        .truncate(),
                )
                .child(
                    IconButton::new("search-everywhere-clear-text-scope", IconName::Close)
                        .icon_size(IconSize::XSmall)
                        .tooltip(Tooltip::text("Clear Text Scope"))
                        .on_click(cx.listener(|picker, _, window, cx| {
                            picker.delegate.set_text_scope(None);
                            picker.refresh(window, cx);
                        })),
                )
                .into_any_element(),
        )
    }

    fn render_file_row(
        &self,
        file_name: String,
        file_name_positions: Vec<usize>,
        directory: String,
        directory_positions: Vec<usize>,
        icon: Option<SharedString>,
    ) -> Div {
        h_flex()
            .w_full()
            .min_w_0()
            .gap_1p5()
            .children(icon.map(|icon| {
                Icon::from_path(icon)
                    .color(Color::Muted)
                    .size(IconSize::Small)
            }))
            .child(
                HighlightedLabel::new(file_name, file_name_positions)
                    .single_line()
                    .flex_none(),
            )
            .child(
                HighlightedLabel::new(directory, directory_positions)
                    .single_line()
                    .size(LabelSize::Small)
                    .color(Color::Muted)
                    .flex_1()
                    .truncate_start(),
            )
    }

    fn render_item(&self, section: Section, index: usize, cx: &App) -> Option<AnyElement> {
        let path_style = self.project.read(cx).path_style(cx);
        let show_file_icons = ItemSettings::get_global(cx).file_icons;
        match section {
            Section::RecentFiles => {
                let found_path = self.recent_files.get(index)?;
                let file_name = found_path
                    .absolute
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let directory = found_path
                    .project
                    .path
                    .parent()
                    .map(|parent| parent.display(path_style).into_owned())
                    .unwrap_or_default();
                let icon = show_file_icons
                    .then(|| FileIcons::get_icon(&found_path.absolute, cx))
                    .flatten();
                Some(
                    self.render_file_row(file_name, Vec::new(), directory, Vec::new(), icon)
                        .into_any_element(),
                )
            }
            Section::Files => {
                let path_match = self.results.files.get(index)?;
                let (file_name, file_name_positions, directory, directory_positions) =
                    file_finder::path_match_labels(path_match, path_style);
                let icon_path = Path::new(&file_name);
                let icon = if path_match.is_dir {
                    FileIcons::get_folder_icon(false, icon_path, cx)
                } else if show_file_icons {
                    FileIcons::get_icon(icon_path, cx)
                } else {
                    None
                };
                Some(
                    self.render_file_row(
                        file_name,
                        file_name_positions,
                        directory,
                        directory_positions,
                        icon,
                    )
                    .into_any_element(),
                )
            }
            Section::Classes | Section::Symbols => {
                let string_match = match section {
                    Section::Classes => self.results.classes.get(index),
                    _ => self.results.other_symbols.get(index),
                }?;
                let symbol = self.results.symbols.get(string_match.candidate_id)?;
                let show_worktree_root_name =
                    self.project.read(cx).visible_worktrees(cx).count() > 1;
                let location = format!(
                    "{}:{}",
                    project_symbols::symbol_path_label(
                        symbol,
                        self.project.read(cx),
                        show_worktree_root_name,
                        cx,
                    ),
                    symbol.range.start.0.row + 1
                );
                Some(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap_2()
                        .child(div().flex_none().child(LabelLike::new().child(
                            project_symbols::symbol_label(symbol, &string_match.positions, cx),
                        )))
                        .child(
                            Label::new(location)
                                .size(LabelSize::Small)
                                .color(Color::Muted)
                                .single_line()
                                .truncate_start(),
                        )
                        .into_any_element(),
                )
            }
            Section::Actions => {
                let action_match = self.results.actions.get(index)?;
                let command = self.commands.get(action_match.candidate_id)?;
                Some(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .justify_between()
                        .gap_2()
                        .child(
                            HighlightedLabel::new(
                                command.name.clone(),
                                action_match.positions.clone(),
                            )
                            .single_line()
                            .truncate(),
                        )
                        .child(KeyBinding::for_action_in(
                            &*command.action,
                            &self.previous_focus_handle,
                            cx,
                        ))
                        .into_any_element(),
                )
            }
            Section::Text => {
                let search_match = self.results.text.get(index)?;
                let file_name = search_match
                    .path
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string();
                Some(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(search::text_finder::render_matched_line(search_match, cx)),
                        )
                        .child(
                            Label::new(format!("{file_name}:{}", search_match.line_number))
                                .size(LabelSize::Small)
                                .color(Color::Muted)
                                .single_line(),
                        )
                        .into_any_element(),
                )
            }
        }
    }
}

impl PickerDelegate for SearchEverywhereDelegate {
    type ListItem = ListItem;

    fn name() -> &'static str {
        "search everywhere"
    }

    fn placeholder_text(&self, _window: &mut Window, _cx: &mut App) -> Arc<str> {
        match self.tab {
            Tab::All => "Search everywhere…",
            Tab::Classes => "Search classes…",
            Tab::Files => "Search files and folders…",
            Tab::Symbols => "Search symbols…",
            Tab::Actions => "Search actions…",
            Tab::Text => "Search text…",
        }
        .into()
    }

    fn no_matches_text(&self, _window: &mut Window, _cx: &mut App) -> Option<SharedString> {
        (!self.query.is_empty()).then(|| "Nothing found".into())
    }

    fn match_count(&self) -> usize {
        self.entries.len()
    }

    fn selected_index(&self) -> usize {
        self.selected_index
    }

    fn set_selected_index(
        &mut self,
        index: usize,
        _window: &mut Window,
        _cx: &mut Context<Picker<Self>>,
    ) {
        self.selected_index = index;
        self.selection_moved = true;
    }

    fn can_select(
        &self,
        index: usize,
        _window: &mut Window,
        _cx: &mut Context<Picker<Self>>,
    ) -> bool {
        self.entries
            .get(index)
            .is_some_and(|entry| !matches!(entry, Entry::Header(_)))
    }

    fn update_matches(
        &mut self,
        raw_query: String,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Task<()> {
        let query = raw_query.trim().to_string();
        if query != self.query {
            self.selection_moved = false;
        }
        self.query = query;
        let search_id = post_inc(&mut self.search_count);
        self.latest_search_id = search_id;
        self.latest_search_has_results = false;
        self.pending_fast_sources = 0;
        self.cancel_flag.store(true, Ordering::Release);
        self.cancel_flag = Arc::new(AtomicBool::new(false));
        self.clear_inactive_results();

        if self.query.is_empty() {
            self.results = Results::default();
            if self.tab == Tab::Actions {
                self.results.actions = self.all_actions();
            }
            self.file_query = None;
            self.latest_search_has_results = true;
            self.search_in_flight.store(false, Ordering::Release);
            self.rebuild_entries();
            cx.notify();
            return Task::ready(());
        }

        self.file_query = Some(file_finder::parse_file_search_query(&self.query));
        self.pending_fast_sources =
            usize::from(searches_files(self.tab)) + usize::from(searches_actions(self.tab));
        self.rebuild_entries();
        cx.notify();
        let cancel_flag = self.cancel_flag.clone();
        let search_in_flight = self.search_in_flight.clone();
        let was_in_flight = search_in_flight.swap(true, Ordering::AcqRel);
        cx.spawn_in(window, async move |picker, cx| {
            if was_in_flight {
                cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            }
            if cancel_flag.load(Ordering::Acquire) {
                return;
            }
            let Ok(sources) = picker.update_in(cx, |picker, window, cx| {
                picker
                    .delegate
                    .spawn_sources(search_id, cancel_flag, window, cx)
            }) else {
                return;
            };
            join_all(sources).await;
            search_in_flight.store(false, Ordering::Release);
        })
    }

    fn finalize_update_matches(
        &mut self,
        _query: String,
        _duration: Duration,
        _window: &mut Window,
        _cx: &mut Context<Picker<Self>>,
    ) -> bool {
        self.latest_search_has_results && self.pending_fast_sources == 0
    }

    fn confirm(&mut self, secondary: bool, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        match entry {
            Entry::Header(_) => {}
            Entry::More(section) => {
                let tab = section.tab();
                cx.defer_in(window, move |picker, window, cx| {
                    activate_tab(picker, tab, window, cx);
                });
            }
            Entry::OpenInTextFinder => self.open_in_text_finder(window, cx),
            Entry::Item(section, index) => match section {
                Section::RecentFiles => {
                    if let Some(found_path) = self.recent_files.get(index).cloned() {
                        self.open_recent_file(&found_path, secondary, window, cx);
                    }
                }
                Section::Files => {
                    let Some(path_match) = self.results.files.get(index) else {
                        return;
                    };
                    let is_dir = path_match.is_dir;
                    let project_path =
                        file_finder::project_path_for_search_match(&self.project, path_match, cx);
                    if is_dir {
                        self.reveal_folder(&project_path, cx);
                    } else {
                        self.open_path(project_path, secondary, window, cx);
                    }
                }
                Section::Classes | Section::Symbols => {
                    let Some(symbol) = self.selected_symbol(section, index).cloned() else {
                        return;
                    };
                    project_symbols::open_symbol(
                        symbol,
                        secondary,
                        self.workspace.clone(),
                        &self.project,
                        window,
                        cx,
                    )
                    .detach_and_log_err(cx);
                    self.dismiss(cx);
                }
                Section::Actions => self.run_action(index, secondary, window, cx),
                Section::Text => {
                    if let Some(search_match) = self.results.text.get(index).cloned() {
                        self.open_text_match(&search_match, window, cx);
                    }
                }
            },
        }
    }

    fn dismissed(&mut self, _window: &mut Window, cx: &mut Context<Picker<Self>>) {
        self.dismiss(cx);
    }

    fn try_get_preview_data_for_match(&self, cx: &App) -> Option<PreviewUpdate> {
        let message = |text: String| {
            let mut builder = picker::HighlightedTextBuilder::default();
            builder.push_plain(text);
            PreviewUpdate::message(builder.build())
        };
        match self.selected_entry()? {
            Entry::Header(_) => None,
            Entry::More(section) => Some(message(format!("Show all {}", section.title()))),
            Entry::OpenInTextFinder => {
                Some(message(format!("Search \"{}\" in Text Finder", self.query)))
            }
            Entry::Item(section, index) => match section {
                Section::RecentFiles => Some(PreviewUpdate::from_path(
                    self.recent_files.get(index)?.absolute.clone(),
                )),
                Section::Files => {
                    let path_match = self.results.files.get(index)?;
                    let abs_path = self.abs_path_for_file_match(path_match, cx)?;
                    if path_match.is_dir {
                        Some(message(format!("Folder {}", abs_path.display())))
                    } else {
                        Some(PreviewUpdate::from_path(abs_path))
                    }
                }
                Section::Classes | Section::Symbols => Some(PreviewUpdate::from_symbol(
                    self.selected_symbol(section, index)?.clone(),
                )),
                Section::Actions => Some(message(self.selected_command(index)?.name.to_string())),
                Section::Text => {
                    let search_match = self.results.text.get(index)?;
                    Some(PreviewUpdate::from_buffer(
                        search_match.buffer.clone(),
                        picker::MatchLocation {
                            anchor_range: search_match.anchor_range.clone(),
                            range: search_match.range.clone(),
                        },
                    ))
                }
            },
        }
    }

    fn render_editor(
        &self,
        editor: &Arc<dyn ErasedEditor>,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Option<Div> {
        let query_row = h_flex()
            .h_9()
            .px_2p5()
            .flex_none()
            .overflow_hidden()
            .child(div().flex_1().child(editor.render(window, cx)));
        Some(match self.presentation {
            Presentation::Modal => v_flex()
                .child(self.render_tab_bar(cx))
                .child(query_row.children(self.searchbar_trailer(window, cx)))
                .child(Divider::horizontal()),
            Presentation::Panel => v_flex()
                .child(self.render_tab_bar(cx))
                .child(query_row)
                .child(
                    h_flex()
                        .px_2p5()
                        .pb_1p5()
                        .flex_none()
                        .overflow_hidden()
                        .children(self.searchbar_trailer(window, cx)),
                )
                .child(Divider::horizontal()),
        })
    }

    fn searchbar_trailer(
        &self,
        _window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Option<AnyElement> {
        let focus_handle = self.focus_handle.clone();
        Some(
            h_flex()
                .gap_2()
                .children(self.render_text_scope(cx))
                .child(
                    Checkbox::new(
                        "search-everywhere-include-non-project-items",
                        self.include_non_project_items.into(),
                    )
                    .label("Include non-project items")
                    .label_size(LabelSize::Small)
                    .tooltip(move |_window, cx| {
                        Tooltip::for_action_in(
                            "Include Non-Project Items",
                            &ToggleNonProjectItems,
                            &focus_handle,
                            cx,
                        )
                    })
                    .on_click(cx.listener(|picker, _, window, cx| {
                        picker.delegate.toggle_non_project_items();
                        picker.refresh(window, cx);
                    })),
                )
                .children(picker::parts::project_scan_indicator(
                    !self.query.is_empty(),
                    &self.project,
                    cx,
                ))
                .into_any_element(),
        )
    }

    fn render_match(
        &self,
        index: usize,
        selected: bool,
        _window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Option<Self::ListItem> {
        let entry = self.entries.get(index).copied()?;
        let row = ListItem::new(index)
            .height(rems(1.75))
            .spacing(ListItemSpacing::Sparse)
            .inset(true);
        let row = match entry {
            Entry::Header(section) => row.selectable(false).child(
                Label::new(section.title())
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            ),
            Entry::More(_) => row.toggle_state(selected).child(
                Label::new("more…")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            ),
            Entry::OpenInTextFinder => row.toggle_state(selected).child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_2()
                    .child(
                        Label::new("Open in Text Finder")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(KeyBinding::for_action_in(
                        &zed_actions::text_finder::Toggle,
                        &self.previous_focus_handle,
                        cx,
                    )),
            ),
            Entry::Item(section, item_index) => row
                .toggle_state(selected)
                .child(self.render_item(section, item_index, cx)?),
        };
        Some(row)
    }
}
