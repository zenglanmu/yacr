//! Linux implementation of the release-page platform seam.
//!
//! `xdg-open` is the freedesktop entry point to the user's default browser. It is
//! spawned and **not waited on**: the IPC answer means "handed to the OS", and a
//! browser that is slow to start must not block the WebView's invoke thread.
//!
//! A missing `xdg-open` (or a spawn failure) is returned as a real error, which
//! the shared command relays as `update.open_url_failed: ...`; it is never turned
//! into a fabricated `accepted:true`.

use cad_desktop_shell::release::ShellReleaseOpener;

/// Opens the fixed release URL through `xdg-open`.
pub struct XdgOpenReleaseOpener;

impl ShellReleaseOpener for XdgOpenReleaseOpener {
    fn open_url(&self, url: &str) -> Result<(), String> {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map(|_child| ())
            .map_err(|error| format!("xdg-open spawn failed: {error}"))
    }
}
