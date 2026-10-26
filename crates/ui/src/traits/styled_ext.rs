use gpui::{App, BoxShadow, Hsla, Styled, Window, hsla, px};
use std::sync::Arc;

use crate::prelude::*;
use crate::{DialogWidth, ElevationIndex, POPUP_SURFACE_METRICS, SurfaceKind, SurfaceMaterial};
use settings::Settings as _;
use theme_settings::GlassSettings;

/// How far below the window top the modal layer places a centred dialog (`top_20`).
pub const DIALOG_TOP_OFFSET: Pixels = px(80.);

/// Whether a surface of `kind` is drawn as Liquid Glass right now: the user can ask macOS for
/// opaque surfaces with Reduce Transparency.
pub fn surface_is_glass(kind: SurfaceKind, cx: &App) -> bool {
    kind.standard().material == SurfaceMaterial::Glass && !cx.should_reduce_transparency()
}

/// Whether the island of a tool window is drawn as Liquid Glass right now: its panel has to ask
/// for glass, and the user can ask macOS for opaque surfaces with Reduce Transparency.
pub fn tool_window_is_glass(panel_wants_glass: bool, cx: &App) -> bool {
    panel_wants_glass && !cx.should_reduce_transparency()
}

/// Fills `this` with the Liquid Glass material: a translucent surface over its blurred backdrop.
/// Popups, dialogs, balloons and tool window islands all take it through [`glass_surface`], so
/// their glass looks the same.
///
/// Flint draws the blur and the tint itself, so every Mac shows the same glass. With
/// `FLINT_NATIVE_GLASS` on macOS 26 the system's `NSGlassEffectView` draws both the blur and the
/// lighting, so the element only hands it the theme tint and paints no fill of its own; a fill
/// would sit above the material and flatten it.
///
/// [`GlassSettings`] sets how much of the theme's colour covers the glass.
fn glass_material<E: Styled>(this: E, cx: &App) -> E {
    let glass = GlassSettings::get_global(cx);
    let native = cx.supports_native_glass();
    let tint = cx
        .theme()
        .colors()
        .elevated_surface_background
        .opacity(glass.tint(native));
    if native {
        this.backdrop_glass(glass.blur_radius, tint)
    } else {
        this.bg(tint)
            .backdrop_blur(glass.blur_radius)
            .backdrop_lens(glass.refraction)
    }
}

/// The colours content fills its areas with: a component is written to paint its panel, its
/// editor or its tab strip with one of the theme's surface colours, which on glass would cover
/// what the glass blurs with a flat panel.
fn surface_fills(cx: &App) -> Arc<[Hsla]> {
    let colors = cx.theme().colors();
    Arc::from([
        colors.background,
        colors.surface_background,
        colors.elevated_surface_background,
        colors.panel_background,
        colors.editor_background,
        colors.editor_subheader_background,
        colors.tab_bar_background,
        colors.toolbar_background,
        colors.terminal_background,
        colors.terminal_ansi_background,
        colors.title_bar_background,
        colors.status_bar_background,
    ])
}

/// The glass every glass surface shares: [`glass_material`] with the shadows of the surface and
/// the specular rim along the top edge of Liquid Glass, which the system's own glass draws itself.
///
/// Everything painted inside leaves the theme's surface colours transparent
/// ([`Styled::glass_fills`]), so no panel, placeholder or editor has to know it lies on glass; a
/// fill that has to hide what lies behind it says so with [`Styled::keep_glass_fill`].
fn glass_surface<E: Styled>(this: E, mut shadows: Vec<BoxShadow>, cx: &App) -> E {
    if !cx.supports_native_glass() {
        shadows.push(BoxShadow::new(px(0.), px(1.), hsla(0., 0., 1., 0.06)).inset());
    }
    glass_material(this, cx)
        .shadow(shadows)
        .glass_fills(surface_fills(cx))
}

/// The fill of an area inside a panel or a popup: none on a glass surface
/// ([`Window::on_glass_surface`]), so its empty areas show the glass instead of covering it.
pub fn content_background(color: Hsla, window: &Window) -> Hsla {
    if window.on_glass_surface() {
        gpui::transparent_black()
    } else {
        color
    }
}

fn surface<E: Styled + InteractiveElement>(this: E, kind: SurfaceKind, cx: &App) -> E {
    let standard = kind.standard();
    let colors = cx.theme().colors();
    let this = this
        .debug_selector(|| kind.debug_selector().to_string())
        .rounded(standard.corner_radius)
        .border(standard.border_width)
        .border_color(colors.border_variant);
    if surface_is_glass(kind, cx) {
        glass_surface(this, standard.elevation.shadow(cx), cx)
    } else {
        this.bg(colors.elevated_surface_background)
            .shadow(standard.elevation.shadow(cx))
    }
}

fn elevated<E: Styled>(this: E, cx: &App, index: ElevationIndex) -> E {
    this.bg(cx.theme().colors().elevated_surface_background)
        .rounded(POPUP_SURFACE_METRICS.corner_radius)
        .border(POPUP_SURFACE_METRICS.border_width)
        .border_color(cx.theme().colors().border_variant)
        .shadow(index.shadow(cx))
}

fn elevated_borderless<E: Styled>(this: E, cx: &mut App, index: ElevationIndex) -> E {
    this.bg(cx.theme().colors().elevated_surface_background)
        .rounded(POPUP_SURFACE_METRICS.corner_radius)
        .shadow(index.shadow(cx))
}

/// The standard chrome of every floating surface. Each method also tags the element with
/// [`SurfaceKind::debug_selector`], so tests can find every surface a window paints.
pub trait StyledSurface: Styled + InteractiveElement + Sized {
    /// [`SurfaceKind::Popup`]: menus, pickers, popovers and the branches popup.
    fn popup_surface(self, cx: &App) -> Self {
        surface(self, SurfaceKind::Popup, cx)
    }

    /// [`SurfaceKind::Dialog`] at one of the standard widths, shrunk to keep its margin from the
    /// window edges.
    fn dialog_surface(self, width: DialogWidth, window: &Window, cx: &App) -> Self {
        let viewport = window.viewport_size();
        let margin = SurfaceKind::Dialog.standard().viewport_margin;
        let fitted = SurfaceKind::Dialog.fit_to_viewport(
            gpui::size(
                width.pixels(window.rem_size()),
                (viewport.height - DIALOG_TOP_OFFSET - margin).max(px(0.)),
            ),
            viewport,
        );
        surface(self, SurfaceKind::Dialog, cx)
            .w(fitted.width)
            .max_h(fitted.height)
            .overflow_hidden()
    }

    /// [`SurfaceKind::Balloon`]: notifications.
    fn balloon_surface(self, cx: &App) -> Self {
        surface(self, SurfaceKind::Balloon, cx)
    }

    /// [`SurfaceKind::Tooltip`].
    fn tooltip_surface(self, cx: &App) -> Self {
        surface(self, SurfaceKind::Tooltip, cx)
    }

    /// The island of a tool window with `corner_radius`, as Liquid Glass with the popups' material
    /// and rim, and an inner hairline where popups have their border. The hairline is a shadow,
    /// so it takes no room from the IntelliJ metrics of the content. The island sits in the
    /// window's frame rather than floating over it, so it casts no shadow. Call it only where
    /// [`tool_window_is_glass`].
    fn tool_window_surface(self, corner_radius: Pixels, cx: &App) -> Self {
        let hairline = BoxShadow::new(px(0.), px(0.), cx.theme().colors().border_variant)
            .spread_radius(SurfaceKind::Popup.standard().border_width)
            .inset();
        glass_surface(self.rounded(corner_radius), vec![hairline], cx)
    }
}

impl<E: Styled + InteractiveElement> StyledSurface for E {}

/// Extends [`gpui::Styled`] with Zed-specific styling methods.
// gate on rust-analyzer so rust-analyzer never needs to expand this macro, it takes up to 10 seconds to expand due to inefficiencies in rust-analyzers proc-macro srv
#[cfg_attr(
    all(debug_assertions, not(rust_analyzer)),
    gpui_macros::derive_inspector_reflection
)]
pub trait StyledExt: Styled + Sized {
    /// Horizontally stacks elements.
    ///
    /// Sets `flex()`, `flex_row()`, `items_center()`
    fn h_flex(self) -> Self {
        self.flex().flex_row().items_center()
    }

    /// Vertically stacks elements.
    ///
    /// Sets `flex()`, `flex_col()`
    fn v_flex(self) -> Self {
        self.flex().flex_col()
    }

    /// The [`Surface`](ElevationIndex::Surface) elevation level, located above the app background, is the standard level for all elements
    ///
    /// Sets `bg()`, `rounded_lg()`, `border()`, `border_color()`, `shadow()`
    ///
    /// Example Elements: Title Bar, Panel, Tab Bar, Editor
    fn elevation_1(self, cx: &App) -> Self {
        elevated(self, cx, ElevationIndex::Surface)
    }

    /// See [`elevation_1`](Self::elevation_1).
    ///
    /// Renders a borderless version [`elevation_1`](Self::elevation_1).
    fn elevation_1_borderless(self, cx: &mut App) -> Self {
        elevated_borderless(self, cx, ElevationIndex::Surface)
    }

    /// Non-Modal Elevated Surfaces appear above the [`Surface`](ElevationIndex::Surface) layer and is used for things that should appear above most UI elements like an editor or panel, but not elements like popovers, context menus, modals, etc.
    ///
    /// Sets `bg()`, `rounded_lg()`, `border()`, `border_color()`, `shadow()`
    ///
    /// Examples: Notifications, Palettes, Detached/Floating Windows, Detached/Floating Panels
    fn elevation_2(self, cx: &App) -> Self {
        elevated(self, cx, ElevationIndex::ElevatedSurface)
    }

    /// See [`elevation_2`](Self::elevation_2).
    ///
    /// Renders a borderless version [`elevation_2`](Self::elevation_2).
    fn elevation_2_borderless(self, cx: &mut App) -> Self {
        elevated_borderless(self, cx, ElevationIndex::ElevatedSurface)
    }

    /// Modal Surfaces are used for elements that should appear above all other UI elements and are located above the wash layer. This is the maximum elevation at which UI elements can be rendered in their default state.
    ///
    /// Elements rendered at this layer should have an enforced behavior: Any interaction outside of the modal will either dismiss the modal or prompt an action (Save your progress, etc) then dismiss the modal.
    ///
    /// If the element does not have this behavior, it should be rendered at the [`Elevated Surface`](ElevationIndex::ElevatedSurface) layer.
    ///
    /// Sets `bg()`, `rounded_lg()`, `border()`, `border_color()`, `shadow()`
    ///
    /// Examples: Settings Modal, Channel Management, Wizards/Setup UI, Dialogs
    fn elevation_3(self, cx: &App) -> Self {
        elevated(self, cx, ElevationIndex::ModalSurface)
    }

    /// See [`elevation_3`](Self::elevation_3).
    ///
    /// Renders a borderless version [`elevation_3`](Self::elevation_3).
    fn elevation_3_borderless(self, cx: &mut App) -> Self {
        elevated_borderless(self, cx, ElevationIndex::ModalSurface)
    }

    /// The theme's primary border color.
    fn border_primary(self, cx: &mut App) -> Self {
        self.border_color(cx.theme().colors().border)
    }

    /// The theme's secondary or muted border color.
    fn border_muted(self, cx: &mut App) -> Self {
        self.border_color(cx.theme().colors().border_variant)
    }

    /// Sets the background color to red for debugging when building UI.
    fn debug_bg_red(self) -> Self {
        self.bg(hsla(0. / 360., 1., 0.5, 1.))
    }

    /// Sets the background color to green for debugging when building UI.
    fn debug_bg_green(self) -> Self {
        self.bg(hsla(120. / 360., 1., 0.5, 1.))
    }

    /// Sets the background color to blue for debugging when building UI.
    fn debug_bg_blue(self) -> Self {
        self.bg(hsla(240. / 360., 1., 0.5, 1.))
    }

    /// Sets the background color to yellow for debugging when building UI.
    fn debug_bg_yellow(self) -> Self {
        self.bg(hsla(60. / 360., 1., 0.5, 1.))
    }

    /// Sets the background color to cyan for debugging when building UI.
    fn debug_bg_cyan(self) -> Self {
        self.bg(hsla(160. / 360., 1., 0.5, 1.))
    }

    /// Sets the background color to magenta for debugging when building UI.
    fn debug_bg_magenta(self) -> Self {
        self.bg(hsla(300. / 360., 1., 0.5, 1.))
    }
}

impl<E: Styled> StyledExt for E {}
