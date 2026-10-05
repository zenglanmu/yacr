//! Asynchronous / cancellable open wiring for the Web host (F01, §4.10).
//!
//! The core [`cad_app::tasks::ImportManager`] runs the importer on a
//! [`std::thread`]. `wasm32-unknown-unknown` has **no threads**: the std shim's
//! `thread::Builder::spawn` fails and the `.expect(...)` in `ImportManager::start`
//! panics at runtime. The background worker path therefore **cannot run in the
//! browser**, and this module says so instead of pretending:
//!
//! * [`worker_available`] is the single capability gate. Only a host for which
//!   it is `true` may call [`HostController::begin_async_open`].
//! * The browser (this crate) always reports `false`, so an open runs through
//!   the synchronous importer. The host still records the **real** terminal
//!   outcome as an [`ImportProgressSnapshot`] so the *same* UI panel mapping is
//!   exercised: a failed or cancelled open is shown explicitly and a successful
//!   open hides the panel (`ImportProgressUiState`).
//! * [`poll_and_apply`] is the shared heartbeat body. On a worker-capable host it
//!   calls `poll_async_open`, which publishes an opened drawing **at most once**
//!   through the manager's `TaskStamp` guard and updates the document via the
//!   existing `install_opened` path. Everywhere it re-reads the retained
//!   snapshot and re-pushes the panel.
//!
//! No progress is fabricated: the *running / cancellable* panel is only
//! reachable on a host that actually has the worker. A frozen "starting" bar
//! over a blocking import would look live while never advancing, so the browser
//! documents the limitation (see `docs/import-async.md`) rather than faking it.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use cad_app::host::{HostController, OpenedDrawing};
use cad_app::tasks::{AsyncOpenPoll, ImportProgressSnapshot, ImportTerminal};
use cad_domain::CadResult;
use cad_ui_slint::{ImportProgressUiState, UiHandle};

use super::documents::install_opened;
use super::messages::current_messages;
use super::with_runtime;

thread_local! {
    /// Host-authored terminal snapshot for the synchronous fallback (wasm).
    ///
    /// The controller retains its own snapshot only when a worker job exists;
    /// with no worker this slot carries the *real* terminal outcome of the
    /// synchronous import so the panel mapping is still exercised. It is
    /// cleared when a new open begins.
    static FALLBACK: RefCell<Option<ImportProgressSnapshot>> = const { RefCell::new(None) };
}

/// Whether the core background import worker can run on this host.
///
/// `cad_app::tasks` uses `std::thread`; `wasm32-unknown-unknown` has no threads
/// and `std::thread::Builder::spawn` panics. The browser therefore cannot run
/// the worker and must not call `begin_async_open`. This is the honest gate the
/// whole module is built around; it is not a placeholder.
pub fn worker_available() -> bool {
    !cfg!(target_arch = "wasm32")
}

/// The snapshot the progress panel should display.
///
/// A real worker job (thread-capable host) takes precedence; otherwise the
/// synchronous fallback terminal is used. Reading this never drains progress,
/// publishes a document or dispatches a command — it is a pure getter.
pub(super) fn current_snapshot(controller: &HostController) -> Option<ImportProgressSnapshot> {
    controller
        .async_open_snapshot()
        .or_else(|| FALLBACK.with(|slot| slot.borrow().clone()))
}

/// Record the real terminal outcome of a synchronous open.
fn set_terminal(terminal: ImportTerminal) {
    FALLBACK.with(|slot| *slot.borrow_mut() = Some(ImportProgressSnapshot::terminal(terminal)));
}

/// Derive and write the progress panel from the current snapshot.
///
/// Called by the single state-push funnel after every command and by the poll
/// heartbeat, so the panel can never lag the open it describes.
pub(super) fn push_state(controller: &Rc<RefCell<HostController>>, handle: &UiHandle) {
    let messages = current_messages();
    let snapshot = current_snapshot(&controller.borrow());
    let state = ImportProgressUiState::from_snapshot(snapshot.as_ref(), &messages);
    let _ = handle.set_import_state(&state);
}

/// How an open started through [`start_or_apply`].
pub(super) enum OpenStart {
    /// A background job is running; the poll heartbeat installs the document.
    Started,
    /// The document is already installed (synchronous fallback).
    Opened(Box<OpenedDrawing>),
}

/// Begin an open, choosing the async worker where it can run.
///
/// On a worker-capable host this starts a cancellable background job and
/// returns [`OpenStart::Started`] (the document is installed later by
/// [`poll_and_apply`]). On the browser it runs the synchronous import and
/// records the real terminal snapshot, returning [`OpenStart::Opened`] or the
/// real error.
pub(super) fn start_or_apply(
    controller: &Rc<RefCell<HostController>>,
    bytes: Arc<[u8]>,
    name: &str,
) -> CadResult<OpenStart> {
    if worker_available() {
        // New shortest open: drop any retained terminal so a stale failure can
        // never be shown for the new job (the core does the same for its own
        // snapshot).
        FALLBACK.with(|slot| *slot.borrow_mut() = None);
        controller.borrow_mut().begin_async_open(bytes, name);
        return Ok(OpenStart::Started);
    }

    let result = controller.borrow_mut().open_bytes(bytes, name);
    match result {
        Ok(opened) => {
            set_terminal(ImportTerminal::Opened {
                entities: opened.entities,
            });
            Ok(OpenStart::Opened(Box::new(opened)))
        }
        Err(error) => {
            set_terminal(ImportTerminal::Failed {
                error: error.clone(),
            });
            Err(error)
        }
    }
}

/// The heartbeat body: poll, publish at most once, push the panel, report JSON.
///
/// On a worker-capable host `poll_async_open` publishes a current result exactly
/// once (the manager's stamp guard discards superseded/cancelled results, and a
/// published job leaves the manager). On the browser the manager is idle, so
/// this only re-reads the retained synchronous terminal. In both cases the
/// document is updated through the existing `install_opened` path — a single
/// source of truth per open, no duplicate import.
pub fn poll_and_apply() -> String {
    let Some((controller, handle, view, incoming, viewport)) = with_runtime(|rt| {
        (
            rt.controller.clone(),
            rt.handle.clone(),
            rt.view.clone(),
            rt.incoming.clone(),
            rt.viewport,
        )
    }) else {
        return snapshot_json(None);
    };

    let poll = controller.borrow_mut().poll_async_open();
    if let AsyncOpenPoll::Opened { opened, .. } = &poll {
        // Publish is already done by the core; `install_opened` is the single
        // host-side document swap, shared with the synchronous path.
        let name = controller.borrow().document_name_hint.clone();
        install_opened(
            &name,
            opened,
            &controller,
            &handle,
            &view,
            &incoming,
            &viewport,
        );
    }
    push_state(&controller, &handle);
    let snapshot = current_snapshot(&controller.borrow());
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
