//! `yacr` desktop binary.
//!
//! Delegates to [`app_linux::entry::run`]. Linux runs the real Tauri/GTK/wgpu
//! host; every other target returns an explicit NOT IMPLEMENTED error (no GUI
//! dependencies are active there).

fn main() {
    if let Err(error) = app_linux::entry::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
