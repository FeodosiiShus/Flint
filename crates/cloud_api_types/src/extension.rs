use serde::{Deserialize, Serialize};
use strum::EnumString;

#[derive(
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    EnumString,
    strum::Display,
    strum::EnumIter,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum ExtensionProvides {
    Languages,
    Themes,
    IconThemes,
    Grammars,
    LanguageServers,
    DebugAdapters,
    ContextServers,
    /// Deprecated
    AgentServers,
    /// Deprecated
    SlashCommands,
    /// Deprecated
    IndexedDocsProviders,
    Snippets,
}

impl ExtensionProvides {
    pub fn is_deprecated(&self) -> bool {
        matches!(
            self,
            ExtensionProvides::ContextServers
                | ExtensionProvides::AgentServers
                | ExtensionProvides::SlashCommands
                | ExtensionProvides::IndexedDocsProviders
        )
    }
}
