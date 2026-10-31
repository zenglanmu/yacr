//! `yacr-macos` desktop binary.
//!
//! Delegates to [`app_macos::entry::run`]. On macOS it runs the real
//! Tauri/WKWebView/wgpu-Metal host; on every other target it returns an explicit
//! NOT IMPLEMENTED error (no GUI dependencies are active there).
//!
//! The binary name is deliberately `yacr-macos`, distinct from the Linux
//! product binary `yacr` (`apps/app-linux`): every host package compiles for the
//! same host target during workspace checks, so a shared name would make the
//! packages compete for (or overwrite) one output artifact path.

fn main() {
    if let Err(error) = app_macos::entry::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
