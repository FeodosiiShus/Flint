> [!IMPORTANT]
> Remove this line to confirm you've reviewed this PR before submitting.

# Flint

Flint is a personal fork of [Zed](https://github.com/zed-industries/zed). It is built only for macOS on Apple Silicon (`aarch64-apple-darwin`), only in GitHub Actions, by the [`build-macos.yml`](.github/workflows/build-macos.yml) workflow.

## Getting a build

- Every push runs the [`checks.yml`](.github/workflows/checks.yml) workflow: `cargo fmt --check`, `cargo check` of the app, and the tests of the crates Flint changes. It does not build the app.
- The app is built on every push or merge to `main`, and on demand: Actions → Build macOS → Run workflow → branch `main`. The DMG is attached to the run as the `flint-macos-aarch64` artifact.
- Running Build macOS from a `v*` tag also publishes a GitHub Release with the DMG attached.
- To run the tests locally, use `script/local-ci-tests` (limits and prerequisites are documented in [AGENTS.md](AGENTS.md) under "Build guidelines"); release builds still run only in GitHub Actions.
- For local non-build checks, use `bash script/check-static`; the same checks run automatically through `.git/hooks/pre-commit`. Clippy-based checks remain build-dependent: `bash script/clippy-advisory` and `bash script/clippy`.

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

Flint ships none of Zed's built-in AI: no agent panel, threads sidebar, inline assist, language model providers, MCP servers, external agents or commit message generation. It also drops the debugger panel, notebooks (REPL), dev containers, remote development (the headless remote server, SSH connections, File → Open Remote…, `ssh://` links, the remote host indicator in the title bar and remote projects in Open Recent), vim mode, the image viewer, the theme, tab, toolchain and settings profile pickers, call hierarchy, the Outline panel (the symbol outline popup of `outline: toggle` stays), the editor minimap, the journal, the SVG preview, the snippets picker, the syntax tree, highlights tree and key context views, the language servers for Tailwind CSS, CSS, Bash, Go, Python, C and C++, Vue, the task runner (`tasks.json` and `debug.json`, the Run menu, gutter run buttons, `task: spawn`), the status bar, and every non-macOS platform backend. Use the terminal panel to run AI command-line tools. Remote projects saved by older builds are dropped from the workspace database and from Open Recent on the first start; `ssh_connections` and `read_ssh_config` in an existing `settings.json` are ignored.

## Background image

Flint can draw a picture behind the editor and the tool windows, like WebStorm's [Background Image](https://www.jetbrains.com/help/webstorm/setting-background-image.html). There are two independent images:

- `editor_and_tools` — behind the editor, tabs, tool windows (project, git, terminal panels) and the title bar.
- `empty_frame` — behind an editor pane that has no open files. Without it, an empty pane shows the plain editor background, as in WebStorm. The welcome page that fills the empty pane of a window without a folder is the exception: without an `empty_frame` image it shows the `editor_and_tools` image.

### Choosing an image

- Command palette: `workspace: select background image`, `workspace: select empty frame background image`, `workspace: clear background image`. The pickers write the path into your user `settings.json`; clearing empties both paths in the user `settings.json` only.
- Settings window: Appearance → Background Image. Every field can be set for the user or for one project: pick "User" or the project in the scope drop-down at the top right of the Settings window. The "Choose Image…" buttons there run the same commands as the command palette. "Clear" runs `workspace: clear background image`; with a project picked in the scope drop-down it also removes both paths from that project's `.zed/settings.json`, so the image goes away like WebStorm's Clear, which removes the project value and the application value.
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
| `path` | Absolute path or a path starting with `~/`. An empty string turns the image off. PNG, JPEG, GIF and WebP (first frame), BMP, TIFF, ICO, TGA, QOI, PNM, HDR, EXR, DDS, Farbfeld and SVG are supported. An SVG is drawn at the size it declares, like WebStorm. | `""` |
| `opacity` | `0`–`100`, where `100` is fully opaque and `0` hides the image. | `15` |
| `fill` | `plain` — one copy at its original size; `scale` — scaled with its proportions kept so it covers the whole window (the edges that do not fit are cropped on the side away from the anchor); `tile` — copies at the original size repeated from the anchor (a centred anchor centres one copy). One image pixel is one logical pixel (2×2 screen pixels on a Retina display), as in WebStorm. | `"scale"` |
| `anchor` | Where the image is aligned: `top_left`, `top_center`, `top_right`, `center_left`, `center`, `center_right`, `bottom_left`, `bottom_center`, `bottom_right`. A `plain` copy sits there, a `tile` grid is aligned to it, and `scale` keeps that part of the image. | `"center"` |
| `flip_horizontal`, `flip_vertical` | Mirror the image. | `false` |

### This project only

Put the same `background_image` object into `<project>/.zed/settings.json`. Project values override the user values key by key, so a project can change only the `path`, or set `"path": ""` to turn the image off for that project. The settings of the project's first folder are used.

### How it behaves

- The `editor_and_tools` image is positioned relative to the whole window, so the editor, panels, terminal and title bar show parts of one continuous picture. It is drawn under text, selections and highlights, so opaque selection and highlight colours hide it; WebStorm also draws the image over those fills at 65% of the opacity. The `empty_frame` image is also positioned relative to the whole window; an empty pane shows its part of it, as in WebStorm.
- Every opaque surface repaints the image right after its own background, so it stays visible in opaque themes such as One Dark. In the default look the frame (title bar, tool window bars and gaps), the docks, the editor and the toolbar are all opaque, so each of them repaints the image over its own fill and `opacity` decides how much of the picture shows. Only `terminal.background` is transparent, so the terminal draws the picture once, through its container. With an editor background that is not opaque, the `editor_and_tools` image also shows through an empty pane (mixed with the `empty_frame` image when both are set).
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

- `title_bar.height` — the exact height of the title bar, but never lower than its buttons, its project tabs or its VCS widget (30 px, 26 px in compact density). With only `title_bar.icon_size` set, the bar grows when needed to keep at least 4px above and below the buttons. On macOS the window buttons (traffic lights) stay vertically centered in the title bar at any height.
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
- The left dock, the right dock, the bottom dock and the editor area are separate rounded islands. Docks use the theme's `panel.background`, the editor area uses `editor.background`. Dock resize handles sit in the gaps. In the default look the frame `#26282c` is lighter than the docks and the editor `#191a1c`, IntelliJ's `layer-1-bg` around `layer-0-bg`, so the islands stand out from the gaps; the terminal body shows the editor colour.
- Editor tabs: the tab bar has the editor background and a 1 px frame-coloured line at the bottom (IntelliJ's `EditorTabs.underTabsBorderColor`); the strip is 42 px high including that line (32 px compact). The selected tab is a 28 px pill (24 px compact) with a 6 px radius (4 px compact) and a 1 px inside border, with no underline and no dividers between tabs. In a focused pane the pill has the `#233558` fill and the `#2E4D89` border (the theme's `tab.active_background` and `border.selected`); in an unfocused pane it has the frame fill (the theme's `background`) and the `border` colour. Tabs get the hover fill (`ghost_element_hover`), except the selected tab of a focused pane. Unselected labels and icons are dimmed (labels blended toward the strip background, icons at 75% opacity); selected and hovered tabs use the full text colour. Every tab reserves a 16 px slot at its right end for the close or pin button, so tabs do not change width on hover: the × shows on the selected tab and on hovered tabs, a modified tab shows a dot in the same slot (the × replaces it on hover), and a pinned tab shows the pin. Titles are no longer cut at 24 characters.
- Tool window bars run down the left and right window edges. The left bar shows the left dock panels (by default Project), a separator, a "More Tool Windows" button (a menu of every panel, including panels whose button is hidden), and the bottom dock panels in its lower group. Like the left-bottom stripe of IntelliJ's new UI, the lower group is drawn from the bottom up: the panel with the lowest activation priority is the bottom-most button, so from top to bottom the default order is Language Services, Search, Terminal, Project Diagnostics and Git. The right bar shows the right dock panels. Click an icon to show or hide its panel; right-click it to move the panel to another dock, hide its button or toggle Show Tool Window Names (`tool_window_bars.show_names`). Right-clicking the empty part of a bar offers only Show Tool Window Names, like IntelliJ. The button tooltips, the names under the icons and the More Tool Windows menu use the tool window title, the panel name without " Panel" (Project, Terminal, Project Diagnostics), the same as the header. The tooltips appear without a delay, like IntelliJ's stripe tooltips; the More button's tooltip reads "More tool windows". The buttons are IntelliJ's stripe buttons: a 37×40 box at the default 20 px icon (32×32 in compact density) holding a 30×30 fill pill (24×24), inset 5 px from the window edge and 2 px from the side facing the editor (5 and 3 px compact), with a 6 px radius (4 px compact). Consecutive boxes touch, like IntelliJ's zero-gap stripe flow, and the separator between the top group and the More button is a 24 × 1 line in the stripe's `toolbar-border` colour (`#FFFFFF21`; `#00000020` in light themes). The pill marks an open panel (`ghost_element_active`, `#FFFFFF29`, IntelliJ's pressed stripe background); a focused panel fills it with the accent colour and a white icon (`#3871e1`, IntelliJ's selected stripe background, `ToolWindow.Button.selectedForeground`); a closed button shows the pill only under the pointer (`ghost_element_hover`, `#FFFFFF17`, IntelliJ's stripe hover). Like IntelliJ, an open button does not react to hover.
- The right bar disappears when no right dock panel has a button, so it never leaves an empty strip. The editor island then sits IntelliJ's 4 px `Islands.emptyGap` plus half the islands gap from the window edge, like the bottom edge.
- Tool window headers: every open dock island starts with a header like a WebStorm tool window header: the panel name in the bold UI font (the New UI header label), then the panel's tabs if it has any, then its own buttons and — (hide the panel; its tooltip shows the `workspace: close active dock` binding, ⇧⎋ in the JetBrains keymap, like IntelliJ's Hide Active Window). There is no ⋮ button: the panel menu (move to another dock, hide the button) opens on a right click of the header or of the tool window button. A 1 px frame-coloured line (`ToolWindow.Header.borderColor`) runs under the header of a tool window that shows tabs in the header (IntelliJ's tabbed header); other tool windows, docked anywhere, have none (IntelliJ also draws it under every bottom tool window, where it reads as a stray line above the content). The Git tool window shows its log tabs there, the Project Diagnostics panel its File and Project tabs, and the terminal its terminal tabs with their +, split and zoom buttons, like IntelliJ's terminal (`Pane::set_tab_bar_in_tool_window_header`). With the terminal split, the header shows the tabs of the focused split; a zoomed terminal has no header and shows its tabs above itself, as before.
- Glass tool windows: where macOS draws Liquid Glass natively (macOS 26), every tool window island except the Project panel is glass of the same material as popups (`ui::glass_material`, the one source for both): the theme's elevated surface colour tints the system's glass (the renderer's emulation, used without native glass, lays it over the blur instead) over a blur of what lies behind it — the frame colour and the Editor and Tools background image that the window paints behind the whole layout. The glass island paints neither a fill nor an image of its own, which would sit under the glass as flat colour and hide the material. An island can still read weaker than the branches popup over the editor: the same material darkens both by the same amount, but behind the island lies the smooth frame and image, behind the popup high-contrast code. Content on glass leaves its empty areas unfilled, so the glass shows through: GPUI tells an element it draws on glass while it lies inside an element with a backdrop blur (`Window::on_glass_surface`), and `ui::content_background` drops the fill there. The terminal, the Project Diagnostics editors and their empty states, the Git log (branches, commit table, details, changes, filter field), the Search preview and tab bars do so, while selection, hover, tab pills and the commit graph keep their colours; popups with a code preview (File Finder, Outline, Symbols) show their preview on the glass the same way. The editor and the Project panel stay opaque and show the background image unblurred. A panel stays opaque by returning `false` from `Panel::glass_background`; with Reduce Transparency, or on macOS without native glass, every island and its content are opaque, because emulating the blur of a whole tool window would cost every frame. Like WebStorm, the buttons appear only while the pointer is over the tool window or the tool window has focus. A panel adds its own buttons by implementing `Panel::header_actions`; the Project panel and the Project Diagnostics panel are the only ones that do. A panel can also show its own content, such as tabs, right after the name by implementing `Panel::header_content`; the name then keeps IntelliJ's tool window header label insets, 12 px before it and 16 px between it and the content.
- Inactive window: when the Flint window is not focused, the title bar widgets and the tool window bar buttons are drawn at 56% opacity, matching the Islands `Island.inactiveAlpha` of 0.56. Backgrounds and the background image are not dimmed. IntelliJ applies that alpha through `inactiveFrameGraphics` on the main toolbar, tool window stripe, status bar and macOS window tabs (`MainToolbar.kt`, `ToolWindowToolbar.kt`, `IdeStatusBarImpl.kt`, `MacWinTabsHandlerV2.java`); editor tabs and tool window headers stay undimmed, the same as in Flint.

```json
"islands": { "enabled": true, "gap": null, "corner_radius": null, "dim_inactive_window": true },
"tool_window_bars": { "show": true, "icon_size": null, "show_names": false, "icon_style": "jetbrains" },
"tool_window_headers": { "show": true, "always_show_actions": false }
```

- `islands.gap` — distance between islands in pixels, 0–16. Unset: 6, or 4 with `"unstable.ui_density": "compact"`, IntelliJ's `Island.borderWidth`: every island keeps a rim of half the gap, 3 px (2 px compact), so island edges sit on whole pixels.
- `islands.corner_radius` — island corner radius in pixels, 0–24. Unset: 10, or 8 in compact density.
- `islands.dim_inactive_window` — dim the frame content of an inactive window. Works only with islands enabled.
- `tool_window_bars.show` — `false` hides both bars. No panel buttons are shown anywhere then; panels open through the View menu, the command palette and key bindings.
- `tool_window_bars.icon_size` — bar icon size in pixels, 12–32; the bar is its buttons plus IntelliJ's 1 px stripe border, 18 px wider than the icon (17 px in compact density); the buttons are IntelliJ's `StripeToolbar.Button.size` "37,40" and "32,32". Unset: 20, or 16 in compact density. These are the IntelliJ Platform sizes: 20×20 and, in Compact Mode, 16×16.
- `tool_window_bars.icon_style` — `"jetbrains"` (default) draws the IntelliJ Platform tool window icons, one per panel: Project, Git (the IntelliJ "Version Control" icon), Terminal, Search (the Find tool window's magnifier), Project Diagnostics (the Problems window's icon) and Language Services (the Services icon), and the "…" of More Tool Windows. `"zed"` returns to Zed's own icons. The artwork has two drawings, a 20×20 one and a 16×16 one with a thinner stroke, like IntelliJ; sizes below 18px use the 16×16 drawing and the others the 20×20 drawing, scaled to `icon_size`.
- `tool_window_bars.show_names` — show the panel name under each icon. Like IntelliJ, a name is split at its first space into at most two lines (Project / Diagnostics), and a two-line button is 13 px taller.
- `tool_window_headers.show` — show the header row. `false` returns to panels without a common header.
- `tool_window_headers.always_show_actions` — show the header buttons (— and a panel's own buttons) all the time, like WebStorm's "Always show tool window header icons".
- `"islands": { "enabled": false }` together with `"tool_window_bars": { "show": false }` and `"tool_window_headers": { "show": false }` restores the classic Zed layout: flat docks with borders and square tabs, without panel buttons.
- Settings window: Window & Layout → Islands, Tool Window Bars and Tool Window Headers.
- WebStorm-like defaults that come with it: `tabs.file_icons: true`, `tab_bar.show_nav_history_buttons: false`, and the Git, Search, Project Diagnostics and Language Services panels docked at the bottom next to the terminal, like WebStorm's bottom tool windows, each 320 px high by default.
- Not implemented: WebStorm's split tool windows under the bar separator and the bottom-right bar group, because a Flint dock shows one panel at a time. Opening the Git, Search, Project Diagnostics or Language Services panel replaces the terminal or the other bottom panel while it is open. `workspace: toggle bottom dock` shows the last shown bottom panel, or the bottom-most one, Git, when none was shown yet.

## Default look: Islands Dark chrome on the Eva theme

- The default look is the chrome of JetBrains' Islands Dark theme (`ManyIslandsDark.theme.json`, intellij-community commit `0ad391f70a62eabbec6db656f473e1284e1110a9`) on top of the Eva Dark theme, like IntelliJ's Islands Dark UI theme with the Eva editor colour scheme. The theme is still named "Eva Dark".
- Chrome copied from Islands Dark: the frame, tool window islands, title bar, tool window bars, popups and menus, hover, pressed and selected states, borders, text and icon colours, the scrollbar thumb and the drop target.
- Stays Eva: syntax, terminal palette, players, git and diagnostic status colours, search highlight and the other editor and gutter colours. `assets/themes/eva/eva.json` is unchanged.
- Editor lines taken from IntelliJ's Islands Dark editor scheme (`IslandSchemeDark.xml`): indent guides, the gutter separator and the breadcrumbs border `#323438` (`INDENT_GUIDE`; the active guide has the same colour, because IntelliJ highlights a guide only while the caret sits on its column), the right margin `#323438` (`RIGHT_MARGIN_COLOR`, also while soft wrap wraps at it), and split editor dividers in the frame colour `#26282c` (`OnePixelDivider.background`).
- Editor colours taken from the JetBrains Eva Dark editor scheme (`Eva_Dark.xml` of the Eva-Theme plugin, the scheme WebStorm uses with Eva): inlay hint text `#747ba4` (`syntax.hint`) on `#383c45` at IntelliJ's 55% hint opacity (`hint.background` `#383c458c`, `INLINE_PARAMETER_HINT`), and matched braces `#7b524b` (`editor.document_highlight.bracket_background`, `MATCHED_BRACE_ATTRIBUTES`). The test-file tab colour `tab.test_file_background` is Islands Dark's `FileColor.Green` `#1c261c`.
- Where it lives: the `theme_overrides."Eva Dark"` block of the bundled default settings. A `theme_overrides."Eva Dark"` in your own `settings.json` merges key by key over it, so override the key to change any colour. The exception is the `accents` list: a `theme_overrides."Eva Dark"` block in your `settings.json` replaces it with an empty list unless you repeat `accents` in that block.
- `background.appearance` is now `opaque`, because Islands Dark is opaque.
- To restore the previous Eva look, override the keys in `settings.json`.

| Surface | Colour | IntelliJ key |
|---|---|---|
| Frame: title bar, tool window bars, gaps | `#26282c` | `main-window-bg` |
| Tool window islands | `#191a1c` | `tool-window-bg` |
| Editor island and tab bar | `#191a1c`, caret row `#1f2024` | `editor-bg`, `CARET_ROW_COLOR` |
| Indent guides, gutter separator, breadcrumbs border, right margin | `#323438` | `INDENT_GUIDE`, `RIGHT_MARGIN_COLOR` |
| Split editor dividers | `#26282c` | `OnePixelDivider.background` |
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

- The tab bar default height is 41 px (31 px compact) like IntelliJ, but `tab_bar.height` and `tab_bar.icon_size` still override it, and Zed's other bars keep their unchanged defaults. The IntelliJ values for those can be set with existing settings: `"title_bar": { "height": 40, "icon_size": 20 }` (main toolbar header 40, button icon 20) and `"panel": { "height": 41 }` (tool window header 41).
- Tabs: the label font is Inter 13 px like IntelliJ's (12 px in compact density); tabs that draw their own label (terminal, diagnostics, image viewer) take the new colours but not the label size; `close_position: left` mirrors the right-hand paddings, which are IntelliJ's numbers; compact mode uses a 2 px right padding derived from IntelliJ's compact insets.
- File colours: only the Tests scope is coloured, and test files are found by name and `__tests__` folders, not by test source roots. Non-project and custom scope colours are not implemented.
- Tabs: the modified dot is always shown (IntelliJ only with "Mark modified"), it is not clickable and its colour is the theme accent, not IntelliJ's `#548AF7`; with `close_button: hidden` a modified tab still shows the dot, so its width can change; the drag ghost shows only icon and label.
- The unselected label blend is computed in Flint as the square root of 0.7·text² + 0.3·editor background² per channel, over the editor background (`#191a1c` in the default look, the same as IntelliJ's strip background).
- Extra `wrap_guides` are drawn in the right margin colour; IntelliJ draws code-style visual guides in `VISUAL_INDENT_GUIDE` (`#2b2d30`).
- Filled buttons that sit on the modal layer (the Restricted Mode dialog) have the same colour as the dialog, because IntelliJ's popup colour `#26282c` equals the frame colour; only their label and their pressed state are visible.
- Hover highlights use IntelliJ's translucent white, so they are fainter than Eva's solid blue; the sticky excerpt header keeps an opaque fill while hovered.

## Popups and menus

Popups, dialogs and notification balloons are Liquid Glass: context menus, drop-downs, pickers (command palette, file finder, Search Everywhere), completion and hover cards, modals, the Git dialogs and balloons. On macOS 26 the glass is AppKit's own `NSGlassEffectView`, so it refracts and tints like the system's menus and follows Reduce Transparency: Flint draws everything below a surface into the window's layer, places the glass view over it, and draws the surface's content into a transparent layer above the glass. The theme colour reaches the glass as the view's `tintColor` (`GLASS_MATERIAL.native_surface_opacity`, 28%), so the surface paints no fill of its own: a fill above the material would read as a flat panel; a menu opened from another surface gets its own level of glass, up to six levels at once (more fall back to the emulation for that frame). Glass views, once created, are moved or hidden but never rebuilt while the window lives, and every frame with glass reaches the screen in the same Core Animation transaction as the glass views' changes, so opening, closing or switching a submenu doesn't flash. The theme's `elevated_surface.background` tints the glass at 50%, so the theme's text stays readable whatever the system's own light or dark appearance. Before macOS 26, Flint draws the glass itself: a 20 px blur under the surface colour at 82%. `FLINT_EMULATED_GLASS=1` keeps that emulation on macOS 26 too, to compare the two. Tooltips stay opaque, like macOS tooltips, and with Reduce Transparency every surface is opaque. Without your own `elevated_surface.background` override, popups use IntelliJ's popup colour `#26282c`.

Popups use the metrics of IntelliJ's New UI on macOS:

| Element | Flint | IntelliJ key |
|---|---|---|
| Surface corners and border | 12 px radius for popups and balloons, 16 px for dialogs (macOS 26 window corners), 6 px for tooltips; 1 px `border.variant` (`#33353b`) | `PopupMenu.borderCornerRadius`, `Popup.borderColor` |
| Context menu padding | 6 px above the first row and below the last, so with the 1 px row inset the first pill starts 7 px below the border | `PopupMenu.borderInsets` |
| Context menu row | 26 px: a 24 px pill with 1 px above and below, so the pills of adjacent rows never touch; hover and keyboard selection are one 4 px-radius pill inset 7 px from the sides, with the text 6 px inside it | `List.rowHeight`, `PopupMenu.Selection.outerInsets`, `innerInsets`, `arc` |
| Selection colour | `ghost_element.selected` (`#2a4371`) for both hover and keyboard selection | `selection-bg-active` |
| Shortcuts | right-aligned, `text.placeholder` (`#73767c`) | `MenuItem.acceleratorForeground` |
| Icon column | 18 px plus a 4 px gap, reserved on every row as soon as one row has an icon or a check mark | `MenuItem.maxGutterIconWidth` |
| Separator | 9 px high, a 1 px `border.variant` line inset 10 px from both sides | `PopupMenuSeparator.height`, `stripeIndent`, `withToEdge` |
| List popup rows (pickers, completions) | at least 24 px; the selection pill is inset 8 px with a 4 px radius and the text 8 px inside it; 8 px above and below the list | `Popup.Selection.leftRightInset`, `innerInsets`, `Popup.Body` insets |
| Dialog buttons (Git dialogs, Conflicts dialog and Merge Revisions window, Language Services panel) | 28 px high (24 px compact), at least 72 px wide, 14 px side padding, 4 px radius, 1 px border, label at the UI font size; the default button is filled with `text.accent` (`#3871e1`) and has a white label, the others are transparent with a `border` (`#40434a`) outline and get the hover colour; disabled buttons have a `border.disabled` outline and a disabled label; 8 px apart in the Git dialogs and the Language Services panel | `Button.minimumSize`, `Button.arc` (an arc of 8 is a 4 px radius), `Button.default.startBackground`, `Button.startBorderColor`, `DarculaButtonUI.HORIZONTAL_PADDING` |
| Tooltips | `ToolTip.background` (`#33353b`), no border, 6 px radius, padding 8 px top, 12 px sides, 9 px bottom, text at most 250 px wide | `ToolTip.background`, `ToolTip.borderCornerRadius` (Islands Dark 6), `HelpTooltip.maxWidth` |

Not implemented: the lighter shortcut colour of the selected row (`#bbbbbb`, no theme key), the larger insets of multi-line tooltips, the native macOS shadow of IntelliJ's popup windows (Flint keeps its own shadow) and IntelliJ's group header insets in pickers.

## Control corners and checkboxes

Controls use the corner radii of IntelliJ's New UI everywhere in Flint, in pixels, so they no longer shrink with `ui_font_size` like Zed's rem-based radii. IntelliJ's `arc` values are diameters, so the radius is half the arc:

| Control | Radius | IntelliJ key |
|---|---|---|
| Buttons, icon buttons, split and toggle buttons, dialog buttons, chips, key caps | 4 px | `Button.arc` 8 |
| Text fields and drop-downs | 4 px (3 px compact) | `Component.arc` 8 / `.compact` 6 |
| List, tree and picker rows (hover and selection) | 4 px | `Popup.Selection.arc`, `Tree.Selection.arc` 8 |
| Checkboxes | 3 px | New UI `checkBox.svg` (`rx` 2.5 plus the 1 px stroke) |
| Title bar widgets | 6 px in every density | `MainToolbar.Dropdown.arc` 12 (there is no `.compact` key) |
| Banners and modal sections | 6 px (4 px compact) | `Banner.arc` 12 / `.compact` 8 |

Checkboxes look like IntelliJ's New UI checkbox: a 16 px box with a 1 px border; unchecked it has the `editor.background` fill and the `border` outline, checked or indeterminate it is filled with `text.accent` (`#3871e1`) and shows a white check or dash. The label starts 8 px after the box (the 4 px margin around the box in IntelliJ's 24 px `CheckBox.iconSize` plus `CheckBox.textIconGap` 4). There is no hover state; keyboard focus draws a 2 px `border.focused` ring.

Not covered here: floating surfaces, whose radii are in [Popups and menus](#popups-and-menus), editor tabs, switches, avatars and other fully round shapes.

## Editor tabs, project panel and scrollbar

- Hidden tabs: when the editor tabs do not fit, a ˅ button at the right end of the tab bar lists the tabs outside the visible area, like WebStorm's "Show Hidden Tabs" drop-down. Picking one activates it and scrolls it into view. `"tab_bar": { "show_hidden_tabs_button": true }`.
- Test-file colours: test files get WebStorm's Tests file colour, a file whose name has `.spec.` or `.test.` before the last extension (`a.spec.ts`, `a.e2e.test.tsx`) or that sits under a `__tests__` folder. The selected test tab keeps the strip-coloured pill with an inner green fill inset 3 px (2 px compact); the fill is solid in the focused pane and at 40% in an unfocused one, like IntelliJ's `IslandsTabPainter`. Unselected test tabs are tinted with the colour at 40%. The colour is the theme's `tab.test_file_background`. `"tabs": { "file_colors": false }` turns it off (Settings window: Window & Layout → Tab Bar).
- Compact tabs: with `"unstable.ui_density": "compact"` the tab label is 1 px smaller, like IntelliJ's `EditorTabs.fontSizeOffset.compact`.
- Tab bar buttons: after the ˅ button, a ⋮ button ("Recent Files, Tab Actions, and More", WebStorm's editor tab entry point) is shown whenever the pane has a tab, focused or not. Its menu has Recent Files, Go to File…, Close All Tabs, Reopen Closed Tab, Unsplit and Unsplit All (only while the editor is split, Unsplit All only with more than two groups) and Configure Editor Tabs…. The active tab's own buttons (the Markdown layout buttons) sit between ˅ and ⋮, separated by dividers like IntelliJ's tab actions: ˅ | buttons | ⋮. While ˅ is hidden, only the divider before ⋮ remains; without ⋮ there are no dividers. The New…, Split Pane and Zoom buttons are gone: splitting is in the tab menu and Zoom keeps its key binding. `"tab_bar": { "show_tab_bar_buttons": false }` hides ⋮.
- Tab menu: right-clicking a tab opens WebStorm's editor tab menu, acting on the clicked tab: Close; Close Other Tabs (disabled with one tab); Close All Tabs; Close All but Pinned (only with pinned and unpinned tabs); Close Tabs to the Left and Close Tabs to the Right (only when there are unpinned tabs on that side); Copy Path/Reference… (Absolute Path, File Name, Path from Content Root, Path from Repository Root); Split Right, Split and Move Right, Split Down, Split and Move Down (the move entries need two tabs); Unsplit and Unsplit All; Pin Tab or Unpin Tab; Keep Tab Open (preview tabs); Configure Editor Tabs…; Reopen Closed Tab; Open In (Finder, Terminal); Git (Open File Permalink, Copy File Permalink). With the editor split, "Close All Tabs" and "Close All but Pinned" end with "In Group". Zed's Close Clean, Close Multibuffers, Make Tab Read-Only and Reveal In Project Panel entries were removed; the Project panel header's Select Opened File button replaces the last one.
- Project panel selection: the hovered and selected rows are rounded pills inset from the panel edges, like WebStorm's tree selection. The selection is the theme's selection color while the panel has focus and a neutral gray otherwise; the keyboard cursor is a border on the pill. `"project_panel": { "rounded_selection": true }`; `false` restores full-width square rows.
- Project panel header buttons: like WebStorm's Project tool window, the header has Select Opened File (⌖), Expand All and Collapse All before —. Select Opened File reveals and selects the file of the active editor tab in the tree, expanding its folders, and moves focus to the tree; with no file behind the tab (an unsaved buffer or a file outside the project) it only focuses the panel. Opening a file does not select it in the tree by itself; `"project_panel": { "auto_reveal_entries": true }` brings that back and hides the Select Opened File button, as IntelliJ does in its Always Select Opened File mode. Expand All and Collapse All are the `project panel: expand all entries` and `project panel: collapse all entries` actions. The buttons follow `tool_window_headers.always_show_actions`; the `project panel: select opened file` action is available from the command palette.
- Excluded directories: like IntelliJ's excluded folders, "Mark Directory As > Excluded" in the Project panel context menu excludes a directory and "Cancel Exclusion" brings it back; `excluded_directories` in the settings lists more, and every `**/node_modules` is excluded by default so a large JavaScript or TypeScript monorepo does not scan, watch or lint its installed packages. An excluded directory stays in the tree with IntelliJ's excluded folder icon, but Flint never scans or loads its contents and project search and Search Everywhere skip it. Like IntelliJ's LSP client, Flint never opens its files in a language server and drops the problems a server reports for it, so linters and type checkers such as ESLint and the TypeScript server report nothing there. Only the TypeScript server, which learns about installed, upgraded and removed packages solely from file events, still gets the changes inside excluded directories that its watchers ask for, taken straight from the file system events without loading the directory; other servers hear nothing from excluded directories. Excluding a directory closes its open files in the language servers and clears their problems; cancelling the exclusion opens them again. The Project panel keeps its exclusions in Flint's own database, not in a project file.
- Editor scrollbar: the thumb is a narrower rounded pill inside the track, like the macOS scrollbar in JetBrains IDEs. Dragging still works on the whole track width. `"scrollbar": { "rounded_thumb": true }`.
- UI font: the default `ui_font_family` is `Inter`, the font IntelliJ's New UI draws its interface in on macOS (`LafManagerImpl`); Inter 4.1 (OFL) is bundled in `assets/fonts/inter`. `".SystemUIFont"` gives San Francisco and `".ZedSans"` gives IBM Plex Sans.
- Text sizes are whole pixels on IntelliJ's macOS type scale. Labels, rows, menus, popups, tool window titles, tabs, the VCS widget, the branches popup, dialogs and the Git log are 13 px. Secondary text is 12 px (`JBFont.medium()`). Counters, hints and tool window stripe names are 11 px (`JBFont.small()`). Dialog titles are 14 px, and larger headings 16, 18 and 22 px. Labels keep these sizes inside editors, whose own rem follows the buffer font. They follow `ui_font_size` 2 px below it (15 by default; 16 makes labels 14 px). Fractional sizes such as 13.125 px no longer appear, so the same label is the same size everywhere and sits on the pixel grid on 1x external displays too.
- Icons are 16, 14 and 12 px at the default `ui_font_size`, like IntelliJ's, and are drawn at whole-pixel sizes, so their strokes stay sharp.
- The editor and its annotations follow IntelliJ: JetBrains Mono 13 px with a 22 px line. Line numbers are JetBrains Mono 12 px. Inlay hints, usages hints, inline blame and inline diagnostics use Inter at the buffer size minus 1 px (12 px). Breadcrumbs, Search Everywhere symbols and multibuffer file headers use the UI font, and the commit message is JetBrains Mono 13 px (`git_commit_buffer_font_size`).
- Project tree and editor: folders show a disclosure chevron followed by the folder icon (`"project_panel": { "folder_indicator": "both" }`). The editor line height is 22 px at the default 13 px font (`"buffer_line_height": { "custom": 1.6923077 }`), WebStorm's line height for JetBrains Mono 13: IntelliJ uses `ceil(font metrics height × 1.2)`, with 1.2 the default line spacing.
- Breadcrumbs: like WebStorm, they are shown below the editor and list only the code structure at the caret, not the file path; see [Breadcrumbs](#breadcrumbs). `toolbar.quick_actions` stays `false`, so no toolbar row is shown above the editor.
- Settings window: Window & Layout → Tab Bar, Panels → Project Panel, Editor → Scrollbar, Appearance → UI Font.

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
- Matching braces are highlighted only when the caret touches a brace, or is separated from one only by spaces and tabs on the same line, like IntelliJ's `BraceHighlightingHandler`. Before, the innermost pair enclosing the caret was always highlighted. Quotes are not braces, as in IntelliJ's brace matchers: after a string's closing quote in `("text")` the parentheses are highlighted, and a string on its own highlights nothing.
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

- One group per severity, errors first, then warnings and weak warnings (language server information diagnostics): IntelliJ's 16 px icon tinted in the severity colour, 1 px before the count in the UI font 2 px smaller, with 6 px between groups and 5 px at the edges. Hints are not counted. Without problems the widget shows IntelliJ's green check.
- With problems, ˄ and ˅ go to the previous and next highlighted problem (`editor: go to previous highlighted error` and `editor: go to next highlighted error`, Shift-F2 and F2 in the JetBrains keymap, and Go → Previous Problem and Next Problem). Like IntelliJ they go to errors first: while the file has errors they skip warnings, while it has warnings they skip weak warnings, and they never stop on hints. They only visit the severities the highlight level shows, so at Syntax they visit errors only. `editor: go to diagnostic` still visits every severity.
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
- The preview follows the editor: it renders the text 20 ms after you stop typing, like IntelliJ's debounced preview updates, and scrolls so that the line at the top of the editor is at the top of the preview. Scrolling the preview does not move the editor.
- Links: `#heading` scrolls the preview; a link to another Markdown file with `#heading` opens that file and scrolls its preview to the heading; project files open in an editor, at the position for `path:line:column` and `#L` fragments; other URLs open in the browser. Task list checkboxes toggle the line in the source. Images are resolved relative to the file.
- In the preview: Page Up and Page Down, Up and Down, Alt-Up and Alt-Down (by block), Cmd-Up and Cmd-Down (top and bottom) scroll. The buffer font size commands change `markdown_preview.font_size` while the preview has focus.
- The old preview tab is gone with its commands (`markdown: open preview`, `markdown: open preview to the side`, `markdown: open following preview`, `markdown: close and return to editor`) and the settings `open_markdown_files_in_preview`, `limit_content_width` and `max_width`.
- Known differences from IntelliJ: IntelliJ's fourth layout, Live Preview (Experimental), whose button sits between Editor and Preview and Preview and which renders the Markdown inline in the text editor, is not implemented; the floating toolbar does not fade in and out; the preview scroll is not animated and cannot be turned off; the layout and the orientation are not saved per file; the preview is rendered by Flint's Markdown renderer, not a browser.

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

### Show Usages popup

Show Usages lists the usages of a symbol in a floating popup next to it, like IntelliJ's Show Usages, instead of a multibuffer tab. It needs the file's language server.

- It opens with `Cmd-Alt-F7` in the JetBrains keymap (`editor: show usages`), with Cmd+click or `Cmd-B` on a declaration, and with a click on a usages hint ("3 usages", "no usages") or an implementations hint ("2 implementations"). On a declaration it takes the place of the find-all-references fallback, so it needs `go_to_definition_fallback` set to `find_all_references`, and `Cmd-B` also needs `lsp_results_location` set to `multi_buffer` (both are the defaults). Cmd+click moves the caret to the clicked symbol and searches it, and holding Cmd over a declaration underlines it like a link. It opens below the caret, at the mouse after Cmd+click and at the click for a hint. `Alt-F7` (`editor: find all references`) still opens the multibuffer tab.
- Usages hints: like IntelliJ's code vision with the position "Right", the language server's code lenses are drawn after the end of the declaration's line, never on an extra line above it: "no usages", "1 usage", "N usages", and "N implementations" (none at 0). Other lenses (for example rust-analyzer's "▶ Run") keep their own text. TypeScript and JavaScript interface properties, class fields and enum members get no hint. Every TypeScript and JavaScript function gets a usages hint. Class methods get an implementations hint only when they are abstract, because each one searches the whole program; `implementationsCodeLens.showOnAllClassMethods` under `lsp.tsgo.settings.typescript` adds it to every class method. A hovered hint is underlined in the link colour; a click opens Show Usages at the click, and a "no usages" hint shows the "No usages found" hint. Inline diagnostics and inline blame on the same line start after the hints. The `code_lens` setting turns them off.
- Code author hint: in a git file, the last hint of a declaration's line is IntelliJ's code author: a person icon and the author of most of the declaration's non-blank lines from git blame (ties go to the alphabetically first name), then "+N" when N other authors wrote lines of it and " *" when it has uncommitted changes; a declaration without committed lines reads "new *". It appears once blame has loaded (Flint starts blame for it even when inline blame is off) and a click shows the git blame gutter, like IntelliJ's Annotate.
- Header: the symbol's type and name in bold, then its container and project path in a muted colour, like IntelliJ ("Property qaseCode (QaseReporterOptions, automation/reporters/qase-reporter.ts)"; a symbol without a container shows only the path, and a symbol whose type is unknown shows only its name). An implementations hint keeps "Implementations of 'name'". On the right: the row count in a muted colour ("16 usages"; "100+ usages" while more rows can be loaded or some usages are out of scope; "3 of 7 usages" when filters hide some) and the Open in Find Tool Window button, which opens every usage that passes the filters and the scope in a multibuffer tab titled "References to name".
- Toolbar: the filters, the Show options menu (eye), Preview Source, the scope chooser, a muted "Usages" label and the gear.
- Rows: the file icon (unless `tabs.file_icons` is off), the file name, the line number, a read or write access icon when the language server reports one, and the code line with the usage in bold and syntax colours. Usages on the same line share a row, and every count in the popup counts rows. Rows are sorted by project folder, path, line and column. A line longer than 200 characters (indentation included) is cut like IntelliJ's: from 40 characters before the first usage to 100 after it, widened to whole words when the usage is an identifier, without ellipses. The row with the usage under the caret is marked "Current" and is not selected first. Enter opens the selected usage, and so does a click (a double-click while Preview Source is on). Escape clears the speed search, then closes the popup.
- Footer: the path of the selected usage's file relative to your home folder (`~/…`), cut to 80 characters.
- Filters, all on by default: Read Access (⌃R), Write Access (⌃W), Import Statements (⌃I) and Comments. Read and write access come from the language server's document highlights; a usage it does not classify is never hidden by them. A row at the end says how many rows the filters hide; opening it turns all filters back on.
- Show options: Short File Path shows the shortest path that tells files with the same name apart instead of the file name; File Structure adds the enclosing declarations (class, method) to each row.
- Preview Source shows the code around the selected usage below the list; the divider between them can be dragged.
- Scope chooser: Project Files (the default), All Places, Open Files and Current File. Once all rows are loaded, a row at the end counts the rows outside the scope ("...<2 usages are out of scope 'Project Files'>..."); opening it searches in All Places, and so does pressing ⌥⌘F7 again.
- ⌥F7 in the popup opens the usages in a multibuffer tab, like Open in Find Tool Window. ⌥⇧⌘F7 or the gear opens the Find Usages dialog. In the editor, ⌥⇧⌘F7 (JetBrains keymap) opens the same dialog for the symbol at the caret, or for the search of the hint on screen; Find in the dialog runs Show Usages with the chosen scope.
- Hints instead of the popup: without any usage a hint says "No usages found in Project Files", and when every usage is outside the scope a warning hint says "2 usages are out of scope 'Project Files'". When a single row is left and nothing is filtered out or out of scope, Flint jumps to its first usage and hints "It's the only usage in Project Files", or "All 3 usages in Project Files are in this line" when the row holds several; once the editor has drawn the moved caret, that hint appears below it (at the original position when the usage opened in another editor). Every hint has a "Find Options…" link, and outside All Places it adds "Press ⌥⌘F7 again to search in 'All Places'".
- The list shows 100 rows at a time; the "...<more usages>..." row at the end loads the next 100. PageUp and PageDown move the selection by a page, ⌥↑ and ⌥↓ go to the first and the last row.
- Typing starts a speed search over the file names and the code lines: case-insensitive, by substring or by the typed letters in order. Matches are highlighted.
- The popup can be moved by its header and resized by its bottom-right corner.
- Remembered across restarts: the filters, the scope picked in the chooser, Preview Source and its divider, the Show options and the popup size (turning Preview Source on or off resets the size).
- Not implemented: selecting several rows, the "Show Results In Generated Code" filter, the production, test and module scopes and the hierarchy button.

## Main toolbar

The title bar works like WebStorm's [main toolbar](https://www.jetbrains.com/help/webstorm/new-ui.html), left to right:

- Window buttons (close, minimize, zoom), described below.
- Back and Forward icon buttons (22 × 22, 16 px icons). They dispatch the pane history actions and are disabled when there is no history.
- Project tabs: one tab per project open in the window, in the same row, right after the Search Everywhere field (see [Several projects in one window](#several-projects-in-one-window)). With a single project its tab is the only place the project name appears. Hidden when no project is open.
- VCS widget: the branch status icon, the branch name, then the incoming and outgoing arrows when the branch is behind or ahead of its upstream, and a ˅. The arrows are IntelliJ's 12 px incoming (blue, the info colour) and outgoing (green, the success colour) icons. The tooltip is "Git Branch: main", followed by "Root: <repository>" when the project has several repositories, then the incoming and outgoing commit counts ("2 incoming and 1 outgoing commits"). A branch name longer than 84 characters is cut to 80 with "…" in the middle, keeping its last five characters. While an operation runs the widget shows a warning icon and "Merging main", "Rebasing main" (tooltip "Git: Rebase in progress"; the branch being rebased comes from git's `rebase-merge/head-name`, since HEAD is detached during a rebase), "Cherry-picking in main" or "Reverting in main". In a detached HEAD the widget shows the tag HEAD is on with a tag icon, or else a warning icon and the 8-character commit hash, with the tooltip "Git: Detached HEAD doesn't point to any branch". Click the widget to open the [Git Branches popup](#git-branches-popup). In a project without a Git repository the widget reads "Version Control", with the tooltip "Configure a version control for the project", and its popup has Create Git Repository…. The worktree button is hidden by default.
- On the right: Search Everywhere and Settings as 22 × 22 icon buttons (`MainToolbar.Icon`), 10 px apart. `title_bar.show_search_button` hides Search Everywhere. The Settings gear opens IntelliJ's settings menu with Settings…, Plugins… and, after a separator, Keymap…; IntelliJ's Theme… and View Mode… entries have no Flint equivalent. With the project gradient a pressed icon button is filled with the 9% hover colour, without it with 16% (`MainToolbar.Icon.pressedBackground`).

Widget look, from IntelliJ's main toolbar combo widget (`ToolbarComboButton`, `MainToolbar.Dropdown.*`):

- The VCS widget and the worktree button are 30 px pills (26 px in compact density, the `MainToolbar.Button.size` height) with a 6 px radius in every density (`MainToolbar.Dropdown.arc` 12), 10 px padding on the left and 6 px on the right, a 6 px gap between the icon and the text and 2 px before the ˅ (`ToolbarComboButtonUI.iconTextGap` 6, `BEFORE_CHEVRON_GAP` 2). The text is in the normal text colour at the UI font size. The icon and the ˅ are 16 px and the arrows 12 px; they are not scaled by `ui_font_size`, only by `title_bar.icon_size` (by N/14 for `icon_size` N, so a 16 px icon becomes 16·N/14), and the pills grow to keep 4 px above and below a scaled icon. The branch, worktree and ˅ icons use the theme's `icon` colour (`#c3c5cb`, IntelliJ's `icon-default-stroke`).
- The pill is transparent until hovered, like IntelliJ: white at 0% rest and 9% (`#FFFFFF17`) when hovered and while its popup is open in dark themes, and the matching blacks in light themes. Like `ToolbarComboButtonUI` it has no pressed colour. There is no outline, except the 2 px `border.focused` focus ring while it has keyboard focus.

Project gradient: like WebStorm's colored project headers, the title bar is tinted with a project color, fading out from the left edge to the middle. The color is always IntelliJ's violet project color `#955ae0` (`RecentProject.Color7` `Avatar.Start`), whatever the project. It appears only when a project is open and no background image is set, as in WebStorm's Islands.

The gradient, the Search Everywhere button and the worktree button can be turned on or off in `settings.json` or in Settings → Window & Layout → Title Bar:

```json
"title_bar": {
  "show_project_gradient": true,
  "show_search_button": true,
  "show_worktree_name": false
}
```

Window buttons, like WebStorm on macOS:

- Every project window has this title bar, and the window buttons (close, minimize, zoom) sit at its left end, vertically centered. The widgets start after the buttons; in full screen, where macOS hides the buttons, they start at the left edge.
- Outside full screen the standard macOS title bar is never drawn over this one. When the window leaves full screen, or when macOS cancels a switch to full screen because it is busy with another animation or gesture, the window gets its transparent title bar and its window button positions back, so the branch picker and Search Everywhere stay visible.
- The Settings, About, Conflicts and Merge Revisions windows use the standard macOS title bar with the window title, like WebStorm dialogs, so the buttons never cover their content.

### Several projects in one window

One Flint window holds several projects, and every git worktree of a project counts as a project of its own. IntelliJ shows each project in its own window and groups windows as macOS window tabs (Window → Merge All Project Windows); Flint keeps the projects in one window and draws IntelliJ's Islands window tabs in the main toolbar row instead of a separate row.

- Opening a project adds it to the current window and switches to it: File → Open…, Open Recent, `flint <folder>` from the command line, a folder dropped on the Dock icon or opened from Finder, and switching to, opening or creating a git worktree (including Open Worktree… in the branches popup and Open Existing Worktree in the Worktree Conflict dialog). The projects already in the window stay open with their editors, tool windows, terminals and language servers. A project that is already open, in this window or another one, is brought to the front instead of being opened twice. The explicit new-window actions (File → New Window, Open Recent's "New Window" and secondary click, Open in New Window in the worktree picker) still open a new window.
- Clicking a tab switches to its project. The active tab is filled with the island colour (the theme's `panel.background`) with the island corner radius, 1 px in from its sides; a hovered tab gets the hover colour. The active project's name is in the normal text colour, the others are muted. Between two tabs that are neither active nor hovered a 1 px line (`#FFFFFF21` in dark themes, `#00000020` in light ones) is drawn 5 px in from the top and bottom. The tabs are as tall as the other toolbar widgets (30 px, 26 px in compact density), with text one size below the UI font (`JBFont.medium()`), 4 px of padding on the left and 15 px on the right (`WindowTabsComponent`, `IslandsUICustomization`).
- A tab is labelled with the project's folder name. When two projects of the window have the same name, both tabs add the folder relative to your home folder, `flint [~/worktrees/feature/flint]`, like IntelliJ's window title (`PlatformFrameTitleBuilder`). The tooltip is the project folder relative to your home folder (`~/IdeaProjects/Flint`).
- The tabs start 10 px after the Search Everywhere field and share the rest of the row equally, like IntelliJ's window tabs, each 220 px wide at most and 72 px at least. The project name is centered in the normal UI font size. The close button has its own 20 px slot on the left, 6 px from the name, and an empty slot of the same size on the right keeps the name centered.
- Hovering a tab shows its close button. Clicking it closes that project only, after asking to save its changes; Option-click closes every other project. A middle click on a tab also closes it. Closing the active project switches to the tab after it, or the one before it when it was the last; closing the last project leaves the empty window.
- Right-click a tab for IntelliJ's tab menu: Close Tab, Close Other Tabs, Move Tab to New Window. Drag a tab onto another tab to move it there.
- Many projects make the tabs narrower, with their names cut short; the VCS widget and the Search Everywhere field never move.
- In the JetBrains keymap ⌥⌘` (`multi_workspace: next project`) and ⇧⌥⌘` (`multi_workspace: previous project`) switch to the next and previous tab and wrap around, like IntelliJ's Next and Previous Project Window. Close Project closes only the active project; a linked worktree no longer closes together with its main checkout.
- Open Recent lists the window's projects under IntelliJ's "Open Projects", before Recent Projects, one row per project and worktree. Choosing one switches to it; the secondary action moves it to a new window and the × closes it.
- Opening a folder (File → Open…, `flint <folder>`, Finder or the Dock) while the window shows only the empty welcome project replaces that empty project instead of keeping it in the background, after asking to save an Untitled file typed into it.
- After a restart every project of every window comes back, in the order of its tabs, and the project that was active is active again.

To open projects in new windows instead, as before, set Default Open Behavior to "Open a New Window" in Settings → General, or in `settings.json` (`cli_default_open_behavior` does the same for `flint <folder>`):

```json
"default_open_behavior": "new_window"
```

With `new_window`, File → Open… and Open Recent open new windows, and switching to a worktree replaces the project in the window, except that a project with unsaved changes is never dropped without asking: it stays in the window as a tab. `flint -e <folder>`, and `flint <folder>` while `cli_default_open_behavior` is `existing_window`, still add the folder to the current window as a tab.

## Settings window

The Settings window looks like IntelliJ's Settings dialog (`SettingsDialog`, `SettingsEditor`, `SettingsTreeView`, `ConfigurableEditorBanner`, `TitledSeparator` and the Kotlin UI DSL spacing of intellij-community):

- The window is titled "Settings", is at least 900 × 700 px (IntelliJ's minimum) and has the standard macOS title bar. Both columns use `panel.background` and are separated by a 1 px `border.variant` line, like IntelliJ's one-pixel splitter.
- Left column, 20% of the window width, between 200 and 300 px: a 28 px search field with a 4 px radius and a magnifier and no placeholder, as in IntelliJ; below it the tree. Pages are bold with a 16 px ˃ / ˅ chevron 13 px from the left edge, like IntelliJ's tree; their sections are indented by 16 px and keep an empty slot as wide as the chevron, so a section's text starts 16 px right of its page's text. The selected row is a flat full-width `ghost_element.selected` band, like IntelliJ's wide tree selection. The row with keyboard focus gets a 1 px `border.focused` outline. The "Focus Navbar" hint at the bottom is gone; its key binding still works.
- Header: the path of the page as bold breadcrumbs separated by chevrons; clicking a parent goes back to it. On the right, a drop-down picks the scope ("User" or a project's settings file) and the "Edit in settings.json" link opens the file of that scope.
- Content: group titles are IntelliJ titled separators, the title in the UI font followed by a 1 px `border.variant` line, with 20 px between groups and no lines between rows. On/off settings are checkboxes with the title as their label; other settings read "Title:" followed by the control; descriptions are smaller muted text below. Drop-downs and pickers are 28 px combo boxes with a ˅, text and number fields are 28 px fields, all with a 4 px radius and a 1 px `border`; buttons are dialog buttons and "Edit in settings.json" entries are links.
- Differences from IntelliJ: there are no Cancel, Apply and OK buttons, because every change is still written to `settings.json` at once; the header has no back and forward buttons and no Reset link (each changed setting keeps its own reset button); the tree is not resizable.

## Git Branches popup

The branch popup works like WebStorm's [Git Branches popup](https://www.jetbrains.com/help/webstorm/manage-branches.html). It replaces the old Branches | Stashes picker.

### Where it opens

- The VCS widget in the title bar opens it as a popover.
- The branch button of the commit window opens it as a popover too.
- `git: branch`, `git: switch` and `git: checkout branch` open it as a modal for the active repository.
- Stashes are no longer a tab. `git: view stash` opens the stash list as its own modal (Drop and Show keys are unchanged).
- The popup surface, its gear menu, the branch submenus and every dialog opened from it are always opaque, even when your theme overrides make `elevated_surface.background` translucent.
- Size: the popup opens at least 375 × 312 px, as wide as its widest row and tall enough for IntelliJ's row count (the top rows, plus the rows inside Recent, or Local when recent branches are hidden, plus one, at most 20). The height stays fixed while you expand sections; the list scrolls instead. Drag the right edge, the bottom edge or the bottom-right corner to resize it; the size is remembered until "Restore Popup Size" in the gear menu. Drag the strip above the search field to move the popup.
- Pressing the shortcut of a top action (Update Project, Commit, Push, New Branch, Checkout Tag or Revision) inside the popup closes it and runs the action for the popup's repository.

### Layout

- Header: a search field with the placeholder "Search for branches and actions", a Fetch button and a gear. Fetch fetches every repository of the project and reports "Fetch successful", or "Fetch failed" with git's errors (each prefixed with its repository when there are several); while it runs the button is a spinner, also in a popup opened later.
- The gear menu has "Show Actions in Search Results", "Group by Directory", "Show Recent Branches", "Show Tags" and "Restore Popup Size" (greyed until you resize the popup). With several repositories the first item becomes an "Add in Search Results" header with "Actions" and "Repositories". The toggles are on by default and are remembered per repository.
- Top actions: Update Project… (⌘T), Commit… (⌘K), Push… (⇧⌘K, which opens the Push dialog for the current branch instead of pushing at once), then the actions of an operation in progress (Abort Rebase, Continue Rebase and Skip Commit; Abort Merge; Abort Cherry-Pick and Continue Cherry-Pick; Abort Revert), then New Branch… (⌥⌘N), New Worktree… and Checkout Tag or Revision…. The shortcuts are shown from your current keymap; with the JetBrains keymap they are the ones listed here. New Worktree… opens Flint's worktree picker, where the new worktree is named and created.
- New Branch… is greyed in an empty repository without commits ("Cannot create new branch in empty repository. Make initial commit first"), Checkout Tag or Revision… with "Checkout is not possible before the first commit.".
- With several repositories in the project, a row per repository sits between the actions and the tree, with the repository's color (IntelliJ's New UI palette, by name order), its branch on the right and a `>`. Selecting one switches the repository the popup shows, and the sections are titled "Recent Branches in <repository>", "Local Branches in <repository>", "Remote Branches in <repository>" and "Tags in <repository>".

### Tree

- Sections: Recent (at most five branches you checked out last, the current branch first, taken from the reflog), Local, Remote and Tags. An empty section is not shown. While searching, a branch in Recent is not repeated in Local.
- Branches are grouped into folders by `/` when "Group by Directory" is on, in Recent too; a remote's branches are grouped under the remote name. `*/HEAD` remote symrefs are hidden.
- Order: the current branch first, then favorites, then folders, then the rest in natural order. A folder that holds the current branch or a favorite moves up with it. Recent keeps the reflog order, with a folder where its first branch is.
- Favorites: `Space` or a click on the branch or tag icon toggles the star. `main`, `master`, `origin/main` and `origin/master` are favorites until you remove them.
- Rows show the tracked remote branch of a local branch and the incoming (↓, blue) and outgoing (↑, green) commit counts in IntelliJ's colors and small font; hover the row for the commit count tooltip. The current branch has its own marker; in a detached HEAD the tag at HEAD is marked. The selected row shows a hollow star to add a favorite. Icons are 16 px. A name too long for the row ends with "…".
- Sections and folders you expand stay expanded the next time the popup opens.
- When the popup opens, it selects the branch you were on before the last checkout or new branch made from Flint (stored per working tree), or the current branch when there is none.

### Search

- Typing filters branches, tags and, with "Show Actions in Search Results", the top actions (and, with several repositories, the repositories with "Repositories"); with a filter off, that whole list stays above the matching branches. Matching is IntelliJ's: the query is matched anywhere in the name as one pattern, case-insensitively, later characters must start a word or a camelCase hump (`fl` finds `feature/login`, `fc` does not find `reformatCode`), a space acts as a wildcard, and a query typed in the Russian keyboard layout is fixed. Matches have IntelliJ's filled search-match background.
- The best match is selected after every change. With no match the list says "Nothing found" (actions filter on) or "Branch not found", and the search text turns red.
- Like IntelliJ, a search opens only the sections and folders that hold the current branch, the recent branches and the best match; other sections with matches keep the state you left them in, and what a search opened stays open, also after the search is cleared. Sections and folders can be opened and closed while searching.
- `Escape` clears the search first and closes the popup when it is already empty.

### Keys

| Key | Action |
|---|---|
| `Up`, `Down` | Move the selection, skipping separators; while searching, move over branches, tags and the searched actions and repositories only, opening closed sections and folders on the way, and wrap around |
| `Enter` | Run a top action; expand or collapse a section or folder; open the submenu of a branch or tag |
| `Right` | Expand a section or folder, step into it when expanded, or open the submenu of a branch or tag |
| `Left` | Collapse an expanded section or folder, otherwise select the parent row |
| `Space` | Toggle favorite |
| `Escape` | Clear the search, then close |

### Submenu of a branch or tag

Every branch and tag row opens a submenu (`>`), 2 px inside the popup's right edge and level with the row. Its items depend on the kind of ref, and unavailable items are greyed with a tooltip (in an empty repository: "Action is not possible before the first commit."):

- Current local branch: New Branch from 'X'…, Show Diff with Working Tree, New Worktree from 'X'…, Update, Push…, Tracked Branch 'origin/X', Rename….
- Other local branch: Checkout, Open Worktree…, New Branch from 'X'…, Checkout and Rebase onto 'current', Checkout and Update, Compare with 'current', Show Diff with Working Tree, Rebase 'current' onto 'X', Merge 'X' into 'current', New Worktree from 'X'…, Update, Push…, Tracked Branch 'origin/X', Rename…, Delete. Open Worktree… appears only when another worktree has the branch checked out; it opens that worktree as a project tab in the current window (or brings forward the window that already has it), and Checkout and Rebase onto 'current' is hidden then. Tracked Branch 'origin/X' appears for a branch with an existing remote upstream and holds the remote branch's items. Update and Checkout and Update are greyed for a branch without a tracked remote branch, Update also while a fetch runs, and Rebase is greyed in a detached HEAD.
- Remote branch: Checkout, New Branch from 'X'…, Checkout and Rebase onto 'current', Compare with 'current', Show Diff with Working Tree, Rebase 'current' onto 'X', Merge 'X' into 'current', New Worktree from 'X'…, Pull into 'current' Using Rebase, Pull into 'current' Using Merge, Delete.
- Tag: Checkout, Show Diff with Working Tree, Merge 'X' into 'current', New Worktree from 'X'…, Push to each remote (a "Push Tag" submenu from six remotes on), Delete. A tag that is the current detached HEAD has no Checkout, no Merge and no Delete.
- New Worktree from 'X'… creates a worktree in a folder named after the ref and opens it as a project tab, like IntelliJ's New Worktree with the ref preselected. A local branch is checked out there, a remote branch gets a local branch of the same name that tracks it, and a tag gets a detached HEAD. In a project with several repositories only the repository of that ref gets the worktree.
- Delete shows IntelliJ's ⌦ shortcut; ⌦, ⌫ or ⌘⌫ while the submenu is open delete the ref.

### What the operations do

- Before checkout, merge, rebase, update, reset, Compare with 'current' and Show Diff with Working Tree, every modified editor of a file is saved without asking, like IntelliJ's `saveAllDocuments`, so the operation works with your edits and smart checkout stashes them.
- While an operation runs, the bottom right shows IntelliJ's progress title: "Checking out main…", "Checking out new branch x…", "Checking out x from origin/x…", "Creating branch x…", "Merging x…", "Rebasing x…", "Rebasing onto x…", "Deleting x…", "Renaming x to y…", "Deleting tag v1 on remote…", "Updating…" or "Updating branches…".
- Checkout, merge and rebase are smart: when local changes would be overwritten, a "Git Checkout Problem" (or "Git Merge Problem") dialog offers Smart Checkout (stash the changes, run the operation, restore them), Force Checkout (checkout only) or Don't Checkout; Enter chooses Smart Checkout. The stash is named like IntelliJ's ("Uncommitted changes before Checkout at 10/20/26, 3:04 PM", "Uncommitted changes before rebase"). A conflict while restoring opens the Conflicts dialog. A stash that fails is reported as "Could not save uncommitted changes on Checkout".
- Checking out a branch that another worktree has checked out asks "Worktree Conflict" ("Branch 'x' is already checked out in a worktree at: <path>") with Checkout Anyway (`git checkout --ignore-other-worktrees`), Open Existing Worktree (opens that worktree as a project tab in the current window, or brings forward the window that already has it) and Cancel.
- Branch workspaces, like IntelliJ's: before a checkout the open editors are remembered for the branch you leave; when you check out a branch again, its editors are reopened in place of the current ones (files opened during the checkout stay open) and the notification "Workspace associated with branch 'x' has been restored" offers Rollback. Turn it off with `"git": { "restore_workspace_on_branch_switch": false }`.
- New Branch… and New Branch from 'X'… ask for a name with "Checkout branch" and "Overwrite existing branch". Names are validated like WebStorm does and cleaned up while you type.
- Checkout Tag or Revision… checks out a tag or any revision as a detached HEAD.
- Merge reports "Already up to date", success (with a "Delete x" action for a merged local branch that is not protected) or a conflict that opens the Conflicts dialog; conflicts left unresolved produce a "<branch> Merged with Conflicts" notification with Resolve…. Changes stashed for the merge are restored once the conflicts are resolved; when the conflicts are left unresolved a "Local changes were not restored" warning with "View saved changes…" says they are still in the stash. A merge or checkout refused because of unmerged files says "Cannot merge because of unmerged files" with "Resolve conflicts…". Rebase reports success or stops at conflicts with a "Rebase stopped due to conflicts" notification (Resolve…, Continue, Abort, and View Stash… when changes were stashed), and asks before rebasing published commits (commits already on a protected remote branch). Checkout and Rebase onto 'current' runs the same checks. A rebase that cannot start says "Rebase not allowed" with the unfinished operation. Abort Cherry-Pick and Abort Revert ask "Abort cherry-pick?" / "Abort revert?" and run `git cherry-pick --abort` / `git revert --abort` ("Cherry-Pick abort succeeded"); Continue Cherry-Pick opens the Conflicts dialog while conflicts are left, then runs `git cherry-pick --continue` and reports "Cherry-pick continue successful" with the applied commits. Abort Merge and Abort Rebase (menu, popup and notification) ask first ("Abort merge?", "Abort rebase in <repository>?" with Abort and Cancel) and report "Merge abort succeeded" / "Abort rebase succeeded" or "Merge abort failed" / "Abort rebase failed". Aborting or continuing restores only the stash Flint made for that operation, never another stash with IntelliJ's "Uncommitted changes before …" message.
- Update Project… and Update pull the current branch with the configured method (merge, rebase or the branch default). The options dialog can be turned off with "Don't show this dialog again"; the row then reads "Update Project" and Shift-click brings the dialog back. An update by merge stashes local changes silently, and only when something is staged or a locally changed file is changed by the incoming commits; it never shows the Smart Merge dialog. The result "N files updated in M commits" has View Commits, which opens a log tab limited to the updated commits. A branch without a tracked branch reports "Cannot update" / "main has no tracked branch" with "Choose upstream branch", which opens IntelliJ's Update dialog: a remote, the remote branch ("Specify branch"), "Set as tracked branch", Merge or Rebase, and Update. Updating several selected branches in the Branches pane fetches them all and reports "Fetched N targets" once.
- Delete removes a local branch without asking. A branch that is not fully merged is deleted anyway, and the notification offers Restore and View Commits (View Commits keeps the notification open; the "Branch Was Not Fully Merged" dialog has Restore on the left and Cancel as the default button, and says "All commits from branch x were merged" when nothing is left), plus Delete Tracked Branch when the upstream is not protected and no other branch tracks it. A successful Restore just closes the notification. Deleting a remote branch asks first; deleting several remote branches at once (Branches pane) asks once, "Delete Remote Branches" / "Delete remote branches a, b?" with Delete All and the tracking local branches of all of them. Deleting a tag does not ask, and its notification offers Restore and Delete on Remote(s), which checks every remote with `git ls-remote`, deletes the tag only where it exists (`git push --force-with-lease=refs/tags/v1:<sha> :refs/tags/v1`) and reports "Deleted Tag on Remotes: v1" or "Tag Doesn't Exist on Remotes: v1".
- Protected branches are `master` and `main`; set `"git": { "protected_branches": ["main", "release/.*"] }` to change them (each entry is a regular expression for the whole branch name). They cannot be force-pushed or deleted on the remote, a merged protected branch is not offered for deletion, and rebasing commits already on one asks first.
- Rename… and Push… open their dialogs; Push… is preset to the selected branch. Rename refuses the unchanged name ("Branch name x already exists"), and when unsetting the upstream fails the rename is reported as failed. Force Push in the Push dialog is greyed for a protected target ("Force push to origin/main is prohibited"); otherwise it asks "Force Push" / "You're going to force push to 'origin/x'. It may overwrite commits at the remote. Are you sure you want to proceed?" with "Don't warn about this target". `git: force push` and the git panel's Force Push go through the same check: a protected target is refused with "Force push to origin/main is prohibited", any other target opens the Push dialog at that confirmation.
- Push (from the Push dialog and from `git: push`, `git: push to` and `git: force push`) runs `git push --progress`. While it runs, a "Pushing…" indicator with a progress bar sits at the bottom right where WebStorm's status bar shows it: the bar moves back and forth until git reports progress, then fills by WebStorm's weights (counting objects 5 %, compressing 10 %, writing 80 %, resolving deltas 5 %), and hovering it shows git's current progress line. When the push ends, the indicator goes away and an IntelliJ balloon reports the result: "Pushed 1 commit to origin/main", "Pushed main to new branch origin/main", "Force pushed main to origin/main" or "Everything is up to date" with "Create Pull Request"; "Push rejected" (a warning, for example "Push to origin/main was rejected") or "Push failed" (an error with git's message), both with "Show details in console". A cancelled sign-in shows nothing.
- The commit message window (⌘K, or Commit… in the Git Branches popup) has IntelliJ's commit panel buttons in its footer: a filled accent "Commit" button (or "Amend" / "Amend Tracked" while amending, IntelliJ's default `Button.default.startBackground` with the white `text-over-accent` label), an outlined "Commit and Push…" (IntelliJ's `action.commit.and.push.text`; "Amend and Push…" while amending) which runs `git push` once the commit succeeds, like `GitCommitAndPushExecutor`, and, at the right end of the row, the gear button of IntelliJ's "Show Commit Options" menu with Amend, Signoff and Skip Hooks. A batch commit, which resolves conflicts during cherry-pick, revert and the like, has no push button.
- Committing saves the edited files first, like IntelliJ's commit, so the commit holds what the editors show. When nothing is staged, the files saved this way are committed with the other tracked changes.
- After every git command Flint runs (commit, checkout, merge, rebase, pull, push, stash, reset and the like), the repository state is read again as soon as the command exits, even when it fails, like IntelliJ's repository update. The Project panel, the editor tabs and the commit window don't wait for file events, which can arrive late. Several projects share the app's file watcher: when a project's tab is closed, the other projects keep their own watches, including a linked worktree's git directory inside the closed project's folder.
- File colours in the Project panel and the editor tabs always show the local changes against HEAD, like IntelliJ's Project view, so committed files go back to the plain colour. "Diff Against Default Branch" (`git.diff_base`) changes only the gutter and the diff views.
- Notifications of branch operations are IntelliJ balloons at the bottom right: 360 px wide, an information, warning or error icon, a bold title, the message, the actions as links and a close button on hover. Information balloons without actions close by themselves after 10 seconds (hovering pauses the countdown); balloons with actions such as Restore, View Commits or Rollback, warnings and errors stay until clicked or closed, since Flint has no Notifications tool window to find them again.
- Compare with 'current' opens a tab with the commits that exist in one branch but not in the other, in both directions. Like IntelliJ's two compare log panels, each list has a Changed Files pane on its right that shows the files of the selected commit; double-click a commit to open it. Show Diff with Working Tree opens the diff of the selected ref against your working tree.
- The Branch Name field of New Branch… and the field of Checkout Tag or Revision… suggest matching branches, directories and tags below the field (click, Tab or Enter accept a suggestion).

### Not implemented

- IntelliJ's New Worktree dialog: New Worktree… opens the worktree picker and New Worktree from 'X'… creates the worktree without asking for a name or location.
- Keeping the issue id of a long branch name visible when it is shortened, since Flint has no issue navigation patterns.
- Running one branch operation on all repositories at once; Flint works on one repository at a time.
- "Commit and stage".
- IntelliJ's Notifications tool window; balloons with actions stay open instead.

## Git tool window

The Git tool window is a copy of IntelliJ's Git (Version Control) tool window. It opens in the bottom dock: use `git_panel: toggle focus`, the Git button of the left tool window bar, or Cmd-9 in the JetBrains keymap.

### Header tabs

- After the "Git" title the header shows the content tabs: "Log" (titled like "Log: HEAD" when the log is filtered), any extra Log tabs, and "Console". The selected tab gets the Islands rounded highlight, or the classic underline when islands are off.
- "+" (Open New Git Log Tab) opens another Log tab with the selected tab's filters. Extra Log tabs and the Console tab have a close button. Alt-click on a tab closes the other tabs, middle-click or Shift-click closes it, and right-click offers Close Tab, Close All Tabs, Close Other Tabs, Select Next Tab, Select Previous Tab and, with more than one tab, Show List of Tabs (a list of all tabs that selects the one you pick). Close Tab, Select Next Tab, Select Previous Tab and Show List of Tabs show their shortcuts.
- "⌄" offers Show History for the current editor file, Show VCS Console Tab (when the console has output but is not shown) and Worktrees….
- Tabs that do not fit are dropped from the edges of the strip; Show Hidden Tabs lists them.
- Extra Log tabs and the selected tab are restored per workspace. Like IntelliJ, only one extra tab without filters is restored.
- Keys in the tool window: Cmd-W closes the selected tab (or hides the window when the main Log tab is selected), Cmd-{ / Cmd-} and Alt-Cmd-Left / Alt-Cmd-Right select the previous / next tab. The JetBrains keymap adds Ctrl-Shift-F4, Ctrl-Left / Ctrl-Right and Ctrl-Shift-Down (Show List of Tabs).

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
| Actions | Every action available where the popup was opened, with its key binding. `Enter` runs the action in the editor or panel that had focus; `Alt-Enter` (or `Cmd-Enter`) opens the keymap editor to change its binding, like IntelliJ's ⌥⏎ "assign a shortcut". Command aliases from `command_aliases` work here too. |
| Text | Text in the project files (case-insensitive, at most 100 matches). The last row, "Open in Text Finder", hands the query to the full Text Finder. |

Classes and Symbols come from the language servers' workspace symbol search, so they are empty for languages whose server does not provide it. The "Include non-project items" checkbox adds git-ignored files and symbols outside the project.

Layout, the same in the popup and in the Search panel, after IntelliJ's `SearchEverywhereUI`: the header row has the tabs on the left at their natural width, as Islands pills (28 px, 24 px compact; the selected tab has the `tab.active_background` `#233558` fill and the `border.selected` border), and on the right the "Include non-project items" checkbox, the "Text in <folder>" chip and the scan indicator. Below a divider comes the query row with a magnifier icon; the query text is aligned with the result names. Section headers are muted labels followed by a thin line. The footer ("Preview", "Actions…"), shared by every picker, uses small `text.placeholder` labels like IntelliJ's advertiser strip.

### Shortcuts (JetBrains keymap)

| Keys | Action |
| --- | --- |
| `Shift` `Shift` | Open on the All tab. Pressing it again on the All tab toggles "Include non-project items"; on another tab it switches to All. |
| `Cmd-O` | Open on the Classes tab. |
| `Cmd-Shift-O`, `Cmd-Shift-N` | Open on the Files tab. |
| `Cmd-Alt-O` | Open on the Symbols tab. |
| `Cmd-Shift-A` | Open on the Actions tab. |
| `Tab`, `Shift-Tab` | Next or previous tab; the query is kept. |
| `Cmd-Down`, `Cmd-Up` (also `PageDown`, `PageUp`) | Jump to the next or previous section; in a single-section tab, to the last or first row. |

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

The Project Diagnostics tool window sits in the bottom dock, with the title "Project Diagnostics". It hosts the project diagnostics view that used to open as an editor tab.

- Its tool window bar button uses the warning icon (tooltip "Project Diagnostics"). While the panel is not shown and the project has errors, the button shows the error count as a badge; there is no badge at 0 errors.
- Toggle it with `diagnostics: deploy`: `Cmd-Shift-M` on macOS, `Ctrl-Shift-M` on Linux and Windows with the default keymaps, `Cmd-6` (`Alt-6` on Linux) with the JetBrains keymaps, or View → Diagnostics. The first press opens the panel and focuses it; a second press returns focus to the editor. With the JetBrains keymaps `Cmd-6` (`Alt-6`) also hides the dock while the panel has focus.
- Settings: `project_diagnostics_panel.button` (default `true`; "Hide Button" in the button's right-click menu sets it to `false`), `project_diagnostics_panel.dock` (`left`, `right` or `bottom`, default `bottom`), `project_diagnostics_panel.default_width` (default 360, used in a left or right dock) and `project_diagnostics_panel.default_height` (default 320, used in the bottom dock), also under Panels → Project Diagnostics Panel in the settings window. The panel's dock position is restored on restart. The old `diagnostics.button` setting no longer exists.
- Tabs, like IntelliJ's Problems tool window: File, the problems of the file in the active editor (it follows the active editor while the panel is visible), and Project, the project diagnostics view (IntelliJ's "Project Errors"; Flint's view also lists warnings). Each tab shows its problem count in a muted colour: errors, plus warnings while warnings are shown. An empty File tab says "Open file in editor to see problems" or "No problems in <file>" ("Problems could be filtered out" below it while warnings are hidden).
- `diagnostics: toggle file problems`, also a click on the [inspection widget](#inspection-widget), works like IntelliJ's: it opens and focuses the panel on the File tab for the active file; run again for the same file it hides the panel; for another file it switches the File tab to it. `diagnostics: deploy` opens the panel on the last selected tab, Project at first.
- The Project view is created when the Project tab is first shown, so no diagnostics buffers are opened before that; until then the tab shows "Checking diagnostics…".
- Header buttons, for the selected tab: Refresh Diagnostics (it becomes Stop Diagnostics Update while an update runs; shown once the view exists) and Toggle Warnings.
- Compared with the old editor tab, these are gone: the buffer search button and the include-warnings colour state of the pane toolbar, navigation history and the tab itself (title, splitting, per-tab badge). The "Show N warnings" button of the empty state remains. The Toggle Warnings header button cannot show an on or off colour. The panel has no zoom.
- A Flint dock shows one panel at a time, so opening this panel in the bottom dock replaces the visible terminal, Git, Search or Language Services panel.

### Language Services panel

The Language Services tool window sits in the bottom dock, with the title "Language Services".

- Its tool window bar button uses the bolt icon (tooltip "Language Services"). While the panel is not shown, the button shows the number of language servers in an error or warning state as a badge; there is no badge when there are none.
- Toggle it with `lsp_tool: toggle focus`: `Ctrl-Cmd-L` on macOS, `Ctrl-Alt-L` on Linux, `Shift-Alt-L` on Windows, or View → Language Services. The JetBrains keymaps add no key for it and no key that hides the dock while it has focus.
- The panel lists the language servers grouped by worktree and follows the active editor. A restricted-mode banner comes first and opens the worktree security dialog on click. Each server row shows a status dot in the theme's status colours (success for running, modified for starting, error, warning, and disabled for stopped; in Eva Dark green, purple, red, orange and grey), the name, the status, the version and the memory use, and the server message if any. Row buttons are outlined dialog buttons (see [Popups and menus](#popups-and-menus)) with an icon and a label: Message (only with a message), Logs (only when logs exist), Restart and Stop (only when the server can be stopped); their tooltips name the full action. Restart All Servers and Stop All Servers sit side by side in one row above the list, so they stay in view however long it is. The header has only —, like every tool window header. An empty list shows "No language servers".
- Settings: `language_services_panel.button` (default `true`; "Hide Button" in the button's right-click menu sets it to `false`), `language_services_panel.dock` (`left`, `right` or `bottom`, default `bottom`), `language_services_panel.default_width` (default 360, used in a left or right dock) and `language_services_panel.default_height` (default 320, used in the bottom dock), also under Panels → Language Services Panel in the settings window. The old `global_lsp_settings.button` setting no longer exists.
- The per-server actions are row buttons; there are no hover submenus. The panel has no header buttons.
- A Flint dock shows one panel at a time, so opening this panel in the bottom dock replaces the visible terminal, Git, Search or Project Diagnostics panel.

## Merge conflicts

Flint resolves merge conflicts like WebStorm 2026.2.3's [Resolve conflicts](https://www.jetbrains.com/help/webstorm/resolve-conflicts.html) with the iterative flow: a "Conflicts" dialog lists every conflicted file, and each file opens in a three-pane "Merge Revisions" window. Both are separate native windows with their own saved position and size, not editor tabs.

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
- Right-click a row: Accept ‘Yours (…)’, Accept ‘Theirs (…)’, then Revert Conflict Resolution. The right-clicked row becomes the selection unless several rows are already selected, so the menu acts on that file (as in WebStorm's table). Revert is enabled only when every selected file already has merged changes (fully or partially resolved); "Confirm Revert" returns them to their original conflicted state.
- The "Resolve All Simple Conflicts" button (wand icon) applies non-conflicting changes and resolves simple conflicts in every unresolved file, under a cancellable "Resolving simple conflicts…" overlay. A line next to it reports the result: "No conflicts were resolved automatically", "All conflicts were resolved automatically" or "2 conflicts resolved. 3 conflicts in 1 file still require attention". The button is disabled when there is nothing to resolve automatically.
- "Resolve Manually" (or "Review Changes" when every selected file is resolved) and a double-click on a file open the Merge Revisions window. Windows open one after another: the selected files first, then the other unresolved files, then resolved files not yet reviewed. "Save and Close" in a window returns to the dialog; "Apply Changes" goes on to the next file. Binary files cannot be opened ("Cannot Show Merge Dialog: Binary files cannot be merged manually"); neither can read-only files ("Cannot resolve conflicts in a read-only file").
- "Accept and Finish" is enabled once every file is resolved; it closes the dialog and applies the finish rule of the operation from the table above. The default button (accent colour, run by Enter, Ctrl-Enter or Cmd-Enter) is "Accept and Finish" when all files are resolved and reviewed, otherwise Resolve Manually / Review Changes. In that all-reviewed state the line above the table reads "All conflicts have been resolved" with a green check and the review button is hidden.
- "Close" writes and stages the fully resolved files. If some files are only partially resolved it first asks "Discard Changes?" ("Closing the dialog will discard your changes in partially resolved files", buttons "Discard Changes" and "Continue Merge"). Esc, Cmd-W and the window close button close without asking and apply the same rule.
- Speed search: typing selects the next file whose name contains the text (case-insensitive), shown as a chip over the column header at the table's left edge; Backspace edits it and Esc clears it.
- While files are being resolved or written, a progress overlay blocks the table. Confirmations (Discard Changes?, Confirm Revert, Overwrite Changes) are opaque cards inside the dialog with WebStorm's question icon; Enter answers the default (right) button and Esc the other one.

### Editor notification

- A text editor showing a file that is in a git conflict state gets a banner above its content, like WebStorm's editor notification: a warning banner "File has unresolved merge conflicts" with the link "Resolve conflicts…", which opens the standalone Merge Revisions window for that file (not the Conflicts dialog).
- That window is standalone, not part of the iterative flow: its bottom buttons are Accept Left, Accept Right, Cancel and Apply Changes. Cancel leaves the file untouched (after asking when the Result was edited); the other three write the file and stage it.
- While that window is open the banner becomes an information banner "Resolving merge conflicts is in progress" with the links "Show resolve conflicts window" (brings the window to the front) and "Cancel resolve" (closes the window the same way its Cancel button does, so it asks first when the Result was edited).
- The standalone window checks the repository status when it opens and after every change. When the file is no longer in conflict (for example after `git add` or `git checkout` on the command line), the window shows an error banner "Conflict is not valid and no longer can be resolved." below its toolbar with the link "Abort Resolve", which closes the window like Cancel but without asking. That error banner stays even if the conflict comes back.
- The editor banner is not shown when both sides deleted the file, when the file is missing on disk, or in multi-file views such as the project diff. It refreshes when the repository status changes and when the Merge Revisions window opens or closes, and exists only for local projects.

### Merge Revisions window

- Title "Merge Revisions for <path>". Three panes: Left = Yours (read-only), Result (editable, starts from the base revision) and Right = Theirs (read-only). Each pane has a title strip; the side panes say what they contain ("Your version, branch …", "Changes from branch …", "Rebasing <hash> from …", "Local changes", "Changes from stash", …) with a "Show Details" link that opens the commit details and WebStorm's read-only icon. The Result strip shows the file path (tooltip: the home-relative path). When the line separators of the three texts differ, each strip also shows LF, CRLF or CR in WebStorm's colours.
- Ribbons link corresponding changes between the panes. Drag a divider to resize the panes (the divider follows the mouse until the button is released); a double-click on a divider gives the Result the full width, and another one restores equal thirds; the mouse wheel over a divider scrolls the Result. Scrolling is synchronised across the panes by default. Folded unchanged fragments (wavy lines in the editors, the gutter and the dividers) expand when you click the folded line, its placeholder or the chevron in the gutter. "Show Line Numbers" off removes the number column of the gutters. Only the focused pane shows a caret.
- Each pane has WebStorm's diff gutter: line numbers (the caret line's number is brighter), the action icons and the folding chevrons. The Left pane's gutter is mirrored and sits on its right side, next to the divider. The icon column keeps its widest width while the window is open, so the text does not shift when icons disappear.
- Gutter icons on each change: Accept (arrows, which become "append" arrows after the other side was applied to a conflict) and Ignore (cross) on the side panes; in the Result pane a wand (Resolve, for a simple conflict). Ctrl+click on Accept resolves the conflict using that side and ignores the other; Ctrl+click on Ignore ignores the whole conflict; Alt+click does nothing. On macOS a right-click on an icon does the same as Ctrl+click, because the system reports Ctrl+click as a right click.
- Right-click menu in a pane: Accept, Resolve using Left / Right, Ignore, Resolve Automatically (only the entries that apply to the selection), Revert (shown whenever text is selected; as in WebStorm it only reverts changes resolved by AI, which Flint does not have, so it does nothing), then Collapse Unchanged Fragments and Synchronize Scrolling.
- Toolbar, left to right: Previous Difference, Next Difference, Collapse Unchanged Fragments, "Apply non-conflicting changes:" Left / All / Right, Resolve Simple Conflicts, Revert Conflict Resolution. The status shows a spinner while differences are computed, then "2 changes, 1 conflict" or a green "All conflicts resolved". The gear menu has Synchronize Scrolling; Ignore Differences (None, Trim whitespaces, Ignore whitespaces); Highlighting Differences (Lines, Words); and an Appearance submenu with Show Whitespaces, Show Line Numbers, Show Indent Guides and Soft-Wrap. Changing Ignore Differences restarts the merge and first asks "Update Highlighting Settings" when the Result was edited; chosen while the differences are still being computed, it is applied once they are done (never after a failed computation). Folds you expanded stay expanded after the restart. Changing Highlighting only recomputes the word differences.
- When the last change is processed, a green "All changes have been processed" panel with an "Apply Changes" link appears in the Result pane.
- Undo and redo in the Result also undo and redo the accept and ignore states.
- Bottom buttons in the iterative flow: Accept Left and Accept Right take a whole side, "Save and Close" keeps the current Result for the dialog, "Apply Changes" (default) saves the Result. If changes or conflicts remain, Apply Changes first asks "Unprocessed Changes" ("Save Current Result" / "Back to Resolving"). Accept Left, Accept Right and the standalone Cancel ask for confirmation ("Discard Changes and …" / "Continue Merge") when the Result has unsaved edits. Apply Changes is disabled while differences are being computed or if the computation failed. A file whose differences cannot be computed shows a warning banner "Unable to calculate diff. File is too big and there are too many changes." with a "Hide" link. Esc and Cmd-W close the window like Cancel; in the iterative flow they never ask.

### Keyboard shortcuts

These bindings are in the default and the JetBrains macOS keymaps, except Cmd-] and Cmd-[: only the default keymap uses them for Next/Previous Difference. The JetBrains keymap unbinds them in the Merge Revisions window, because WebStorm's macOS keymap uses them for Forward and Back.

| Shortcut | Action | Where |
|---|---|---|
| F7 (Cmd-] in the default keymap) | `merge_tool::NextDifference` | Merge Revisions window, also with an editor focused |
| Shift-F7 (Cmd-[ in the default keymap) | `merge_tool::PreviousDifference` | Merge Revisions window |
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
- The gear menu choices of the Merge Revisions window are remembered across windows and restarts (not per project): Ignore Differences, Highlighting Differences, Show Whitespaces (default off), Show Line Numbers (default on), Show Indent Guides (default off), Soft-Wrap (default off) and Collapse Unchanged Fragments (default off, as in WebStorm). Synchronize Scrolling (default on) is kept for later windows until Flint quits; it is not saved across restarts. A model that the Conflicts dialog already prepared keeps the Ignore Differences policy it was prepared with.

### Not implemented

- Compare Contents popup of the Merge Revisions window (Left / Middle / Right partial diffs and "Base and …") and "Compare with Clipboard" in the editor menu: they need a standalone two-side diff window, which Flint does not have (`MergeThreesideViewer`).
- Binary merge viewer: binary files can only be accepted from one side (`BinaryMergeTool`).
- Semantic conflict resolution (PSI, import merging) and resolution by AI are not ported; they are inert for Git conflicts in WebStorm 2026.2.3.
- Gear menu: Breadcrumbs, Context Help and "Show Diff in Editor Tab / New Window" are missing. The Ignore Differences and Highlighting Differences choices carry check marks instead of radio marks, and the right-click menu has no icons.
- Title strips: no charset label (Flint reads every revision as UTF-8 and rejects other encodings), the line-separator label of the Result is taken from the base text (WebStorm uses the working-tree file), the labels cannot be selected, and there is no read-only tooltip (`DiffEditorTitleDetails`).
- The Left pane keeps its scrollbar on the right because the editor has no left-hand scrollbar.
- Scrollbar stripe marks for changes, the expanded-region chevrons, the enclosing-class hint on fold waves, and the hand cursor on the two panes that are not under the pointer when a fold is hovered. Next/Previous Difference and Apply Non-Conflicting scroll without animation.
- A selection that touches a collapsed fold selects only the caret row for the merge actions. Ctrl+Alt+Shift+Up/Down are not consumed. Unfolding with the editor's own fold commands is not mirrored to the other panes, the number of context lines kept around changes is fixed at WebStorm's default of 4, and Synchronize Scrolling is not paused while Soft-Wrap is toggled.
- Gutter icons act when the mouse button is pressed and released on the icon; WebStorm also acts when only the release is on it. The divider cannot be dragged in the title row.
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
- Operations around the dialog that Flint does not have, so their WebStorm flows are not ported: cherry-pick and revert conflicts (`GitApplyChangesConflictResolver`; Abort Cherry-Pick, Continue Cherry-Pick and Abort Revert are in the Git Branches popup), the "Abort and Rollback" variants of Abort Rebase, the "Rebase stopped for editing" notification as an actionable notice (inside the continue loop it is an information notice without Continue or Abort), "Continue rebase failed" with "Stage and Retry" / "Show Files" after `rebase --continue` finds unstaged changes (Flint reports the remaining conflicts instead), the "View Files" dialog of the "Untracked Files Prevent …" notifications (the files are listed in the notification), the "Cannot commit changes due to unresolved conflicts" commit check (the commit window disables Commit instead), the unstash index-conflict notice, a Git → Resolve Conflicts… menu entry (the action is in the command palette only). Flint also refuses to start a merge while a merge or rebase is unfinished, before running git.
- Smart operations keep Flint's own stash mechanism (WebStorm's `GitPreservingProcess`): the restore step opens the Conflicts dialog with WebStorm's texts, but the stash is restored by `git stash pop` without `--index`. A merge or update that leaves conflicts while changes are stashed shows the WebStorm warning "Local changes were not restored" ("Before merge your uncommitted changes were saved to stash.") with "View saved changes…"; WebStorm restores the stash after the dialog in every case, Flint only once everything is resolved.
- The Git Conflicts tool window, the Changes view banner, the hover icons (Merge, Rollback) of changed files and the non-modal merge in the Changes view: they are off by default in WebStorm (registry keys `git.merge.conflicts.toolwindow` and `vcs.non.modal.merge.enabled`), so they are not built. Revert Resolved Files is not built either; use the Conflicts dialog.

## Languages and code navigation

Cmd+click (and `Cmd-B` with the JetBrains keymap) goes to a definition through the file's language server. Cmd+click or `Cmd-B` on a declaration, `Cmd-Alt-F7` and a click on a usages hint open the [Show Usages popup](#show-usages-popup), and `Alt-F7` lists usages in a multibuffer tab. All of it works once that server runs:

- Built in, no install step: Rust (rust-analyzer from `rustup` or `PATH`, otherwise downloaded), TypeScript and JavaScript (the native TypeScript 7 server `tsc --lsp`, a Go binary downloaded from npm on first use that runs without Node; see [Language server memory](#language-server-memory) for its memory limit), ESLint, oxlint (the project's own `node_modules/oxlint/bin/oxlint --lsp`, found like Node resolves packages), Biome (the project's own `node_modules/@biomejs/biome/bin/biome lsp-proxy`, or a `biome` on `PATH`), JSON, YAML. C, CSS, Go, Python and Bash keep syntax highlighting (Python also keeps its toolchain selection) but have no language server. C++ is not supported.
- TypeScript and JavaScript diagnostics arrive through LSP pull diagnostics (`diagnostics.lsp_pull_diagnostics.enabled`, on by default), so turning that setting off hides them. If you point `lsp.tsgo.binary.path` at your own build, also set `"arguments": ["--lsp", "--stdio"]`, because a binary override replaces the default arguments.
- Servers that only make sense for some projects start only when the project uses them. `eslint` needs an `eslint.config.{js,cjs,mjs,ts,cts,mts}` or `.eslintrc*` file, or `eslint` in the `dependencies`, `devDependencies`, `peerDependencies` or `optionalDependencies` of a non-ignored `package.json`. `oxlint` needs `.oxlintrc.json`, `.oxlintrc.jsonc`, `oxlint.config.ts` or `oxlint.config.mts` (the configurations oxlint finds itself) or an `oxlint` dependency. These Node tools also count a configuration or dependency in the directories above the project up to the repository root, so a package of a monorepo opened as its own project uses the repository's ESLint, oxlint and Biome. `biome` needs `biome.json`, `biome.jsonc`, `.biome.json`, `.biome.jsonc` or an `@biomejs/biome` dependency. `roslyn` needs a `.sln`, `.slnx` or `.csproj`. `jdtls` needs `pom.xml`, `build.gradle(.kts)`, `settings.gradle(.kts)`, `.project` or `.classpath`, and `gradle-language-server` needs a Gradle file. `marksman` needs a `.marksman.toml`, `.obsidian`, `mkdocs.yml`, `mkdocs.yaml` or `book.toml` entry. Listing a server by name in `language_servers` starts it regardless of these rules.
- `lsp.oxlint.settings` answers oxlint's `oxc_language_server` request with oxlint's options (`run`, `configPath`, `tsConfigPath`, `typeAware`, `fixKind`, `disableNestedConfig`, `unusedDisableDirectives`, `rulesCustomization`). `lsp.biome.settings` answers Biome's `biome` request with `requireConfiguration`, `configurationPath`, `inlineConfig` and `goToDefinition`; the `require_config_file`, `config_path`, `inline_config` and `goto_definition` names documented by Biome's Zed extension are accepted too. Without `configurationPath`, Biome finds its configuration itself, walking up from the project folder past nested `"root": false` files to the monorepo's root `biome.json`. In a project that uses Biome and does not install Prettier, `"formatter": "auto"` formats with Biome instead of Flint's bundled Prettier; a project that installs Prettier keeps formatting with it.
- Java and C# come as extensions that Flint installs on its own, like IntelliJ's bundled Java and .NET support. Opening a project with a `pom.xml`, `build.gradle(.kts)` or `settings.gradle(.kts)` installs Java (`java`: the Java, Gradle and `.properties` languages and `jdtls`). A `.sln`, `.slnx` or `.csproj` installs C# (`csharp`: C#, `.csproj` and `.sln` files and Roslyn). Nested build files count too, so a monorepo with a Java service and a .NET API gets both. Flint downloads the newest version it can load from the Zed extension registry, once, with no Extensions marketplace page; its servers then start for the project's files. To never install one, set it to `false`: `"auto_install_extensions": { "csharp": false }`. Any other extension can still be installed from a local folder with Install Dev Extension on the Extensions page.
- C# needs a .NET runtime. If .NET is installed only in `~/.dotnet` and `DOTNET_ROOT` is not set, Flint passes `DOTNET_ROOT=~/.dotnet` to language servers, so Roslyn starts without extra setup. A `DOTNET_ROOT` or `DOTNET_ROOT_ARM64` you set yourself, in the shell or in `lsp.<server>.binary.env`, always wins.
- Each server still needs its project files to resolve symbols: `Cargo.toml` for Rust, a `.csproj` or `.sln` for C#, and a JDK for Java. `jdtls` runs on Java 21 or newer from `JAVA_HOME` or the `PATH`, or the one in `lsp.jdtls.settings.java_home`.

### Language servers with several projects

- Like IntelliJ, every project owns its language servers. Each project in a window, and each git worktree (a project of its own), starts its own `tsgo`, `eslint` and other servers, rooted at that project with its own `node_modules`, settings and trust. No server is shared between projects.
- A project in a background tab keeps its servers running and keeps receiving their problems. Closing its tab stops its servers and leaves the other projects' servers running.
- Inside one project, every folder added to it starts its own servers too. Sharing one `tsgo` or `eslint` process between the folders of one project, as IntelliJ's project-wide servers do, is not implemented.

### Language server memory

- `jdtls` starts with `-Xms100m` and may grow to 2 GB (`lsp.jdtls.settings.min_memory` and `max_memory`), and `JDK_JAVA_OPTIONS` in `lsp.jdtls.binary.env` switches it to the parallel collector, which gives freed memory back to the system. These are the VS Code Java extension's defaults. On a small Maven project this took jdtls from about 1.3 GB to about 0.8 GB, with the first errors still arriving in about 10 seconds. Raise `max_memory` for very large projects; replace `JDK_JAVA_OPTIONS` to use other JVM flags.
- Roslyn ships configured for the server garbage collector, which keeps a heap per CPU core. Flint starts it with `DOTNET_gcServer=0` and `DOTNET_GCConserveMemory=7` (in `lsp.roslyn.binary.env`), the workstation collector compacting harder. On a small console project Roslyn kept about 375 MB instead of 422 MB, with the first errors arriving as fast. Most of that wait is MSBuild loading the project.

- `tsrs` ([oxc-project/tsrs](https://github.com/oxc-project/tsrs)), a Rust port of `tsgo` that its authors measure at about 70% less memory, can run instead of `tsgo`. It speaks the same protocol and takes the same settings (under `lsp.tsrs` instead of `lsp.tsgo`), and Flint installs it from npm like `tsgo`; it has a build for Apple silicon only. It is experimental (0.18 as of October 2026, about 90% of the TypeScript language service tests pass), so `tsgo` stays the default. Compared over LSP on the tanstack-router sources (646 files, October 2026), it matched `tsgo` on hover, go to definition and completions, never crashed and used 2–2.6 times less memory, but its diagnostics still depend on checking order (extra TS2536 errors in `router-core/src/link.ts` when files open in reverse or are checked concurrently, [tsrs#725](https://github.com/oxc-project/tsrs/issues/725)) and it misses two circularity errors (TS7022, TS7023) that `tsgo` reports. To switch, per language:

```json
"languages": {
  "TypeScript": { "language_servers": ["tsrs", "!tsgo", "..."] },
  "TSX": { "language_servers": ["tsrs", "!tsgo", "..."] },
  "JavaScript": { "language_servers": ["tsrs", "!tsgo", "..."] }
}
```

- `tsgo` is a Go program, and Go keeps freed memory until a soft limit asks it to give it back. Flint starts `tsgo` with `GOMEMLIMIT` set to a quarter of the Mac's memory, at least 2 GiB and at most 8 GiB (4096 MiB on a 16 GB Mac). Near the limit the server collects garbage more often and can get slower, but it is never stopped, so a limit below what the project needs only costs CPU. The default is not set for your own build in `lsp.tsgo.binary.path`, nor when your shell environment already exports `GOMEMLIMIT` (for example in `~/.zshrc` or a direnv `.envrc`). Your own value in `lsp.tsgo.binary.env` wins over both:

  ```json
  "lsp": { "tsgo": { "binary": { "env": { "GOMEMLIMIT": "6GiB" } } } }
  ```

- Most of `tsgo`'s memory goes to the TypeScript programs it loads and to its auto-import cache. Settings under `lsp.tsgo.settings["js/ts"]` reach the server. `preferences.autoImportFileExcludePatterns` keeps large packages out of the auto-import cache. `suggest.autoImports: false` turns the cache off, and auto-import completions with it:

  ```json
  "lsp": {
    "tsgo": {
      "settings": {
        "js/ts": {
          "preferences": { "autoImportFileExcludePatterns": ["**/node_modules/@aws-sdk/**"] },
          "suggest": { "autoImports": false }
        }
      }
    }
  }
  ```

- In a monorepo with project references, set `"disableReferencedProjectLoad": true` and `"disableSolutionSearching": true` in the `compilerOptions` of `tsconfig.json`. `tsgo` then loads a referenced project only when one of its files is opened.
- `tsgo` reads the files its `tsconfig.json` includes from disk itself. `excluded_directories` and "Mark Directory As > Excluded" therefore do not shrink its programs; narrow `include` or add `exclude` in `tsconfig.json` instead.
- Each usages hint is a find-all-references in the server. After an edit, the hints are resolved again only once typing pauses for 300 ms (IntelliJ's default autoreparse delay), and a hint request for text that has changed since is cancelled. Until then each hint keeps its previous count.
- Show Usages opens the files of the usages it lists only in the servers that report read and write access (document highlights), not in linters such as ESLint.
- ESLint runs on Node with `--max-old-space-size=8192`. `lsp.eslint.settings.execArgv`, like VS Code's `eslint.execArgv`, replaces these Node arguments: `["--max-old-space-size=4096"]` caps the heap lower, and `[]` leaves Node at its own default. Restart the server after changing it.
- `"run": "onSave"` in `lsp.eslint.settings`, like VS Code's `eslint.run`, lints only saved text: when a file opens and when it is saved, not while you type. The default `"onType"` lints while you type.
- Typed linting (`parserOptions.project` or `projectService` in the ESLint config) builds a second TypeScript program inside the ESLint process, separate from `tsgo`'s.
- While you type, ESLint lints a file once typing pauses for 300 ms, like IntelliJ's reanalysis delay, because every pull lints the whole file again, typed rules included; the TypeScript server's problems still update after each edit.
- ESLint reloads its configuration when an `.eslintrc*`, `eslint.config.*`, `.eslintignore` or `package.json` file changes. These are the files VS Code's ESLint client watches: the server registers no watchers of its own, so Flint registers them for it.

## Memory and startup

- The Performance Profiler window is removed, so the profiler's trace buffers are never collected.
- Extensions are compiled to native code once and cached in the `.compiled-components` folder inside the extensions work folder, so later launches skip the compilation. Delete the folder to rebuild the cache; a changed or incompatible extension is recompiled by itself.
- Bundled fonts, themes and settings are embedded uncompressed, so they stay in file-backed memory instead of being unpacked into the heap at launch.

## Checking the UI and performance

Flint can run a script of UI steps in the real app and save screenshots of its windows as the window server composites them, native glass included. An app may always capture its own windows, so this needs no Screen Recording permission, and it works while the window is covered: during a script Flint draws frames as a visible window would and keeps macOS from putting it into App Nap, which would move it to slower cores.

```sh
FLINT_UI_SCRIPT=script/ui-scripts/surfaces.json FLINT_UI_SCRIPT_OUTPUT=target/ui-script \
    target/debug/flint --user-data-dir target/ui-script/data script/ui-scripts/fixture
```

- The script is JSON with a `steps` array. Steps: `{"wait": {"milliseconds": 500}}`; `{"action": {"name": "command_palette::Toggle", "arguments": null}}` dispatches an action as a key binding would; `{"keystrokes": {"keys": "cmd-shift-p escape"}}`; `{"open": {"path": "src/main.rs"}}`, relative to the first project folder; `{"click": {"x": 600, "y": 300, "right": true}}` and `{"hover": {"x": 415, "y": 240}}`, in logical pixels from the window's top left; `{"capture": {"name": "palette"}}` writes `palette.png` (`palette-1.png`, `palette-2.png`… with several windows); input steps and `capture` take `"frames": n`, the frames drawn afterwards (3 by default), so an input with `"frames": 0` followed by captures with `"frames": 1` records a transition frame by frame; `{"repeat": {"times": 100, "steps": [...]}}`; `{"memory": {"label": "x"}}` logs the physical memory footprint; `{"frame_times": {"label": "x"}}` logs how long the window's frames took to draw since the previous `frame_times` step; `"quit"`. The app quits when the script ends, and a failed step is written to `error.txt` in the output directory.
- `script/ui-scripts/surfaces.json` opens a menu, its submenu, the command palette and the file finder over `script/ui-scripts/fixture/dense.rs`, whose colourful lines show how the glass refracts. `script/ui-scripts/leak-check.json` opens and closes them 200 times with memory checkpoints, then waits 30 s so `leaks <pid>`, `footprint <pid>` or `vmmap --summary <pid>` can look at the process. Use a release build (`--release` or `--profile release-fast`) for timings, without `MallocStackLogging`: it records a stack for every allocation and slows frames down several times.
- Memory is judged by growth between the checkpoints. `leaks` reports every large Rust `HashMap` as a leak, because hashbrown keeps a pointer into the middle of its table, so its leaked-bytes total overstates real leaks; the stacks under `MallocStackLogging=1` tell the two apart.

## Licensing

Zed source code is licensed under GPL-3.0-or-later, with Apache-2.0 components where marked; see [LICENSE-GPL](LICENSE-GPL) and [LICENSE-APACHE](LICENSE-APACHE).

Third-party licenses are generated by [`cargo-about`](https://github.com/EmbarkStudios/cargo-about) from [`script/licenses/zed-licenses.toml`](script/licenses/zed-licenses.toml) during bundling and embedded in the app. The build fails if a dependency's license is not in the accepted list.

The `assets/icons/tool_window_*.svg` files, `assets/icons/locate.svg`, `assets/icons/expand_all.svg` and `assets/icons/collapse_all.svg` are unmodified copies of [JetBrains/intellij-community](https://github.com/JetBrains/intellij-community) icons (commit `0ad391f70a62eabbec6db656f473e1284e1110a9`), licensed under Apache-2.0, each keeping its original JetBrains copyright header. Sources: `platform/icons/src/expui/toolwindows/{project,vcs,find,problems,services}.svg`, `platform/icons/src/expui/general/moreHorizontal.svg` and `plugins/terminal/resources/icons/expui/toolwindow/terminal.svg`; the 20×20 drawing is the `@20x20` file and the 16×16 drawing is the file without the suffix. `locate.svg`, `expand_all.svg` and `collapse_all.svg` are `platform/icons/src/expui/general/{locate,expandAll,collapseAll}.svg` under snake_case names. JetBrains and IntelliJ names and logos are trademarks of JetBrains s.r.o. The default colours in `theme_overrides."Eva Dark"` are taken from `platform/platform-resources/src/themes/islands/ManyIslandsDark.theme.json` of the same commit, Apache-2.0.

The Git Branches popup icons `assets/icons/{fetch,current_branch_label,current_branch_favorite_label,tag_label,branch_node,incoming_commits,outgoing_commits,favorite_outline,menu_arrow}.svg` are unmodified copies of [JetBrains/intellij-community](https://github.com/JetBrains/intellij-community) icons (commit `0ad391f70a62eabbec6db656f473e1284e1110a9`), licensed under Apache-2.0, each keeping its original JetBrains copyright header. Sources, in the order above: `platform/icons/src/expui/vcs/fetch.svg`, `platform/dvcs-impl/shared/resources/icons/new/{currentBranchLabel,currentBranchFavoriteLabel,branchLabel}.svg`, `platform/icons/src/vcs/branchNode.svg`, `platform/dvcs-impl/shared/resources/icons/new/{incomingUpdate,outgoingPush}.svg`, `platform/icons/src/nodes/notFavoriteOnHover.svg` and `platform/icons/src/icons/ide/menuArrow.svg`.

The inspection widget and editor-with-preview icons `assets/icons/{inspections_ok,inspections_ok_empty,inspections_error,inspections_error_empty,inspections_warning,find_and_show_prev_matches,find_and_show_next_matches,layout_editor_only,layout_editor_preview,editor_preview_vertical,layout_preview_only}.svg` are unmodified copies of [JetBrains/intellij-community](https://github.com/JetBrains/intellij-community) icons (commit `ace3ff1afc7b514bf4e69d9c83adc7adf736f781`), licensed under Apache-2.0, each keeping its original JetBrains copyright header. Sources, in the order above: `platform/icons/src/expui/general/inspections/{inspectionsOK,inspectionsOKEmpty,inspectionsError,inspectionsErrorEmpty,inspectionsWarning}.svg` and `platform/icons/src/expui/general/{chevronUp,chevronDown,editorOnly,editorPreview,editorPreviewVertical,previewOnly}.svg`.

The Git tool window icons `assets/icons/{arrow_collapse,arrow_expand,chevron_down_large,cherry_pick,close_small,close_small_hovered,general_add,general_delete,general_history,general_open_new_tab,general_refresh,general_scroll_down,general_search,general_soft_wrap,inline_match_case,inline_regex,intelli_sort,nodes_folder,nodes_star,vcs_changes,vcs_update}.svg` are unmodified copies of [JetBrains/intellij-community](https://github.com/JetBrains/intellij-community) icons (commit `ace3ff1afc7b514bf4e69d9c83adc7adf736f781`), licensed under Apache-2.0, each keeping its original JetBrains copyright header. Sources, in the order above: `platform/icons/src/expui/general/{chevronLeft,chevronRight,chevronDownLarge}.svg`, `platform/dvcs-impl/shared/resources/icons/new/cherryPick.svg`, `platform/icons/src/expui/general/{closeSmall,closeSmallHovered,add,delete,history,openNewTab,refresh,scrollDown,search,softWrap}.svg`, `platform/icons/src/expui/inline/{matchCase,regex}.svg`, `platform/vcs-log/impl/resources/icons/new/intelliSort.svg`, `platform/icons/src/expui/nodes/{folder,star}.svg` and `platform/icons/src/expui/vcs/{changes,update}.svg`.

The commit graph of the Git tool window (`crates/git_ui/src/git_log/graph.rs` and `crates/git_ui/src/git_log/graph/`) is a Rust port of the IntelliJ Platform VCS Log graph code from [JetBrains/intellij-community](https://github.com/JetBrains/intellij-community) (commit `ace3ff1afc7b514bf4e69d9c83adc7adf736f781`, `platform/vcs-log/graph`, `platform/vcs-log/graph-api` and `platform/vcs-log/impl/src/com/intellij/vcs/log/{graph,paint}`), licensed under Apache-2.0.

The Show Usages popup (`crates/show_usages`) is a Rust port of `ShowUsagesAction`, `ShowUsagesTable`, `ShowUsagesTableCellRenderer` and `ShowUsagesHeader` from [JetBrains/intellij-community](https://github.com/JetBrains/intellij-community) (commit `7abdf9b3f062d0b4fe954587e124d6d587241621`, `platform/lang-impl/src/com/intellij/find/actions`), licensed under Apache-2.0. Its icons `assets/icons/{usages_import_statements,usages_comments,usages_show_options,usages_preview,usages_open_in_find_window}.svg` are unmodified copies of `platform/icons/src/expui/actions/showImportStatements.svg`, `platform/icons/src/expui/codeInsight/inlayRenameInComments.svg` and `platform/icons/src/expui/general/{show,previewVertically,openInToolWindow}.svg` of the same commit. `assets/icons/{usages_read_access,usages_write_access,usage_read_access,usage_write_access}.svg` and their `_accent` twins split `platform/icons/src/expui/actions/{showReadAccess,showWriteAccess}.svg` and `platform/icons/src/expui/gutter/{readAccess,writeAccess}.svg` into the grey shape and the coloured shape, because Flint tints an icon with a single colour; the shapes themselves are unchanged. Each icon keeps its original JetBrains copyright header and is licensed under Apache-2.0.

The code author hint ports `getCodeAuthorInfo` and the author texts of `platform/vcs-impl/src/com/intellij/codeInsight/hints/VcsCodeVisionProvider.kt` (tag `idea/243.22562.145`) and the `label.new.code` / `label.*.author.*` strings of `VcsBundle.properties` from the same repository, licensed under Apache-2.0. Its icon `assets/icons/vcs_author.svg` is the outline layer of `platform/icons/src/expui/general/user.svg` (the new-UI icon of `AllIcons.Vcs.Author`), without the fill layer because Flint tints an icon with a single colour; it keeps the original JetBrains copyright header and is licensed under Apache-2.0.
