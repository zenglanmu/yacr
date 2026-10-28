//! The Android host's Tauri entry point and its **two** commands.
//!
//! This is an **asset/WebView carrier**. The shared React chrome
//! (`apps/desktop-ui`, built with `YACR_RUNTIME=android`) runs here and routes
//! everything CAD to the wasm `dom-canvas` session, which is the single
//! `cad-db` authority (`AGENTS.md` iron rule 8, ADR 0024). The native side
//! therefore:
//!
//! - registers **only** the two shell-only reports the chrome sends over IPC
//!   ([`canvas_session_status`], [`canvas_runtime_failed`]) — the same two names
//!   `apps/desktop-ui/src/host/invoke.ts` routes to Tauri when the bridge exists;
//! - holds **no** document state, opens no drawing, creates no second session and
//!   registers no CAD command: `cad_status`, `cad_command`, `cad_commands`,
//!   `cad_command_aliases` and everything else are answered by the wasm session
//!   through `apps/desktop-ui/src/host/web-adapter.ts`;
//! - keeps only WebView/Activity lifetime.
//!
//! Anything the chrome asks that this carrier cannot serve is the wasm adapter's
//! explicit `unsupported-runtime: <command>` rejection; there is no silent
//! fallback and no empty success here.

use crate::refusal;
use crate::report;

/// `logcat` tag of every line this host prints (`adb logcat -s yacr`).
pub const LOG_TAG: &str = "yacr";

/// Install the logger that makes [`log`] records reach `logcat`.
///
/// Idempotent (`init_once`): a second call is a no-op, so a re-created Activity
/// cannot double-install it. Level `Info` is what the two reports use; the refusal
/// uses `Error`.
pub fn init_logging() {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag(LOG_TAG),
    );
}

/// Report the session's drawing identity, one `logcat` line, no state.
///
/// `apps/desktop-ui/src/App.tsx` sends this whenever the wasm session's status
/// names a drawing (deduplicated by the chrome). On Android the identity is the
/// session's own, so the host **only logs** it — storing it would be a second
/// document model (iron rule 8). `Err` is reserved for an IPC-shape failure the
/// chrome should surface; the log write itself cannot fail in a way worth
/// reporting.
#[tauri::command]
pub fn canvas_session_status(
    label: String,
    batches: Option<u32>,
    draws: Option<u32>,
) -> Result<(), String> {
    log::info!("{}", report::session_status_line(&label, batches, draws));
    Ok(())
}

/// Report a `dom-canvas` transport failure: marker line **first**, then the exit.
///
/// Contract (`AGENTS.md` iron rule 8, ADR 0024 D5):
///
/// 1. the stable marker line `yacr: unsupported-runtime: <step>` is emitted
///    **first** on both channels (`logcat` via the logger, `stderr` via
///    `eprintln!`), because on Android the marker in `logcat` is the primary
///    judge — there is no shell to read an exit status from;
/// 2. one technical line names `step` and the session's own `detail`;
/// 3. when `step == "webgl2"` the process is ended with
///    [`refusal::WEBGL2_EXIT_CODE`] (`6`).
///
/// Every other step (module load, session start, …) is **reported and left
/// running**: the WebView is still alive and the chrome can show its own error
/// state, so killing the process would turn a recoverable failure into an
/// app crash. This is the one deliberate difference from the Linux
/// `dom-canvas` host (`apps/app-linux/src/canvas_runtime.rs` exits on every step),
/// and it is a consequence of Android having no shell exit channel.
///
/// The localized user-facing copy is **not** printed here: on this carrier the
/// React chrome owns every user-visible word (i18n catalogs in
/// `crates/cad-ui-core/i18n`), and the native side has no window to put it in.
#[tauri::command]
pub fn canvas_runtime_failed(step: String, detail: String) -> Result<(), String> {
    // The step is untrusted IPC input and it lands **inside** the marker line, so
    // a raw newline in it could forge a second `logcat` entry. Flatten before use;
    // a real step (`webgl2`, …) is unchanged by this.
    let step = report::flatten_step(&step);
    let marker = refusal::marker_line(&step);
    // Marker first, on both channels: `logcat` for evidence capture, stderr for a
    // locally attached debug session. Never reordered, never deduplicated.
    log::error!("{marker}");
    eprintln!("{marker}");

    let detail = if detail.trim().is_empty() {
        "(no detail reported)".to_string()
    } else {
        report::flatten_detail(&detail)
    };
    let diagnostic = format!("[probe] canvas runtime failed step={step} detail={detail}");
    log::error!("{diagnostic}");
    eprintln!("{diagnostic}");

    if refusal::is_webgl2_step(&step) {
        // The platform has no exit-status channel: this ends the process and
        // `logcat` records the marker (and, in practice, the process death).
        std::process::exit(refusal::WEBGL2_EXIT_CODE);
    }
    Ok(())
}

/// Start the Tauri application.
///
/// `#[tauri::mobile_entry_point]` generates the `start_app` symbol the generated
/// Kotlin glue calls (`gen/android`), and it needs the
/// `TAURI_ANDROID_PACKAGE_NAME_*` env vars `build.rs`/`tauri-build` exports.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_logging();
    log::info!(
        "app-android: Tauri carrier starting (wasm dom-canvas CAD session owns the document)"
    );

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            canvas_session_status,
            canvas_runtime_failed
        ])
        .run(tauri::generate_context!())
        .expect("app-android: the Tauri application failed to run");
}
