//! Import-progress panel heartbeat for the Web host (F01, §4.10).
//!
//! The owner-thread host pipeline is the single source of truth: an open is
//! delivered with `HostCore::deliver_external_open` (see [`super::documents`]),
//! and this heartbeat only drives the fixed P3 order
//! `IO → metrics → binding.pump_requests (completion drain) → flush` through
//! [`HostCore::tick`]. The panel snapshot is read straight from the host
//! (`HostCore::import_snapshot`), never from a platform-authored fallback.
//!
//! `wasm32-unknown-unknown` has no threads, so [`worker_available`] is `false`
//! and the browser cannot run the background importer. The module therefore does
//! not fabricate a running/opened/worker-started state: with no worker the panel
//! is idle (or shows the host's real terminal when one exists). Progress is only
//! shown for a state the host actually reports.

use cad_app::tasks::{ImportProgressSnapshot, ImportTerminal};
use cad_host::HostCore;

use super::fonts::spawn_font_load_if_identity_changed;
use super::with_runtime;

/// Whether the core background import worker can run on this host.
///
/// `cad_app::tasks` uses `std::thread`; `wasm32-unknown-unknown` has no threads
/// and `std::thread::Builder::spawn` panics. The browser therefore cannot run
/// the worker and must not call `begin_async_open`. This is the honest gate the
/// whole module is built around; it is not a placeholder.
pub fn worker_available() -> bool {
    !cfg!(target_arch = "wasm32")
}

/// The snapshot the progress panel should display, read from the host.
///
/// Reading this never drains progress, publishes a document or dispatches a
/// command — it is a pure getter over the host's real import state.
pub(super) fn current_snapshot(core: &HostCore) -> Option<ImportProgressSnapshot> {
    core.import_snapshot()
}

/// The heartbeat body: one host tick (real IO/import/render pipeline), the
/// bounded queue pump, the state flush, then the real panel JSON.
///
/// The document swap itself happens in the host pipeline (via
/// `deliver_external_open` / the async publish inside `tick`), never here; this
/// only reports and starts the web font load once the published identity
/// changed.
pub fn poll_and_apply() -> String {
    let Some(rt) = with_runtime(|rt| rt.clone()) else {
        return snapshot_json(None);
    };

    // One host tick: drain platform file events, advance the import, refresh
    // canvas metrics and sync the render view (including the synchronous WASM
    // scene preparation). A teardown cancellation is not a user-facing failure.
    if let Err(error) = rt.core.tick() {
        if !matches!(error, cad_domain::CadError::Cancelled) {
            cad_ui_slint::web::console_error(&format!("host tick failed: {error}"));
        }
    }
    // Fixed P3 order: IO (tick) → metrics (in tick) → bounded queue pump with
    // completion drain → flush. The waker schedules additional pumps; the
    // heartbeat stays the backstop and is never the only queue driver.
    if let Err(error) = rt.binding.pump_requests() {
        cad_ui_slint::web::console_error(&format!("queue pump failed: {error}"));
    }
    if let Err(error) = rt.binding.flush_updates() {
        cad_ui_slint::web::console_error(&format!("host update flush failed: {error}"));
    }

    // Start the web font load only after the host published a new document
    // identity (sync install or worker publish); never a fabricated open.
    spawn_font_load_if_identity_changed(&rt);
    rt.push_web_overlay();
    let snapshot = current_snapshot(&rt.core);
    snapshot_json(snapshot.as_ref())
}

/// Stable JSON of the current panel snapshot, for the JS heartbeat and tests.
///
/// The shape is intentionally small and stable:
/// `{worker, running, visible, terminal, phase, done, total, bytes,
/// cancellable, opened_entities, error}`. It carries only real values: unknown
/// totals/bytes are `null`, and the terminal word is one of
/// `none|opened|cancelled|failed`. Hand-encoded (no serializer dependency); all
/// strings pass through [`json_escape`] so a host error can never break the
/// object.
pub(super) fn snapshot_json(snapshot: Option<&ImportProgressSnapshot>) -> String {
    let (running, visible, terminal, phase, done, total, bytes, cancellable, opened, error) =
        match snapshot {
            None => (
                false,
                false,
                "none",
                "null".to_string(),
                0,
                "null".to_string(),
                "null".to_string(),
                false,
                "null".to_string(),
                "null".to_string(),
            ),
            Some(snapshot) => {
                let (terminal, opened, error) = match &snapshot.terminal {
                    None => ("none", "null".to_string(), "null".to_string()),
                    Some(ImportTerminal::Opened { entities }) => {
                        ("opened", entities.to_string(), "null".to_string())
                    }
                    Some(ImportTerminal::Cancelled) => {
                        ("cancelled", "null".to_string(), "null".to_string())
                    }
                    Some(ImportTerminal::Failed { error }) => (
                        "failed",
                        "null".to_string(),
                        format!("\"{}\"", json_escape(&error.to_string())),
                    ),
                };
                // Mirror the panel's own visibility rule: hidden while
                // idle/opened, visible for a running job or an explicit
                // cancelled/failed terminal.
                let visible = snapshot.running || matches!(terminal, "cancelled" | "failed");
                let phase = match snapshot.phase_key() {
                    Some(key) => format!("\"{key}\""),
                    None => "null".to_string(),
                };
                let total = match snapshot.entities_total {
                    Some(total) => total.to_string(),
                    None => "null".to_string(),
                };
                let bytes = match snapshot.bytes {
                    Some(bytes) => bytes.to_string(),
                    None => "null".to_string(),
                };
                (
                    snapshot.running,
                    visible,
                    terminal,
                    phase,
                    snapshot.entities_done,
                    total,
                    bytes,
                    snapshot.cancellable,
                    opened,
                    error,
                )
            }
        };
    format!(
        "{{\"worker\":{},\"running\":{running},\"visible\":{visible},\"terminal\":\"{terminal}\",\
\"phase\":{phase},\"done\":{done},\"total\":{total},\"bytes\":{bytes},\
\"cancellable\":{cancellable},\"opened_entities\":{opened},\"error\":{error}}}",
        worker_available()
    )
}

/// Escape a string for embedding in a JSON string literal.
///
/// Only the characters JSON requires are escaped; the importer's `Display`
/// messages can contain quotes/backslashes/newlines, and an unescaped one would
/// corrupt the whole object the JS host parses.
fn json_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_app::tasks::ImportProgressSnapshot;
    use cad_domain::CadError;

    /// The pure JSON encoder is the host-independent part of this module; the
    /// `std::thread` worker is unavailable on wasm, so the JSON shape and the
    /// capability gate are what can be asserted without a browser.
    #[test]
    fn idle_snapshot_json_is_explicit_and_carries_the_capability_gate() {
        let json = snapshot_json(None);
        assert!(json.contains("\"running\":false"));
        assert!(json.contains("\"visible\":false"));
        assert!(json.contains("\"terminal\":\"none\""));
        // The browser honestly reports no worker; a threaded host would take
        // the same function and report `true`.
        assert!(json.contains(&format!("\"worker\":{}", worker_available())));
    }

    #[test]
    fn terminal_snapshots_are_named_explicitly() {
        let opened = ImportProgressSnapshot::terminal(ImportTerminal::Opened { entities: 4 });
        let json = snapshot_json(Some(&opened));
        assert!(json.contains("\"terminal\":\"opened\""));
        assert!(json.contains("\"opened_entities\":4"));
        assert!(json.contains("\"running\":false"));
        // A successful open hides the panel.
        assert!(json.contains("\"visible\":false"));

        let cancelled = ImportProgressSnapshot::terminal(ImportTerminal::Cancelled);
        let json = snapshot_json(Some(&cancelled));
        assert!(json.contains("\"terminal\":\"cancelled\""));
        assert!(json.contains("\"visible\":true"));
        assert!(json.contains("\"cancellable\":false"));

        let failed = ImportProgressSnapshot::terminal(ImportTerminal::Failed {
            error: CadError::CorruptData("bad".into()),
        });
        let json = snapshot_json(Some(&failed));
        assert!(json.contains("\"terminal\":\"failed\""));
        assert!(json.contains("\"visible\":true"));
        assert!(json.contains("bad"));
    }

    #[test]
    fn running_snapshot_json_never_fabricates_a_total_or_bytes() {
        let running = ImportProgressSnapshot::running();
        let json = snapshot_json(Some(&running));
        assert!(json.contains("\"running\":true"));
        assert!(json.contains("\"visible\":true"));
        assert!(json.contains("\"phase\":null"));
        assert!(json.contains("\"total\":null"));
        assert!(json.contains("\"bytes\":null"));
        // The running snapshot is genuinely cancellable only where the worker
        // can actually run; the encoder preserves the real flag.
        assert!(json.contains(&format!("\"cancellable\":{}", running.cancellable)));
    }

    #[test]
    fn running_snapshot_carries_real_phase_counts_and_bytes() {
        let mut running = ImportProgressSnapshot::running();
        running.entities_done = 3;
        running.entities_total = Some(4);
        running.bytes = Some(123);
        let json = snapshot_json(Some(&running));
        assert!(json.contains("\"done\":3"));
        assert!(json.contains("\"total\":4"));
        assert!(json.contains("\"bytes\":123"));
        assert!(json.contains("\"running\":true"));
    }

    #[test]
    fn json_escape_protects_the_object_from_a_hostile_error_string() {
        // A real importer error can contain quotes/backslashes/newlines; the
        // encoder must keep the object parseable rather than truncate it.
        let failed = ImportProgressSnapshot::terminal(ImportTerminal::Failed {
            error: CadError::CorruptData("say \"hi\"\\\nnext".into()),
        });
        let json = snapshot_json(Some(&failed));
        assert!(json.contains("\\\""), "quote is escaped: {json}");
        assert!(json.contains("\\\\"), "backslash is escaped: {json}");
        assert!(json.contains("\\n"), "newline is escaped: {json}");
        // The escaping helper itself is pure and directly assertable.
        assert_eq!(json_escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }
}
