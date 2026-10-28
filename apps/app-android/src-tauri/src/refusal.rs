//! The `dom-canvas` runtime refusal on Android: marker line, then exit code `6`.
//!
//! Android has **no shell exit channel** the way a desktop process does: the app
//! is started by the Activity manager, and a `std::process::exit` shows up in
//! `logcat` as process death (best effort, at the mercy of the platform's own
//! reporting), never as a POSIX exit status a script can read. So the **primary
//! judge is the marker line in `logcat`**:
//!
//! ```text
//! yacr: unsupported-runtime: webgl2
//! ```
//!
//! The order is the contract (marker **first**, then diagnostics, then the exit):
//! a blank canvas, a silent software-rendering fallback and an exit `0` are all
//! forbidden (`AGENTS.md` iron rule 8). The same marker/step vocabulary is used by
//! the Linux `dom-canvas` host (`apps/app-linux/src/canvas_runtime.rs`), so
//! evidence tooling reads one line for both carriers; the exit code is registered
//! here as `6` for the desktop hosts' registry, and is what this host attempts
//! after the marker.
//!
//! This module is deliberately target-independent (no `tauri`, no `log`, no
//! Android APIs) so its contract tests run on any host.

/// Stable marker prefix of a `dom-canvas` runtime failure.
pub const UNSUPPORTED_RUNTIME_PREFIX: &str = "yacr: unsupported-runtime: ";

/// The step name the wasm session reports when WebGL2 is not available.
pub const WEBGL2_STEP: &str = "webgl2";

/// Stable marker line of a WebGL2 refusal (`yacr: unsupported-runtime: webgl2`).
pub const WEBGL2_MARKER: &str = "yacr: unsupported-runtime: webgl2";

/// Exit code this host attempts after the marker (`6`, the `dom-canvas` refusal).
///
/// Registry shared with the desktop hosts: `0` success, `1` generic, `2` display
/// policy, `3` post-load driver failure, `4` historical unsupported backend
/// (never produced now), `5` Windows WebView2 missing, `6` this refusal.
pub const WEBGL2_EXIT_CODE: i32 = 6;

/// The command the chrome calls to report a canvas transport failure.
pub const CANVAS_RUNTIME_FAILED_COMMAND: &str = "canvas_runtime_failed";

/// The command the chrome calls to report its own drawing identity.
pub const CANVAS_SESSION_STATUS_COMMAND: &str = "canvas_session_status";

/// The marker line for one reported step, verbatim.
///
/// Every runtime failure of this carrier is a failure to provide the capability
/// end-to-end, so the line names the capability WebGL2; the `step=` token in the
/// diagnostic line is what distinguishes "no context" from "the wasm module is
/// missing" (`ui-module-load`).
pub fn marker_line(step: &str) -> String {
    format!("{UNSUPPORTED_RUNTIME_PREFIX}{step}")
}

/// Whether a reported step is the session's own WebGL2 step.
///
/// Only used for wording/evidence: the refusal itself (marker + exit) is the same
/// for every step.
pub fn is_webgl2_step(step: &str) -> bool {
    step == WEBGL2_STEP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_is_the_registered_dom_canvas_line() {
        // Byte-for-byte stable: evidence collection classifies on this line.
        assert_eq!(WEBGL2_MARKER, "yacr: unsupported-runtime: webgl2");
        assert_eq!(marker_line(WEBGL2_STEP), WEBGL2_MARKER);
        assert!(WEBGL2_MARKER.starts_with(UNSUPPORTED_RUNTIME_PREFIX));
    }

    #[test]
    fn exit_code_is_six_and_distinct_from_every_other_registered_code() {
        assert_eq!(WEBGL2_EXIT_CODE, 6);
        for other in [0, 1, 2, 3, 4, 5] {
            assert_ne!(WEBGL2_EXIT_CODE, other);
        }
    }

    #[test]
    fn step_classification_matches_only_the_webgl2_step() {
        assert!(is_webgl2_step(WEBGL2_STEP));
        assert!(!is_webgl2_step("ui-module-load"));
        assert!(!is_webgl2_step(""));
        assert_eq!(
            marker_line("ui-module-load"),
            "yacr: unsupported-runtime: ui-module-load"
        );
    }

    #[test]
    fn command_names_are_the_chrome_shell_only_reports() {
        // `apps/desktop-ui/src/host/invoke.ts` SHELL_ONLY_COMMANDS.
        assert_eq!(CANVAS_RUNTIME_FAILED_COMMAND, "canvas_runtime_failed");
        assert_eq!(CANVAS_SESSION_STATUS_COMMAND, "canvas_session_status");
    }
}
