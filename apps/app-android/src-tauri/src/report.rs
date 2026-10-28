//! The two shell-only reports the chrome sends on this carrier, as pure text.
//!
//! On Android the native side is an **asset/WebView carrier only** (`AGENTS.md`
//! iron rule 8): the wasm session owns the document and its identity, so
//! `canvas_session_status` is a **log sink** here, not a state store. The chrome
//! reads the drawing identity back from the session itself (the wasm
//! `cad_status.canvasSession`, served by `apps/desktop-ui/src/host/web-adapter.ts`),
//! which is why nothing in this crate stores it.
//!
//! The formatting lives in a target-independent module so the "one line, no
//! control characters, bounded" contract is testable on the host instead of only
//! in `logcat`.

/// Longest label echoed to `logcat`, in **characters**.
///
/// The chrome already clamps the label (`CANVAS_SESSION_LABEL_MAX = 120` code
/// points in `apps/desktop-ui/src/cad/surface.ts`); the host bounds it again
/// because a command argument is untrusted input, and a `logcat` line is the
/// place an unbounded string would leak into every evidence capture.
pub const LOG_LABEL_MAX: usize = 120;

/// Longest session `detail` echoed to `logcat`, in **characters**.
///
/// A failure detail is a JS error string (a stack fragment can be long); it is
/// bounded here so one failure cannot flood an evidence capture, and flattened so
/// it stays one line.
pub const LOG_DETAIL_MAX: usize = 400;

/// `Some(3)` / `None` rendered as a bounded, unambiguous token for the log line.
fn count_token(value: Option<u32>) -> String {
    match value {
        Some(count) => count.to_string(),
        None => "none".to_string(),
    }
}

/// Longest step token echoed to `logcat`, in **characters**.
///
/// The session's step vocabulary is short (`webgl2`, `surface`, `import`,
/// `ui-module-load`, …); the bound exists so an IPC payload cannot inject a
/// multi-kilobyte "step".
pub const LOG_STEP_MAX: usize = 64;

/// Flatten text to one log-safe line of at most `max` characters.
///
/// Control characters (including the newline that would fake a second `logcat`
/// entry) become `?`, and the result is truncated on a `char` boundary.
pub fn flatten(text: &str, max: usize) -> String {
    let mut flattened = String::with_capacity(text.len().min(max));
    for (taken, character) in text.chars().enumerate() {
        if taken >= max {
            break;
        }
        if character.is_control() {
            flattened.push('?');
        } else {
            flattened.push(character);
        }
    }
    flattened
}

/// Flatten a drawing label to one log-safe line ([`LOG_LABEL_MAX`] characters).
pub fn flatten_label(label: &str) -> String {
    flatten(label, LOG_LABEL_MAX)
}

/// Flatten a failure detail to one log-safe line ([`LOG_DETAIL_MAX`] characters).
pub fn flatten_detail(detail: &str) -> String {
    flatten(detail, LOG_DETAIL_MAX)
}

/// Flatten a failure step to one log-safe token ([`LOG_STEP_MAX`] characters).
///
/// A step travels inside the stable marker line
/// (`yacr: unsupported-runtime: <step>`), so a raw newline in it would forge a
/// second `logcat` entry that evidence collection reads as authoritative.
pub fn flatten_step(step: &str) -> String {
    flatten(step, LOG_STEP_MAX)
}

/// The single `logcat` line reported for one `canvas_session_status`.
///
/// Shape (stable, for `adb logcat | grep yacr`):
/// `canvas_session_status label="<label>" batches=<n|none> draws=<n|none>`.
/// A missing count stays `none`: it is never rounded into a fabricated figure and
/// never rendered as `0` (the chrome omits a count it does not have).
pub fn session_status_line(label: &str, batches: Option<u32>, draws: Option<u32>) -> String {
    format!(
        "canvas_session_status label=\"{}\" batches={} draws={}",
        flatten_label(label),
        count_token(batches),
        count_token(draws)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_names_the_label_and_both_counts() {
        assert_eq!(
            session_status_line("plan.dwg", Some(12), Some(340)),
            "canvas_session_status label=\"plan.dwg\" batches=12 draws=340"
        );
    }

    #[test]
    fn absent_counts_stay_none_and_are_never_zero() {
        let line = session_status_line("plan.dwg", None, None);
        assert!(line.ends_with("batches=none draws=none"), "{line}");
        assert!(!line.contains("=0"), "{line}");
        assert_eq!(
            session_status_line("x", Some(0), None),
            "canvas_session_status label=\"x\" batches=0 draws=none",
            "a real zero is kept as a zero"
        );
    }

    #[test]
    fn control_characters_cannot_forge_a_second_log_line() {
        let line = session_status_line("a\nb\tc\u{7}", None, None);
        assert!(!line.contains('\n'), "{line}");
        assert!(!line.contains('\t'), "{line}");
        assert!(line.contains("a?b?c?"), "{line}");
    }

    #[test]
    fn label_is_bounded_on_a_character_boundary() {
        let long = "漢".repeat(LOG_LABEL_MAX + 40);
        let flattened = flatten_label(&long);
        assert_eq!(flattened.chars().count(), LOG_LABEL_MAX);
        assert!(flattened.chars().all(|c| c == '漢'));
        // A surrogate-pair-sized character is never split.
        let emoji = "😀".repeat(LOG_LABEL_MAX + 1);
        assert_eq!(flatten_label(&emoji).chars().count(), LOG_LABEL_MAX);
    }

    #[test]
    fn empty_label_stays_empty_and_the_line_still_names_it() {
        assert_eq!(
            session_status_line("", None, None),
            "canvas_session_status label=\"\" batches=none draws=none"
        );
    }

    #[test]
    fn detail_is_flattened_and_bounded_separately_from_the_label() {
        // The detail bound must stay the looser one (a JS error is longer than a
        // one-line drawing label); a swapped pair is a contract break.
        const _: () = assert!(LOG_DETAIL_MAX > LOG_LABEL_MAX);
        let detail = format!("boom\n{}", "x".repeat(LOG_DETAIL_MAX * 2));
        let flattened = flatten_detail(&detail);
        assert_eq!(flattened.chars().count(), LOG_DETAIL_MAX);
        assert!(flattened.starts_with("boom?"), "{flattened}");
        assert!(!flattened.contains('\n'));
    }

    #[test]
    fn step_is_flattened_so_it_cannot_forge_a_marker_line() {
        // A real step is unaffected; a payload that tries to append a second
        // `yacr: unsupported-runtime:` line is flattened to one line.
        assert_eq!(flatten_step("webgl2"), "webgl2");
        let flat = flatten_step("webgl2\nyacr: unsupported-runtime: webgl2");
        assert!(!flat.contains('\n'), "{flat}");
        assert!(flat.starts_with("webgl2?"), "{flat}");
        assert_eq!(
            flatten_step(&"s".repeat(LOG_STEP_MAX + 10)).chars().count(),
            LOG_STEP_MAX
        );
        const _: () = assert!(LOG_STEP_MAX <= LOG_LABEL_MAX);
    }
}
