use gpui::{App, Hsla, rgba};
use theme::{ActiveTheme, Appearance};

const OPAQUE_ALPHA: u32 = 0xFF;
const CHANNEL_MASK: u32 = 0xFF;
const CHANNEL_SHIFTS: [u32; 4] = [24, 16, 8, 0];

struct CommitColors {
    hovered_background: u32,
    current_branch_background: u32,
    unmatched_foreground: u32,
    reference_foreground: u32,
}

const ISLANDS_DARK_COMMIT_COLORS: CommitColors = CommitColors {
    hovered_background: 0xFFFFFFED,
    current_branch_background: 0x1D2336FF,
    unmatched_foreground: 0x6F737AFF,
    reference_foreground: 0x6F737AFF,
};

const ISLANDS_LIGHT_COMMIT_COLORS: CommitColors = CommitColors {
    hovered_background: 0xE9EAECFF,
    current_branch_background: 0xEDF3FFFF,
    unmatched_foreground: 0x818594FF,
    reference_foreground: 0x6C707EFF,
};

fn commit_colors(cx: &App) -> &'static CommitColors {
    match cx.theme().appearance() {
        Appearance::Light => &ISLANDS_LIGHT_COMMIT_COLORS,
        Appearance::Dark => &ISLANDS_DARK_COMMIT_COLORS,
    }
}

pub(crate) fn hovered_background_over(background: Hsla, cx: &App) -> Hsla {
    rgba(hovered_background(
        commit_colors(cx).hovered_background,
        rgba_hex(background),
    ))
    .into()
}

pub(crate) fn current_branch_background(cx: &App) -> Hsla {
    rgba(commit_colors(cx).current_branch_background).into()
}

pub(crate) fn unmatched_foreground(cx: &App) -> Hsla {
    rgba(commit_colors(cx).unmatched_foreground).into()
}

pub(crate) fn reference_foreground(cx: &App) -> Hsla {
    rgba(commit_colors(cx).reference_foreground).into()
}

fn hovered_background(hovered: u32, background: u32) -> u32 {
    let alpha = hovered & CHANNEL_MASK;
    if alpha == OPAQUE_ALPHA {
        return hovered;
    }
    if alpha == 0 {
        return background;
    }
    mix(
        hovered | OPAQUE_ALPHA,
        background,
        f64::from(alpha) / f64::from(OPAQUE_ALPHA),
    )
}

fn mix(first: u32, second: u32, balance: f64) -> u32 {
    if balance <= 0.0 {
        return first;
    }
    if balance >= 1.0 {
        return second;
    }
    CHANNEL_SHIFTS.iter().fold(0, |mixed, shift| {
        let first_value = (first >> shift) & CHANNEL_MASK;
        let second_value = (second >> shift) & CHANNEL_MASK;
        let value = if first_value == second_value {
            first_value
        } else {
            let difference = f64::from(second_value) - f64::from(first_value);
            (f64::from(first_value) + java_round(balance * difference)) as u32
        };
        mixed | (value << shift)
    })
}

fn java_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

fn rgba_hex(color: Hsla) -> u32 {
    let color = color.to_rgb();
    [color.r, color.g, color.b, color.a]
        .into_iter()
        .fold(0, |hex, component| {
            (hex << 8) | (component.clamp(0.0, 1.0) * 255.0).round() as u32
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn translucent_hover_is_mixed_into_the_background_by_its_alpha() {
        assert_eq!(hovered_background(0xFFFFFFED, 0x191A1CFF), 0x292A2CFF);
    }

    #[test]
    fn opaque_hover_replaces_the_background_and_transparent_hover_keeps_it() {
        assert_eq!(hovered_background(0xE9EAECFF, 0x191A1CFF), 0xE9EAECFF);
        assert_eq!(hovered_background(0xFFFFFF00, 0x191A1CFF), 0x191A1CFF);
    }

    #[test]
    fn background_round_trips_through_hsla() {
        assert_eq!(rgba_hex(rgba(0x191A1CFF).into()), 0x191A1CFF);
    }
}
