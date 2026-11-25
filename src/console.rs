use std::{env, fs, os::unix::fs::PermissionsExt, path::Path};

use vte4::{
    PtyFlags, Terminal,
    gtk::{self, gdk, gio, glib, pango},
    prelude::*,
};

const SHELL_FALLBACK: &str = "/bin/sh";

pub fn run() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("org.vantage.Console")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(open_console);
    app.run_with_args(&["vantage-console"])
}

fn open_console(app: &gtk::Application) {
    let terminal = Terminal::new();
    terminal.set_font(Some(&pango::FontDescription::from_string(
        "JetBrains Mono 11",
    )));
    terminal.set_focusable(true);
    terminal.set_scrollback_lines(10_000);
    terminal.set_scroll_on_keystroke(true);
    terminal.set_mouse_autohide(true);
    let ink = gdk::RGBA::new(238.0 / 255.0, 244.0 / 255.0, 1.0, 1.0);
    let navy = gdk::RGBA::new(13.0 / 255.0, 22.0 / 255.0, 41.0 / 255.0, 1.0);
    let blue = gdk::RGBA::new(145.0 / 255.0, 191.0 / 255.0, 250.0 / 255.0, 1.0);
    terminal.set_color_foreground(&ink);
    terminal.set_color_background(&navy);
    terminal.set_color_cursor(Some(&blue));
    terminal.set_margin_top(8);
    terminal.set_margin_bottom(8);
    terminal.set_margin_start(12);
    terminal.set_margin_end(12);

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&terminal)
        .build();
    let title = gtk::Label::new(Some("Console"));
    title.add_css_class("console-title");
    let mark = gtk::Label::new(Some("›_"));
    mark.add_css_class("console-mark");
    let header = gtk::HeaderBar::builder()
        .title_widget(&title)
        .decoration_layout(":close")
        .build();
    header.pack_start(&mark);
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Console")
        .titlebar(&header)
        .default_width(760)
        .default_height(420)
        .child(&scroller)
        .build();
    let style = gtk::CssProvider::new();
    style.load_from_string(
        "window { background: #0d1629; color: #eef4ff; }
         scrolledwindow { background: #0d1629; }
         headerbar { min-height: 42px; padding: 0 8px;
                     background: linear-gradient(#233556, #1b2c4d); color: #eef4ff;
                     border-bottom: 1px solid #375579; box-shadow: none; }
         .console-title { font-weight: 700; }
         .console-mark { font-family: monospace; font-weight: bold; font-size: 14px;
                         color: #d9eaff; background: #304d77; border-radius: 8px;
                         min-width: 26px; margin: 7px 4px; }
         headerbar button { color: #eef4ff; }
         headerbar button:hover { background: #3a5171; }
         scrollbar slider { background: #526e97; min-width: 5px; }",
    );
    gtk::style_context_add_provider_for_display(
        &WidgetExt::display(&window),
        &style,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    bind_clipboard_keys(&terminal);

    let window_to_close = window.downgrade();
    terminal.connect_child_exited(move |_, _| {
        if let Some(window) = window_to_close.upgrade() {
            window.close();
        }
    });
    // Present the window once VTE has a PTY and can accept input.
    start_login_shell(&terminal, &window);
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
    let home = glib::home_dir();
    let view = terminal.downgrade();
    let window = window.downgrade();
    terminal.spawn_async(
        PtyFlags::DEFAULT,
        home.to_str().filter(|path| Path::new(path).is_dir()),
        &[shell, &login_name],
        &[],
        glib::SpawnFlags::FILE_AND_ARGV_ZERO,
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
                terminal
                    .feed(format!("Could not start the login shell.\r\n{error}\r\n").as_bytes());
            }
            GtkWindowExt::set_focus(&window, Some(&terminal));
            window.present();
            terminal.grab_focus();
        },
    );
}
