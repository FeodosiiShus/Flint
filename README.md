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

The app is ad-hoc signed and not notarized, so macOS Gatekeeper blocks a copy that carries the `com.apple.quarantine` attribute. Install it with [`script/install.sh`](script/install.sh). It mounts the DMG, copies `Flint.app` into `/Applications` (replacing an existing copy), removes the quarantine attribute from the whole bundle, checks that no file still carries it, and detaches the image. It refuses to run while Flint is open.

Latest release:

```sh
curl -fsSL https://raw.githubusercontent.com/FeodosiiShus/Flint/main/script/install.sh | sh
```

A specific release, by its tag:

```sh
curl -fsSL https://raw.githubusercontent.com/FeodosiiShus/Flint/main/script/install.sh | FLINT_VERSION=vX.Y.Z sh
```

A build from an Actions run (the `flint-macos-aarch64` artifact zip, or the `Flint-aarch64.dmg` inside it):

```sh
sh script/install.sh ~/Downloads/flint-macos-aarch64.zip
curl -fsSL https://raw.githubusercontent.com/FeodosiiShus/Flint/main/script/install.sh | sh -s -- ~/Downloads/flint-macos-aarch64.zip
```

The download uses GitHub's [`/releases/latest/download/<asset>`](https://docs.github.com/en/repositories/releasing-projects-on-github/linking-to-releases) link, which points at the newest published release; drafts and prereleases [cannot be latest](https://docs.github.com/en/rest/releases/releases), so the one-line install works once a `v*` release exists. `FLINT_INSTALL_DIR` installs somewhere other than `/Applications`.

The DMG cannot do this by itself: dragging the app out of it is a plain copy that runs no code, and [Gatekeeper checks downloaded apps the first time they are opened](https://support.apple.com/guide/security/gatekeeper-and-runtime-protection-sec5599b66df/web). The only way to avoid the check without a script is [notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution), which needs a Developer ID certificate.

Manual alternative:

```sh
xattr -dr com.apple.quarantine "/Applications/Flint.app"
```

The app is built on the `dev` release channel and is named "Flint". It has no auto-update, Zed account sign-in or onboarding flow.

Flint ships none of Zed's built-in AI: no agent panel, threads sidebar, inline assist, language model providers, MCP servers, external agents or commit message generation. It also drops the debugger panel, notebooks (REPL), dev containers, the headless remote server, vim mode, the image viewer, the theme, tab, toolchain and settings profile pickers, call hierarchy, the Outline panel (the symbol outline popup of `outline: toggle` stays), the editor minimap, the journal, the SVG preview, the snippets picker, the syntax tree, highlights tree and key context views, the language servers for Tailwind CSS, CSS, Bash, Go, Python, C and C++, Vue, the task runner (`tasks.json` and `debug.json`, the Run menu, gutter run buttons, `task: spawn`), the status bar, and every non-macOS platform backend. Use the terminal panel to run AI command-line tools.

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
- `panel.height` — the exact height of the tool window headers of dock panels, but never lower than their buttons. Unset, the headers use the tab bar height (41 px, 31 px compact).
- `panel.icon_size` — the icon size of the buttons in those headers: the panel's own header actions, Options and Hide. Panel content and the terminal panel tabs are not affected; the terminal panel tabs follow `tab_bar`.
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
- Tool window bars run down the left and right window edges. The left bar shows the left dock panels (by default Project), a separator, a "More Tool Windows" button (a menu of every panel, including panels whose button is hidden), and the bottom dock panels in its lower group. Like the left-bottom stripe of IntelliJ's new UI, the lower group is drawn from the bottom up: the panel with the lowest activation priority is the bottom-most button, so from top to bottom the default order is Language Services, Search, Terminal, Project Diagnostics and Git. The right bar shows the right dock panels. Click an icon to show or hide its panel; right-click it to move the panel to another dock or hide its button. A filled background marks an open panel, an accent background a focused one.
- The right bar disappears when no right dock panel has a button, so it never leaves an empty strip. The editor island then keeps exactly the islands gap to the window edge, the same as on every other side.
- Tool window headers: every open dock island starts with a header like a WebStorm tool window header: the panel name in semibold, then the panel's own buttons, ⋮ (the panel menu: move to another dock, hide the button) and — (hide the panel). Like WebStorm, the buttons appear only while the pointer is over the tool window or the tool window has focus. Right-clicking the header opens the same menu. A panel that hosts its own tab strip, like the terminal, has no common header: its tabs start at the top of the island. A panel adds its own buttons by implementing `Panel::header_actions`; the Project panel and the Project Diagnostics panel are the only ones that do. A panel can also show its own content, such as tabs, right after the name by implementing `Panel::header_content`; the name then keeps IntelliJ's tool window header label insets, 12 px before it and 16 px between it and the content.
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
- WebStorm-like defaults that come with it: `tabs.file_icons: true`, `tab_bar.show_nav_history_buttons: false`, and the Git, Search, Project Diagnostics and Language Services panels docked at the bottom next to the terminal, like WebStorm's bottom tool windows, each 320 px high by default.
- Not implemented: WebStorm's split tool windows under the bar separator and the bottom-right bar group, because a Flint dock shows one panel at a time. Opening the Git, Search, Project Diagnostics or Language Services panel replaces the terminal or the other bottom panel while it is open. `workspace: toggle bottom dock` shows the last shown bottom panel, or the bottom-most one, Git, when none was shown yet.

## Default look: Islands Dark chrome on the Eva theme

- The default look is the chrome of JetBrains' Islands Dark theme (`ManyIslandsDark.theme.json`, intellij-community commit `0ad391f70a62eabbec6db656f473e1284e1110a9`) on top of the Eva Dark theme, like IntelliJ's Islands Dark UI theme with the Eva editor colour scheme. The theme is still named "Eva Dark".
- Chrome copied from Islands Dark: the frame, tool window islands, title bar, tool window bars, popups and menus, hover, pressed and selected states, borders, text and icon colours, the scrollbar thumb and the drop target.
- Stays Eva: syntax, terminal palette, players, git and diagnostic status colours, search highlight and the other editor and gutter colours. `assets/themes/eva/eva.json` is unchanged.
- Editor colours taken from the JetBrains Eva Dark editor scheme (`Eva_Dark.xml` of the Eva-Theme plugin, the scheme WebStorm uses with Eva): indent guides and the gutter separator `#454963` (`INDENT_GUIDE`; the active guide has the same colour, because IntelliJ highlights a guide only while the caret sits on its column), inlay hint text `#747ba4` (`syntax.hint`) on `#383c45` at IntelliJ's 55% hint opacity (`hint.background` `#383c458c`, `INLINE_PARAMETER_HINT`), and matched braces `#7b524b` (`editor.document_highlight.bracket_background`, `MATCHED_BRACE_ATTRIBUTES`). The test-file tab colour `tab.test_file_background` is Islands Dark's `FileColor.Green` `#1c261c`.
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

- The nine IntelliJ project colours `#e08855`, `#b08b14`, `#a1a359`, `#3b92b8`, `#3574f0`, `#c84d8f`, `#955ae0`, `#24a394` and `#5fad65` (IntelliJ `Color1`–`Color9` `Avatar.Start`) are the theme `accents`. They are the palette of the bracket pair colours while bracket colorization is on (`colorize_brackets`; Flint adjusts each colour for contrast and may reorder them) and of the lanes of the Git graph. The title bar gradient always uses `Color7` (violet, `#955ae0`) at 35% opacity over `#26282c`.

### Known differences from IntelliJ

- The editor island uses Eva's editor background, not `#191a1c`.
- The tab bar default height is 41 px (31 px compact) like IntelliJ, but `tab_bar.height` and `tab_bar.icon_size` still override it, and Zed's other bars keep their unchanged defaults. The IntelliJ values for those can be set with existing settings: `"title_bar": { "height": 40, "icon_size": 20 }` (main toolbar header 40, button icon 20) and `"panel": { "height": 41 }` (tool window header 41).
- Tabs: the label font is the UI font size (13.1 px at the default `ui_font_size` of 15, 1 px less in compact density) where IntelliJ uses Inter 13 px; tabs that draw their own label (terminal, diagnostics, image viewer) take the new colours but not the label size; `close_position: left` mirrors the right-hand paddings, which are IntelliJ's numbers; compact mode uses a 2 px right padding derived from IntelliJ's compact insets.
- File colours: only the Tests scope is coloured, and test files are found by name and `__tests__` folders, not by test source roots. Non-project and custom scope colours are not implemented.
- Tabs: the modified dot is always shown (IntelliJ only with "Mark modified"), it is not clickable and its colour is the theme accent, not IntelliJ's `#548AF7`; with `close_button: hidden` a modified tab still shows the dot, so its width can change; the drag ghost shows only icon and label.
- The unselected label blend is computed in Flint as the square root of 0.7·text² + 0.3·editor background² per channel, over the editor background, not IntelliJ's strip background `#191a1c`.
- The right margin uses `editor.wrap_guide` (`#454963`); IntelliJ draws the right margin in the scheme's `RIGHT_MARGIN_COLOR` (`#21252b` in Eva Dark) and code-style visual guides in `#454963`.
- IntelliJ's `Island.borderWidth` is 6; the Flint gap default of 4 is left as is.
- Split dividers between editor panes use `border` (`#40434a`), not IntelliJ's `#26282c`, which is invisible on Eva's editor background.
- Filled buttons that sit on the modal layer (the Restricted Mode and disconnected-project dialogs) have the same colour as the dialog, because IntelliJ's popup colour `#26282c` equals the frame colour; only their label and their pressed state are visible.
- Hover highlights use IntelliJ's translucent white, so they are fainter than Eva's solid blue; the sticky excerpt header keeps an opaque fill while hovered.

## Popups and menus

Every popup surface is opaque: context menus, drop-downs, pickers (command palette, file finder, Search Everywhere), completion and hover cards, tooltips, modals and the Git dialogs. A translucent `elevated_surface.background`, from a theme file or from `theme_overrides` in your `settings.json`, is drawn with full opacity in the same colour. Without your own `elevated_surface.background` override, popups use IntelliJ's popup colour `#26282c`.

Popups use the metrics of IntelliJ's New UI on macOS (see [docs/webstorm-ui-research.md](docs/webstorm-ui-research.md), Exception 4):

| Element | Flint | IntelliJ key |
|---|---|---|
| Popup corners and border | 8 px radius, 1 px `border.variant` (`#33353b`) | `PopupMenu.borderCornerRadius`, `Popup.borderColor` |
| Context menu padding | 6 px above the first row and below the last | `PopupMenu.borderInsets` |
| Context menu row | 26 px; hover and keyboard selection are one 4 px-radius pill inset 7 px from the sides and 1 px from the row edges, with the text 6 px inside it | `List.rowHeight`, `PopupMenu.Selection.outerInsets`, `innerInsets`, `arc` |
| Selection colour | `ghost_element.selected` (`#2a4371`) for both hover and keyboard selection | `selection-bg-active` |
| Shortcuts | right-aligned, `text.placeholder` (`#73767c`) | `MenuItem.acceleratorForeground` |
| Icon column | 18 px plus a 4 px gap, reserved on every row as soon as one row has an icon or a check mark | `MenuItem.maxGutterIconWidth` |
| Separator | 9 px high, a 1 px `border.variant` line inset 10 px from both sides | `PopupMenuSeparator.height`, `stripeIndent`, `withToEdge` |
| List popup rows (pickers, completions) | at least 24 px; the selection pill is inset 8 px with a 4 px radius and the text 8 px inside it; 8 px above and below the list | `Popup.Selection.leftRightInset`, `innerInsets`, `Popup.Body` insets |
| Dialog buttons (Git dialogs, Conflicts dialog and Merge Revisions window, Language Services panel) | 28 px high (24 px compact), at least 72 px wide, 14 px side padding, 4 px radius, 1 px border, label at the UI font size; the default button is filled with `text.accent` (`#3871e1`) and has a white label, the others are transparent with a `border` (`#40434a`) outline and get the hover colour; disabled buttons have a `border.disabled` outline and a disabled label; 8 px apart in the Git dialogs and the Language Services panel | `Button.minimumSize`, `Button.arc` (an arc of 8 is a 4 px radius), `Button.default.startBackground`, `Button.startBorderColor`, `DarculaButtonUI.HORIZONTAL_PADDING` |
| Tooltips | `element.background` over the popup colour (`#33353b`), no border, 4 px radius, padding 8 px top, 12 px sides, 9 px bottom, text at most 250 px wide | `ToolTip.background`, `ToolTip.borderCornerRadius`, `HelpTooltip.maxWidth` |

Not implemented: the lighter shortcut colour of the selected row (`#bbbbbb`, no theme key), the larger insets of multi-line tooltips, the native macOS shadow of IntelliJ's popup windows (Flint keeps its own shadow) and IntelliJ's group header insets in pickers.

## Editor tabs, project panel and scrollbar

- Hidden tabs: when the editor tabs do not fit, a ˅ button at the right end of the tab bar lists the tabs outside the visible area, like WebStorm's "Show Hidden Tabs" drop-down. Picking one activates it and scrolls it into view. `"tab_bar": { "show_hidden_tabs_button": true }`.
- Test-file colours: test files get WebStorm's Tests file colour, a file whose name has `.spec.` or `.test.` before the last extension (`a.spec.ts`, `a.e2e.test.tsx`) or that sits under a `__tests__` folder. The selected test tab keeps the strip-coloured pill with an inner green fill inset 3 px (2 px compact); the fill is solid in the focused pane and at 40% in an unfocused one, like IntelliJ's `IslandsTabPainter`. Unselected test tabs are tinted with the colour at 40%. The colour is the theme's `tab.test_file_background`. `"tabs": { "file_colors": false }` turns it off (Settings window: Window & Layout → Tab Bar).
- Compact tabs: with `"unstable.ui_density": "compact"` the tab label is 1 px smaller, like IntelliJ's `EditorTabs.fontSizeOffset.compact`.
- Tab bar buttons: after the ˅ button, a ⋮ button ("Recent Files, Tab Actions, and More", WebStorm's editor tab entry point) is shown whenever the pane has a tab, focused or not. Its menu has Recent Files, Go to File…, Close All Tabs, Reopen Closed Tab, Unsplit and Unsplit All (only while the editor is split, Unsplit All only with more than two groups) and Configure Editor Tabs…. The New…, Split Pane and Zoom buttons are gone: splitting is in the tab menu and Zoom keeps its key binding. `"tab_bar": { "show_tab_bar_buttons": false }` hides ⋮.
- Tab menu: right-clicking a tab opens WebStorm's editor tab menu, acting on the clicked tab: Close; Close Other Tabs (disabled with one tab); Close All Tabs; Close All but Pinned (only with pinned and unpinned tabs); Close Tabs to the Left and Close Tabs to the Right (only when there are unpinned tabs on that side); Copy Path/Reference… (Absolute Path, File Name, Path from Content Root, Path from Repository Root); Split Right, Split and Move Right, Split Down, Split and Move Down (the move entries need two tabs); Unsplit and Unsplit All; Pin Tab or Unpin Tab; Keep Tab Open (preview tabs); Configure Editor Tabs…; Reopen Closed Tab; Open In (Finder, Terminal); Git (Open File Permalink, Copy File Permalink). With the editor split, "Close All Tabs" and "Close All but Pinned" end with "In Group". Zed's Close Clean, Close Multibuffers, Make Tab Read-Only and Reveal In Project Panel entries were removed; the Project panel header's Select Opened File button replaces the last one.
- Project panel selection: the hovered and selected rows are rounded pills inset from the panel edges, like WebStorm's tree selection. The selection is the theme's selection color while the panel has focus and a neutral gray otherwise; the keyboard cursor is a border on the pill. `"project_panel": { "rounded_selection": true }`; `false` restores full-width square rows.
- Project panel header buttons: like WebStorm's Project tool window, the header has Select Opened File (⌖), Expand All and Collapse All before ⋮ and —. Select Opened File reveals and selects the file of the active editor tab in the tree, expanding its folders, and moves focus to the tree; with no file behind the tab (an unsaved buffer or a file outside the project) it only focuses the panel. Opening a file does not select it in the tree by itself; `"project_panel": { "auto_reveal_entries": true }` brings that back. Expand All and Collapse All are the `project panel: expand all entries` and `project panel: collapse all entries` actions. The buttons follow `tool_window_headers.always_show_actions`; the `project panel: select opened file` action is available from the command palette.
- Editor scrollbar: the thumb is a narrower rounded pill inside the track, like the macOS scrollbar in JetBrains IDEs. Dragging still works on the whole track width. `"scrollbar": { "rounded_thumb": true }`.
- UI font: the default `ui_font_family` is `Inter`, the font JetBrains IDEs use; Inter 4.1 (OFL) is bundled in `assets/fonts/inter`. `".SystemUIFont"` gives San Francisco and `".ZedSans"` gives IBM Plex Sans.
- Project tree and editor: folders show a disclosure chevron followed by the folder icon (`"project_panel": { "folder_indicator": "both" }`). The editor line height is 22 px at the default 13 px font (`"buffer_line_height": { "custom": 1.6923077 }`), WebStorm's line height for JetBrains Mono 13: IntelliJ uses `ceil(font metrics height × 1.2)`, with 1.2 the default line spacing.
- Breadcrumbs: like WebStorm, they are shown below the editor and list only the code structure at the caret, not the file path; see [Breadcrumbs](#breadcrumbs). `toolbar.quick_actions` stays `false`, so no toolbar row is shown above the editor.
- Settings window: Window & Layout → Tab Bar, Panels → Project Panel, Editor → Scrollbar, Appearance → UI Font.

The WebStorm reference used for these changes, with quotes from JetBrains documentation and the full gap list, is in [docs/webstorm-ui-research.md](docs/webstorm-ui-research.md).

## Editor

### Context menu

Right-clicking the editor opens WebStorm's editor menu, with the Flint actions that exist: Show Context Actions; Cut, Copy, Paste; Copy / Paste Special (Copy and Trim, Copy Permalink to Line); Find in Files; Find Usages; Go To (Declaration or Usages, Implementation(s), Type Declaration); Folding (Expand, Expand Recursively, Expand All, Collapse, Collapse Recursively, Collapse All); Rename…; Open In (Finder, Terminal); Git (Annotate with Git Blame, Show History, Open Permalink to Line, Copy Permalink to Line); Compare with Clipboard. Open In needs a file on disk and Git a git repository. Zed's Go to Declaration, Format Buffer and Format Selections entries were removed from the menu; their actions and key bindings stay.

Not implemented, because Flint has no such action: Column Selection Mode, Copy Reference, Paste as Plain Text, Paste from History, Super Method, Related Symbol, Test, Expand to Level, Refactor, Generate…, run configurations, Open in Right Split, Local History, External Tools.

### Gutter

- The gutter is laid out like IntelliJ's New UI, left to right: line numbers (1 px smaller than the code, right-aligned), the bookmark and breakpoint column, the folding chevrons, the Git change markers, then a 1 px separator in `editor.indent_guide`. The code starts 3 px after the separator.
- Spacing follows `unstable.ui_density`, like IntelliJ's compact mode: in `compact` the numbers start 2 px from the editor edge with 2 px gaps around the bookmark column; otherwise the gaps are 4 px with 4 px before the numbers. At 13 px JetBrains Mono in compact density, the numbers of a 100–999-line file end 24 px from the editor edge, the separator is at 61 px and the code starts at 64 px.
- The line-number column is as wide as the largest line number, and at least 14 px. `gutter.min_line_number_digits` (default 0) reserves room for more digits.
- The bookmark and breakpoint column is only reserved when `gutter.bookmarks` or `gutter.breakpoints` is on and the editor shows a project file or a multibuffer. Buttons are centred in a 14 px slot. Right-clicking anywhere left of the folding chevrons opens the gutter menu.
- Git change markers are 4 px capsules after the folding chevrons, 3 px into an 8 px column, and they cover the separator. Deleted lines are an 8 px capsule on the boundary between two lines. A custom `gutter.git_gutter_width` sets the capsule width (5 px and wider sit 1 px into the column). With the Git gutter off, no column is reserved and the separator follows the chevrons.

### Code font, right margin and braces

- Ligatures are off (`"buffer_font_features": { "calt": false }`), like IntelliJ's default, so `=>` stays two characters.
- Right margin: a guide is always drawn at `preferred_line_length`, like IntelliJ's right margin ("Hard wrap at"). The default is 120, IntelliJ's default, and an `.editorconfig` `max_line_length` moves it, as in IntelliJ. When `soft_wrap` wraps at the preferred line length the guide is drawn in `editor.active_wrap_guide` as before. `wrap_guides` adds more guides and `show_wrap_guides: false` hides all of them.
- Rainbow brackets are off by default (`"colorize_brackets": false`); WebStorm has none.
- Matching braces are highlighted only when the caret touches a brace, or is separated from one only by spaces and tabs on the same line, like IntelliJ's `BraceHighlightingHandler`. Before, the innermost pair enclosing the caret was always highlighted.
- TypeScript and JavaScript parameter name hints are shown only for literal arguments, like WebStorm: the built-in `tsgo` configuration sends `"inlayHints": { "parameterNames": { "enabled": "literals" } }`. `"all"` in `lsp.tsgo.settings.typescript.inlayHints.parameterNames.enabled` restores the old behaviour.

### Breadcrumbs

Breadcrumbs are on by default and sit below the editor, like WebStorm: a strip with a 1 px top border in `editor.indent_guide`, the crumbs of the code structure at the caret in the UI font, separated by chevrons, the last one in the normal text colour. Clicking the strip opens the symbol outline and right-clicking it copies the file path, as before.

```json
"toolbar": { "breadcrumbs": true, "breadcrumbs_placement": "bottom", "breadcrumbs_file_path": false }
```

- `breadcrumbs_placement`: `"bottom"` (default) or `"top"`, which puts them back into the toolbar above the editor with Zed's look.
- `breadcrumbs_file_path`: `true` starts the crumbs with the file path again.
- Settings window: Editor → Toolbar.
- Not implemented: clicking a crumb to jump to that symbol, and IntelliJ's arrow-shaped hover background.

### Inspection widget

The top-right corner of the editor shows IntelliJ's inspection widget (`EditorMarkupModelImpl`, intellij-community `ace3ff1afc7b514bf4e69d9c83adc7adf736f781`) on a rounded chip in the editor background, at the top of the scrollbar column; the scrollbar starts below it.

- One group per severity, errors first, then warnings and weak warnings (language server information diagnostics): IntelliJ's 16 px icon tinted in the severity colour, followed by the count in the UI font 2 px smaller. Hints are not counted. Without problems the widget shows IntelliJ's green check.
- With problems, ˄ and ˅ go to the previous and next highlighted problem (`editor: go to previous diagnostic` and `editor: go to diagnostic`). Like IntelliJ they go to errors first: while the file has errors they skip warnings, and they never stop on hints.
- Clicking the counts runs `diagnostics: toggle file problems`: it opens the Project Diagnostics panel on its File tab for this file, and a second click for the same file hides the panel (see [Project Diagnostics panel](#project-diagnostics-panel)).
- Hovering shows a popup with "No problems found" or the counts ("2 errors, 1 warning, 1 weak warning") and the link "Highlight: All Problems". The link lists IntelliJ's levels for this editor: All Problems highlights every diagnostic up to `diagnostics_max_severity`; Syntax highlights errors only and the widget switches to IntelliJ's outline icons; None turns diagnostics off and the widget shows "OFF". The counts follow the level. `editor: toggle diagnostics` switches between None and All Problems.
- It is shown for files of the project. `"show_inspection_widget": false` hides it (Settings window: Editor → Scrollbar).
- Known differences from IntelliJ: the popup opens at the mouse instead of right-aligned under the widget and has no ⋯ menu (Configure Inspections, Compact View); there is no "Analyzing…" or translucent state while a language server works; Syntax means errors of any kind, because language servers do not mark syntax errors; the level is kept per editor and is not saved.

### Markdown preview

Markdown files open like IntelliJ's editor with preview (`TextEditorWithPreview` and the Markdown plugin, intellij-community `ace3ff1afc7b514bf4e69d9c83adc7adf736f781`): the text and the rendered Markdown share one tab.

- Layouts: Editor, Editor and Preview, Preview. Three buttons on the right of the editor tab bar, before ⋮, switch them; the commands are `editor: layout editor only`, `editor: layout editor and preview` and `editor: layout preview only`, without default keys, as in IntelliJ. Clicking Editor and Preview while it is selected switches between side by side (preview on the right) and stacked (preview below); its icon shows the current orientation.
- With the tab bar hidden (`"tab_bar": { "show": false }`) the buttons sit in a floating toolbar 25 px below the top and 20 px from the right edge of the editor. It appears when the mouse moves over the editor and hides 1.4 s after the last move, when the mouse leaves or on a key press.
- The layout chosen last is used for every Markdown file opened afterwards and is kept across restarts; the first default is Editor and Preview. Each editor starts side by side.
- Drag the 1 px divider to resize, double-click it to reset to half. The proportion is shared by all Markdown editors and kept across restarts.
- Focus: Editor and Editor and Preview focus the text, Preview focuses the preview.
- The preview follows the editor: it renders the text 200 ms after you stop typing and scrolls so that the line at the top of the editor is at the top of the preview. Scrolling the preview does not move the editor.
- Links: `#heading` scrolls the preview; a link to another Markdown file with `#heading` opens that file and scrolls its preview to the heading; project files open in an editor, at the position for `path:line:column` and `#L` fragments; other URLs open in the browser. Task list checkboxes toggle the line in the source. Images are resolved relative to the file.
- In the preview: Page Up and Page Down, Up and Down, Alt-Up and Alt-Down (by block), Cmd-Up and Cmd-Down (top and bottom) scroll. The buffer font size commands change `markdown_preview.font_size` while the preview has focus.
- The old preview tab is gone with its commands (`markdown: open preview`, `markdown: open preview to the side`, `markdown: open following preview`, `markdown: close and return to editor`) and the settings `open_markdown_files_in_preview`, `limit_content_width` and `max_width`.
- Known differences from IntelliJ: the floating toolbar does not fade in and out; the preview scroll is not animated and cannot be turned off; the layout and the orientation are not saved per file; the preview is rendered by Flint's Markdown renderer, not a browser.

### Inlay hints

Language server inlay hints are drawn like WebStorm's: a rounded pill with the hint text in the UI font (`ui_font_family`, Inter by default) at the buffer font size minus 1 px.

- Colours: the pill is the theme's `hint.background` (`#383c458c` in the default look, Eva's `#383c45` at IntelliJ's 55% hint opacity), the text the syntax `hint` colour.
- Geometry, from IntelliJ's inlay hint renderer: 6 px padding on each side of the text, a 2 px margin on each side of the hint (WebStorm draws both after the pill), the pill 1 px below the top and 1 px above the bottom of the line, a corner radius of 3.5 px at a 12 px hint font, and the text baseline 1 px above the code baseline.
- The `paddingLeft`/`paddingRight` spaces a language server asks for are not drawn; the pill's padding and margins separate the hint from the code, like in WebStorm.
- Hovering a hint shows its tooltip; Cmd-hover underlines and Cmd-click follows the label part under the pointer. The pill padding belongs to the nearest character of the hint.
- `"inlay_hints": { "show_background": false }` keeps the font and spacing and draws no pill.
- Debugger values and colour swatches keep their previous look.
- Cursor movement, column selection, IME positions and soft wrap measure hints with the same pill geometry. A hint is never split by soft wrap: it moves to the next row as a whole.
- Not implemented: WebStorm's "Use editor font in inlays" option and the shortening of long type hints. Hints in the sticky scroll header have no pill.

## Main toolbar

The title bar works like WebStorm's [main toolbar](https://www.jetbrains.com/help/webstorm/new-ui.html), left to right:

- Window buttons (close, minimize, zoom), described below.
- VCS widget: the branch name, then `↓N ↑M` when the branch is behind or ahead of its upstream (only the non-zero directions are shown), and a ˅. Click it to open the [Git Branches popup](#git-branches-popup). The worktree button is hidden by default. In a remote project, the remote host indicator comes before it.
- Search Everywhere field: a field labelled "Search Everywhere" with the shortcut of `search everywhere: toggle` from your current keymap (`Shift` `Shift` with the JetBrains keymap). Click it to open Search Everywhere on its default tab.

Project gradient: like WebStorm's colored project headers, the title bar is tinted with a project color, fading out from the left edge to the middle. The color is always IntelliJ's violet project color `#955ae0` (`RecentProject.Color7` `Avatar.Start`), whatever the project. It appears only when a project is open.

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
- The branch button of the commit window opens it as a popover too.
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

## Git tool window

The Git tool window is a copy of IntelliJ's Git (Version Control) tool window. It opens in the bottom dock: use `git_panel: toggle focus`, the Git button of the left tool window bar, or Cmd-9 in the JetBrains keymap.

### Header tabs

- After the "Git" title the header shows the content tabs: "Log" (titled like "Log: HEAD" when the log is filtered), any extra Log tabs, and "Console". The selected tab gets the Islands rounded highlight, or the classic underline when islands are off.
- "+" (Open New Git Log Tab) opens another Log tab with the selected tab's filters. Extra Log tabs and the Console tab have a close button. Alt-click on a tab closes the other tabs, middle-click or Shift-click closes it, and right-click offers Close Tab, Close All Tabs, Close Other Tabs, Select Next Tab and Select Previous Tab.
- "⌄" offers Show History for the current editor file, Show VCS Console Tab (when the console has output but is not shown) and Worktrees….
- Tabs that do not fit are dropped from the edges of the strip; Show Hidden Tabs lists them.
- Extra Log tabs and the selected tab are restored per workspace. Like IntelliJ, only one extra tab without filters is restored.
- Keys in the tool window: Cmd-W closes the selected tab (or hides the window when the main Log tab is selected), Cmd-{ / Cmd-} and Alt-Cmd-Left / Alt-Cmd-Right select the previous / next tab. The JetBrains keymap adds Ctrl-Shift-F4 and Ctrl-Left / Ctrl-Right.

### Log tab

The Log tab is IntelliJ's VCS Log (`MainFrame`, intellij-community `ace3ff1afc7b514bf4e69d9c83adc7adf736f781`) for the active repository: the Branches pane on the left, the filter toolbar and the commit graph table in the middle, and the Changed Files pane above Commit Details on the right. The dividers are 1 px and draggable; the default proportions are IntelliJ's (branches 0.3, table 0.7, changes 0.7) and are remembered.

- Branches pane: a vertical toolbar (Hide, New Branch, Update Selected, Delete, Compare with Current, Show My Branches, Fetch, Mark/Unmark As Favorite, Navigate Log to Selected Branch Head, Branches Pane Settings, Expand All, Collapse All), the "Branch or tag" search and the tree HEAD (Current Branch), Local, Remote and Tags with folders, favourites and incoming/outgoing counters. Double-click or Enter filters the log by the selected branches; the settings popup can make a single click update the filter or navigate the log. Hidden, the pane becomes a vertical "Branches" stripe. Favourites, Group by Directory and Show Tags are shared with the Git Branches popup.
- Filter toolbar: "Text or hash" with search history, Regex (`.*`) and Match Case (`Cc`); the Branch, User, Date and Paths filters (`Branch: HEAD ×` when set, × clears); the IntelliSort options (By Commit Date, Topologically, First Parent); Refresh, Cherry-Pick, View Options (Compact References View, Tag Names, Long Edges, Commit Timestamp, References on the Left, Columns, and the Current Branch, Merge Commits and My Commits highlighters) and Go To Hash/Branch/Tag.
- Commit table: the graph is a port of IntelliJ's graph code (layout, long-edge arrows, dotted edges for filtered commits, branch colours, head node), then the subject with right-aligned reference labels, Author (`*` when the committer is someone else) and Date; Hash is available under Columns. Rows are 26 px with rounded selection. Merge commits are grey, your commits bold. Click, Cmd-click and Shift-click select; arrows on collapsed edges jump to the target commit; Left/Right go to the child/parent commit. The context menu has Copy Revision Number, Cherry-Pick, Checkout Revision, Show Diff, New Branch…, New Tag…, Reset Current Branch to Here…, Revert Commit, Undo Commit, Go to Child/Parent Commit and View in browser.
- Changed Files: the changes of the selected commits as a tree grouped by directory ("Select commit to view changes" when nothing is selected); double-click or Show Diff (Cmd-D) opens the commit diff, History opens the file history, and View Options toggles Group by Directory and Show Details.
- Commit Details: the message, `<hash> <author> <email> on <date> at <time>` with "committed by" when the committer differs, branch and tag labels, and "In N branches: … Show all".
- Keys: Cmd-L focuses the text filter, Cmd-F opens Go To Hash/Branch/Tag, Cmd-R refreshes, Cmd-Alt-Shift-C copies the hashes, Cmd-D shows the diff.
- Saved state: filters, text filter options, graph options, highlighters, column widths and the Branches pane state per tab and repository; reference label options, columns and divider proportions for all tabs.
- Not implemented: diff preview in the log, Revert Selected Changes, Show Repository at Revision, Create Patch, interactive rebase, edit message, fixup, squash and drop, Push All up to Here, "No Merges", collapsing linear branches, the cherry-picked commits highlighter, and Show My Branches (IntelliJ needs its log index for it; the button stays disabled).

### Console tab

- The Console tab appears, without being selected, when the first git command is logged: `HH:mm:ss.SSS: [<working dir>] git <args>` for commands that change the repository, followed by their error output in red.
- Closing it hides it until new output arrives or Show VCS Console Tab is used.
- Its toolbar has Soft-Wrap (remembered), Scroll to End and Clear All. The console keeps following new output while it is scrolled to the end.

## Search Everywhere

Search Everywhere is a WebStorm-style popup that searches files, classes, symbols, actions and text at once. With the macOS JetBrains base keymap it opens on a double `Shift`; with any keymap it is available as `search everywhere: toggle` in the command palette. Like WebStorm, it has a row of tabs above the query and a preview of the selected item. Like every popup, its surface is opaque (see [Popups and menus](#popups-and-menus)).

| Tab | What it searches |
| --- | --- |
| All | Classes, Files, Symbols, Actions and Text, a few results each, grouped under section headers. Every source contributes whenever there is a query. A `more…` row opens the full tab for that section. An empty query lists the recently opened files. |
| Classes | Workspace symbols of kind class, interface, enum and struct. |
| Files | Files and folders by name (fuzzy, so `CamelCase` and `snake_case` abbreviations work). An empty query lists the recently opened files. `name:line` and `name:line:column` open the file at that position. Opening a folder reveals it in the project panel. |
| Symbols | All workspace symbols. |
| Actions | Every action available where the popup was opened, with its key binding. `Enter` runs the action in the editor or panel that had focus; `Cmd-Enter` opens the keymap editor to change its binding. Command aliases from `command_aliases` work here too. |
| Text | Text in the project files (case-insensitive, at most 100 matches). The last row, "Open in Text Finder", hands the query to the full Text Finder. |

Classes and Symbols come from the language servers' workspace symbol search, so they are empty for languages whose server does not provide it. The "Include non-project items" checkbox adds git-ignored files and symbols outside the project.

Layout, the same in the popup and in the Search panel, after IntelliJ's `SearchEverywhereUI`: the header row has the tabs on the left at their natural width, as Islands pills (28 px, 24 px compact; the selected tab has the `tab.active_background` `#233558` fill and the `border.selected` border), and on the right the "Include non-project items" checkbox, the "Text in <folder>" chip and the scan indicator. Below a divider comes the query row with a magnifier icon; the query text is aligned with the result names. Section headers are muted labels followed by a thin line. The footer ("Preview", "Actions…"), shared by every picker, uses small `text.placeholder` labels like IntelliJ's advertiser strip.

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

The Search tool window sits in the bottom dock, with the title "Search". It hosts the same tabs and sources as the popup, fills the dock and keeps the preview hidden until you toggle it. In the bottom dock the preview opens to the right of the results and takes half of the width, like the preview of WebStorm's Find tool window; in a left or right dock it opens below the results.

- Toggle it with `search panel: toggle focus`, `Cmd-3` (`Alt-3` on Linux) with the JetBrains keymaps, `Ctrl-Shift-S` on macOS (`Alt-Shift-S` on Linux and Windows) with the default keymaps, or View → Search Panel. Settings: `search_panel.button`, `search_panel.dock` (`left`, `right` or `bottom`, default `bottom`), `search_panel.default_width` (default 360, used in a left or right dock) and `search_panel.default_height` (default 320, used in the bottom dock), also under Panels in the settings window.
- `Cmd-Shift-F`, Find in Project, the tab "+" menu entry "Search Project" and the Project panel's "Find in Folder…" open the panel on the Text tab, seeded from the buffer search query or the editor selection. "Find in Folder…" also limits the Text tab to that folder; the chip "Text in <folder> ×" shows the limit and removes it.
- Enter opens a result and keeps the panel and its results; Escape moves focus back to the editor.
- Replace in project and the regex, whole-word, case, include and exclude options of `pane: deploy search` still open the project search tab.

### Not implemented

- Math evaluation in the query.
- `/` to list settings groups.
- The Git tab (branches and commits).
- The filter popup (for example, only recent files) and "Open in Find tool window" for tabs other than Text.

### Project Diagnostics panel

The Project Diagnostics tool window sits in the bottom dock, with the title "Project Diagnostics Panel". It hosts the project diagnostics view that used to open as an editor tab.

- Its tool window bar button uses the warning icon (tooltip "Project Diagnostics Panel"). While the project has errors, the button shows the error count as a badge; there is no badge at 0 errors.
- Toggle it with `diagnostics: deploy`: `Cmd-Shift-M` on macOS, `Ctrl-Shift-M` on Linux and Windows with the default keymaps, `Cmd-6` (`Alt-6` on Linux) with the JetBrains keymaps, or View → Diagnostics. The first press opens the panel and focuses it; a second press returns focus to the editor. With the JetBrains keymaps `Cmd-6` (`Alt-6`) also hides the dock while the panel has focus.
- Settings: `project_diagnostics_panel.button` (default `true`; "Hide Button" in the button's right-click menu sets it to `false`), `project_diagnostics_panel.dock` (`left`, `right` or `bottom`, default `bottom`), `project_diagnostics_panel.default_width` (default 360, used in a left or right dock) and `project_diagnostics_panel.default_height` (default 320, used in the bottom dock), also under Panels → Project Diagnostics Panel in the settings window. The panel's dock position is restored on restart. The old `diagnostics.button` setting no longer exists.
- Tabs, like IntelliJ's Problems tool window: File, the problems of the file in the active editor (it follows the active editor while the panel is visible), and Project, the project diagnostics view (IntelliJ's "Project Errors"; Flint's view also lists warnings). Each tab shows its problem count in a muted colour: errors, plus warnings while warnings are shown. An empty File tab says "Open file in editor to see problems" or "No problems in <file>" ("Problems could be filtered out" below it while warnings are hidden).
- `diagnostics: toggle file problems`, also a click on the [inspection widget](#inspection-widget), works like IntelliJ's: it opens and focuses the panel on the File tab for the active file; run again for the same file it hides the panel; for another file it switches the File tab to it. `diagnostics: deploy` opens the panel on the last selected tab, Project at first.
- The Project view is created when the Project tab is first shown, so no diagnostics buffers are opened before that; until then the tab shows "Checking diagnostics…".
- Header buttons, for the selected tab: Refresh Diagnostics (it becomes Stop Diagnostics Update while an update runs; shown once the view exists) and Toggle Warnings.
- Compared with the old editor tab, these are gone: the buffer search button and the include-warnings colour state of the pane toolbar, navigation history and the tab itself (title, splitting, per-tab badge). The "Show N warnings" button of the empty state remains. The Toggle Warnings header button cannot show an on or off colour. The panel has no zoom.
- A Flint dock shows one panel at a time, so opening this panel in the bottom dock replaces the visible terminal, Git, Search or Language Services panel.

### Language Services panel

The Language Services tool window sits in the bottom dock, with the title "Language Services Panel".

- Its tool window bar button uses the bolt icon (tooltip "Language Services Panel"). While the panel is not shown, the button shows the number of language servers in an error or warning state as a badge; there is no badge when there are none.
- Toggle it with `lsp_tool: toggle focus`: `Ctrl-Cmd-L` on macOS, `Ctrl-Alt-L` on Linux, `Shift-Alt-L` on Windows, or View → Language Services. The JetBrains keymaps add no key for it and no key that hides the dock while it has focus.
- The panel lists the language servers grouped by worktree and follows the active editor. A restricted-mode banner comes first and opens the worktree security dialog on click. Each server row shows a status dot (green running, yellow starting, red error, yellow warning, grey stopped), the name, the status, the version and the memory use, and the server message if any. Row buttons are outlined dialog buttons (see [Popups and menus](#popups-and-menus)) with an icon and a label: Message (only with a message), Logs (only when logs exist), Restart and Stop (only when the server can be stopped); their tooltips name the full action. Restart All Servers and Stop All Servers sit side by side in one row at the end of the list. An empty list shows "No language servers".
- Settings: `language_services_panel.button` (default `true`; "Hide Button" in the button's right-click menu sets it to `false`), `language_services_panel.dock` (`left`, `right` or `bottom`, default `bottom`), `language_services_panel.default_width` (default 360, used in a left or right dock) and `language_services_panel.default_height` (default 320, used in the bottom dock), also under Panels → Language Services Panel in the settings window. The old `global_lsp_settings.button` setting no longer exists.
- The per-server actions are row buttons; there are no hover submenus. The panel has no header buttons.
- A Flint dock shows one panel at a time, so opening this panel in the bottom dock replaces the visible terminal, Git, Search or Project Diagnostics panel.

## Merge conflicts

Flint resolves merge conflicts like WebStorm 2026.2.3's [Resolve conflicts](https://www.jetbrains.com/help/webstorm/resolve-conflicts.html) with the iterative flow: a "Conflicts" dialog lists every conflicted file, and each file opens in a three-pane "Merge Revisions" window. Both are separate native windows with their own saved position and size, not editor tabs. They exist only for local repositories and are hidden in remote projects.

### Opening the Conflicts dialog

- `merge tool: resolve conflicts` in the command palette (`merge_tool::ResolveConflicts`, WebStorm's Git → Resolve Conflicts…). It works for conflicts made on the command line too.
- Automatically after an operation that leaves conflicts: merge, update, rebase, stash pop and the restore step of a smart checkout. The dialog reads the repository state, so during a rebase the sides are swapped as in WebStorm and "Yours" stays your own commits.
- What happens when the dialog closes depends on the operation:

| Operation | All files resolved | Conflicts remain |
|---|---|---|
| Merge | Accept and Finish creates the merge commit, only while a merge is in progress | Warning "<branch> Merged with Conflicts" with Resolve… |
| Update by merge, `git: pull` | The merge commit is created | Warning "Cannot complete update" with Resolve… |
| Update by rebase, `git: pull rebase`, unfinished rebase | `rebase --continue` runs by itself, skips an empty commit, and reopens the dialog at the next conflicting commit | Warning "Cannot continue rebase" with Resolve… |
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

### Editor notification

- A text editor showing a file that is in a git conflict state gets a banner above its content, like WebStorm's editor notification: a warning banner "File has unresolved merge conflicts" with the link "Resolve conflicts…", which opens the standalone Merge Revisions window for that file (not the Conflicts dialog).
- That window is standalone, not part of the iterative flow: its bottom buttons are Accept Left, Accept Right, Cancel and Apply Changes. Cancel leaves the file untouched (after asking when the Result was edited); the other three write the file and stage it.
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
- The Merge Revisions window opens after the revisions are loaded (WebStorm shows it empty first with a loading overlay). The minimum size is 700×450 where WebStorm derives it from the content, the buttons have no focus ring, and in light themes the button colours come from the theme (`text.accent`, `border`), not WebStorm's light values. Cmd-Enter does nothing while Apply Changes is disabled (WebStorm runs it anyway).
- Confirmation dialogs of both windows are opaque cards inside the window with the title as a bold first line, not separate dialog windows with a title bar.
- Conflicts dialog: no Dock bounce when it opens, no cancellable "Loading unmerged files…" progress, no block on project reloads while it is open, a context menu is closed by focus loss rather than by a resize, no vertical separators between header cells, one selection colour whether the window is focused or not, speed search is a case-insensitive substring match without highlighting (WebStorm uses word-prefix and wildcard matching), the inline Accept button is Flint's subtle button instead of `ActionToolbar.smallVariant`, Tab and Shift-Tab move between rows, not between cells, and the Resolve button spinner is a rotating icon rather than the 8-step `AnimatedIcon`.
- The fallback description texts of `GitStashChangesSaver`, and the raw (unbolded) rebase description that WebStorm shows when `REBASE_HEAD` is missing, are not reproduced; Flint always passes its own restore description and renders the bold.
- Modality: the dialog and the windows block only the workspace window that opened them, while WebStorm's `IdeModalityType.IDE` blocks all IDE windows. The native close button and Cmd-Q of the workspace window still work while a dialog is open. The zoomed state of a window is not saved, and bounds are written when the window closes, not at quit.
- Operations around the dialog that Flint does not have, so their WebStorm flows are not ported: cherry-pick and revert conflicts (`GitApplyChangesConflictResolver`) and their Abort Cherry-Pick / Abort Revert actions, the "Abort and Rollback" variants of Abort Rebase, the "Rebase stopped for editing" notification as an actionable notice (inside the continue loop it is an information notice without Continue or Abort), "Continue rebase failed" with "Stage and Retry" / "Show Files" after `rebase --continue` finds unstaged changes (Flint reports the remaining conflicts instead), the "View Files" dialog of the "Untracked Files Prevent …" notifications (the files are listed in the notification), the "Cannot commit changes due to unresolved conflicts" commit check (the commit window disables Commit instead), the unstash index-conflict notice, a Git → Resolve Conflicts… menu entry (the action is in the command palette only). Flint also refuses to start a merge while a merge or rebase is unfinished, before running git.
- Smart operations keep Flint's own stash mechanism (WebStorm's `GitPreservingProcess`): the restore step opens the Conflicts dialog with WebStorm's texts, but the stash is restored by `git stash pop` without `--index`. A merge or update that leaves conflicts while changes are stashed shows the WebStorm warning "Local changes were not restored" ("Before merge your uncommitted changes were saved to stash.") with "View saved changes…"; WebStorm restores the stash after the dialog in every case, Flint only once everything is resolved.
- The Git Conflicts tool window, the Changes view banner, the hover icons (Merge, Rollback) of changed files and the non-modal merge in the Changes view: they are off by default in WebStorm (registry keys `git.merge.conflicts.toolwindow` and `vcs.non.modal.merge.enabled`), so they are not built. Revert Resolved Files is not built either; use the Conflicts dialog.

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

The inspection widget and editor-with-preview icons `assets/icons/{inspections_ok,inspections_ok_empty,inspections_error,inspections_error_empty,inspections_warning,find_and_show_prev_matches,find_and_show_next_matches,layout_editor_only,layout_editor_preview,editor_preview_vertical,layout_preview_only}.svg` are unmodified copies of [JetBrains/intellij-community](https://github.com/JetBrains/intellij-community) icons (commit `ace3ff1afc7b514bf4e69d9c83adc7adf736f781`), licensed under Apache-2.0, each keeping its original JetBrains copyright header. Sources, in the order above: `platform/icons/src/expui/general/inspections/{inspectionsOK,inspectionsOKEmpty,inspectionsError,inspectionsErrorEmpty,inspectionsWarning}.svg` and `platform/icons/src/expui/general/{chevronUp,chevronDown,editorOnly,editorPreview,editorPreviewVertical,previewOnly}.svg`.

The Git tool window icons `assets/icons/{arrow_collapse,arrow_expand,chevron_down_large,cherry_pick,close_small,close_small_hovered,general_add,general_delete,general_history,general_open_new_tab,general_refresh,general_scroll_down,general_search,general_soft_wrap,inline_match_case,inline_regex,intelli_sort,nodes_folder,nodes_star,vcs_changes,vcs_update}.svg` are unmodified copies of [JetBrains/intellij-community](https://github.com/JetBrains/intellij-community) icons (commit `ace3ff1afc7b514bf4e69d9c83adc7adf736f781`), licensed under Apache-2.0, each keeping its original JetBrains copyright header. Sources, in the order above: `platform/icons/src/expui/general/{chevronLeft,chevronRight,chevronDownLarge}.svg`, `platform/dvcs-impl/shared/resources/icons/new/cherryPick.svg`, `platform/icons/src/expui/general/{closeSmall,closeSmallHovered,add,delete,history,openNewTab,refresh,scrollDown,search,softWrap}.svg`, `platform/icons/src/expui/inline/{matchCase,regex}.svg`, `platform/vcs-log/impl/resources/icons/new/intelliSort.svg`, `platform/icons/src/expui/nodes/{folder,star}.svg` and `platform/icons/src/expui/vcs/{changes,update}.svg`.

The commit graph of the Git tool window (`crates/git_ui/src/git_log/graph.rs` and `crates/git_ui/src/git_log/graph/`) is a Rust port of the IntelliJ Platform VCS Log graph code from [JetBrains/intellij-community](https://github.com/JetBrains/intellij-community) (commit `ace3ff1afc7b514bf4e69d9c83adc7adf736f781`, `platform/vcs-log/graph`, `platform/vcs-log/graph-api` and `platform/vcs-log/impl/src/com/intellij/vcs/log/{graph,paint}`), licensed under Apache-2.0.
