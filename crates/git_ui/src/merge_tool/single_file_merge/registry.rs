use std::collections::HashMap;

use git::repository::RepoPath;
use gpui::{App, Global, WindowHandle};
use project::git_store::RepositoryId;
use util::ResultExt as _;

use crate::merge_tool::dialog_window::DialogWindowShell;

#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct MergeKey {
    repository: RepositoryId,
    path: RepoPath,
}

impl MergeKey {
    pub(super) fn new(repository: RepositoryId, path: RepoPath) -> Self {
        Self { repository, path }
    }
}

#[derive(Clone)]
enum WindowState {
    Loading,
    Open(WindowHandle<DialogWindowShell>),
}

#[derive(Default)]
pub(super) struct OpenMergeWindows {
    windows: HashMap<MergeKey, WindowState>,
}

impl Global for OpenMergeWindows {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Claim {
    Claimed,
    AlreadyLoading,
    Activated,
}

pub(super) fn claim(key: &MergeKey, cx: &mut App) -> Claim {
    let state = cx
        .default_global::<OpenMergeWindows>()
        .windows
        .get(key)
        .cloned();
    match state {
        None => {
            insert(key, WindowState::Loading, cx);
            Claim::Claimed
        }
        Some(WindowState::Loading) => Claim::AlreadyLoading,
        Some(WindowState::Open(window)) => {
            if window
                .update(cx, |_, window, _| window.activate_window())
                .log_err()
                .is_some()
            {
                Claim::Activated
            } else {
                insert(key, WindowState::Loading, cx);
                Claim::Claimed
            }
        }
    }
}

pub(super) fn mark_open(key: &MergeKey, window: WindowHandle<DialogWindowShell>, cx: &mut App) {
    insert(key, WindowState::Open(window), cx);
}

pub(super) fn release(key: &MergeKey, cx: &mut App) {
    cx.default_global::<OpenMergeWindows>().windows.remove(key);
}

fn insert(key: &MergeKey, state: WindowState, cx: &mut App) {
    cx.default_global::<OpenMergeWindows>()
        .windows
        .insert(key.clone(), state);
}

pub(super) fn open_window(key: &MergeKey, cx: &App) -> Option<WindowHandle<DialogWindowShell>> {
    match cx.try_global::<OpenMergeWindows>()?.windows.get(key) {
        Some(WindowState::Open(window)) => Some(*window),
        Some(WindowState::Loading) | None => None,
    }
}
