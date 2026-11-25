#[path = "../console.rs"]
mod console;

fn main() -> vte4::gtk::glib::ExitCode {
    console::run()
}
