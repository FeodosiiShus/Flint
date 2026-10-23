use anyhow::{Result, anyhow};
use async_trait::async_trait;
use gpui::AsyncApp;
use language::{LspAdapter, LspAdapterDelegate, LspInstaller, Toolchain};
use lsp::{CodeActionKind, LanguageServerBinary, LanguageServerName, Uri};
use node_runtime::NodeRuntime;
use project::Fs;
use project::lsp_store::language_server_settings_for;
use serde_json::{Map, Value, json};
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
const CONFIG_FILES: [&str; 4] = ["biome.json", "biome.jsonc", ".biome.json", ".biome.jsonc"];
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

/// Finds the nearest Biome configuration (`biome.json`, `biome.jsonc`, `.biome.json` or
/// `.biome.jsonc`), so a package opened inside a monorepo uses the configuration the repository
/// keeps above it.
async fn find_config_file(fs: &dyn Fs, worktree_root: &Path) -> Option<PathBuf> {
    for directory in worktree_root.ancestors() {
        for file_name in CONFIG_FILES {
            let config = directory.join(file_name);
            if fs.is_file(&config).await {
                return Some(config);
            }
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

fn workspace_configuration_for(user_settings: Option<Value>, config_file: Option<&Path>) -> Value {
    let mut settings = Map::new();
    if let Some(Value::Object(user_settings)) = user_settings {
        for (name, value) in user_settings {
            settings.insert(server_setting_name(&name).to_string(), value);
        }
    }
    if !settings.contains_key("configurationPath")
        && let Some(config_file) = config_file
    {
        settings.insert(
            "configurationPath".to_string(),
            Value::String(config_file.to_string_lossy().into_owned()),
        );
    }
    json!({ CONFIGURATION_SECTION: settings })
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
        let config_file = find_config_file(self.fs.as_ref(), worktree_root).await;
        Ok(workspace_configuration_for(
            user_settings,
            config_file.as_deref(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::fs::FakeFs;
    use gpui::TestAppContext;
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
    async fn the_configuration_file_is_found_up_the_monorepo(cx: &mut TestAppContext) {
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            path!("/repo"),
            json!({
                "biome.json": "{}",
                "packages": {
                    "web": { "biome.jsonc": "{}" },
                    "ui": { ".biome.json": "{}" },
                    "admin": { "biome.json": "{}", ".biome.json": "{}" },
                    "api": { "src": { "index.ts": "" } },
                },
            }),
        )
        .await;

        assert_eq!(
            find_config_file(fs.as_ref(), Path::new(path!("/repo/packages/api"))).await,
            Some(PathBuf::from(path!("/repo/biome.json"))),
            "a package uses the monorepo's configuration"
        );
        assert_eq!(
            find_config_file(fs.as_ref(), Path::new(path!("/repo/packages/web"))).await,
            Some(PathBuf::from(path!("/repo/packages/web/biome.jsonc"))),
            "the nearest configuration wins"
        );
        assert_eq!(
            find_config_file(fs.as_ref(), Path::new(path!("/repo/packages/ui"))).await,
            Some(PathBuf::from(path!("/repo/packages/ui/.biome.json"))),
            "a hidden configuration in the nearest directory wins over a parent's"
        );
        assert_eq!(
            find_config_file(fs.as_ref(), Path::new(path!("/repo/packages/admin"))).await,
            Some(PathBuf::from(path!("/repo/packages/admin/biome.json"))),
            "biome.json wins over .biome.json in the same directory"
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
            workspace_configuration_for(None, None),
            json!({ "biome": {} }),
            "without user settings biome reads the project's configuration"
        );
        assert_eq!(
            workspace_configuration_for(
                Some(json!({
                    "require_config_file": true,
                    "config_path": "packages/web/biome.json",
                    "inline_config": { "linter": { "rules": { "suspicious": { "noConsole": "off" } } } },
                    "goto_definition": true
                })),
                None
            ),
            json!({
                "biome": {
                    "requireConfiguration": true,
                    "configurationPath": "packages/web/biome.json",
                    "inlineConfig": {
                        "linter": { "rules": { "suspicious": { "noConsole": "off" } } }
                    },
                    "goToDefinition": true
                }
            }),
            "the names the Biome Zed extension documents map to Biome's LSP settings"
        );
    }

    #[test]
    fn workspace_configuration_passes_biomes_camel_case_settings_through() {
        assert_eq!(
            workspace_configuration_for(
                Some(json!({
                    "requireConfiguration": false,
                    "inlineConfig": { "formatter": { "lineWidth": 100 } }
                })),
                None
            ),
            json!({
                "biome": {
                    "requireConfiguration": false,
                    "inlineConfig": { "formatter": { "lineWidth": 100 } }
                }
            })
        );
    }

    #[test]
    fn workspace_configuration_uses_the_detected_configuration_unless_the_user_sets_one() {
        assert_eq!(
            workspace_configuration_for(None, Some(Path::new("/repo/biome.json"))),
            json!({
                "biome": { "configurationPath": "/repo/biome.json" }
            })
        );
        assert_eq!(
            workspace_configuration_for(
                Some(json!({ "config_path": "packages/web/biome.json" })),
                Some(Path::new("/repo/biome.json"))
            ),
            json!({
                "biome": { "configurationPath": "packages/web/biome.json" }
            }),
            "an explicit path wins over the detected one"
        );
        assert_eq!(
            workspace_configuration_for(
                Some(json!({ "configurationPath": null })),
                Some(Path::new("/repo/biome.json"))
            ),
            json!({ "biome": { "configurationPath": null } }),
            "an explicit null still stops the detected path from being sent"
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
