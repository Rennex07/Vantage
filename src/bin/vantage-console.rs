#[path = "../console.rs"]
mod console;

use vantage::{palette, preferences};

fn main() -> vte4::gtk::glib::ExitCode {
    console::run(false)
}
