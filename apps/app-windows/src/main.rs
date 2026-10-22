//! `yacr-windows` desktop binary.
//!
//! Delegates to [`app_windows::entry::run`]. On Windows it runs the real
//! Tauri/WebView2/wgpu host; on every other target it returns an explicit NOT
//! IMPLEMENTED error (no GUI dependencies are active there).
//!
//! The binary name is deliberately `yacr-windows`, distinct from the Linux
//! product binary `yacr` (`apps/app-linux`): both packages compile for the same
//! host target during workspace checks, so a shared name would make the two
//! hosts compete for (or overwrite) one output artifact path.

fn main() {
    if let Err(error) = app_windows::entry::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
