use std::{
    env,
    ffi::OsStr,
    io,
    process::{Child, Command},
    thread,
};

use vte4::gtk::{self, gio, glib, prelude::*};

use crate::preferences::{self, DockPin, SETTINGS_APP_ID};

mod settings;
pub mod x11;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Apps,
    Network,
    Bluetooth,
    Sound,
    Mixer,
    Settings,
}

impl Tool {
    pub fn open(self, socket: &OsStr) -> io::Result<()> {
        self.open_with_x11(socket, None)
    }

    pub fn open_with_x11(self, socket: &OsStr, x11: Option<&x11::X11>) -> io::Result<()> {
        let mut command = match self {
            Self::Apps | Self::Network | Self::Settings | Self::Sound => {
                let mut command = Command::new(env::current_exe()?);
                command.arg(match self {
                    Self::Apps => "--apps",
                    Self::Network => "--network",
                    Self::Sound => "--sound",
                    _ => "--settings",
                });
                command
            }
            Self::Bluetooth => Command::new("blueman-manager"),
            Self::Mixer => Command::new("pavucontrol"),
        };
        wayland(&mut command, socket);
        if let Some(x11) = x11 {
            x11.configure(&mut command);
        }
        let mut child = command.spawn()?;
        thread::spawn(move || {
            if let Ok(status) = child.wait()
                && !status.success()
            {
                eprintln!("{self:?} exited with {status}");
            }
        });
        Ok(())
    }
}

fn wayland(command: &mut Command, socket: &OsStr) {
    command
        .env("WAYLAND_DISPLAY", socket)
        .env("GDK_BACKEND", "wayland")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("DISPLAY");
}

pub struct Services(Option<Child>);

impl Services {
    pub fn start(socket: &OsStr, x11: Option<&x11::X11>) -> Self {
        let mut activation = Command::new("dbus-update-activation-environment");
        activation
            .args([
                "--systemd",
                "WAYLAND_DISPLAY",
                "XDG_CURRENT_DESKTOP",
                "XDG_SESSION_TYPE",
            ])
            .env("WAYLAND_DISPLAY", socket)
            .env("XDG_CURRENT_DESKTOP", "Vantage")
            .env("XDG_SESSION_TYPE", "wayland");
        if let Some(x11) = x11 {
            x11.configure(&mut activation);
            activation.args(["DISPLAY", "XAUTHORITY"]);
        } else {
            activation.args(["DISPLAY=", "XAUTHORITY="]);
        }
        match activation.status() {
            Ok(status) if status.success() => (),
            result => eprintln!("Could not publish the Wayland session environment: {result:?}"),
        }
        let mut agent = Command::new("lxpolkit");
        wayland(&mut agent, socket);
        Self(
            agent
                .spawn()
                .map_err(|error| {
                    eprintln!("Could not start the authentication agent: {error}");
                })
                .ok(),
        )
    }
}

impl Drop for Services {
    fn drop(&mut self) {
        if let Some(agent) = &mut self.0 {
            let _ = agent.kill();
            let _ = agent.wait();
        }
    }
}

pub fn run_launcher() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("org.vantage.Apps")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(show_launcher);
    app.run_with_args(&["vantage-apps"])
}

pub fn run_settings(sound: bool) -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("org.vantage.Settings")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| settings::show(app, sound));
    app.run_with_args(&["vantage-settings"])
}

fn show_launcher(app: &gtk::Application) {
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search apps and settings")
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .build();
    list.add_css_class("boxed-list");
    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list)
        .build();
    let error = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .build();
    error.add_css_class("error");
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_margin_start(16);
    body.set_margin_end(16);
    body.set_margin_top(14);
    body.set_margin_bottom(16);
    body.append(&search);
    body.append(&scroll);
    body.append(&error);
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Applications")
        .titlebar(&gtk::HeaderBar::new())
        .default_width(474)
        .default_height(460)
        .child(&body)
        .build();
    install_style(&window);

    let mut apps: Vec<_> = gio::AppInfo::all()
        .into_iter()
        .filter(|app| {
            app.should_show() && !app.id().is_some_and(|id| id == "vantage-current.desktop")
        })
        .map(|app| (app.id().map(|id| id.to_string()), Some(app)))
        .collect();
    if let Ok(executable) = env::current_exe()
        && let Ok(settings) = gio::AppInfo::create_from_commandline(
            format!(
                "{} --settings",
                glib::shell_quote(executable).to_string_lossy()
            ),
            Some("Settings"),
            gio::AppInfoCreateFlags::NONE,
        )
    {
        apps.push((Some(SETTINGS_APP_ID.into()), Some(settings)));
    }
    let pins = preferences::dock_pins();
    for pin in &pins {
        if !apps.iter().any(|(id, _)| id.as_deref() == Some(&pin.id)) {
            apps.push((Some(pin.id.clone()), None));
        }
    }
    let title_for = |(id, app): &(Option<String>, Option<gio::AppInfo>)| {
        app.as_ref().map_or_else(
            || {
                id.as_deref()
                    .unwrap_or_default()
                    .trim_end_matches(".desktop")
                    .into()
            },
            app_name,
        )
    };
    apps.sort_by_cached_key(|entry| title_for(entry).to_lowercase());
    let theme = gtk::IconTheme::for_display(&WidgetExt::display(&window));
    for entry in &apps {
        let (id, app) = entry;
        let name = title_for(entry);
        let settings = id.as_deref() == Some(SETTINGS_APP_ID);
        let app_icon = app
            .as_ref()
            .and_then(|app| app.icon())
            .or_else(|| settings.then(|| gio::ThemedIcon::new("preferences-system").upcast()));
        let icon = app_icon.as_ref().map_or_else(
            || gtk::Image::from_icon_name("application-x-executable"),
            gtk::Image::from_gicon,
        );
        icon.set_pixel_size(30);
        let title = gtk::Label::builder()
            .label(name.as_str())
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .hexpand(true)
            .build();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_margin_start(12);
        row.set_margin_end(12);
        row.set_margin_top(8);
        row.set_margin_bottom(8);
        row.append(&icon);
        row.append(&title);
        if let Some(id) = id.as_deref().filter(|id| preferences::valid_dock_id(id)) {
            let pin = DockPin {
                id: id.to_owned(),
                icon: app_icon.as_ref().and_then(|icon| {
                    theme
                        .lookup_by_gicon(
                            icon,
                            48,
                            2,
                            gtk::TextDirection::None,
                            gtk::IconLookupFlags::FORCE_REGULAR,
                        )
                        .file()
                        .and_then(|file| file.path())
                }),
            };
            let button = gtk::Button::new();
            let set_label = move |button: &gtk::Button, pinned| {
                button.set_label(if pinned { "Unpin" } else { "Pin" });
                let action = if pinned {
                    format!("Unpin {name} from the dock")
                } else {
                    format!("Pin {name} to the dock")
                };
                button.set_tooltip_text(Some(&action));
                button.update_property(&[gtk::accessible::Property::Label(&action)]);
            };
            set_label(&button, pins.iter().any(|entry| entry.id == id));
            let error = error.clone();
            let installed = app.is_some();
            button.connect_clicked(move |button| {
                let pinned = button.label().as_deref() == Some("Unpin");
                if !pinned && !installed {
                    error.set_label("This application is no longer available.");
                    error.set_visible(true);
                    return;
                }
                match preferences::set_dock_pin(&pin, !pinned) {
                    Ok(()) => {
                        set_label(button, !pinned);
                        error.set_visible(false);
                    }
                    Err(failure) => {
                        error.set_label(&format!("Could not change the dock. {failure}"));
                        error.set_visible(true);
                    }
                }
            });
            row.append(&button);
        }
        list.append(&row);
    }
    let search_for_filter = search.downgrade();
    let names: Vec<_> = apps
        .iter()
        .map(|entry| {
            let (id, app) = entry;
            let name = title_for(entry);
            app.as_ref().map_or_else(
                || name.to_lowercase(),
                |app| {
                    format!(
                        "{name} {} {}",
                        app.display_name(),
                        id.as_deref().unwrap_or_default()
                    )
                    .to_lowercase()
                },
            )
        })
        .collect();
    list.set_filter_func(move |row| {
        search_for_filter.upgrade().is_some_and(|search| {
            names[row.index() as usize].contains(&search.text().trim().to_lowercase())
        })
    });
    let search_list = list.downgrade();
    search.connect_search_changed(move |_| {
        if let Some(list) = search_list.upgrade() {
            list.invalidate_filter();
            let first = (0..)
                .map_while(|index| list.row_at_index(index))
                .find(|row| row.is_child_visible());
            list.select_row(first.as_ref());
        }
    });
    let search_list = list.downgrade();
    search.connect_activate(move |_| {
        if let Some(list) = search_list.upgrade()
            && let Some(row) = list.selected_row()
        {
            list.emit_by_name::<()>("row-activated", &[&row]);
        }
    });
    let launcher = window.downgrade();
    search.connect_stop_search(move |_| {
        if let Some(window) = launcher.upgrade() {
            window.close();
        }
    });
    let launcher = window.downgrade();
    list.connect_row_activated(move |_, row| {
        let Some(window) = launcher.upgrade() else {
            return;
        };
        let (_, Some(app)) = &apps[row.index() as usize] else {
            error.set_label("This application is no longer available.");
            error.set_visible(true);
            return;
        };
        let context = WidgetExt::display(&window).app_launch_context();
        context.unsetenv("GDK_BACKEND");
        let result = if is_files(app) {
            env::current_exe()
                .and_then(|program| Command::new(program).arg("--files").spawn())
                .map(|mut child| {
                    thread::spawn(move || {
                        let _ = child.wait();
                    });
                })
                .map_err(|failure| failure.to_string())
        } else {
            app.launch(&[], Some(&context))
                .map_err(|failure| failure.to_string())
        };
        match result {
            Ok(()) => window.close(),
            Err(failure) => {
                error.set_label(&format!("Could not open {}. {failure}", app_name(app)));
                error.set_visible(true);
            }
        }
    });
    list.select_row(list.row_at_index(0).as_ref());
    window.present();
    search.grab_focus();
}

pub fn launch_pinned(id: &str) -> Result<(), Box<dyn std::error::Error>> {
    if !preferences::valid_dock_id(id) {
        return Err("Invalid desktop entry ID".into());
    }
    let app = crate::shell::launch::desktop_app(id).ok_or("Application is no longer installed")?;
    app.launch(&[], None::<&gio::AppLaunchContext>)?;
    Ok(())
}

fn is_files(app: &gio::AppInfo) -> bool {
    app.id()
        .is_some_and(|id| matches!(id.as_str(), "thunar.desktop" | "org.xfce.Thunar.desktop"))
}

fn app_name(app: &gio::AppInfo) -> glib::GString {
    if is_files(app) {
        "Files".into()
    } else {
        app.display_name()
    }
}

fn install_style(window: &gtk::ApplicationWindow) {
    let display = WidgetExt::display(window);
    let style = gtk::CssProvider::new();
    let mut current = preferences::appearance();
    apply_style(&style, current);
    gtk::style_context_add_provider_for_display(
        &display,
        &style,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let window = window.downgrade();
    glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
        if window.upgrade().is_none() {
            gtk::style_context_remove_provider_for_display(&display, &style);
            return glib::ControlFlow::Break;
        }
        let next = preferences::appearance();
        if current != next {
            current = next;
            apply_style(&style, current);
        }
        glib::ControlFlow::Continue
    });
}

fn apply_style(style: &gtk::CssProvider, appearance: preferences::Appearance) {
    use crate::palette::{Mode, hex};
    let theme = appearance.theme;
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(theme.mode == Mode::Dark);
    }
    let colors = theme.colors();
    let base = hex(colors.base);
    let surface = hex(colors.surface);
    let raised = hex(colors.raised);
    let selected = hex(colors.selected);
    let text = hex(colors.text);
    let muted = hex(colors.muted);
    let accent = hex(colors.accent);
    let warning = hex(colors.warning);
    let duration = if appearance.reduce_motion { 0 } else { 200 };
    let tint =
        if appearance.reduce_transparency || appearance.effects == preferences::Effects::Reduced {
            1.0
        } else {
            0.80
        };
    style.load_from_string(&format!(
        "window {{ background: alpha({base}, {tint}); color: {text}; font-family: Roboto, sans-serif; }}
         .title-1, .title-2 {{ font-weight: 900; }}
         button, row, flowboxchild {{ transition: background-color {duration}ms ease-out; }}
         headerbar {{ background: linear-gradient(shade({raised}, 1.16), {surface}); color: {text};
                      border-bottom: 1px solid {selected}; box-shadow: inset 0 2px alpha(white, 0.45); }}
         searchentry, dropdown button, popover contents, .boxed-list,
         button {{ background: {surface}; color: {text}; }}
         button, searchentry, dropdown button {{
             background: linear-gradient({raised}, {surface}); border-color: {selected};
             border-top-color: alpha(white, 0.40); border-radius: 16px;
         }}
         button:hover, .boxed-list row:hover {{ background: {raised}; }}
         button:focus-visible, searchentry:focus-within, dropdown button:focus-visible {{
             outline: 2px solid {accent}; outline-offset: -2px;
         }}
         .settings-row {{ background: linear-gradient({raised}, {surface}); color: {text};
                          border: 1px solid {selected}; border-top-color: alpha(white, 0.45);
                          border-radius: 18px; padding: 11px 14px; }}
         .settings-row:hover {{ background: {selected}; }}
         .boxed-list row:selected, .vantage-settings stackswitcher button:checked {{
             background: {selected}; color: {text};
         }}
         .color-grid flowboxchild {{
             background: {surface}; color: {text}; border: 1px solid {selected};
             border-radius: 16px; padding: 9px;
         }}
         .color-grid flowboxchild:hover {{ background: {raised}; }}
         .color-grid flowboxchild:selected {{ background: {selected}; border-color: {accent}; }}
         .color-grid flowboxchild:focus-visible {{ outline: 2px solid {accent}; outline-offset: -2px; }}
         .vantage-settings scale trough {{ background: {raised}; }}
         .vantage-settings scale highlight, .vantage-settings scale slider {{ background: {accent}; }}
         .dim-label {{ color: {muted}; opacity: 1; }}
         .error {{ color: {warning}; }}"
    ));
}
