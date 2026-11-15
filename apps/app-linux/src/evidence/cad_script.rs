//! `YACR_PROBE_CAD_SCRIPT`: a **generic**, env-gated evidence driver that
//! evaluates one operator-supplied JavaScript source in the real main webview.
//!
//! It exists so a real-GUI run (real Wayland session, real host, real drawing)
//! can execute arbitrary `cad_command`/`cad_status` IPC — through the same
//! `window.__TAURI_INTERNALS__.invoke(...)` funnel [`super::tool`] and
//! [`super::draw`] use — and log the real outcome, without adding one bespoke
//! driver per command that gets promoted.
//!
//! Evidence-only, **default OFF**, independent of every other probe driver
//! (setting only this one does not trigger the `YACR_PROBE_EVIDENCE`/`TOOL`/
//! `DRAW`/`EDIT` passes, and it is spawned from the same host-init entry point
//! those drivers use, so no file outside `src/evidence/` is touched except the
//! module declaration in [`super`]).
//!
//! The input is **synthetic IPC, not a real mouse**: the script is injected
//! with `Webview::eval_with_callback`, exactly like the other evidence drivers,
//! and never touches the kernel/evdev/XInput path. Nothing is synthesized on the
//! Rust side: the reported values are the script's own return string and
//! whatever the script (or its async continuations) stashed under
//! `window.__yacr_cad_script`, read back from the **real** host 2 s later. If a
//! script returns nothing past a dispatch, the log says `undefined` /
//! `promise-dispatched`; it never invents an outcome.
//!
//! ## Env contract (all default OFF)
//!
//! * `YACR_PROBE_CAD_SCRIPT` — inline JavaScript source. Enables the driver when
//!   non-blank.
//! * `YACR_PROBE_CAD_SCRIPT_FILE` — path to a `.js` file. Enables the driver when
//!   non-blank; **wins over** the inline variable when both are set (a file is
//!   the only form that carries multi-line JS without shell quoting).
//! * `YACR_PROBE_CAD_SCRIPT_AT` — whole seconds after driver start, default
//!   [`DEFAULT_AT_SECONDS`] (12), floored at [`MIN_AT_SECONDS`] (1). An
//!   unparseable value is reported (never silently accepted) and falls back to
//!   the default.
//!
//! ## Log lines (one stable, greppable prefix: `[probe] cad script:`)
//!
//! * the enabled line: `enabled delay=<n>s source=inline|file:<path> bytes=<n>`;
//! * any env/parse note (the delay clamp or the default fallback);
//! * exactly one diagnostic when the driver cannot run: an unreadable file, an
//!   empty script, or no child webview — each its own message, and the run
//!   continues unchanged;
//! * at `AT`: `t=<n>s eval returned -> <string>`, or `t=<n>s eval failed -> …`;
//! * 2 s after the eval: `t=<n>s window.__yacr_cad_script -> <json>` where the
//!   JSON is the real `JSON.stringify(window.__yacr_cad_script || null)` read
//!   back from the page.
//!
//! ## Writing a script
//!
//! The source runs inside a function body, so it is JavaScript statements, not
//! an expression:
//!
//! ```js
//! var internals = window.__TAURI_INTERNALS__;
//! var box = window.__yacr_cad_script = window.__yacr_cad_script || {};
//! internals.invoke('cad_command', { command: 'LINE' }).then(function (ack) {
//!   box.line_ack = ack;
//! });
//! internals.invoke('cad_status').then(function (status) {
//!   box.before = status;
//! });
//! return 'line-dispatched';
//! ```
//!
//! The source is a function body (statements), not an expression, and the page
//! has no top-level `await`: the wrapper records the script's return value in
//! `window.__yacr_cad_script.evalResult` (and a thrown error in `evalError`), and
//! follows a returned promise, so the 2 s read-back shows the settled value even
//! when the script itself only dispatched IPC. Stash `cad_status` snapshots or
//! command acks under the same box to see them.

use std::time::{Duration, Instant};

/// Inline JavaScript source (enables the driver when non-blank).
const ENV_SCRIPT: &str = "YACR_PROBE_CAD_SCRIPT";
/// Path to a `.js` file (wins over [`ENV_SCRIPT`] when both are set).
const ENV_SCRIPT_FILE: &str = "YACR_PROBE_CAD_SCRIPT_FILE";
/// Whole seconds after driver start before the eval ([`DEFAULT_AT_SECONDS`]).
const ENV_AT: &str = "YACR_PROBE_CAD_SCRIPT_AT";

/// Default schedule, seconds after driver start.
const DEFAULT_AT_SECONDS: u64 = 12;
/// Floor for `YACR_PROBE_CAD_SCRIPT_AT`: an eval before the host has finished
/// its first paint/upload has nothing real to read back.
const MIN_AT_SECONDS: u64 = 1;
/// How long after the eval the `window.__yacr_cad_script` snapshot is read, so a
/// script's `invoke(...)` promises (and on-disk commits) can settle.
const READBACK_SECONDS: u64 = 2;
/// How long one eval waits for the webview to deliver its callback. A page that
/// never answers cannot hang the evidence run.
const EVAL_TIMEOUT: Duration = Duration::from_millis(5_000);

/// Whether the generic CAD-script driver is enabled
/// (`YACR_PROBE_CAD_SCRIPT` or `YACR_PROBE_CAD_SCRIPT_FILE`, non-blank).
///
/// Evidence-only, default OFF, independent of every other probe driver.
pub fn cad_script_enabled() -> bool {
    non_blank(std::env::var(ENV_SCRIPT).ok().as_deref())
        || non_blank(std::env::var(ENV_SCRIPT_FILE).ok().as_deref())
}

/// The env-value predicate, split out so it is testable without touching the
/// process environment.
fn non_blank(raw: Option<&str>) -> bool {
    matches!(raw, Some(value) if !value.trim().is_empty())
}

/// Where the evaluated source came from, plus its text.
struct Source {
    /// `inline` or `file:<path>` — logged verbatim so a reviewer can tell them
    /// apart in one grep.
    origin: String,
    text: String,
}

/// Schedule the generic CAD-script driver if enabled. Never panics on spawn
/// failure (a failed spawn is logged; the run continues unchanged).
///
/// Every failure mode is explicit and distinct: an unreadable file, an empty
/// script, or a missing child webview each print one `[probe] cad script: …`
/// diagnostic and nothing is evaluated. No pass, count or ack is ever faked.
///
/// The thread only dispatches `Webview::eval_with_callback` scripts; it calls no
/// command/session logic itself and touches no GPU state.
pub fn maybe_spawn_cad_script(webview: Option<&tauri::Webview>) {
    if !cad_script_enabled() {
        return;
    }

    let (delay, note) = parse_delay_seconds(std::env::var(ENV_AT).ok().as_deref());
    if let Some(note) = note {
        eprintln!("[probe] cad script: {note}");
    }

    let source = match load_source() {
        Ok(source) => source,
        Err(diagnostic) => {
            eprintln!("[probe] cad script: {diagnostic}");
            return;
        }
    };

    let Some(webview) = webview else {
        eprintln!(
            "[probe] cad script: no child webview; {} script not evaluated (nothing faked)",
            source.origin
        );
        return;
    };

    eprintln!(
        "[probe] cad script: enabled delay={delay}s source={} bytes={} \
         (synthetic IPC through the real webview, NOT a real mouse; \
         outcomes are read back from the host)",
        source.origin,
        source.text.len()
    );

    let webview = webview.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("probe-cad-script".into())
        .spawn(move || run_cad_script(webview, source, delay))
    {
        eprintln!("[probe] cad script: failed to spawn: {error}");
    }
}

/// Resolve the script source. `YACR_PROBE_CAD_SCRIPT_FILE` wins when both
/// variables are set.
///
/// Every error string is a complete diagnostic: distinct per failure mode, and
/// explicit that nothing was evaluated.
fn load_source() -> Result<Source, String> {
    let file = std::env::var(ENV_SCRIPT_FILE).ok();
    let file = file
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty());
    if let Some(path) = file {
        let text = std::fs::read_to_string(path).map_err(|error| {
            format!("source file {path:?} unreadable ({error}); nothing evaluated (nothing faked)")
        })?;
        if text.trim().is_empty() {
            return Err(format!(
                "source file {path:?} is empty; nothing evaluated (nothing faked)"
            ));
        }
        return Ok(Source {
            origin: format!("file:{path}"),
            text,
        });
    }

    let inline = std::env::var(ENV_SCRIPT).ok().unwrap_or_default();
    if inline.trim().is_empty() {
        return Err(
            "YACR_PROBE_CAD_SCRIPT is empty; nothing evaluated (nothing faked)".to_string(),
        );
    }
    Ok(Source {
        origin: "inline".to_string(),
        text: inline,
    })
}

/// Parse `YACR_PROBE_CAD_SCRIPT_AT`, clamped to at least [`MIN_AT_SECONDS`].
///
/// An unset/blank value uses [`DEFAULT_AT_SECONDS`]; a non-integer or
/// out-of-range value is reported (never silently accepted) and falls back to
/// the default (for `0`) or the floor.
fn parse_delay_seconds(raw: Option<&str>) -> (u64, Option<String>) {
    match raw.map(str::trim) {
        None | Some("") => (DEFAULT_AT_SECONDS, None),
        Some(text) => match text.parse::<u64>() {
            Ok(seconds) if seconds < MIN_AT_SECONDS => (
                MIN_AT_SECONDS,
                Some(format!(
                    "YACR_PROBE_CAD_SCRIPT_AT={seconds}s is below the {MIN_AT_SECONDS}s floor; \
                     using {MIN_AT_SECONDS}s"
                )),
            ),
            Ok(seconds) => (seconds, None),
            Err(_) => (
                DEFAULT_AT_SECONDS,
                Some(format!(
                    "YACR_PROBE_CAD_SCRIPT_AT={text:?} is not an integer number of seconds; \
                     using {DEFAULT_AT_SECONDS}"
                )),
            ),
        },
    }
}

/// The whole timeline relative to driver start: one eval at `delay` seconds,
/// then the `window.__yacr_cad_script` read-back [`READBACK_SECONDS`] later.
fn run_cad_script(webview: tauri::Webview, source: Source, delay: u64) {
    let start = Instant::now();
    sleep_until(&start, Duration::from_secs(delay));

    let dispatched_at = Instant::now();
    match eval_string(&webview, &wrap_script(&source.text)) {
        Ok(returned) => eprintln!("[probe] cad script: t={delay}s eval returned -> {returned}"),
        Err(error) => eprintln!("[probe] cad script: t={delay}s eval failed -> {error}"),
    }

    sleep_until(&dispatched_at, Duration::from_secs(READBACK_SECONDS));
    let readback = delay + READBACK_SECONDS;
    match eval_string(&webview, STATE_JS) {
        Ok(state) => {
            eprintln!("[probe] cad script: t={readback}s window.__yacr_cad_script -> {state}")
        }
        Err(error) => eprintln!(
            "[probe] cad script: t={readback}s window.__yacr_cad_script read failed -> {error}"
        ),
    }
}

/// Sleep until `target` has elapsed since `start` (never sleeps backwards).
fn sleep_until(start: &Instant, target: Duration) {
    if let Some(remaining) = target.checked_sub(start.elapsed()) {
        std::thread::sleep(remaining);
    }
}

/// Evaluate `js` in the real webview and return its decoded string result.
///
/// `eval_with_callback` serializes the script's return value into a JSON string;
/// the wait is bounded by [`EVAL_TIMEOUT`] so a page that never answers cannot
/// hang the evidence run.
fn eval_string(webview: &tauri::Webview, js: &str) -> Result<String, String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    webview
        .eval_with_callback(js, move |payload| {
            let _ = sender.send(payload);
        })
        .map_err(|error| format!("dispatch failed: {error}"))?;
    let payload = receiver
        .recv_timeout(EVAL_TIMEOUT)
        .map_err(|error| format!("the webview did not answer within {EVAL_TIMEOUT:?}: {error}"))?;
    Ok(super::ui_dialogs_report::decode_payload(&payload))
}

/// Wrap the operator script in the reporting shell.
///
/// The shell (not the operator) owns the bookkeeping: it records the script's
/// return value in `window.__yacr_cad_script.evalResult` and a thrown error in
/// `evalError`, follows a returned promise, and returns the same string so the
/// Rust callback can log it. `__SOURCE__` is replaced verbatim (no escaping:
/// the source is JavaScript and is not interpolated anywhere else).
fn wrap_script(source: &str) -> String {
    SCRIPT_TEMPLATE.replace("__SOURCE__", source)
}

const SCRIPT_TEMPLATE: &str = r#"
(function () {
  var box = window.__yacr_cad_script = window.__yacr_cad_script || {};
  function stringify(value) {
    if (typeof value === 'undefined') { return 'undefined'; }
    if (typeof value === 'string') { return value; }
    try { return JSON.stringify(value); } catch (err) { return String(value); }
  }
  try {
    var value = (function () {
__SOURCE__
    })();
    if (value && typeof value.then === 'function') {
      box.evalResult = 'promise-dispatched';
      value.then(function (settled) {
        box.evalResult = stringify(settled);
      }, function (error) {
        box.evalError = String(error);
      });
      return box.evalResult;
    }
    box.evalResult = stringify(value);
    return box.evalResult;
  } catch (error) {
    box.evalError = String(error);
    box.evalResult = 'error:' + error;
    return box.evalResult;
  }
})()
"#;

/// Read back what the script (or its async continuations) stashed under
/// `window.__yacr_cad_script`. `null` when it stashed nothing — never a
/// synthesized placeholder.
const STATE_JS: &str = r#"
(function () {
  try {
    return JSON.stringify(window.__yacr_cad_script || null);
  } catch (error) {
    return 'error:' + error;
  }
})()
"#;

#[cfg(test)]
mod tests {
    use super::{parse_delay_seconds, wrap_script, DEFAULT_AT_SECONDS, MIN_AT_SECONDS};

    #[test]
    fn cad_script_delay_defaults_and_clamps_to_at_least_one_second() {
        // Unset / blank -> documented default, no diagnostic.
        assert_eq!(parse_delay_seconds(None), (DEFAULT_AT_SECONDS, None));
        assert_eq!(parse_delay_seconds(Some("")), (DEFAULT_AT_SECONDS, None));
        assert_eq!(parse_delay_seconds(Some("   ")), (DEFAULT_AT_SECONDS, None));

        // A usable integer is taken verbatim.
        assert_eq!(parse_delay_seconds(Some("30")), (30, None));
        assert_eq!(parse_delay_seconds(Some(" 30 ")), (30, None));
        assert_eq!(parse_delay_seconds(Some("1")), (1, None));

        // Zero is clamped to the floor and reported.
        let (seconds, note) = parse_delay_seconds(Some("0"));
        assert_eq!(seconds, MIN_AT_SECONDS);
        assert!(note.is_some());
        assert_eq!(MIN_AT_SECONDS, 1);

        // Non-integer / negative -> default, reported (never silently accepted).
        for raw in ["12s", "-3", "1.5", "abc"] {
            let (seconds, note) = parse_delay_seconds(Some(raw));
            assert_eq!(seconds, DEFAULT_AT_SECONDS, "raw={raw}");
            assert!(note.is_some(), "raw={raw}");
        }
    }

    #[test]
    fn cad_script_wrapper_embeds_source_and_owns_the_box() {
        let wrapped = wrap_script("return 'x';");
        assert!(wrapped.contains("return 'x';"));
        assert!(!wrapped.contains("__SOURCE__"));
        // The bookkeeping box the read-back reads.
        assert!(wrapped.contains("window.__yacr_cad_script"));
        assert!(wrapped.contains("evalResult"));
    }
}
