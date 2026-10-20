use std::sync::Arc;

use collections::HashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use settings_macros::{MergeFrom, with_fallible_options};

#[with_fallible_options]
#[derive(Debug, PartialEq, Clone, Default, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct ExtensionSettingsContent {
    /// The extensions Flint installs from the extension registry the first time it opens a
    /// project that needs them: `java` for Maven and Gradle projects, `csharp` for .NET
    /// solutions and projects. Set one to `false` to never install it automatically.
    ///
    /// Default: { "java": true, "csharp": true }
    #[serde(default)]
    pub auto_install_extensions: HashMap<Arc<str>, bool>,
    /// The capabilities granted to extensions.
    pub granted_extension_capabilities: Option<Vec<ExtensionCapabilityContent>>,
}

/// A capability for an extension.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExtensionCapabilityContent {
    #[serde(rename = "process:exec")]
    ProcessExec {
        /// The command to execute.
        command: String,
        /// The arguments to pass to the command. Use `*` for a single wildcard argument.
        /// If the last element is `**`, then any trailing arguments are allowed.
        args: Vec<String>,
    },
    DownloadFile {
        host: String,
        path: Vec<String>,
    },
    #[serde(rename = "npm:install")]
    NpmInstallPackage {
        package: String,
    },
}
