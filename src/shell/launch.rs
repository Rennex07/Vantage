use std::{
    env,
    ffi::{OsStr, OsString},
    fs, io,
    os::unix::fs::symlink,
    path::Path,
    process::{Command, ExitStatus},
    thread,
    time::Instant,
};

use vte4::gtk::{gio, glib, prelude::*};

use smithay::reexports::calloop::channel::Sender;

use crate::{
    palette::{Mode, Theme, hex},
    preferences::{self, SETTINGS_APP_ID},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum App {
    Console,
    Files,
}

impl App {
    pub const ALL: [Self; 2] = [Self::Console, Self::Files];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Console => "Console",
            Self::Files => "Files",
        }
    }

    pub fn matches(self, id: &str) -> bool {
        match self {
            Self::Console => id == "org.vantage.Console",
            Self::Files => {
                id.eq_ignore_ascii_case("thunar") || id.eq_ignore_ascii_case("org.xfce.Thunar")
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Target {
    BuiltIn(App),
    Desktop(String),
}

pub fn installed_desktop_ids() -> std::collections::HashSet<String> {
    gio::AppInfo::all()
        .into_iter()
        .filter_map(|app| app.id().map(Into::into))
        .filter(|id: &String| preferences::valid_dock_id(id))
        .chain(std::iter::once(SETTINGS_APP_ID.into()))
        .collect()
}

pub fn desktop_app(id: &str) -> Option<gio::AppInfo> {
    gio::AppInfo::all()
        .into_iter()
        .find(|app| app.id().as_deref() == Some(id))
}

pub fn desktop_name(id: &str) -> String {
    if id == SETTINGS_APP_ID {
        return "Settings".into();
    }
    desktop_app(id).map_or_else(
        || id.strip_suffix(".desktop").unwrap_or(id).to_owned(),
        |app| app.display_name().into(),
    )
}

pub struct Exit {
    pub target: Target,
    pub started: Instant,
    pub message: &'static str,
}

pub fn open(
    target: Target,
    started: Instant,
    socket: &OsStr,
    exits: &Sender<Exit>,
    x11: Option<&crate::desktop::x11::X11>,
) -> io::Result<()> {
    let mut command = Command::new(env::current_exe()?);
    command
        .env("WAYLAND_DISPLAY", socket)
        .env_remove("WAYLAND_SOCKET")
        .env_remove("DISPLAY")
        .env_remove("XAUTHORITY");
    crate::graphics::configure_client(&mut command);
    let title = match &target {
        Target::BuiltIn(app) => {
            command
                .arg(match app {
                    App::Console => "--console",
                    App::Files => "--files",
                })
                .env("GDK_BACKEND", "wayland");
            app.name().to_owned()
        }
        Target::Desktop(id) => {
            if id == SETTINGS_APP_ID {
                command.arg("--settings").env("GDK_BACKEND", "wayland");
            } else {
                command
                    .args(["--launch-pinned", id])
                    .env_remove("GDK_BACKEND");
            }
            desktop_name(id)
        }
    };
    if let Some(x11) = x11 {
        x11.configure(&mut command);
    }

    let exits = exits.clone();
    thread::Builder::new()
        .name("vantage-app-launch".into())
        .spawn(move || {
            let message = match command.status() {
                Ok(status) if status.success() && matches!(target, Target::Desktop(_)) => return,
                Ok(status) if status.success() => "The app closed before its window appeared.",
                Ok(status) => {
                    eprintln!("{title} exited with {status}");
                    if target == Target::BuiltIn(App::Files) && status.code() == Some(127) {
                        "Install thunar and dbus-daemon, then try again."
                    } else {
                        "The app exited before its window appeared."
                    }
                }
                Err(error) => {
                    eprintln!("Could not open {title}: {error}");
                    "The app could not be started."
                }
            };
            let _ = exits.send(Exit {
                target,
                started,
                message,
            });
        })?;
    Ok(())
}

pub fn run_files() -> io::Result<ExitStatus> {
    // A private bus keeps an existing desktop session from capturing the window.
    let mut command = Command::new("dbus-run-session");
    let palette = preferences::appearance().theme;
    command.env(
        "GTK_THEME",
        if palette.mode == Mode::Dark {
            "Adwaita:dark"
        } else {
            "Adwaita"
        },
    );
    let profile = files_profile(&mut command, palette)
        .map_err(|error| eprintln!("Could not load the Files profile: {error}"))
        .ok();
    command.arg("--");
    if profile.is_some() {
        // Only the private bus and its settings service use the temporary profile.
        // Thunar keeps the usual GTK bookmarks and file associations.
        let mut config_home = OsString::from("XDG_CONFIG_HOME=");
        config_home.push(glib::user_config_dir());
        command.arg("/usr/bin/env").arg(config_home);
    }
    command
        .args(["thunar", "--window", "--"])
        .arg(glib::home_dir());
    command.status()
}

fn files_profile(command: &mut Command, palette: Theme) -> io::Result<tempfile::TempDir> {
    let theme = tempfile::Builder::new()
        .prefix("vantage-theme-")
        .tempdir_in(glib::user_runtime_dir())?;
    let config = theme.path().join("config");
    let channel = config.join("xfce4/xfconf/xfce-perchannel-xml");
    fs::create_dir_all(&channel)?;
    fs::write(channel.join("thunar.xml"), include_str!("files.xml"))?;

    let gtk = theme.path().join("themes/Vantage-Session/gtk-3.0");
    fs::create_dir_all(&gtk)?;
    let colors = palette.colors();
    let variant = if palette.mode == Mode::Dark {
        "-dark"
    } else {
        ""
    };
    let stylesheet = format!(
        "@import url(\"resource:///org/gtk/libgtk/theme/Adwaita/gtk-contained{variant}.css\");\n\
        @define-color theme_bg_color {};\n\
        @define-color theme_base_color {};\n\
        @define-color theme_fg_color {};\n\
        @define-color theme_text_color @theme_fg_color;\n\
        @define-color theme_selected_bg_color {};\n\
        @define-color theme_selected_fg_color @theme_fg_color;\n\
        @define-color theme_unfocused_bg_color @theme_bg_color;\n\
        @define-color theme_unfocused_base_color @theme_base_color;\n\
        @define-color theme_unfocused_fg_color @theme_fg_color;\n\
        @define-color theme_unfocused_text_color @theme_fg_color;\n\
        @define-color borders {};\n\
        @define-color vantage_raised {};\n{}",
        hex(colors.surface),
        hex(colors.base),
        hex(colors.text),
        hex(colors.selected),
        hex(colors.accent),
        hex(colors.raised),
        include_str!("files.css"),
    );
    fs::write(gtk.join("gtk.css"), stylesheet)?;
    place_icons(theme.path())?;
    fs::write(
        gtk.join("settings.ini"),
        "[Settings]\ngtk-icon-theme-name=Vantage-Places\ngtk-decoration-layout=:maximize,close\n",
    )?;
    let inherited = env::var_os("XDG_DATA_DIRS")
        .filter(|paths| !paths.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let data = env::join_paths(
        std::iter::once(theme.path().to_path_buf()).chain(env::split_paths(&inherited)),
    )
    .map_err(io::Error::other)?;
    command
        .env("XDG_CONFIG_HOME", config)
        .env("GTK_THEME", "Vantage-Session")
        .env("XDG_DATA_DIRS", data);
    Ok(theme)
}

fn place_icons(profile: &Path) -> io::Result<()> {
    let icons = profile.join("icons");
    let places = icons.join("Vantage-Places/scalable/vantage-places");
    fs::create_dir_all(&places)?;
    fs::write(
        icons.join("Vantage-Places/index.theme"),
        "[Icon Theme]\nName=Vantage Places\nInherits=Adwaita,hicolor\nDirectories=scalable/vantage-places\n\n[scalable/vantage-places]\nSize=24\nMinSize=8\nMaxSize=256\nType=Scalable\nContext=Places\n",
    )?;
    for (name, svg) in [
        (
            "user-desktop",
            include_str!("../../assets/icons/user-desktop.svg"),
        ),
        (
            "user-trash",
            include_str!("../../assets/icons/user-trash.svg"),
        ),
        (
            "user-trash-full",
            include_str!("../../assets/icons/user-trash-full.svg"),
        ),
    ] {
        for variant in [name.to_owned(), format!("{name}-symbolic")] {
            fs::write(places.join(format!("{variant}.svg")), svg)?;
        }
    }

    // GTK on Wayland can take its icon theme from GSettings ahead of theme defaults.
    // Extend that theme in our private search path and keep its other icon directories.
    let active = gio::SettingsSchemaSource::default()
        .and_then(|source| source.lookup("org.gnome.desktop.interface", true))
        .filter(|schema| schema.has_key("icon-theme"))
        .map(|schema| {
            gio::Settings::new_full(&schema, gio::SettingsBackend::NONE, None).string("icon-theme")
        });
    for name in ["Adwaita", active.as_deref().unwrap_or("Adwaita")] {
        if !matches!(
            Path::new(name).components().collect::<Vec<_>>().as_slice(),
            [std::path::Component::Normal(_)]
        ) || name == "Vantage-Places"
        {
            continue;
        }
        let root = icons.join(name);
        if root.exists() {
            continue;
        }
        let index = glib::KeyFile::new();
        for directory in glib::system_data_dirs() {
            if index
                .load_from_file(
                    directory.join("icons").join(name).join("index.theme"),
                    glib::KeyFileFlags::NONE,
                )
                .is_ok()
            {
                break;
            }
        }
        if !index.has_group("Icon Theme") {
            index.set_string("Icon Theme", "Name", name);
            index.set_string("Icon Theme", "Inherits", "Adwaita,hicolor");
        }
        let mut directories = String::new();
        for size in [16, 20, 22, 24, 32, 48, 64] {
            let group = format!("{size}x{size}/vantage-places");
            let destination = root.join(&group);
            fs::create_dir_all(destination.parent().expect("icon directory has a parent"))?;
            symlink(&places, destination)?;
            index.set_integer(&group, "Size", size);
            index.set_string(&group, "Type", "Fixed");
            index.set_string(&group, "Context", "Places");
            directories.push_str(&group);
            directories.push(',');
        }
        if let Ok(existing) = index.string("Icon Theme", "Directories") {
            directories.push_str(&existing);
        }
        index.set_string(
            "Icon Theme",
            "Directories",
            directories.trim_end_matches(','),
        );
        fs::write(root.join("index.theme"), index.to_data().as_bytes())?;
    }
    Ok(())
}
