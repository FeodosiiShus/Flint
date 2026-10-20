use std::path::{Path, PathBuf};

use anyhow::Result;
use collections::HashSet;
use db::kvp::KeyValueStore;
use gpui::{App, AsyncWindowContext, Entity, EntityId};
use project::project_settings::ProjectSettings;
use serde::{Deserialize, Serialize};
use settings::Settings as _;
use util::ResultExt as _;
use workspace::{Pane, SaveIntent, Workspace};

use super::{BranchContext, BranchNotice};

const KEY_PREFIX: &str = "git_branch_workspace";
const ROLLBACK_LABEL: &str = "Rollback";

/// The editors IntelliJ's `BranchContextTracker` saves per branch: the files
/// open in the editor area, in tab order, and the selected one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchWorkspace {
    pub open_paths: Vec<PathBuf>,
    pub active_path: Option<PathBuf>,
}

/// What a checkout remembers between `branch_will_change` and
/// `branch_has_changed`: the files that were open when it started.
pub struct PendingBranchSwitch {
    open_before: Vec<PathBuf>,
}

pub fn restored_message(branch: &str) -> String {
    format!("Workspace associated with branch '{branch}' has been restored")
}

fn storage_key(repository_path: &Path, branch: &str) -> String {
    format!("{KEY_PREFIX}:{}:{branch}", repository_path.display())
}

fn restore_enabled(cx: &App) -> bool {
    ProjectSettings::get_global(cx)
        .git
        .restore_workspace_on_branch_switch
}

pub fn capture(workspace: &Workspace, cx: &App) -> BranchWorkspace {
    let project = workspace.project().read(cx);
    let mut open_paths = Vec::new();
    for pane in workspace.panes() {
        for item in pane.read(cx).items() {
            let Some(path) = item
                .project_path(cx)
                .and_then(|project_path| project.absolute_path(&project_path, cx))
            else {
                continue;
            };
            if !open_paths.contains(&path) {
                open_paths.push(path);
            }
        }
    }
    let active_path = workspace
        .active_item(cx)
        .and_then(|item| item.project_path(cx))
        .and_then(|project_path| project.absolute_path(&project_path, cx));
    BranchWorkspace {
        open_paths,
        active_path,
    }
}

/// The files opened while the checkout ran: they belong to no saved workspace,
/// so a restore keeps them open like IntelliJ does.
pub fn opened_during_checkout(open_before: &[PathBuf], open_now: &[PathBuf]) -> Vec<PathBuf> {
    open_now
        .iter()
        .filter(|path| !open_before.contains(path))
        .cloned()
        .collect()
}

fn head_name(context: &BranchContext, cx: &App) -> Option<String> {
    let repository = context.repository.read(cx);
    match &repository.branch {
        Some(branch) => Some(branch.name().to_string()),
        None => repository
            .head_commit
            .as_ref()
            .map(|commit| commit.sha.to_string()),
    }
}

/// Saves the open editors under the branch being left, before the checkout.
pub fn branch_will_change(context: &BranchContext, cx: &mut App) -> Option<PendingBranchSwitch> {
    let workspace = context.workspace.upgrade()?;
    let snapshot = capture(workspace.read(cx), cx);
    let open_before = snapshot.open_paths.clone();
    if let Some(branch) = head_name(context, cx) {
        save(context, &branch, &snapshot, cx);
    }
    Some(PendingBranchSwitch { open_before })
}

fn save(context: &BranchContext, branch: &str, snapshot: &BranchWorkspace, cx: &mut App) {
    let key = storage_key(&context.repository.read(cx).work_directory_abs_path, branch);
    let Some(value) = serde_json::to_string(snapshot).log_err() else {
        return;
    };
    let store = KeyValueStore::global(cx);
    db::write_and_log(cx, move || async move { store.write_kvp(key, value).await });
}

fn load(context: &BranchContext, branch: &str, cx: &App) -> Option<BranchWorkspace> {
    let key = storage_key(&context.repository.read(cx).work_directory_abs_path, branch);
    let stored = KeyValueStore::global(cx)
        .read_kvp(&key)
        .log_err()
        .flatten()?;
    serde_json::from_str(&stored).log_err()
}

/// Replaces the open editors with the ones saved for `branch` after a checkout
/// and offers to roll the replacement back.
pub async fn branch_has_changed(
    context: &BranchContext,
    pending: Option<PendingBranchSwitch>,
    branch: &str,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let Some(pending) = pending else {
        return Ok(());
    };
    let Some(saved) = cx.update(|_, app| {
        if restore_enabled(app) {
            load(context, branch, app)
        } else {
            None
        }
    })?
    else {
        return Ok(());
    };
    let Some(workspace) = context.workspace.upgrade() else {
        return Ok(());
    };
    let before_restore = cx.update(|_, app| capture(workspace.read(app), app))?;
    let opened_during = opened_during_checkout(&pending.open_before, &before_restore.open_paths);
    // Like IntelliJ's `BranchContextTracker.switchContext`, a file opened during the checkout
    // keeps the selection when it was the selected one.
    let active_path = before_restore
        .active_path
        .clone()
        .filter(|path| opened_during.contains(path))
        .or_else(|| saved.active_path.clone());
    apply(
        &workspace,
        &saved,
        &opened_during,
        active_path.as_deref(),
        cx,
    )
    .await?;

    let rollback_workspace = workspace.downgrade();
    let notice =
        BranchNotice::info(restored_message(branch)).action(ROLLBACK_LABEL, move |window, cx| {
            let Some(workspace) = rollback_workspace.upgrade() else {
                return;
            };
            let before_restore = before_restore.clone();
            window
                .spawn(cx, async move |cx| {
                    let active_path = before_restore.active_path.clone();
                    apply(&workspace, &before_restore, &[], active_path.as_deref(), cx)
                        .await
                        .log_err();
                })
                .detach();
        });
    cx.update(|_, app| context.notify(notice, app))?;
    Ok(())
}

fn items_to_close(
    workspace: &Workspace,
    keep: &HashSet<PathBuf>,
    cx: &App,
) -> Vec<(Entity<Pane>, EntityId)> {
    let project = workspace.project().read(cx);
    let mut items = Vec::new();
    for pane in workspace.panes() {
        for item in pane.read(cx).items() {
            let Some(path) = item
                .project_path(cx)
                .and_then(|project_path| project.absolute_path(&project_path, cx))
            else {
                continue;
            };
            if !keep.contains(&path) {
                items.push((pane.clone(), item.item_id()));
            }
        }
    }
    items
}

async fn apply(
    workspace: &Entity<Workspace>,
    target: &BranchWorkspace,
    also_open: &[PathBuf],
    active_path: Option<&Path>,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let keep: HashSet<PathBuf> = target.open_paths.iter().chain(also_open).cloned().collect();
    let closing = workspace.update_in(cx, |workspace, window, cx| {
        items_to_close(workspace, &keep, cx)
            .into_iter()
            .map(|(pane, item_id)| {
                pane.update(cx, |pane, cx| {
                    pane.close_item_by_id(item_id, SaveIntent::Close, window, cx)
                })
            })
            .collect::<Vec<_>>()
    })?;
    for close in closing {
        close.await.log_err();
    }

    let to_open = target
        .open_paths
        .iter()
        .chain(also_open)
        .cloned()
        .collect::<Vec<_>>();
    for path in to_open {
        let focus = active_path == Some(path.as_path());
        open(workspace, &path, focus, cx).await;
    }
    if let Some(active) = active_path {
        open(workspace, active, true, cx).await;
    }
    Ok(())
}

async fn open(
    workspace: &Entity<Workspace>,
    path: &Path,
    focus: bool,
    cx: &mut AsyncWindowContext,
) {
    let opening = workspace.update_in(cx, |workspace, window, cx| {
        let project_path = workspace.project().read(cx).find_project_path(path, cx)?;
        Some(workspace.open_path(project_path, None, focus, window, cx))
    });
    match opening {
        Ok(Some(task)) => {
            task.await.log_err();
        }
        Ok(None) => {}
        Err(error) => log::error!("could not restore {}: {error:#}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_restored_notice_quotes_the_branch_like_intellij() {
        assert_eq!(
            restored_message("feature"),
            "Workspace associated with branch 'feature' has been restored"
        );
    }

    #[test]
    fn files_opened_during_the_checkout_are_the_ones_not_open_before() {
        let before = vec![PathBuf::from("/a.rs"), PathBuf::from("/b.rs")];
        let now = vec![
            PathBuf::from("/b.rs"),
            PathBuf::from("/c.rs"),
            PathBuf::from("/a.rs"),
        ];
        assert_eq!(
            opened_during_checkout(&before, &now),
            vec![PathBuf::from("/c.rs")]
        );
        assert!(opened_during_checkout(&now, &before).is_empty());
    }

    #[test]
    fn workspaces_are_stored_per_repository_and_branch() {
        assert_ne!(
            storage_key(Path::new("/one"), "main"),
            storage_key(Path::new("/two"), "main")
        );
        assert_ne!(
            storage_key(Path::new("/one"), "main"),
            storage_key(Path::new("/one"), "feature")
        );
    }
}
