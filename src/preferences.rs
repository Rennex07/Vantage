use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde_json::{Value, json};

use crate::palette::{Mode, Theme};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Effects {
    Full,
    #[default]
    Balanced,
    Reduced,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Aod {
    Off,
    Classic,
    #[default]
    Wallpaper,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Appearance {
    pub effects: Effects,
    pub theme: Theme,
    pub reduce_transparency: bool,
    pub reduce_motion: bool,
    pub aod: Aod,
}

pub const MAX_DOCK_PINS: usize = 8;
pub const SETTINGS_APP_ID: &str = "org.vantage.Settings.desktop";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DockPin {
    pub id: String,
    pub icon: Option<PathBuf>,
}

pub fn dock_pins() -> Vec<DockPin> {
    path("dock.json").map_or_else(Vec::new, |path| read_dock_pins(&path))
}

pub fn set_dock_pin(pin: &DockPin, pinned: bool) -> io::Result<()> {
    let path = path("dock.json").ok_or_else(missing_config)?;
    let dir = path.parent().expect("preferences have a parent directory");
    fs::create_dir_all(dir)?;
    // Multiple Applications windows can update the same ordered list.
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("dock.lock"))?;
    lock.lock()?;
    let mut pins = read_dock_pins(&path);
    let existing = pins.iter().position(|entry| entry.id == pin.id);
    match (pinned, existing) {
        (true, None) if pins.len() >= MAX_DOCK_PINS => {
            return Err(io::Error::other(format!(
                "The dock holds up to {MAX_DOCK_PINS} additional apps."
            )));
        }
        (true, None) => pins.push(pin.clone()),
        (false, Some(index)) => {
            pins.remove(index);
        }
        _ => return Ok(()),
    }
    let apps: Vec<_> = pins
        .iter()
        .map(|pin| json!({ "id": pin.id, "icon": pin.icon.as_deref().and_then(Path::to_str) }))
        .collect();
    let settings = json!({ "version": 1, "apps": apps });
    write_atomic(&path, |file| writeln!(file, "{settings}"))
}

fn read_dock_pins(path: &Path) -> Vec<DockPin> {
    fs::read(path).map_or_else(|_| Vec::new(), |bytes| parse_dock_pins(&bytes))
}

fn parse_dock_pins(bytes: &[u8]) -> Vec<DockPin> {
    let Ok(settings) = serde_json::from_slice::<Value>(bytes) else {
        return Vec::new();
    };
    let Some(apps) = settings["apps"]
        .as_array()
        .filter(|_| settings["version"] == 1)
    else {
        return Vec::new();
    };
    let mut pins: Vec<DockPin> = Vec::new();
    for app in apps {
        let Some(id) = app["id"].as_str().filter(|id| valid_dock_id(id)) else {
            continue;
        };
        if pins.iter().any(|pin| pin.id == id) {
            continue;
        }
        let icon = app["icon"]
            .as_str()
            .map(PathBuf::from)
            .filter(|path| path.is_absolute());
        pins.push(DockPin {
            id: id.to_owned(),
            icon,
        });
        if pins.len() == MAX_DOCK_PINS {
            break;
        }
    }
    pins
}

pub fn valid_dock_id(id: &str) -> bool {
    id.len() <= 255
        && id
            .strip_suffix(".desktop")
            .is_some_and(|name| !name.is_empty())
        && !id.contains(['/', '\\', '\0'])
        && !matches!(
            id,
            "thunar.desktop" | "org.xfce.Thunar.desktop" | "vantage-current.desktop"
        )
}

fn path(name: &str) -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|dir| dir.join("vantage").join(name))
}

pub fn brightness() -> u8 {
    path("display-brightness")
        .and_then(|file| fs::read_to_string(file).ok())
        .and_then(|value| value.trim().parse::<u8>().ok())
        .map_or(100, |value| value.clamp(35, 100))
}

pub fn set_brightness(value: u8) -> io::Result<()> {
    let path = path("display-brightness").ok_or_else(missing_config)?;
    write_atomic(&path, |file| writeln!(file, "{}", value.clamp(35, 100)))
}

pub fn appearance() -> Appearance {
    path("appearance.json")
        .and_then(|file| fs::read(file).ok())
        .map_or_else(Appearance::default, |bytes| parse_appearance(&bytes))
}

pub fn saved_theme(mode: Mode) -> Theme {
    if mode == Mode::Light {
        return Theme::from_key(mode, "");
    }
    let settings = path("appearance.json")
        .and_then(|file| fs::read(file).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    settings.map_or_else(
        || Theme::from_key(mode, ""),
        |value| Theme::from_key(mode, value["dark_palette"].as_str().unwrap_or_default()),
    )
}

pub fn set_appearance(appearance: Appearance) -> io::Result<()> {
    let path = path("appearance.json").ok_or_else(missing_config)?;
    let effects = match appearance.effects {
        Effects::Full => "full",
        Effects::Balanced => "balanced",
        Effects::Reduced => "reduced",
    };
    let settings = json!({
        "version": 1,
        "effects": effects,
        "mode": appearance.theme.mode.key(),
        "palette": appearance.theme.key(),
        "dark_palette": if appearance.theme.mode == Mode::Dark { appearance.theme.key() } else { saved_theme(Mode::Dark).key() },
        "reduce_transparency": appearance.reduce_transparency,
        "reduce_motion": appearance.reduce_motion,
        "aod": match appearance.aod {
            Aod::Off => "off",
            Aod::Classic => "classic",
            Aod::Wallpaper => "wallpaper",
        },
    });
    write_atomic(&path, |file| writeln!(file, "{settings}"))
}

fn parse_appearance(bytes: &[u8]) -> Appearance {
    let Ok(settings) = serde_json::from_slice::<Value>(bytes) else {
        return Appearance::default();
    };
    if settings["version"].as_u64() != Some(1) {
        return Appearance::default();
    }
    Appearance {
        theme: Theme::from_key(
            if settings["mode"] == "light" {
                Mode::Light
            } else {
                Mode::Dark
            },
            settings["palette"].as_str().unwrap_or_default(),
        ),
        effects: match settings["effects"].as_str() {
            Some("full") => Effects::Full,
            Some("reduced") => Effects::Reduced,
            _ => Effects::Balanced,
        },
        reduce_transparency: settings["reduce_transparency"]
            .as_bool()
            .unwrap_or_default(),
        reduce_motion: settings["reduce_motion"].as_bool().unwrap_or_default(),
        aod: match settings["aod"].as_str() {
            Some("off") => Aod::Off,
            Some("classic") => Aod::Classic,
            _ => Aod::Wallpaper,
        },
    }
}

fn missing_config() -> io::Error {
    io::Error::other("No configuration directory found")
}

fn write_atomic(
    path: &Path,
    write: impl FnOnce(&mut fs::File) -> io::Result<()>,
) -> io::Result<()> {
    let dir = path.parent().expect("preferences have a parent directory");
    fs::create_dir_all(dir)?;
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    write(temp.as_file_mut())?;
    temp.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_pins_keep_order_and_ignore_duplicates_and_invalid_entries() {
        let pins = parse_dock_pins(
            br#"{"version":1,"apps":[
            {"id":"org.vantage.Settings.desktop","icon":"relative.png"},
            {"id":"org.vantage.Settings.desktop"},
            {"id":"../invalid.desktop"},
            {"id":"thunar.desktop"},
            {"id":"org.example.App.desktop","icon":"/usr/share/icon.svg"}
        ]}"#,
        );
        assert_eq!(
            pins,
            [
                DockPin {
                    id: SETTINGS_APP_ID.into(),
                    icon: None
                },
                DockPin {
                    id: "org.example.App.desktop".into(),
                    icon: Some("/usr/share/icon.svg".into())
                },
            ]
        );
    }

    #[test]
    fn valid_preferences_preserve_independent_settings() {
        assert_eq!(
            parse_appearance(br#"{"version":1,"effects":"full","reduce_motion":true}"#),
            Appearance {
                effects: Effects::Full,
                reduce_motion: true,
                ..Appearance::default()
            }
        );
    }

    #[test]
    fn invalid_fields_fall_back_without_discarding_valid_fields() {
        assert_eq!(
            parse_appearance(
                br#"{"version":1,"effects":"unknown","reduce_motion":"yes","reduce_transparency":true}"#
            ),
            Appearance {
                reduce_transparency: true,
                ..Appearance::default()
            }
        );
        assert_eq!(
            parse_appearance(br#"{"version":2,"effects":"full"}"#),
            Appearance::default()
        );
        assert_eq!(parse_appearance(b"{"), Appearance::default());
    }

    #[test]
    fn mode_and_palette_fall_back_independently_of_effects() {
        let value = parse_appearance(
            br#"{"version":1,"mode":"light","palette":"sage","reduce_transparency":true}"#,
        );
        assert_eq!(value.theme.key(), "opal");
        assert!(value.reduce_transparency);
        assert_eq!(
            parse_appearance(br#"{"version":1,"mode":"light","palette":"midnight"}"#).theme,
            Theme::from_key(Mode::Light, "opal")
        );
    }
}
