use anyhow::{Result, anyhow};
use async_trait::async_trait;
use gpui::AsyncApp;
use language::{LspAdapter, LspAdapterDelegate, LspInstaller, Toolchain};
use lsp::{CodeActionKind, LanguageServerBinary, LanguageServerName, Uri};
use node_runtime::NodeRuntime;
use project::Fs;
use project::lsp_store::language_server_settings_for;
use serde_json::{Value, json};
use settings::SettingsLocation;
use std::{
    ffi::OsString,
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
};
use util::{ResultExt as _, rel_path::RelPath};

const SERVER_NAME: LanguageServerName = LanguageServerName::new_static("oxlint");
/// The `oxlint` npm package's launcher, a Node script that loads oxlint's native binding.
const PROJECT_LAUNCHER: &str = "node_modules/oxlint/bin/oxlint";
/// The section oxlint's server asks for in `workspace/configuration`.
const CONFIGURATION_SECTION: &str = "oxc_language_server";
const FIX_ALL_CODE_ACTION: &str = "source.fixAll.oxc";
const NOT_INSTALLED: &str =
    "oxlint is not installed in this project; add it with `npm install --save-dev oxlint`";

/// Runs the project's own `oxlint --lsp`, as Oxc's VS Code and JetBrains integrations do, so
/// files are linted with the oxlint version, plugins and config the project pins.
pub struct OxlintLspAdapter {
    node: NodeRuntime,
    fs: Arc<dyn Fs>,
}

impl OxlintLspAdapter {
    pub fn new(node: NodeRuntime, fs: Arc<dyn Fs>) -> Self {
        Self { node, fs }
    }
}

/// Finds oxlint the way Node resolves a package: in the worktree's `node_modules` or the
/// nearest ancestor's, which covers a package opened inside a monorepo.
async fn find_project_launcher(fs: &dyn Fs, worktree_root: &Path) -> Option<PathBuf> {
    for directory in worktree_root.ancestors() {
        let launcher = directory.join(PROJECT_LAUNCHER);
        if fs.is_file(&launcher).await {
            return Some(launcher);
        }
    }
    None
}

fn server_binary(node: PathBuf, launcher: &Path) -> LanguageServerBinary {
    LanguageServerBinary {
        path: node,
        env: None,
        arguments: vec![OsString::from(launcher), OsString::from("--lsp")],
    }
}

/// The user's `lsp.oxlint.settings` are oxlint's per-workspace options (`run`, `configPath`,
/// `fixKind`, `typeAware`, …); without them oxlint uses its defaults and the project's config.
fn workspace_configuration_for(user_settings: Option<Value>) -> Value {
    json!({ CONFIGURATION_SECTION: user_settings.unwrap_or_else(|| json!({})) })
}

impl LspInstaller for OxlintLspAdapter {
    type BinaryVersion = ();

    async fn check_if_user_installed(
        &self,
        delegate: &Arc<dyn LspAdapterDelegate>,
        _: Option<Toolchain>,
        _: &AsyncApp,
    ) -> Option<LanguageServerBinary> {
        let launcher =
            find_project_launcher(self.fs.as_ref(), delegate.worktree_root_path()).await?;
        let node = self.node.binary_path().await.log_err()?;
        Some(server_binary(node, &launcher))
    }

    async fn fetch_latest_server_version(
        &self,
        _: &Arc<dyn LspAdapterDelegate>,
        _: bool,
        _: &mut AsyncApp,
    ) -> Result<Self::BinaryVersion> {
        Err(anyhow!(NOT_INSTALLED))
    }

    fn fetch_server_binary(
        &self,
        _: Self::BinaryVersion,
        _: PathBuf,
        _: &Arc<dyn LspAdapterDelegate>,
    ) -> impl Send + Future<Output = Result<LanguageServerBinary>> + use<> {
        futures::future::ready(Err(anyhow!(NOT_INSTALLED)))
    }

    async fn cached_server_binary(
        &self,
        _: PathBuf,
        _: &dyn LspAdapterDelegate,
    ) -> Option<LanguageServerBinary> {
        None
    }
}

#[async_trait(?Send)]
impl LspAdapter for OxlintLspAdapter {
    fn name(&self) -> LanguageServerName {
        SERVER_NAME
    }

    fn code_action_kinds(&self) -> Option<Vec<CodeActionKind>> {
        Some(vec![
            CodeActionKind::QUICKFIX,
            CodeActionKind::new(FIX_ALL_CODE_ACTION),
        ])
    }

    async fn workspace_configuration(
        self: Arc<Self>,
        delegate: &Arc<dyn LspAdapterDelegate>,
        _: Option<Toolchain>,
        requested_uri: Option<Uri>,
        cx: &mut AsyncApp,
    ) -> Result<Value> {
        let worktree_root = delegate.worktree_root_path();
        let file_path = requested_uri
            .filter(|uri| uri.scheme() == "file")
            .and_then(|uri| uri.to_file_path().ok())
            .and_then(|path| {
                path.strip_prefix(worktree_root)
                    .ok()
                    .and_then(|path| RelPath::from_unix_str(path).ok().map(ToOwned::to_owned))
            })
            .unwrap_or_else(|| RelPath::empty().to_owned());
        let user_settings = cx.update(|cx| {
            language_server_settings_for(
                SettingsLocation {
                    worktree_id: delegate.worktree_id(),
                    path: &file_path,
                },
                &SERVER_NAME,
                cx,
            )
            .and_then(|settings| settings.settings.clone())
        });
        Ok(workspace_configuration_for(user_settings))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::fs::FakeFs;
    use gpui::TestAppContext;
    use util::path;

    #[test]
    fn the_server_is_the_projects_oxlint_run_by_node_with_the_lsp_flag() {
        let binary = server_binary(
            PathBuf::from("/node/bin/node"),
            Path::new("/repo/node_modules/oxlint/bin/oxlint"),
        );
        assert_eq!(binary.path, PathBuf::from("/node/bin/node"));
        assert_eq!(
            binary.arguments,
            vec![
                OsString::from("/repo/node_modules/oxlint/bin/oxlint"),
                OsString::from("--lsp")
            ]
        );
    }

    #[gpui::test]
    async fn the_launcher_is_found_like_node_resolves_packages(cx: &mut TestAppContext) {
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            path!("/repo"),
            json!({
                "node_modules": { "oxlint": { "bin": { "oxlint": "" } } },
                "packages": {
                    "web": { "src": { "index.ts": "" } },
                    "api": {
                        "node_modules": { "oxlint": { "bin": { "oxlint": "" } } },
                    },
                },
            }),
        )
        .await;

        assert_eq!(
            find_project_launcher(fs.as_ref(), Path::new(path!("/repo"))).await,
            Some(PathBuf::from(path!("/repo/node_modules/oxlint/bin/oxlint"))),
            "the worktree's own oxlint"
        );
        assert_eq!(
            find_project_launcher(fs.as_ref(), Path::new(path!("/repo/packages/web"))).await,
            Some(PathBuf::from(path!("/repo/node_modules/oxlint/bin/oxlint"))),
            "a package of a monorepo uses the root's oxlint"
        );
        assert_eq!(
            find_project_launcher(fs.as_ref(), Path::new(path!("/repo/packages/api"))).await,
            Some(PathBuf::from(path!(
                "/repo/packages/api/node_modules/oxlint/bin/oxlint"
            ))),
            "the nearest oxlint wins, as for Node"
        );
    }

    #[gpui::test]
    async fn a_project_without_oxlint_has_no_launcher(cx: &mut TestAppContext) {
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            path!("/repo"),
            json!({ ".oxlintrc.json": "{}", "node_modules": { "eslint": {} } }),
        )
        .await;
        assert_eq!(
            find_project_launcher(fs.as_ref(), Path::new(path!("/repo"))).await,
            None
        );
    }

    #[test]
    fn workspace_configuration_answers_the_oxc_language_server_section() {
        assert_eq!(
            workspace_configuration_for(None),
            json!({ "oxc_language_server": {} }),
            "without user settings oxlint keeps its defaults and the project's config"
        );
        assert_eq!(
            workspace_configuration_for(Some(json!({ "run": "onSave", "typeAware": true }))),
            json!({ "oxc_language_server": { "run": "onSave", "typeAware": true } })
        );
    }

    #[gpui::test]
    fn oxlint_offers_quick_fixes_and_fix_all(cx: &mut TestAppContext) {
        let adapter = OxlintLspAdapter::new(NodeRuntime::unavailable(), FakeFs::new(cx.executor()));
        let kinds = adapter.code_action_kinds().unwrap_or_default();
        assert!(kinds.contains(&CodeActionKind::QUICKFIX));
        assert!(kinds.contains(&CodeActionKind::new("source.fixAll.oxc")));
    }
}
