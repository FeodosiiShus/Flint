use anyhow::{Context as _, Result};
use async_trait::async_trait;
use collections::HashMap;
use futures::AsyncBufReadExt;
use futures::future::BoxFuture;
use gpui::{App, AsyncApp, SharedString};
use language::{LanguageName, ManifestName, ManifestProvider, ManifestQuery};
use language::{LspAdapter, LspAdapterDelegate, LspInstaller, Symbol};
use language::{Toolchain, ToolchainList, ToolchainLister, ToolchainMetadata};
use lsp::{CompletionItemKind, LanguageServerBinary, LanguageServerName, Uri};
use node_runtime::{NodeRuntime, VersionStrategy};
use pet_core::Configuration;
use pet_core::os_environment::Environment;
use pet_core::python_environment::{PythonEnvironment, PythonEnvironmentKind};
use pet_virtualenv::is_virtualenv_dir;
use project::Fs;
use project::lsp_store::language_server_settings;
use rope::Rope;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use settings::{SemanticTokenRules, Settings};
use terminal::terminal_settings::TerminalSettings;

use parking_lot::Mutex;
use std::cmp::{Ordering, Reverse};
use std::{
    ffi::OsString,
    fmt::Write,
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
};
use util::ResultExt;
use util::rel_path::RelPath;
use util::shell::ShellKind;

pub(crate) fn semantic_token_rules() -> SemanticTokenRules {
    let content = grammars::get_file("python/semantic_token_rules.json")
        .expect("missing python/semantic_token_rules.json");
    let json = std::str::from_utf8(&content.data).expect("invalid utf-8 in semantic_token_rules");
    settings::parse_json_with_comments::<SemanticTokenRules>(json)
        .expect("failed to parse python semantic_token_rules.json")
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PythonToolchainData {
    #[serde(flatten)]
    environment: PythonEnvironment,
    #[serde(skip_serializing_if = "Option::is_none")]
    activation_scripts: Option<HashMap<ShellKind, PathBuf>>,
}

pub(crate) struct PyprojectTomlManifestProvider;

impl ManifestProvider for PyprojectTomlManifestProvider {
    fn name(&self) -> ManifestName {
        SharedString::new_static("pyproject.toml").into()
    }

    fn search(
        &self,
        ManifestQuery {
            path,
            depth,
            delegate,
        }: ManifestQuery,
    ) -> Option<Arc<RelPath>> {
        const WORKSPACE_LOCKFILES: &[&str] =
            &["uv.lock", "poetry.lock", "pdm.lock", "Pipfile.lock"];

        let mut innermost_pyproject = None;
        let mut outermost_workspace_root = None;

        for path in path.ancestors().take(depth) {
            let pyproject_path = path.join(RelPath::from_unix_str("pyproject.toml").unwrap());
            if delegate.exists(&pyproject_path, Some(false)) {
                if innermost_pyproject.is_none() {
                    innermost_pyproject = Some(Arc::from(path));
                }

                let has_lockfile = WORKSPACE_LOCKFILES.iter().any(|lockfile| {
                    let lockfile_path = path.join(RelPath::from_unix_str(lockfile).unwrap());
                    delegate.exists(&lockfile_path, Some(false))
                });
                if has_lockfile {
                    outermost_workspace_root = Some(Arc::from(path));
                }
            }
        }

        outermost_workspace_root.or(innermost_pyproject)
    }
}

/// Pyright assigns each completion item a `sortText` of the form `XX.YYYY.name`.
/// Where `XX` is the sorting category, `YYYY` is based on most recent usage,
/// and `name` is the symbol name itself.
///
/// The problem with it is that Pyright adjusts the sort text based on previous resolutions (items for which we've issued `completion/resolve` call have their sortText adjusted),
/// which - long story short - makes completion items list non-stable. Pyright probably relies on VSCode's implementation detail.
/// see https://github.com/microsoft/pyright/blob/95ef4e103b9b2f129c9320427e51b73ea7cf78bd/packages/pyright-internal/src/languageService/completionProvider.ts#LL2873
///
/// upd 02.12.25:
/// Decided to ignore Pyright's sortText() completely and to manually sort all entries
fn process_pyright_completions(items: &mut [lsp::CompletionItem]) {
    for item in items {
        let is_named_argument = item.label.ends_with('=');

        let is_dunder = item.label.starts_with("__") && item.label.ends_with("__");

        let visibility_priority = if is_dunder {
            '3'
        } else if item.label.starts_with("__") {
            '2' // private non-dunder
        } else if item.label.starts_with('_') {
            '1' // protected
        } else {
            '0' // public
        };

        let is_external = item
            .detail
            .as_ref()
            .is_some_and(|detail| detail == "Auto-import");

        let source_priority = if is_external { '1' } else { '0' };

        // Kind priority within same visibility level
        let kind_priority = match item.kind {
            Some(lsp::CompletionItemKind::KEYWORD) => '0',
            Some(lsp::CompletionItemKind::ENUM_MEMBER) => '1',
            Some(lsp::CompletionItemKind::FIELD) => '2',
            Some(lsp::CompletionItemKind::PROPERTY) => '3',
            Some(lsp::CompletionItemKind::VARIABLE) => '4',
            Some(lsp::CompletionItemKind::CONSTANT) => '5',
            Some(lsp::CompletionItemKind::METHOD) => '6',
            Some(lsp::CompletionItemKind::FUNCTION) => '6',
            Some(lsp::CompletionItemKind::CLASS) => '7',
            Some(lsp::CompletionItemKind::MODULE) => '8',

            _ => 'z',
        };

        // Named arguments get higher priority
        let argument_priority = if is_named_argument { '0' } else { '1' };

        item.sort_text = Some(format!(
            "{}{}{}{}{}",
            argument_priority, source_priority, visibility_priority, kind_priority, item.label
        ));
    }
}

fn label_for_pyright_completion(
    item: &lsp::CompletionItem,
    language: &Arc<language::Language>,
) -> Option<language::CodeLabel> {
    let label = &item.label;
    let label_len = label.len();
    let grammar = language.grammar()?;
    let highlight_id = highlight_id_for_completion(item.kind?, grammar)?;

    let mut text = label.clone();
    if let Some(completion_details) = item
        .label_details
        .as_ref()
        .and_then(|details| details.description.as_ref())
    {
        write!(&mut text, " {}", completion_details).ok();
    }
    Some(language::CodeLabel::filtered(
        text,
        label_len,
        item.filter_text.as_deref(),
        highlight_id
            .map(|id| (0..label_len, id))
            .into_iter()
            .collect(),
    ))
}

fn label_for_python_symbol(
    symbol: &Symbol,
    language: &Arc<language::Language>,
) -> Option<language::CodeLabel> {
    let name = &symbol.name;
    let (text, filter_range, display_range) = match symbol.kind {
        language::SymbolKind::Method | language::SymbolKind::Function => {
            let text = format!("def {}():\n", name);
            let filter_range = 4..4 + name.len();
            let display_range = 0..filter_range.end;
            (text, filter_range, display_range)
        }
        language::SymbolKind::Class => {
            let text = format!("class {}:", name);
            let filter_range = 6..6 + name.len();
            let display_range = 0..filter_range.end;
            (text, filter_range, display_range)
        }
        language::SymbolKind::Constant => {
            let text = format!("{} = 0", name);
            let filter_range = 0..name.len();
            let display_range = 0..filter_range.end;
            (text, filter_range, display_range)
        }
        _ => return None,
    };
    Some(language::CodeLabel::new(
        text.get(display_range.clone())?.to_string(),
        filter_range,
        language.highlight_text(&Rope::from(text.as_str()), display_range),
    ))
}

/// Returns the highlight ID for the given completion item kind, if it is supported.
///
/// The outer `Option` is `None` if the item kind returned by the language server is not covered.
/// The inner `Option` is `None` if the item kind is covered, but the highlight name is not present in the grammar.
fn highlight_id_for_completion(
    item_kind: CompletionItemKind,
    grammar: &Arc<language::Grammar>,
) -> Option<Option<language::HighlightId>> {
    match item_kind {
        CompletionItemKind::METHOD => Some(grammar.highlight_id_for_name("function.method.call")),
        CompletionItemKind::FUNCTION => Some(grammar.highlight_id_for_name("function.call")),
        CompletionItemKind::CLASS => Some(grammar.highlight_id_for_name("type")),
        CompletionItemKind::CONSTANT => Some(grammar.highlight_id_for_name("constant")),
        CompletionItemKind::VARIABLE => Some(grammar.highlight_id_for_name("variable")),
        _ => None,
    }
}

/// Older pyright-derived servers request a literal `<configuration section>.analysis` section,
/// while new versions request `<configuration section>` and read its nested `analysis` object.
///
/// Basedpyright changed this behavior in v1.39.10, context: https://github.com/DetachHead/basedpyright/pull/1847
/// Pyright itself changed this behavior in v1.1.411 context: https://github.com/microsoft/pyright/pull/11480
fn normalize_pyright_analysis_configuration(
    workspace_configuration: &mut Value,
    configuration_section: &str,
) {
    let Some(workspace_configuration) = workspace_configuration.as_object_mut() else {
        return;
    };

    let flat_analysis_section = format!("{configuration_section}.analysis");

    let nested_analysis = workspace_configuration
        .get(configuration_section)
        .and_then(Value::as_object)
        .and_then(|server_configuration| server_configuration.get("analysis"))
        .and_then(Value::as_object);
    let flat_analysis = workspace_configuration
        .get(&flat_analysis_section)
        .and_then(Value::as_object);

    let nested_analysis = match (nested_analysis, flat_analysis) {
        (Some(nested_analysis), Some(flat_analysis)) => {
            let mut merged_analysis = nested_analysis.clone();
            for (key, value) in flat_analysis {
                merged_analysis
                    .entry(key.clone())
                    .or_insert_with(|| value.clone());
            }
            Value::Object(merged_analysis)
        }
        (Some(nested_analysis), None) => Value::Object(nested_analysis.clone()),
        (None, Some(flat_analysis)) => Value::Object(flat_analysis.clone()),
        (None, None) => return,
    };

    let server_configuration = workspace_configuration
        .entry(configuration_section)
        .or_insert_with(|| Value::Object(serde_json::Map::default()));
    let Some(server_configuration) = server_configuration.as_object_mut() else {
        return;
    };
    server_configuration.insert("analysis".to_owned(), nested_analysis.clone());

    workspace_configuration.insert(flat_analysis_section, nested_analysis);
}

fn basedpyright_workspace_configuration(
    user_settings: Option<Value>,
    toolchain: Option<&Toolchain>,
) -> Value {
    let mut configuration = match user_settings {
        Some(Value::Object(settings)) => settings,
        _ => serde_json::Map::default(),
    };

    // Basedpyright's own default (`recommended`) reports far more than pyright does, which would
    // bury a project that never configured it.
    if let Some(analysis) = configuration
        .entry("basedpyright.analysis")
        .or_insert_with(|| Value::Object(serde_json::Map::default()))
        .as_object_mut()
    {
        analysis
            .entry("typeCheckingMode")
            .or_insert_with(|| Value::String("standard".to_owned()));
    }

    let user_chose_interpreter = configuration
        .get("python")
        .and_then(Value::as_object)
        .is_some_and(|python| {
            ["pythonPath", "venvPath"]
                .into_iter()
                .any(|known_key| python.contains_key(known_key))
        });
    if let Some(toolchain) = toolchain
        && !user_chose_interpreter
        && serde_json::from_value::<PythonEnvironment>(toolchain.as_json.clone()).is_ok()
        && let Some(python) = configuration
            .entry("python")
            .or_insert_with(|| Value::Object(serde_json::Map::default()))
            .as_object_mut()
    {
        python.insert(
            "pythonPath".to_owned(),
            Value::String(toolchain.path.to_string()),
        );
    }

    let mut configuration = Value::Object(configuration);
    normalize_pyright_analysis_configuration(&mut configuration, "basedpyright");
    configuration
}

pub(crate) struct BasedPyrightLspAdapter {
    node: NodeRuntime,
}

impl BasedPyrightLspAdapter {
    const SERVER_NAME: LanguageServerName = LanguageServerName::new_static("basedpyright");
    const PACKAGE_NAME: &'static str = "basedpyright";
    const BINARY_NAME: &'static str = "basedpyright-langserver";
    const SERVER_PATH: &'static str = "node_modules/basedpyright/langserver.index.js";
    const NODE_MODULE_RELATIVE_SERVER_PATH: &'static str = "basedpyright/langserver.index.js";

    pub(crate) fn new(node: NodeRuntime) -> Self {
        BasedPyrightLspAdapter { node }
    }
}

fn basedpyright_node_binary(node: PathBuf, server_path: &Path) -> LanguageServerBinary {
    LanguageServerBinary {
        path: node,
        env: None,
        arguments: vec![OsString::from(server_path), "--stdio".into()],
    }
}

#[async_trait(?Send)]
impl LspAdapter for BasedPyrightLspAdapter {
    fn name(&self) -> LanguageServerName {
        Self::SERVER_NAME
    }

    async fn initialization_options(
        self: Arc<Self>,
        _: &Arc<dyn LspAdapterDelegate>,
        _: &mut AsyncApp,
    ) -> Result<Option<Value>> {
        // The interpreter comes later, from `workspace/configuration`.
        Ok(Some(json!({
            "python": {
                "analysis": {
                    "autoSearchPaths": true,
                    "useLibraryCodeForTypes": true,
                    "autoImportCompletions": true
                }
            }
        })))
    }

    async fn process_completions(&self, items: &mut [lsp::CompletionItem]) {
        process_pyright_completions(items);
    }

    async fn label_for_completion(
        &self,
        item: &lsp::CompletionItem,
        language: &Arc<language::Language>,
    ) -> Option<language::CodeLabel> {
        label_for_pyright_completion(item, language)
    }

    async fn label_for_symbol(
        &self,
        symbol: &Symbol,
        language: &Arc<language::Language>,
    ) -> Option<language::CodeLabel> {
        label_for_python_symbol(symbol, language)
    }

    async fn workspace_configuration(
        self: Arc<Self>,
        delegate: &Arc<dyn LspAdapterDelegate>,
        toolchain: Option<Toolchain>,
        _: Option<Uri>,
        cx: &mut AsyncApp,
    ) -> Result<Value> {
        let user_settings = cx.update(|cx| {
            language_server_settings(delegate.as_ref(), &Self::SERVER_NAME, cx)
                .and_then(|settings| settings.settings.clone())
        });
        Ok(basedpyright_workspace_configuration(
            user_settings,
            toolchain.as_ref(),
        ))
    }
}

impl LspInstaller for BasedPyrightLspAdapter {
    type BinaryVersion = Version;

    async fn fetch_latest_server_version(
        &self,
        _: &Arc<dyn LspAdapterDelegate>,
        _: bool,
        _: &mut AsyncApp,
    ) -> Result<Self::BinaryVersion> {
        self.node
            .npm_package_latest_version(Self::PACKAGE_NAME)
            .await
    }

    async fn check_if_user_installed(
        &self,
        delegate: &Arc<dyn LspAdapterDelegate>,
        _: Option<Toolchain>,
        _: &AsyncApp,
    ) -> Option<LanguageServerBinary> {
        if let Some(path) = delegate.which(Self::BINARY_NAME.as_ref()).await {
            return Some(LanguageServerBinary {
                path,
                env: None,
                arguments: vec!["--stdio".into()],
            });
        }

        let (node_modules_path, _) = delegate
            .npm_package_installed_version(Self::PACKAGE_NAME)
            .await
            .log_err()??;
        let node = self.node.binary_path().await.log_err()?;
        Some(basedpyright_node_binary(
            node,
            &node_modules_path.join(Self::NODE_MODULE_RELATIVE_SERVER_PATH),
        ))
    }

    fn fetch_server_binary(
        &self,
        _latest_version: Self::BinaryVersion,
        container_dir: PathBuf,
        _: &Arc<dyn LspAdapterDelegate>,
    ) -> impl Send + Future<Output = Result<LanguageServerBinary>> + use<> {
        let node = self.node.clone();

        async move {
            node.npm_install_latest_packages(&container_dir, &[Self::PACKAGE_NAME])
                .await?;

            let server_path = container_dir.join(Self::SERVER_PATH);
            anyhow::ensure!(
                server_path.exists(),
                "basedpyright language server is missing at {server_path:?}"
            );
            Ok(basedpyright_node_binary(
                node.binary_path().await?,
                &server_path,
            ))
        }
    }

    fn check_if_version_installed(
        &self,
        version: &Self::BinaryVersion,
        container_dir: &PathBuf,
        _: &Arc<dyn LspAdapterDelegate>,
    ) -> impl Send + Future<Output = Option<LanguageServerBinary>> + use<> {
        let node = self.node.clone();
        let version = version.clone();
        let container_dir = container_dir.clone();

        async move {
            let server_path = container_dir.join(Self::SERVER_PATH);

            if node
                .should_install_npm_package(
                    Self::PACKAGE_NAME,
                    &server_path,
                    &container_dir,
                    VersionStrategy::Latest(&version),
                )
                .await
            {
                return None;
            }

            Some(basedpyright_node_binary(
                node.binary_path().await.log_err()?,
                &server_path,
            ))
        }
    }

    async fn cached_server_binary(
        &self,
        container_dir: PathBuf,
        _: &dyn LspAdapterDelegate,
    ) -> Option<LanguageServerBinary> {
        let server_path = container_dir.join(Self::SERVER_PATH);
        if !server_path.exists() {
            return None;
        }
        Some(basedpyright_node_binary(
            self.node.binary_path().await.log_err()?,
            &server_path,
        ))
    }
}

fn is_python_env_global(k: &PythonEnvironmentKind) -> bool {
    matches!(
        k,
        PythonEnvironmentKind::Homebrew
            | PythonEnvironmentKind::Pyenv
            | PythonEnvironmentKind::GlobalPaths
            | PythonEnvironmentKind::MacPythonOrg
            | PythonEnvironmentKind::MacCommandLineTools
            | PythonEnvironmentKind::LinuxGlobal
            | PythonEnvironmentKind::MacXCode
            | PythonEnvironmentKind::WindowsStore
            | PythonEnvironmentKind::WindowsRegistry
    )
}

fn python_env_kind_display(k: &PythonEnvironmentKind) -> &'static str {
    match k {
        PythonEnvironmentKind::Conda => "Conda",
        PythonEnvironmentKind::Pixi => "pixi",
        PythonEnvironmentKind::Homebrew => "Homebrew",
        PythonEnvironmentKind::Pyenv => "global (Pyenv)",
        PythonEnvironmentKind::GlobalPaths => "global",
        PythonEnvironmentKind::PyenvVirtualEnv => "Pyenv",
        PythonEnvironmentKind::Pipenv => "Pipenv",
        PythonEnvironmentKind::Poetry => "Poetry",
        PythonEnvironmentKind::Hatch => "Hatch",
        PythonEnvironmentKind::MacPythonOrg => "global (Python.org)",
        PythonEnvironmentKind::MacCommandLineTools => "global (Command Line Tools for Xcode)",
        PythonEnvironmentKind::LinuxGlobal => "global",
        PythonEnvironmentKind::MacXCode => "global (Xcode)",
        PythonEnvironmentKind::Venv => "venv",
        PythonEnvironmentKind::VirtualEnv => "virtualenv",
        PythonEnvironmentKind::VirtualEnvWrapper => "virtualenvwrapper",
        PythonEnvironmentKind::WinPython => "WinPython",
        PythonEnvironmentKind::WindowsStore => "global (Windows Store)",
        PythonEnvironmentKind::WindowsRegistry => "global (Windows Registry)",
        PythonEnvironmentKind::Uv => "uv",
        PythonEnvironmentKind::UvWorkspace => "uv (Workspace)",
    }
}

pub(crate) struct PythonToolchainProvider {
    fs: Arc<dyn Fs>,
}

impl PythonToolchainProvider {
    pub fn new(fs: Arc<dyn Fs>) -> Self {
        Self { fs }
    }
}

static ENV_PRIORITY_LIST: &[PythonEnvironmentKind] = &[
    // Prioritize non-Conda environments.
    PythonEnvironmentKind::UvWorkspace,
    PythonEnvironmentKind::Uv,
    PythonEnvironmentKind::Poetry,
    PythonEnvironmentKind::Pipenv,
    PythonEnvironmentKind::VirtualEnvWrapper,
    PythonEnvironmentKind::Venv,
    PythonEnvironmentKind::VirtualEnv,
    PythonEnvironmentKind::PyenvVirtualEnv,
    PythonEnvironmentKind::Pixi,
    PythonEnvironmentKind::Conda,
    PythonEnvironmentKind::Pyenv,
    PythonEnvironmentKind::GlobalPaths,
    PythonEnvironmentKind::Homebrew,
];

fn env_priority(kind: Option<PythonEnvironmentKind>) -> usize {
    if let Some(kind) = kind {
        ENV_PRIORITY_LIST
            .iter()
            .position(|blessed_env| blessed_env == &kind)
            .unwrap_or(ENV_PRIORITY_LIST.len())
    } else {
        // Unknown toolchains are less useful than non-blessed ones.
        ENV_PRIORITY_LIST.len() + 1
    }
}

/// Return the name of environment declared in <worktree-root/.venv.
///
/// https://virtualfish.readthedocs.io/en/latest/plugins.html#auto-activation-auto-activation
async fn get_worktree_venv_declaration(worktree_root: &Path) -> Option<String> {
    let file = async_fs::File::open(worktree_root.join(".venv"))
        .await
        .ok()?;
    let mut venv_name = String::new();
    smol::io::BufReader::new(file)
        .read_line(&mut venv_name)
        .await
        .ok()?;
    Some(venv_name.trim().to_string())
}

fn get_venv_parent_dir(env: &PythonEnvironment) -> Option<PathBuf> {
    // If global, we aren't a virtual environment
    if let Some(kind) = env.kind
        && is_python_env_global(&kind)
    {
        return None;
    }

    // Check to be sure we are a virtual environment using pet's most generic
    // virtual environment type, VirtualEnv
    let venv = env
        .executable
        .as_ref()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .filter(|p| is_virtualenv_dir(p))?;

    venv.parent().map(|parent| parent.to_path_buf())
}

// How far is this venv from the root of our current project?
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum SubprojectDistance {
    WithinSubproject(Reverse<usize>),
    WithinWorktree(Reverse<usize>),
    NotInWorktree,
}

fn wr_distance(
    wr: &PathBuf,
    subroot_relative_path: &RelPath,
    venv: Option<&PathBuf>,
) -> SubprojectDistance {
    if let Some(venv) = venv
        && let Ok(p) = venv.strip_prefix(wr)
    {
        if subroot_relative_path.components().next().is_some()
            && let Ok(distance) = p
                .strip_prefix(subroot_relative_path.as_std_path())
                .map(|p| p.components().count())
        {
            SubprojectDistance::WithinSubproject(Reverse(distance))
        } else {
            SubprojectDistance::WithinWorktree(Reverse(p.components().count()))
        }
    } else {
        SubprojectDistance::NotInWorktree
    }
}

fn micromamba_shell_name(kind: ShellKind) -> &'static str {
    match kind {
        ShellKind::Csh => "csh",
        ShellKind::Fish => "fish",
        ShellKind::Nushell => "nu",
        ShellKind::PowerShell | ShellKind::Pwsh => "powershell",
        ShellKind::Cmd => "cmd.exe",
        // default / catch-all:
        _ => "posix",
    }
}

#[async_trait]
impl ToolchainLister for PythonToolchainProvider {
    async fn list(
        &self,
        worktree_root: PathBuf,
        subroot_relative_path: Arc<RelPath>,
        project_env: Option<HashMap<String, String>>,
    ) -> ToolchainList {
        let fs = &*self.fs;
        let env = project_env.unwrap_or_default();
        let environment = EnvironmentApi::from_env(&env);
        let locators = pet::locators::create_locators(
            Arc::new(pet_conda::Conda::from(&environment)),
            Arc::new(pet_poetry::Poetry::from(&environment)),
            &environment,
        );
        let mut config = Configuration::default();

        // `.ancestors()` will yield at least one path, so in case of empty `subroot_relative_path`, we'll just use
        // worktree root as the workspace directory.
        config.workspace_directories = Some(
            subroot_relative_path
                .ancestors()
                .map(|ancestor| {
                    // remove trailing separator as it alters the environment name hash used by Poetry.
                    let path = worktree_root.join(ancestor.as_std_path());
                    let path_str = path.to_string_lossy();
                    if path_str.ends_with(std::path::MAIN_SEPARATOR) && path_str.len() > 1 {
                        PathBuf::from(path_str.trim_end_matches(std::path::MAIN_SEPARATOR))
                    } else {
                        path
                    }
                })
                .collect(),
        );
        for locator in locators.iter() {
            locator.configure(&config);
        }

        let reporter = pet_reporter::collect::create_reporter();
        pet::find::find_and_report_envs(&reporter, config, &locators, &environment, None, None);

        let mut toolchains = reporter
            .environments
            .lock()
            .map_or(Vec::new(), |mut guard| std::mem::take(&mut guard));

        let wr = worktree_root;
        let wr_venv = get_worktree_venv_declaration(&wr).await;
        // Sort detected environments by:
        //     environment name matching activation file (<workdir>/.venv)
        //     environment project dir matching worktree_root
        //     general env priority
        //     environment path matching the CONDA_PREFIX env var
        //     executable path
        toolchains.sort_by(|lhs, rhs| {
            // Compare venv names against worktree .venv file
            let venv_ordering =
                wr_venv
                    .as_ref()
                    .map_or(Ordering::Equal, |venv| match (&lhs.name, &rhs.name) {
                        (Some(l), Some(r)) => (r == venv).cmp(&(l == venv)),
                        (Some(l), None) if l == venv => Ordering::Less,
                        (None, Some(r)) if r == venv => Ordering::Greater,
                        _ => Ordering::Equal,
                    });

            // Compare project paths against worktree root
            let proj_ordering =
                || {
                    let lhs_project = lhs.project.clone().or_else(|| get_venv_parent_dir(lhs));
                    let rhs_project = rhs.project.clone().or_else(|| get_venv_parent_dir(rhs));
                    wr_distance(&wr, &subroot_relative_path, lhs_project.as_ref()).cmp(
                        &wr_distance(&wr, &subroot_relative_path, rhs_project.as_ref()),
                    )
                };

            // Compare environment priorities
            let priority_ordering = || env_priority(lhs.kind).cmp(&env_priority(rhs.kind));

            // Compare conda prefixes
            let conda_ordering = || {
                if lhs.kind == Some(PythonEnvironmentKind::Conda) {
                    environment
                        .get_env_var("CONDA_PREFIX".to_string())
                        .map(|conda_prefix| {
                            let is_match = |exe: &Option<PathBuf>| {
                                exe.as_ref().is_some_and(|e| e.starts_with(&conda_prefix))
                            };
                            match (is_match(&lhs.executable), is_match(&rhs.executable)) {
                                (true, false) => Ordering::Less,
                                (false, true) => Ordering::Greater,
                                _ => Ordering::Equal,
                            }
                        })
                        .unwrap_or(Ordering::Equal)
                } else {
                    Ordering::Equal
                }
            };

            // Compare Python executables
            let exe_ordering = || lhs.executable.cmp(&rhs.executable);

            venv_ordering
                .then_with(proj_ordering)
                .then_with(priority_ordering)
                .then_with(conda_ordering)
                .then_with(exe_ordering)
        });

        let mut out_toolchains = Vec::new();
        for toolchain in toolchains {
            let Some(toolchain) = venv_to_toolchain(toolchain, fs).await else {
                continue;
            };
            out_toolchains.push(toolchain);
        }
        out_toolchains.dedup();
        ToolchainList {
            toolchains: out_toolchains,
            default: None,
            groups: Default::default(),
        }
    }
    fn meta(&self) -> ToolchainMetadata {
        ToolchainMetadata {
            term: SharedString::new_static("Virtual Environment"),
            new_toolchain_placeholder: SharedString::new_static(
                "A path to the python3 executable within a virtual environment, or path to virtual environment itself",
            ),
            manifest_name: ManifestName::from(SharedString::new_static("pyproject.toml")),
        }
    }

    async fn resolve(
        &self,
        path: PathBuf,
        env: Option<HashMap<String, String>>,
    ) -> anyhow::Result<Toolchain> {
        let fs = &*self.fs;
        let env = env.unwrap_or_default();
        let environment = EnvironmentApi::from_env(&env);
        let locators = pet::locators::create_locators(
            Arc::new(pet_conda::Conda::from(&environment)),
            Arc::new(pet_poetry::Poetry::from(&environment)),
            &environment,
        );
        let toolchain = pet::resolve::resolve_environment(&path, &locators, &environment)
            .context("Could not find a virtual environment in provided path")?;
        let venv = toolchain.resolved.unwrap_or(toolchain.discovered);
        venv_to_toolchain(venv, fs)
            .await
            .context("Could not convert a venv into a toolchain")
    }

    fn activation_script(
        &self,
        toolchain: &Toolchain,
        shell: ShellKind,
        cx: &App,
    ) -> BoxFuture<'static, Vec<String>> {
        let settings = TerminalSettings::get_global(cx);
        let conda_manager = settings
            .detect_venv
            .as_option()
            .map(|venv| venv.conda_manager)
            .unwrap_or(settings::CondaManager::Auto);

        let toolchain_clone = toolchain.clone();
        Box::pin(async move {
            let Ok(toolchain) =
                serde_json::from_value::<PythonToolchainData>(toolchain_clone.as_json.clone())
            else {
                return vec![];
            };

            log::debug!("(Python) Composing activation script for toolchain {toolchain:?}");

            let mut activation_script = vec![];

            match toolchain.environment.kind {
                Some(PythonEnvironmentKind::Conda) => {
                    if toolchain.environment.manager.is_none() {
                        return vec![];
                    };

                    let manager = match conda_manager {
                        settings::CondaManager::Conda => "conda",
                        settings::CondaManager::Mamba => "mamba",
                        settings::CondaManager::Micromamba => "micromamba",
                        settings::CondaManager::Auto => toolchain
                            .environment
                            .manager
                            .as_ref()
                            .and_then(|m| m.executable.file_name())
                            .and_then(|name| name.to_str())
                            .filter(|name| matches!(*name, "conda" | "mamba" | "micromamba"))
                            .unwrap_or("conda"),
                    };

                    // Activate micromamba shell in the child shell
                    // [required for micromamba]
                    if manager == "micromamba" {
                        match shell {
                            ShellKind::PowerShell | ShellKind::Pwsh => {
                                activation_script.push(format!(r#"(& {manager} shell hook --shell powershell) | Out-String | Invoke-Expression"#));
                            }
                            _ => {
                                let shell_name = micromamba_shell_name(shell);
                                activation_script.push(format!(
                                    r#"eval "$({manager} shell hook --shell {shell_name})""#
                                ));
                            }
                        }
                    }

                    // Only inject `{manager} activate <name>` when we have a
                    // safely-quotable name. Never silently fall back to
                    // `activate base`: a user with miniforge installed but a
                    // local uv/venv project should not have their terminal
                    // hijacked just because we couldn't resolve a name.
                    if let Some(name) = &toolchain.environment.name {
                        if let Some(quoted_name) = shell.try_quote(name) {
                            activation_script.push(format!("{manager} activate {quoted_name}"));
                        } else {
                            log::warn!(
                                "Conda environment name {:?} could not be safely quoted; \
                                 skipping terminal activation",
                                name
                            );
                        }
                    } else {
                        log::warn!("Conda toolchain has no name; skipping terminal activation");
                    }
                }
                Some(
                    PythonEnvironmentKind::Venv
                    | PythonEnvironmentKind::VirtualEnv
                    | PythonEnvironmentKind::Uv
                    | PythonEnvironmentKind::UvWorkspace
                    | PythonEnvironmentKind::Poetry,
                ) => {
                    if let Some(activation_scripts) = &toolchain.activation_scripts {
                        if let Some(activate_script_path) = activation_scripts.get(&shell) {
                            let activate_keyword = shell.activate_keyword();
                            if let Some(quoted) =
                                shell.try_quote(&activate_script_path.to_string_lossy())
                            {
                                activation_script.push(format!("{activate_keyword} {quoted}"));
                            }
                        }
                    }
                }
                Some(PythonEnvironmentKind::Pyenv) => {
                    let Some(manager) = &toolchain.environment.manager else {
                        return vec![];
                    };
                    let version = toolchain.environment.version.as_deref().unwrap_or("system");
                    let pyenv = &manager.executable;
                    let pyenv = pyenv.display();
                    activation_script.extend(match shell {
                        ShellKind::Fish => Some(format!("\"{pyenv}\" shell - fish {version}")),
                        ShellKind::Posix => Some(format!("\"{pyenv}\" shell - sh {version}")),
                        ShellKind::Nushell => Some(format!("^\"{pyenv}\" shell - nu {version}")),
                        ShellKind::PowerShell | ShellKind::Pwsh => None,
                        ShellKind::Csh => None,
                        ShellKind::Tcsh => None,
                        ShellKind::Cmd => None,
                        ShellKind::Rc => None,
                        ShellKind::Xonsh => None,
                        ShellKind::Elvish => None,
                    })
                }
                _ => {}
            }
            activation_script
        })
    }
}

async fn venv_to_toolchain(venv: PythonEnvironment, fs: &dyn Fs) -> Option<Toolchain> {
    let mut name = String::from("Python");
    if let Some(ref version) = venv.version {
        _ = write!(name, " {version}");
    }

    let name_and_kind = match (&venv.name, &venv.kind) {
        (Some(name), Some(kind)) => Some(format!("({name}; {})", python_env_kind_display(kind))),
        (Some(name), None) => Some(format!("({name})")),
        (None, Some(kind)) => Some(format!("({})", python_env_kind_display(kind))),
        (None, None) => None,
    };

    if let Some(nk) = name_and_kind {
        _ = write!(name, " {nk}");
    }

    let mut activation_scripts = HashMap::default();
    match venv.kind {
        Some(
            PythonEnvironmentKind::Venv
            | PythonEnvironmentKind::VirtualEnv
            | PythonEnvironmentKind::Uv
            | PythonEnvironmentKind::UvWorkspace
            | PythonEnvironmentKind::Poetry,
        ) => resolve_venv_activation_scripts(&venv, fs, &mut activation_scripts).await,
        _ => {}
    }
    let data = PythonToolchainData {
        environment: venv,
        activation_scripts: Some(activation_scripts),
    };

    Some(Toolchain {
        name: name.into(),
        path: data
            .environment
            .executable
            .as_ref()?
            .to_str()?
            .to_owned()
            .into(),
        language_name: LanguageName::new_static("Python"),
        as_json: serde_json::to_value(data).ok()?,
    })
}

const BINARY_DIR: &str = if cfg!(target_os = "windows") {
    "Scripts"
} else {
    "bin"
};

async fn resolve_venv_activation_scripts(
    venv: &PythonEnvironment,
    fs: &dyn Fs,
    activation_scripts: &mut HashMap<ShellKind, PathBuf>,
) {
    log::debug!("(Python) Resolving activation scripts for venv toolchain {venv:?}");
    if let Some(prefix) = &venv.prefix {
        for (shell_kind, script_name) in &[
            (ShellKind::Posix, "activate"),
            (ShellKind::Rc, "activate"),
            (ShellKind::Csh, "activate.csh"),
            (ShellKind::Tcsh, "activate.csh"),
            (ShellKind::Fish, "activate.fish"),
            (ShellKind::Nushell, "activate.nu"),
            (ShellKind::PowerShell, "activate.ps1"),
            (ShellKind::Pwsh, "activate.ps1"),
            (ShellKind::Cmd, "activate.bat"),
            (ShellKind::Xonsh, "activate.xsh"),
        ] {
            let path = prefix.join(BINARY_DIR).join(script_name);

            log::debug!("Trying path: {}", path.display());

            if fs.is_file(&path).await {
                activation_scripts.insert(*shell_kind, path);
            }
        }
    }
}

pub struct EnvironmentApi<'a> {
    global_search_locations: Arc<Mutex<Vec<PathBuf>>>,
    project_env: &'a HashMap<String, String>,
    pet_env: pet_core::os_environment::EnvironmentApi,
}

impl<'a> EnvironmentApi<'a> {
    pub fn from_env(project_env: &'a HashMap<String, String>) -> Self {
        let paths = project_env
            .get("PATH")
            .map(|p| std::env::split_paths(p).collect())
            .unwrap_or_default();

        EnvironmentApi {
            global_search_locations: Arc::new(Mutex::new(paths)),
            project_env,
            pet_env: pet_core::os_environment::EnvironmentApi::new(),
        }
    }

    fn user_home(&self) -> Option<PathBuf> {
        self.project_env
            .get("HOME")
            .or_else(|| self.project_env.get("USERPROFILE"))
            .map(|home| pet_fs::path::norm_case(PathBuf::from(home)))
            .or_else(|| self.pet_env.get_user_home())
    }
}

impl pet_core::os_environment::Environment for EnvironmentApi<'_> {
    fn get_user_home(&self) -> Option<PathBuf> {
        self.user_home()
    }

    fn get_root(&self) -> Option<PathBuf> {
        None
    }

    fn get_env_var(&self, key: String) -> Option<String> {
        self.project_env
            .get(&key)
            .cloned()
            .or_else(|| self.pet_env.get_env_var(key))
    }

    fn get_know_global_search_locations(&self) -> Vec<PathBuf> {
        if self.global_search_locations.lock().is_empty() {
            let mut paths = std::env::split_paths(
                &self
                    .get_env_var("PATH".to_string())
                    .or_else(|| self.get_env_var("Path".to_string()))
                    .unwrap_or_default(),
            )
            .collect::<Vec<PathBuf>>();

            log::trace!("Env PATH: {:?}", paths);
            for p in self.pet_env.get_know_global_search_locations() {
                if !paths.contains(&p) {
                    paths.push(p);
                }
            }

            let mut paths = paths
                .into_iter()
                .filter(|p| p.exists())
                .collect::<Vec<PathBuf>>();

            self.global_search_locations.lock().append(&mut paths);
        }
        self.global_search_locations.lock().clone()
    }
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext as _, BorrowAppContext, Context, TestAppContext};
    use language::{AutoindentMode, Buffer};
    use settings::SettingsStore;
    use std::num::NonZeroU32;

    use crate::python::{
        basedpyright_workspace_configuration, normalize_pyright_analysis_configuration,
    };

    #[test]
    fn test_normalize_legacy_basedpyright_analysis_configuration() {
        let mut workspace_configuration = serde_json::json!({
            "basedpyright.analysis": {
                "diagnosticMode": "workspace",
                "typeCheckingMode": "basic"
            }
        });

        normalize_pyright_analysis_configuration(&mut workspace_configuration, "basedpyright");

        assert_eq!(
            workspace_configuration,
            serde_json::json!({
                "basedpyright": {
                    "analysis": {
                        "diagnosticMode": "workspace",
                        "typeCheckingMode": "basic"
                    }
                },
                "basedpyright.analysis": {
                    "diagnosticMode": "workspace",
                    "typeCheckingMode": "basic"
                }
            })
        );
    }

    #[test]
    fn test_normalize_nested_basedpyright_analysis_configuration() {
        let mut workspace_configuration = serde_json::json!({
            "basedpyright": {
                "analysis": {
                    "diagnosticMode": "workspace"
                },
                "unrelated": true
            }
        });

        normalize_pyright_analysis_configuration(&mut workspace_configuration, "basedpyright");

        assert_eq!(
            workspace_configuration,
            serde_json::json!({
                "basedpyright": {
                    "analysis": {
                        "diagnosticMode": "workspace"
                    },
                    "unrelated": true
                },
                "basedpyright.analysis": {
                    "diagnosticMode": "workspace"
                }
            })
        );
    }

    #[test]
    fn test_normalize_merges_both_analysis_configuration_with_conflicts() {
        let mut workspace_configuration = serde_json::json!({
            "basedpyright": {
                "analysis": {
                    "diagnosticMode": "workspace",
                }
            },
            "basedpyright.analysis": {
                "typeCheckingMode": "standard",
                "diagnosticMode": "openFilesOnly"
            }
        });

        normalize_pyright_analysis_configuration(&mut workspace_configuration, "basedpyright");

        // Settings from both forms survive, with the nested form winning on conflicting keys.
        assert_eq!(
            workspace_configuration,
            serde_json::json!({
                "basedpyright": {
                    "analysis": {
                        "diagnosticMode": "workspace",
                        "typeCheckingMode": "standard",
                    }
                },
                "basedpyright.analysis": {
                    "diagnosticMode": "workspace",
                    "typeCheckingMode": "standard",
                }
            })
        );
    }

    #[test]
    fn test_normalize_pyright_analysis_configuration() {
        let mut workspace_configuration = serde_json::json!({
            "python.analysis": {
                "diagnosticMode": "workspace",
                "typeCheckingMode": "basic"
            }
        });

        normalize_pyright_analysis_configuration(&mut workspace_configuration, "python");

        assert_eq!(
            workspace_configuration,
            serde_json::json!({
                "python": {
                    "analysis": {
                        "diagnosticMode": "workspace",
                        "typeCheckingMode": "basic"
                    }
                },
                "python.analysis": {
                    "diagnosticMode": "workspace",
                    "typeCheckingMode": "basic"
                }
            })
        );
    }

    #[test]
    fn test_basedpyright_workspace_configuration_uses_the_project_toolchain() {
        let venv_toolchain = language::Toolchain {
            name: "Python 3.12.4 (.venv; venv)".into(),
            path: "/project/.venv/bin/python".into(),
            language_name: language::LanguageName::new_static("Python"),
            as_json: serde_json::json!({
                "executable": "/project/.venv/bin/python",
                "kind": "Venv",
                "prefix": "/project/.venv",
            }),
        };

        let configuration = basedpyright_workspace_configuration(None, Some(&venv_toolchain));
        assert_eq!(
            configuration["python"]["pythonPath"],
            "/project/.venv/bin/python"
        );
        assert_eq!(
            configuration["basedpyright"]["analysis"]["typeCheckingMode"],
            "standard"
        );

        let configuration = basedpyright_workspace_configuration(
            Some(serde_json::json!({
                "python": { "venvPath": "/elsewhere" },
                "basedpyright": { "analysis": { "typeCheckingMode": "strict" } },
            })),
            Some(&venv_toolchain),
        );
        assert_eq!(
            configuration["python"],
            serde_json::json!({ "venvPath": "/elsewhere" }),
            "an interpreter the user picked wins over the detected toolchain"
        );
        assert_eq!(
            configuration["basedpyright"]["analysis"]["typeCheckingMode"],
            "strict"
        );

        let configuration = basedpyright_workspace_configuration(None, None);
        assert_eq!(configuration.get("python"), None);
    }

    #[gpui::test]
    async fn test_conda_activation_script_injection(cx: &mut TestAppContext) {
        use language::{LanguageName, Toolchain, ToolchainLister};
        use settings::{CondaManager, VenvSettings};
        use util::shell::ShellKind;

        use crate::python::PythonToolchainProvider;

        cx.executor().allow_parking();

        cx.update(|cx| {
            let test_settings = SettingsStore::test(cx);
            cx.set_global(test_settings);
            cx.update_global::<SettingsStore, _>(|store, cx| {
                store.update_user_settings(cx, |s| {
                    s.terminal
                        .get_or_insert_with(Default::default)
                        .project
                        .detect_venv = Some(VenvSettings::On {
                        activate_script: None,
                        venv_name: None,
                        directories: None,
                        conda_manager: Some(CondaManager::Conda),
                    });
                });
            });
        });

        let fs = project::FakeFs::new(cx.executor());
        let provider = PythonToolchainProvider::new(fs);
        let malicious_name = "foo; rm -rf /";

        let manager_executable = std::env::current_exe().unwrap();

        let data = serde_json::json!({
            "name": malicious_name,
            "kind": "Conda",
            "executable": "/tmp/conda/bin/python",
            "version": serde_json::Value::Null,
            "prefix": serde_json::Value::Null,
            "arch": serde_json::Value::Null,
            "displayName": serde_json::Value::Null,
            "project": serde_json::Value::Null,
            "symlinks": serde_json::Value::Null,
            "manager": {
                "executable": manager_executable,
                "version": serde_json::Value::Null,
                "tool": "Conda",
            },
        });

        let toolchain = Toolchain {
            name: "test".into(),
            path: "/tmp/conda".into(),
            language_name: LanguageName::new_static("Python"),
            as_json: data,
        };

        let script = cx
            .update(|cx| provider.activation_script(&toolchain, ShellKind::Posix, cx))
            .await;

        assert!(
            script
                .iter()
                .any(|s| s.contains("conda activate 'foo; rm -rf /'")),
            "Script should contain quoted malicious name, actual: {:?}",
            script
        );
    }

    #[gpui::test]
    async fn test_conda_activation_skips_when_name_missing(cx: &mut TestAppContext) {
        use language::{LanguageName, Toolchain, ToolchainLister};
        use settings::{CondaManager, VenvSettings};
        use util::shell::ShellKind;

        use crate::python::PythonToolchainProvider;

        cx.executor().allow_parking();

        cx.update(|cx| {
            let test_settings = SettingsStore::test(cx);
            cx.set_global(test_settings);
            cx.update_global::<SettingsStore, _>(|store, cx| {
                store.update_user_settings(cx, |s| {
                    s.terminal
                        .get_or_insert_with(Default::default)
                        .project
                        .detect_venv = Some(VenvSettings::On {
                        activate_script: None,
                        venv_name: None,
                        directories: None,
                        conda_manager: Some(CondaManager::Conda),
                    });
                });
            });
        });

        let fs = project::FakeFs::new(cx.executor());
        let provider = PythonToolchainProvider::new(fs);
        let manager_executable = std::env::current_exe().unwrap();

        let data = serde_json::json!({
            "name": serde_json::Value::Null,
            "kind": "Conda",
            "executable": "/tmp/conda/bin/python",
            "version": serde_json::Value::Null,
            "prefix": serde_json::Value::Null,
            "arch": serde_json::Value::Null,
            "displayName": serde_json::Value::Null,
            "project": serde_json::Value::Null,
            "symlinks": serde_json::Value::Null,
            "manager": {
                "executable": manager_executable,
                "version": serde_json::Value::Null,
                "tool": "Conda",
            },
        });

        let toolchain = Toolchain {
            name: "test".into(),
            path: "/tmp/conda".into(),
            language_name: LanguageName::new_static("Python"),
            as_json: data,
        };

        let script = cx
            .update(|cx| provider.activation_script(&toolchain, ShellKind::Posix, cx))
            .await;

        assert!(
            script.is_empty(),
            "Nameless conda toolchains must not fall back to `conda activate base`, actual: {:?}",
            script
        );
    }

    #[gpui::test]
    async fn test_conda_activation_skips_unquotable_name(cx: &mut TestAppContext) {
        use language::{LanguageName, Toolchain, ToolchainLister};
        use settings::{CondaManager, VenvSettings};
        use util::shell::ShellKind;

        use crate::python::PythonToolchainProvider;

        cx.executor().allow_parking();

        cx.update(|cx| {
            let test_settings = SettingsStore::test(cx);
            cx.set_global(test_settings);
            cx.update_global::<SettingsStore, _>(|store, cx| {
                store.update_user_settings(cx, |s| {
                    s.terminal
                        .get_or_insert_with(Default::default)
                        .project
                        .detect_venv = Some(VenvSettings::On {
                        activate_script: None,
                        venv_name: None,
                        directories: None,
                        conda_manager: Some(CondaManager::Conda),
                    });
                });
            });
        });

        let fs = project::FakeFs::new(cx.executor());
        let provider = PythonToolchainProvider::new(fs);
        // shlex::try_quote rejects strings containing a NUL byte, so this name
        // is guaranteed to fail the Posix quoting path.
        let unquotable_name = "foo\0bar";
        let manager_executable = std::env::current_exe().unwrap();

        let data = serde_json::json!({
            "name": unquotable_name,
            "kind": "Conda",
            "executable": "/tmp/conda/bin/python",
            "version": serde_json::Value::Null,
            "prefix": serde_json::Value::Null,
            "arch": serde_json::Value::Null,
            "displayName": serde_json::Value::Null,
            "project": serde_json::Value::Null,
            "symlinks": serde_json::Value::Null,
            "manager": {
                "executable": manager_executable,
                "version": serde_json::Value::Null,
                "tool": "Conda",
            },
        });

        let toolchain = Toolchain {
            name: "test".into(),
            path: "/tmp/conda".into(),
            language_name: LanguageName::new_static("Python"),
            as_json: data,
        };

        let script = cx
            .update(|cx| provider.activation_script(&toolchain, ShellKind::Posix, cx))
            .await;

        assert!(
            !script.iter().any(|s| s.contains("conda activate")),
            "Unquotable conda env names must not emit any `conda activate` line, actual: {:?}",
            script
        );
    }

    #[gpui::test]
    async fn test_python_autoindent(cx: &mut TestAppContext) {
        cx.executor().set_block_on_ticks(usize::MAX..=usize::MAX);
        let language = crate::language("python", tree_sitter_python::LANGUAGE.into());
        cx.update(|cx| {
            let test_settings = SettingsStore::test(cx);
            cx.set_global(test_settings);
            cx.update_global::<SettingsStore, _>(|store, cx| {
                store.update_user_settings(cx, |s| {
                    s.project.all_languages.defaults.tab_size = NonZeroU32::new(2);
                });
            });
        });

        cx.new(|cx| {
            let mut buffer = Buffer::local("", cx).with_language(language, cx);
            let append = |buffer: &mut Buffer, text: &str, cx: &mut Context<Buffer>| {
                let ix = buffer.len();
                buffer.edit([(ix..ix, text)], Some(AutoindentMode::EachLine), cx);
            };

            // indent after "def():"
            append(&mut buffer, "def a():\n", cx);
            assert_eq!(buffer.text(), "def a():\n  ");

            // preserve indent after blank line
            append(&mut buffer, "\n  ", cx);
            assert_eq!(buffer.text(), "def a():\n  \n  ");

            // indent after "if"
            append(&mut buffer, "if a:\n  ", cx);
            assert_eq!(buffer.text(), "def a():\n  \n  if a:\n    ");

            // preserve indent after statement
            append(&mut buffer, "b()\n", cx);
            assert_eq!(buffer.text(), "def a():\n  \n  if a:\n    b()\n    ");

            // preserve indent after statement
            append(&mut buffer, "else", cx);
            assert_eq!(buffer.text(), "def a():\n  \n  if a:\n    b()\n    else");

            // dedent "else""
            append(&mut buffer, ":", cx);
            assert_eq!(buffer.text(), "def a():\n  \n  if a:\n    b()\n  else:");

            // indent lines after else
            append(&mut buffer, "\n", cx);
            assert_eq!(
                buffer.text(),
                "def a():\n  \n  if a:\n    b()\n  else:\n    "
            );

            // indent after an open paren. the closing paren is not indented
            // because there is another token before it on the same line.
            append(&mut buffer, "foo(\n1)", cx);
            assert_eq!(
                buffer.text(),
                "def a():\n  \n  if a:\n    b()\n  else:\n    foo(\n      1)"
            );

            // dedent the closing paren if it is shifted to the beginning of the line
            let argument_ix = buffer.text().find('1').unwrap();
            buffer.edit(
                [(argument_ix..argument_ix + 1, "")],
                Some(AutoindentMode::EachLine),
                cx,
            );
            assert_eq!(
                buffer.text(),
                "def a():\n  \n  if a:\n    b()\n  else:\n    foo(\n    )"
            );

            // preserve indent after the close paren
            append(&mut buffer, "\n", cx);
            assert_eq!(
                buffer.text(),
                "def a():\n  \n  if a:\n    b()\n  else:\n    foo(\n    )\n    "
            );

            // manually outdent the last line
            let end_whitespace_ix = buffer.len() - 4;
            buffer.edit(
                [(end_whitespace_ix..buffer.len(), "")],
                Some(AutoindentMode::EachLine),
                cx,
            );
            assert_eq!(
                buffer.text(),
                "def a():\n  \n  if a:\n    b()\n  else:\n    foo(\n    )\n"
            );

            // preserve the newly reduced indentation on the next newline
            append(&mut buffer, "\n", cx);
            assert_eq!(
                buffer.text(),
                "def a():\n  \n  if a:\n    b()\n  else:\n    foo(\n    )\n\n"
            );

            // reset to a for loop statement
            let statement = "for i in range(10):\n  print(i)\n";
            buffer.edit([(0..buffer.len(), statement)], None, cx);

            // insert single line comment after each line
            let eol_ixs = statement
                .char_indices()
                .filter_map(|(ix, c)| if c == '\n' { Some(ix) } else { None })
                .collect::<Vec<usize>>();
            let editions = eol_ixs
                .iter()
                .enumerate()
                .map(|(i, &eol_ix)| (eol_ix..eol_ix, format!(" # comment {}", i + 1)))
                .collect::<Vec<(std::ops::Range<usize>, String)>>();
            buffer.edit(editions, Some(AutoindentMode::EachLine), cx);
            assert_eq!(
                buffer.text(),
                "for i in range(10): # comment 1\n  print(i) # comment 2\n"
            );

            // reset to a simple if statement
            buffer.edit([(0..buffer.len(), "if a:\n  b(\n  )")], None, cx);

            // dedent "else" on the line after a closing paren
            append(&mut buffer, "\n  else:\n", cx);
            assert_eq!(buffer.text(), "if a:\n  b(\n  )\nelse:\n  ");

            buffer
        });
    }

    mod pyproject_manifest_tests {
        use std::collections::HashSet;
        use std::sync::Arc;

        use language::{ManifestDelegate, ManifestProvider, ManifestQuery};
        use settings::WorktreeId;
        use util::rel_path::RelPath;

        use crate::python::PyprojectTomlManifestProvider;

        struct FakeManifestDelegate {
            existing_files: HashSet<&'static str>,
        }

        impl ManifestDelegate for FakeManifestDelegate {
            fn worktree_id(&self) -> WorktreeId {
                WorktreeId::from_usize(0)
            }

            fn exists(&self, path: &RelPath, _is_dir: Option<bool>) -> bool {
                self.existing_files.contains(path.as_unix_str())
            }
        }

        fn search(files: &[&'static str], query_path: &str) -> Option<Arc<RelPath>> {
            let delegate = Arc::new(FakeManifestDelegate {
                existing_files: files.iter().copied().collect(),
            });
            let provider = PyprojectTomlManifestProvider;
            provider.search(ManifestQuery {
                path: RelPath::from_unix_str(query_path).unwrap().into(),
                depth: 10,
                delegate,
            })
        }

        #[test]
        fn test_simple_project_no_lockfile() {
            let result = search(&["project/pyproject.toml"], "project/src/main.py");
            assert_eq!(result.as_deref(), RelPath::from_unix_str("project").ok());
        }

        #[test]
        fn test_uv_workspace_returns_root() {
            let result = search(
                &[
                    "pyproject.toml",
                    "uv.lock",
                    "packages/subproject/pyproject.toml",
                ],
                "packages/subproject/src/main.py",
            );
            assert_eq!(result.as_deref(), RelPath::from_unix_str("").ok());
        }

        #[test]
        fn test_poetry_workspace_returns_root() {
            let result = search(
                &["pyproject.toml", "poetry.lock", "libs/mylib/pyproject.toml"],
                "libs/mylib/src/main.py",
            );
            assert_eq!(result.as_deref(), RelPath::from_unix_str("").ok());
        }

        #[test]
        fn test_pdm_workspace_returns_root() {
            let result = search(
                &[
                    "pyproject.toml",
                    "pdm.lock",
                    "packages/mypackage/pyproject.toml",
                ],
                "packages/mypackage/src/main.py",
            );
            assert_eq!(result.as_deref(), RelPath::from_unix_str("").ok());
        }

        #[test]
        fn test_independent_subprojects_no_lockfile_at_root() {
            let result_a = search(
                &["project-a/pyproject.toml", "project-b/pyproject.toml"],
                "project-a/src/main.py",
            );
            assert_eq!(
                result_a.as_deref(),
                RelPath::from_unix_str("project-a").ok()
            );

            let result_b = search(
                &["project-a/pyproject.toml", "project-b/pyproject.toml"],
                "project-b/src/main.py",
            );
            assert_eq!(
                result_b.as_deref(),
                RelPath::from_unix_str("project-b").ok()
            );
        }

        #[test]
        fn test_no_pyproject_returns_none() {
            let result = search(&[], "src/main.py");
            assert_eq!(result, None);
        }

        #[test]
        fn test_subproject_with_own_lockfile_and_workspace_root() {
            // Both root and subproject have lockfiles; should return root (outermost)
            let result = search(
                &[
                    "pyproject.toml",
                    "uv.lock",
                    "packages/sub/pyproject.toml",
                    "packages/sub/uv.lock",
                ],
                "packages/sub/src/main.py",
            );
            assert_eq!(result.as_deref(), RelPath::from_unix_str("").ok());
        }

        #[test]
        fn test_depth_limits_search() {
            let delegate = Arc::new(FakeManifestDelegate {
                existing_files: ["pyproject.toml", "uv.lock", "deep/nested/pyproject.toml"]
                    .into_iter()
                    .collect(),
            });
            let provider = PyprojectTomlManifestProvider;
            // depth=3 from "deep/nested/src/main.py" searches:
            //   "deep/nested/src/main.py", "deep/nested/src", and "deep/nested"
            // It won't reach "deep" or root ""
            let result = provider.search(ManifestQuery {
                path: RelPath::from_unix_str("deep/nested/src/main.py")
                    .unwrap()
                    .into(),
                depth: 3,
                delegate,
            });
            assert_eq!(
                result.as_deref(),
                RelPath::from_unix_str("deep/nested").ok()
            );
        }
    }
}
