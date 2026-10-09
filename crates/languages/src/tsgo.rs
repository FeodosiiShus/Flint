use anyhow::{Result, bail};
use async_trait::async_trait;
use collections::HashMap;
use gpui::AsyncApp;
use language::{LanguageName, LspAdapter, LspAdapterDelegate, LspInstaller, Toolchain};
use lsp::{CodeActionKind, LanguageServerBinary, LanguageServerName, Uri};
use node_runtime::{NodeRuntime, VersionStrategy};
use project::lsp_store::language_server_settings;
use regex::Regex;
use semver::Version;
use serde_json::{Value, json};
use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};
use util::{ResultExt, merge_json_value_into};

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
        enhance_diagnostic_message(message)
    }

    fn language_ids(&self) -> HashMap<LanguageName, String> {
        HashMap::from_iter([
            (LanguageName::new_static("TypeScript"), "typescript".into()),
            (LanguageName::new_static("JavaScript"), "javascript".into()),
            (LanguageName::new_static("TSX"), "typescriptreact".into()),
        ])
    }
}

fn enhance_diagnostic_message(message: &str) -> Option<String> {
    static SINGLE_WORD_REGEX: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"'([^\s']*)'").expect("Failed to create REGEX"));

    static MULTI_WORD_REGEX: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"'([^']+\s+[^']*)'").expect("Failed to create REGEX"));

    let first = SINGLE_WORD_REGEX.replace_all(message, "`$1`").to_string();
    let second = MULTI_WORD_REGEX
        .replace_all(&first, "\n```typescript\n$1\n```\n")
        .to_string();
    Some(second)
}

fn completion_label(
    item: &lsp::CompletionItem,
    language: &Arc<language::Language>,
) -> Option<language::CodeLabel> {
    use lsp::CompletionItemKind as Kind;
    let label_len = item.label.len();
    let grammar = language.grammar()?;
    let highlight_id = match item.kind? {
        Kind::CLASS | Kind::INTERFACE | Kind::ENUM => grammar.highlight_id_for_name("type"),
        Kind::CONSTRUCTOR => grammar.highlight_id_for_name("type"),
        Kind::CONSTANT => grammar.highlight_id_for_name("constant"),
        Kind::FUNCTION | Kind::METHOD => grammar.highlight_id_for_name("function"),
        Kind::PROPERTY | Kind::FIELD => grammar.highlight_id_for_name("property"),
        Kind::VARIABLE => grammar.highlight_id_for_name("variable"),
        _ => None,
    }?;

    let text = if let Some(description) = item
        .label_details
        .as_ref()
        .and_then(|label_details| label_details.description.as_ref())
    {
        format!("{} {}", item.label, description)
    } else if let Some(detail) = &item.detail {
        format!("{} {}", item.label, detail)
    } else {
        item.label.clone()
    };
    Some(language::CodeLabel::filtered(
        text,
        label_len,
        item.filter_text.as_deref(),
        vec![(0..label_len, highlight_id)],
    ))
}

#[cfg(test)]
mod tests {
    use super::{enhance_diagnostic_message, npm_platform_package_name};

    #[test]
    fn enhance_diagnostic_message_formats_quoted_types() {
        let message = "The expected type comes from the return type of this signature.";
        assert_eq!(
            enhance_diagnostic_message(message).expect("Should be some"),
            message
        );

        let message = "Property 'baz' is missing in type '{ foo: string; bar: string; }' but required in type 'User'.";
        let expected = "Property `baz` is missing in type \n```typescript\n{ foo: string; bar: string; }\n```\n but required in type `User`.";
        assert_eq!(
            enhance_diagnostic_message(message).expect("Should be some"),
            expected
        );

        let message = "Type '() => { foo: string; bar: string; }' is not assignable to type 'GetUserFunction'.\n  Property 'baz' is missing in type '{ foo: string; bar: string; }' but required in type 'User'.";
        let expected = "Type \n```typescript\n() => { foo: string; bar: string; }\n```\n is not assignable to type `GetUserFunction`.\n  Property `baz` is missing in type \n```typescript\n{ foo: string; bar: string; }\n```\n but required in type `User`.";
        assert_eq!(
            enhance_diagnostic_message(message).expect("Should be some"),
            expected
        );
    }

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
