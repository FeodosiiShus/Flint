use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use git::repository::{RepositoryOperation, UpstreamTracking, UpstreamTrackingStatus};
use gpui::{AppContext as _, TestAppContext, VisualTestContext, WindowHandle};
use project::git_store::RepositoryEvent;
use project::{FakeFs, Project};
use serde_json::json;
use settings::SettingsStore;
use util::path;
use workspace::MultiWorkspace;

use super::delegate::PopupToggle;
use super::rows::{PopupRow, summarize};
use super::tree::RefRow;
use super::*;
use crate::branch_operations::integrate;

fn init_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
        cx.set_global(db::AppDatabase::test_new());
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        editor::init(cx);
    });
}

async fn create_repository(
    fs: &FakeFs,
    root: &str,
    configure: impl FnOnce(&FakeFs, &Path),
) -> PathBuf {
    fs.insert_tree(root, json!({ ".git": {}, "file.txt": "content" }))
        .await;
    let dot_git = Path::new(root).join(".git");
    fs.set_head_for_repo(&dot_git, &[("file.txt", "content".to_string())], "deadbeef");
    configure(fs, &dot_git);
    dot_git
}

fn standard_repository(fs: &FakeFs, dot_git: &Path) {
    fs.insert_branches(
        dot_git,
        &["main", "topic", "refs/heads/feature/login", "origin/main"],
    );
    fs.set_upstream_for_repo(
        dot_git,
        "main",
        "origin/main",
        UpstreamTracking::Tracked(UpstreamTrackingStatus {
            ahead: 1,
            behind: 2,
        }),
    );
    fs.insert_tags(dot_git, &[("v1.0", "aaa111")]);
    fs.set_recent_branches_for_repo(dot_git, &["main", "topic"]);
    fs.set_remote_for_repo(dot_git, "origin", "https://example.com/repo.git");
}

struct Fixture {
    fs: Arc<FakeFs>,
    dot_git: PathBuf,
    window: WindowHandle<MultiWorkspace>,
    workspace: Entity<Workspace>,
    repository: Entity<Repository>,
    popup: Entity<BranchesPopup>,
    cx: VisualTestContext,
}

impl Fixture {
    async fn open(
        cx: &mut TestAppContext,
        root: &str,
        configure: impl FnOnce(&FakeFs, &Path),
    ) -> Self {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        let dot_git = create_repository(&fs, root, configure).await;
        let project = Project::test(fs.clone(), [Path::new(root)], cx).await;
        cx.run_until_parked();
        let repository = project
            .read_with(cx, |project, cx| project.active_repository(cx))
            .expect("the project has an active repository");
        let window =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = window
            .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
            .expect("window is alive");
        let mut visual_cx = VisualTestContext::from_window(window.into(), cx);
        let popup = Self::create_popup(window, &workspace, &repository, &mut visual_cx);
        visual_cx.run_until_parked();
        Self {
            fs,
            dot_git,
            window,
            workspace,
            repository,
            popup,
            cx: visual_cx,
        }
    }

    fn create_popup(
        window: WindowHandle<MultiWorkspace>,
        workspace: &Entity<Workspace>,
        repository: &Entity<Repository>,
        cx: &mut VisualTestContext,
    ) -> Entity<BranchesPopup> {
        let workspace = workspace.downgrade();
        let repository = repository.clone();
        window
            .update(cx, |_, window, cx| {
                cx.new(|cx| {
                    BranchesPopup::new(
                        workspace,
                        Some(repository),
                        BranchesPopupStyle::Modal,
                        window,
                        cx,
                    )
                })
            })
            .expect("window is alive")
    }

    fn reopen(&mut self) {
        self.popup =
            Self::create_popup(self.window, &self.workspace, &self.repository, &mut self.cx);
        self.cx.run_until_parked();
    }

    fn picker(&self) -> Entity<Picker<BranchesDelegate>> {
        self.popup
            .read_with(&self.cx, |popup, _| popup.picker.clone())
    }

    fn update_picker<R>(
        &mut self,
        update: impl FnOnce(
            &mut Picker<BranchesDelegate>,
            &mut Window,
            &mut Context<Picker<BranchesDelegate>>,
        ) -> R,
    ) -> R {
        let picker = self.picker();
        let result = picker.update_in(&mut self.cx, update);
        self.cx.run_until_parked();
        result
    }

    fn rows(&self) -> Vec<String> {
        let picker = self.picker();
        picker.read_with(&self.cx, |picker, _| summarize(picker.delegate.rows()))
    }

    fn selected(&self) -> Option<String> {
        let picker = self.picker();
        picker.read_with(&self.cx, |picker, _| {
            picker
                .delegate
                .selected_row()
                .and_then(|row| summarize(std::slice::from_ref(row)).into_iter().next())
        })
    }

    fn index_of(&self, line: &str) -> usize {
        self.rows()
            .iter()
            .position(|row| row == line)
            .unwrap_or_else(|| panic!("row {line:?} not found in {:?}", self.rows()))
    }

    fn select(&mut self, line: &str) {
        let index = self.index_of(line);
        self.update_picker(|picker, window, cx| {
            picker.set_selected_index(index, None, true, window, cx);
        });
        assert_eq!(self.selected().as_deref(), Some(line));
    }

    fn select_child(&mut self) {
        self.update_picker(|picker, window, cx| {
            picker.delegate.select_child(window, cx);
        });
    }

    fn select_parent(&mut self) {
        self.update_picker(|picker, window, cx| {
            picker.delegate.select_parent(window, cx);
        });
    }

    fn set_query(&mut self, query: &str) {
        self.update_picker(|picker, window, cx| picker.set_query(query, window, cx));
    }

    fn ref_row(&self, name: &str) -> RefRow {
        let picker = self.picker();
        picker.read_with(&self.cx, |picker, _| {
            picker
                .delegate
                .rows()
                .iter()
                .find_map(|row| match row {
                    PopupRow::Ref(entry) if &*entry.row.target.name == name => {
                        Some(entry.row.clone())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("ref row {name:?} not found"))
        })
    }
}

#[gpui::test]
async fn opening_the_popup_lists_actions_recent_and_collapsed_sections(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx, path!("/branches-popup-open"), standard_repository).await;

    assert_eq!(
        fixture.rows(),
        vec![
            "action:Update Project…",
            "action:Commit…",
            "action:Push…",
            "---",
            "action:New Branch…",
            "action:Checkout Tag or Revision…",
            "---",
            "section:Recent:open",
            "  ref:main",
            "  ref:topic",
            "section:Local:closed",
            "section:Remote:closed",
            "section:Tags:closed",
        ]
    );
    assert_eq!(fixture.selected().as_deref(), Some("  ref:main"));
}

#[gpui::test]
async fn ref_rows_carry_tracking_information_and_remote_names_are_loaded(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx, path!("/branches-popup-tracking"), standard_repository).await;

    let main = fixture.ref_row("main");
    assert!(main.is_current);
    assert_eq!(main.tracked.as_deref(), Some("origin/main"));
    assert_eq!(main.incoming, 2);
    assert_eq!(main.outgoing, 1);

    let picker = fixture.picker();
    picker.read_with(&fixture.cx, |picker, _| {
        let remote_names: Vec<&str> = picker
            .delegate
            .data
            .remote_names
            .iter()
            .map(|name| &**name)
            .collect();
        assert_eq!(remote_names, vec!["origin"]);
        assert_eq!(picker.delegate.data.tags.len(), 1);
    });
}

#[gpui::test]
async fn select_child_and_select_parent_walk_the_tree(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-walk"), |fs, dot_git| {
        fs.insert_branches(dot_git, &["main", "topic", "refs/heads/feature/login"]);
        fs.set_recent_branches_for_repo(dot_git, &["main"]);
    })
    .await;

    fixture.select("section:Local:closed");
    fixture.select_child();
    assert!(fixture.rows().contains(&"section:Local:open".to_string()));
    assert!(
        fixture
            .rows()
            .contains(&"  folder:feature:closed".to_string())
    );
    assert!(fixture.rows().contains(&"  ref:topic".to_string()));
    assert_eq!(fixture.selected().as_deref(), Some("section:Local:open"));

    fixture.select_child();
    assert_eq!(
        fixture.selected().as_deref(),
        Some("  folder:feature:closed")
    );

    fixture.select_child();
    assert!(fixture.rows().contains(&"    ref:login".to_string()));
    assert_eq!(fixture.selected().as_deref(), Some("  folder:feature:open"));

    fixture.select_child();
    assert_eq!(fixture.selected().as_deref(), Some("    ref:login"));

    fixture.select_parent();
    assert_eq!(fixture.selected().as_deref(), Some("  folder:feature:open"));

    fixture.select_parent();
    assert!(!fixture.rows().contains(&"    ref:login".to_string()));
    assert_eq!(
        fixture.selected().as_deref(),
        Some("  folder:feature:closed")
    );

    fixture.select_parent();
    assert_eq!(fixture.selected().as_deref(), Some("section:Local:open"));

    fixture.select_parent();
    assert!(fixture.rows().contains(&"section:Local:closed".to_string()));
    assert_eq!(fixture.selected().as_deref(), Some("section:Local:closed"));
}

#[gpui::test]
async fn confirming_a_section_row_toggles_its_expansion(cx: &mut TestAppContext) {
    let mut fixture =
        Fixture::open(cx, path!("/branches-popup-confirm"), standard_repository).await;

    fixture.select("section:Remote:closed");
    fixture.update_picker(|picker, window, cx| picker.delegate.confirm(false, window, cx));
    assert!(fixture.rows().contains(&"section:Remote:open".to_string()));
    assert!(
        fixture
            .rows()
            .contains(&"  folder:origin:closed".to_string())
    );

    fixture.update_picker(|picker, window, cx| picker.delegate.confirm(false, window, cx));
    assert!(
        fixture
            .rows()
            .contains(&"section:Remote:closed".to_string())
    );
    assert!(
        !fixture
            .rows()
            .contains(&"  folder:origin:closed".to_string())
    );
}

#[gpui::test]
async fn typing_a_query_filters_rows_and_selects_the_top_match(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-query"), standard_repository).await;

    fixture.set_query("topic");
    assert_eq!(fixture.rows(), vec!["section:Recent:open", "  ref:topic"]);
    assert_eq!(fixture.selected().as_deref(), Some("  ref:topic"));

    fixture.set_query("new");
    assert_eq!(fixture.rows(), vec!["action:New Branch…"]);
    assert_eq!(fixture.selected().as_deref(), Some("action:New Branch…"));

    fixture.set_query("login");
    assert_eq!(
        fixture.rows(),
        vec![
            "section:Local:open",
            "  folder:feature:open",
            "    ref:login"
        ]
    );
    assert_eq!(fixture.selected().as_deref(), Some("    ref:login"));

    fixture.set_query("");
    assert_eq!(fixture.selected().as_deref(), Some("  ref:main"));
}

#[gpui::test]
async fn empty_results_report_nothing_found_or_branch_not_found(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-empty"), standard_repository).await;

    fixture.set_query("zzzz");
    assert!(fixture.rows().is_empty());
    let message =
        fixture.update_picker(|picker, window, cx| picker.delegate.no_matches_text(window, cx));
    assert_eq!(message.as_deref(), Some("Nothing found"));

    fixture.update_picker(|picker, _, cx| {
        picker
            .delegate
            .toggle_setting(PopupToggle::ShowActionsInSearch, cx);
    });
    let message =
        fixture.update_picker(|picker, window, cx| picker.delegate.no_matches_text(window, cx));
    assert_eq!(message.as_deref(), Some("Branch not found"));
}

#[gpui::test]
async fn disabling_actions_in_search_changes_the_placeholder_and_hides_action_matches(
    cx: &mut TestAppContext,
) {
    let mut fixture = Fixture::open(
        cx,
        path!("/branches-popup-actions-toggle"),
        standard_repository,
    )
    .await;

    let placeholder =
        fixture.update_picker(|picker, window, cx| picker.delegate.placeholder_text(window, cx));
    assert_eq!(&*placeholder, "Search for branches and actions");

    fixture.update_picker(|picker, window, cx| {
        picker
            .delegate
            .toggle_setting(PopupToggle::ShowActionsInSearch, cx);
        picker.refresh_placeholder(window, cx);
        picker.refresh(window, cx);
    });
    let placeholder =
        fixture.update_picker(|picker, window, cx| picker.delegate.placeholder_text(window, cx));
    assert_eq!(&*placeholder, "Search for branches");

    fixture.set_query("new");
    assert!(fixture.rows().is_empty());
}

#[gpui::test]
async fn settings_toggles_rebuild_the_tree(cx: &mut TestAppContext) {
    let mut fixture =
        Fixture::open(cx, path!("/branches-popup-toggles"), standard_repository).await;

    fixture.update_picker(|picker, window, cx| {
        picker.delegate.toggle_setting(PopupToggle::ShowRecent, cx);
        picker.delegate.toggle_setting(PopupToggle::ShowTags, cx);
        picker.refresh(window, cx);
    });
    let rows = fixture.rows();
    assert!(!rows.contains(&"section:Recent:open".to_string()));
    assert!(!rows.contains(&"section:Tags:closed".to_string()));
    assert!(rows.contains(&"section:Local:closed".to_string()));

    fixture.select("section:Local:closed");
    fixture.select_child();
    fixture.update_picker(|picker, window, cx| {
        picker
            .delegate
            .toggle_setting(PopupToggle::GroupByDirectory, cx);
        picker.refresh(window, cx);
    });
    assert!(fixture.rows().contains(&"  ref:feature/login".to_string()));
}

#[gpui::test]
async fn toggling_favorites_reorders_refs_and_records_the_change(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-favorites"), |fs, dot_git| {
        fs.insert_branches(dot_git, &["main", "topic", "alpha", "beta"]);
        fs.set_recent_branches_for_repo(dot_git, &["main"]);
        fs.insert_tags(dot_git, &[("v1.0", "aaa111")]);
    })
    .await;

    fixture.select("section:Local:closed");
    fixture.select_child();
    let local_refs: Vec<String> = fixture
        .rows()
        .into_iter()
        .filter(|row| row.starts_with("  ref:"))
        .collect();
    assert_eq!(
        local_refs,
        vec!["  ref:main", "  ref:alpha", "  ref:beta", "  ref:topic"]
    );
    assert!(fixture.ref_row("main").is_favorite);
    assert!(!fixture.ref_row("beta").is_favorite);

    let beta_index = fixture.index_of("  ref:beta");
    let toggled = fixture.update_picker(|picker, window, cx| {
        let toggled = picker.delegate.toggle_favorite_at(beta_index, cx);
        picker.refresh(window, cx);
        toggled
    });
    assert!(toggled);
    assert!(fixture.ref_row("beta").is_favorite);
    let rows = fixture.rows();
    let local_section = rows
        .iter()
        .position(|row| row == "section:Local:open")
        .expect("local section is open");
    assert_eq!(rows[local_section + 1], "  ref:beta");

    let main_index = fixture.index_of("  ref:main");
    fixture.update_picker(|picker, window, cx| {
        picker.delegate.toggle_favorite_at(main_index, cx);
        picker.refresh(window, cx);
    });
    assert!(!fixture.ref_row("main").is_favorite);

    let persisted = fixture.picker().read_with(&fixture.cx, |picker, _| {
        picker.delegate.state.to_persisted()
    });
    assert_eq!(persisted.added_favorites.len(), 1);
    assert_eq!(persisted.removed_favorites.len(), 1);
}

#[gpui::test]
async fn tags_cannot_be_marked_as_favorites(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(
        cx,
        path!("/branches-popup-tag-favorite"),
        standard_repository,
    )
    .await;

    fixture.select("section:Tags:closed");
    fixture.select_child();
    let tag_index = fixture.index_of("  ref:v1.0");
    let toggled =
        fixture.update_picker(|picker, _, cx| picker.delegate.toggle_favorite_at(tag_index, cx));
    assert!(!toggled);
    assert!(!fixture.ref_row("v1.0").is_favorite);
}

#[gpui::test]
async fn ongoing_rebase_adds_abort_continue_and_skip_rows(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx, path!("/branches-popup-rebase"), |fs, dot_git| {
        standard_repository(fs, dot_git);
        fs.set_operation_in_progress_for_repo(dot_git, Some(RepositoryOperation::Rebase));
    })
    .await;

    let rows = fixture.rows();
    for label in [
        "action:Abort Rebase",
        "action:Continue Rebase",
        "action:Skip Commit",
    ] {
        assert!(rows.contains(&label.to_string()), "missing {label}");
    }
    assert!(!rows.contains(&"action:Abort Merge".to_string()));
}

#[gpui::test]
async fn head_changes_reload_recent_branches(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-head"), |fs, dot_git| {
        fs.insert_branches(dot_git, &["main", "topic"]);
        fs.set_recent_branches_for_repo(dot_git, &["main"]);
    })
    .await;
    assert!(!fixture.rows().contains(&"  ref:topic".to_string()));

    fixture
        .fs
        .set_recent_branches_for_repo(&fixture.dot_git, &["topic", "main"]);
    fixture.repository.update(&mut fixture.cx, |_, cx| {
        cx.emit(RepositoryEvent::HeadChanged);
    });
    fixture.cx.run_until_parked();

    assert!(fixture.rows().contains(&"  ref:topic".to_string()));
}

#[gpui::test]
async fn escape_clears_the_query_before_dismissing(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-escape"), standard_repository).await;
    let dismissed = Rc::new(Cell::new(false));
    let _subscription = fixture.cx.update(|_, cx| {
        let dismissed = dismissed.clone();
        cx.subscribe(&fixture.popup, move |_, _: &DismissEvent, _| {
            dismissed.set(true)
        })
    });

    fixture.update_picker(|picker, window, cx| picker.set_query("top", window, cx));
    let query = fixture
        .picker()
        .read_with(&fixture.cx, |picker, cx| picker.query(cx));
    assert_eq!(query, "top");
    assert_eq!(fixture.rows(), vec!["section:Recent:open", "  ref:topic"]);

    fixture
        .popup
        .update_in(&mut fixture.cx, |popup, window, cx| {
            popup.clear_search_or_close(&ClearSearchOrClose, window, cx)
        });
    fixture.cx.run_until_parked();
    let query = fixture
        .picker()
        .read_with(&fixture.cx, |picker, cx| picker.query(cx));
    assert_eq!(query, "");
    assert!(!dismissed.get());
    assert!(fixture.rows().contains(&"action:Commit…".to_string()));

    fixture
        .popup
        .update_in(&mut fixture.cx, |popup, window, cx| {
            popup.clear_search_or_close(&ClearSearchOrClose, window, cx)
        });
    fixture.cx.run_until_parked();
    assert!(dismissed.get());
}

#[gpui::test]
async fn expansion_state_is_persisted_and_restored_by_a_new_popup(cx: &mut TestAppContext) {
    let mut fixture =
        Fixture::open(cx, path!("/branches-popup-persist"), standard_repository).await;
    fixture.select("section:Local:closed");
    fixture.select_child();
    assert!(fixture.rows().contains(&"section:Local:open".to_string()));

    fixture.reopen();
    fixture.cx.run_until_parked();

    assert!(fixture.rows().contains(&"section:Local:open".to_string()));
    assert!(
        fixture
            .rows()
            .contains(&"section:Remote:closed".to_string())
    );
}

#[gpui::test]
async fn the_current_branch_outside_recent_is_revealed_on_open(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx, path!("/branches-popup-reveal"), |fs, dot_git| {
        fs.insert_branches(dot_git, &["refs/heads/feature/login", "main"]);
        fs.set_recent_branches_for_repo(dot_git, &["main"]);
    })
    .await;

    let rows = fixture.rows();
    assert!(rows.contains(&"section:Local:open".to_string()), "{rows:?}");
    assert!(
        rows.contains(&"  folder:feature:open".to_string()),
        "{rows:?}"
    );
    assert_eq!(fixture.selected().as_deref(), Some("    ref:login"));
}

#[gpui::test]
async fn multiple_repositories_add_repository_rows_and_switching_changes_the_tree(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    let first_root = path!("/branches-popup-multi-alpha");
    let second_root = path!("/branches-popup-multi-beta");
    create_repository(&fs, first_root, |fs, dot_git| {
        fs.insert_branches(dot_git, &["main", "alpha-branch"]);
        fs.set_recent_branches_for_repo(dot_git, &["alpha-branch", "main"]);
    })
    .await;
    create_repository(&fs, second_root, |fs, dot_git| {
        fs.insert_branches(dot_git, &["trunk", "beta-branch"]);
        fs.set_recent_branches_for_repo(dot_git, &["beta-branch", "trunk"]);
    })
    .await;
    let project = Project::test(
        fs.clone(),
        [Path::new(first_root), Path::new(second_root)],
        cx,
    )
    .await;
    cx.run_until_parked();

    let alpha = cx.read(|cx| {
        project
            .read(cx)
            .git_store()
            .read(cx)
            .repositories()
            .values()
            .find(|repository| &*repository.read(cx).display_name() == "branches-popup-multi-alpha")
            .cloned()
    });
    let alpha = alpha.expect("alpha repository exists");

    let window = cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
    let workspace = window
        .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
        .expect("window is alive");
    let mut visual_cx = VisualTestContext::from_window(window.into(), cx);
    let popup = Fixture::create_popup(window, &workspace, &alpha, &mut visual_cx);
    visual_cx.run_until_parked();
    let mut fixture = Fixture {
        fs,
        dot_git: Path::new(first_root).join(".git"),
        window,
        workspace,
        repository: alpha,
        popup,
        cx: visual_cx,
    };

    let rows = fixture.rows();
    assert!(rows.contains(&"repo:branches-popup-multi-alpha".to_string()));
    assert!(rows.contains(&"repo:branches-popup-multi-beta".to_string()));
    assert!(rows.contains(&"  ref:alpha-branch".to_string()));
    assert!(!rows.contains(&"  ref:beta-branch".to_string()));

    fixture.select("repo:branches-popup-multi-beta");
    fixture.update_picker(|picker, window, cx| picker.delegate.confirm(false, window, cx));
    fixture.cx.run_until_parked();

    let rows = fixture.rows();
    assert!(rows.contains(&"  ref:beta-branch".to_string()), "{rows:?}");
    assert!(!rows.contains(&"  ref:alpha-branch".to_string()));
    let active_name = fixture.picker().read_with(&fixture.cx, |picker, cx| {
        picker
            .delegate
            .repository
            .as_ref()
            .map(|repository| repository.read(cx).display_name())
    });
    assert_eq!(active_name.as_deref(), Some("branches-popup-multi-beta"));
}

#[gpui::test]
async fn without_a_repository_the_popup_has_no_rows_and_says_so(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-none"), standard_repository).await;
    let workspace = fixture.workspace.downgrade();
    fixture.popup = fixture
        .window
        .update(&mut fixture.cx, |_, window, cx| {
            cx.new(|cx| BranchesPopup::new(workspace, None, BranchesPopupStyle::Modal, window, cx))
        })
        .expect("window is alive");
    fixture.cx.run_until_parked();

    assert!(fixture.rows().is_empty());
    let message =
        fixture.update_picker(|picker, window, cx| picker.delegate.no_matches_text(window, cx));
    assert_eq!(message.as_deref(), Some("No Git repository"));
}

#[gpui::test]
async fn the_best_scoring_match_is_selected_not_the_first_in_display_order(
    cx: &mut TestAppContext,
) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-best"), |fs, dot_git| {
        fs.insert_branches(dot_git, &["main", "topic", "a-topic", "topic-b"]);
        fs.set_recent_branches_for_repo(dot_git, &[]);
    })
    .await;

    fixture.set_query("topic");
    assert_eq!(
        fixture.rows(),
        vec![
            "section:Local:open",
            "  ref:a-topic",
            "  ref:topic",
            "  ref:topic-b"
        ]
    );
    assert_eq!(fixture.selected().as_deref(), Some("  ref:topic"));
}

#[gpui::test]
async fn left_and_right_drive_the_tree_only_while_the_search_field_has_focus(
    cx: &mut TestAppContext,
) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-arrows"), standard_repository).await;
    fixture.select("section:Local:closed");

    fixture
        .popup
        .update_in(&mut fixture.cx, |popup, window, cx| {
            popup.select_child(&menu::SelectChild, window, cx)
        });
    fixture.cx.run_until_parked();
    assert!(fixture.rows().contains(&"section:Local:closed".to_string()));

    fixture
        .popup
        .update_in(&mut fixture.cx, |popup, window, cx| {
            popup.focus_handle(cx).focus(window, cx)
        });
    fixture
        .popup
        .update_in(&mut fixture.cx, |popup, window, cx| {
            popup.select_child(&menu::SelectChild, window, cx)
        });
    fixture.cx.run_until_parked();
    assert!(fixture.rows().contains(&"section:Local:open".to_string()));

    fixture
        .popup
        .update_in(&mut fixture.cx, |popup, window, cx| {
            popup.select_parent(&menu::SelectParent, window, cx)
        });
    fixture.cx.run_until_parked();
    assert!(fixture.rows().contains(&"section:Local:closed".to_string()));
}

#[gpui::test]
async fn a_change_made_before_the_saved_state_loads_is_merged_instead_of_overwritten(
    cx: &mut TestAppContext,
) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-merge"), standard_repository).await;
    fixture.update_picker(|picker, _, cx| {
        picker.delegate.toggle_setting(PopupToggle::ShowTags, cx);
    });

    fixture.popup = Fixture::create_popup(
        fixture.window,
        &fixture.workspace,
        &fixture.repository,
        &mut fixture.cx,
    );
    fixture.update_picker(|picker, _, cx| {
        assert!(!picker.delegate.state_loaded);
        picker.delegate.toggle_setting(PopupToggle::ShowRecent, cx);
    });
    let toggles = fixture
        .picker()
        .read_with(&fixture.cx, |picker, _| picker.delegate.state.toggles);
    assert!(!toggles.show_tags, "the saved change must survive the load");
    assert!(
        !toggles.show_recent,
        "the early change must survive the load"
    );

    fixture.reopen();
    let toggles = fixture
        .picker()
        .read_with(&fixture.cx, |picker, _| picker.delegate.state.toggles);
    assert!(!toggles.show_tags);
    assert!(!toggles.show_recent);
}

#[gpui::test]
async fn update_project_loses_its_ellipsis_when_the_options_dialog_is_disabled(
    cx: &mut TestAppContext,
) {
    let mut fixture = Fixture::open(
        cx,
        path!("/branches-popup-update-label"),
        standard_repository,
    )
    .await;
    assert!(
        fixture
            .rows()
            .contains(&"action:Update Project…".to_string())
    );

    fixture
        .cx
        .update(|_, cx| integrate::set_show_update_options(false, cx));
    fixture.cx.run_until_parked();
    fixture.reopen();

    assert!(
        fixture
            .rows()
            .contains(&"action:Update Project".to_string())
    );
}

#[test]
fn commit_counts_are_capped_at_ninety_nine_plus() {
    assert_eq!(delegate::commit_count_label(1), "1");
    assert_eq!(delegate::commit_count_label(99), "99");
    assert_eq!(delegate::commit_count_label(100), "99+");
    assert_eq!(delegate::commit_count_label(5000), "99+");
}

#[test]
fn ref_icons_follow_the_intellij_priority() {
    assert_eq!(
        delegate::ref_icon(true, true, false),
        IconName::CurrentBranchFavoriteLabel
    );
    assert_eq!(
        delegate::ref_icon(true, false, false),
        IconName::CurrentBranchLabel
    );
    assert_eq!(delegate::ref_icon(false, true, false), IconName::StarFilled);
    assert_eq!(delegate::ref_icon(false, false, true), IconName::TagLabel);
    assert_eq!(
        delegate::ref_icon(false, false, false),
        IconName::BranchNode
    );
}

#[test]
fn highlight_positions_are_converted_from_characters_to_utf8_byte_offsets() {
    assert_eq!(
        delegate::char_positions_to_byte_offsets("New Branch…", &[0, 4, 10]),
        vec![0, 4, 10]
    );
    assert_eq!(
        delegate::char_positions_to_byte_offsets("héllo", &[0, 1, 2, 4]),
        vec![0, 1, 3, 5]
    );
}

#[test]
fn out_of_range_highlight_positions_are_dropped() {
    assert_eq!(
        delegate::char_positions_to_byte_offsets("abc", &[1, 7]),
        vec![1]
    );
    assert!(delegate::char_positions_to_byte_offsets("abc", &[]).is_empty());
}

#[test]
fn search_placeholder_mentions_actions_only_when_they_are_searchable() {
    assert_eq!(
        delegate::search_placeholder(true),
        "Search for branches and actions"
    );
    assert_eq!(delegate::search_placeholder(false), "Search for branches");
}
