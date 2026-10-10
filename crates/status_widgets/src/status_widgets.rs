mod indentation;
mod read_only_indicator;

#[cfg(test)]
mod status_widgets_tests;

use gpui::{App, actions};

pub use indentation::{
    INDENTATION_OPTIONS, Indentation, IndentationIndicator, IndentationOption, IndentationSelector,
};
pub use read_only_indicator::{ReadOnlyIndicator, ReadOnlyState};

actions!(status_widgets, [SelectIndentation]);

pub fn init(cx: &mut App) {
    cx.observe_new(IndentationSelector::register).detach();
}
