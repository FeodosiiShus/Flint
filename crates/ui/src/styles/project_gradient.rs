use gpui::{
    Background, Hsla, InteractiveElement, IntoElement, LinearColorStop, Styled, div,
    linear_color_stop, linear_gradient,
};

const LEFT_TO_RIGHT_DEGREES: f32 = 90.;
const PROJECT_GRADIENT_START_OPACITY: f32 = 0.35;
const PROJECT_GRADIENT_FADE_END: f32 = 0.5;

fn project_gradient_stops(color: Hsla) -> (LinearColorStop, LinearColorStop) {
    (
        linear_color_stop(color.opacity(PROJECT_GRADIENT_START_OPACITY), 0.),
        linear_color_stop(color.alpha(0.), PROJECT_GRADIENT_FADE_END),
    )
}

pub fn project_gradient_background(color: Hsla) -> Background {
    let (start, end) = project_gradient_stops(color);
    linear_gradient(LEFT_TO_RIGHT_DEGREES, start, end)
}

pub fn project_gradient_layer(color: Hsla) -> impl IntoElement {
    div()
        .debug_selector(|| "project_gradient".into())
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .bg(project_gradient_background(color))
}

#[cfg(test)]
mod tests {
    use gpui::{Context, ParentElement, Render, TestAppContext, Window, hsla, px};

    use super::*;

    fn project_colors() -> [Hsla; 3] {
        [
            gpui::red(),
            hsla(0.58, 0.62, 0.48, 1.),
            hsla(0.33, 0.4, 0.6, 0.8),
        ]
    }

    struct ProjectGradientSurface;

    impl Render for ProjectGradientSurface {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .debug_selector(|| "surface".into())
                .flex()
                .w(px(320.))
                .h(px(28.))
                .child(project_gradient_layer(gpui::red()))
                .child(
                    div()
                        .debug_selector(|| "content".into())
                        .w(px(40.))
                        .h(px(12.)),
                )
        }
    }

    #[test]
    fn project_gradient_starts_at_the_left_edge_with_a_softened_project_color() {
        for color in project_colors() {
            let (start, _) = project_gradient_stops(color);
            assert_eq!(start.percentage, 0., "the tint starts at the left edge");
            assert_eq!(
                (start.color.h, start.color.s, start.color.l),
                (color.h, color.s, color.l),
                "the tint keeps the project hue, saturation and lightness"
            );
            assert!(
                (start.color.a - color.a * PROJECT_GRADIENT_START_OPACITY).abs() < f32::EPSILON,
                "the tint keeps {PROJECT_GRADIENT_START_OPACITY} of the project alpha, got {}",
                start.color.a
            );
            assert!(
                start.color.a > 0. && start.color.a < color.a,
                "the tint is visible but softer than the project color, got {}",
                start.color.a
            );
        }
    }

    #[test]
    fn project_gradient_fades_to_fully_transparent_before_the_right_edge() {
        for color in project_colors() {
            let (start, end) = project_gradient_stops(color);
            assert!(end.color.is_transparent(), "the tint fades out completely");
            assert_eq!(
                (end.color.h, end.color.s, end.color.l),
                (color.h, color.s, color.l),
                "only the alpha fades, so the tint does not darken on its way out"
            );
            assert!(
                start.percentage < end.percentage && end.percentage < 1.,
                "the fade ends inside the surface, got {}",
                end.percentage
            );
        }
    }

    #[test]
    fn project_gradient_runs_from_the_left_edge_to_the_right() {
        for color in project_colors() {
            let (start, end) = project_gradient_stops(color);
            assert_eq!(
                project_gradient_background(color),
                linear_gradient(90., start, end),
                "a 90 degree gradient puts the first stop on the left edge"
            );
        }
    }

    #[gpui::test]
    fn project_gradient_layer_covers_its_surface_without_moving_content(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| ProjectGradientSurface);
        let surface = cx.debug_bounds("surface").expect("the surface is painted");
        let content = cx.debug_bounds("content").expect("the content is painted");
        assert_eq!(
            cx.debug_bounds("project_gradient"),
            Some(surface),
            "the gradient overlays the whole surface"
        );
        assert_eq!(
            content.origin, surface.origin,
            "the gradient takes no room from the content after it"
        );
        let painted_quads = cx.update(|window, _| window.painted_quads());
        assert!(
            painted_quads
                .iter()
                .any(|quad| quad.background == project_gradient_background(gpui::red())),
            "the overlay paints the project gradient"
        );
    }
}
