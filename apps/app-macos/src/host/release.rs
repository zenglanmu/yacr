//! macOS implementation of the release-page platform seam.
//!
//! `open <url>` hands the URL to LaunchServices, which opens it in the user's
//! default browser. The child is spawned, not waited on: the IPC answer means
//! "handed to the OS", not "the browser rendered".
//!
//! A spawn failure is returned as a real error, which the shared command relays
//! as `update.open_url_failed: ...`; it is never a fabricated `accepted:true`.

#[cfg(target_os = "macos")]
use cad_desktop_shell::release::ShellReleaseOpener;

/// Opens the fixed release URL through LaunchServices.
#[cfg(target_os = "macos")]
pub struct MacReleaseOpener;

#[cfg(target_os = "macos")]
impl ShellReleaseOpener for MacReleaseOpener {
    fn open_url(&self, url: &str) -> Result<(), String> {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map(|_child| ())
            .map_err(|error| format!("open spawn failed: {error}"))
    }
}
