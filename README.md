> [!IMPORTANT]
> Remove this line to confirm you've reviewed this PR before submitting.

# Flint

Flint is a personal fork of [Zed](https://github.com/zed-industries/zed). It is built only for macOS on Apple Silicon (`aarch64-apple-darwin`), only in GitHub Actions, by the [`build-macos.yml`](.github/workflows/build-macos.yml) workflow.

## Getting a build

- Every push runs the [`checks.yml`](.github/workflows/checks.yml) workflow: `cargo fmt --check`, `cargo check` of the app, and the tests of the crates Flint changes. It does not build the app.
- The app is built only on demand: Actions → Build macOS → Run workflow → branch `main`. The DMG is attached to the run as the `flint-macos-aarch64` artifact.
- Running Build macOS from a `v*` tag also publishes a GitHub Release with the DMG attached.

To cut a release:

```sh
git tag vX.Y.Z
git push origin vX.Y.Z
```

Then start Actions → Build macOS → Run workflow and pick the `vX.Y.Z` tag in "Use workflow from".

## First launch

The app is ad-hoc signed and not notarized, so macOS Gatekeeper blocks it after download. After copying it to `/Applications`, remove the quarantine attribute:

```sh
xattr -dr com.apple.quarantine "/Applications/Flint.app"
```

The app is built on the `dev` release channel and is named "Flint". It has no auto-update, Zed account sign-in or onboarding flow.

## Background image

Flint can draw a picture behind the editor and the tool windows, like WebStorm's [Background Image](https://www.jetbrains.com/help/webstorm/setting-background-image.html). There are two independent images:

- `editor_and_tools` — behind the editor, tabs, tool windows (project, git, agent, terminal, debugger panels), the title bar and the status bar.
- `empty_frame` — behind an editor pane that has no open files. Without it, the `editor_and_tools` image shows there.

### Choosing an image

- Command palette: `workspace: select background image`, `workspace: select empty frame background image`, `workspace: clear background image`. The pickers write the path into your user `settings.json`; clearing empties both paths.
- Settings window: Appearance → Background Image. Every field can be set for the user or for one project: switch between "User" and the project at the top of the Settings window. The "Choose Image…" and "Clear" buttons there run the same commands as the command palette.
- `settings.json`:

```json
{
  "background_image": {
    "editor_and_tools": {
      "path": "~/Pictures/background.png",
      "opacity": 15,
      "fill": "scale",
      "anchor": "center",
      "flip_horizontal": false,
      "flip_vertical": false
    },
    "empty_frame": {
      "path": "~/Pictures/logo.png",
      "opacity": 30,
      "fill": "plain",
      "anchor": "bottom_right"
    }
  }
}
```

| Key | Values | Default |
| --- | --- | --- |
| `path` | Absolute path or a path starting with `~/`. An empty string turns the image off. PNG, JPEG, GIF and WebP (first frame), BMP, TIFF, ICO, TGA, QOI, PNM, HDR, EXR, DDS and Farbfeld are supported. | `""` |
| `opacity` | `0`–`100`, where `100` is fully opaque and `0` hides the image. | `15` |
| `fill` | `plain` — one copy at its original size; `scale` — scaled with its proportions kept so it covers the whole area, the window for `editor_and_tools` and the empty pane for `empty_frame` (the edges that do not fit are cropped); `tile` — copies at the original size repeated from the top-left corner. One image pixel is one screen pixel, so on a Retina display a plain or tiled image looks half as large as in an image viewer at 100%. | `"scale"` |
| `anchor` | Where a `plain` image sits: `top_left`, `top_center`, `top_right`, `center_left`, `center`, `center_right`, `bottom_left`, `bottom_center`, `bottom_right`. Ignored by `scale` and `tile`. | `"center"` |
| `flip_horizontal`, `flip_vertical` | Mirror the image. | `false` |

### This project only

Put the same `background_image` object into `<project>/.zed/settings.json`. Project values override the user values key by key, so a project can change only the `path`, or set `"path": ""` to turn the image off for that project. The settings of the project's first folder are used.

### How it behaves

- The `editor_and_tools` image is positioned relative to the whole window, so the editor, panels, terminal, title bar and status bar show parts of one continuous picture. It is drawn under text, selections and highlights. The `empty_frame` image is positioned relative to the empty pane itself.
- Every surface repaints the image right after its own opaque background, so it stays visible in opaque themes such as One Dark. In the default Eva Dark the docks and the editor have opaque backgrounds, so each island repaints the image over its own fill and `opacity` decides how much of the picture shows; the title bar, status bar, tab bar and terminal backgrounds are translucent, so they are skipped and the picture is drawn once under them, never twice. With both images set and a translucent editor background, the `editor_and_tools` image also shows through an empty pane, mixed with the `empty_frame` image.
- Pop-ups, menus, modals, notifications and the Settings window are not covered.
- Images larger than 8192 pixels on the longest side are scaled down when loaded. If you edit the image file in place, Flint reloads it when its window becomes active again.
- If the file is missing, unreadable, not an image, or the path is relative, Flint shows an error notification once and removes the image. It tries again when the setting changes or the window becomes active again.
- Images from a URL are not supported.

## Toolbar and icon sizes

Flint can make the title bar, the tab bar, the toolbar, the status bar and the dock panel headers taller and their icons bigger, closer to WebStorm. Each of the five groups has two keys, `height` and `icon_size`, ten keys in total:

```json
"title_bar":  { "height": null, "icon_size": null }
"tab_bar":    { "height": null, "icon_size": null }
"toolbar":    { "height": null, "icon_size": null }
"status_bar": { "height": null, "icon_size": null }
"panel":      { "height": null, "icon_size": null }
```

- The default `null` keeps Zed's built-in sizes. The keys are read only from your user `settings.json`, not from a project's `.zed/settings.json`.
- Values are logical pixels (points on macOS). They are not scaled by `ui_font_size`.
- `height` accepts 24–64 and `icon_size` accepts 10–32. Values outside these ranges are clamped.

### What each key does

- `title_bar.height` — the exact height of the title bar, but never lower than its buttons. With only `title_bar.icon_size` set, the bar grows when needed to keep at least 4px above and below the buttons. On macOS the window buttons (traffic lights) are re-centered vertically.
- `tab_bar.height` — the exact height of the editor tab bar, but never lower than its buttons. Dock panel headers follow it unless `panel.height` is set.
- `panel.height` — the exact height of the header and toolbar rows of dock panels (Git, Outline, Debugger, Agent, thread history), but never lower than their buttons. Unset, these rows use the tab bar height.
- `panel.icon_size` — the icon size of the controls in those header, toolbar and footer rows: in the Git panel View Diff, the filter, Stage All, the branch row and Fetch/Push/Pull, the commit editor buttons, Commit and the last-commit row; the Outline filter row; the Debugger controls; the Agent panel toolbar and thread history toolbar; the thread sidebar's bottom bar. List rows, the terminal panel tabs and the debugger's pane tabs are not affected; the latter two follow `tab_bar`.
- `toolbar.height`, `status_bar.height` — the minimum height of the bar. The content is centered vertically, and the bar is never smaller than its content.
- `icon_size` — the size of the primary icons of the bar (Zed's default is 14px). Secondary icons scale proportionally, so a 12px chevron becomes 12·N/14 for `icon_size` N. Buttons grow to at least the icon size plus 8px, and the bars grow so nothing is clipped. Text keeps the UI font size.

### Setting the sizes

- Settings window: Window & Layout → Status Bar, Title Bar and Tab Bar; Editor → Toolbar; Panels → Panel Headers. An unset key shows 0 there. Any number you enter is written to `settings.json` and clamped to the range above; the field's reset button removes the key again.
- `settings.json`:

```json
{
  "status_bar": { "height": 36, "icon_size": 20 },
  "tab_bar": { "height": 40 },
  "title_bar": { "height": 40 },
  "toolbar": { "icon_size": 18 },
  "panel": { "height": 32, "icon_size": 18 }
}
```

### WebStorm equivalents

| Key | WebStorm area |
|---|---|
| `title_bar` | main toolbar / window header |
| `tab_bar` | editor tabs |
| `toolbar` | breadcrumbs / navigation bar above the editor |
| `status_bar` | status bar (plus the tool window buttons when `tool_window_bars.show` is false) |
| `panel` | tool window header and toolbar |

### Relationship to `unstable.ui_density`

- `unstable.ui_density` changes only the dynamic spacing paddings: a padding of N px becomes N − 4 px (at least 0) in `compact` and N + 4 px in `comfortable`, except for a few paddings with their own per-density values. The default tab bar height is derived from these paddings, so it changes with density too.
- An explicit `height` overrides the density-derived default for its bar.
- Icon sizes of these five groups are not affected by density. Tool window bar icons are; see [Islands layout and tool window bars](#islands-layout-and-tool-window-bars).

## Islands layout and tool window bars

Flint uses WebStorm's [Islands](https://plugins.jetbrains.com/docs/intellij/supporting-islands-theme.html) layout by default:

- The title bar, the tool window bars, the gaps and the status bar share one window background (the theme's `background` color), with no borders between them.
- The left dock, the right dock, the bottom dock and the editor area are separate rounded islands. Docks use the theme's `panel.background`, the editor area uses `editor.background`. Dock resize handles sit in the gaps. In the default Eva Dark the window background is darker than the island fills (`background` `#16181db3`, docks `#21252b`, editor `#282c34`), so the islands stand out from the gaps; the terminal background stays transparent and shows the dock fill.
- Editor tabs: the tab bar has the editor background and no borders. The active tab is a rounded pill with an accent border and a light accent fill while its pane is focused, and a plain border otherwise. Inactive tabs show only their text, with a thin vertical divider between neighbouring tabs: none next to the active tab and none after the last tab of a strip (the pinned tabs and the other tabs are separate strips).
- Tool window bars run down the left and right window edges. The left bar shows the left dock panels, a separator, a "More Tool Windows" button (a menu of every panel, including panels whose button is hidden), and the bottom dock panels at the bottom. The right bar shows the right dock panels. Click an icon to show or hide its panel; right-click it to move the panel to another dock or hide its button. A filled background marks an open panel, an accent background a focused one. While the bars are shown, the status bar no longer has panel buttons.
- The right bar disappears when no right dock panel has a button, so it never leaves an empty strip. The editor island then keeps exactly the islands gap to the window edge, the same as on every other side.
- Tool window headers: every open dock island starts with a header like a WebStorm tool window header: the panel name in semibold, then ⋮ (the panel menu: move to another dock, hide the button) and — (hide the panel). Like WebStorm, the two buttons appear only while the pointer is over the tool window or the tool window has focus. Right-clicking the header opens the same menu. A panel that hosts its own tab strip, like the terminal, has no common header: its tabs start at the top of the island.
- Inactive window: when the Flint window is not focused, the title bar widgets, the tool window bar buttons and the status bar items are drawn at 56% opacity, matching the Islands `Island.inactiveAlpha` of 0.44. Backgrounds and the background image are not dimmed.

```json
"islands": { "enabled": true, "gap": null, "corner_radius": null, "dim_inactive_window": true },
"tool_window_bars": { "show": true, "icon_size": null, "show_names": false },
"tool_window_headers": { "show": true, "always_show_actions": false }
```

- `islands.gap` — distance between islands in pixels, 0–16. Unset: 4, or 3 with `"unstable.ui_density": "compact"`.
- `islands.corner_radius` — island corner radius in pixels, 0–24. Unset: 10, or 8 in compact density.
- `islands.dim_inactive_window` — dim the frame content of an inactive window. Works only with islands enabled.
- `tool_window_bars.icon_size` — bar icon size in pixels, 12–32; the bar is 20px wider than the icon. Unset: 20, or 16 in compact density.
- `tool_window_bars.show_names` — show the panel name under each icon.
- `tool_window_headers.show` — show the header row. `false` returns to panels without a common header.
- `tool_window_headers.always_show_actions` — show ⋮ and — all the time, like WebStorm's "Always show tool window header icons".
- `"islands": { "enabled": false }` together with `"tool_window_bars": { "show": false }` and `"tool_window_headers": { "show": false }` restores the classic Zed layout: flat docks with borders, square tabs and panel buttons in the status bar.
- Settings window: Window & Layout → Islands, Tool Window Bars and Tool Window Headers.
- WebStorm-like defaults that come with it: `tabs.file_icons: true`, `tab_bar.show_nav_history_buttons: false`, and the Git and Outline panels docked on the left.
- Not implemented: WebStorm's split tool windows under the bar separator and the bottom-right bar group, because a Flint dock shows one panel at a time.

## Editor tabs, project panel and scrollbar

- Hidden tabs: when the editor tabs do not fit, a ˅ button at the right end of the tab bar lists the tabs outside the visible area, like WebStorm's "Show Hidden Tabs" drop-down. Picking one activates it and scrolls it into view. `"tab_bar": { "show_hidden_tabs_button": true }`.
- Project panel selection: the hovered and selected rows are rounded pills inset from the panel edges, like WebStorm's tree selection. The selection is the theme's selection color while the panel has focus and a neutral gray otherwise; the keyboard cursor is a border on the pill. `"project_panel": { "rounded_selection": true }`; `false` restores full-width square rows.
- Editor scrollbar: the thumb is a narrower rounded pill inside the track, like the macOS scrollbar in JetBrains IDEs. Dragging still works on the whole track width. `"scrollbar": { "rounded_thumb": true }`.
- UI font: the default `ui_font_family` is `Inter`, the font JetBrains IDEs use; Inter 4.1 (OFL) is bundled in `assets/fonts/inter`. `".SystemUIFont"` gives San Francisco and `".ZedSans"` gives IBM Plex Sans.
- Project tree and editor: folders show a disclosure chevron followed by the folder icon (`"project_panel": { "folder_indicator": "both" }`), and the editor line height is `comfortable` (`"buffer_line_height": "comfortable"`, 1.618), like WebStorm.
- Settings window: Window & Layout → Tab Bar, Panels → Project Panel, Editor → Scrollbar, Appearance → UI Font.

The WebStorm reference used for these changes, with quotes from JetBrains documentation and the full gap list, is in [docs/webstorm-ui-research.md](docs/webstorm-ui-research.md).

## Main toolbar

The title bar works like WebStorm's [main toolbar](https://www.jetbrains.com/help/webstorm/new-ui.html), left to right:

- Project widget: a rounded badge with one or two initials of the project name, the name and a ˅. Click it to open the recent projects popover. The badge color is picked from the theme's accent colors by a stable hash of the project name, so a project keeps its color across restarts.
- Project gradient: like WebStorm's colored project headers, the title bar is tinted with the badge color, fading out from the left edge to the middle. It appears only when a project is open.
- VCS widget: the branch name, then `↓N ↑M` when the branch is behind or ahead of its upstream (only the non-zero directions are shown), and a ˅. Click it to open the branch picker. The worktree button is hidden by default.
- Run widget: the label of the last task you ran (or "Run…") with a ˅ that opens the task picker (`task: spawn`), a green ▷ that reruns the last task (`task: rerun`) and a green bug that starts debugging (`debugger: start`).
- Right edge: a magnifying glass that opens Search Everywhere and a gear that opens the Settings window.

Each widget can be turned off in `settings.json` or in Settings → Window & Layout → Title Bar:

```json
"title_bar": {
  "show_project_badge": true,
  "show_project_gradient": true,
  "show_run_widget": true,
  "show_search_button": true,
  "show_settings_button": true,
  "show_worktree_name": false
}
```

## Status bar

The status bar works like WebStorm's [status bar and navigation bar](https://www.jetbrains.com/help/webstorm/guided-tour-around-the-user-interface.html):

- Left: a navigation bar `project › directory › … › file › symbol`. Clicking the project, a directory or the file reveals it in the project panel; clicking a symbol (the outline items around the cursor) moves the cursor to it. When the path is wider than half the window, the middle segments collapse into `…`, keeping the first and the last segment. It replaces the active file name (`show_active_file`) while it is enabled.
- Right, left to right: cursor position, line ending, encoding, indentation, read-only lock, language.
- Indentation shows `N spaces` or `Tab` for the active file. Clicking it (or `status widgets: select indentation`) opens a picker with 2 spaces, 4 spaces, 8 spaces and Tab; the choice is written to `languages.<Language>.tab_size` / `hard_tabs` in your user `settings.json` (to the top-level `tab_size` / `hard_tabs` for files without a language).
- The lock shows whether the active file is read-only. Clicking it toggles read-only mode like `workspace: toggle read only file`; it is disabled for editors that are read-only by construction.
- The path no longer appears above the editor: `toolbar.breadcrumbs` and `toolbar.quick_actions` default to `false`. Set them to `true` to bring the editor toolbar row back.

Each widget can be turned off in `settings.json`, in Settings → Window & Layout → Status Bar, or with "Hide Button" in the widget's right-click menu:

```json
"status_bar": {
  "navigation_bar": true,
  "indentation_button": true,
  "read_only_button": true,
  "line_endings_button": true,
  "active_encoding_button": "enabled"
}
```

## Search Everywhere

Search Everywhere is a WebStorm-style popup that searches files, classes, symbols, actions and text at once. With the JetBrains base keymap it opens on a double `Shift`; with any keymap it is available as `search everywhere: toggle` in the command palette. Like WebStorm, it has a row of tabs above the query and a preview of the selected item.

| Tab | What it searches |
| --- | --- |
| All | Classes, Files, Symbols and Actions, a few results each, grouped under section headers. Text matches are added at the bottom when the other sections have fewer than five results. A `more…` row opens the full tab for that section. An empty query lists the recently opened files. |
| Classes | Workspace symbols of kind class, interface, enum and struct. |
| Files | Files and folders by name (fuzzy, so `CamelCase` and `snake_case` abbreviations work). An empty query lists the recently opened files. `name:line` and `name:line:column` open the file at that position. Opening a folder reveals it in the project panel. |
| Symbols | All workspace symbols. |
| Actions | Every action available where the popup was opened, with its key binding. `Enter` runs the action in the editor or panel that had focus; `Cmd-Enter` opens the keymap editor to change its binding. Command aliases from `command_aliases` work here too. |
| Text | Text in the project files (case-insensitive, at most 100 matches). The last row, "Open in Text Finder", hands the query to the full Text Finder. |

Classes and Symbols come from the language servers' workspace symbol search, so they are empty for languages whose server does not provide it. The "Include non-project items" checkbox adds git-ignored files and symbols outside the project.

### Shortcuts (JetBrains keymap)

| Keys | Action |
| --- | --- |
| `Shift` `Shift` | Open on the All tab. Pressing it again while the popup is open toggles "Include non-project items". |
| `Cmd-O` | Open on the Classes tab. |
| `Cmd-Shift-O`, `Cmd-Shift-N` | Open on the Files tab. |
| `Cmd-Alt-O` | Open on the Symbols tab. |
| `Cmd-Shift-A` | Open on the Actions tab. |
| `Tab`, `Shift-Tab` | Next or previous tab; the query is kept. |
| `Ctrl-Down`, `Ctrl-Up` | Jump to the next or previous section; in a single-section tab, to the last or first row. |

Pressing a tab shortcut while the popup is open switches to that tab, or toggles "Include non-project items" when that tab is already active. `Cmd-E` still opens the file finder with its recent files.

### Not implemented

- Math evaluation in the query.
- `/` to list settings groups.
- The Git tab (branches and commits).
- The filter popup (for example, only recent files) and "Open in Find tool window" for tabs other than Text.

## Merge conflicts

Flint resolves merge conflicts like WebStorm's [Resolve conflicts](https://www.jetbrains.com/help/webstorm/resolve-conflicts.html): a Conflicts dialog lists the conflicted files, and a three-pane Merge Revisions window resolves one file. Both work only for local repositories; they are hidden in remote projects.

### Conflicts dialog

- Opens by itself when an in-app pull, stash pop or stash apply leaves conflicted files. For conflicts made on the command line, click "Resolve…" in the header of the Git panel's Conflicts section, or run `merge tool: open conflicts`.
- Columns: the file, "Yours (<current branch>)" and "Theirs (<merged branch>)", each showing whether that side added, deleted or modified the file. Files are grouped into Unresolved and Resolved, with a resolved/total changes badge per file.
- Accept Yours keeps the checked-out branch's version of the file and Accept Theirs keeps the merged branch's version; either one stages the file.
- Resolve All Simple Conflicts applies the non-conflicting changes and the simple conflicts of every file; a file left with nothing to resolve is written and staged.
- Resolve Manually (or a double-click) opens the file in the Merge Revisions window.
- Accept and Finish is enabled once every file is resolved and staged; it closes the dialog and focuses the commit editor of the Git panel.
- Right-click a resolved file → Revert conflict resolution returns it to its conflicted state.

### Git panel

- Double-clicking a conflicted file opens it in the Merge Revisions window; a single click keeps its usual behaviour.
- The context menu of a conflicted file starts with Accept Yours, Accept Theirs and Merge….

### Merge Revisions window

- Left pane: your version (the checked-out branch), read-only. Right pane: their version (the merged branch), read-only. Centre pane: the Result, a normal editor that starts from the base revision.
- Each change has `>>` (left) or `<<` (right) to accept it into the Result and `X` to ignore it. Cmd-click on `>>`/`<<` resolves the conflict using that side and ignores the other one; Cmd-click on `X` ignores both sides and keeps the Result text. Alt-click appends the side after the text already in the Result. A magic wand in the Result gutter resolves a simple conflict.
- Toolbar: previous/next difference, apply non-conflicting changes from the left, from both sides or from the right, resolve simple conflicts, and the gear with Synchronize Scrolling and Ignore Differences (None, Trim whitespaces, Ignore whitespaces, Ignore whitespaces and empty lines). Ignore Differences can be changed only before the first change is applied or the Result is edited. The counter shows the remaining changes and conflicts, for example "3 changes. 1 conflict.".
- Undo and redo in the Result also undo and redo the accept and ignore states.
- Bottom buttons: Accept Left and Accept Right take the whole file from one side. Save and Close keeps the partial result in memory and returns to the Conflicts dialog. Apply Changes writes the Result, stages the file and opens the next conflicted file; while unresolved changes remain it first asks "Apply the result anyway?".

### Keyboard shortcuts

| Shortcut | Action |
|---|---|
| F7 | Next difference |
| Shift-F7 | Previous difference |
| Ctrl-Cmd-Right | Accept the left side of the change at the cursor |
| Ctrl-Cmd-Left | Accept the right side of the change at the cursor |
| Ctrl-Shift-Tab | Focus the opposite pane |
| Cmd-Shift-D | Show the settings popup |

The shortcuts work in the default and the JetBrains keymaps; inside the merge window they take precedence over the debugger's F7.

### Setting

```json
"git": { "merge_tool": { "auto_apply_non_conflicting": false } }
```

With `true`, the merge window applies all non-conflicting changes as soon as it opens. Settings window: Version Control → Merge Tool.

## Languages and code navigation

Cmd+click (and `Cmd-B`, `Alt-F7` with the JetBrains keymap) goes to a definition or lists usages through the file's language server, so it works once that server runs:

- Built in, no install step: Rust (rust-analyzer from `rustup` or `PATH`, otherwise downloaded), C and C++ (clangd), Go (gopls, needs `go` on `PATH`), Python, TypeScript and JavaScript, JSON, YAML, CSS, Bash.
- Installed automatically on first launch from the extension registry: C# (`csharp`, Roslyn), Java (`java`, jdtls), Swift, Dart, Vue, Svelte, Astro, plus HTML, Dockerfile, TOML, Markdown and `.env`. Other languages, such as Kotlin, PHP or Ruby, get their extension after you accept the install suggestion shown when you open such a file. To skip an automatic install, set it to `false` in your `settings.json`: `"auto_install_extensions": { "swift": false }`.
- C# needs a .NET runtime. If .NET is installed only in `~/.dotnet` and `DOTNET_ROOT` is not set, Flint passes `DOTNET_ROOT=~/.dotnet` to language servers, so Roslyn starts without extra setup. A `DOTNET_ROOT` or `DOTNET_ROOT_ARM64` you set yourself, in the shell or in `lsp.<server>.binary.env`, always wins.
- Each server still needs its project files to resolve symbols: `Cargo.toml` for Rust, `compile_commands.json` for C and C++, `go.mod` for Go, a `.csproj` or `.sln` for C#, and a JDK for Java.

## Licensing

Zed source code is licensed under GPL-3.0-or-later, with Apache-2.0 components where marked; see [LICENSE-GPL](LICENSE-GPL) and [LICENSE-APACHE](LICENSE-APACHE).

Third-party licenses are generated by [`cargo-about`](https://github.com/EmbarkStudios/cargo-about) from [`script/licenses/zed-licenses.toml`](script/licenses/zed-licenses.toml) during bundling and embedded in the app. The build fails if a dependency's license is not in the accepted list.
