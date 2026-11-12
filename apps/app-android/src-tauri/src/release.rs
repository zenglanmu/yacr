//! Android opener for the shell's fixed landing/download page.
//!
//! The URL is the shared compile-time constant
//! [`cad_desktop_shell::release::RELEASE_PAGE_URL`], handed over by the shared
//! command body; it is **never** caller input and this module has no second URL.
//!
//! The bridge is `MainActivity.openExternalUrl(String): Boolean` (Rust → Kotlin,
//! an instance method reached through [`crate::saf::with_env`] — the same captured
//! `JavaVM` + Activity global reference the SAF bridge uses). The Activity method
//! marshals the `ACTION_VIEW` intent onto the UI thread, because JNI calls arrive
//! from a worker/tick thread, and reports whether the system accepted it.
//!
//! A rejection — no browser activity, a finishing/destroyed activity, or the
//! bounded 2 s UI-thread answer expiring — is an explicit error. The shared
//! command wraps it as `update.open_url_failed: …`, so the message here stays free
//! of that prefix, and nothing is ever reported as a silent success.

use cad_desktop_shell::release::ShellReleaseOpener;
use jni::objects::JValue;

/// `MainActivity.openExternalUrl(String): Boolean`.
const OPEN_EXTERNAL_URL_METHOD: &str = "openExternalUrl";

/// Hands the fixed page to the Android system browser.
pub struct AndroidReleaseOpener;

impl ShellReleaseOpener for AndroidReleaseOpener {
    fn open_url(&self, url: &str) -> Result<(), String> {
        let handed = crate::saf::with_env(|env, activity| {
            let url = env.new_string(url)?;
            env.call_method(
                activity,
                OPEN_EXTERNAL_URL_METHOD,
                "(Ljava/lang/String;)Z",
                &[JValue::Object(url.as_ref())],
            )?
            .z()
        })?;
        if handed {
            Ok(())
        } else {
            Err(
                "the Android system did not accept the URL (no browser activity, or the activity \
                 is finishing)"
                    .to_string(),
            )
        }
    }
}
