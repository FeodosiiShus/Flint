use std::{cell::RefCell, rc::Rc, sync::Arc};

use editor::Editor;
use fs::Fs;
use gpui::{Entity, TestAppContext, VisualTestContext, px};
use language::{Point, rust_lang};
use project::{FakeFs, Project, ProjectEntryId, ProjectPath, WorktreeId};
use serde_json::json;
use settings::{SettingsContent, SettingsStore};
use ui::SharedString;
use util::{path, rel_path::rel_path};
use workspace::{AppState, MultiWorkspace, StatusItemView, Workspace};

use crate::{
    INDENTATION_OPTIONS, Indentation, IndentationIndicator, IndentationOption, IndentationSelector,
    NavigationBar, NavigationTarget, ReadOnlyIndicator, ReadOnlyState, build_segments,
    hidden_middle_range,
};

fn init_test(cx: &mut TestAppContext) -> Arc<AppState> {
    cx.update(|cx| {
        let app_state = AppState::test(cx);
        editor::init(cx);
        crate::init(cx);
        app_state
    })
}

fn entry_target(worktree_id: WorktreeId, path: &str) -> NavigationTarget {
    NavigationTarget::Entry(ProjectPath {
        worktree_id,
        path: rel_path(path).into_arc(),
    })
}

fn labels(segments: &[crate::NavigationSegment]) -> Vec<&str> {
    segments
        .iter()
        .map(|segment| segment.label.as_ref())
        .collect()
}

async fn open_workspace_with_rust_file(
    fs: Arc<FakeFs>,
    cx: &mut TestAppContext,
) -> (Entity<Workspace>, Entity<Editor>, &mut VisualTestContext) {
    fs.insert_tree(
        path!("/root"),
        json!({
            "src": {
                "main.rs": "fn main() {\n    let value = 1;\n}\n",
            },
        }),
    )
    .await;
    let project = Project::test(fs, [path!("/root").as_ref()], cx).await;
    project.read_with(cx, |project, _| project.languages().add(rust_lang()));
    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
    let workspace =
        multi_workspace.read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone());
    let worktree_id = project.read_with(cx, |project, cx| {
        project
            .worktrees(cx)
            .next()
            .expect("the project has a worktree")
            .read(cx)
            .id()
    });
    let editor = workspace
        .update_in(cx, |workspace, window, cx| {
            workspace.open_path(
                (worktree_id, rel_path("src/main.rs")),
                None,
                true,
                window,
                cx,
            )
        })
        .await
        .expect("the file opens")
        .downcast::<Editor>()
        .expect("the item is an editor");
    cx.run_until_parked();
    (workspace, editor, cx)
}

#[test]
fn test_segments_list_project_directories_file_and_symbols() {
    let worktree_id = WorktreeId::from_usize(1);
    let project_path = ProjectPath {
        worktree_id,
        path: rel_path("src/main/File.rs").into_arc(),
    };
    let segments = build_segments(
        Some((&project_path, "project")),
        [SharedString::from("impl Foo"), SharedString::from("fn bar")],
    );

    assert_eq!(
        labels(&segments),
        vec!["project", "src", "main", "File.rs", "impl Foo", "fn bar"]
    );
    let targets = segments
        .iter()
        .map(|segment| segment.target.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        targets,
        vec![
            entry_target(worktree_id, ""),
            entry_target(worktree_id, "src"),
            entry_target(worktree_id, "src/main"),
            entry_target(worktree_id, "src/main/File.rs"),
            NavigationTarget::Symbol(0),
            NavigationTarget::Symbol(1),
        ],
        "the project segment reveals the worktree root and symbols keep their outline index"
    );
}

#[test]
fn test_segments_for_single_file_worktree_and_items_without_paths() {
    let worktree_id = WorktreeId::from_usize(7);
    let single_file = ProjectPath {
        worktree_id,
        path: rel_path("").into_arc(),
    };
    let segments = build_segments(Some((&single_file, "notes.md")), Vec::new());
    assert_eq!(labels(&segments), vec!["notes.md"]);
    assert_eq!(segments[0].target, entry_target(worktree_id, ""));

    let symbols_only = build_segments(None, [SharedString::from("fn untitled")]);
    assert_eq!(labels(&symbols_only), vec!["fn untitled"]);
    assert_eq!(symbols_only[0].target, NavigationTarget::Symbol(0));

    assert!(build_segments(None, Vec::new()).is_empty());
}

#[test]
fn test_hidden_middle_range_keeps_all_segments_when_they_fit() {
    let widths = [px(40.), px(30.), px(50.)];
    assert_eq!(
        hidden_middle_range(&widths, px(10.), px(10.), px(140.)),
        None
    );
    assert_eq!(
        hidden_middle_range(&widths, px(10.), px(10.), px(139.)),
        Some(1..2),
        "one pixel short of the full row hides the middle segment"
    );
}

#[test]
fn test_hidden_middle_range_truncates_from_the_middle() {
    let widths = [px(50.); 6];
    let separator = px(10.);
    let marker = px(10.);

    assert_eq!(
        hidden_middle_range(&widths, separator, marker, px(350.)),
        None
    );
    assert_eq!(
        hidden_middle_range(&widths, separator, marker, px(349.)),
        Some(3..4),
        "five segments plus the marker take 5 * 50 + 10 + 5 * 10 = 310 px"
    );
    assert_eq!(
        hidden_middle_range(&widths, separator, marker, px(300.)),
        Some(2..4),
        "four segments plus the marker take 4 * 50 + 10 + 4 * 10 = 250 px"
    );
    assert_eq!(
        hidden_middle_range(&widths, separator, marker, px(10.)),
        Some(1..5),
        "the first and the last segment always stay visible"
    );
}

#[test]
fn test_hidden_middle_range_never_hides_one_of_two_segments() {
    assert_eq!(
        hidden_middle_range(&[px(500.), px(500.)], px(10.), px(10.), px(100.)),
        None
    );
    assert_eq!(hidden_middle_range(&[], px(10.), px(10.), px(0.)), None);
}

#[test]
fn test_indentation_labels() {
    let label = |tab_size, hard_tabs| {
        Indentation {
            tab_size,
            hard_tabs,
        }
        .label()
    };
    assert_eq!(label(4, false), "4 spaces");
    assert_eq!(label(2, false), "2 spaces");
    assert_eq!(label(1, false), "1 space");
    assert_eq!(label(8, true), "Tab", "hard tabs ignore the tab size");

    let option_labels = INDENTATION_OPTIONS
        .iter()
        .map(|option| option.label())
        .collect::<Vec<_>>();
    assert_eq!(
        option_labels,
        vec!["2 spaces", "4 spaces", "8 spaces", "Tab"]
    );
}

#[test]
fn test_indentation_option_matches_current_indentation() {
    let four_spaces = Indentation {
        tab_size: 4,
        hard_tabs: false,
    };
    let tabs = Indentation {
        tab_size: 4,
        hard_tabs: true,
    };
    assert!(IndentationOption::Spaces(4).matches(four_spaces));
    assert!(!IndentationOption::Spaces(2).matches(four_spaces));
    assert!(!IndentationOption::Tab.matches(four_spaces));
    assert!(IndentationOption::Tab.matches(tabs));
    assert!(
        !IndentationOption::Spaces(4).matches(tabs),
        "spaces never match an indentation that uses hard tabs"
    );
}

#[test]
fn test_indentation_option_writes_language_or_default_settings() {
    let mut settings = SettingsContent::default();
    IndentationOption::Spaces(2).apply(&mut settings, Some("Rust"));
    let rust = settings
        .languages_mut()
        .get("Rust")
        .cloned()
        .expect("the Rust language entry is created");
    assert_eq!(rust.tab_size.map(|tab_size| tab_size.get()), Some(2));
    assert_eq!(rust.hard_tabs, Some(false));
    assert_eq!(settings.project.all_languages.defaults.tab_size, None);

    IndentationOption::Tab.apply(&mut settings, Some("Rust"));
    let rust = settings
        .languages_mut()
        .get("Rust")
        .cloned()
        .expect("the Rust language entry is kept");
    assert_eq!(rust.hard_tabs, Some(true));
    assert_eq!(
        rust.tab_size.map(|tab_size| tab_size.get()),
        Some(2),
        "switching to tabs keeps the configured tab width"
    );

    IndentationOption::Tab.apply(&mut settings, None);
    assert_eq!(
        settings.project.all_languages.defaults.hard_tabs,
        Some(true)
    );
    assert_eq!(settings.project.all_languages.defaults.tab_size, None);
}

#[test]
fn test_read_only_tooltips() {
    let state = |read_only, toggleable| ReadOnlyState {
        read_only,
        toggleable,
    };
    assert_eq!(
        state(true, true).tooltip(),
        "File is read-only. Click to make it writable"
    );
    assert_eq!(state(true, false).tooltip(), "File is read-only");
    assert_eq!(
        state(false, true).tooltip(),
        "File is writable. Click to make it read-only"
    );
}

#[gpui::test]
async fn test_indentation_selector_writes_user_settings(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    let settings_directory = paths::settings_file()
        .parent()
        .expect("the settings file has a parent directory");
    fs.create_dir(settings_directory)
        .await
        .expect("the settings directory is created");
    fs.insert_file(paths::settings_file(), b"{}".to_vec()).await;
    let (workspace, editor, cx) = open_workspace_with_rust_file(fs.clone(), cx).await;

    let indicator = workspace.update_in(cx, |workspace, window, cx| {
        let indicator = cx.new(IndentationIndicator::new);
        workspace.status_bar().update(cx, |status_bar, cx| {
            status_bar.add_right_item(indicator.clone(), window, cx);
        });
        indicator
    });
    cx.run_until_parked();
    let initial = indicator.read_with(cx, |indicator, cx| indicator.indentation(cx));
    assert_eq!(
        initial.map(|indentation| indentation.hard_tabs),
        Some(false),
        "the default settings indent with spaces"
    );

    cx.update(|window, cx| IndentationSelector::toggle(&editor.downgrade(), window, cx));
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, cx| {
        assert!(
            workspace.active_modal::<IndentationSelector>(cx).is_some(),
            "the indentation picker is open"
        );
    });

    cx.dispatch_action(menu::SelectLast);
    cx.dispatch_action(menu::Confirm);
    cx.run_until_parked();

    workspace.read_with(cx, |workspace, cx| {
        assert!(
            workspace.active_modal::<IndentationSelector>(cx).is_none(),
            "confirming closes the picker"
        );
    });
    let settings_text = fs
        .load(paths::settings_file())
        .await
        .expect("the settings file is readable");
    let settings_json = settings::parse_json_with_comments::<serde_json::Value>(&settings_text)
        .expect("the settings file is valid JSON");
    let language_name = rust_lang().name().0.to_string();
    assert_eq!(
        settings_json["languages"][language_name.as_str()]["hard_tabs"],
        json!(true),
        "the Tab option is stored for the buffer's language: {settings_text}"
    );
    indicator.read_with(cx, |indicator, cx| {
        assert_eq!(
            indicator
                .indentation(cx)
                .map(|indentation| indentation.label()),
            Some(SharedString::from("Tab")),
            "the widget reflects the new setting"
        );
    });
}

#[gpui::test]
async fn test_read_only_indicator_toggles_and_hides(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    let (workspace, editor, cx) = open_workspace_with_rust_file(fs, cx).await;

    let indicator = workspace.update_in(cx, |workspace, window, cx| {
        let indicator = cx.new(|_| ReadOnlyIndicator::default());
        workspace.status_bar().update(cx, |status_bar, cx| {
            status_bar.add_right_item(indicator.clone(), window, cx);
        });
        indicator
    });
    cx.run_until_parked();
    indicator.read_with(cx, |indicator, cx| {
        assert_eq!(
            indicator.state(),
            Some(ReadOnlyState {
                read_only: false,
                toggleable: true,
            })
        );
        assert!(indicator.is_visible(cx));
    });

    indicator.update_in(cx, |indicator, window, cx| indicator.toggle(window, cx));
    cx.run_until_parked();
    assert!(editor.read_with(cx, |editor, cx| editor.read_only(cx)));
    assert_eq!(
        indicator.read_with(cx, |indicator, _| indicator.state()),
        Some(ReadOnlyState {
            read_only: true,
            toggleable: true,
        }),
        "the lock follows the buffer capability"
    );

    indicator.update_in(cx, |indicator, window, cx| indicator.toggle(window, cx));
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |editor, cx| editor.read_only(cx)));

    editor.update(cx, |editor, cx| {
        editor.set_read_only(true);
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(
        indicator.read_with(cx, |indicator, _| indicator.state()),
        Some(ReadOnlyState {
            read_only: true,
            toggleable: false,
        }),
        "an editor that is read-only by construction cannot be toggled"
    );
    indicator.update_in(cx, |indicator, window, cx| indicator.toggle(window, cx));
    cx.run_until_parked();
    assert!(editor.read_with(cx, |editor, cx| editor.read_only(cx)));

    cx.update(|_, cx| {
        cx.update_global::<SettingsStore, _>(|store, cx| {
            store.update_user_settings(cx, |settings| {
                settings.status_bar.get_or_insert_default().read_only_button = Some(false);
            });
        });
    });
    indicator.read_with(cx, |indicator, cx| {
        assert!(
            !indicator.is_visible(cx),
            "the setting hides the widget even when an editor is active"
        );
    });

    indicator.update_in(cx, |indicator, window, cx| {
        indicator.set_active_pane_item(None, window, cx)
    });
    cx.run_until_parked();
    assert_eq!(
        indicator.read_with(cx, |indicator, _| indicator.state()),
        None,
        "without an active editor there is nothing to lock"
    );
}

#[gpui::test]
async fn test_navigation_bar_reveals_entries_and_jumps_to_symbols(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    let (workspace, editor, cx) = open_workspace_with_rust_file(fs, cx).await;
    let project = workspace.read_with(cx, |workspace, _| workspace.project().clone());

    let navigation_bar = workspace.update_in(cx, |workspace, window, cx| {
        let navigation_bar = cx.new(|_| NavigationBar::new(workspace));
        workspace.status_bar().update(cx, |status_bar, cx| {
            status_bar.add_left_item(navigation_bar.clone(), window, cx);
        });
        navigation_bar
    });
    editor.update_in(cx, |editor, window, cx| {
        editor.change_selections(Default::default(), window, cx, |selections| {
            selections.select_ranges([Point::new(1, 8)..Point::new(1, 8)])
        });
    });
    cx.run_until_parked();

    let segments =
        navigation_bar.read_with(cx, |navigation_bar, _| navigation_bar.segments().to_vec());
    assert_eq!(labels(&segments[..3]), vec!["root", "src", "main.rs"]);
    let symbol = segments
        .get(3)
        .expect("the function around the cursor is a segment");
    assert_eq!(symbol.target, NavigationTarget::Symbol(0));
    assert!(
        symbol.label.contains("main"),
        "the symbol segment names the enclosing function, got {:?}",
        symbol.label
    );

    let revealed: Rc<RefCell<Vec<ProjectEntryId>>> = Rc::default();
    cx.update(|_, cx| {
        let revealed = revealed.clone();
        cx.subscribe(&project, move |_, event, _| {
            if let project::Event::RevealInProjectPanel(entry_id) = event {
                revealed.borrow_mut().push(*entry_id);
            }
        })
        .detach();
    });
    let directory_target = segments[1].target.clone();
    navigation_bar.update_in(cx, |navigation_bar, window, cx| {
        navigation_bar.navigate(&directory_target, window, cx)
    });
    cx.run_until_parked();
    let directory_entry = project.read_with(cx, |project, cx| {
        project
            .worktrees(cx)
            .next()
            .expect("the project has a worktree")
            .read(cx)
            .entry_for_path(rel_path("src"))
            .expect("the directory is in the worktree")
            .id
    });
    assert_eq!(*revealed.borrow(), vec![directory_entry]);

    navigation_bar.update_in(cx, |navigation_bar, window, cx| {
        navigation_bar.navigate(&NavigationTarget::Symbol(0), window, cx)
    });
    cx.run_until_parked();
    let cursor = editor.update(cx, |editor, cx| {
        editor
            .selections
            .newest::<Point>(&editor.display_snapshot(cx))
            .head()
    });
    assert_eq!(
        cursor.row, 0,
        "clicking the symbol moves the cursor to its declaration"
    );
}
