use gpui::{Pixels, px};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComponentCornerRadii {
    pub button: Pixels,
    pub input: Pixels,
    pub checkbox: Pixels,
    pub selection: Pixels,
    pub toolbar_widget: Pixels,
    pub banner: Pixels,
}

pub const COMPONENT_CORNER_RADII: ComponentCornerRadii = ComponentCornerRadii {
    button: px(4.),
    input: px(4.),
    checkbox: px(3.),
    selection: px(4.),
    toolbar_widget: px(6.),
    banner: px(6.),
};
