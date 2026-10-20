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

    fn open_in_modal_layer(&mut self) {
        self.workspace
            .update_in(&mut self.cx, |workspace, window, cx| {
                open_modal(workspace, window, cx)
            });
        self.cx.run_until_parked();
        self.popup = self
            .workspace
            .read_with(&self.cx, |workspace, cx| {
                workspace.active_modal::<BranchesPopup>(cx)
            })
            .expect("the modal layer shows the popup");
    }

    fn focus_workspace(&mut self) {
        self.workspace
            .update_in(&mut self.cx, |workspace, window, cx| {
                workspace.focus_handle(cx).focus(window, cx)
            });
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

    fn selected_index(&self) -> usize {
        let picker = self.picker();
        picker.read_with(&self.cx, |picker, _| picker.delegate.selected_index)
    }

    fn rows_after(&self, line: &str) -> Vec<String> {
        let rows = self.rows();
        let start = self.index_of(line) + 1;
        rows.get(start..)
            .map(<[String]>::to_vec)
            .unwrap_or_default()
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
            "action:New Worktree…",
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
    let section_name = fixture.picker().read_with(&fixture.cx, |picker, _| {
        picker.delegate.section_repository_name.clone()
    });
    assert_eq!(section_name, None, "one repository keeps the short titles");
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
    assert_eq!(
        fixture.rows_after("section:Local:open"),
        vec!["  ref:main", "  folder:feature:closed", "  ref:topic"]
    );
    assert_eq!(fixture.selected().as_deref(), Some("section:Local:open"));

    fixture.select_child();
    assert_eq!(
        fixture.selected_index(),
        fixture.index_of("section:Local:open") + 1
    );
    assert_eq!(fixture.selected().as_deref(), Some("  ref:main"));

    fixture.select("  folder:feature:closed");
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
    assert_eq!(
        fixture.rows(),
        vec!["action:New Branch…", "action:New Worktree…"]
    );
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
    assert_eq!(
        fixture.rows_after("section:Local:open"),
        vec![
            "  ref:main",
            "  ref:alpha",
            "  ref:beta",
            "  ref:topic",
            "section:Tags:closed"
        ]
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
    assert_eq!(
        fixture.rows_after("section:Local:open"),
        vec![
            "  ref:main",
            "  ref:beta",
            "  ref:alpha",
            "  ref:topic",
            "section:Tags:closed"
        ]
    );

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
async fn tags_can_be_marked_as_favorites(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-tag-favorite"), |fs, dot_git| {
        standard_repository(fs, dot_git);
        fs.insert_tags(dot_git, &[("v0.9", "aaa000")]);
    })
    .await;

    fixture.select("section:Tags:closed");
    fixture.select_child();
    assert_eq!(
        fixture.rows_after("section:Tags:open"),
        vec!["  ref:v0.9", "  ref:v1.0"]
    );
    let tag_index = fixture.index_of("  ref:v1.0");
    let toggled = fixture.update_picker(|picker, window, cx| {
        let toggled = picker.delegate.toggle_favorite_at(tag_index, cx);
        picker.refresh(window, cx);
        toggled
    });
    assert!(toggled);
    assert!(fixture.ref_row("v1.0").is_favorite);
    assert_eq!(
        fixture.rows_after("section:Tags:open"),
        vec!["  ref:v1.0", "  ref:v0.9"]
    );
}

#[gpui::test]
async fn the_tag_at_a_detached_head_is_marked_current_but_not_preselected(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-detached-tag"), |fs, dot_git| {
        fs.insert_branches(dot_git, &["main"]);
        fs.set_branch_name(dot_git, None::<String>);
        fs.insert_tags(dot_git, &[("v0.9", "aaa000"), ("v1.0", "deadbeef")]);
    })
    .await;

    fixture.select("section:Tags:closed");
    fixture.select_child();
    assert_eq!(
        fixture.rows_after("section:Tags:open"),
        vec!["  ref:v1.0", "  ref:v0.9"]
    );
    assert!(fixture.ref_row("v1.0").is_current);
    assert!(!fixture.ref_row("v0.9").is_current);

    fixture.reopen();
    assert!(fixture.rows().contains(&"section:Tags:open".to_string()));
    assert_eq!(
        fixture.selected().as_deref(),
        Some("action:Update Project…")
    );
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
    let (section_name, color_indices) = fixture.picker().read_with(&fixture.cx, |picker, _| {
        let color_indices: Vec<(String, usize)> = picker
            .delegate
            .rows()
            .iter()
            .filter_map(|row| match row {
                PopupRow::Repository(row) => {
                    Some((row.choice.name.to_string(), row.choice.color_index))
                }
                _ => None,
            })
            .collect();
        (
            picker.delegate.section_repository_name.clone(),
            color_indices,
        )
    });
    assert_eq!(section_name.as_deref(), Some("branches-popup-multi-alpha"));
    assert_eq!(
        color_indices,
        vec![
            ("branches-popup-multi-alpha".to_string(), 0),
            ("branches-popup-multi-beta".to_string(), 1)
        ]
    );

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
    let section_name = fixture.picker().read_with(&fixture.cx, |picker, _| {
        picker.delegate.section_repository_name.clone()
    });
    assert_eq!(section_name.as_deref(), Some("branches-popup-multi-beta"));
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
    fixture.open_in_modal_layer();
    fixture.focus_workspace();
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

    // Without a state saved in this session, the popup loads the stored one asynchronously.
    fixture
        .cx
        .update(|_, cx| cx.set_global(state::SavedPopupStates::default()));
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

#[gpui::test]
fn the_jetbrains_keymap_has_no_shortcut_for_the_branches_popup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let bindings = settings::KeymapFile::load_asset_allow_partial_failure(
            "keymaps/macos/jetbrains.json",
            cx,
        )
        .expect("the JetBrains keymap loads");
        assert!(
            !bindings
                .iter()
                .any(|binding| binding.action().partial_eq(&zed_actions::git::Branch)),
            "IntelliJ removes Git.Branches' shift-cmd-` from its macOS keymaps"
        );
    });
}

#[test]
fn commit_count_tooltip_names_only_the_directions_that_have_commits() {
    assert_eq!(
        delegate::commit_counts_tooltip(3, 2),
        "3 incoming and 2 outgoing commits"
    );
    assert_eq!(delegate::commit_counts_tooltip(1, 0), "1 incoming commit");
    assert_eq!(delegate::commit_counts_tooltip(4, 0), "4 incoming commits");
    assert_eq!(delegate::commit_counts_tooltip(0, 1), "1 outgoing commit");
    assert_eq!(delegate::commit_counts_tooltip(0, 7), "7 outgoing commits");
}

#[test]
fn popup_metrics_follow_the_intellij_new_ui_header_and_tree_rows() {
    assert_eq!(delegate::SEPARATOR_HEIGHT_PX, delegate::ROW_HEIGHT_PX);
    assert_eq!(delegate::ROW_HEIGHT_PX, 24.);
    assert_eq!(
        delegate::HEADER_TOP_INSET_PX + delegate::SEARCH_BOX_HEIGHT_PX,
        40.
    );
    assert_eq!(delegate::HEADER_TOP_INSET_PX, 12.);
    assert_eq!(delegate::HEADER_LEFT_INSET_PX, 10.);
    assert_eq!(delegate::HEADER_RIGHT_INSET_PX, 8.);
    assert_eq!(delegate::HEADER_TOOLBAR_GAP_PX, 6.);
}

#[test]
fn folder_rows_always_use_the_closed_folder_icon() {
    assert_eq!(delegate::FOLDER_ICON, IconName::Folder);
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

fn notice_titles(fixture: &Fixture) -> Vec<Option<String>> {
    fixture.workspace.read_with(&fixture.cx, |workspace, cx| {
        workspace
            .notification_views()
            .into_iter()
            .filter_map(|view| {
                view.downcast::<crate::branch_operations::branch_notification::BranchNotification>()
                    .ok()
            })
            .map(|notification| {
                notification
                    .read(cx)
                    .notice()
                    .title
                    .as_ref()
                    .map(|title| title.to_string())
            })
            .collect()
    })
}

fn drag(cx: &mut VisualTestContext, from: gpui::Point<Pixels>, by: gpui::Point<Pixels>) {
    let modifiers = gpui::Modifiers::none();
    cx.simulate_mouse_move(from, None, modifiers);
    cx.simulate_mouse_down(from, MouseButton::Left, modifiers);
    cx.simulate_mouse_move(
        from + gpui::point(px(5.), px(5.)),
        MouseButton::Left,
        modifiers,
    );
    cx.simulate_mouse_move(from + by, MouseButton::Left, modifiers);
    cx.simulate_mouse_up(from + by, MouseButton::Left, modifiers);
    cx.run_until_parked();
}

fn bounds_of(cx: &mut VisualTestContext, selector: String) -> gpui::Bounds<Pixels> {
    let selector: &'static str = Box::leak(selector.into_boxed_str());
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is rendered"))
}

#[gpui::test]
async fn the_previous_branch_is_preselected_after_a_checkout_from_flint(cx: &mut TestAppContext) {
    let mut fixture =
        Fixture::open(cx, path!("/branches-popup-previous"), standard_repository).await;
    let context = crate::branch_operations::BranchContext::new(
        fixture.workspace.downgrade(),
        fixture.repository.clone(),
    );
    fixture.cx.update(|window, cx| {
        crate::branch_operations::checkout::checkout(
            context,
            crate::branch_refs::RefTarget::local("topic"),
            window,
            cx,
        )
    });
    fixture.cx.run_until_parked();
    let current = fixture.repository.read_with(&fixture.cx, |repository, _| {
        repository
            .branch
            .as_ref()
            .map(|branch| branch.name().to_string())
    });
    assert_eq!(current.as_deref(), Some("topic"));

    fixture.reopen();
    assert!(fixture.ref_row("topic").is_current);
    assert_eq!(fixture.selected().as_deref(), Some("  ref:main"));

    let key = state::previous_branch_key(Path::new(path!("/branches-popup-previous")));
    let stored = fixture
        .cx
        .update(|_, cx| state::load_previous_branch(key, cx));
    assert_eq!(stored.await.as_deref(), Some("main"));
}

#[gpui::test]
async fn the_current_branch_is_preselected_when_the_previous_one_is_gone(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(
        cx,
        path!("/branches-popup-previous-gone"),
        standard_repository,
    )
    .await;
    let key = state::previous_branch_key(Path::new(path!("/branches-popup-previous-gone")));
    fixture
        .cx
        .update(|_, cx| state::record_previous_branch(key, "deleted-branch".to_string(), cx))
        .await;
    fixture.reopen();
    assert_eq!(fixture.selected().as_deref(), Some("  ref:main"));
}

#[gpui::test]
async fn while_filtering_arrow_keys_move_over_leaves_only_and_wrap(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-leaf-keys"), |fs, dot_git| {
        fs.insert_branches(
            dot_git,
            &[
                "main",
                "refs/heads/feature/alpha",
                "refs/heads/feature/beta",
            ],
        );
        fs.set_recent_branches_for_repo(dot_git, &["main"]);
    })
    .await;
    fixture.open_in_modal_layer();
    fixture.set_query("feature");
    assert_eq!(
        fixture.rows(),
        vec![
            "section:Local:open",
            "  folder:feature:open",
            "    ref:alpha",
            "    ref:beta"
        ]
    );
    assert_eq!(fixture.selected().as_deref(), Some("    ref:alpha"));

    let press = |fixture: &mut Fixture, action: Box<dyn gpui::Action>| {
        fixture
            .cx
            .update(|window, cx| window.dispatch_action(action, cx));
        fixture.cx.run_until_parked();
        fixture.selected()
    };
    assert_eq!(
        press(&mut fixture, Box::new(menu::SelectNext)).as_deref(),
        Some("    ref:beta")
    );
    assert_eq!(
        press(&mut fixture, Box::new(menu::SelectNext)).as_deref(),
        Some("    ref:alpha")
    );
    assert_eq!(
        press(&mut fixture, Box::new(zed_actions::editor::MoveUp)).as_deref(),
        Some("    ref:beta")
    );
    assert_eq!(
        press(&mut fixture, Box::new(menu::SelectFirst)).as_deref(),
        Some("    ref:alpha")
    );
}

#[gpui::test]
async fn searching_opens_only_intellij_paths_and_the_arrows_open_the_rest(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-search-paths"), |fs, dot_git| {
        fs.insert_branches(
            dot_git,
            &["main", "refs/heads/feature/login", "origin/feature/login"],
        );
        fs.set_recent_branches_for_repo(dot_git, &["main"]);
    })
    .await;
    fixture.open_in_modal_layer();
    fixture.set_query("login");
    assert_eq!(
        fixture.rows(),
        vec![
            "section:Local:open",
            "  folder:feature:open",
            "    ref:login",
            "section:Remote:closed",
        ]
    );
    assert_eq!(fixture.selected().as_deref(), Some("    ref:login"));

    fixture.cx.dispatch_action(menu::SelectNext);
    fixture.cx.run_until_parked();
    assert_eq!(
        fixture.rows(),
        vec![
            "section:Local:open",
            "  folder:feature:open",
            "    ref:login",
            "section:Remote:open",
            "  folder:origin:open",
            "    folder:feature:open",
            "      ref:login",
        ]
    );
    assert_eq!(fixture.selected().as_deref(), Some("      ref:login"));

    fixture.set_query("");
    assert!(
        fixture.rows().contains(&"section:Remote:open".to_string()),
        "what a search opened stays open, like IntelliJ's expanded paths"
    );
}

#[gpui::test]
async fn the_search_text_turns_red_when_nothing_matches(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-red"), standard_repository).await;
    let nothing_matches = |fixture: &Fixture| {
        fixture
            .picker()
            .read_with(&fixture.cx, |picker, _| picker.delegate.nothing_matches())
    };
    let search_text_color = |fixture: &Fixture| {
        fixture.picker().read_with(&fixture.cx, |picker, cx| {
            picker.delegate.search_text_color(cx)
        })
    };
    let (error_color, text_color) = fixture.cx.update(|_, cx| {
        (
            ui::Color::Error.color(cx),
            ui::ActiveTheme::theme(cx).colors().text,
        )
    });
    assert!(!nothing_matches(&fixture));
    assert_eq!(search_text_color(&fixture), text_color);
    fixture.set_query("zzzz");
    assert!(nothing_matches(&fixture));
    assert_eq!(search_text_color(&fixture), error_color);
    fixture.set_query("top");
    assert!(!nothing_matches(&fixture));
    fixture.set_query("/");
    assert!(!nothing_matches(&fixture));
}

#[gpui::test]
async fn the_search_field_is_drawn_red_when_nothing_matches(cx: &mut TestAppContext) {
    let mut fixture =
        Fixture::open(cx, path!("/branches-popup-red-draw"), standard_repository).await;
    fixture.open_in_modal_layer();
    let drawn_text_color = |fixture: &mut Fixture| {
        fixture.cx.update(|window, _| window.refresh());
        fixture.cx.run_until_parked();
        let query_editor = fixture
            .picker()
            .read_with(&fixture.cx, |picker, _| picker.query_editor())
            .expect("the popup has a search field");
        let editor = query_editor
            .as_any()
            .downcast_ref::<Entity<editor::Editor>>()
            .expect("the search field is an editor")
            .clone();
        editor.update(&mut fixture.cx, |editor, cx| editor.style(cx).text.color)
    };
    let (error_color, text_color) = fixture.cx.update(|_, cx| {
        (
            ui::Color::Error.color(cx),
            ui::ActiveTheme::theme(cx).colors().text,
        )
    });

    fixture.set_query("top");
    assert_eq!(drawn_text_color(&mut fixture), text_color);
    fixture.set_query("zzzz");
    assert_eq!(
        drawn_text_color(&mut fixture),
        error_color,
        "IntelliJ's search field turns red when nothing matches"
    );
}

#[gpui::test]
fn search_matches_use_the_intellij_search_match_colors(cx: &mut TestAppContext) {
    init_test(cx);
    cx.update(|cx| {
        let style = delegate::search_match_style(cx);
        let (background, foreground) = if cx.theme().appearance().is_light() {
            (0xFEE6B1, 0x323232)
        } else {
            (0xBA9752, 0x000000)
        };
        assert_eq!(style.background_color, Some(gpui::rgb(background).into()));
        assert_eq!(style.color, Some(gpui::rgb(foreground).into()));
    });
}

#[gpui::test]
async fn new_worktree_opens_the_worktree_picker(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(
        cx,
        path!("/branches-popup-new-worktree"),
        standard_repository,
    )
    .await;
    fixture.select("action:New Worktree…");
    fixture.update_picker(|picker, window, cx| picker.delegate.confirm(false, window, cx));
    fixture.cx.run_until_parked();
    let opened = fixture.workspace.read_with(&fixture.cx, |workspace, cx| {
        workspace
            .active_modal::<git_ui_core::worktree_picker::WorktreePicker>(cx)
            .is_some()
    });
    assert!(opened, "New Worktree… opens the worktree picker");
}

#[gpui::test]
async fn a_branch_checked_out_in_another_worktree_offers_open_worktree(cx: &mut TestAppContext) {
    let fixture = Fixture::open(
        cx,
        path!("/branches-popup-other-worktree"),
        standard_repository,
    )
    .await;
    let worktree_path = PathBuf::from(path!("/branches-popup-other-worktree-topic"));
    fixture
        .fs
        .add_linked_worktree_for_repo(
            &fixture.dot_git,
            true,
            git::repository::Worktree {
                path: worktree_path.clone(),
                ref_name: Some("refs/heads/topic".into()),
                sha: "deadbeef".into(),
                is_main: false,
                is_bare: false,
            },
        )
        .await;
    fixture.cx.run_until_parked();

    let plan = fixture.picker().read_with(&fixture.cx, |picker, cx| {
        picker
            .delegate
            .menu_plan_for(&crate::branch_refs::RefTarget::local("topic"), cx)
    });
    let actions: Vec<ref_menu::MenuAction> = plan
        .iter()
        .filter_map(|planned| match planned {
            ref_menu::MenuItemPlan::Item(item) => Some(item.action.clone()),
            _ => None,
        })
        .collect();
    assert!(actions.contains(&ref_menu::MenuAction::OpenWorktree {
        path: worktree_path
    }));
    assert!(!actions.contains(&ref_menu::MenuAction::CheckoutAndRebase));
    assert!(actions.contains(&ref_menu::MenuAction::NewWorktreeFrom));
}

#[gpui::test]
async fn open_worktree_adds_a_project_tab_instead_of_opening_a_window(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(
        cx,
        path!("/branches-popup-open-worktree"),
        standard_repository,
    )
    .await;
    let worktree_path = PathBuf::from(path!("/branches-popup-open-worktree-topic"));
    fixture
        .fs
        .insert_tree(&worktree_path, json!({ "file.txt": "content" }))
        .await;
    let fs = fixture.fs.clone();
    fixture.cx.update(|_, cx| <dyn fs::Fs>::set_global(fs, cx));
    let context = fixture
        .picker()
        .read_with(&fixture.cx, |picker, _| picker.delegate.branch_context())
        .expect("the popup has a repository");
    let windows_before = fixture.cx.update(|_, cx| cx.windows().len());

    fixture.cx.update(|window, cx| {
        ref_menu::run_menu_action(
            ref_menu::MenuAction::OpenWorktree {
                path: worktree_path.clone(),
            },
            context,
            crate::branch_refs::RefTarget::local("topic"),
            window,
            cx,
        )
    });
    fixture.cx.run_until_parked();

    assert_eq!(
        fixture.cx.update(|_, cx| cx.windows().len()),
        windows_before,
        "Open Worktree… never opens a window"
    );
    fixture
        .window
        .read_with(&fixture.cx, |multi_workspace, cx| {
            assert_eq!(multi_workspace.workspaces().count(), 2);
            assert!(
                multi_workspace
                    .workspaces()
                    .any(|workspace| workspace == &fixture.workspace),
                "the source project stays in the window"
            );
            assert_eq!(
                multi_workspace.workspace().read(cx).root_paths(cx),
                vec![Arc::from(worktree_path.as_path())]
            );
        })
        .expect("window is alive");
}

#[gpui::test]
async fn a_branch_with_an_upstream_gets_the_tracked_branch_submenu(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx, path!("/branches-popup-tracked"), standard_repository).await;
    let plan = fixture.picker().read_with(&fixture.cx, |picker, cx| {
        picker
            .delegate
            .menu_plan_for(&crate::branch_refs::RefTarget::local("main"), cx)
    });
    let submenu = plan.iter().find_map(|planned| match planned {
        ref_menu::MenuItemPlan::Submenu { label, .. } => Some(label.clone()),
        _ => None,
    });
    assert_eq!(submenu.as_deref(), Some("Tracked Branch 'origin/main'"));
}

#[gpui::test]
async fn the_delete_shortcut_deletes_the_selected_ref_only_when_delete_is_offered(
    cx: &mut TestAppContext,
) {
    let mut fixture =
        Fixture::open(cx, path!("/branches-popup-delete-key"), standard_repository).await;
    let branch_names = |fixture: &Fixture| {
        fixture.repository.read_with(&fixture.cx, |repository, _| {
            repository
                .branch_list
                .iter()
                .map(|branch| branch.name().to_string())
                .collect::<Vec<_>>()
        })
    };

    fixture.select("  ref:main");
    fixture
        .popup
        .update_in(&mut fixture.cx, |popup, window, cx| {
            popup.delete_ref(&DeleteRef, window, cx)
        });
    fixture.cx.run_until_parked();
    assert!(branch_names(&fixture).contains(&"main".to_string()));

    fixture.select("  ref:topic");
    fixture
        .popup
        .update_in(&mut fixture.cx, |popup, window, cx| {
            popup.delete_ref(&DeleteRef, window, cx)
        });
    fixture.cx.run_until_parked();
    assert!(!branch_names(&fixture).contains(&"topic".to_string()));
}

#[gpui::test]
fn the_ref_menu_binds_intellij_delete_and_shows_the_delete_key(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let bindings = settings::KeymapFile::load_asset_allow_partial_failure(
            "keymaps/default-macos.json",
            cx,
        )
        .expect("the default keymap loads");
        let delete: Vec<String> = bindings
            .iter()
            .filter(|binding| binding.action().partial_eq(&DeleteRef))
            .flat_map(|binding| {
                binding
                    .keystrokes()
                    .iter()
                    .map(|keystroke| keystroke.unparse())
            })
            .collect();
        assert_eq!(delete, vec!["backspace", "cmd-backspace", "delete"]);
    });
}

#[gpui::test]
async fn fetch_reports_success_and_shows_the_running_state(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-fetch"), standard_repository).await;
    let fetching = fixture
        .picker()
        .update_in(&mut fixture.cx, |picker, window, cx| {
            picker.delegate.start_fetch(window, cx);
            picker.delegate.fetching(cx)
        });
    assert!(fetching);
    fixture.cx.run_until_parked();
    let fetching = fixture
        .picker()
        .read_with(&fixture.cx, |picker, cx| picker.delegate.fetching(cx));
    assert!(!fetching);
    assert_eq!(
        notice_titles(&fixture),
        vec![Some("Fetch successful".to_string())]
    );
}

#[test]
fn fetch_failures_name_the_repository_only_with_several_repositories() {
    let failures = vec![(
        SharedString::from("api"),
        "fatal: could not read from remote".to_string(),
    )];
    let single = delegate::fetch_result_notice(&failures, false);
    assert_eq!(single.title.as_deref(), Some("Fetch failed"));
    assert_eq!(&*single.message, "fatal: could not read from remote");
    let several = delegate::fetch_result_notice(&failures, true);
    assert_eq!(&*several.message, "api: fatal: could not read from remote");
    let success = delegate::fetch_result_notice(&[], true);
    assert_eq!(success.title.as_deref(), Some("Fetch successful"));
    assert!(success.message.is_empty());
}

#[gpui::test]
async fn the_popup_opens_at_the_intellij_size_and_keeps_its_height_while_expanding(
    cx: &mut TestAppContext,
) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-size"), standard_repository).await;
    fixture.open_in_modal_layer();
    let visible_rows = fixture
        .picker()
        .read_with(&fixture.cx, |picker, _| picker.delegate.visible_row_count());
    // 6 actions, 2 separators and 4 sections, the 2 recent branches, plus one.
    assert_eq!(visible_rows, 15);
    let size = fixture
        .popup
        .read_with(&fixture.cx, |popup, cx| popup.popup_size(cx));
    let list_padding = fixture.cx.update(|_, cx| DynamicSpacing::Base04.px(cx));
    assert_eq!(size.width, px(MIN_WIDTH_PX));
    assert_eq!(
        size.height,
        packed_size(px(0.), visible_rows, list_padding).height
    );
    let bounds = bounds_of(&mut fixture.cx, DEBUG_SELECTOR.to_string());
    assert_eq!(bounds.size, size);

    fixture.select("section:Local:closed");
    fixture.select_child();
    fixture.select("section:Remote:closed");
    fixture.select_child();
    let expanded = fixture
        .popup
        .read_with(&fixture.cx, |popup, cx| popup.popup_size(cx));
    assert_eq!(expanded, size);
}

#[gpui::test]
async fn the_popup_grows_to_the_widest_row(cx: &mut TestAppContext) {
    let long_name = format!("feature-{}", "very-long-branch-name-".repeat(6));
    let mut fixture = Fixture::open(cx, path!("/branches-popup-wide"), |fs, dot_git| {
        fs.insert_branches(dot_git, &["main", &long_name]);
        fs.set_recent_branches_for_repo(dot_git, &[long_name.as_str(), "main"]);
    })
    .await;
    fixture.open_in_modal_layer();
    let size = fixture
        .popup
        .read_with(&fixture.cx, |popup, cx| popup.popup_size(cx));
    assert!(size.width > px(MIN_WIDTH_PX), "{size:?}");
}

#[test]
fn packed_sizes_respect_the_intellij_minimum_and_the_twenty_row_cap() {
    let minimum = packed_size(px(100.), 3, px(4.));
    assert_eq!(minimum.width, px(MIN_WIDTH_PX));
    assert_eq!(minimum.height, px(MIN_HEIGHT_PX));
    let wide = packed_size(px(500.), 3, px(4.));
    assert_eq!(wide.width, px(502.));
    assert_eq!(
        packed_size(px(0.), 40, px(4.)).height,
        packed_size(px(0.), MAX_VISIBLE_ROWS, px(4.)).height
    );
}

#[gpui::test]
async fn resizing_is_remembered_until_the_size_is_restored(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-resize"), standard_repository).await;
    fixture.open_in_modal_layer();
    let packed = fixture
        .popup
        .read_with(&fixture.cx, |popup, cx| popup.popup_size(cx));
    let corner = bounds_of(&mut fixture.cx, "branches-popup-resize-corner".to_string());
    drag(
        &mut fixture.cx,
        corner.center(),
        gpui::point(px(60.), px(50.)),
    );
    let resized = fixture
        .popup
        .read_with(&fixture.cx, |popup, cx| popup.popup_size(cx));
    assert_eq!(
        resized,
        gpui::size(packed.width + px(60.), packed.height + px(50.))
    );
    let user_size = fixture
        .picker()
        .read_with(&fixture.cx, |picker, _| picker.delegate.user_size());
    assert!(user_size.is_some());

    fixture
        .popup
        .update(&mut fixture.cx, |_, cx| cx.emit(DismissEvent));
    fixture.cx.run_until_parked();
    fixture.open_in_modal_layer();
    let reopened = fixture
        .popup
        .read_with(&fixture.cx, |popup, cx| popup.popup_size(cx));
    assert_eq!(reopened, resized);
    let restore_enabled = fixture.picker().read_with(&fixture.cx, |picker, _| {
        delegate::gear_menu_items(delegate::GearMenuOptions {
            toggles: picker.delegate.state.toggles,
            several_repositories: false,
            user_resized: picker.delegate.user_size().is_some(),
        })
        .contains(&delegate::GearMenuItem::RestorePopupSize { enabled: true })
    });
    assert!(restore_enabled);

    fixture.update_picker(|picker, window, cx| {
        picker.delegate.restore_popup_size(cx);
        picker.refresh(window, cx);
    });
    let restored = fixture
        .popup
        .read_with(&fixture.cx, |popup, cx| popup.popup_size(cx));
    assert_eq!(restored, packed);
}

#[gpui::test]
async fn the_drag_area_moves_the_popup(cx: &mut TestAppContext) {
    let mut fixture = Fixture::open(cx, path!("/branches-popup-move"), standard_repository).await;
    fixture.open_in_modal_layer();
    let before = bounds_of(&mut fixture.cx, DEBUG_SELECTOR.to_string());
    let drag_area = bounds_of(&mut fixture.cx, "branches-popup-drag-area".to_string());
    drag(
        &mut fixture.cx,
        drag_area.center(),
        gpui::point(px(30.), px(20.)),
    );
    let offset = fixture
        .popup
        .read_with(&fixture.cx, |popup, _| popup.move_offset());
    assert_eq!(offset, gpui::point(px(30.), px(20.)));
    let after = bounds_of(&mut fixture.cx, DEBUG_SELECTOR.to_string());
    assert_eq!(after.origin, before.origin + offset);
}

#[gpui::test]
async fn a_ref_menu_opens_two_pixels_inside_the_popup_right_edge(cx: &mut TestAppContext) {
    let mut fixture =
        Fixture::open(cx, path!("/branches-popup-submenu"), standard_repository).await;
    fixture.open_in_modal_layer();
    let index = fixture.index_of("  ref:topic");
    let popup = bounds_of(&mut fixture.cx, DEBUG_SELECTOR.to_string());
    let arrow = bounds_of(&mut fixture.cx, format!("branches-popup-ref-arrow-{index}"));
    let offset = delegate::ref_menu_offset();
    assert_eq!(arrow.right() + offset.x, popup.right() - px(2.));
}

#[gpui::test]
async fn long_branch_names_are_truncated_inside_a_narrow_popup(cx: &mut TestAppContext) {
    let long_name = format!("feature-{}", "very-long-branch-name-".repeat(8));
    let mut fixture = Fixture::open(cx, path!("/branches-popup-truncate"), |fs, dot_git| {
        fs.insert_branches(dot_git, &["main", &long_name]);
        fs.set_recent_branches_for_repo(dot_git, &[long_name.as_str(), "main"]);
    })
    .await;
    fixture.open_in_modal_layer();
    fixture.update_picker(|picker, _, cx| {
        picker.delegate.set_user_size(
            state::PopupSize {
                width: MIN_WIDTH_PX as u32,
                height: 400,
            },
            cx,
        );
        cx.notify();
    });
    let index = fixture.index_of(&format!("  ref:{long_name}"));
    let popup = bounds_of(&mut fixture.cx, DEBUG_SELECTOR.to_string());
    let arrow = bounds_of(&mut fixture.cx, format!("branches-popup-ref-arrow-{index}"));
    assert!(arrow.right() <= popup.right(), "{arrow:?} {popup:?}");
    let label = bounds_of(&mut fixture.cx, format!("branches-popup-ref-label-{index}"));
    assert!(label.right() <= arrow.left(), "{label:?} {arrow:?}");
}

#[test]
fn the_gear_menu_follows_intellij_for_one_and_several_repositories() {
    let options = |several_repositories, user_resized| delegate::GearMenuOptions {
        toggles: state::PopupToggles::default(),
        several_repositories,
        user_resized,
    };
    let labels = |items: Vec<delegate::GearMenuItem>| -> Vec<String> {
        items
            .into_iter()
            .map(|item| match item {
                delegate::GearMenuItem::Header(title) => format!("# {title}"),
                delegate::GearMenuItem::Toggle { label, .. } => label.to_string(),
                delegate::GearMenuItem::RestorePopupSize { enabled } => {
                    format!("Restore Popup Size ({enabled})")
                }
                delegate::GearMenuItem::Separator => "---".to_string(),
            })
            .collect()
    };
    assert_eq!(
        labels(delegate::gear_menu_items(options(false, false))),
        vec![
            "Show Actions in Search Results",
            "---",
            "Group by Directory",
            "Show Recent Branches",
            "Show Tags",
            "Restore Popup Size (false)",
        ]
    );
    assert_eq!(
        labels(delegate::gear_menu_items(options(true, true))),
        vec![
            "# Add in Search Results",
            "Actions",
            "Repositories",
            "---",
            "Group by Directory",
            "Show Recent Branches",
            "Show Tags",
            "Restore Popup Size (true)",
        ]
    );
}

#[test]
fn the_selected_ref_offers_a_favorite_with_a_hollow_star() {
    assert_eq!(
        delegate::ref_row_icon(true, false, false, true),
        IconName::FavoriteOutline
    );
    assert_eq!(
        delegate::ref_row_icon(false, false, true, true),
        IconName::FavoriteOutline
    );
    assert_eq!(
        delegate::ref_row_icon(true, true, false, true),
        IconName::CurrentBranchFavoriteLabel
    );
    assert_eq!(
        delegate::ref_row_icon(true, false, false, false),
        IconName::CurrentBranchLabel
    );
}

#[test]
fn row_icons_are_the_sixteen_pixel_intellij_size() {
    assert!(delegate::ROW_ICON_SIZE == IconSize::Medium);
}

#[gpui::test]
fn repositories_take_the_intellij_new_ui_colors_in_name_order(cx: &mut TestAppContext) {
    init_test(cx);
    cx.update(|cx| {
        let light = cx.theme().appearance().is_light();
        let expected = |light_color: u32, dark_color: u32| -> gpui::Hsla {
            gpui::rgb(if light { light_color } else { dark_color }).into()
        };
        assert_eq!(
            delegate::repository_color(0, cx),
            expected(0xFF6285, 0x865762)
        );
        assert_eq!(
            delegate::repository_color(1, cx),
            expected(0x89C398, 0x436946)
        );
        assert_eq!(
            delegate::repository_color(14, cx),
            expected(0xFF6285, 0x865762)
        );
    });
}

#[gpui::test]
fn incoming_and_outgoing_counters_use_the_intellij_colors(cx: &mut TestAppContext) {
    init_test(cx);
    cx.update(|cx| {
        let light = cx.theme().appearance().is_light();
        let (incoming, outgoing) = delegate::commit_counter_colors(cx);
        let expected = |light_color: u32, dark_color: u32| -> gpui::Hsla {
            gpui::rgb(if light { light_color } else { dark_color }).into()
        };
        assert_eq!(incoming, expected(0x3574F0, 0x548AF7));
        assert_eq!(outgoing, expected(0x369650, 0x57965C));
    });
}

#[gpui::test]
async fn a_top_action_shortcut_closes_the_popup_and_runs_the_action(cx: &mut TestAppContext) {
    let mut fixture =
        Fixture::open(cx, path!("/branches-popup-shortcut"), standard_repository).await;
    fixture.open_in_modal_layer();
    let dismissed = Rc::new(Cell::new(false));
    let _subscription = fixture.cx.update(|_, cx| {
        let dismissed = dismissed.clone();
        cx.subscribe(&fixture.popup, move |_, _: &DismissEvent, _| {
            dismissed.set(true)
        })
    });
    fixture.cx.dispatch_action(git::NewBranch);
    fixture.cx.run_until_parked();
    assert!(dismissed.get());
    let popup_open = fixture.workspace.read_with(&fixture.cx, |workspace, cx| {
        workspace.active_modal::<BranchesPopup>(cx).is_some()
    });
    assert!(!popup_open);
    let dialog_open = fixture
        .workspace
        .update_in(&mut fixture.cx, |workspace, window, cx| {
            workspace.has_active_modal(window, cx)
        });
    assert!(dialog_open, "New Branch… opens its dialog");
}

#[gpui::test]
async fn delete_ref_dispatched_inside_the_popup_deletes_the_ref_of_the_open_menu(
    cx: &mut TestAppContext,
) {
    let mut fixture = Fixture::open(
        cx,
        path!("/branches-popup-delete-menu"),
        standard_repository,
    )
    .await;
    fixture.open_in_modal_layer();
    fixture.select("  ref:topic");
    fixture.update_picker(|picker, window, cx| picker.delegate.confirm(false, window, cx));
    let menu_open = fixture.picker().read_with(&fixture.cx, |picker, cx| {
        picker.delegate.has_another_open_menu_for_tests(cx)
    });
    assert!(menu_open, "Enter opens the ref menu");

    // The ref menu re-dispatches its `DeleteRef` entry inside the popup, like this.
    fixture.cx.dispatch_action(DeleteRef);
    fixture.cx.run_until_parked();
    let branches = fixture.repository.read_with(&fixture.cx, |repository, _| {
        repository
            .branch_list
            .iter()
            .map(|branch| branch.name().to_string())
            .collect::<Vec<_>>()
    });
    assert!(!branches.contains(&"topic".to_string()), "{branches:?}");
}

#[gpui::test]
async fn delete_ref_deletes_the_ref_of_the_open_menu_after_the_selection_moved(
    cx: &mut TestAppContext,
) {
    let mut fixture = Fixture::open(
        cx,
        path!("/branches-popup-delete-moved"),
        standard_repository,
    )
    .await;
    fixture.open_in_modal_layer();
    fixture.select("  ref:topic");
    fixture.update_picker(|picker, window, cx| picker.delegate.confirm(false, window, cx));
    let main_index = fixture.index_of("  ref:main");
    fixture.update_picker(|picker, window, cx| {
        picker.set_selected_index(main_index, None, false, window, cx)
    });
    assert_eq!(fixture.selected().as_deref(), Some("  ref:main"));
    let menu_open = fixture.picker().read_with(&fixture.cx, |picker, cx| {
        picker.delegate.has_another_open_menu_for_tests(cx)
    });
    assert!(menu_open, "hovering another row keeps the ref menu open");

    fixture.cx.dispatch_action(DeleteRef);
    fixture.cx.run_until_parked();
    let branches = fixture.repository.read_with(&fixture.cx, |repository, _| {
        repository
            .branch_list
            .iter()
            .map(|branch| branch.name().to_string())
            .collect::<Vec<_>>()
    });
    assert!(!branches.contains(&"topic".to_string()), "{branches:?}");
    assert!(branches.contains(&"main".to_string()), "{branches:?}");
}

#[gpui::test]
async fn branch_rows_are_intellijs_whole_24_pixel_rows(cx: &mut TestAppContext) {
    let mut fixture =
        Fixture::open(cx, path!("/branches-popup-row-height"), standard_repository).await;
    fixture.open_in_modal_layer();
    fixture.select("section:Local:closed");
    fixture.select_child();
    let rows = fixture.rows();
    let first_ref = rows
        .iter()
        .zip(rows.iter().skip(1))
        .position(|(row, next)| row.contains("ref:") && next.contains("ref:"))
        .unwrap_or_else(|| panic!("two branch rows follow each other: {rows:#?}"));

    let first = bounds_of(
        &mut fixture.cx,
        format!("branches-popup-ref-label-{first_ref}"),
    );
    let second = bounds_of(
        &mut fixture.cx,
        format!("branches-popup-ref-label-{}", first_ref + 1),
    );
    assert_eq!(
        second.top() - first.top(),
        px(delegate::ROW_HEIGHT_PX),
        "rows are 24 px like IntelliJ's trees, not 24/16 of the rem"
    );
}
