pub(crate) mod highlights;

use gpui::{App, Global, Hsla, rgb};
use theme::ActiveTheme;

const EVA_DARK_THEME_NAME: &str = "Eva Dark";
const FADE_BALANCE: f64 = 0.6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum ChangeKind {
    Inserted = 0,
    Deleted = 1,
    Modified = 2,
    Conflict = 3,
}

impl ChangeKind {
    pub(crate) const ALL: [ChangeKind; 4] = [
        ChangeKind::Inserted,
        ChangeKind::Deleted,
        ChangeKind::Modified,
        ChangeKind::Conflict,
    ];

    pub(crate) fn index(self) -> usize {
        self as usize
    }

    pub(crate) fn from_index(index: u8) -> ChangeKind {
        match index {
            0 => ChangeKind::Inserted,
            1 => ChangeKind::Deleted,
            2 => ChangeKind::Modified,
            _ => ChangeKind::Conflict,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MergePaletteKind {
    EvaDark,
    IslandsDark,
    Light,
}

impl MergePaletteKind {
    pub(crate) fn for_theme(theme_name: &str, is_light: bool) -> Self {
        if is_light {
            MergePaletteKind::Light
        } else if theme_name == EVA_DARK_THEME_NAME {
            MergePaletteKind::EvaDark
        } else {
            MergePaletteKind::IslandsDark
        }
    }

    pub(crate) fn is_dark(self) -> bool {
        !matches!(self, MergePaletteKind::Light)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MergePaletteOverride(pub Option<MergePaletteKind>);

impl Global for MergePaletteOverride {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChangeColorSpec {
    pub background: u32,
    pub stripe: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MergePalette {
    pub kind: MergePaletteKind,
    pub editor_background: u32,
    pub editor_foreground: u32,
    pub inserted: ChangeColorSpec,
    pub deleted: ChangeColorSpec,
    pub modified: ChangeColorSpec,
    pub conflict: ChangeColorSpec,
    pub ai_resolved: u32,
    pub tearline: u32,
    pub wave: u32,
    pub line_numbers: u32,
    pub icon_stroke: u32,
    pub label_foreground: u32,
    pub status_no_conflicts: u32,
    pub header_island_fill: u32,
    pub header_island_border: u32,
    pub success_balloon_fill: u32,
    pub success_balloon_border: u32,
    pub fold_chevron: u32,
    pub hint_foreground: u32,
    pub hint_background: u32,
}

const ISLANDS_DARK: MergePalette = MergePalette {
    kind: MergePaletteKind::IslandsDark,
    editor_background: 0x191A1C,
    editor_foreground: 0xBCBEC4,
    inserted: ChangeColorSpec {
        background: 0x294436,
        stripe: 0x447152,
    },
    deleted: ChangeColorSpec {
        background: 0x484A4A,
        stripe: 0x656E76,
    },
    modified: ChangeColorSpec {
        background: 0x385570,
        stripe: 0x43698D,
    },
    conflict: ChangeColorSpec {
        background: 0x45302B,
        stripe: 0x8F5247,
    },
    ai_resolved: 0xA571E6,
    tearline: 0x555555,
    wave: 0x555555,
    line_numbers: 0x4B5059,
    icon_stroke: 0xC3C5CB,
    label_foreground: 0xD1D3D9,
    status_no_conflicts: 0x57965C,
    header_island_fill: 0x212326,
    header_island_border: 0x26282C,
    success_balloon_fill: 0x203B2A,
    success_balloon_border: 0x29583C,
    fold_chevron: 0x6F737A,
    hint_foreground: 0x858A94,
    hint_background: 0x393B40,
};

const EVA_DARK: MergePalette = MergePalette {
    kind: MergePaletteKind::EvaDark,
    editor_background: 0x282C34,
    editor_foreground: 0xB0B7C3,
    inserted: ChangeColorSpec {
        background: 0x33573C,
        stripe: 0x37673F,
    },
    deleted: ChangeColorSpec {
        background: 0x813A3F,
        stripe: 0xA14043,
    },
    modified: ChangeColorSpec {
        background: 0x6A5E9B,
        stripe: 0xA78CFA,
    },
    conflict: ChangeColorSpec {
        background: 0x813A3F,
        stripe: 0xA14043,
    },
    ai_resolved: 0xA571E6,
    tearline: 0x3A3E55,
    wave: 0x4E546E,
    line_numbers: 0x535773,
    icon_stroke: 0xC3C5CB,
    label_foreground: 0xD1D3D9,
    status_no_conflicts: 0x57965C,
    header_island_fill: 0x212326,
    header_island_border: 0x26282C,
    success_balloon_fill: 0x203B2A,
    success_balloon_border: 0x29583C,
    fold_chevron: 0x6F737A,
    hint_foreground: 0x747BA4,
    hint_background: 0x383C45,
};

const LIGHT: MergePalette = MergePalette {
    kind: MergePaletteKind::Light,
    editor_background: 0xFFFFFF,
    editor_foreground: 0x080808,
    inserted: ChangeColorSpec {
        background: 0xBEE6BE,
        stripe: 0xAADEAA,
    },
    deleted: ChangeColorSpec {
        background: 0xD6D6D6,
        stripe: 0xC8C8C8,
    },
    modified: ChangeColorSpec {
        background: 0xC2D8F2,
        stripe: 0xB6D2F2,
    },
    conflict: ChangeColorSpec {
        background: 0xFFD5CC,
        stripe: 0xFFC8BD,
    },
    ai_resolved: 0x834DF0,
    tearline: 0xD4D4D4,
    wave: 0xD0D0D0,
    line_numbers: 0xAEB3C2,
    icon_stroke: 0x6C707E,
    label_foreground: 0x000000,
    status_no_conflicts: 0x1F7536,
    header_island_fill: 0xF7F8F9,
    header_island_border: 0xE9EAEE,
    success_balloon_fill: 0xF5FAF3,
    success_balloon_border: 0xBBDBC2,
    fold_chevron: 0xA8ADBD,
    hint_foreground: 0x7A7A7A,
    hint_background: 0xEDEDED,
};

impl MergePalette {
    pub(crate) fn of(kind: MergePaletteKind) -> Self {
        match kind {
            MergePaletteKind::EvaDark => EVA_DARK,
            MergePaletteKind::IslandsDark => ISLANDS_DARK,
            MergePaletteKind::Light => LIGHT,
        }
    }

    fn spec(&self, kind: ChangeKind) -> ChangeColorSpec {
        match kind {
            ChangeKind::Inserted => self.inserted,
            ChangeKind::Deleted => self.deleted,
            ChangeKind::Modified => self.modified,
            ChangeKind::Conflict => self.conflict,
        }
    }

    pub(crate) fn background(&self, kind: ChangeKind) -> u32 {
        self.spec(kind).background
    }

    pub(crate) fn stripe(&self, kind: ChangeKind) -> u32 {
        self.spec(kind).stripe
    }

    pub(crate) fn faded(&self, kind: ChangeKind, editor_background: u32) -> u32 {
        fade_toward(self.background(kind), editor_background)
    }
}

fn channel(color: u32, shift: u32) -> u8 {
    ((color >> shift) & 0xFF) as u8
}

fn mix_channel(value: u8, background: u8) -> u8 {
    if value == background {
        return value;
    }
    let delta = FADE_BALANCE * (f64::from(background) - f64::from(value));
    let rounded = (delta + 0.5).floor();
    (f64::from(value) + rounded) as u8
}

pub(crate) fn fade_toward(color: u32, editor_background: u32) -> u32 {
    let red = mix_channel(channel(color, 16), channel(editor_background, 16));
    let green = mix_channel(channel(color, 8), channel(editor_background, 8));
    let blue = mix_channel(channel(color, 0), channel(editor_background, 0));
    (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)
}

pub(crate) fn hex_color(hex: u32) -> Hsla {
    Hsla::from(rgb(hex))
}

pub(crate) fn color_hex(color: Hsla) -> u32 {
    let rgba = color.to_rgb();
    let to_byte = |value: f32| (value.clamp(0., 1.) * 255.).round() as u32;
    (to_byte(rgba.r) << 16) | (to_byte(rgba.g) << 8) | to_byte(rgba.b)
}

pub(crate) fn current_palette_kind(cx: &App) -> MergePaletteKind {
    let override_kind = cx
        .try_global::<MergePaletteOverride>()
        .and_then(|selection| selection.0);
    override_kind.unwrap_or_else(|| {
        let theme = cx.theme();
        MergePaletteKind::for_theme(&theme.name, theme.appearance().is_light())
    })
}

pub(crate) fn current_palette(cx: &App) -> MergePalette {
    MergePalette::of(current_palette_kind(cx))
}

pub(crate) fn editor_background(cx: &App) -> Hsla {
    cx.theme().colors().editor_background.alpha(1.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ChangeColors {
    pub solid: Hsla,
    pub faded: Hsla,
    pub stripe: Hsla,
}

pub(crate) fn change_colors(cx: &App, kind: ChangeKind, ai_resolved: bool) -> ChangeColors {
    let palette = current_palette(cx);
    let background_hex = color_hex(editor_background(cx));
    let base = palette.background(kind);
    let solid = if ai_resolved {
        palette.ai_resolved
    } else {
        base
    };
    ChangeColors {
        solid: hex_color(solid),
        faded: hex_color(palette.faded(kind, background_hex)),
        stripe: hex_color(palette.stripe(kind)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use settings::SettingsStore;
    use theme::LoadThemes;

    #[test]
    fn merge_tool_fade_matches_islands_dark_table() {
        let background = ISLANDS_DARK.editor_background;
        assert_eq!(
            ISLANDS_DARK.faded(ChangeKind::Inserted, background),
            0x1F2B26
        );
        assert_eq!(
            ISLANDS_DARK.faded(ChangeKind::Modified, background),
            0x25323E
        );
        assert_eq!(
            ISLANDS_DARK.faded(ChangeKind::Deleted, background),
            0x2C2D2E
        );
        assert_eq!(
            ISLANDS_DARK.faded(ChangeKind::Conflict, background),
            0x2B2322
        );
    }

    #[test]
    fn merge_tool_fade_matches_eva_dark_table() {
        let background = EVA_DARK.editor_background;
        assert_eq!(EVA_DARK.faded(ChangeKind::Inserted, background), 0x2C3D37);
        assert_eq!(EVA_DARK.faded(ChangeKind::Modified, background), 0x42405D);
        assert_eq!(EVA_DARK.faded(ChangeKind::Deleted, background), 0x4C3238);
        assert_eq!(EVA_DARK.faded(ChangeKind::Conflict, background), 0x4C3238);
    }

    #[test]
    fn merge_tool_fade_matches_light_table() {
        let background = LIGHT.editor_background;
        assert_eq!(LIGHT.faded(ChangeKind::Inserted, background), 0xE5F5E5);
        assert_eq!(LIGHT.faded(ChangeKind::Modified, background), 0xE7EFFA);
        assert_eq!(LIGHT.faded(ChangeKind::Deleted, background), 0xEFEFEF);
        assert_eq!(LIGHT.faded(ChangeKind::Conflict, background), 0xFFEEEB);
    }

    #[test]
    fn merge_tool_fade_keeps_channels_equal_to_background() {
        assert_eq!(fade_toward(0x102030, 0x102030), 0x102030);
        assert_eq!(fade_toward(0x112233, 0x112200), 0x112214);
    }

    #[gpui::test]
    fn merge_tool_ai_resolved_changes_turn_purple_while_faded_and_stripe_keep_the_base_kind(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(LoadThemes::JustBase, cx);
            cx.set_global(MergePaletteOverride(Some(MergePaletteKind::IslandsDark)));

            let plain = change_colors(cx, ChangeKind::Inserted, false);
            let ai_resolved = change_colors(cx, ChangeKind::Inserted, true);

            assert_eq!(plain.solid, hex_color(0x294436));
            assert_eq!(plain.stripe, hex_color(0x447152));
            assert_eq!(ai_resolved.solid, hex_color(0xA571E6));
            assert_eq!(ai_resolved.faded, plain.faded);
            assert_eq!(ai_resolved.stripe, plain.stripe);
        });
    }

    #[gpui::test]
    fn merge_tool_palette_override_wins_over_the_active_theme(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(LoadThemes::JustBase, cx);

            cx.set_global(MergePaletteOverride(Some(MergePaletteKind::EvaDark)));
            assert_eq!(current_palette_kind(cx), MergePaletteKind::EvaDark);
            assert_eq!(
                change_colors(cx, ChangeKind::Conflict, false).stripe,
                hex_color(0xA14043)
            );

            cx.set_global(MergePaletteOverride(Some(MergePaletteKind::Light)));
            assert_eq!(current_palette_kind(cx), MergePaletteKind::Light);
            assert_eq!(
                change_colors(cx, ChangeKind::Conflict, false).stripe,
                hex_color(0xFFC8BD)
            );
        });
    }

    #[test]
    fn merge_tool_palette_selection_follows_theme_rule() {
        assert_eq!(
            MergePaletteKind::for_theme("Eva Dark", false),
            MergePaletteKind::EvaDark
        );
        assert_eq!(
            MergePaletteKind::for_theme("One Dark", false),
            MergePaletteKind::IslandsDark
        );
        assert_eq!(
            MergePaletteKind::for_theme("Eva Dark", true),
            MergePaletteKind::Light
        );
        assert_eq!(
            MergePaletteKind::for_theme("One Light", true),
            MergePaletteKind::Light
        );
    }

    #[test]
    fn merge_tool_hex_color_round_trips() {
        for hex in [0x000000, 0xFFFFFF, 0x282C34, 0x813A3F, 0xA571E6, 0x191A1C] {
            assert_eq!(color_hex(hex_color(hex)), hex);
        }
    }

    #[test]
    fn merge_tool_change_kind_index_round_trips() {
        for kind in ChangeKind::ALL {
            assert_eq!(ChangeKind::from_index(kind.index() as u8), kind);
        }
    }
}
