use std::{
    env,
    ffi::{OsStr, OsString},
    fs, io,
    process::Command,
    thread,
    time::Instant,
};

use vte4::gtk::glib;

use smithay::reexports::calloop::channel::Sender;

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

pub struct Exit {
    pub app: App,
    pub started: Instant,
    pub message: &'static str,
}

pub fn open(app: App, started: Instant, socket: &OsStr, exits: &Sender<Exit>) -> io::Result<()> {
    let mut command = match app {
        App::Console => {
            let mut command = Command::new(env::current_exe()?);
            command.arg("--console");
            command
        }
        App::Files => {
            // A private bus keeps an existing desktop session from capturing the window.
            let mut command = Command::new("dbus-run-session");
            command.env("GTK_THEME", "Adwaita:dark");
            command
        }
    };
    command
        .env("WAYLAND_DISPLAY", socket)
        .env_remove("WAYLAND_SOCKET")
        .env("GDK_BACKEND", "wayland");

    let exits = exits.clone();
    thread::Builder::new()
        .name(app.name().into())
        .spawn(move || {
            let _profile = if app == App::Files {
                let profile = files_profile(&mut command)
                    .map_err(|error| {
                        eprintln!("Could not load the Files profile: {error}");
                    })
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
                profile
            } else {
                None
            };
            let message = match command.status() {
                Ok(status) if status.success() => "The app closed before its window appeared.",
                Ok(status) => {
                    eprintln!("{} exited with {status}", app.name());
                    if app == App::Files && status.code() == Some(127) {
                        "Install thunar and try again."
                    } else {
                        "The app exited before its window appeared."
                    }
                }
                Err(error) => {
                    eprintln!("Could not open {}: {error}", app.name());
                    if app == App::Files && error.kind() == io::ErrorKind::NotFound {
                        "Install thunar and dbus-daemon, then try again."
                    } else {
                        "The app could not be started."
                    }
                }
            };
            let _ = exits.send(Exit {
                app,
                started,
                message,
            });
        })?;
    Ok(())
}

fn files_profile(command: &mut Command) -> io::Result<tempfile::TempDir> {
    let theme = tempfile::Builder::new()
        .prefix("vantage-theme-")
        .tempdir_in(glib::user_runtime_dir())?;
    let config = theme.path().join("config");
    let channel = config.join("xfce4/xfconf/xfce-perchannel-xml");
    fs::create_dir_all(&channel)?;
    fs::write(channel.join("thunar.xml"), include_str!("files.xml"))?;

    let gtk = theme.path().join("themes/Vantage-Session/gtk-3.0");
    fs::create_dir_all(&gtk)?;
    fs::write(gtk.join("gtk.css"), include_str!("files.css"))?;
    fs::write(
        gtk.join("settings.ini"),
        "[Settings]\ngtk-icon-theme-name=Adwaita\ngtk-decoration-layout=:close\n",
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
