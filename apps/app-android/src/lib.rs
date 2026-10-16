//! Android host — RETIRED: an explicit NOT IMPLEMENTED placeholder.
//!
//! The Android UI has been retired. The Slint `android-activity` boot chain
//! (`android_main` → `slint::android::init` → `start()`), the
//! `UiAdapter`/`EngineAdapter` host wiring, the SAF `jni` file bridge, the font
//! asset loader and the Slint poll timer are all gone, so this crate no longer
//! boots a UI.
//!
//! What remains is an explicit refusal: [`android_main`] reports the retirement
//! and then aborts the process instead of showing a blank shell or returning an
//! empty success. Nothing here fakes a started host.

/// The stable retirement diagnostic reported by the native entry point.
pub const RETIRED_DIAGNOSTIC: &str = "Android UI retired — NOT IMPLEMENTED";

/// Explicit refusal of the retired Android UI.
///
/// Host-target builds get a plain modeled failure instead of an empty success
/// path; the native entry point reports the same diagnostic.
pub fn start() -> Result<(), String> {
    Err(RETIRED_DIAGNOSTIC.to_string())
}

/// Android entry point (`android-activity` 0.6 calls `fn android_main(app)`).
///
/// The UI is retired, so this fails visibly: it writes the diagnostic to
/// `logcat` and then aborts the process. `android_main` cannot return a failure
/// to the framework, and `android-activity`'s glue wraps `android_main` in
/// `catch_unwind` (a plain panic is swallowed and the activity is gracefully
/// finished — i.e. an invisible close), so an explicit `abort()` is the only
/// way the retirement is observable: a blank window is never shown and the app
/// never continues as if it started.
#[cfg(target_os = "android")]
#[no_mangle]
pub fn android_main(_app: android_activity::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Error),
    );
    if let Err(error) = start() {
        log::error!("{}", error);
    }
    // Unconditional: the retired host must never resume as an empty shell.
    // Not `panic!` — the activity glue would catch it and finish() silently.
    std::process::abort();
}
