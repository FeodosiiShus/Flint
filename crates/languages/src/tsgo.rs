use anyhow::{Result, bail};
use async_trait::async_trait;
use collections::HashMap;
use gpui::AsyncApp;
use language::{LanguageName, LspAdapter, LspAdapterDelegate, LspInstaller, Toolchain};
use lsp::{CodeActionKind, LanguageServerBinary, LanguageServerName, Uri};
use node_runtime::{NodeRuntime, VersionStrategy};
use project::lsp_store::language_server_settings;
use semver::Version;
use serde_json::{Value, json};
use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
};
use util::{ResultExt, merge_json_value_into};

use crate::vtsls::{VtslsLspAdapter, completion_label};

const SERVER_NAME: LanguageServerName = LanguageServerName::new_static("tsgo");
const CODE_LENS_SHOW_LOCATIONS_COMMAND: &str = "editor.action.showReferences";
const EXECUTABLE_NAME: &str = if cfg!(windows) { "tsc.exe" } else { "tsc" };

pub struct TsgoLspAdapter {
    node: NodeRuntime,
}

impl TsgoLspAdapter {
    pub fn new(node: NodeRuntime) -> Self {
        TsgoLspAdapter { node }
    }
}

fn npm_platform_package_name(operating_system: &str, architecture: &str) -> Result<String> {
    let platform = match operating_system {
        "macos" => "darwin",
        "linux" => "linux",
        "windows" => "win32",
        unsupported => {
            bail!("TypeScript native server has no build for operating system {unsupported}")
        }
    };
    let processor = match architecture {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        unsupported => {
            bail!("TypeScript native server has no build for architecture {unsupported}")
        }
    };
    Ok(format!("@typescript/typescript-{platform}-{processor}"))
}

fn current_platform_package_name() -> Result<String> {
    npm_platform_package_name(std::env::consts::OS, std::env::consts::ARCH)
}

fn server_binary(container_dir: &Path, package_name: &str) -> LanguageServerBinary {
    LanguageServerBinary {
        path: container_dir
            .join("node_modules")
            .join(package_name)
            .join("lib")
            .join(EXECUTABLE_NAME),
        arguments: vec!["--lsp".into(), "--stdio".into()],
        env: None,
    }
}

impl LspInstaller for TsgoLspAdapter {
    type BinaryVersion = Version;

    async fn fetch_latest_server_version(
        &self,
        _: &Arc<dyn LspAdapterDelegate>,
        _: bool,
        _: &mut AsyncApp,
    ) -> Result<Self::BinaryVersion> {
        let package_name = current_platform_package_name()?;
        self.node.npm_package_latest_version(&package_name).await
    }

    fn fetch_server_binary(
        &self,
        latest_version: Self::BinaryVersion,
        container_dir: PathBuf,
        _: &Arc<dyn LspAdapterDelegate>,
    ) -> impl Send + Future<Output = Result<LanguageServerBinary>> + use<> {
        let node = self.node.clone();

        async move {
            let package_name = current_platform_package_name()?;
            let version = latest_version.to_string();

            node.npm_install_packages(&container_dir, &[(package_name.as_str(), version.as_str())])
                .await?;

            let binary = server_binary(&container_dir, &package_name);
            anyhow::ensure!(
                binary.path.exists(),
                "TypeScript native server executable is missing at {:?}",
                binary.path
            );
            Ok(binary)
        }
    }

    fn check_if_version_installed(
        &self,
        version: &Self::BinaryVersion,
        container_dir: &PathBuf,
        _: &Arc<dyn LspAdapterDelegate>,
    ) -> impl Send + Future<Output = Option<LanguageServerBinary>> + use<> {
        let node = self.node.clone();
        let server_version = version.clone();
        let container_dir = container_dir.clone();

        async move {
            let package_name = current_platform_package_name().log_err()?;
            let binary = server_binary(&container_dir, &package_name);

            if node
                .should_install_npm_package(
                    &package_name,
                    &binary.path,
                    &container_dir,
                    VersionStrategy::Latest(&server_version),
                )
                .await
            {
                return None;
            }

            Some(binary)
        }
    }

    async fn cached_server_binary(
        &self,
        container_dir: PathBuf,
        _: &dyn LspAdapterDelegate,
    ) -> Option<LanguageServerBinary> {
        let package_name = current_platform_package_name().log_err()?;
        let binary = server_binary(&container_dir, &package_name);
        binary.path.exists().then_some(binary)
    }
}

#[async_trait(?Send)]
impl LspAdapter for TsgoLspAdapter {
    fn name(&self) -> LanguageServerName {
        SERVER_NAME
    }

    fn code_action_kinds(&self) -> Option<Vec<CodeActionKind>> {
        Some(vec![
            CodeActionKind::QUICKFIX,
            CodeActionKind::REFACTOR,
            CodeActionKind::REFACTOR_EXTRACT,
            CodeActionKind::SOURCE,
        ])
    }

    async fn label_for_completion(
        &self,
        item: &lsp::CompletionItem,
        language: &Arc<language::Language>,
    ) -> Option<language::CodeLabel> {
        completion_label(item, language)
    }

    async fn initialization_options(
        self: Arc<Self>,
        _: &Arc<dyn LspAdapterDelegate>,
        _: &mut AsyncApp,
    ) -> Result<Option<Value>> {
        Ok(Some(json!({
            "codeLensShowLocationsCommandName": CODE_LENS_SHOW_LOCATIONS_COMMAND,
        })))
    }

    async fn workspace_configuration(
        self: Arc<Self>,
        delegate: &Arc<dyn LspAdapterDelegate>,
        _: Option<Toolchain>,
        _: Option<Uri>,
        cx: &mut AsyncApp,
    ) -> Result<Value> {
        let config = json!({
            "inlayHints": {
                "parameterNames": {
                    "enabled": "all",
                    "suppressWhenArgumentMatchesName": false
                },
                "parameterTypes": {
                    "enabled": true
                },
                "variableTypes": {
                    "enabled": true,
                    "suppressWhenTypeMatchesName": false
                },
                "propertyDeclarationTypes": {
                    "enabled": true
                },
                "functionLikeReturnTypes": {
                    "enabled": true
                },
                "enumMemberValues": {
                    "enabled": true
                }
            },
            "implementationsCodeLens": {
                "enabled": true,
                "showOnAllClassMethods": true,
                "showOnInterfaceMethods": true
            },
            "referencesCodeLens": {
                "enabled": true,
                "showOnAllFunctions": true
            },
        });

        let mut default_workspace_configuration = json!({
            "typescript": config,
            "javascript": config,
        });

        let override_options = cx.update(|cx| {
            language_server_settings(delegate.as_ref(), &SERVER_NAME, cx)
                .and_then(|s| s.settings.clone())
        });

        if let Some(override_options) = override_options {
            merge_json_value_into(override_options, &mut default_workspace_configuration)
        }

        Ok(default_workspace_configuration)
    }

    fn diagnostic_message_to_markdown(&self, message: &str) -> Option<String> {
        VtslsLspAdapter::enhance_diagnostic_message(message)
    }

    fn language_ids(&self) -> HashMap<LanguageName, String> {
        HashMap::from_iter([
            (LanguageName::new_static("TypeScript"), "typescript".into()),
            (LanguageName::new_static("JavaScript"), "javascript".into()),
            (LanguageName::new_static("TSX"), "typescriptreact".into()),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::npm_platform_package_name;

    #[test]
    fn maps_supported_platforms_to_npm_platform_packages() {
        let cases = [
            ("macos", "aarch64", "@typescript/typescript-darwin-arm64"),
            ("macos", "x86_64", "@typescript/typescript-darwin-x64"),
            ("linux", "x86_64", "@typescript/typescript-linux-x64"),
            ("linux", "aarch64", "@typescript/typescript-linux-arm64"),
            ("windows", "x86_64", "@typescript/typescript-win32-x64"),
            ("windows", "aarch64", "@typescript/typescript-win32-arm64"),
        ];

        for (operating_system, architecture, expected) in cases {
            let package_name = npm_platform_package_name(operating_system, architecture)
                .expect("supported platform should map to a package");
            assert_eq!(package_name, expected);
        }
    }

    #[test]
    fn rejects_platforms_without_a_native_build() {
        assert!(npm_platform_package_name("freebsd", "x86_64").is_err());
        assert!(npm_platform_package_name("macos", "riscv64").is_err());
    }
}
