use super::{
    BranchContext,
    compare_view::{CompareRefs, CompareView},
    ref_diff,
};
use crate::branch_refs::RefTarget;
use gpui::{App, AppContext as _, Context, Entity, Window};
use project::git_store::Repository;
use util::ResultExt as _;
use workspace::Workspace;

pub fn compare_with_current(
    context: BranchContext,
    target: RefTarget,
    window: &mut Window,
    cx: &mut App,
) {
    let current_branch = context.current_branch_name(cx);
    let refs = CompareRefs::new(&target, current_branch.as_deref());
    let repository = context.repository.clone();
    let saving = context.save_documents(cx);
    let workspace = context.workspace;
    window
        .spawn(cx, async move |cx| {
            saving.await;
            workspace
                .update_in(cx, |workspace, window, cx| {
                    open_compare_view(workspace, refs, repository, window, cx);
                })
                .log_err();
        })
        .detach();
}

pub fn show_diff_with_working_tree(
    context: BranchContext,
    target: RefTarget,
    window: &mut Window,
    cx: &mut App,
) {
    let base_ref = ref_diff::working_tree_base_revision(&target);
    let repository = context.repository.clone();
    let saving = context.save_documents(cx);
    let workspace = context.workspace;
    window
        .spawn(cx, async move |cx| {
            saving.await;
            workspace
                .update_in(cx, |workspace, window, cx| {
                    ref_diff::open_working_tree_diff(workspace, repository, base_ref, window, cx);
                })
                .log_err();
        })
        .detach();
}

fn open_compare_view(
    workspace: &mut Workspace,
    refs: CompareRefs,
    repository: Entity<Repository>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let existing = workspace.items_of_type::<CompareView>(cx).find(|item| {
        let view = item.read(cx);
        view.refs() == &refs && view.repository().read(cx).id == repository.read(cx).id
    });
    if let Some(existing) = existing {
        existing.update(cx, |view, cx| view.reload(cx));
        workspace.activate_item(&existing, true, true, window, cx);
        return;
    }

    let workspace_handle = workspace.weak_handle();
    let view = cx.new(|cx| CompareView::new(refs, repository, workspace_handle, window, cx));
    workspace.add_item_to_active_pane(Box::new(view), None, true, window, cx);
}
