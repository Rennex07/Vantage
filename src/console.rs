use std::{env, fs, os::unix::fs::PermissionsExt, path::Path};

use vte4::{
    PtyFlags, Terminal,
    gtk::{self, gdk, gio, glib, pango},
    prelude::*,
};

use crate::{
    palette::{Mode, Theme, hex},
    preferences,
};

const SHELL_FALLBACK: &str = "/bin/sh";
const NETWORK_COLORS: &str = "NEWT_COLORS=+root=white,black:border=gray,blue:window=white,blue:shadow=white,black:+title=brightcyan,blue:button=white,cyan:actbutton=white,magenta:+checkbox=white,cyan:actcheckbox=white,magenta:entry=white,black:+label=lightgray,blue:listbox=white,blue:actlistbox=white,magenta:+textbox=white,blue:acttextbox=white,magenta:emptyscale=gray,blue:+disentry=gray,blue:compactbutton=white,blue:actsellistbox=white,magenta:+sellistbox=brightcyan,blue";

pub fn run(network: bool) -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id(if network {
            "org.vantage.Network"
        } else {
            "org.vantage.Console"
        })
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| open_console(app, network));
    app.run_with_args(&["vantage-console"])
}

fn open_console(app: &gtk::Application, network: bool) {
    let terminal = Terminal::new();
    terminal.set_font(Some(&pango::FontDescription::from_string(
        "JetBrains Mono 11",
    )));
    terminal.set_focusable(true);
    terminal.set_scrollback_lines(10_000);
    terminal.set_scroll_on_keystroke(true);
    terminal.set_mouse_autohide(true);
    terminal.set_margin_top(8);
    terminal.set_margin_bottom(8);
    terminal.set_margin_start(12);
    terminal.set_margin_end(12);

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&terminal)
        .build();
    let name = if network {
        "Network & Wi-Fi"
    } else {
        "Console"
    };
    let title = gtk::Label::new(Some(name));
    title.add_css_class("console-title");
    let mark = gtk::Label::new(Some(if network { "⌁" } else { "›_" }));
    mark.add_css_class("console-mark");
    let header = gtk::HeaderBar::builder()
        .title_widget(&title)
        .decoration_layout(":minimize,maximize,close")
        .build();
    header.pack_start(&mark);
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(name)
        .titlebar(&header)
        .default_width(760)
        .default_height(420)
        .child(&scroller)
        .build();
    follow_appearance(&terminal, &window, network);

    bind_clipboard_keys(&terminal);

    let window_to_close = window.downgrade();
    terminal.connect_child_exited(move |_, _| {
        if let Some(window) = window_to_close.upgrade() {
            window.close();
        }
    });
    // Present the window once VTE has a PTY and can accept input.
    if network {
        spawn(
            &terminal,
            &window,
            &["nmtui"],
            &[NETWORK_COLORS],
            glib::SpawnFlags::SEARCH_PATH,
            false,
            "Could not start NetworkManager settings. Install network-manager.",
        );
    } else {
        start_login_shell(&terminal, &window);
    }
}

fn follow_appearance(terminal: &Terminal, window: &gtk::ApplicationWindow, network: bool) {
    let style = gtk::CssProvider::new();
    let display = WidgetExt::display(window);
    let mut current = preferences::appearance().theme;
    apply_theme(terminal, &style, current, network);
    gtk::style_context_add_provider_for_display(
        &display,
        &style,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let terminal = terminal.downgrade();
    glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
        let Some(terminal) = terminal.upgrade() else {
            gtk::style_context_remove_provider_for_display(&display, &style);
            return glib::ControlFlow::Break;
        };
        let next = preferences::appearance().theme;
        if next != current {
            current = next;
            apply_theme(&terminal, &style, current, network);
        }
        glib::ControlFlow::Continue
    });
}

fn apply_theme(terminal: &Terminal, style: &gtk::CssProvider, theme: Theme, network: bool) {
    let colors = theme.colors();
    let [base, surface, raised, selected, text, accent, muted] = [
        colors.base,
        colors.surface,
        colors.raised,
        colors.selected,
        colors.text,
        colors.accent,
        colors.muted,
    ]
    .map(hex);
    let rgba = |color: &str| gdk::RGBA::parse(color).expect("valid terminal color");
    let palette = match (network, theme.mode) {
        (true, _) => [
            "#0d1629", "#ab515c", "#488065", "#a47a55", "#1b2c4d", "#395579", "#293f5d", "#b4c6df",
            "#6981a6", "#ff9da4", "#86cda9", "#ecb77c", "#669cec", "#ba9ce0", "#91bffa", "#eef4ff",
        ],
        (false, Mode::Dark) => [
            "#141922", "#e77f88", "#87ca9d", "#e5b77c", "#8ab3fd", "#cfa1dd", "#8ed3db", "#dae0e8",
            "#788397", "#ff9ba5", "#a9e4b3", "#f7d09b", "#abc9ff", "#e4b6ee", "#a9e5eb", "#eff3f8",
        ],
        (false, Mode::Light) => [
            "#18212d", "#9e3045", "#2f6747", "#81521c", "#1c56a4", "#793580", "#286469", "#d6dbe1",
            "#536173", "#98243c", "#286546", "#714b15", "#164b93", "#76327d", "#245b68", "#f5f5f7",
        ],
    }
    .map(rgba);
    terminal.set_colors(
        Some(&rgba(if network { "#eef4ff" } else { &text })),
        Some(&rgba(if network { "#0d1629" } else { &base })),
        &palette.each_ref(),
    );
    terminal.set_color_cursor(Some(&rgba(&accent)));
    style.load_from_string(&format!(
        "window, scrolledwindow {{ background: {base}; color: {text}; }}
         headerbar {{ min-height: 42px; padding: 0 8px; font-family: Roboto, sans-serif;
                      background: linear-gradient(shade({raised}, 1.16), {surface}); color: {text};
                      border-bottom: 1px solid {selected}; box-shadow: inset 0 2px alpha(white, 0.45); }}
         .console-title {{ font-weight: 900; }}
         .console-mark {{ font-family: monospace; font-weight: bold; font-size: 14px;
                          color: {text}; background: {selected}; border-radius: 8px;
                          min-width: 26px; margin: 7px 4px; box-shadow: inset 0 1px alpha(white, 0.45); }}
         headerbar button {{ color: {text}; }}
         headerbar button:hover {{ background: {selected}; }}
         scrollbar slider {{ background: {muted}; min-width: 5px; }}"
    ));
}

fn bind_clipboard_keys(terminal: &Terminal) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let target = terminal.downgrade();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let shortcut = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK;
        if !modifiers.contains(shortcut) {
            return glib::Propagation::Proceed;
        }
        if let Some(terminal) = target.upgrade() {
            match key.to_lower() {
                gdk::Key::c => terminal.copy_clipboard_format(vte4::Format::Text),
                gdk::Key::v => terminal.paste_clipboard(),
                _ => return glib::Propagation::Proceed,
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    terminal.add_controller(keys);
}

fn start_login_shell(terminal: &Terminal, window: &gtk::ApplicationWindow) {
    let shell = env::var("SHELL")
        .ok()
        .filter(|shell| {
            Path::new(shell).is_absolute()
                && fs::metadata(shell).is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                })
        })
        .unwrap_or_else(|| SHELL_FALLBACK.into());
    spawn_shell(terminal, window, &shell);
}

fn spawn_shell(terminal: &Terminal, window: &gtk::ApplicationWindow, shell: &str) {
    let can_fallback = shell != SHELL_FALLBACK;
    let name = Path::new(shell)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("sh");
    let login_name = format!("-{name}");
    spawn(
        terminal,
        window,
        &[shell, &login_name],
        &[],
        glib::SpawnFlags::FILE_AND_ARGV_ZERO,
        can_fallback,
        "Could not start the login shell.",
    );
}

fn spawn(
    terminal: &Terminal,
    window: &gtk::ApplicationWindow,
    command: &[&str],
    environment: &[&str],
    flags: glib::SpawnFlags,
    can_fallback: bool,
    error_message: &'static str,
) {
    let home = glib::home_dir();
    let view = terminal.downgrade();
    let window = window.downgrade();
    terminal.spawn_async(
        PtyFlags::DEFAULT,
        home.to_str().filter(|path| Path::new(path).is_dir()),
        command,
        environment,
        flags,
        || {},
        -1,
        gio::Cancellable::NONE,
        move |result| {
            let (Some(terminal), Some(window)) = (view.upgrade(), window.upgrade()) else {
                return;
            };
            if let Err(error) = result {
                if can_fallback {
                    spawn_shell(&terminal, &window, SHELL_FALLBACK);
                    return;
                }
                terminal.set_input_enabled(false);
                terminal.feed(format!("{error_message}\r\n{error}\r\n").as_bytes());
            }
            GtkWindowExt::set_focus(&window, Some(&terminal));
            window.present();
            terminal.grab_focus();
        },
    );
}
