//! The release-page shell capability shared by every native Tauri carrier.
//!
//! The in-app About/update surface asks the host to open the project's fixed
//! download page in the OS default browser. That is **shell capability**, not a
//! CAD wire command: it carries no document request, and the URL is a compile-time
//! constant, never caller input.
//!
//! * the opener is a platform seam ([`ShellReleaseOpener`]) installed by each
//!   concrete host in `apps/app-*`, so this crate stays free of platform I/O;
//! * a carrier with no opener (today: Android) keeps
//!   [`Shared::release_opener`](crate::state::Shared::release_opener) as `None`
//!   and the command refuses with a stable `unsupported-runtime` marker instead of
//!   pretending a browser opened;
//! * an opener failure is a real `update.open_url_failed` error, never swallowed.
//!
//! [`open_release_page`] is the testable body: it clones the opener out of
//! [`Shared`] and **releases the lock before calling it**, the same rule
//! `ShellPopupHost` follows, so a platform call that blocks (or re-enters) can
//! never deadlock the IPC thread.

use std::sync::{Arc, Mutex};

use tauri::ipc::Invoke;

use crate::state::{lock, ProbeShared, Shared};

/// The fixed download page. The OS always receives **this** value; the frontend
/// cannot pass a URL.
pub const RELEASE_PAGE_URL: &str = "https://yacr.pages.dev/#download";

/// The shell command name.
pub const CAD_OPEN_RELEASE_PAGE_COMMAND: &str = "cad_open_release_page";

/// The stable refusal when a carrier has no URL opener.
pub const NO_OPENER_REFUSAL: &str =
    "unsupported-runtime: cad_open_release_page: this host has no URL opener";

/// Platform seam: handed to the OS default browser. Ack means "handed to the
/// OS", not "the browser rendered".
pub trait ShellReleaseOpener: Send + Sync {
    fn open_url(&self, url: &str) -> Result<(), String>;
}

/// Whether `name` is this module's command.
pub fn is_release_command(name: &str) -> bool {
    name == CAD_OPEN_RELEASE_PAGE_COMMAND
}

/// Command body of [`cad_open_release_page`], without the Tauri state wrapper.
///
/// Split out so the tests drive the **same** implementation the IPC path runs.
/// The opener is cloned out and the shell lock is released before the platform
/// call: no lock is held while the OS is asked to open a browser.
pub fn open_release_page(shared: &Arc<Mutex<Shared>>) -> Result<serde_json::Value, String> {
    let opener = lock(shared).release_opener.clone();
    let Some(opener) = opener else {
        return Err(NO_OPENER_REFUSAL.to_string());
    };
    opener
        .open_url(RELEASE_PAGE_URL)
        .map_err(|error| format!("update.open_url_failed: {error}"))?;
    Ok(serde_json::json!({ "accepted": true }))
}

/// `cad_open_release_page`: hand the fixed download page to the OS browser.
#[tauri::command]
pub fn cad_open_release_page(
    shared: tauri::State<'_, ProbeShared>,
) -> Result<serde_json::Value, String> {
    open_release_page(&shared.0)
}

/// The release-page command as one typed handler, composed by
/// [`crate::vendor::register_commands_with_vendor`].
pub fn register_release_commands<R: tauri::Runtime>(
) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![cad_open_release_page]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake opener that records what the command handed it. It never touches
    /// the OS.
    #[derive(Default)]
    struct RecordingOpener {
        urls: Mutex<Vec<String>>,
        fail: Option<String>,
    }

    impl RecordingOpener {
        fn failing(error: &str) -> Self {
            Self {
                urls: Mutex::new(Vec::new()),
                fail: Some(error.to_string()),
            }
        }

        fn urls(&self) -> Vec<String> {
            self.urls.lock().expect("recording opener").clone()
        }
    }

    impl ShellReleaseOpener for RecordingOpener {
        fn open_url(&self, url: &str) -> Result<(), String> {
            self.urls
                .lock()
                .expect("recording opener")
                .push(url.to_string());
            match &self.fail {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }
    }

    fn shared() -> Arc<Mutex<Shared>> {
        Arc::new(Mutex::new(Shared::new(
            cad_ui_core::launch::LaunchOptions::default(),
        )))
    }

    #[test]
    fn only_the_one_name_is_the_release_command() {
        assert!(is_release_command(CAD_OPEN_RELEASE_PAGE_COMMAND));
        for name in ["cad_app_version", "cad_status", "cad_plugin_status", ""] {
            assert!(!is_release_command(name), "{name}");
        }
    }

    #[test]
    fn a_configured_opener_receives_the_fixed_url_and_is_acknowledged() {
        let shared = shared();
        let opener = Arc::new(RecordingOpener::default());
        crate::state::configure_release_opener(&shared, opener.clone());

        let answer = open_release_page(&shared).expect("acknowledged");
        assert_eq!(answer, serde_json::json!({ "accepted": true }));
        // Exactly one call, with the compile-time URL — never caller input.
        assert_eq!(opener.urls(), vec![RELEASE_PAGE_URL.to_string()]);
    }

    #[test]
    fn a_carrier_without_an_opener_refuses_with_the_stable_marker() {
        let shared = shared();
        let error = open_release_page(&shared).expect_err("no opener must refuse");
        assert_eq!(error, NO_OPENER_REFUSAL);
        assert!(error.starts_with("unsupported-runtime:"));
    }

    #[test]
    fn an_opener_failure_is_relayed_verbatim() {
        let shared = shared();
        crate::state::configure_release_opener(
            &shared,
            Arc::new(RecordingOpener::failing("xdg-open: not found")),
        );
        let error = open_release_page(&shared).expect_err("opener failure is not a success");
        assert_eq!(error, "update.open_url_failed: xdg-open: not found");
    }
}
