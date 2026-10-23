//! The `dom-canvas` runtime refusal (ADR 0024 D5): marker + i18n + exit code `6`.
//!
//! The `dom-canvas` carrier draws the CAD image with **WebGL2** from a wasm CAD
//! session inside the WebView. Whether that works can only be known **after** the
//! WebView exists, so this is a **runtime** refusal, in a different fault domain
//! from the pre-GTK backend policy (`crate::backend`, exit `2`) and from the
//! historical unsupported backend (exit `4`, see
//! [`crate::backend::UNSUPPORTED_BACKEND_MARKER`]). The codes are never merged.
//!
//! # Contract
//!
//! When the frontend transport (`apps/desktop-ui/src/canvas-host.mjs`) cannot
//! draw — the module fails to load, `start()` rejects, or the session's own status
//! reports `ok: false` — it reports the machine-readable step to the host through
//! the `canvas_runtime_failed` command. The host then refuses **the whole
//! process**, in this order:
//!
//! 1. the stable marker line [`WEBGL2_MARKER`] **first** (evidence collection
//!    classifies on this line; the exit code is only supporting information),
//! 2. the localized user-facing diagnostic (`startup.webgl2_unavailable` and
//!    `.detail`, from the single i18n catalog),
//! 3. one technical line naming the `step` and the session's `detail`,
//! 4. `std::process::exit(6)`.
//!
//! The marker names the **capability the carrier requires** (WebGL2). Every
//! `dom-canvas` runtime failure is a failure to provide that capability
//! end-to-end, so the registered marker is the same for all of them; the `step=`
//! token on line 3 is what distinguishes "no WebGL2 context" from "the wasm
//! module is missing" (e.g. `ui-module-load`). Never a blank canvas, never exit
//! `0`, never a silent fall back to the retired X11 native-child window.
//!
//! # Why exiting from a command thread
//!
//! The refusal must end the process, not leave a window showing an empty
//! partition: there is no usable CAD image and no second carrier to fall back to
//! (ADR 0024 §3 rejects reviving the X11 native child). Exiting here means the
//! normal teardown does **not** run, which is the honest cost of a startup
//! refusal and is why nothing is claimed about cleanup.

use std::io::Write;

/// Stable stderr marker of a `dom-canvas` runtime failure.
///
/// Byte-for-byte stable: evidence collection classifies on this exact line.
pub const WEBGL2_MARKER: &str = "yacr: unsupported-runtime: webgl2";

/// Process exit code of the `dom-canvas` runtime refusal (`6`).
///
/// Registry: `0` success, `1` generic failure, `2` display-backend policy error,
/// `3` postload-pan driver failure, `4` historical unsupported backend (not
/// produced), `5` Windows WebView2 missing, `6` this runtime refusal.
pub const WEBGL2_EXIT_CODE: i32 = 6;

/// The wasm session's step name for "no usable WebGL2 context".
///
/// The session owns its vocabulary (`webgl2`, `surface`, `import`, …); the host
/// only fixes the one name it classifies on.
pub const WEBGL2_STEP: &str = "webgl2";

/// The command the frontend calls to report a canvas transport failure.
pub const CANVAS_RUNTIME_FAILED_COMMAND: &str = "canvas_runtime_failed";

/// i18n key of the user-facing headline (present in every catalog).
pub const MESSAGE_KEY: &str = "startup.webgl2_unavailable";
/// i18n key of the user-facing detail paragraph.
pub const DETAIL_KEY: &str = "startup.webgl2_unavailable.detail";

/// `true` when a reported step is the session's own WebGL2 step.
///
/// Only used for diagnostics/evidence wording — the refusal itself is the same
/// for every step (see the module docs).
pub fn is_webgl2_step(step: &str) -> bool {
    step == WEBGL2_STEP
}

/// The refusal block, as lines, in emission order.
///
/// Pure so the **order** (marker first) is a contract test rather than a reading
/// of `main`. The catalog is read through the host's single i18n source.
pub fn refusal_lines(step: &str, detail: &str) -> Vec<String> {
    let messages =
        cad_ui_core::i18n::MessageSource::from_request(cad_ui_core::i18n::host_locale_tag());
    let detail = if detail.trim().is_empty() {
        "(no detail reported)".to_string()
    } else {
        detail.to_string()
    };
    vec![
        WEBGL2_MARKER.to_string(),
        messages.text(MESSAGE_KEY, &[]),
        messages.text(DETAIL_KEY, &[]),
        format!("[probe] canvas runtime failed step={step} detail={detail}"),
    ]
}

/// Write the refusal block to `out` (one line per entry, in order).
pub fn write_refusal(out: &mut impl Write, step: &str, detail: &str) -> std::io::Result<()> {
    for line in refusal_lines(step, detail) {
        writeln!(out, "{line}")?;
    }
    out.flush()
}

/// Emit the refusal and end the process with [`WEBGL2_EXIT_CODE`].
///
/// Never returns. Called from the `canvas_runtime_failed` command
/// (`crate::host::dispatch`), i.e. after the WebView exists and the frontend has
/// proven the carrier cannot draw.
pub fn refuse(step: &str, detail: &str) -> ! {
    let mut stderr = std::io::stderr().lock();
    if let Err(error) = write_refusal(&mut stderr, step, detail) {
        // A failed write must not turn into a success path: fall back to a raw
        // line on the same stream and still exit non-zero.
        eprintln!("{WEBGL2_MARKER}");
        eprintln!("[probe] canvas runtime refusal write failed: {error}");
    }
    std::process::exit(WEBGL2_EXIT_CODE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(step: &str, detail: &str) -> Vec<String> {
        let mut buffer = Vec::new();
        write_refusal(&mut buffer, step, detail).expect("write to a Vec");
        let text = String::from_utf8(buffer).expect("utf-8");
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn marker_first_then_diagnostics_and_the_exit_code_is_six() {
        let lines = rendered(WEBGL2_STEP, "no webgl2 context");
        assert_eq!(
            lines.first().map(String::as_str),
            Some("yacr: unsupported-runtime: webgl2"),
            "the stable marker must be the first line, got {lines:?}"
        );
        // The registered code is 6, distinct from 2 (policy) and 4 (historical).
        assert_eq!(WEBGL2_EXIT_CODE, 6);
        assert_eq!(WEBGL2_MARKER, "yacr: unsupported-runtime: webgl2");
        // The marker is not the historical backend marker.
        assert_ne!(WEBGL2_MARKER, crate::backend::UNSUPPORTED_BACKEND_MARKER);
        // The technical line carries the machine-readable step and the session's
        // own detail, and comes after the marker.
        assert_eq!(
            lines.len(),
            4,
            "marker + 2 catalog lines + 1 detail: {lines:?}"
        );
        assert!(lines[3].contains("step=webgl2"), "{lines:?}");
        assert!(lines[3].contains("no webgl2 context"), "{lines:?}");
    }

    #[test]
    fn every_reported_step_refuses_with_the_same_registered_marker() {
        // A missing artifact (`ui-module-load`), a rejected call and an unreadable
        // status all mean the carrier cannot draw: same marker + code, different
        // `step=` token. Never a blank canvas, never exit 0.
        for step in [
            "webgl2",
            "ui-module-load",
            "ui-export-threw",
            "ui-status-unreadable",
            "surface",
        ] {
            let lines = rendered(step, "");
            assert_eq!(lines[0], WEBGL2_MARKER);
            assert!(lines[3].contains(&format!("step={step}")), "{lines:?}");
            // No detail at all is reported explicitly instead of an empty tail.
            assert!(lines[3].contains("(no detail reported)"), "{lines:?}");
        }
    }

    #[test]
    fn the_i18n_keys_exist_in_every_catalog_and_are_not_the_key_itself() {
        use cad_ui_core::i18n::{Locale, MessageSource};
        for locale in Locale::ALL {
            let source = MessageSource::for_locale(locale);
            for key in [MESSAGE_KEY, DETAIL_KEY] {
                let message = source.message(key, &[]);
                assert!(
                    message.is_found(),
                    "{key} missing from the {} catalog",
                    locale.tag()
                );
                let text = message.text();
                assert!(
                    !text.trim().is_empty(),
                    "{key} is empty for {}",
                    locale.tag()
                );
                assert!(
                    !text.contains('\u{27e6}'),
                    "{key} rendered as a missing key for {}: {text}",
                    locale.tag()
                );
            }
        }
    }

    #[test]
    fn only_the_session_webgl2_step_is_classified_as_webgl2() {
        assert!(is_webgl2_step("webgl2"));
        assert!(!is_webgl2_step("ui-module-load"));
        assert!(!is_webgl2_step(""));
        assert_eq!(WEBGL2_STEP, "webgl2");
    }

    #[test]
    fn the_command_name_matches_the_frontend_call() {
        // `apps/desktop-ui/src/App.jsx` invokes exactly this name.
        assert_eq!(CANVAS_RUNTIME_FAILED_COMMAND, "canvas_runtime_failed");
    }
}
