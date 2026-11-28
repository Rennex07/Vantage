mod console;
mod desktop;
mod frame_metrics;
mod lock_auth;
mod native;
mod nested;
mod session;
mod shell;
mod wallpaper;
mod wayland;

use vantage::{palette, preferences};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let backend = arguments.next();
    if backend.as_deref() == Some(std::ffi::OsStr::new("--render-boot-wordmark")) {
        let path = arguments.next().ok_or("Missing wordmark PNG path")?;
        if arguments.next().is_some() {
            return Err("Expected one wordmark PNG path".into());
        }
        return shell::lockscreen::boot_wordmark(std::path::Path::new(&path));
    }
    if backend.as_deref() == Some(std::ffi::OsStr::new("--launch-pinned")) {
        let id = arguments.next().ok_or("Missing desktop entry ID")?;
        if arguments.next().is_some() {
            return Err("Expected one desktop entry ID".into());
        }
        return desktop::launch_pinned(id.to_str().ok_or("Invalid desktop entry ID")?);
    }
    if arguments.next().is_some() {
        return Err("Usage: vantage [--nested | --tty]".into());
    }
    match backend.as_deref().and_then(std::ffi::OsStr::to_str) {
        Some("--tty") => native::run(),
        Some("--authenticate-lock") => std::process::exit(lock_auth::run_helper()),
        Some("--console") => std::process::exit(i32::from(console::run(false).get())),
        Some("--files") => {
            let code = match shell::launch::run_files() {
                Ok(status) => status.code().unwrap_or(1),
                Err(error) => {
                    eprintln!("Could not open Files: {error}");
                    if error.kind() == std::io::ErrorKind::NotFound {
                        127
                    } else {
                        1
                    }
                }
            };
            std::process::exit(code);
        }
        Some("--network") => std::process::exit(i32::from(console::run(true).get())),
        Some("--apps") => std::process::exit(i32::from(desktop::run_launcher().get())),
        Some("--settings") => std::process::exit(i32::from(desktop::run_settings(false).get())),
        Some("--sound") => std::process::exit(i32::from(desktop::run_settings(true).get())),
        Some("--nested") => nested::run(),
        None if backend.is_none() => nested::run(),
        Some("--help" | "-h") => {
            println!("Usage: vantage [--nested | --tty]");
            Ok(())
        }
        _ => Err("Usage: vantage [--nested | --tty]".into()),
    }
}
