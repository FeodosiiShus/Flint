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

The app is built on the `dev` release channel and is named "Flint". Dev builds never check for updates, so it never auto-updates to official Zed.

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
- Every surface repaints the image right after its own opaque background, so it stays visible in opaque themes such as One Dark. In translucent themes such as the default Eva Dark, translucent panels are skipped and the picture is drawn once under them, never twice. With both images set in a translucent theme, the `editor_and_tools` image also shows through an empty pane, mixed with the `empty_frame` image.
- Pop-ups, menus, modals, notifications and the Settings window are not covered.
- Images larger than 8192 pixels on the longest side are scaled down when loaded. If you edit the image file in place, Flint reloads it when its window becomes active again.
- If the file is missing, unreadable, not an image, or the path is relative, Flint shows an error notification once and removes the image. It tries again when the setting changes or the window becomes active again.
- Images from a URL are not supported.

## Toolbar and icon sizes

Flint can make the title bar, the tab bar, the toolbar and the status bar taller and their icons bigger, closer to WebStorm. Each bar has two keys, `height` and `icon_size`, eight keys in total:

```json
"title_bar":  { "height": null, "icon_size": null }
"tab_bar":    { "height": null, "icon_size": null }
"toolbar":    { "height": null, "icon_size": null }
"status_bar": { "height": null, "icon_size": null }
```

- The default `null` keeps Zed's built-in sizes. The keys are read only from your user `settings.json`, not from a project's `.zed/settings.json`.
- Values are logical pixels (points on macOS). They are not scaled by `ui_font_size`.
- `height` accepts 24–64 and `icon_size` accepts 10–32. Values outside these ranges are clamped.

### What each key does

- `title_bar.height` — the exact height of the title bar, but never lower than its buttons. With only `title_bar.icon_size` set, the bar grows when needed to keep at least 4px above and below the buttons. On macOS the window buttons (traffic lights) are re-centered vertically.
- `tab_bar.height` — the exact height of the editor tab bar, but never lower than its buttons. Panel headers that use the tab bar height (Git, Outline, Debugger, Agent, …) follow it.
- `toolbar.height`, `status_bar.height` — the minimum height of the bar. The content is centered vertically, and the bar is never smaller than its content.
- `icon_size` — the size of the primary icons of the bar (Zed's default is 14px). Secondary icons scale proportionally, so a 12px chevron becomes 12·N/14 for `icon_size` N. Buttons grow to at least the icon size plus 8px, and the bars grow so nothing is clipped. Text keeps the UI font size.

### Setting the sizes

- Settings window: Window & Layout → Status Bar, Title Bar and Tab Bar; Editor → Toolbar. An unset key shows 0 there. Any number you enter is written to `settings.json` and clamped to the range above; the field's reset button removes the key again.
- `settings.json`:

```json
{
  "status_bar": { "height": 36, "icon_size": 20 },
  "tab_bar": { "height": 40 },
  "title_bar": { "height": 40 },
  "toolbar": { "icon_size": 18 }
}
```

### WebStorm equivalents

| Key | WebStorm area |
|---|---|
| `title_bar` | main toolbar / window header |
| `tab_bar` | editor tabs |
| `toolbar` | breadcrumbs / navigation bar above the editor |
| `status_bar` | status bar plus the tool window buttons |

### Relationship to `unstable.ui_density`

- `unstable.ui_density` changes only the dynamic spacing paddings: a padding of N px becomes N − 4 px (at least 0) in `compact` and N + 4 px in `comfortable`, except for a few paddings with their own per-density values. The default tab bar height is derived from these paddings, so it changes with density too.
- An explicit `height` overrides the density-derived default for its bar.
- Icon sizes are not affected by density.

## Licensing

Zed source code is licensed under GPL-3.0-or-later, with Apache-2.0 components where marked; see [LICENSE-GPL](LICENSE-GPL) and [LICENSE-APACHE](LICENSE-APACHE).

Third-party licenses are generated by [`cargo-about`](https://github.com/EmbarkStudios/cargo-about) from [`script/licenses/zed-licenses.toml`](script/licenses/zed-licenses.toml) during bundling and embedded in the app. The build fails if a dependency's license is not in the accepted list.
