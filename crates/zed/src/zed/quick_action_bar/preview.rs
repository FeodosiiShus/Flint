use editor::MultiBuffer;
use gpui::{AnyElement, Modifiers};
use svg_preview::svg_preview_view::SvgPreviewView;
use ui::{Tooltip, prelude::*, text_for_keystroke};

use super::QuickActionBar;

impl QuickActionBar {
    pub fn render_preview_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let active_item = self.active_item.as_ref()?;
        let buffer = active_item.act_as::<MultiBuffer>(cx)?;
        if !SvgPreviewView::is_svg_file(&buffer, cx) {
            return None;
        }

        let alt_click = gpui::Keystroke {
            key: "click".into(),
            modifiers: Modifiers::alt(),
            ..Default::default()
        };

        let button = IconButton::new("toggle-svg-preview", IconName::Eye)
            .icon_size(IconSize::Small)
            .chrome_region(ui::ChromeRegion::Toolbar)
            .style(ButtonStyle::Subtle)
            .tooltip(move |_window, cx| {
                Tooltip::with_meta(
                    "Preview SVG",
                    Some(&svg_preview::OpenPreview as &dyn gpui::Action),
                    format!(
                        "{} to open in a split",
                        text_for_keystroke(&alt_click.modifiers, &alt_click.key, cx)
                    ),
                    cx,
                )
            })
            .on_click({
                let workspace_handle = self.workspace.clone();
                let active_item = active_item.boxed_clone();
                move |_, window, cx| {
                    let Some(workspace) = workspace_handle.upgrade() else {
                        return;
                    };
                    let buffer = buffer.clone();
                    workspace.update(cx, |workspace, cx| {
                        let Some(pane) = workspace.pane_for(active_item.as_ref()) else {
                            return;
                        };
                        if window.modifiers().alt {
                            SvgPreviewView::open_preview_to_the_side_of_pane(
                                workspace, buffer, pane, window, cx,
                            );
                        } else {
                            SvgPreviewView::open_preview_in_pane(
                                workspace, buffer, pane, window, cx,
                            );
                        }
                    });
                }
            });

        Some(button.into_any_element())
    }
}
