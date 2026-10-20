use std::path::Path;
use std::sync::Arc;

use git::console::{GitConsoleChunk, GitConsoleContentType, GitConsoleLine};
use gpui::{Entity, SharedString, TestAppContext, VisualTestContext, px};
use project::{FakeFs, Project};
use serde_json::json;
use settings::SettingsStore;
use util::path;
use workspace::MultiWorkspace;

use super::console::{console_document, kept_lines_start};
use super::content_tabs::{
    CONSOLE_TAB_ORDER_WEIGHT, ContentTabKey, ContentTabs, TabSlot, layout_tab_strip,
    order_weight_for_tab_name,
};
use super::header::{
    TabMenuCommand, TabMenuItem, pill_vertical_geometry, tab_decoration, tab_menu_items, tab_width,
};
use super::persistence::{self, PersistedLogTab, PersistedTabs, restore_plan, uuid_from_random};
use super::*;

fn log_key(id: &str) -> ContentTabKey {
    ContentTabKey::Log(SharedString::from(id.to_string()))
}

fn main_key() -> ContentTabKey {
    log_key(MAIN_LOG_TAB_ID)
}

fn console_line(text: &str, content_type: GitConsoleContentType) -> GitConsoleLine {
    GitConsoleLine::new(vec![GitConsoleChunk {
        text: SharedString::from(text.to_string()),
        content_type,
    }])
    .expect("non-blank console line")
}

fn init_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
        cx.set_global(db::AppDatabase::test_new());
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        editor::init(cx);
        crate::init(cx);
    });
}

async fn tool_window_for_repository(
    cx: &mut TestAppContext,
) -> (Arc<FakeFs>, Entity<GitToolWindow>, VisualTestContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.background_executor.clone());
    fs.insert_tree(
        path!("/project"),
        json!({ ".git": {}, "file.txt": "content" }),
    )
    .await;
    let project = Project::test(fs.clone(), [Path::new(path!("/project"))], cx).await;
    let window_handle =
        cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
    let workspace = window_handle
        .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
        .expect("the window has a workspace");
    let mut cx = VisualTestContext::from_window(window_handle.into(), cx);
    cx.run_until_parked();
    let tool_window = workspace.update_in(&mut cx, |workspace, window, cx| {
        let workspace_handle = workspace.weak_handle();
        let project = workspace.project().clone();
        cx.new(|cx| GitToolWindow::new(workspace_handle, project, window, cx))
    });
    cx.run_until_parked();
    (fs, tool_window, cx)
}

fn push_console_line(fs: &FakeFs, text: &str, content_type: GitConsoleContentType) {
    let sink = fs
        .with_git_state(Path::new(path!("/project/.git")), false, |state| {
            state.console.clone()
        })
        .expect("the fake repository exists")
        .expect("the git store installed its console sink");
    sink.push(console_line(text, content_type));
}

fn tab_keys(tool_window: &Entity<GitToolWindow>, cx: &VisualTestContext) -> Vec<ContentTabKey> {
    tool_window.read_with(cx, |tool_window, _| {
        tool_window.tabs.keys().cloned().collect()
    })
}

fn selected_key(
    tool_window: &Entity<GitToolWindow>,
    cx: &VisualTestContext,
) -> Option<ContentTabKey> {
    tool_window.read_with(cx, |tool_window, _| tool_window.tabs.selected().cloned())
}

#[test]
fn console_tab_is_placed_by_weight_like_changes_view_content_manager() {
    let mut tabs = ContentTabs::default();
    tabs.append(main_key(), order_weight_for_tab_name("Log"));
    tabs.append(log_key("filtered"), order_weight_for_tab_name("Log: HEAD"));
    tabs.insert_by_weight(ContentTabKey::Console, CONSOLE_TAB_ORDER_WEIGHT);
    assert_eq!(
        tabs.keys().cloned().collect::<Vec<_>>(),
        vec![main_key(), ContentTabKey::Console, log_key("filtered")],
        "a tab named 'Log: HEAD' weighs OTHER (100), so the console (60) goes before it"
    );

    let mut tabs = ContentTabs::default();
    tabs.append(main_key(), order_weight_for_tab_name("Log"));
    tabs.append(log_key("plain"), order_weight_for_tab_name("Log"));
    tabs.insert_by_weight(ContentTabKey::Console, CONSOLE_TAB_ORDER_WEIGHT);
    assert_eq!(
        tabs.keys().cloned().collect::<Vec<_>>(),
        vec![main_key(), log_key("plain"), ContentTabKey::Console],
        "a filterless extra tab keeps the VCS_LOG weight (50) and stays before the console"
    );
}

#[test]
fn removing_the_selected_tab_selects_the_most_recently_selected_one() {
    let mut tabs = ContentTabs::default();
    tabs.append(main_key(), 50);
    tabs.append(log_key("second"), 100);
    tabs.append(log_key("third"), 100);
    tabs.select(&main_key());
    tabs.select(&log_key("third"));
    tabs.select(&log_key("second"));

    assert!(tabs.remove(&log_key("second")));
    assert_eq!(
        tabs.selected(),
        Some(&log_key("third")),
        "ContentManagerImpl.doRemoveContent selects the head of the selection history"
    );
    assert!(tabs.remove(&log_key("third")));
    assert_eq!(tabs.selected(), Some(&main_key()));
    assert!(!tabs.remove(&log_key("third")), "removing twice is a no-op");
}

#[test]
fn next_and_previous_tab_wrap_around() {
    let mut tabs = ContentTabs::default();
    tabs.append(main_key(), 50);
    assert_eq!(tabs.next_key(), None, "navigation needs more than one tab");
    tabs.append(ContentTabKey::Console, 60);
    tabs.select(&main_key());
    assert_eq!(tabs.next_key(), Some(ContentTabKey::Console));
    assert_eq!(tabs.previous_key(), Some(ContentTabKey::Console));
    tabs.select(&ContentTabKey::Console);
    assert_eq!(tabs.next_key(), Some(main_key()));
}

#[test]
fn tab_menu_lists_the_intellij_content_actions_and_shows_their_shortcuts() {
    let several_tabs = TabMenuState {
        closeable: true,
        can_close_all: true,
        can_close_others: true,
        can_navigate: true,
    };
    let command =
        |command: TabMenuCommand, enabled: bool| TabMenuItem::Command { command, enabled };
    assert_eq!(
        tab_menu_items(&several_tabs),
        vec![
            command(TabMenuCommand::CloseTab, true),
            command(TabMenuCommand::CloseAllTabs, true),
            command(TabMenuCommand::CloseOtherTabs, true),
            TabMenuItem::Separator,
            command(TabMenuCommand::SelectNextTab, true),
            command(TabMenuCommand::SelectPreviousTab, true),
            command(TabMenuCommand::ShowListOfTabs, true),
        ]
    );
    assert_eq!(TabMenuCommand::ShowListOfTabs.label(), "Show List of Tabs");

    let single_tab = TabMenuState {
        closeable: false,
        can_close_all: false,
        can_close_others: false,
        can_navigate: false,
    };
    assert_eq!(
        tab_menu_items(&single_tab),
        vec![
            command(TabMenuCommand::CloseAllTabs, false),
            command(TabMenuCommand::CloseOtherTabs, false),
            TabMenuItem::Separator,
            command(TabMenuCommand::SelectNextTab, false),
            command(TabMenuCommand::SelectPreviousTab, false),
        ],
        "IntelliJ's ShowContent is hidden with a single tab"
    );

    let shortcut_of = |command: TabMenuCommand| command.shortcut_action();
    assert!(
        shortcut_of(TabMenuCommand::CloseTab)
            .is_some_and(|action| action.partial_eq(&CloseActiveTab))
    );
    assert!(
        shortcut_of(TabMenuCommand::SelectNextTab)
            .is_some_and(|action| action.partial_eq(&SelectNextTab))
    );
    assert!(
        shortcut_of(TabMenuCommand::SelectPreviousTab)
            .is_some_and(|action| action.partial_eq(&SelectPreviousTab))
    );
    assert!(
        shortcut_of(TabMenuCommand::ShowListOfTabs)
            .is_some_and(|action| action.partial_eq(&ShowListOfTabs))
    );
    assert!(shortcut_of(TabMenuCommand::CloseAllTabs).is_none());
    assert!(shortcut_of(TabMenuCommand::CloseOtherTabs).is_none());
}

#[gpui::test]
fn jetbrains_keymap_shows_the_list_of_tabs_on_ctrl_shift_down(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let bindings = settings::KeymapFile::load_asset_allow_partial_failure(
            "keymaps/macos/jetbrains.json",
            cx,
        )
        .expect("the JetBrains keymap loads");
        let binding = bindings
            .iter()
            .find(|binding| binding.action().partial_eq(&ShowListOfTabs))
            .expect("Show List of Tabs has a binding");
        let keystrokes: Vec<String> = binding
            .keystrokes()
            .iter()
            .map(|keystroke| keystroke.unparse())
            .collect();
        assert_eq!(keystrokes, vec!["ctrl-shift-down".to_string()]);
        assert_eq!(
            binding
                .predicate()
                .map(|predicate| predicate.to_string())
                .as_deref(),
            Some("GitToolWindow")
        );
    });
}

#[gpui::test]
fn the_tab_menu_shows_the_first_intellij_shortcut_of_each_tab_action(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let bindings = settings::KeymapFile::load_asset_allow_partial_failure(
            "keymaps/macos/jetbrains.json",
            cx,
        )
        .expect("the JetBrains keymap loads");
        let shown_shortcut = |action: &dyn gpui::Action| {
            bindings
                .iter()
                .rev()
                .find(|binding| binding.action().partial_eq(action))
                .map(|binding| {
                    binding
                        .keystrokes()
                        .iter()
                        .map(|keystroke| keystroke.unparse())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
        };
        assert_eq!(
            shown_shortcut(&SelectPreviousTab).as_deref(),
            Some("cmd-{"),
            "IntelliJ's PreviousTab lists ⇧⌘[ before ⌃←"
        );
        assert_eq!(
            shown_shortcut(&SelectNextTab).as_deref(),
            Some("cmd-}"),
            "IntelliJ's NextTab lists ⇧⌘] before ⌃→"
        );
        assert_eq!(
            shown_shortcut(&CloseActiveTab).as_deref(),
            Some("ctrl-shift-f4"),
            "IntelliJ's CloseActiveTab is ⌃⇧F4; ⌘W is CloseContent"
        );
    });
}

#[test]
fn tab_strip_drops_non_selected_tabs_from_the_edges_and_squeezes_the_last_one() {
    let widths = [px(100.), px(100.), px(100.)];
    let layout = layout_tab_strip(&widths, Some(1), Some(px(400.)), px(44.));
    assert_eq!(layout.slots, vec![TabSlot::Full; 3]);
    assert!(!layout.has_dropped_tabs);

    let layout = layout_tab_strip(&widths, Some(1), Some(px(243.)), px(44.));
    assert_eq!(
        layout.slots,
        vec![TabSlot::Hidden, TabSlot::Full, TabSlot::Squeezed(px(99.))],
        "the first non-selected tab is dropped, the last visible one is squeezed to the remaining width"
    );
    assert!(layout.has_dropped_tabs);
    assert_eq!(layout.hidden_indices(), vec![0]);

    let layout = layout_tab_strip(&widths, Some(0), None, px(44.));
    assert_eq!(
        layout.slots,
        vec![TabSlot::Full; 3],
        "unmeasured strips show every tab"
    );
}

#[test]
fn tab_metrics_follow_content_label_and_islands_painter() {
    assert_eq!(tab_width(px(40.), false), px(64.));
    assert_eq!(tab_width(px(40.), true), px(83.));
    assert_eq!(tab_width(px(1000.), false), px(400.));
    assert_eq!(pill_vertical_geometry(px(40.)), (px(6.), px(28.)));
    assert_eq!(pill_vertical_geometry(px(32.)), (px(4.), px(24.)));

    let colors = theme::ThemeColors::dark();
    let focused_pill = ui::island_tab_pill_colors(true, &colors);
    let active = tab_decoration(true, true, true, &colors);
    assert_eq!(active.fill, Some(focused_pill.background));
    assert_eq!(active.border, Some(focused_pill.border));
    assert_eq!(active.hover_fill, None);
    let inactive = tab_decoration(true, false, true, &colors);
    assert_eq!(
        inactive.fill,
        Some(ui::island_tab_pill_colors(false, &colors).background)
    );
    assert!(inactive.hover_fill.is_some());
    let unselected = tab_decoration(false, true, true, &colors);
    assert_eq!(unselected.fill, None);
    assert_eq!(unselected.hover_fill, Some(colors.ghost_element_hover));
    let classic = tab_decoration(true, false, false, &colors);
    assert_eq!(classic.underline, Some(colors.border));
}

#[test]
fn restore_keeps_filtered_tabs_and_only_one_filterless_tab() {
    let saved = [
        PersistedLogTab {
            id: "first".to_string(),
            has_filters: false,
        },
        PersistedLogTab {
            id: "second".to_string(),
            has_filters: true,
        },
        PersistedLogTab {
            id: "third".to_string(),
            has_filters: false,
        },
    ];
    let plan = restore_plan(&saved);
    assert_eq!(
        plan.reopened,
        vec!["first".to_string(), "second".to_string()]
    );
    assert_eq!(plan.dropped, vec!["third".to_string()]);
}

#[test]
fn new_tab_ids_are_random_version_four_uuids() {
    let id = uuid_from_random(u128::MAX);
    assert_eq!(id.len(), 36);
    assert_eq!(id, "ffffffff-ffff-4fff-bfff-ffffffffffff");
    assert_eq!(uuid_from_random(0), "00000000-0000-4000-8000-000000000000");
    let generated = persistence::new_log_tab_id(|candidate| candidate == MAIN_LOG_TAB_ID);
    assert_eq!(generated.chars().nth(14), Some('4'));
}

#[test]
fn console_document_prints_chunks_and_a_newline_in_the_last_chunk_type() {
    let line = GitConsoleLine::new(vec![
        GitConsoleChunk {
            text: "fatal: ".into(),
            content_type: GitConsoleContentType::ErrorOutput,
        },
        GitConsoleChunk {
            text: "out".into(),
            content_type: GitConsoleContentType::NormalOutput,
        },
    ])
    .expect("non-blank line");
    let document = console_document(&[line]);
    assert_eq!(document.text, "fatal: out\n");
    assert_eq!(document.ranges[1], vec![0..7]);
    assert_eq!(document.ranges[0], vec![7..11]);
}

#[test]
fn console_keeps_the_overlap_when_the_oldest_lines_are_dropped() {
    let first = console_line("first", GitConsoleContentType::SystemOutput);
    let second = console_line("second", GitConsoleContentType::SystemOutput);
    let third = console_line("third", GitConsoleContentType::ErrorOutput);
    let shown = [first.clone(), second.clone()];
    assert_eq!(kept_lines_start(&shown, &[first, second.clone()]), 0);
    assert_eq!(kept_lines_start(&shown, &[second, third.clone()]), 1);
    assert_eq!(kept_lines_start(&shown, &[third]), 2);
}

#[gpui::test]
async fn console_tab_appears_unselected_on_the_first_line_and_reappears_after_closing(
    cx: &mut TestAppContext,
) {
    let (fs, tool_window, mut cx) = tool_window_for_repository(cx).await;
    assert_eq!(tab_keys(&tool_window, &cx), vec![main_key()]);

    push_console_line(
        &fs,
        "12:00:00.000: [/project] git fetch",
        GitConsoleContentType::SystemOutput,
    );
    cx.run_until_parked();
    assert_eq!(
        tab_keys(&tool_window, &cx),
        vec![main_key(), ContentTabKey::Console]
    );
    assert_eq!(
        selected_key(&tool_window, &cx),
        Some(main_key()),
        "ShowConsoleSignal(focusLatest = false) must not select the console"
    );
    let text = tool_window.read_with(&cx, |tool_window, cx| {
        tool_window
            .console
            .as_ref()
            .map(|console| console.read(cx).text(cx))
    });
    assert_eq!(
        text.as_deref(),
        Some("12:00:00.000: [/project] git fetch\n")
    );

    tool_window.update_in(&mut cx, |tool_window, window, cx| {
        tool_window.close_tab(&ContentTabKey::Console, window, cx)
    });
    assert_eq!(tab_keys(&tool_window, &cx), vec![main_key()]);
    let show_console = tool_window.read_with(&cx, |tool_window, cx| {
        tool_window.dropdown_state(cx).show_console_tab
    });
    assert!(
        show_console,
        "a hidden console with messages offers Show VCS Console Tab"
    );

    push_console_line(&fs, "error: failed", GitConsoleContentType::ErrorOutput);
    cx.run_until_parked();
    assert_eq!(
        tab_keys(&tool_window, &cx),
        vec![main_key(), ContentTabKey::Console]
    );
    assert_eq!(selected_key(&tool_window, &cx), Some(main_key()));

    tool_window.update_in(&mut cx, |tool_window, window, cx| {
        tool_window.close_tab(&ContentTabKey::Console, window, cx);
        tool_window.show_console_tab(window, cx);
    });
    assert_eq!(
        selected_key(&tool_window, &cx),
        Some(ContentTabKey::Console)
    );
    let show_console = tool_window.read_with(&cx, |tool_window, cx| {
        tool_window.dropdown_state(cx).show_console_tab
    });
    assert!(
        !show_console,
        "a visible console hides Show VCS Console Tab"
    );
}

#[gpui::test]
async fn extra_log_tabs_get_unique_ids_and_close_back_to_the_previous_tab(cx: &mut TestAppContext) {
    let (_fs, tool_window, mut cx) = tool_window_for_repository(cx).await;
    tool_window.update_in(&mut cx, |tool_window, window, cx| {
        tool_window.open_another_log_tab(window, cx);
        tool_window.open_another_log_tab(window, cx);
    });
    let keys = tab_keys(&tool_window, &cx);
    assert_eq!(keys.len(), 3);
    assert_eq!(keys.first(), Some(&main_key()));
    let second = keys.get(1).cloned().expect("second tab");
    let third = keys.get(2).cloned().expect("third tab");
    assert_ne!(second, third);
    assert_eq!(selected_key(&tool_window, &cx), Some(third.clone()));
    if let ContentTabKey::Log(id) = &second {
        assert_eq!(
            id.len(),
            36,
            "tab ids are UUID strings like VcsLogManager.generateNewLogId"
        );
    }

    tool_window.update_in(&mut cx, |tool_window, window, cx| {
        tool_window.close_tab(&main_key(), window, cx);
        tool_window.close_tab(&third, window, cx);
    });
    assert_eq!(
        tab_keys(&tool_window, &cx),
        vec![main_key(), second.clone()]
    );
    assert_eq!(selected_key(&tool_window, &cx), Some(second.clone()));

    let persisted = tool_window.read_with(&cx, |tool_window, cx| tool_window.persisted_state(cx));
    let ContentTabKey::Log(second_id) = second else {
        panic!("extra tabs are log tabs");
    };
    assert_eq!(
        persisted.log_tabs,
        vec![PersistedLogTab {
            id: second_id.to_string(),
            has_filters: false,
        }]
    );
    assert_eq!(persisted.selected_tab, Some(second_id.to_string()));
}

#[gpui::test]
async fn selected_tab_survives_a_persistence_round_trip(cx: &mut TestAppContext) {
    let (_fs, tool_window, mut cx) = tool_window_for_repository(cx).await;
    let state = PersistedTabs {
        log_tabs: vec![
            PersistedLogTab {
                id: "restored".to_string(),
                has_filters: true,
            },
            PersistedLogTab {
                id: "plain".to_string(),
                has_filters: false,
            },
            PersistedLogTab {
                id: "dropped".to_string(),
                has_filters: false,
            },
        ],
        selected_tab: Some("restored".to_string()),
    };
    let key = "git-tool-window-test".to_string();
    cx.update(|_, cx| persistence::save_tabs(key.clone(), state.clone(), cx))
        .await;
    let loaded = cx
        .update(|_, cx| persistence::load_tabs(key, cx))
        .await
        .expect("the saved tabs are read back");
    assert_eq!(loaded, state);

    tool_window.update_in(&mut cx, |tool_window, window, cx| {
        tool_window.apply_restored_state(loaded, window, cx)
    });
    assert_eq!(
        tab_keys(&tool_window, &cx),
        vec![main_key(), log_key("restored"), log_key("plain")]
    );
    assert_eq!(selected_key(&tool_window, &cx), Some(log_key("restored")));
}

#[gpui::test]
async fn header_tab_titles_follow_log_tab_titles(cx: &mut TestAppContext) {
    let (_fs, tool_window, mut cx) = tool_window_for_repository(cx).await;
    let main_tab = tool_window.read_with(&cx, |tool_window, _| {
        tool_window
            .log_tab(MAIN_LOG_TAB_ID)
            .cloned()
            .expect("the main log tab exists")
    });
    main_tab.update_in(&mut cx, |tab, window, cx| {
        tab.set_filters(
            LogFilterState {
                branches: vec!["HEAD".to_string()],
                ..LogFilterState::default()
            },
            window,
            cx,
        )
    });
    cx.run_until_parked();
    let (log_title, header_title) = tool_window.update_in(&mut cx, |tool_window, window, cx| {
        let log_title = main_tab.read(cx).title();
        let header_title = tool_window
            .header_snapshot(window, cx)
            .tabs
            .first()
            .map(|tab| tab.title.clone());
        (log_title, header_title)
    });
    assert_eq!(header_title, Some(log_title.clone()));
    assert_eq!(log_title.as_ref(), "Log: HEAD");
}

#[gpui::test]
async fn show_list_of_tabs_opens_the_tab_list_only_when_there_are_several_tabs(
    cx: &mut TestAppContext,
) {
    let (_fs, tool_window, mut cx) = tool_window_for_repository(cx).await;
    let header = tool_window.read_with(&cx, |tool_window, _| tool_window.header.clone());
    tool_window.update_in(&mut cx, |tool_window, window, cx| {
        tool_window.show_list_of_tabs(window, cx)
    });
    assert!(
        !header.read_with(&cx, |header, _| header.has_open_menu()),
        "a single tab has no list"
    );

    tool_window.update_in(&mut cx, |tool_window, window, cx| {
        tool_window.open_another_log_tab(window, cx);
        tool_window.run_tab_menu_command(TabMenuCommand::ShowListOfTabs, &main_key(), window, cx)
    });
    assert!(header.read_with(&cx, |header, _| header.has_open_menu()));

    tool_window.update_in(&mut cx, |tool_window, window, cx| {
        tool_window.run_tab_menu_command(TabMenuCommand::SelectNextTab, &main_key(), window, cx)
    });
    assert_eq!(selected_key(&tool_window, &cx), Some(main_key()));
}
