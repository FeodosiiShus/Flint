use std::cmp::Ordering;

use gpui::{Hsla, SharedString, rgb};
use smallvec::SmallVec;
use theme::Appearance;

use crate::git_log::properties::HEAD_REVISION;

pub(crate) const DEFAULT_COLOR_ID: i32 = 0;
const ISLANDS_DARK_GRAPH_SATURATION: f32 = 0.6;
const ISLANDS_DARK_GRAPH_BRIGHTNESS: f32 = 0.6;
const ISLANDS_LIGHT_GRAPH_SATURATION: f32 = 0.6;
const ISLANDS_LIGHT_GRAPH_BRIGHTNESS: f32 = 0.7;
const RED_STEP: i32 = 200;
const RED_OFFSET: i32 = 30;
const GREEN_STEP: i32 = 130;
const GREEN_OFFSET: i32 = 50;
const BLUE_STEP: i32 = 90;
const BLUE_OFFSET: i32 = 100;
const CHANNEL_RANGE: i32 = 100;
const CHANNEL_MINIMUM: i32 = 70;

const TAG_DECORATION_PREFIX: &str = "tag: ";
const SYMBOLIC_REF_SEPARATOR: &str = " -> ";
const OTHER_REF_PREFIX: &str = "refs/";
const ORIGIN_HEAD: &str = "origin/HEAD";
const MASTER: &str = "master";
const MAIN: &str = "main";
const ORIGIN_MASTER: &str = "origin/master";
const ORIGIN_MAIN: &str = "origin/main";

const SPACE: u16 = b' ' as u16;
const ZERO: u16 = b'0' as u16;
const NINE: u16 = b'9' as u16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GraphPalette {
    saturation: f32,
    brightness: f32,
    default_color: Hsla,
}

impl GraphPalette {
    pub(crate) fn new(appearance: Appearance, label_foreground: Hsla) -> Self {
        match appearance {
            Appearance::Light => Self {
                saturation: ISLANDS_LIGHT_GRAPH_SATURATION,
                brightness: ISLANDS_LIGHT_GRAPH_BRIGHTNESS,
                default_color: Hsla::black(),
            },
            Appearance::Dark => Self {
                saturation: ISLANDS_DARK_GRAPH_SATURATION,
                brightness: ISLANDS_DARK_GRAPH_BRIGHTNESS,
                default_color: label_foreground,
            },
        }
    }

    pub(crate) fn color(&self, color_id: i32) -> Hsla {
        if color_id == DEFAULT_COLOR_ID {
            return self.default_color;
        }
        Hsla::from(rgb(self.generated_rgb(color_id)))
    }

    pub(crate) fn mark_color(&self) -> Hsla {
        self.default_color
    }

    fn generated_rgb(&self, color_id: i32) -> u32 {
        let red = color_id.wrapping_mul(RED_STEP).wrapping_add(RED_OFFSET);
        let green = color_id.wrapping_mul(GREEN_STEP).wrapping_add(GREEN_OFFSET);
        let blue = color_id.wrapping_mul(BLUE_STEP).wrapping_add(BLUE_OFFSET);
        let hue = rgb_to_hue(range_fix(red), range_fix(green), range_fix(blue));
        hsb_to_rgb(hue, self.saturation, self.brightness)
    }
}

fn range_fix(value: i32) -> i32 {
    (value % CHANNEL_RANGE).abs() + CHANNEL_MINIMUM
}

fn rgb_to_hue(red: i32, green: i32, blue: i32) -> f32 {
    let maximum = red.max(green).max(blue);
    let minimum = red.min(green).min(blue);
    if maximum == 0 || maximum == minimum {
        return 0.0;
    }
    let range = (maximum - minimum) as f32;
    let red_distance = (maximum - red) as f32 / range;
    let green_distance = (maximum - green) as f32 / range;
    let blue_distance = (maximum - blue) as f32 / range;
    let mut hue = if red == maximum {
        blue_distance - green_distance
    } else if green == maximum {
        2.0 + red_distance - blue_distance
    } else {
        4.0 + green_distance - red_distance
    };
    hue /= 6.0;
    if hue < 0.0 {
        hue += 1.0;
    }
    hue
}

fn hsb_to_rgb(hue: f32, saturation: f32, brightness: f32) -> u32 {
    let channel = |value: f32| (value * 255.0 + 0.5) as u32;
    if saturation == 0.0 {
        let value = channel(brightness);
        return (value << 16) | (value << 8) | value;
    }
    let sector = (hue - hue.floor()) * 6.0;
    let fraction = sector - sector.floor();
    let lowest = brightness * (1.0 - saturation);
    let falling = brightness * (1.0 - saturation * fraction);
    let rising = brightness * (1.0 - saturation * (1.0 - fraction));
    let (red, green, blue) = match sector as i32 {
        0 => (brightness, rising, lowest),
        1 => (falling, brightness, lowest),
        2 => (lowest, brightness, rising),
        3 => (lowest, falling, brightness),
        4 => (rising, lowest, brightness),
        5 => (brightness, lowest, falling),
        _ => (0.0, 0.0, 0.0),
    };
    (channel(red) << 16) | (channel(green) << 8) | channel(blue)
}

pub(crate) fn java_string_hash(value: &str) -> i32 {
    value.encode_utf16().fold(0i32, |hash, unit| {
        hash.wrapping_mul(31).wrapping_add(i32::from(unit))
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefType {
    Head,
    LocalBranch,
    RemoteBranch,
    Tag,
    Other,
}

impl RefType {
    pub(crate) fn is_branch(self) -> bool {
        matches!(self, Self::Head | Self::LocalBranch | Self::RemoteBranch)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GraphRef {
    pub(crate) name: SharedString,
    pub(crate) ref_type: RefType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum BranchLayoutType {
    OriginMaster,
    RemoteBranch,
    Master,
    LocalBranch,
    Tag,
    Head,
    Other,
}

impl GraphRef {
    fn branch_layout_type(&self) -> BranchLayoutType {
        let name = self.name.as_ref();
        match self.ref_type {
            RefType::RemoteBranch if name == ORIGIN_MASTER || name == ORIGIN_MAIN => {
                BranchLayoutType::OriginMaster
            }
            RefType::RemoteBranch => BranchLayoutType::RemoteBranch,
            RefType::LocalBranch if name == MASTER || name == MAIN => BranchLayoutType::Master,
            RefType::LocalBranch => BranchLayoutType::LocalBranch,
            RefType::Tag => BranchLayoutType::Tag,
            RefType::Head => BranchLayoutType::Head,
            RefType::Other => BranchLayoutType::Other,
        }
    }
}

pub(crate) fn compare_for_branch_layout(left: &GraphRef, right: &GraphRef) -> Ordering {
    left.branch_layout_type()
        .cmp(&right.branch_layout_type())
        .then_with(|| natural_compare(&left.name, &right.name))
}

pub(crate) fn best_ref_for_layout(refs: &[GraphRef]) -> Option<&GraphRef> {
    refs.iter().fold(None, |best, candidate| match best {
        Some(best) if compare_for_branch_layout(best, candidate) != Ordering::Greater => Some(best),
        _ => Some(candidate),
    })
}

pub(crate) fn parse_refs(
    ref_names: &[SharedString],
    remote_names: &[SharedString],
) -> SmallVec<[GraphRef; 2]> {
    let mut refs = SmallVec::new();
    let mut push = |name: &str, ref_type: RefType| {
        if !name.is_empty() && name != ORIGIN_HEAD {
            refs.push(GraphRef {
                name: SharedString::from(name.to_string()),
                ref_type,
            });
        }
    };
    for decoration in ref_names {
        if let Some(tag) = decoration.strip_prefix(TAG_DECORATION_PREFIX) {
            push(tag.trim(), RefType::Tag);
        } else {
            for name in decoration.split(SYMBOLIC_REF_SEPARATOR).map(str::trim) {
                push(name, ref_type_for_name(name, remote_names));
            }
        }
    }
    refs
}

fn ref_type_for_name(name: &str, remote_names: &[SharedString]) -> RefType {
    if name == HEAD_REVISION {
        RefType::Head
    } else if name.starts_with(OTHER_REF_PREFIX) {
        RefType::Other
    } else if name.split_once('/').is_some_and(|(remote, _)| {
        remote_names
            .iter()
            .any(|remote_name| remote_name.as_ref() == remote)
    }) {
        RefType::RemoteBranch
    } else {
        RefType::LocalBranch
    }
}

pub(crate) fn natural_compare(left: &str, right: &str) -> Ordering {
    let left: SmallVec<[u16; 32]> = left.encode_utf16().collect();
    let right: SmallVec<[u16; 32]> = right.encode_utf16().collect();
    natural_compare_units(&left, &right, true).cmp(&0)
}

fn is_decimal_digit(unit: u16) -> bool {
    (ZERO..=NINE).contains(&unit)
}

fn skip_unit(units: &[u16], start: usize, skipped: u16) -> usize {
    start
        + units
            .get(start..)
            .unwrap_or_default()
            .iter()
            .take_while(|unit| **unit == skipped)
            .count()
}

fn skip_digits(units: &[u16], start: usize) -> usize {
    start
        + units
            .get(start..)
            .unwrap_or_default()
            .iter()
            .take_while(|unit| is_decimal_digit(**unit))
            .count()
}

fn compare_unit_range(
    left: &[u16],
    right: &[u16],
    left_start: usize,
    right_start: usize,
    left_end: usize,
) -> i32 {
    let left_units = left.get(left_start..left_end).unwrap_or_default();
    let right_units = right.get(right_start..).unwrap_or_default();
    left_units
        .iter()
        .zip(right_units)
        .map(|(left, right)| i32::from(*left) - i32::from(*right))
        .find(|difference| *difference != 0)
        .unwrap_or_default()
}

fn single_unit(mut characters: impl Iterator<Item = char>) -> Option<u16> {
    let first = characters.next()?;
    if characters.next().is_some() {
        return None;
    }
    u16::try_from(u32::from(first)).ok()
}

fn upper_case(unit: u16) -> u16 {
    if unit <= 0x7F {
        return if (u16::from(b'a')..=u16::from(b'z')).contains(&unit) {
            unit & 0b1101_1111
        } else {
            unit
        };
    }
    char::from_u32(u32::from(unit))
        .and_then(|character| single_unit(character.to_uppercase()))
        .unwrap_or(unit)
}

fn lower_case(unit: u16) -> u16 {
    if unit <= 0x7F {
        return if (u16::from(b'A')..=u16::from(b'Z')).contains(&unit) {
            unit | 0b0010_0000
        } else {
            unit
        };
    }
    char::from_u32(u32::from(unit))
        .and_then(|character| single_unit(character.to_lowercase()))
        .unwrap_or(unit)
}

fn compare_units(left: u16, right: u16, ignore_case: bool) -> i32 {
    if left == SPACE && right > SPACE && right < ZERO {
        return 1;
    }
    if right == SPACE && left > SPACE && left < ZERO {
        return -1;
    }
    let difference = i32::from(left) - i32::from(right);
    if difference == 0 || !ignore_case {
        return difference;
    }
    let left_upper = upper_case(left);
    let right_upper = upper_case(right);
    let upper_difference = i32::from(left_upper) - i32::from(right_upper);
    if upper_difference == 0 {
        return 0;
    }
    i32::from(lower_case(left_upper)) - i32::from(lower_case(right_upper))
}

fn natural_compare_units(left: &[u16], right: &[u16], ignore_case: bool) -> i32 {
    let left_length = left.len();
    let right_length = right.len();
    let mut left_index = 0;
    let mut right_index = 0;
    while let (Some(&left_unit), Some(&right_unit)) = (left.get(left_index), right.get(right_index))
    {
        let left_numeric = is_decimal_digit(left_unit) || left_unit == SPACE;
        let right_numeric = is_decimal_digit(right_unit) || right_unit == SPACE;
        if left_numeric && right_numeric {
            let left_start = skip_unit(left, skip_unit(left, left_index, SPACE), ZERO);
            let right_start = skip_unit(right, skip_unit(right, right_index, SPACE), ZERO);
            let left_end = skip_digits(left, left_start);
            let right_end = skip_digits(right, right_start);

            let length_difference =
                (left_end - left_start) as i32 - (right_end - right_start) as i32;
            if length_difference != 0 {
                return length_difference;
            }
            let number_difference =
                compare_unit_range(left, right, left_start, right_start, left_end);
            if number_difference != 0 {
                return number_difference;
            }
            let full_length_difference =
                (left_end - left_index) as i32 - (right_end - right_index) as i32;
            if full_length_difference != 0 {
                return full_length_difference;
            }
            let leading_difference =
                compare_unit_range(left, right, left_index, right_index, left_start);
            if leading_difference != 0 {
                return leading_difference;
            }
            left_index = left_end.saturating_sub(1);
            right_index = right_end.saturating_sub(1);
        } else {
            let difference = compare_units(left_unit, right_unit, ignore_case);
            if difference != 0 {
                return difference;
            }
        }
        left_index += 1;
        right_index += 1;
    }
    if left_index < left_length {
        return 1;
    }
    if right_index < right_length {
        return -1;
    }
    if left_length != right_length {
        return left_length as i32 - right_length as i32;
    }
    if ignore_case {
        natural_compare_units(left, right, false)
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn graph_ref(name: &str, ref_type: RefType) -> GraphRef {
        GraphRef {
            name: SharedString::from(name.to_string()),
            ref_type,
        }
    }

    #[test]
    fn java_string_hash_wraps_like_java_int_arithmetic() {
        assert_eq!(java_string_hash(""), 0);
        assert_eq!(java_string_hash("a"), 97);
        assert_eq!(java_string_hash("HEAD"), 2213344);
        assert_eq!(java_string_hash("main"), 3343801);
        assert_eq!(java_string_hash("origin/main"), 1685401346);
        assert_eq!(
            java_string_hash("feature/very-long-branch-name"),
            -1082828957
        );
    }

    #[test]
    fn default_color_generator_matches_islands_saturation_and_brightness() {
        let dark = GraphPalette::new(Appearance::Dark, Hsla::white());
        assert_eq!(dark.generated_rgb(1), 0x3D8A99);
        assert_eq!(dark.generated_rgb(2), 0x573D99);
        assert_eq!(dark.generated_rgb(3), 0x3D5499);
        assert_eq!(
            dark.generated_rgb(java_string_hash("origin/main")),
            0x4D3D99
        );
        assert_eq!(dark.generated_rgb(-7), 0x99823D);
        let light = GraphPalette::new(Appearance::Light, Hsla::white());
        assert_eq!(light.generated_rgb(1), 0x47A1B3);
        assert_eq!(
            light.generated_rgb(java_string_hash("origin/main")),
            0x5A47B3
        );
    }

    #[test]
    fn default_color_id_uses_the_label_foreground_in_dark_themes() {
        let foreground = Hsla::from(rgb(0xD1D3D9));
        assert_eq!(
            GraphPalette::new(Appearance::Dark, foreground).color(DEFAULT_COLOR_ID),
            foreground
        );
        assert_eq!(
            GraphPalette::new(Appearance::Light, foreground).color(DEFAULT_COLOR_ID),
            Hsla::black()
        );
        assert_eq!(
            GraphPalette::new(Appearance::Dark, foreground).color(1),
            Hsla::from(rgb(0x3D8A99))
        );
    }

    #[test]
    fn parse_refs_splits_symbolic_refs_and_classifies_short_names() {
        let remotes = [SharedString::from("origin"), SharedString::from("upstream")];
        let refs = parse_refs(
            &[
                SharedString::from("HEAD -> main"),
                SharedString::from("origin/main"),
                SharedString::from("origin/HEAD"),
                SharedString::from("upstream/HEAD"),
                SharedString::from("tag: v1.0"),
                SharedString::from("feature/x"),
                SharedString::from("refs/stash"),
            ],
            &remotes,
        );
        assert_eq!(
            refs.into_vec(),
            vec![
                graph_ref("HEAD", RefType::Head),
                graph_ref("main", RefType::LocalBranch),
                graph_ref("origin/main", RefType::RemoteBranch),
                graph_ref("upstream/HEAD", RefType::RemoteBranch),
                graph_ref("v1.0", RefType::Tag),
                graph_ref("feature/x", RefType::LocalBranch),
                graph_ref("refs/stash", RefType::Other),
            ]
        );
    }

    #[test]
    fn branch_layout_prefers_origin_main_then_remotes_then_main() {
        let refs = [
            graph_ref("HEAD", RefType::Head),
            graph_ref("feature", RefType::LocalBranch),
            graph_ref("main", RefType::LocalBranch),
            graph_ref("v1", RefType::Tag),
            graph_ref("origin/feature", RefType::RemoteBranch),
            graph_ref("origin/main", RefType::RemoteBranch),
            graph_ref("refs/stash", RefType::Other),
        ];
        let mut sorted = refs.to_vec();
        sorted.sort_by(compare_for_branch_layout);
        let names: Vec<&str> = sorted
            .iter()
            .map(|graph_ref| graph_ref.name.as_ref())
            .collect();
        assert_eq!(
            names,
            vec![
                "origin/main",
                "origin/feature",
                "main",
                "feature",
                "v1",
                "HEAD",
                "refs/stash"
            ]
        );
        assert_eq!(
            best_ref_for_layout(&refs).map(|graph_ref| graph_ref.name.as_ref()),
            Some("origin/main")
        );
        assert_eq!(best_ref_for_layout(&[]), None);
    }

    #[test]
    fn natural_compare_orders_numbers_by_value_and_ignores_case_first() {
        assert_eq!(natural_compare("v2", "v10"), Ordering::Less);
        assert_eq!(natural_compare("v10", "v2"), Ordering::Greater);
        assert_eq!(natural_compare("a7", "a007"), Ordering::Less);
        assert_eq!(natural_compare("Beta", "alpha"), Ordering::Greater);
        assert_eq!(natural_compare("Main", "main"), Ordering::Less);
        assert_eq!(natural_compare("main", "main"), Ordering::Equal);
        assert_eq!(natural_compare("fix", "fix-1"), Ordering::Less);
    }
}
