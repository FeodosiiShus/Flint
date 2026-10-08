use crate::{branch_diff::BranchDiff, branch_refs::RefTarget};
use gpui::{AppContext as _, Context, Entity, SharedString, Window};
use project::git_store::{Repository, diff_buffer_list::DiffBase};
use workspace::{Workspace, notifications::NotifyTaskExt};

const REF_PREFIXES: [&str; 3] = ["refs/heads/", "refs/remotes/", "refs/tags/"];

pub fn working_tree_base_revision(target: &RefTarget) -> SharedString {
    target.full_ref_name().into()
}

pub fn display_ref_name(revision: &str) -> &str {
    REF_PREFIXES
        .iter()
        .find_map(|prefix| revision.strip_prefix(prefix))
        .unwrap_or(revision)
}

pub fn working_tree_diff_title(base_ref: &str) -> String {
    format!(
        "Changes Between {} and Current Working Tree",
        display_ref_name(base_ref)
    )
}

pub fn open_working_tree_diff(
    workspace: &mut Workspace,
    repository: Entity<Repository>,
    base_ref: SharedString,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let existing = workspace.items_of_type::<BranchDiff>(cx).find(|item| {
        let item = item.read(cx);
        matches!(
            item.diff_base(cx),
            DiffBase::WorkingTree { base_ref: existing_base_ref } if existing_base_ref == &base_ref
        ) && item
            .repo(cx)
            .is_some_and(|repo| repo.read(cx).id == repository.read(cx).id)
    });
    if let Some(existing) = existing {
        workspace.activate_item(&existing, true, true, window, cx);
        return;
    }

    let project = workspace.project().clone();
    let workspace_entity = cx.entity();
    let workspace_weak = workspace_entity.downgrade();
    window
        .spawn(cx, async move |cx| {
            let item = cx.update(|window, cx| {
                cx.new(|cx| {
                    BranchDiff::new_with_diff_base(
                        project,
                        workspace_entity.clone(),
                        DiffBase::WorkingTree { base_ref },
                        Some(repository),
                        None,
                        window,
                        cx,
                    )
                })
            })?;
            workspace_entity
                .update_in(cx, |workspace, window, cx| {
                    workspace.add_item_to_active_pane(Box::new(item), None, true, window, cx);
                })
                .ok();
            anyhow::Ok(())
        })
        .detach_and_notify_err(workspace_weak, window, cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use git::repository::repo_path;
    use gpui::TestAppContext;
    use project::{FakeFs, Project, git_store::diff_buffer_list::DiffBufferList};
    use serde_json::json;
    use settings::SettingsStore;
    use std::path::Path;
    use util::path;

    #[test]
    fn display_ref_name_strips_the_namespace_of_full_ref_names() {
        assert_eq!(display_ref_name("refs/heads/feature/x"), "feature/x");
        assert_eq!(display_ref_name("refs/remotes/origin/main"), "origin/main");
        assert_eq!(display_ref_name("refs/tags/v1.0"), "v1.0");
    }

    #[test]
    fn display_ref_name_keeps_plain_revisions_untouched() {
        assert_eq!(display_ref_name("HEAD"), "HEAD");
        assert_eq!(display_ref_name("main"), "main");
    }

    #[test]
    fn working_tree_diff_title_names_the_ref_and_the_working_tree() {
        assert_eq!(
            working_tree_diff_title("refs/heads/main"),
            "Changes Between main and Current Working Tree"
        );
    }

    #[test]
    fn working_tree_base_revision_uses_the_unambiguous_full_ref_name() {
        assert_eq!(
            working_tree_base_revision(&RefTarget::local("main")).as_ref(),
            "refs/heads/main"
        );
        assert_eq!(
            working_tree_base_revision(&RefTarget::remote("origin/main")).as_ref(),
            "refs/remotes/origin/main"
        );
        assert_eq!(
            working_tree_base_revision(&RefTarget::tag("v1")).as_ref(),
            "refs/tags/v1"
        );
    }

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            crate::init(cx);
        });
    }

    #[gpui::test]
    async fn working_tree_base_lists_ref_differences_and_local_modifications(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            path!("/project"),
            json!({
                ".git": {},
                "a.txt": "C",
                "d.txt": "created-in-head",
                "e.txt": "E2",
            }),
        )
        .await;
        fs.set_head_and_index_for_repo(
            Path::new(path!("/project/.git")),
            &[
                ("a.txt", "B".into()),
                ("d.txt", "created-in-head".into()),
                ("e.txt", "E".into()),
            ],
        );
        let ref_oids = fs.set_merge_base_content_for_repo(
            Path::new(path!("/project/.git")),
            &[("a.txt", "A".into()), ("e.txt", "E".into())],
        );

        let project = Project::test(fs.clone(), [path!("/project").as_ref()], cx).await;
        cx.run_until_parked();
        let (git_store, repository) = cx.update(|cx| {
            let project = project.read(cx);
            (
                project.git_store().clone(),
                project.active_repository(cx).unwrap(),
            )
        });
        let list = cx.update(|cx| {
            cx.new(|cx| {
                DiffBufferList::new(
                    DiffBase::WorkingTree {
                        base_ref: "refs/heads/topic".into(),
                    },
                    git_store,
                    Some(repository),
                    cx,
                )
            })
        });
        cx.run_until_parked();

        let buffers = list.update(cx, |list, cx| list.load_buffers(cx));
        let paths = buffers
            .iter()
            .map(|buffer| buffer.repo_path.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![repo_path("a.txt"), repo_path("d.txt"), repo_path("e.txt")],
            "ref differences and local modifications must both be listed"
        );

        list.read_with(cx, |list, _| {
            assert_eq!(
                list.base_oid_for_path(&repo_path("a.txt")),
                Some(Some(ref_oids[0])),
                "a file that differs between the ref and HEAD diffs against the ref blob"
            );
            assert_eq!(
                list.base_oid_for_path(&repo_path("d.txt")),
                Some(None),
                "a file missing from the ref diffs against empty content"
            );
            assert_eq!(
                list.base_oid_for_path(&repo_path("e.txt")),
                None,
                "a file identical in the ref and HEAD diffs against HEAD"
            );
        });
    }
}
