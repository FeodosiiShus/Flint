use crate::{component_corner_radii, prelude::*};
use gpui::{AnyElement, IntoElement, ParentElement, Styled};

/// Banners provide informative and brief messages without interrupting the user.
/// This component offers four severity levels that can be used depending on the message.
///
/// # Usage Example
///
/// ```
/// use ui::prelude::*;
/// use ui::{Banner, Button, Icon, IconName, IconSize, Label, Severity};
///
/// Banner::new()
///     .severity(Severity::Success)
///     .children([Label::new("This is a success message")])
///     .action_slot(
///         Button::new("learn-more", "Learn More")
///             .end_icon(Icon::new(IconName::ArrowUpRight).size(IconSize::Small)),
///     );
/// ```
#[derive(IntoElement, RegisterComponent)]
pub struct Banner {
    severity: Severity,
    children: Vec<AnyElement>,
    action_slot: Option<AnyElement>,
    wrap_content: bool,
}

impl Default for Banner {
    fn default() -> Self {
        Self::new()
    }
}

impl Banner {
    /// Creates a new `Banner` component with default styling.
    pub fn new() -> Self {
        Self {
            severity: Severity::Info,
            children: Vec::new(),
            action_slot: None,
            wrap_content: false,
        }
    }

    /// Sets the severity of the banner.
    pub fn severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }

    /// A slot for actions, such as CTA or dismissal buttons.
    pub fn action_slot(mut self, element: impl IntoElement) -> Self {
        self.action_slot = Some(element.into_any_element());
        self
    }

    /// Sets whether the banner content should wrap.
    pub fn wrap_content(mut self, wrap: bool) -> Self {
        self.wrap_content = wrap;
        self
    }
}

impl ParentElement for Banner {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements)
    }
}

impl RenderOnce for Banner {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let banner = h_flex()
            .min_w_0()
            .py_0p5()
            .gap_1p5()
            .when(self.wrap_content, |this| this.flex_wrap())
            .justify_between()
            .rounded(component_corner_radii(cx).banner)
            .border_1();

        let (icon, icon_color, bg_color, border_color) = match self.severity {
            Severity::Info => (
                IconName::Info,
                Color::Muted,
                cx.theme().status().info_background.opacity(0.5),
                cx.theme().colors().border.opacity(0.5),
            ),
            Severity::Success => (
                IconName::Check,
                Color::Success,
                cx.theme().status().success.opacity(0.1),
                cx.theme().status().success.opacity(0.2),
            ),
            Severity::Warning => (
                IconName::Warning,
                Color::Warning,
                cx.theme().status().warning_background.opacity(0.5),
                cx.theme().status().warning_border.opacity(0.4),
            ),
            Severity::Error => (
                IconName::XCircle,
                Color::Error,
                cx.theme().status().error.opacity(0.1),
                cx.theme().status().error.opacity(0.2),
            ),
        };

        let mut banner = banner.bg(bg_color).border_color(border_color);

        let icon_and_child = h_flex()
            .items_start()
            .min_w_0()
            .flex_1()
            .gap_1p5()
            .child(
                h_flex()
                    .h(window.line_height())
                    .flex_shrink_0()
                    .child(Icon::new(icon).size(IconSize::XSmall).color(icon_color)),
            )
            .child(div().min_w_0().flex_1().children(self.children));

        if let Some(action_slot) = self.action_slot {
            banner = banner
                .pl_2()
                .pr_1()
                .child(icon_and_child)
                .child(action_slot);
        } else {
            banner = banner.px_2().child(icon_and_child);
        }

        banner
    }
}

impl Component for Banner {
    fn scope() -> ComponentScope {
        ComponentScope::DataDisplay
    }

    fn description() -> &'static str {
        "A non-blocking, severity-aware message strip used to surface informative, \
        success, warning, or error messages without interrupting the user."
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> AnyElement {
        let severity_examples = vec![
            single_example(
                "Default",
                Banner::new()
                    .child(Label::new("This is a default banner with no customization"))
                    .into_any_element(),
            ),
            single_example(
                "Info",
                Banner::new()
                    .severity(Severity::Info)
                    .child(Label::new("This is an informational message"))
                    .action_slot(
                        Button::new("learn-more", "Learn More")
                            .end_icon(Icon::new(IconName::ArrowUpRight).size(IconSize::Small)),
                    )
                    .into_any_element(),
            ),
            single_example(
                "Success",
                Banner::new()
                    .severity(Severity::Success)
                    .child(Label::new("Operation completed successfully"))
                    .action_slot(Button::new("dismiss", "Dismiss"))
                    .into_any_element(),
            ),
            single_example(
                "Warning",
                Banner::new()
                    .severity(Severity::Warning)
                    .child(Label::new("Your settings file uses deprecated settings"))
                    .action_slot(Button::new("update", "Update Settings"))
                    .into_any_element(),
            ),
            single_example(
                "Error",
                Banner::new()
                    .severity(Severity::Error)
                    .child(Label::new("Connection error: unable to connect to server"))
                    .action_slot(Button::new("reconnect", "Retry"))
                    .into_any_element(),
            ),
        ];

        example_group(severity_examples)
            .vertical()
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Render, TestAppContext, VisualTestContext};

    struct BannerProbe;

    impl Render for BannerProbe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .p_4()
                .child(Banner::new().child(Label::new("Banner message")))
        }
    }

    fn init_theme(density: settings::UiDensity, cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            cx.update_global::<settings::SettingsStore, _>(|store, cx| {
                store.update_user_settings(cx, |settings| {
                    settings.theme.ui_density = Some(density);
                });
            });
        });
    }

    fn painted_banner_radius(cx: &mut VisualTestContext) -> f32 {
        cx.update(|window, cx| {
            let border_color = cx.theme().colors().border.opacity(0.5);
            let scale = window.scale_factor();
            window
                .painted_quads()
                .into_iter()
                .find(|quad| quad.border_color == border_color && quad.border_widths.top.0 > 0.)
                .map(|quad| quad.corner_radii.top_left.0 / scale)
                .expect("the info banner paints its bordered box")
        })
    }

    #[gpui::test]
    fn banner_uses_the_islands_banner_arc(cx: &mut TestAppContext) {
        init_theme(settings::UiDensity::Default, cx);
        let (_, cx) = cx.add_window_view(|_, _| BannerProbe);
        assert!(
            (painted_banner_radius(cx) - 6.).abs() < 0.01,
            "Banner.arc 12 is a 6 px radius"
        );
    }

    #[gpui::test]
    fn banner_uses_the_islands_compact_banner_arc_in_compact_density(cx: &mut TestAppContext) {
        init_theme(settings::UiDensity::Compact, cx);
        let (_, cx) = cx.add_window_view(|_, _| BannerProbe);
        assert!(
            (painted_banner_radius(cx) - 4.).abs() < 0.01,
            "Banner.arc.compact 8 is a 4 px radius"
        );
    }
}
