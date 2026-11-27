pub type Color = (f64, f64, f64);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Mode {
    #[default]
    Dark,
    Light,
}

impl Mode {
    pub fn key(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
}

pub struct DarkColor {
    pub key: &'static str,
    pub name: &'static str,
    accent: u32,
    tint: u32,
}

const fn dark(key: &'static str, name: &'static str, accent: u32, tint: u32) -> DarkColor {
    DarkColor {
        key,
        name,
        accent,
        tint,
    }
}

pub const DARK_COLORS: [DarkColor; 7] = [
    dark("navy-blue", "Navy Blue", 0x92baf7, 0x355dc2),
    dark("purple", "Purple", 0xc9a2f6, 0x963ed8),
    dark("green", "Green", 0x8bd9a5, 0x289c56),
    dark("teal", "Teal", 0x7bd4d0, 0x269d9d),
    dark("burgundy", "Burgundy", 0xf3a1b7, 0xaf3361),
    dark("amber", "Amber", 0xf3c47f, 0xbc692b),
    dark("charcoal", "Charcoal", 0xc3c7cf, 0x747982),
];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Theme {
    pub mode: Mode,
    dark_index: usize,
}

impl Theme {
    pub fn from_key(mode: Mode, key: &str) -> Self {
        if mode == Mode::Light {
            return Self {
                mode,
                ..Self::default()
            };
        }
        // Preserve the closest color from version 1 appearance files.
        let key = match key {
            "obsidian" => "charcoal",
            "deep-ocean" | "aurora" => "teal",
            "evergreen" => "green",
            "amethyst" | "mulberry" => "purple",
            "rosewood" => "burgundy",
            "ember" | "brass" | "espresso" => "amber",
            _ => key,
        };
        let dark_index = DARK_COLORS
            .iter()
            .position(|preset| preset.key == key)
            .unwrap_or(0);
        Self { mode, dark_index }
    }

    pub fn dark(index: usize) -> Self {
        Self {
            mode: Mode::Dark,
            dark_index: index.min(DARK_COLORS.len() - 1),
        }
    }

    pub fn dark_index(self) -> usize {
        self.dark_index
    }

    pub fn key(self) -> &'static str {
        match self.mode {
            Mode::Dark => DARK_COLORS[self.dark_index].key,
            Mode::Light => "opal",
        }
    }

    pub fn dark_tint(self) -> Color {
        rgb(DARK_COLORS[self.dark_index].tint)
    }

    pub fn colors(self) -> Palette {
        let tint = self.dark_tint();
        let accent = rgb(DARK_COLORS[self.dark_index].accent);
        match self.mode {
            Mode::Dark => Palette {
                base: mix(rgb(0x08090b), tint, 0.14),
                surface: mix(rgb(0x141416), tint, 0.22),
                raised: mix(rgb(0x202024), tint, 0.34),
                selected: mix(rgb(0x1c1c20), tint, 0.48),
                text: mix(rgb(0xffffff), accent, 0.08),
                muted: mix(rgb(0xd3d2d4), accent, 0.20),
                accent,
                warning: rgb(0xe6b97c),
            },
            Mode::Light => Palette {
                base: rgb(0xeee9e0),
                surface: rgb(0xf6f2eb),
                raised: rgb(0xe8dfd1),
                selected: rgb(0xe1d1bb),
                text: rgb(0x2d251e),
                muted: rgb(0x5e4c3c),
                accent: rgb(0x795132),
                warning: rgb(0x8b3e24),
            },
        }
    }
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub base: Color,
    pub surface: Color,
    pub raised: Color,
    pub selected: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub warning: Color,
}

fn rgb(value: u32) -> Color {
    let channel = |shift: u32| f64::from((value >> shift) & 255) / 255.0;
    (channel(16), channel(8), channel(0))
}

pub fn mix(a: Color, b: Color, t: f64) -> Color {
    (
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
    )
}

pub fn hex((r, g, b): Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        (255.0 * r).round() as u8,
        (255.0 * g).round() as u8,
        (255.0 * b).round() as u8
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_controls_are_legible_across_all_presets() {
        fn luminance((r, g, b): Color) -> f64 {
            let linear = |c: f64| {
                if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
        }
        let contrast = |a, b| {
            let (a, b) = (luminance(a), luminance(b));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        };
        let themes = (0..DARK_COLORS.len())
            .map(Theme::dark)
            .chain(std::iter::once(Theme::from_key(Mode::Light, "opal")));
        for theme in themes {
            let colors = theme.colors();
            for surface in [colors.base, colors.surface, colors.raised, colors.selected] {
                assert!(contrast(colors.text, surface) >= 4.5, "{}", theme.key());
                assert!(contrast(colors.muted, surface) >= 4.5, "{}", theme.key());
                assert!(contrast(colors.accent, surface) >= 3.0, "{}", theme.key());
            }
        }
    }

    #[test]
    fn existing_keys_select_their_closest_color() {
        assert_eq!(Theme::from_key(Mode::Dark, "blue-hour").key(), "navy-blue");
        assert_eq!(Theme::from_key(Mode::Dark, "amethyst").key(), "purple");
        assert_eq!(Theme::from_key(Mode::Dark, "evergreen").key(), "green");
        assert_eq!(Theme::from_key(Mode::Light, "sage").key(), "opal");
    }
}
