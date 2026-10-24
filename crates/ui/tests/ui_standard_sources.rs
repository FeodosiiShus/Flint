//! Keeps every crate on the standard components: a hand-made close button or floating surface
//! anywhere in the workspace fails here with its file and line.

use std::path::{Path, PathBuf};

/// Close glyphs that are status icons, not buttons.
const CLOSE_GLYPH_ALLOWLIST: &[(&str, &str)] = &[
    (
        "crates/ui/src/components/button/close_button.rs",
        "the standard close button itself",
    ),
    (
        "crates/ui/src/components/indicator.rs",
        "an error indicator glyph",
    ),
    (
        "crates/ui/src/components/ai/thread_item.rs",
        "a failed-thread status glyph",
    ),
    (
        "crates/editor/src/items.rs",
        "a deleted-file status glyph in the tab",
    ),
    ("crates/icons/src/icons.rs", "the icon names themselves"),
    (
        "crates/ui/tests/ui_standard_sources.rs",
        "the patterns this test looks for",
    ),
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root exists")
}

fn rust_sources(directory: &Path, sources: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if name != "target" && !name.starts_with('.') {
                rust_sources(&path, sources);
            }
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
}

struct Source {
    relative: String,
    text: String,
}

fn workspace_sources() -> Vec<Source> {
    let root = workspace_root();
    let mut paths = Vec::new();
    rust_sources(&root.join("crates"), &mut paths);
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let relative = path
                .strip_prefix(&root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            Some(Source { relative, text })
        })
        .collect()
}

fn line_of(text: &str, offset: usize) -> usize {
    text[..offset].matches('\n').count() + 1
}

/// The text of the call starting at `open` (the index of its `(`), up to the matching `)`.
fn call_arguments(text: &str, open: usize) -> &str {
    let mut depth = 0usize;
    for (index, character) in text[open..].char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &text[open..open + index + 1];
                }
            }
            _ => {}
        }
    }
    &text[open..]
}

/// The arguments of the innermost call that encloses `offset`, or the rest of the statement.
fn enclosing_call(text: &str, offset: usize) -> &str {
    let mut depth = 0usize;
    for (index, character) in text[..offset].char_indices().rev() {
        match character {
            ')' => depth += 1,
            '(' if depth == 0 => return call_arguments(text, index),
            '(' => depth -= 1,
            ';' | '{' if depth == 0 => break,
            _ => {}
        }
    }
    let rest = &text[offset..];
    &rest[..rest.find(';').unwrap_or(rest.len())]
}

fn is_standard_component_source(relative: &str) -> bool {
    relative.starts_with("crates/ui/")
}

#[test]
fn every_close_button_is_the_standard_close_button() {
    let mut offenders = Vec::new();
    for source in workspace_sources() {
        if CLOSE_GLYPH_ALLOWLIST
            .iter()
            .any(|(path, _)| *path == source.relative)
        {
            continue;
        }
        for (offset, _) in source.text.match_indices("IconButton::new(") {
            let open = offset + "IconButton::new".len();
            let arguments = call_arguments(&source.text, open);
            if arguments.contains("IconName::Close") {
                offenders.push(format!(
                    "{}:{} builds an IconButton with a close glyph; use ui::CloseButton",
                    source.relative,
                    line_of(&source.text, offset)
                ));
            }
        }
        for glyph in [
            "IconName::CloseSmallHovered",
            "IconName::CloseSmall",
            "Icon::new(IconName::Close)",
        ] {
            for (offset, _) in source.text.match_indices(glyph) {
                offenders.push(format!(
                    "{}:{} draws its own close glyph ({glyph}); use ui::CloseButton",
                    source.relative,
                    line_of(&source.text, offset)
                ));
            }
        }
    }
    offenders.dedup();
    assert!(
        offenders.is_empty(),
        "close buttons outside the standard:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn every_floating_surface_uses_the_standard_chrome() {
    let mut offenders = Vec::new();
    for source in workspace_sources() {
        if is_standard_component_source(&source.relative) {
            continue;
        }
        for helper in [
            ".elevation_1(",
            ".elevation_2(",
            ".elevation_3(",
            ".elevation_1_borderless(",
            ".elevation_2_borderless(",
            ".elevation_3_borderless(",
        ] {
            for (offset, _) in source.text.match_indices(helper) {
                offenders.push(format!(
                    "{}:{} uses {helper}..); use popup_surface, dialog_surface, \
                     balloon_surface or tooltip_surface",
                    source.relative,
                    line_of(&source.text, offset)
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "floating surfaces outside the standard:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn every_dialog_footer_lays_out_its_buttons_through_the_standard_row() {
    let mut offenders = Vec::new();
    for source in workspace_sources() {
        if is_standard_component_source(&source.relative) {
            continue;
        }
        for (offset, _) in source.text.match_indices("ModalFooter::new()") {
            let footer = enclosing_call(&source.text, offset);
            if footer.contains(".end_slot(") && footer.contains("DialogButton::new(") {
                offenders.push(format!(
                    "{}:{} puts its dialog buttons in end_slot; add each with end_button",
                    source.relative,
                    line_of(&source.text, offset)
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "dialog footers outside the standard:\n{}",
        offenders.join("\n")
    );
}

/// Dialogs whose title and buttons live in their own content instead of a header row.
const HEADERLESS_DIALOGS: &[(&str, &str)] = &[(
    "crates/git_ui/src/commit_modal.rs",
    "IntelliJ's commit card keeps the branch, the hints and the buttons in its own footer",
)];

#[test]
fn every_dialog_has_the_standard_header() {
    let mut offenders = Vec::new();
    for source in workspace_sources() {
        if is_standard_component_source(&source.relative)
            || HEADERLESS_DIALOGS
                .iter()
                .any(|(path, _)| *path == source.relative)
        {
            continue;
        }
        if let Some(offset) = source.text.find(".dialog_surface(")
            && !source.text.contains("ModalHeader::new()")
        {
            offenders.push(format!(
                "{}:{} draws a dialog without ModalHeader; its title and close button must be \
                 the standard ones",
                source.relative,
                line_of(&source.text, offset)
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "dialogs outside the standard:\n{}",
        offenders.join("\n")
    );
}

/// Hand-built buttons that are themselves reusable components with IntelliJ metrics of their own;
/// every other button is built from `crates/ui`, so buttons look and behave the same everywhere.
const BUTTON_COMPONENT_ALLOWLIST: &[(&str, &str)] = &[
    (
        "crates/workspace/src/tool_window_bar.rs",
        "the tool window stripe buttons",
    ),
    (
        "crates/title_bar/src/toolbar_widgets.rs",
        "the main toolbar widgets and search field",
    ),
    (
        "crates/settings_ui/src/components/number_field.rs",
        "the number field's stepper arrows",
    ),
    (
        "crates/settings_ui/src/settings_ui.rs",
        "the focus ring around a standard DialogButton",
    ),
    (
        "crates/gpui/src/_accessibility.rs",
        "GPUI's accessibility documentation",
    ),
    ("crates/gpui/examples/a11y.rs", "a GPUI example"),
];

#[test]
fn every_button_is_built_from_the_standard_components() {
    let mut offenders = Vec::new();
    for source in workspace_sources() {
        if source.relative.starts_with("crates/ui/") {
            continue;
        }
        if BUTTON_COMPONENT_ALLOWLIST
            .iter()
            .any(|(path, _)| *path == source.relative)
        {
            continue;
        }
        for (offset, _) in source.text.match_indices("role(Role::Button)") {
            offenders.push(format!(
                "{}:{} makes its own button; use ui::Button, IconButton, DialogButton or \
                 ButtonLike so it looks and behaves like every other button",
                source.relative,
                line_of(&source.text, offset)
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "buttons outside the standard components:\n{}",
        offenders.join("\n")
    );
}
