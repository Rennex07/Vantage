mod console;
mod native;
mod nested;
mod session;
mod shell;
mod wallpaper;
mod wayland;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let backend = arguments.next();
    if arguments.next().is_some() {
        return Err("Usage: vantage [--nested | --tty]".into());
    }
    match backend.as_deref().and_then(std::ffi::OsStr::to_str) {
        Some("--tty") => native::run(),
        Some("--console") => std::process::exit(i32::from(console::run().get())),
        Some("--nested") => nested::run(),
        None if backend.is_none() => nested::run(),
        Some("--help" | "-h") => {
            println!("Usage: vantage [--nested | --tty]");
            Ok(())
        }
        _ => Err("Usage: vantage [--nested | --tty]".into()),
    }
}
