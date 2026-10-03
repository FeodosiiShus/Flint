//! Contains helper functions for constructing URLs to various Zed-related pages.

use gpui::App;

use crate::ZED_SERVER_URL;

/// Returns the URL to Zed's Agent sandboxing documentation.
///
/// Pass `section` to deep-link to a specific section anchor on the page (for
/// example, `Some("installing-bubblewrap")`); pass `None` to link to the top of
/// the page.
pub fn sandboxing_docs(section: Option<&str>, cx: &App) -> String {
    let base = release_channel::docs_url("ai/sandboxing", cx);
    match section {
        Some(section) => format!("{base}#{section}"),
        None => base,
    }
}
pub fn llm_provider_docs(cx: &App) -> String {
    release_channel::docs_url("ai/llm-providers", cx)
}

/// Returns the URL to Zed's ACP registry blog post.
pub fn acp_registry_blog(_cx: &App) -> String {
    format!("{ZED_SERVER_URL}/blog/acp-registry")
}
