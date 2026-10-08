use std::ops::Range;

use gpui::{Hsla, PathBuilder, Pixels, Point, Window, point, px};
use util::ResultExt as _;

pub(crate) const WAVE_STEP: f32 = 4.0;
pub(crate) const WAVE_HALF_HEIGHT: f32 = 3.0;
pub(crate) const WAVE_GAP: f32 = 1.0;
pub(crate) const WAVE_PERIOD: f32 = WAVE_STEP * 4.0;
pub(crate) const WAVE_START_PHASE: i32 = 4;
pub(crate) const CONNECTOR_FLAT_SLOPE: f32 = 0.2;

pub(crate) type Xy = (f32, f32);

impl PathCommand {
    pub(crate) fn mirrored(self, container_width: f32) -> PathCommand {
        let flip = |xy: Xy| (container_width - xy.0, xy.1);
        match self {
            PathCommand::MoveTo(to) => PathCommand::MoveTo(flip(to)),
            PathCommand::QuadTo { control, to } => PathCommand::QuadTo {
                control: flip(control),
                to: flip(to),
            },
            PathCommand::CubicTo {
                control_a,
                control_b,
                to,
            } => PathCommand::CubicTo {
                control_a: flip(control_a),
                control_b: flip(control_b),
                to: flip(to),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PathCommand {
    MoveTo(Xy),
    QuadTo {
        control: Xy,
        to: Xy,
    },
    CubicTo {
        control_a: Xy,
        control_b: Xy,
        to: Xy,
    },
}

pub(crate) fn wave_stroke_width(hovered: bool) -> f32 {
    if hovered { 2.0 } else { 1.0 }
}

pub(crate) fn wave_vertical_offset(line_height: f32) -> f32 {
    ((line_height - 2.0 * WAVE_HALF_HEIGHT - 2.0 * WAVE_GAP) / 2.0).trunc()
}

pub(crate) fn wave_commands(tile_origin_x: f32, top: f32, visible: Range<f32>) -> Vec<PathCommand> {
    let upper = top + WAVE_GAP;
    let center = top + WAVE_HALF_HEIGHT + WAVE_GAP;
    let lower = top + 2.0 * WAVE_HALF_HEIGHT + WAVE_GAP;
    let first_index = ((visible.start - tile_origin_x) / WAVE_PERIOD).floor();
    let mut tile_x = tile_origin_x + first_index * WAVE_PERIOD;
    let mut commands = vec![PathCommand::MoveTo((tile_x, upper))];
    while tile_x < visible.end {
        commands.push(PathCommand::QuadTo {
            control: (tile_x + 2.0, upper),
            to: (tile_x + 4.0, center),
        });
        commands.push(PathCommand::QuadTo {
            control: (tile_x + 6.0, lower),
            to: (tile_x + 8.0, lower),
        });
        commands.push(PathCommand::QuadTo {
            control: (tile_x + 10.0, lower),
            to: (tile_x + 12.0, center),
        });
        commands.push(PathCommand::QuadTo {
            control: (tile_x + 14.0, upper),
            to: (tile_x + 16.0, upper),
        });
        tile_x += WAVE_PERIOD;
    }
    commands
}

pub(crate) fn editor_wave_shift(
    content_width: i32,
    gutter_width: i32,
    mirrored: bool,
    horizontal_scroll: i32,
) -> i32 {
    let interval = WAVE_PERIOD as i32;
    let mut shift = -interval;
    if mirrored {
        shift += content_width % interval - interval;
        shift += gutter_width % interval - interval;
        shift -= WAVE_START_PHASE;
    } else {
        shift += -gutter_width % interval - interval;
        shift += WAVE_START_PHASE;
    }
    shift + horizontal_scroll
}

pub(crate) fn connector_commands(
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
    line_height: f32,
) -> Vec<PathCommand> {
    let step = WAVE_STEP;
    let height = WAVE_HALF_HEIGHT;
    let align = wave_vertical_offset(line_height) + height + WAVE_GAP;
    let y1 = y1 + align;
    let y2 = y2 + align;
    let mut commands = vec![PathCommand::MoveTo((x1 - 0.5, y1))];
    let delta = (y2 - y1).abs() / (x2 - x1).abs();
    if delta < CONNECTOR_FLAT_SLOPE {
        let middle_x = (x1 + x2) / 2.0;
        let middle_y = (y1 + y2) / 2.0;
        if x2 - x1 > 5.0 * step {
            commands.extend([
                PathCommand::QuadTo {
                    control: (x1 + step * 0.5, y1 + height),
                    to: (x1 + step, y1 + height),
                },
                PathCommand::QuadTo {
                    control: (x1 + step * 1.5, y1 + height),
                    to: (x1 + step * 2.0, middle_y),
                },
                PathCommand::QuadTo {
                    control: (x1 + step * 2.5, middle_y - height),
                    to: (middle_x, middle_y - height),
                },
                PathCommand::QuadTo {
                    control: (x2 - step * 2.5, middle_y - height),
                    to: (x2 - step * 2.0, middle_y),
                },
                PathCommand::QuadTo {
                    control: (x2 - step * 1.5, y2 + height),
                    to: (x2 - step, y2 + height),
                },
                PathCommand::QuadTo {
                    control: (x2 - step * 0.5, y2 + height),
                    to: (x2, y2),
                },
            ]);
        } else {
            commands.push(PathCommand::QuadTo {
                control: (middle_x, middle_y + 2.0 * height),
                to: (x2, y2),
            });
        }
    } else if y1 > y2 {
        commands.push(PathCommand::CubicTo {
            control_a: (x1 + step * 0.125, y1 + height * 0.125),
            control_b: (x1 + step * 0.125, y1 + height * 0.5),
            to: (x1 + step * 0.5, y1 + height * 0.5),
        });
        commands.push(PathCommand::CubicTo {
            control_a: (x2 - step * 2.0, y1 + height * 0.5),
            control_b: (x2 - step * 2.0, y2 + 2.0 * height * 2.0),
            to: (x2, y2),
        });
    } else {
        commands.push(PathCommand::CubicTo {
            control_a: (x1 + step * 2.0, y1 + 2.0 * height * 2.0),
            control_b: (x1 + step * 2.0, y2 + height * 0.5),
            to: (x2 - step * 0.5, y2 + height * 0.5),
        });
        commands.push(PathCommand::CubicTo {
            control_a: (x2 - step * 0.125, y2 + height * 0.5),
            control_b: (x2 - step * 0.125, y2 + height * 0.125),
            to: (x2, y2),
        });
    }
    commands
}

pub(crate) fn paint_stroke(
    window: &mut Window,
    commands: &[PathCommand],
    origin: Point<Pixels>,
    width: f32,
    color: Hsla,
) {
    let place = |xy: Xy| point(origin.x + px(xy.0), origin.y + px(xy.1));
    let mut builder = PathBuilder::stroke(px(width));
    for command in commands {
        match *command {
            PathCommand::MoveTo(to) => builder.move_to(place(to)),
            PathCommand::QuadTo { control, to } => builder.curve_to(place(to), place(control)),
            PathCommand::CubicTo {
                control_a,
                control_b,
                to,
            } => builder.cubic_bezier_to(place(to), place(control_a), place(control_b)),
        }
    }
    if let Some(path) = builder.build().log_err() {
        window.paint_path(path, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_tool_wave_vertical_offset_centers_eight_pixel_tile() {
        assert_eq!(wave_vertical_offset(20.0), 6.0);
        assert_eq!(wave_vertical_offset(17.0), 4.0);
        assert_eq!(wave_vertical_offset(8.0), 0.0);
    }

    #[test]
    fn merge_tool_wave_stroke_widens_when_hovered() {
        assert_eq!(wave_stroke_width(false), 1.0);
        assert_eq!(wave_stroke_width(true), 2.0);
    }

    #[test]
    fn merge_tool_wave_tile_matches_spec_points() {
        let commands = wave_commands(0.0, 0.0, 0.0..16.0);
        assert_eq!(commands[0], PathCommand::MoveTo((0.0, 1.0)));
        assert_eq!(
            commands[1],
            PathCommand::QuadTo {
                control: (2.0, 1.0),
                to: (4.0, 4.0)
            }
        );
        assert_eq!(
            commands[2],
            PathCommand::QuadTo {
                control: (6.0, 7.0),
                to: (8.0, 7.0)
            }
        );
        assert_eq!(
            commands[3],
            PathCommand::QuadTo {
                control: (10.0, 7.0),
                to: (12.0, 4.0)
            }
        );
        assert_eq!(
            commands[4],
            PathCommand::QuadTo {
                control: (14.0, 1.0),
                to: (16.0, 1.0)
            }
        );
        assert_eq!(commands.len(), 5);
    }

    #[test]
    fn merge_tool_wave_repeats_tiles_to_cover_visible_range() {
        let commands = wave_commands(0.0, 0.0, 0.0..17.0);
        assert_eq!(commands.len(), 9);
        let shifted = wave_commands(3.0, 0.0, 20.0..30.0);
        assert_eq!(shifted[0], PathCommand::MoveTo((19.0, 1.0)));
    }

    #[test]
    fn merge_tool_wave_shift_matches_java_remainder_semantics() {
        assert_eq!(editor_wave_shift(400, 79, false, 0), -43);
        assert_eq!(editor_wave_shift(400, 79, true, 0), -37);
        assert_eq!(editor_wave_shift(400, 79, false, 10), -33);
    }

    #[test]
    fn merge_tool_connector_uses_six_quads_for_flat_default_divider() {
        let commands = connector_commands(0.0, 24.0, 10.0, 10.0, 20.0);
        assert_eq!(commands[0], PathCommand::MoveTo((-0.5, 20.0)));
        assert_eq!(commands.len(), 7);
        assert_eq!(
            commands[1],
            PathCommand::QuadTo {
                control: (2.0, 23.0),
                to: (4.0, 23.0)
            }
        );
        assert_eq!(
            commands[3],
            PathCommand::QuadTo {
                control: (10.0, 17.0),
                to: (12.0, 17.0)
            }
        );
        assert_eq!(
            commands[6],
            PathCommand::QuadTo {
                control: (22.0, 23.0),
                to: (24.0, 20.0)
            }
        );
    }

    #[test]
    fn merge_tool_connector_uses_single_quad_for_narrow_flat_span() {
        let commands = connector_commands(0.0, 20.0, 10.0, 10.0, 20.0);
        assert_eq!(commands.len(), 2);
        assert_eq!(
            commands[1],
            PathCommand::QuadTo {
                control: (10.0, 26.0),
                to: (20.0, 20.0)
            }
        );
    }

    #[test]
    fn merge_tool_connector_rising_span_uses_two_cubics_ending_at_the_second_side() {
        let commands = connector_commands(0.0, 24.0, 100.0, 40.0, 20.0);
        assert_eq!(commands.len(), 3);
        assert_eq!(commands[0], PathCommand::MoveTo((-0.5, 110.0)));
        assert_eq!(
            commands[1],
            PathCommand::CubicTo {
                control_a: (0.5, 110.375),
                control_b: (0.5, 111.5),
                to: (2.0, 111.5)
            }
        );
        assert_eq!(
            commands[2],
            PathCommand::CubicTo {
                control_a: (16.0, 111.5),
                control_b: (16.0, 62.0),
                to: (24.0, 50.0)
            }
        );
    }

    #[test]
    fn merge_tool_connector_falling_span_uses_two_cubics_ending_at_the_second_side() {
        let commands = connector_commands(0.0, 24.0, 40.0, 100.0, 20.0);
        assert_eq!(commands.len(), 3);
        assert_eq!(commands[0], PathCommand::MoveTo((-0.5, 50.0)));
        assert_eq!(
            commands[1],
            PathCommand::CubicTo {
                control_a: (8.0, 62.0),
                control_b: (8.0, 111.5),
                to: (22.0, 111.5)
            }
        );
        assert_eq!(
            commands[2],
            PathCommand::CubicTo {
                control_a: (23.5, 111.5),
                control_b: (23.5, 110.375),
                to: (24.0, 110.0)
            }
        );
    }

    #[test]
    fn merge_tool_connector_slope_below_one_fifth_stays_a_flat_wave() {
        let commands = connector_commands(0.0, 24.0, 10.0, 14.0, 20.0);
        assert_eq!(commands.len(), 7);
        assert_eq!(
            commands[6],
            PathCommand::QuadTo {
                control: (22.0, 27.0),
                to: (24.0, 24.0)
            }
        );
        let steeper = connector_commands(0.0, 24.0, 10.0, 15.0, 20.0);
        assert_eq!(steeper.len(), 3);
        assert!(matches!(steeper[1], PathCommand::CubicTo { .. }));
    }

    #[test]
    fn merge_tool_path_command_mirrors_x_inside_container() {
        let command = PathCommand::QuadTo {
            control: (2.0, 1.0),
            to: (4.0, 4.0),
        };
        assert_eq!(
            command.mirrored(79.0),
            PathCommand::QuadTo {
                control: (77.0, 1.0),
                to: (75.0, 4.0)
            }
        );
        assert_eq!(
            PathCommand::MoveTo((0.0, 1.0)).mirrored(79.0),
            PathCommand::MoveTo((79.0, 1.0))
        );
    }
}
