use anyhow::{Result, anyhow};
use async_trait::async_trait;
use gpui::AsyncApp;
use language::{LspAdapter, LspAdapterDelegate, LspInstaller, Toolchain};
use lsp::{CodeActionKind, LanguageServerBinary, LanguageServerName, Uri};
use node_runtime::NodeRuntime;
use project::Fs;
use project::lsp_store::language_server_settings_for;
use serde_json::{Map, Value};
use settings::SettingsLocation;
use std::{
    ffi::{OsStr, OsString},
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
};
use util::{ResultExt as _, rel_path::RelPath};

const SERVER_NAME: LanguageServerName = LanguageServerName::new_static("biome");
/// The `@biomejs/biome` npm package's launcher, a Node script that spawns the platform binary.
const PROJECT_LAUNCHER: &str = "node_modules/@biomejs/biome/bin/biome";
/// The section Biome's server asks for in `workspace/configuration`.
const CONFIGURATION_SECTION: &str = "biome";
const FIX_ALL_CODE_ACTION: &str = "source.fixAll.biome";
const ORGANIZE_IMPORTS_CODE_ACTION: &str = "source.organizeImports.biome";
const NOT_INSTALLED: &str =
    "biome is not installed in this project; add it with `npm install --save-dev @biomejs/biome`";

/// Runs the project's own `biome lsp-proxy`, as Biome's own editor integrations do, so files are
/// linted and formatted with the Biome version and configuration the project pins.
pub struct BiomeLspAdapter {
    node: NodeRuntime,
    fs: Arc<dyn Fs>,
}

impl BiomeLspAdapter {
    pub fn new(node: NodeRuntime, fs: Arc<dyn Fs>) -> Self {
        Self { node, fs }
    }
}

/// Finds Biome the way Node resolves a package: in the worktree's `node_modules` or the nearest
/// ancestor's, which covers a package opened inside a monorepo.
async fn find_project_launcher(fs: &dyn Fs, worktree_root: &Path) -> Option<PathBuf> {
    for directory in worktree_root.ancestors() {
        let launcher = directory.join(PROJECT_LAUNCHER);
        if fs.is_file(&launcher).await {
            return Some(launcher);
        }
    }
    None
}

fn node_server_binary(node: PathBuf, launcher: &Path) -> LanguageServerBinary {
    LanguageServerBinary {
        path: node,
        env: None,
        arguments: vec![OsString::from(launcher), OsString::from("lsp-proxy")],
    }
}

fn path_server_binary(binary: PathBuf) -> LanguageServerBinary {
    LanguageServerBinary {
        path: binary,
        env: None,
        arguments: vec![OsString::from("lsp-proxy")],
    }
}

/// Biome's server reads camelCase settings: `requireConfiguration`, `configurationPath`,
/// `inlineConfig` and `goToDefinition`. The Biome Zed extension exposes the first three under
/// the names below, so both spellings are accepted here.
fn server_setting_name(user_name: &str) -> &str {
    match user_name {
        "require_config_file" => "requireConfiguration",
        "config_path" | "configuration_path" => "configurationPath",
        "inline_config" => "inlineConfig",
        "goto_definition" | "go_to_definition" => "goToDefinition",
        _ => user_name,
    }
}

/// No `configurationPath` is sent unless the user sets one: an explicit path replaces Biome's
/// own search, which walks up from the opened folder past nested `"root": false` configurations
/// to the monorepo's root configuration.
fn workspace_configuration_for(user_settings: Option<Value>) -> Value {
    let mut settings = Map::new();
    if let Some(Value::Object(user_settings)) = user_settings {
        for (name, value) in user_settings {
            settings.insert(server_setting_name(&name).to_string(), value);
        }
    }
    // `workspace/configuration` reads the `biome` section, and so does
    // `workspace/didChangeConfiguration` since Biome 2.4.9; the 2.4 releases before it parse
    // the notification's settings as flat options, so both shapes are sent.
    let mut configuration = settings.clone();
    configuration.insert(CONFIGURATION_SECTION.to_string(), Value::Object(settings));
    Value::Object(configuration)
}

impl LspInstaller for BiomeLspAdapter {
    type BinaryVersion = ();

    async fn check_if_user_installed(
        &self,
        delegate: &Arc<dyn LspAdapterDelegate>,
        _: Option<Toolchain>,
        _: &AsyncApp,
    ) -> Option<LanguageServerBinary> {
        if let Some(launcher) =
            find_project_launcher(self.fs.as_ref(), delegate.worktree_root_path()).await
            && let Some(node) = self.node.binary_path().await.log_err()
        {
            return Some(node_server_binary(node, &launcher));
        }

        let binary = delegate.which(OsStr::new("biome")).await?;
        Some(path_server_binary(binary))
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
impl LspAdapter for BiomeLspAdapter {
    fn name(&self) -> LanguageServerName {
        SERVER_NAME
    }

    fn code_action_kinds(&self) -> Option<Vec<CodeActionKind>> {
        Some(vec![
            CodeActionKind::QUICKFIX,
            CodeActionKind::new(FIX_ALL_CODE_ACTION),
            CodeActionKind::new(ORGANIZE_IMPORTS_CODE_ACTION),
        ])
    }

    fn formats_projects_without_prettier(&self) -> bool {
        true
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
    use serde_json::json;
    use util::path;

    #[test]
    fn the_server_is_the_projects_biome_run_by_node_with_the_lsp_proxy_command() {
        let binary = node_server_binary(
            PathBuf::from("/node/bin/node"),
            Path::new("/repo/node_modules/@biomejs/biome/bin/biome"),
        );
        assert_eq!(binary.path, PathBuf::from("/node/bin/node"));
        assert_eq!(
            binary.arguments,
            vec![
                OsString::from("/repo/node_modules/@biomejs/biome/bin/biome"),
                OsString::from("lsp-proxy")
            ]
        );
    }

    #[test]
    fn a_system_biome_runs_with_the_lsp_proxy_command() {
        let binary = path_server_binary(PathBuf::from("/usr/local/bin/biome"));
        assert_eq!(binary.path, PathBuf::from("/usr/local/bin/biome"));
        assert_eq!(binary.arguments, vec![OsString::from("lsp-proxy")]);
    }

    #[gpui::test]
    async fn the_launcher_is_found_like_node_resolves_packages(cx: &mut TestAppContext) {
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            path!("/repo"),
            json!({
                "node_modules": {
                    "@biomejs": { "biome": { "bin": { "biome": "" } } }
                },
                "packages": {
                    "web": { "src": { "index.ts": "" } },
                    "api": {
                        "node_modules": {
                            "@biomejs": { "biome": { "bin": { "biome": "" } } }
                        },
                    },
                },
            }),
        )
        .await;

        assert_eq!(
            find_project_launcher(fs.as_ref(), Path::new(path!("/repo"))).await,
            Some(PathBuf::from(path!(
                "/repo/node_modules/@biomejs/biome/bin/biome"
            ))),
            "the worktree's own biome"
        );
        assert_eq!(
            find_project_launcher(fs.as_ref(), Path::new(path!("/repo/packages/web"))).await,
            Some(PathBuf::from(path!(
                "/repo/node_modules/@biomejs/biome/bin/biome"
            ))),
            "a package of a monorepo uses the root's biome"
        );
        assert_eq!(
            find_project_launcher(fs.as_ref(), Path::new(path!("/repo/packages/api"))).await,
            Some(PathBuf::from(path!(
                "/repo/packages/api/node_modules/@biomejs/biome/bin/biome"
            ))),
            "the nearest biome wins, as for Node"
        );
    }

    #[gpui::test]
    async fn a_project_without_biome_has_no_launcher(cx: &mut TestAppContext) {
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            path!("/repo"),
            json!({ "biome.json": "{}", "node_modules": { "oxlint": {} } }),
        )
        .await;
        assert_eq!(
            find_project_launcher(fs.as_ref(), Path::new(path!("/repo"))).await,
            None
        );
    }

    #[test]
    fn workspace_configuration_answers_the_biome_section() {
        assert_eq!(
            workspace_configuration_for(None),
            json!({ "biome": {} }),
            "without user settings no configuration path is sent, so Biome finds a monorepo's \
             root configuration itself"
        );
        assert_eq!(
            workspace_configuration_for(Some(json!({
                "require_config_file": true,
                "config_path": "packages/web/biome.json",
                "inline_config": { "linter": { "rules": { "suspicious": { "noConsole": "off" } } } },
                "goto_definition": true
            })))["biome"],
            json!({
                "requireConfiguration": true,
                "configurationPath": "packages/web/biome.json",
                "inlineConfig": {
                    "linter": { "rules": { "suspicious": { "noConsole": "off" } } }
                },
                "goToDefinition": true
            }),
            "the names the Biome Zed extension documents map to Biome's LSP settings"
        );
    }

    #[test]
    fn workspace_configuration_passes_biomes_camel_case_settings_through() {
        assert_eq!(
            workspace_configuration_for(Some(json!({
                "requireConfiguration": false,
                "inlineConfig": { "formatter": { "lineWidth": 100 } }
            })))["biome"],
            json!({
                "requireConfiguration": false,
                "inlineConfig": { "formatter": { "lineWidth": 100 } }
            })
        );
    }

    #[test]
    fn workspace_configuration_also_carries_the_settings_flat_for_biome_before_2_4_9() {
        assert_eq!(
            workspace_configuration_for(Some(json!({
                "config_path": "biome.base.json",
                "requireConfiguration": true
            }))),
            json!({
                "configurationPath": "biome.base.json",
                "requireConfiguration": true,
                "biome": {
                    "configurationPath": "biome.base.json",
                    "requireConfiguration": true
                }
            })
        );
    }

    #[gpui::test]
    fn biome_offers_quick_fixes_and_fix_all(cx: &mut TestAppContext) {
        let adapter = BiomeLspAdapter::new(NodeRuntime::unavailable(), FakeFs::new(cx.executor()));
        let kinds = adapter.code_action_kinds().unwrap_or_default();
        assert!(kinds.contains(&CodeActionKind::QUICKFIX));
        assert!(kinds.contains(&CodeActionKind::new("source.fixAll.biome")));
        assert!(kinds.contains(&CodeActionKind::new("source.organizeImports.biome")));
    }
}
