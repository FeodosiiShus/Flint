> [!IMPORTANT]
> Remove this line to confirm you've reviewed this PR before submitting.

# Flint

Flint is a personal fork of [Zed](https://github.com/zed-industries/zed). It is built only for macOS on Apple Silicon (`aarch64-apple-darwin`), only in GitHub Actions, by the [`build-macos.yml`](.github/workflows/build-macos.yml) workflow.

## Getting a build

- Every push runs the [`checks.yml`](.github/workflows/checks.yml) workflow: `cargo fmt --check`, `cargo check` of the app, and the tests of the crates Flint changes. It does not build the app.
- The app is built on every push or merge to `main`, and on demand: Actions → Build macOS → Run workflow → branch `main`. The DMG is attached to the run as the `flint-macos-aarch64` artifact.
- Running Build macOS from a `v*` tag also publishes a GitHub Release with the DMG attached.
- To run the tests locally, use `script/local-ci-tests` (limits and prerequisites are documented in [AGENTS.md](AGENTS.md) under "Build guidelines"); release builds still run only in GitHub Actions.

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

Flint ships none of Zed's built-in AI: no agent panel, threads sidebar, inline assist, language model providers, MCP servers, external agents or commit message generation. It also drops the debugger panel, notebooks (REPL), dev containers, the headless remote server, vim mode, Markdown preview, the image viewer, the theme, tab, toolchain and settings profile pickers, call hierarchy, the Outline panel (the symbol outline popup of `outline: toggle` stays), the editor minimap, the journal, the SVG preview, the snippets picker, the syntax tree, highlights tree and key context views, the language servers for Tailwind CSS, CSS, Bash, Go, Python, C and C++, Vue, the task runner (`tasks.json` and `debug.json`, the Run menu, gutter run buttons, `task: spawn`), the status bar, and every non-macOS platform backend. Use the terminal panel to run AI command-line tools.

## Background image

Flint can draw a picture behind the editor and the tool windows, like WebStorm's [Background Image](https://www.jetbrains.com/help/webstorm/setting-background-image.html). There are two independent images:

- `editor_and_tools` — behind the editor, tabs, tool windows (project, git, terminal panels) and the title bar.
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

- The `editor_and_tools` image is positioned relative to the whole window, so the editor, panels, terminal and title bar show parts of one continuous picture. It is drawn under text, selections and highlights. The `empty_frame` image is positioned relative to the empty pane itself.
- Every opaque surface repaints the image right after its own background, so it stays visible in opaque themes such as One Dark. In the default look the frame (title bar, tool window bars and gaps), the docks, the editor and the toolbar are all opaque, so each of them repaints the image over its own fill and `opacity` decides how much of the picture shows. Only `terminal.background` is transparent, so the terminal draws the picture once, through its container. With both images set and an editor background that is not opaque, the `editor_and_tools` image also shows through an empty pane, mixed with the `empty_frame` image.
- Pop-ups, menus, modals, notifications and the Settings window are not covered.
- Images larger than 8192 pixels on the longest side are scaled down when loaded. With `fill: "scale"`, an image larger than the biggest connected display (counted at 2x Retina) is also scaled down to the smallest size that still covers it, which saves memory without changing how it looks; `plain` and `tile` images are never resized apart from the 8192 pixel cap. Workspaces and layers that use the same file and settings share one decoded copy. When you connect a larger display, the image is decoded again the next time the window becomes active. If you edit the image file in place, Flint reloads it when its window becomes active again.
- If the file is missing, unreadable, not an image, or the path is relative, Flint shows an error notification once and removes the image. It tries again when the setting changes or the window becomes active again.
- Images from a URL are not supported.

## Toolbar and icon sizes

Flint can make the title bar, the tab bar, the toolbar and the dock panel headers taller and their icons bigger, closer to WebStorm. Each of the four groups has two keys, `height` and `icon_size`, eight keys in total:

```json
"title_bar":  { "height": null, "icon_size": null }
"tab_bar":    { "height": null, "icon_size": null }
"toolbar":    { "height": null, "icon_size": null }
"panel":      { "height": null, "icon_size": null }
```

- The default `null` keeps Zed's built-in sizes. The keys are read only from your user `settings.json`, not from a project's `.zed/settings.json`.
- Values are logical pixels (points on macOS). They are not scaled by `ui_font_size`.
- `height` accepts 24–64 and `icon_size` accepts 10–32. Values outside these ranges are clamped.

### What each key does

- `title_bar.height` — the exact height of the title bar, but never lower than its buttons. With only `title_bar.icon_size` set, the bar grows when needed to keep at least 4px above and below the buttons. On macOS the window buttons (traffic lights) stay vertically centered in the title bar at any height.
- `tab_bar.height` — the exact height of the editor tab bar, but never lower than its buttons. Unset, the tab bar is 41 px high (31 px in compact density). Dock panel headers follow it unless `panel.height` is set.
- `panel.height` — the exact height of the header and toolbar rows of dock panels (Git), but never lower than their buttons. Unset, these rows use the tab bar height (41 px, 31 px compact).
- `panel.icon_size` — the icon size of the controls in those header, toolbar and footer rows: in the Git panel View Diff, the filter, Stage All, the branch row and Fetch/Push/Pull, the commit editor buttons, Commit and the last-commit row. List rows and the terminal panel tabs are not affected; the terminal panel tabs follow `tab_bar`.
- `toolbar.height` — the minimum height of the bar. The content is centered vertically, and the bar is never smaller than its content.
- `icon_size` — the size of the primary icons of the bar (Zed's default is 14px; the editor tab file icon is 16px by default). Secondary icons scale proportionally, so a 12px chevron becomes 12·N/14 for `icon_size` N. Buttons grow to at least the icon size plus 8px, and the bars grow so nothing is clipped. Text keeps the UI font size.

### Setting the sizes

- Settings window: Window & Layout → Title Bar and Tab Bar; Editor → Toolbar; Panels → Panel Headers. An unset key shows 0 there. Any number you enter is written to `settings.json` and clamped to the range above; the field's reset button removes the key again.
- `settings.json`:

```json
{
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
| `panel` | tool window header and toolbar |

### Relationship to `unstable.ui_density`

- `unstable.ui_density` changes the dynamic spacing paddings: a padding of N px becomes N − 4 px (at least 0) in `compact` and N + 4 px in `comfortable`, except for a few paddings with their own per-density values. The default tab bar height does not use these paddings: it is 41 px, and 31 px in `compact`; `comfortable` keeps 41 px.
- An explicit `height` overrides the density-derived default for its bar.
- Icon sizes of these four groups are not affected by density. Tool window bar icons are; see [Islands layout and tool window bars](#islands-layout-and-tool-window-bars).

## Islands layout and tool window bars

Flint uses WebStorm's [Islands](https://plugins.jetbrains.com/docs/intellij/supporting-islands-theme.html) layout by default:

- The title bar, the tool window bars and the gaps share one window background (the theme's `background` color), with no borders between them.
- The left dock, the right dock, the bottom dock and the editor area are separate rounded islands. Docks use the theme's `panel.background`, the editor area uses `editor.background`. Dock resize handles sit in the gaps. In the default look the frame `#26282c` is lighter than the docks `#191a1c`, and the editor island keeps Eva's `#282c34`, so the islands stand out from the gaps; the terminal body shows the editor colour.
- Editor tabs: the tab bar has the editor background and no borders. The strip is 41 px high (31 px compact). The selected tab is a 28 px pill (24 px compact) with a 6 px radius (4 px compact) and a 1 px inside border, with no underline and no dividers between tabs. In a focused pane the pill has the `#233558` fill and the `#2E4D89` border (the theme's `tab.active_background` and `border.selected`); in an unfocused pane it has the frame fill (the theme's `background`) and the `border` colour. Tabs get the hover fill (`ghost_element_hover`), except the selected tab of a focused pane. Unselected labels and icons are dimmed (labels blended toward the strip background, icons at 75% opacity); selected and hovered tabs use the full text colour. Every tab reserves a 16 px slot at its right end for the close or pin button, so tabs do not change width on hover: the × shows on the selected tab and on hovered tabs, a modified tab shows a dot in the same slot (the × replaces it on hover), and a pinned tab shows the pin. Titles are no longer cut at 24 characters.
- Tool window bars run down the left and right window edges. The left bar shows the left dock panels (by default Project, Git, Search, Project Diagnostics and Language Services), a separator, a "More Tool Windows" button (a menu of every panel, including panels whose button is hidden), and the bottom dock panels at the bottom. The right bar shows the right dock panels. Click an icon to show or hide its panel; right-click it to move the panel to another dock or hide its button. A filled background marks an open panel, an accent background a focused one.
- The right bar disappears when no right dock panel has a button, so it never leaves an empty strip. The editor island then keeps exactly the islands gap to the window edge, the same as on every other side.
- Tool window headers: every open dock island starts with a header like a WebStorm tool window header: the panel name in semibold, then the panel's own buttons, ⋮ (the panel menu: move to another dock, hide the button) and — (hide the panel). Like WebStorm, the buttons appear only while the pointer is over the tool window or the tool window has focus. Right-clicking the header opens the same menu. A panel that hosts its own tab strip, like the terminal, has no common header: its tabs start at the top of the island. A panel adds its own buttons by implementing `Panel::header_actions`; the Project panel and the Project Diagnostics panel are the only ones that do.
- Inactive window: when the Flint window is not focused, the title bar widgets and the tool window bar buttons are drawn at 56% opacity, matching the Islands `Island.inactiveAlpha` of 0.44. Backgrounds and the background image are not dimmed.

```json
"islands": { "enabled": true, "gap": null, "corner_radius": null, "dim_inactive_window": true },
"tool_window_bars": { "show": true, "icon_size": null, "show_names": false, "icon_style": "jetbrains" },
"tool_window_headers": { "show": true, "always_show_actions": false }
```

- `islands.gap` — distance between islands in pixels, 0–16. Unset: 4, or 3 with `"unstable.ui_density": "compact"`.
- `islands.corner_radius` — island corner radius in pixels, 0–24. Unset: 10, or 8 in compact density.
- `islands.dim_inactive_window` — dim the frame content of an inactive window. Works only with islands enabled.
- `tool_window_bars.show` — `false` hides both bars. No panel buttons are shown anywhere then; panels open through the View menu, the command palette and key bindings.
- `tool_window_bars.icon_size` — bar icon size in pixels, 12–32; the bar is 20px wider than the icon. Unset: 20, or 16 in compact density. These are the IntelliJ Platform sizes: 20×20 and, in Compact Mode, 16×16.
- `tool_window_bars.icon_style` — `"jetbrains"` (default) draws the IntelliJ Platform tool window icons: Project, Git (the IntelliJ "Version Control" icon), Terminal and the "…" of More Tool Windows. The Search panel keeps Zed's magnifier icon. `"zed"` returns to Zed's own icons. The artwork has two drawings, a 20×20 one and a 16×16 one with a thinner stroke, like IntelliJ; sizes below 18px use the 16×16 drawing and the others the 20×20 drawing, scaled to `icon_size`.
- `tool_window_bars.show_names` — show the panel name under each icon.
- `tool_window_headers.show` — show the header row. `false` returns to panels without a common header.
- `tool_window_headers.always_show_actions` — show the header buttons (⋮, — and a panel's own buttons) all the time, like WebStorm's "Always show tool window header icons".
- `"islands": { "enabled": false }` together with `"tool_window_bars": { "show": false }` and `"tool_window_headers": { "show": false }` restores the classic Zed layout: flat docks with borders and square tabs, without panel buttons.
- Settings window: Window & Layout → Islands, Tool Window Bars and Tool Window Headers.
- WebStorm-like defaults that come with it: `tabs.file_icons: true`, `tab_bar.show_nav_history_buttons: false`, and the Git, Search, Project Diagnostics and Language Services panels docked on the left.
- Not implemented: WebStorm's split tool windows under the bar separator and the bottom-right bar group, because a Flint dock shows one panel at a time. Opening the Search, Project Diagnostics or Language Services panel replaces the Project or Git view in the left dock while it is open.

## Default look: Islands Dark chrome on the Eva theme

- The default look is the chrome of JetBrains' Islands Dark theme (`ManyIslandsDark.theme.json`, intellij-community commit `0ad391f70a62eabbec6db656f473e1284e1110a9`) on top of the Eva Dark theme, like IntelliJ's Islands Dark UI theme with the Eva editor colour scheme. The theme is still named "Eva Dark".
- Chrome copied from Islands Dark: the frame, tool window islands, title bar, tool window bars, popups and menus, hover, pressed and selected states, borders, text and icon colours, the scrollbar thumb and the drop target.
- Stays Eva: editor colours and gutter, syntax, terminal palette, players, git and diagnostic status colours and search highlight. `assets/themes/eva/eva.json` is unchanged.
- Where it lives: the `theme_overrides."Eva Dark"` block of the bundled default settings. A `theme_overrides."Eva Dark"` in your own `settings.json` merges key by key over it, so override the key to change any colour. The exception is the `accents` list: a `theme_overrides."Eva Dark"` block in your `settings.json` replaces it with an empty list unless you repeat `accents` in that block.
- `background.appearance` is now `opaque`, because Islands Dark is opaque.
- To restore the previous Eva look, override the keys in `settings.json`.

| Surface | Colour | IntelliJ key |
|---|---|---|
| Frame: title bar, tool window bars, gaps | `#26282c` | `main-window-bg` |
| Tool window islands | `#191a1c` | `tool-window-bg` |
| Editor island and tab bar | `#282c34` (Eva editor background) | none, kept from Eva |
| Popups and menus | `#26282c`, border `#33353b` | `popup-bg`, `popup-border` |
| Control border | `#40434a` | `control-border` |
| Hover | `#ffffff17` | `toolbar-bg-hovered` |
| Pressed and open tool window button | `#ffffff29` | `toolbar-bg-pressed` |
| Tree and list hover | `#ffffff10` | `selection-bg-hovered` |
| Selection | `#2a4371` | `selection-bg-active` |
| Focused tool window button, accent and focus border | `#3871e1`, white icon on the focused button | `blue-80` |
| Text: default, muted, secondary, disabled | `#d1d3d9`, `#9fa2a8`, `#73767c`, `#4c4f56` | `text-default`, `text-muted`, `text-secondary`, `text-disabled` |
| Icons: default, disabled | `#c3c5cb`, `#5f6269` | `icon-default-stroke`, `icon-disabled` |
| Scrollbar thumb, hovered thumb, dragged thumb | `#80808059`, `#8080808c`, `#808080c0` | `ScrollBar.thumbColor`, `ScrollBar.hoverThumbColor`; the dragged thumb colour is Flint's, IntelliJ has none |
| Drop target area | `#366acf4d` | `ToolWindow.DragAndDrop.areaBackground` |

- The nine project colours are `#e08855`, `#b08b14`, `#a1a359`, `#3b92b8`, `#3574f0`, `#c84d8f`, `#955ae0`, `#24a394` and `#5fad65` (IntelliJ `Color1`–`Color9` `Avatar.Start`). They are the theme `accents`, so the title bar gradient (the accent at 35% opacity over `#26282c`) matches IntelliJ. The same list is the palette of the bracket pair colours while bracket colorization is on (`colorize_brackets`; Flint adjusts each colour for contrast and may reorder them) and of the lanes of the Git graph.

### Known differences from IntelliJ

- The editor island uses Eva's editor background, not `#191a1c`.
- The tab bar default height is 41 px (31 px compact) like IntelliJ, but `tab_bar.height` and `tab_bar.icon_size` still override it, and Zed's other bars keep their unchanged defaults. The IntelliJ values for those can be set with existing settings: `"title_bar": { "height": 40, "icon_size": 20 }` (main toolbar header 40, button icon 20) and `"panel": { "height": 41 }` (tool window header 41).
- Tabs: the label font stays the UI font size (13.1 px at the default `ui_font_size` of 15) where IntelliJ uses Inter 13 px; non-editor tabs (terminal, diagnostics) take the new colours but not the label size; `close_position: left` mirrors the right-hand paddings, which are IntelliJ's numbers; compact mode uses a 2 px right padding derived from IntelliJ's compact insets.
- Tabs: the modified dot is always shown (IntelliJ only with "Mark modified"), it is not clickable and its colour is the theme accent, not IntelliJ's `#548AF7`; with `close_button: hidden` a modified tab still shows the dot, so its width can change; the drag ghost shows only icon and label.
- The unselected label blend is computed in Flint as the square root of 0.7·text² + 0.3·editor background² per channel, over the editor background, not IntelliJ's strip background `#191a1c`.
- IntelliJ's `Island.borderWidth` is 6; the Flint gap default of 4 is left as is.
- Split dividers between editor panes use `border` (`#40434a`), not IntelliJ's `#26282c`, which is invisible on Eva's editor background.
- Filled buttons that sit on the modal layer (the Restricted Mode and disconnected-project dialogs) have the same colour as the dialog, because IntelliJ's popup colour `#26282c` equals the frame colour; only their label and their pressed state are visible.
- Hover highlights use IntelliJ's translucent white, so they are fainter than Eva's solid blue; the sticky excerpt header keeps an opaque fill while hovered.

## Editor tabs, project panel and scrollbar

- Hidden tabs: when the editor tabs do not fit, a ˅ button at the right end of the tab bar lists the tabs outside the visible area, like WebStorm's "Show Hidden Tabs" drop-down. Picking one activates it and scrolls it into view. `"tab_bar": { "show_hidden_tabs_button": true }`.
- Project panel selection: the hovered and selected rows are rounded pills inset from the panel edges, like WebStorm's tree selection. The selection is the theme's selection color while the panel has focus and a neutral gray otherwise; the keyboard cursor is a border on the pill. `"project_panel": { "rounded_selection": true }`; `false` restores full-width square rows.
- Project panel header buttons: like WebStorm's Project tool window, the header has Select Opened File (⌖), Expand All and Collapse All before ⋮ and —. Select Opened File reveals and selects the file of the active editor tab in the tree, expanding its folders, and moves focus to the tree; with no file behind the tab (an unsaved buffer or a file outside the project) it only focuses the panel. Opening a file does not select it in the tree by itself; `"project_panel": { "auto_reveal_entries": true }` brings that back. Expand All and Collapse All are the `project panel: expand all entries` and `project panel: collapse all entries` actions. The buttons follow `tool_window_headers.always_show_actions`; the `project panel: select opened file` action is available from the command palette.
- Editor scrollbar: the thumb is a narrower rounded pill inside the track, like the macOS scrollbar in JetBrains IDEs. Dragging still works on the whole track width. `"scrollbar": { "rounded_thumb": true }`.
- UI font: the default `ui_font_family` is `Inter`, the font JetBrains IDEs use; Inter 4.1 (OFL) is bundled in `assets/fonts/inter`. `".SystemUIFont"` gives San Francisco and `".ZedSans"` gives IBM Plex Sans.
- Project tree and editor: folders show a disclosure chevron followed by the folder icon (`"project_panel": { "folder_indicator": "both" }`), and the editor line height is `comfortable` (`"buffer_line_height": "comfortable"`, 1.618), like WebStorm.
- The path no longer appears above the editor: `toolbar.breadcrumbs` and `toolbar.quick_actions` default to `false`. Set them to `true` to bring the editor toolbar row back.
- Settings window: Window & Layout → Tab Bar, Panels → Project Panel, Editor → Scrollbar, Appearance → UI Font.

The WebStorm reference used for these changes, with quotes from JetBrains documentation and the full gap list, is in [docs/webstorm-ui-research.md](docs/webstorm-ui-research.md).

## Main toolbar

The title bar works like WebStorm's [main toolbar](https://www.jetbrains.com/help/webstorm/new-ui.html), left to right:

- Window buttons (close, minimize, zoom), described below.
- VCS widget: the branch name, then `↓N ↑M` when the branch is behind or ahead of its upstream (only the non-zero directions are shown), and a ˅. Click it to open the [Git Branches popup](#git-branches-popup). The worktree button is hidden by default. In a remote project, the remote host indicator comes before it.
- Search Everywhere field: a field labelled "Search Everywhere" with the shortcut of `search everywhere: toggle` from your current keymap (`Shift` `Shift` with the JetBrains keymap). Click it to open Search Everywhere on its default tab.

Project gradient: like WebStorm's colored project headers, the title bar is tinted with a project color, fading out from the left edge to the middle. The color is picked from the theme's accent colors by a stable hash of the project name, so a project keeps its color across restarts. It appears only when a project is open.

The gradient, the search field and the worktree button can be turned on or off in `settings.json` or in Settings → Window & Layout → Title Bar:

```json
"title_bar": {
  "show_project_gradient": true,
  "show_search_button": true,
  "show_worktree_name": false
}
```

Window buttons, like WebStorm on macOS:

- Every project window has this title bar, and the window buttons (close, minimize, zoom) sit at its left end, vertically centered. The widgets start after the buttons; in full screen, where macOS hides the buttons, they start at the left edge.
- The Settings, About, Conflicts and Merge Revisions windows use the standard macOS title bar with the window title, like WebStorm dialogs, so the buttons never cover their content.

## Git Branches popup

The branch popup works like WebStorm's [Git Branches popup](https://www.jetbrains.com/help/webstorm/manage-branches.html). It replaces the old Branches | Stashes picker.

### Where it opens

- The VCS widget in the title bar opens it as a popover.
- The branch buttons of the Git panel and of the commit window open it as a popover too.
- `git: branch`, `git: switch` and `git: checkout branch` open it as a modal for the active repository.
- Stashes are no longer a tab. `git: view stash` opens the stash list as its own modal (Drop and Show keys are unchanged).
- The popup surface, its gear menu, the branch submenus and every dialog opened from it are always opaque, even when your theme overrides make `elevated_surface.background` translucent.

### Layout

- Header: a search field with the placeholder "Search for branches and actions", a Fetch button (a spinner while it runs, errors are shown as notifications) and a gear.
- The gear menu has "Show Actions in Search Results", "Group by Directory", "Show Recent Branches" and "Show Tags". All four are on by default and are remembered per repository.
- Top actions: Update Project… (⌘T), Commit… (⌘K), Push… (⇧⌘K, which opens the Push dialog for the current branch instead of pushing at once), then, while a rebase or merge is in progress, its Abort, Continue and Skip actions, then New Branch… (⌥⌘N) and Checkout Tag or Revision…. The shortcuts are shown from your current keymap; with the JetBrains keymap they are the ones listed here.
- New Branch… is greyed with an explanation in an empty repository without commits.
- With several repositories in the project, a row per repository sits between the actions and the tree. Selecting one switches the repository the popup shows.

### Tree

- Sections: Recent (at most five branches you checked out last, the current branch first, taken from the reflog), Local, Remote and Tags. An empty section is not shown. A branch in Recent is not repeated in Local.
- Branches are grouped into folders by `/` when "Group by Directory" is on; a remote's branches are grouped under the remote name. `*/HEAD` remote symrefs are hidden.
- Order: the current branch first, then favorites, then folders, then the rest in natural order.
- Favorites: `Space` or a click on the branch icon toggles the star. `main`, `master`, `origin/main` and `origin/master` are favorites until you remove them.
- Rows show the tracked remote branch of a local branch and the incoming (↓) and outgoing (↑) commit counts. The current branch has its own marker; nothing is marked in a detached HEAD.
- Sections and folders you expand stay expanded the next time the popup opens.

### Search

- Typing filters branches, tags and, with "Show Actions in Search Results", the top actions. Matching is case-insensitive and also accepts camelCase initials; matches are highlighted.
- The best match is selected after every change. With no match the list says "Nothing found" (actions filter on) or "Branch not found".
- `Escape` clears the search first and closes the popup when it is already empty.

### Keys

| Key | Action |
|---|---|
| `Up`, `Down` | Move the selection, skipping separators |
| `Enter` | Run a top action; expand or collapse a section or folder; open the submenu of a branch or tag |
| `Right` | Expand a section or folder, step into it when expanded, or open the submenu of a branch or tag |
| `Left` | Collapse an expanded section or folder, otherwise select the parent row |
| `Space` | Toggle favorite |
| `Escape` | Clear the search, then close |

### Submenu of a branch or tag

Every branch and tag row opens a submenu (`>`). Its items depend on the kind of ref, and unavailable items are greyed with a tooltip:

- Current local branch: New Branch from 'X'…, Show Diff with Working Tree, Update, Push…, Rename….
- Other local branch: Checkout, New Branch from 'X'…, Checkout and Rebase onto 'current', Checkout and Update, Compare with 'current', Show Diff with Working Tree, Rebase 'current' onto 'X', Merge 'X' into 'current', Update, Push…, Rename…, Delete. Update and Checkout and Update are greyed for a branch without a tracked remote branch, and Rebase is greyed in a detached HEAD.
- Remote branch: Checkout, New Branch from 'X'…, Checkout and Rebase onto 'current', Compare with 'current', Show Diff with Working Tree, Rebase 'current' onto 'X', Merge 'X' into 'current', Pull into 'current' Using Rebase, Pull into 'current' Using Merge, Delete.
- Tag: Checkout, Show Diff with Working Tree, Merge 'X' into 'current', Push to each remote (a "Push Tag" submenu from six remotes on), Delete. A tag that is the current detached HEAD has no Checkout and no Merge.

### What the operations do

- Checkout, merge and rebase are smart: when local changes would be overwritten, a "Git Checkout Problem" (or "Git Merge Problem") dialog offers Smart Checkout (stash the changes, run the operation, restore them), Force Checkout (checkout only) or Don't Checkout. A conflict while restoring opens the Conflicts dialog.
- New Branch… and New Branch from 'X'… ask for a name with "Checkout branch" and "Overwrite existing branch". Names are validated like WebStorm does and cleaned up while you type.
- Checkout Tag or Revision… checks out a tag or any revision as a detached HEAD.
- Merge reports "Already up to date", success (with a Delete action for the merged local branch) or a conflict that opens the Conflicts dialog; conflicts left unresolved produce a "<branch> Merged with Conflicts" notification with Resolve…. When the changes were stashed for the merge, a "Local changes were not restored" warning with "View saved changes…" is shown first. A merge or checkout refused because of unmerged files says "Cannot merge because of unmerged files" with "Resolve conflicts…". Rebase reports success or stops at conflicts with a "Rebase stopped due to conflicts" notification (Resolve…, Continue, Abort, and View Stash… when changes were stashed), and asks before rebasing published commits. A rebase that cannot start says "Rebase not allowed" with the unfinished operation. Abort Merge and Abort Rebase (menu, popup and notification) ask first ("Abort merge?", "Abort rebase in <repository>?" with Abort and Cancel) and report "Merge abort succeeded" / "Abort rebase succeeded" or "Merge abort failed" / "Abort rebase failed".
- Update Project… and Update pull the current branch with the configured method (merge, rebase or the branch default). The options dialog can be turned off with "Don't show this dialog again"; the row then reads "Update Project" and Shift-click brings the dialog back.
- Delete removes a local branch without asking. A branch that is not fully merged is deleted anyway, and the notification offers Restore and View Commits. Deleting a remote branch asks first; deleting a tag does not, and its notification offers Restore.
- Rename… and Push… open their dialogs; Push… is preset to the selected branch.
- Compare with 'current' opens a tab with the commits that exist in one branch but not in the other, in both directions. Show Diff with Working Tree opens the diff of the selected ref against your working tree.
- The Branch Name field of New Branch… and the field of Checkout Tag or Revision… suggest matching branches, directories and tags below the field (click, Tab or Enter accept a suggestion).

### Not implemented

- Worktree items ("New Worktree…", "Open existing worktree").
- The "Tracked Branch" submenu.
- "Restore Popup Size": the popup is not resizable.
- Running one branch operation on all repositories at once; Flint works on one repository at a time.
- "Commit and stage" and the cherry-pick and revert ongoing-operation actions.

## Search Everywhere

Search Everywhere is a WebStorm-style popup that searches files, classes, symbols, actions and text at once. With the macOS JetBrains base keymap it opens on a double `Shift`; with any keymap it is available as `search everywhere: toggle` in the command palette. Like WebStorm, it has a row of tabs above the query and a preview of the selected item. The popup surface is always opaque, even when your theme overrides make `elevated_surface.background` translucent, like the Git Branches popup.

| Tab | What it searches |
| --- | --- |
| All | Classes, Files, Symbols, Actions and Text, a few results each, grouped under section headers. Every source contributes whenever there is a query. A `more…` row opens the full tab for that section. An empty query lists the recently opened files. |
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

### Search panel

The Search tool window sits in the left dock next to Project and Git, with the title "Search". It hosts the same tabs and sources as the popup, fills the dock and keeps the preview hidden until you toggle it.

- Toggle it with `search panel: toggle focus`, `Cmd-3` (`Alt-3` on Linux) with the JetBrains keymaps, `Ctrl-Shift-S` on macOS (`Alt-Shift-S` on Linux and Windows) with the default keymaps, or View → Search Panel. Settings: `search_panel.button`, `search_panel.dock` (`left`, `right` or `bottom`) and `search_panel.default_width`, also under Panels in the settings window.
- `Cmd-Shift-F`, Find in Project, the tab "+" menu entry "Search Project" and the Project panel's "Find in Folder…" open the panel on the Text tab, seeded from the buffer search query or the editor selection. "Find in Folder…" also limits the Text tab to that folder; the chip "Text in <folder> ×" shows the limit and removes it.
- Enter opens a result and keeps the panel and its results; Escape moves focus back to the editor.
- Replace in project and the regex, whole-word, case, include and exclude options of `pane: deploy search` still open the project search tab.

### Not implemented

- Math evaluation in the query.
- `/` to list settings groups.
- The Git tab (branches and commits).
- The filter popup (for example, only recent files) and "Open in Find tool window" for tabs other than Text.

### Project Diagnostics panel

The Project Diagnostics tool window sits in the left dock next to Project, Git and Search, with the title "Project Diagnostics Panel". It hosts the project diagnostics view that used to open as an editor tab.

- Its tool window bar button uses the warning icon (tooltip "Project Diagnostics Panel"). While the project has errors, the button shows the error count as a badge; there is no badge at 0 errors.
- Toggle it with `diagnostics: deploy`: `Cmd-Shift-M` on macOS, `Ctrl-Shift-M` on Linux and Windows with the default keymaps, `Cmd-6` (`Alt-6` on Linux) with the JetBrains keymaps, or View → Diagnostics. The first press opens the panel and focuses it; a second press returns focus to the editor. With the JetBrains keymaps `Cmd-6` (`Alt-6`) also hides the dock while the panel has focus.
- Settings: `project_diagnostics_panel.button` (default `true`; "Hide Button" in the button's right-click menu sets it to `false`), `project_diagnostics_panel.dock` (`left`, `right` or `bottom`, default `left`) and `project_diagnostics_panel.default_width` (default 360), also under Panels → Project Diagnostics Panel in the settings window. In the bottom dock the panel is 320 px high. The panel's dock position is restored on restart. The old `diagnostics.button` setting no longer exists.
- The diagnostics view is created when the panel is first shown, so no diagnostics buffers are opened before that; until then the panel shows "Checking diagnostics…".
- Header buttons: Refresh Diagnostics (it becomes Stop Diagnostics Update while an update runs; shown once the view exists) and Toggle Warnings.
- Compared with the old editor tab, these are gone: the buffer search button and the include-warnings colour state of the pane toolbar, navigation history and the tab itself (title, splitting, per-tab badge). The "Show N warnings" button of the empty state remains. The Toggle Warnings header button cannot show an on or off colour. The panel has no zoom.
- A Flint dock shows one panel at a time, so opening this panel in the left dock replaces the visible Project, Git or Search panel.

### Language Services panel

The Language Services tool window sits in the left dock next to Project, Git, Search and Project Diagnostics, with the title "Language Services Panel".

- Its tool window bar button uses the bolt icon (tooltip "Language Services Panel"). While the panel is not shown, the button shows the number of language servers in an error or warning state as a badge; there is no badge when there are none.
- Toggle it with `lsp_tool: toggle focus`: `Ctrl-Cmd-L` on macOS, `Ctrl-Alt-L` on Linux, `Shift-Alt-L` on Windows, or View → Language Services. The JetBrains keymaps add no key for it and no key that hides the dock while it has focus.
- The panel lists the language servers grouped by worktree and follows the active editor. A restricted-mode banner comes first and opens the worktree security dialog on click. Each server row shows a status dot (green running, yellow starting, red error, yellow warning, grey stopped), the name, the status, the version and the memory use, and the server message if any. Row buttons: View Message (only with a message), View Logs (only when logs exist), Restart Server and Stop Server (only when the server can be stopped). Restart All Servers and Stop All Servers sit at the end of the list. An empty list shows "No language servers".
- Settings: `language_services_panel.button` (default `true`; "Hide Button" in the button's right-click menu sets it to `false`), `language_services_panel.dock` (`left`, `right` or `bottom`, default `left`) and `language_services_panel.default_width` (default 360), also under Panels → Language Services Panel in the settings window. The old `global_lsp_settings.button` setting no longer exists.
- The per-server actions are row buttons; there are no hover submenus. The panel has no header buttons.
- A Flint dock shows one panel at a time, so opening this panel in the left dock replaces the visible Project, Git, Search or Project Diagnostics panel.

## Merge conflicts

Flint resolves merge conflicts like WebStorm 2026.2.3's [Resolve conflicts](https://www.jetbrains.com/help/webstorm/resolve-conflicts.html) with the iterative flow: a "Conflicts" dialog lists every conflicted file, and each file opens in a three-pane "Merge Revisions" window. Both are separate native windows with their own saved position and size, not editor tabs. They exist only for local repositories and are hidden in remote projects.

### Opening the Conflicts dialog

- Git panel: the "Resolve…" button in the header of the Conflicts section (tooltip "Open the Conflicts dialog"), or `merge tool: resolve conflicts` in the command palette (`merge_tool::ResolveConflicts`, WebStorm's Git → Resolve Conflicts…). It works for conflicts made on the command line too.
- Automatically after an operation that leaves conflicts: merge, update, rebase, stash pop and the restore step of a smart checkout. The dialog reads the repository state, so during a rebase the sides are swapped as in WebStorm and "Yours" stays your own commits.
- What happens when the dialog closes depends on the operation:

| Operation | All files resolved | Conflicts remain |
|---|---|---|
| Merge | Accept and Finish creates the merge commit, only while a merge is in progress | Warning "<branch> Merged with Conflicts" with Resolve… |
| Update by merge, Pull in the Git panel | The merge commit is created | Warning "Cannot complete update" with Resolve… |
| Update by rebase, Pull with rebase in the Git panel, unfinished rebase | `rebase --continue` runs by itself, skips an empty commit, and reopens the dialog at the next conflicting commit | Warning "Cannot continue rebase" with Resolve… |
| Rebase from the branch popup, Checkout and Rebase onto | The rebase is continued | Warning "Rebase stopped due to conflicts" with Resolve…, Continue and Abort (and View Stash… when changes were stashed). Continue with conflicts left opens the Conflicts dialog. When Resolve… finds nothing left to resolve it asks "All conflicts have been resolved. Do you want to continue rebase?" |
| Unfinished rebase, unfinished merge or unmerged files found before an update | The dialog opens straight away (as in WebStorm's update checks): an unfinished rebase is continued, the update goes on | Warning "Cannot update" with Resolve… |
| Stash pop | Nothing more to do | Warning "Conflicts were not resolved during unstash" with "Resolve conflicts…" |
| Smart checkout or update restoring local changes | Nothing more to do | Warning "Local changes were restored with conflicts" with "View saved changes…" and "Resolve conflicts…" |

Choosing Resolve… in a notification reopens the dialog; if conflicts are still left, the notification comes back as "Pending Unresolved conflicts".

### Conflicts dialog

- Title "Conflicts". A line above the table says what is being merged, for example "Merging branch feature into branch main", "Rebasing branch …", "Conflicts during unstashing …" or "The following files have conflicts:".
- The table has a Name column and the columns "Yours (<branch>)" and "Theirs (<branch>)" (a short hash replaces a branch name that does not exist). Files are grouped under Unresolved and Resolved. A file row shows its name, the muted parent directory, a badge with resolved/total changes and, in the side columns, whether that side modified or deleted the file. Selection supports Cmd-click and Shift-click, Up/Down, Shift-Up/Down, PageUp/PageDown (one visible page), Shift-PageUp/PageDown, Ctrl-Home/Ctrl-End (first/last row), Ctrl-Shift-Home/End, Cmd-A and Ctrl-A (all rows). Home and End do nothing, as in WebStorm's table.
- The View Options button (eye icon) has "Group by directory". The choice is remembered per project root.
- The "Accept" button appears in the Yours and Theirs cells of the hovered unresolved file, or of the single selected one. Accept takes that whole side for every selected unresolved file. If a file already has merged changes, "Overwrite Changes" asks first ("Discard and Accept"). Text files are written and staged when the dialog closes; files without a text model (binary, not UTF-8, too big, read-only) are accepted with git straight away.
- Right-click on the selection: Accept ‘Yours (…)’, Accept ‘Theirs (…)’, then Revert Conflict Resolution. Revert is enabled only for resolved files; "Confirm Revert" returns them to their original conflicted state.
- The "Resolve All Simple Conflicts" button (wand icon) applies non-conflicting changes and resolves simple conflicts in every unresolved file, under a cancellable "Resolving simple conflicts…" overlay. A line next to it reports the result: "No conflicts were resolved automatically", "All conflicts were resolved automatically" or "2 conflicts resolved. 3 conflicts in 1 file still require attention". The button is disabled when there is nothing to resolve automatically.
- "Resolve Manually" (or "Review Changes" when every selected file is resolved) and a double-click on a file open the Merge Revisions window. Windows open one after another: the selected files first, then the other unresolved files, then resolved files not yet reviewed. "Save and Close" in a window returns to the dialog; "Apply Changes" goes on to the next file. Binary files cannot be opened ("Cannot Show Merge Dialog: Binary files cannot be merged manually"); neither can read-only files ("Cannot resolve conflicts in a read-only file").
- "Accept and Finish" is enabled once every file is resolved; it closes the dialog and applies the finish rule of the operation from the table above. The default button (accent colour, run by Enter, Ctrl-Enter or Cmd-Enter) is "Accept and Finish" when all files are resolved and reviewed, otherwise Resolve Manually / Review Changes. In that all-reviewed state the line above the table reads "All conflicts have been resolved" with a green check and the review button is hidden.
- "Close" writes and stages the fully resolved files. If some files are only partially resolved it first asks "Discard Changes?" ("Closing the dialog will discard your changes in partially resolved files", buttons "Discard Changes" and "Continue Merge"). Esc, Cmd-W and the window close button close without asking and apply the same rule.
- Speed search: typing selects the next file whose name contains the text (case-insensitive), shown as a chip; Backspace edits it and Esc clears it.
- While files are being resolved or written, a progress overlay blocks the table. Confirmations (Discard Changes?, Confirm Revert, Overwrite Changes) are opaque cards inside the dialog with WebStorm's question icon; Enter answers the default (right) button and Esc the other one.

### Git panel

- The context menu of a conflicted file starts with Merge…, Accept Theirs and Accept Yours (WebStorm's `Git.ChangesView.Conflicts` group), followed by a separator.
- Double-clicking a conflicted file opens it in the Merge Revisions window; a single click keeps its usual behaviour.
- Accept Theirs and Accept Yours run `git checkout --theirs` / `--ours` for the file and stage it at once. During a rebase the two sides are swapped, so Yours is still your own version.
- A window opened from the Git panel is standalone, not part of the iterative flow: its bottom buttons are Accept Left, Accept Right, Cancel and Apply Changes. Cancel leaves the file untouched (after asking when the Result was edited); the other three write the file and stage it.

### Editor notification

- A text editor showing a file that is in a git conflict state gets a banner above its content, like WebStorm's editor notification: a warning banner "File has unresolved merge conflicts" with the link "Resolve conflicts…", which opens the standalone Merge Revisions window for that file (not the Conflicts dialog).
- While that window is open the banner becomes an information banner "Resolving merge conflicts is in progress" with the links "Show resolve conflicts window" (brings the window to the front) and "Cancel resolve" (closes the window the same way its Cancel button does, so it asks first when the Result was edited).
- The banner is not shown when both sides deleted the file, when the file is missing on disk, or in multi-file views such as the project diff. It refreshes when the repository status changes and when the Merge Revisions window opens or closes, and exists only for local projects.

### Merge Revisions window

- Title "Merge Revisions for <path>". Three panes: Left = Yours (read-only), Result (editable, starts from the base revision) and Right = Theirs (read-only). Each pane has a title strip; the side panes say what they contain ("Your version, branch …", "Changes from branch …", "Rebasing <hash> from …", "Local changes", "Changes from stash", …) with a "Show Details" link that opens the commit details and WebStorm's read-only icon. The Result strip shows the file path (tooltip: the home-relative path). When the line separators of the three texts differ, each strip also shows LF, CRLF or CR in WebStorm's colours.
- Ribbons link corresponding changes between the panes. Drag a divider to resize the panes; a double-click on a divider gives the Result the full width, and another one restores equal thirds; the mouse wheel over a divider scrolls the Result. Scrolling is synchronised across the panes by default. Folded unchanged fragments (wavy lines in the editors, the gutter and the dividers) expand when you click the folded line, its placeholder or the chevron in the gutter. "Show Line Numbers" off removes the number column of the gutters.
- Gutter icons on each change: Accept (arrows, which become "append" arrows after the other side was applied to a conflict) and Ignore (cross) on the side panes; in the Result pane a wand (Resolve, for a simple conflict). Ctrl+click on Accept resolves the conflict using that side and ignores the other; Ctrl+click on Ignore ignores the whole conflict. On macOS a right-click on an icon does the same, because the system reports Ctrl+click as a right click.
- Right-click menu in a pane: Accept, Resolve using Left / Right, Ignore, Resolve Automatically (only the entries that apply to the selection), Revert (shown whenever text is selected; it reverts the resolved changes in the selection), then Collapse Unchanged Fragments and Synchronize Scrolling.
- Toolbar, left to right: Previous Difference, Next Difference, Collapse Unchanged Fragments, "Apply non-conflicting changes:" Left / All / Right, Resolve Simple Conflicts, Revert Conflict Resolution. The status shows a spinner while differences are computed, then "2 changes, 1 conflict" or a green "All conflicts resolved". The gear menu has Synchronize Scrolling; Ignore Differences (None, Trim whitespaces, Ignore whitespaces); Highlighting Differences (Lines, Words); and an Appearance submenu with Show Whitespaces, Show Line Numbers, Show Indent Guides and Soft-Wrap. Changing Ignore Differences or Highlighting restarts the merge and first asks "Update Highlighting Settings" when the Result was edited.
- When the last change is processed, a green "All changes have been processed" panel with an "Apply Changes" link appears in the Result pane.
- Undo and redo in the Result also undo and redo the accept and ignore states.
- Bottom buttons in the iterative flow: Accept Left and Accept Right take a whole side, "Save and Close" keeps the current Result for the dialog, "Apply Changes" (default) saves the Result. If changes or conflicts remain, Apply Changes first asks "Unprocessed Changes" ("Save Current Result" / "Back to Resolving"). Accept Left, Accept Right and the standalone Cancel ask for confirmation ("Discard Changes and …" / "Continue Merge") when the Result has unsaved edits. Apply Changes is disabled while differences are being computed or if the computation failed. A file whose differences cannot be computed shows a warning banner "Unable to calculate diff. File is too big and there are too many changes." with a "Hide" link. Esc and Cmd-W close the window like Cancel; in the iterative flow they never ask.

### Keyboard shortcuts

These bindings are in the default and the JetBrains macOS keymaps.

| Shortcut | Action | Where |
|---|---|---|
| F7, Cmd-] | `merge_tool::NextDifference` | Merge Revisions window, also with an editor focused |
| Shift-F7, Cmd-[ | `merge_tool::PreviousDifference` | Merge Revisions window |
| Ctrl-Cmd-Right | `merge_tool::AcceptLeftSide` (accept the left side of the change at the cursor) | Merge Revisions window |
| Ctrl-Cmd-Left | `merge_tool::AcceptRightSide` (accept the right side of the change at the cursor) | Merge Revisions window |
| Ctrl-Shift-Tab | `merge_tool::FocusOppositePane` | Merge Revisions window |
| Cmd-Enter | `merge_tool::ApplyChanges` | Merge Revisions window, also from the Result editor |
| Cmd-A, Ctrl-A | `merge_tool::ConflictsSelectAllRows` | Conflicts dialog |
| Shift-Up, Shift-Down | `merge_tool::ConflictsExtendSelectionUp`, `ConflictsExtendSelectionDown` | Conflicts dialog |
| Left, Right | `merge_tool::ConflictsCollapseRow`, `ConflictsExpandRow` | Conflicts dialog |
| PageUp, PageDown | `merge_tool::ConflictsPageUp`, `ConflictsPageDown` | Conflicts dialog |
| Shift-PageUp, Shift-PageDown | `merge_tool::ConflictsExtendPageUp`, `ConflictsExtendPageDown` | Conflicts dialog |
| Ctrl-Home, Ctrl-End | `menu::SelectFirst`, `menu::SelectLast` | Conflicts dialog |
| Ctrl-Shift-Home, Ctrl-Shift-End | `merge_tool::ConflictsExtendToFirstRow`, `ConflictsExtendToLastRow` | Conflicts dialog |
| Home, End | unbound (do nothing) | Conflicts dialog |
| Cmd-W | `workspace::CloseWindow` | Conflicts dialog and Merge Revisions window |
| Enter, Ctrl-Enter, Cmd-Enter | `menu::Confirm`, `menu::SecondaryConfirm`: run the default button | Conflicts dialog |
| Esc | `menu::Cancel`: clears the speed search, then closes | Conflicts dialog |

The other actions of the window (`merge_tool::NextConflict`, `PreviousConflict`, `IgnoreLeftSide`, `IgnoreRightSide`, `ResolveUsingLeft`, `ResolveUsingRight`, `ApplyNonConflictingLeft`, `ApplyNonConflictingAll`, `ApplyNonConflictingRight`, `ResolveSimpleConflicts`, `RevertConflictResolution`, `ToggleSynchronizeScrolling`, `ToggleCollapseUnchangedFragments`, `AcceptLeft`, `AcceptRight`, `SaveAndClose` and so on) have no default shortcut; they are in the command palette and can be bound in `keymap.json` with the context `MergeView`.

### Setting

```json
"git": { "merge_tool": { "auto_apply_non_conflicting": false } }
```

With `true`, the Merge Revisions window applies all non-conflicting changes as soon as the differences are computed. Settings window: Version Control → Merge → "Automatically Apply Non-Conflicting Changes".

### Other saved state

- Position and size of the Conflicts dialog (key `MultipleFileMergeDialog`) and of the Merge Revisions window (key `MergeDialog`), per project. The size is saved only when it differs from the default; a window that no longer fits the screen is moved back inside.
- "Group by directory" of the Conflicts dialog, per project root.
- The gear menu choices of the Merge Revisions window are remembered across windows and restarts (not per project): Ignore Differences, Highlighting Differences, Show Whitespaces (default off), Show Line Numbers (default on), Show Indent Guides (default off), Soft-Wrap (default off) and Collapse Unchanged Fragments (default on). Synchronize Scrolling is not remembered and starts on in every window. A model that the Conflicts dialog already prepared keeps the Ignore Differences policy it was prepared with.

### Not implemented

- Compare Contents popup of the Merge Revisions window (Left / Middle / Right partial diffs and "Base and …") and "Compare with Clipboard" in the editor menu: they need a standalone two-side diff window, which Flint does not have (`MergeThreesideViewer`).
- Binary merge viewer: binary files can only be accepted from one side (`BinaryMergeTool`).
- Semantic conflict resolution (PSI, import merging) and resolution by AI are not ported; they are inert for Git conflicts in WebStorm 2026.2.3.
- Gear menu: Breadcrumbs, Context Help and "Show Diff in Editor Tab / New Window" are missing. The Ignore Differences and Highlighting Differences choices carry check marks instead of radio marks, and the right-click menu has no icons.
- Title strips: no charset label (Flint reads every revision as UTF-8 and rejects other encodings), the line-separator label of the Result is taken from the base text (WebStorm uses the working-tree file), the labels cannot be selected, and there is no read-only tooltip (`DiffEditorTitleDetails`).
- The Left pane keeps its scrollbar on the right because the editor has no left-hand scrollbar.
- Scrollbar stripe marks for changes, the expanded-region chevrons, the enclosing-class hint on fold waves, and the hand cursor on the two panes that are not under the pointer when a fold is hovered. Next/Previous Difference and Apply Non-Conflicting scroll without animation.
- After Ignore Differences restarts the merge the folds are rebuilt in the default state (WebStorm restores the expanded ones, `ExpandSuggester`). A selection that touches a collapsed fold selects only the caret row for the merge actions. Ctrl+Alt+Shift+Up/Down are not consumed.
- The diff row highlights keep the editor's own z-order (below the text selection, above the active line); WebStorm paints them between syntax and selection layers.
- Ctrl+click on macOS: a real right-click on a gutter icon is indistinguishable from Ctrl+click, because GPUI reports Ctrl+click as a right-button event; WebStorm reacts only to the left button (`DiffGutterRenderer.performAction`).
- The "All changes have been processed" notice is drawn inside the Result pane instead of a native balloon popup; it hides on mouse press, key press or window resize.
- "Show Details" opens a Flint-built window with the commit or commit range (list, changed files, "Filter by conflicted file") instead of `ChangeListViewerDialog`.
- The pane editors have no project, so they have no language-server features while merging.
- The Merge Revisions window opens after the revisions are loaded (WebStorm shows it empty first with a loading overlay). The minimum size is 700×450 where WebStorm derives it from the content, the buttons are 24 px high like WebStorm's compact density (28 px otherwise), disabled buttons are dimmed and have no hover, pressed or focus-ring state, and the light-theme button colours are WebStorm's light values. Cmd-Enter does nothing while Apply Changes is disabled (WebStorm runs it anyway).
- Confirmation dialogs of both windows are opaque cards inside the window with the title as a bold first line, not separate dialog windows with a title bar.
- Conflicts dialog: no Dock bounce when it opens, no cancellable "Loading unmerged files…" progress, no block on project reloads while it is open, a context menu is closed by focus loss rather than by a resize, no vertical separators between header cells, one selection colour whether the window is focused or not, speed search is a case-insensitive substring match without highlighting (WebStorm uses word-prefix and wildcard matching), the inline Accept button is Flint's subtle button instead of `ActionToolbar.smallVariant`, Tab and Shift-Tab move between rows, not between cells, and the Resolve button spinner is a rotating icon rather than the 8-step `AnimatedIcon`.
- The fallback description texts of `GitStashChangesSaver`, and the raw (unbolded) rebase description that WebStorm shows when `REBASE_HEAD` is missing, are not reproduced; Flint always passes its own restore description and renders the bold.
- Modality: the dialog and the windows block only the workspace window that opened them, while WebStorm's `IdeModalityType.IDE` blocks all IDE windows. The native close button and Cmd-Q of the workspace window still work while a dialog is open. The zoomed state of a window is not saved, and bounds are written when the window closes, not at quit.
- Operations around the dialog that Flint does not have, so their WebStorm flows are not ported: cherry-pick and revert conflicts (`GitApplyChangesConflictResolver`) and their Abort Cherry-Pick / Abort Revert actions, the "Abort and Rollback" variants of Abort Rebase, the "Rebase stopped for editing" notification as an actionable notice (inside the continue loop it is an information notice without Continue or Abort), "Continue rebase failed" with "Stage and Retry" / "Show Files" after `rebase --continue` finds unstaged changes (Flint reports the remaining conflicts instead), the "View Files" dialog of the "Untracked Files Prevent …" notifications (the files are listed in the notification), the "Cannot commit changes due to unresolved conflicts" commit check (the Git panel disables Commit instead), the unstash index-conflict notice, a Git → Resolve Conflicts… menu entry (the action is in the command palette and the Git panel only). Flint also refuses to start a merge while a merge or rebase is unfinished, before running git.
- Smart operations keep Flint's own stash mechanism (WebStorm's `GitPreservingProcess`): the restore step opens the Conflicts dialog with WebStorm's texts, but the stash is restored by `git stash pop` without `--index`. A merge or update that leaves conflicts while changes are stashed shows the WebStorm warning "Local changes were not restored" ("Before merge your uncommitted changes were saved to stash.") with "View saved changes…"; WebStorm restores the stash after the dialog in every case, Flint only once everything is resolved.
- The Git Conflicts tool window, the Changes view banner, the hover icons (Merge, Rollback) of changed files and the non-modal merge in the Changes view: they are off by default in WebStorm (registry keys `git.merge.conflicts.toolwindow` and `vcs.non.modal.merge.enabled`), so they are not built. Revert Resolved Files in the Git panel is not built either; use the Conflicts dialog.

## Languages and code navigation

Cmd+click (and `Cmd-B`, `Alt-F7` with the JetBrains keymap) goes to a definition or lists usages through the file's language server, so it works once that server runs:

- Built in, no install step: Rust (rust-analyzer from `rustup` or `PATH`, otherwise downloaded), TypeScript and JavaScript (the native TypeScript 7 server `tsc --lsp`, a Go binary downloaded from npm on first use that runs without Node; to cap its memory set `"lsp": { "tsgo": { "binary": { "env": { "GOMEMLIMIT": "1GiB" } } } }`), ESLint, JSON, YAML. C, CSS, Go, Python and Bash keep syntax highlighting (Python also keeps its toolchain selection) but have no language server. C++ is not supported.
- TypeScript and JavaScript diagnostics arrive through LSP pull diagnostics (`diagnostics.lsp_pull_diagnostics.enabled`, on by default), so turning that setting off hides them. If you point `lsp.tsgo.binary.path` at your own build, also set `"arguments": ["--lsp", "--stdio"]`, because a binary override replaces the default arguments.
- Servers that only make sense for some projects start only when the project uses them. `eslint` needs an `eslint.config.{js,cjs,mjs,ts,cts,mts}` or `.eslintrc*` file, or `eslint` in the `dependencies`, `devDependencies`, `peerDependencies` or `optionalDependencies` of a non-ignored `package.json`. `roslyn` needs a `.sln`, `.slnx` or `.csproj`. `jdtls` needs `pom.xml`, `build.gradle(.kts)`, `settings.gradle(.kts)`, `.project` or `.classpath`, and `gradle-language-server` needs a Gradle file. `marksman` needs a `.marksman.toml`, `.obsidian`, `mkdocs.yml`, `mkdocs.yaml` or `book.toml` entry. Listing a server by name in `language_servers` starts it regardless of these rules.
- Installed automatically on first launch from the extension registry: Material Icon Theme, TOML, `.env` and Git Firefly. Language servers are pulled per project: C# (`csharp`, Roslyn) is installed once a worktree contains a `.sln`, `.slnx` or `.csproj` file, and Java (`java`, jdtls) once it contains a `pom.xml`, `build.gradle`, `build.gradle.kts`, `settings.gradle` or `settings.gradle.kts`. Other languages, such as Kotlin, PHP, Swift or Vue, get their extension after you accept the install suggestion shown when you open such a file. To skip an automatic install, set it to `false` in your `settings.json`: `"auto_install_extensions": { "csharp": false }`.
- C# needs a .NET runtime. If .NET is installed only in `~/.dotnet` and `DOTNET_ROOT` is not set, Flint passes `DOTNET_ROOT=~/.dotnet` to language servers, so Roslyn starts without extra setup. A `DOTNET_ROOT` or `DOTNET_ROOT_ARM64` you set yourself, in the shell or in `lsp.<server>.binary.env`, always wins.
- Each server still needs its project files to resolve symbols: `Cargo.toml` for Rust, a `.csproj` or `.sln` for C#, and a JDK for Java.

## Memory and startup

- The Performance Profiler window is removed, so the profiler's trace buffers are never collected.
- Extensions are compiled to native code once and cached in the `.compiled-components` folder inside the extensions work folder, so later launches skip the compilation. Delete the folder to rebuild the cache; a changed or incompatible extension is recompiled by itself.
- Bundled fonts, themes and settings are embedded uncompressed, so they stay in file-backed memory instead of being unpacked into the heap at launch.

## Licensing

Zed source code is licensed under GPL-3.0-or-later, with Apache-2.0 components where marked; see [LICENSE-GPL](LICENSE-GPL) and [LICENSE-APACHE](LICENSE-APACHE).

Third-party licenses are generated by [`cargo-about`](https://github.com/EmbarkStudios/cargo-about) from [`script/licenses/zed-licenses.toml`](script/licenses/zed-licenses.toml) during bundling and embedded in the app. The build fails if a dependency's license is not in the accepted list.

The `assets/icons/tool_window_*.svg` files, `assets/icons/locate.svg`, `assets/icons/expand_all.svg` and `assets/icons/collapse_all.svg` are unmodified copies of [JetBrains/intellij-community](https://github.com/JetBrains/intellij-community) icons (commit `0ad391f70a62eabbec6db656f473e1284e1110a9`), licensed under Apache-2.0, each keeping its original JetBrains copyright header. Sources: `platform/icons/src/expui/toolwindows/{project,vcs}.svg`, `platform/icons/src/expui/general/moreHorizontal.svg` and `plugins/terminal/resources/icons/expui/toolwindow/terminal.svg`; the 20×20 drawing is the `@20x20` file and the 16×16 drawing is the file without the suffix. `locate.svg`, `expand_all.svg` and `collapse_all.svg` are `platform/icons/src/expui/general/{locate,expandAll,collapseAll}.svg` under snake_case names. JetBrains and IntelliJ names and logos are trademarks of JetBrains s.r.o. The default colours in `theme_overrides."Eva Dark"` are taken from `platform/platform-resources/src/themes/islands/ManyIslandsDark.theme.json` of the same commit, Apache-2.0.

The Git Branches popup icons `assets/icons/{fetch,current_branch_label,current_branch_favorite_label,tag_label,branch_node,incoming_commits,outgoing_commits,favorite_outline,menu_arrow}.svg` are unmodified copies of [JetBrains/intellij-community](https://github.com/JetBrains/intellij-community) icons (commit `0ad391f70a62eabbec6db656f473e1284e1110a9`), licensed under Apache-2.0, each keeping its original JetBrains copyright header. Sources, in the order above: `platform/icons/src/expui/vcs/fetch.svg`, `platform/dvcs-impl/shared/resources/icons/new/{currentBranchLabel,currentBranchFavoriteLabel,branchLabel}.svg`, `platform/icons/src/vcs/branchNode.svg`, `platform/dvcs-impl/shared/resources/icons/new/{incomingUpdate,outgoingPush}.svg`, `platform/icons/src/nodes/notFavoriteOnHover.svg` and `platform/icons/src/icons/ide/menuArrow.svg`.
