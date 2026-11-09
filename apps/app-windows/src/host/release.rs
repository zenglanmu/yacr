//! Windows implementation of the release-page platform seam.
//!
//! `cmd /C start "" <url>` is the std-only way to hand a URL to the shell's
//! default browser without linking Win32 APIs (whose signatures cannot be
//! verified on this Linux host). The empty title argument is required so `start`
//! does not treat the URL as the window title. The child is spawned, not waited
//! on: the IPC answer means "handed to the OS", not "the browser rendered".
//!
//! A spawn failure is returned as a real error, which the shared command relays
//! as `update.open_url_failed: ...`; it is never a fabricated `accepted:true`.

#[cfg(windows)]
use cad_desktop_shell::release::ShellReleaseOpener;

/// Opens the fixed release URL through the shell's default browser.
#[cfg(windows)]
pub struct WindowsReleaseOpener;

#[cfg(windows)]
impl ShellReleaseOpener for WindowsReleaseOpener {
    fn open_url(&self, url: &str) -> Result<(), String> {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map(|_child| ())
            .map_err(|error| format!("cmd start spawn failed: {error}"))
    }
}
