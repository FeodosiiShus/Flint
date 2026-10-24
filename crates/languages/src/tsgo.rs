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
use sysinfo::{MemoryRefreshKind, RefreshKind, System};
use util::{ResultExt, merge_json_value_into};

const CODE_LENS_SHOW_LOCATIONS_COMMAND: &str = "editor.action.showReferences";
const TSGO_EXECUTABLE_NAME: &str = if cfg!(windows) { "tsc.exe" } else { "tsc" };
const TSRS_EXECUTABLE_NAME: &str = if cfg!(windows) { "tsrs.exe" } else { "tsrs" };
const GO_MEMORY_LIMIT_VARIABLE: &str = "GOMEMLIMIT";
const MEBIBYTE: u64 = 1024 * 1024;
const GIBIBYTE: u64 = 1024 * MEBIBYTE;

/// The native TypeScript language servers: Microsoft's Go port (`tsgo`), and `tsrs`, an
/// experimental Rust port of it that speaks the same protocol, takes the same settings and uses
/// less memory. Only `tsgo` runs by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeServer {
    Tsgo,
    Tsrs,
}

impl NativeServer {
    fn name(self) -> LanguageServerName {
        match self {
            Self::Tsgo => LanguageServerName::new_static("tsgo"),
            Self::Tsrs => LanguageServerName::new_static("tsrs"),
        }
    }

    fn npm_platform_package_name(
        self,
        operating_system: &str,
        architecture: &str,
    ) -> Result<String> {
        let name = self.name();
        let (platform, processor) = match (self, operating_system, architecture) {
            (Self::Tsgo, "macos", "aarch64") => ("darwin", "arm64"),
            (Self::Tsgo, "macos", "x86_64") => ("darwin", "x64"),
            (Self::Tsgo, "linux", "aarch64") => ("linux", "arm64"),
            (Self::Tsgo, "linux", "x86_64") => ("linux", "x64"),
            (Self::Tsgo, "windows", "aarch64") => ("win32", "arm64"),
            (Self::Tsgo, "windows", "x86_64") => ("win32", "x64"),
            (Self::Tsrs, "macos", "aarch64") => ("darwin", "arm64"),
            (Self::Tsrs, "linux", "aarch64") => ("linux", "arm64-gnu"),
            (Self::Tsrs, "linux", "x86_64") => ("linux", "x64-gnu"),
            _ => bail!("{name} has no build for {operating_system} on {architecture}"),
        };
        Ok(match self {
            Self::Tsgo => format!("@typescript/typescript-{platform}-{processor}"),
            Self::Tsrs => format!("@tsrs/{platform}-{processor}"),
        })
    }

    fn current_platform_package_name(self) -> Result<String> {
        self.npm_platform_package_name(std::env::consts::OS, std::env::consts::ARCH)
    }

    fn binary(self, container_dir: &Path, package_name: &str) -> LanguageServerBinary {
        let package_dir = container_dir.join("node_modules").join(package_name);
        LanguageServerBinary {
            path: match self {
                Self::Tsgo => package_dir.join("lib").join(TSGO_EXECUTABLE_NAME),
                Self::Tsrs => package_dir.join(TSRS_EXECUTABLE_NAME),
            },
            arguments: vec!["--lsp".into(), "--stdio".into()],
            env: None,
        }
    }

    fn default_env(self) -> HashMap<String, String> {
        match self {
            Self::Tsgo => default_server_env(),
            // A Rust server frees memory as soon as it drops it.
            Self::Tsrs => HashMap::default(),
        }
    }
}

pub struct TsgoLspAdapter {
    node: NodeRuntime,
    server: NativeServer,
}

impl TsgoLspAdapter {
    pub fn new(node: NodeRuntime) -> Self {
        TsgoLspAdapter {
            node,
            server: NativeServer::Tsgo,
        }
    }

    pub fn tsrs(node: NodeRuntime) -> Self {
        TsgoLspAdapter {
            node,
            server: NativeServer::Tsrs,
        }
    }
}

/// Go keeps freed heap resident unless a soft memory limit asks it to return pages to the OS.
fn go_memory_limit(total_memory_bytes: u64) -> String {
    let limit = (total_memory_bytes / 4).clamp(2 * GIBIBYTE, 8 * GIBIBYTE);
    format!("{}MiB", limit / MEBIBYTE)
}

fn default_go_memory_limit() -> String {
    let system = System::new_with_specifics(
        RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    go_memory_limit(system.total_memory())
}

fn default_server_env() -> HashMap<String, String> {
    HashMap::from_iter([(
        GO_MEMORY_LIMIT_VARIABLE.to_string(),
        default_go_memory_limit(),
    )])
}

fn workspace_configuration_with_overrides(override_options: Option<Value>) -> Value {
    let config = json!({
        "inlayHints": {
            "parameterNames": {
                "enabled": "literals",
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
        // Resolving an implementations lens searches every file of the program and again from
        // each hit, so in a large monorepo class methods only get one when they are abstract.
        // A references lens on a function that is not exported searches only its own file,
        // which keeps "N usages" on every function, as in IntelliJ, at little cost.
        "implementationsCodeLens": {
            "enabled": true,
            "showOnAllClassMethods": false,
            "showOnInterfaceMethods": true
        },
        "referencesCodeLens": {
            "enabled": true,
            "showOnAllFunctions": true
        },
    });

    // tsgo asks for the `js/ts` section too and lets it override `typescript` and `javascript`;
    // its auto-import memory switches (`preferences.autoImportFileExcludePatterns`,
    // `suggest.autoImports`) are usually set there.
    let mut default_workspace_configuration = json!({
        "typescript": config,
        "javascript": config,
        "js/ts": {},
    });

    if let Some(override_options) = override_options {
        merge_json_value_into(override_options, &mut default_workspace_configuration)
    }

    default_workspace_configuration
}

impl LspInstaller for TsgoLspAdapter {
    type BinaryVersion = Version;

    async fn fetch_latest_server_version(
        &self,
        _: &Arc<dyn LspAdapterDelegate>,
        _: bool,
        _: &mut AsyncApp,
    ) -> Result<Self::BinaryVersion> {
        let package_name = self.server.current_platform_package_name()?;
        self.node.npm_package_latest_version(&package_name).await
    }

    fn fetch_server_binary(
        &self,
        latest_version: Self::BinaryVersion,
        container_dir: PathBuf,
        _: &Arc<dyn LspAdapterDelegate>,
    ) -> impl Send + Future<Output = Result<LanguageServerBinary>> + use<> {
        let node = self.node.clone();
        let server = self.server;

        async move {
            let package_name = server.current_platform_package_name()?;
            let version = latest_version.to_string();

            node.npm_install_packages(&container_dir, &[(package_name.as_str(), version.as_str())])
                .await?;

            let binary = server.binary(&container_dir, &package_name);
            anyhow::ensure!(
                binary.path.exists(),
                "{} executable is missing at {:?}",
                server.name(),
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
        let server = self.server;
        let server_version = version.clone();
        let container_dir = container_dir.clone();

        async move {
            let package_name = server.current_platform_package_name().log_err()?;
            let binary = server.binary(&container_dir, &package_name);

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
        let package_name = self.server.current_platform_package_name().log_err()?;
        let binary = self.server.binary(&container_dir, &package_name);
        binary.path.exists().then_some(binary)
    }
}

#[async_trait(?Send)]
impl LspAdapter for TsgoLspAdapter {
    fn name(&self) -> LanguageServerName {
        self.server.name()
    }

    /// The server learns about installed, upgraded and removed packages only from the changes
    /// inside `node_modules` its watchers ask for, and keeps stale types until it restarts
    /// otherwise.
    fn watches_excluded_directories(&self) -> bool {
        true
    }

    fn default_server_env(&self) -> HashMap<String, String> {
        self.server.default_env()
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
        let override_options = cx.update(|cx| {
            language_server_settings(delegate.as_ref(), &self.server.name(), cx)
                .and_then(|s| s.settings.clone())
        });

        Ok(workspace_configuration_with_overrides(override_options))
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
    use super::{
        GIBIBYTE, GO_MEMORY_LIMIT_VARIABLE, NativeServer, default_server_env,
        enhance_diagnostic_message, go_memory_limit, workspace_configuration_with_overrides,
    };
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn go_memory_limit_is_a_quarter_of_ram_clamped_to_two_to_eight_gibibytes() {
        assert_eq!(go_memory_limit(16 * GIBIBYTE), "4096MiB");
        assert_eq!(go_memory_limit(24 * GIBIBYTE), "6144MiB");
        assert_eq!(go_memory_limit(4 * GIBIBYTE), "2048MiB");
        assert_eq!(go_memory_limit(64 * GIBIBYTE), "8192MiB");
        assert_eq!(
            go_memory_limit(0),
            "2048MiB",
            "an unknown RAM size still gets the lower bound"
        );
    }

    #[test]
    fn tsgo_defaults_to_a_go_memory_limit() {
        let env = default_server_env();
        let limit = env
            .get(GO_MEMORY_LIMIT_VARIABLE)
            .expect("tsgo should start with a GOMEMLIMIT");
        let mebibytes = limit
            .strip_suffix("MiB")
            .and_then(|mebibytes| mebibytes.parse::<u64>().ok())
            .expect("GOMEMLIMIT should be a whole number of MiB");
        assert!((2048..=8192).contains(&mebibytes), "{limit}");
    }

    #[test]
    fn workspace_configuration_passes_js_ts_settings_through() {
        let defaults = workspace_configuration_with_overrides(None);
        assert_eq!(defaults["js/ts"], json!({}));
        assert_eq!(
            defaults["typescript"]["referencesCodeLens"]["enabled"],
            true
        );
        assert_eq!(
            defaults["typescript"]["referencesCodeLens"]["showOnAllFunctions"], true,
            "every function shows its usages; a function that is not exported is searched for \
             only in its own file"
        );
        assert_eq!(
            defaults["typescript"]["implementationsCodeLens"]["showOnAllClassMethods"], false,
            "a large monorepo should not resolve implementations for every class method"
        );

        let configuration = workspace_configuration_with_overrides(Some(json!({
            "js/ts": {
                "preferences": {
                    "autoImportFileExcludePatterns": ["**/node_modules/@aws-sdk/**"]
                },
                "suggest": { "autoImports": false }
            }
        })));
        assert_eq!(
            configuration["js/ts"],
            json!({
                "preferences": {
                    "autoImportFileExcludePatterns": ["**/node_modules/@aws-sdk/**"]
                },
                "suggest": { "autoImports": false }
            })
        );
        assert_eq!(
            configuration["javascript"]["inlayHints"]["parameterNames"]["enabled"],
            "literals"
        );
    }

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
            let package_name = NativeServer::Tsgo
                .npm_platform_package_name(operating_system, architecture)
                .expect("supported platform should map to a package");
            assert_eq!(package_name, expected);
        }
    }

    #[test]
    fn rejects_platforms_without_a_native_build() {
        for server in [NativeServer::Tsgo, NativeServer::Tsrs] {
            assert!(
                server
                    .npm_platform_package_name("freebsd", "x86_64")
                    .is_err()
            );
            assert!(
                server
                    .npm_platform_package_name("macos", "riscv64")
                    .is_err()
            );
        }
        assert!(
            NativeServer::Tsrs
                .npm_platform_package_name("macos", "x86_64")
                .is_err(),
            "tsrs publishes no Intel macOS build"
        );
    }

    #[test]
    fn tsrs_runs_the_native_binary_of_its_platform_package_without_a_go_memory_limit() {
        let package_name = NativeServer::Tsrs
            .npm_platform_package_name("macos", "aarch64")
            .expect("tsrs has an Apple silicon build");
        assert_eq!(package_name, "@tsrs/darwin-arm64");
        let binary = NativeServer::Tsrs.binary(Path::new("/servers/tsrs"), &package_name);
        assert_eq!(
            binary.path,
            Path::new("/servers/tsrs/node_modules/@tsrs/darwin-arm64/tsrs")
        );
        assert_eq!(binary.arguments, ["--lsp", "--stdio"]);
        assert!(NativeServer::Tsrs.default_env().is_empty());
        assert_eq!(NativeServer::Tsrs.name().0.as_ref(), "tsrs");
    }
}
