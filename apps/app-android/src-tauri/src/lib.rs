//! Android host for the CAD viewer/measure/annotation app.
//!
//! The host is a **Tauri 2 WebView/asset carrier**: the shared React chrome
//! (`apps/desktop-ui`, built with `YACR_RUNTIME=android` into `../dist`) runs in
//! the Android WebView and the **wasm `dom-canvas` CAD session owns the document
//! and the pixels** (ADR 0024; `AGENTS.md` iron rule 8). The native side holds no
//! CAD state, opens no CAD file, instantiates no second session and registers no
//! CAD command: it keeps WebView/Activity lifetime and the two shell-only reports
//! the chrome sends on this carrier ([`host`]).
//!
//! On every other target this crate is a documented empty shell: `tauri` is an
//! Android-only dependency, so a host `--all-targets` check and the
//! `wasm32-unknown-unknown --lib` gate compile the module below instead of
//! pretending an Android app can run there. [`start`] is the explicit
//! NOT IMPLEMENTED answer for those targets, not an empty success.

pub mod refusal;
pub mod report;

#[cfg(target_os = "android")]
pub mod host;

#[cfg(target_os = "android")]
pub use host::run;

/// What this crate reports when it is asked to run somewhere it cannot: an
/// Android host is only buildable for Android targets.
pub const NON_ANDROID_DIAGNOSTIC: &str =
    "app-android: an Android host; build with `tauri android build` (NOT IMPLEMENTED for this target)";

/// Explicit refusal for non-Android targets.
///
/// There is no binary here, so nothing user-facing calls this: it exists so a
/// future caller gets a modeled failure instead of an empty `Ok(())`.
#[cfg(not(target_os = "android"))]
pub fn start() -> Result<(), String> {
    Err(NON_ANDROID_DIAGNOSTIC.to_string())
}
