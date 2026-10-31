//! Desktop event transport: the Tauri sink and `Shared`'s per-flush bridge.
//!
//! The **wire contract and the bus** live in `cad_ui_core::session::events`
//! (tauri-free). This module is the one place that knows the desktop transport:
//!
//! * [`TauriEventSink`] implements [`EventSink`] with
//!   [`tauri::Emitter::emit`] on the window handle already stored in
//!   [`Shared::window`] (set by each host's `host/init.rs`), emitting the
//!   `"cad_event"` event the React listener subscribes to.
//! * [`flush_events_current`] is the session-independent flush the owner tick
//!   calls: it derives the semantic batch against the previous flush, pushes
//!   those **first**, then exactly one `status-changed`, releases the lock, and
//!   only then delivers every event. It projects the same status `cad_status`
//!   answers (host error + live popup included) even before a session exists, so
//!   probe mode, the lazy-session window and a creation failure all keep the push
//!   channel truthful.
//!
//! # Acceptance of the `emit` cost
//!
//! Emitting the **full** status every rendered frame (≈60 Hz on the native hosts)
//! is deliberate: the alternative was a subscription-gating Tauri command, which
//! would need a second IPC round-trip and a host-side subscriber registry. One
//! `status-changed` is ≈2–25 KB of JSON (the command log and selection rows are
//! the variable part), so a steady 60 Hz stream is ≈0.1–1.5 MB/s of webview IPC.
//! The in-process ring retains the last
//! [`EVENT_RING_CAPACITY`](cad_ui_core::session::events::EVENT_RING_CAPACITY) full events
//! (≈0.5–6 MB) as a replay window; it is not the webview queue.
//!
//! The per-frame status is emitted even when nothing changed: that is how
//! `frameCount`/`cursorWorld` travel. An unchanged status derives **no** semantic
//! events (`derive_batch` sees identical public fields), so the idle case is one
//! `status-changed` per frame and nothing else. There is **no polling timer** here
//! — the owner tick is the only producer.
//!
//! # Status bytes
//!
//! The flushed `status-changed.status` is projected exactly like `cad_status`:
//! the shell's mirrored session snapshot + the host error + the projected popup,
//! wrapped in the same [`StatusFrame`] (`CadStatus` flattened, `hostProfile`,
//! optional `canvasSession`). Both paths call [`project_current_status`], so a
//! consumer can treat the event payload and the `cad_status` answer as the same
//! bytes by construction.

use std::sync::{Arc, Mutex};

use cad_ui_core::session::events::{
    derive_batch, CadEvent, CadEventKind, CanvasJson, EventSink, StatusFrame,
};
use cad_ui_core::session::{unsupported_status, CadStatus, HostProfile};
use cad_ui_core::shell::{projected_popup, projected_status};
use serde::Serialize;
use tauri::Emitter;

use crate::state::{lock, Shared};

/// The status `cad_status` would answer from `shared`, without locking it.
///
/// This is the **one** projection: the session mirror (or the explicit
/// unsupported status before a session exists) plus the host transient error,
/// then the host's live popup. Both [`crate::commands::cad_status_impl`] and the
/// event flush call it, so the push channel and the IPC read cannot diverge.
pub fn project_current_status(shared: &Shared) -> CadStatus {
    let host_error = shared
        .shell
        .cad_host_error
        .as_ref()
        .map(|error| error.text.as_str());
    let status = match shared.shell.cad_status.clone() {
        Some(snapshot) => projected_status(snapshot, host_error),
        None => unsupported_status(None),
    };
    projected_popup(status, shared.shell.popup.as_ref())
}

/// Flush the current status as one owner-tick event batch.
///
/// Locks `shared` once, projects the exact `cad_status` payload, pushes the
/// semantic diff plus exactly one `status-changed`, releases the lock, and only
/// then delivers the events through the registered window sink. A live session is
/// **not** required: probe mode, the lazy-session window and a session-creation
/// failure still emit (the unsupported status carries the failure text).
///
/// `canvas` is a host-serialized `canvasSession` descriptor, or `None` for a
/// carrier without one; it is embedded unchanged so the status stays
/// byte-identical to `cad_status`.
pub fn flush_events_current<C: Serialize>(
    shared: &Arc<Mutex<Shared>>,
    profile: &HostProfile,
    canvas: Option<C>,
) -> Vec<CadEvent> {
    let canvas = canvas.map(|descriptor| {
        serde_json::to_string(&descriptor).expect("the canvas descriptor is plain JSON")
    });
    let (emitted, sinks) = {
        let mut state = lock(shared);
        state.ensure_event_sink();
        let status = project_current_status(&state);
        let mut emitted = Vec::new();
        for kind in derive_batch(state.events_prev.as_ref(), &status) {
            emitted.extend(state.events.push_quiet(kind));
        }
        let frame = StatusFrame::new(status.clone(), profile.clone())
            .with_canvas(canvas.map(CanvasJson::new));
        emitted.extend(state.events.push_quiet(CadEventKind::StatusChanged {
            status: Box::new(frame),
        }));
        state.events_prev = Some(status);
        (emitted, state.events.sink_handles())
    };
    // The guard is gone: a transport emit never runs while `Shared` is locked.
    for event in &emitted {
        for sink in &sinks {
            sink.deliver(event);
        }
    }
    emitted
}

/// [`EventSink`] over the host's live Tauri window.
///
/// The window handle is cloned out of [`Shared::window`] (the same handle the
/// tick uses for size/state), so no second window registry exists.
pub struct TauriEventSink {
    window: tauri::Window,
}

impl TauriEventSink {
    /// Build a sink for `window`.
    pub fn new(window: tauri::Window) -> Self {
        Self { window }
    }
}

impl EventSink for TauriEventSink {
    fn deliver(&self, event: &CadEvent) {
        // The React listener subscribes to this one event name. A failed emit
        // (window closing) is not escalated: the next tick simply retries, and
        // the revision gap is recoverable through `cad_status`/re-subscription.
        let _ = self.window.emit("cad_event", event);
    }
}

impl Shared {
    /// Register the window sink on first use (the window exists by the first
    /// owner tick; creating a second sink is never done).
    fn ensure_event_sink(&mut self) {
        if self.events.subscribers() == 0 {
            if let Some(window) = self.window.clone() {
                self.events.add_sink(Box::new(TauriEventSink::new(window)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_ui_core::launch::LaunchOptions;
    use cad_ui_core::popup::POPUP_LABEL;
    use cad_ui_core::session::CadPopup;

    fn shared() -> Arc<Mutex<Shared>> {
        Arc::new(Mutex::new(Shared::new(LaunchOptions::default())))
    }

    fn kinds(events: &[CadEvent]) -> Vec<&'static str> {
        events
            .iter()
            .map(|event| match &event.kind {
                CadEventKind::StatusChanged { .. } => "status-changed",
                CadEventKind::CommandAdmitted { .. } => "command-admitted",
                CadEventKind::CommandRejected { .. } => "command-rejected",
                CadEventKind::SelectionChanged { .. } => "selection-changed",
                CadEventKind::ToolStep { .. } => "tool-step",
                CadEventKind::DocumentOpened { .. } => "document-opened",
                CadEventKind::DocumentClosed => "document-closed",
                CadEventKind::ErrorRaised { .. } => "error-raised",
                CadEventKind::PopupChanged { .. } => "popup-changed",
                CadEventKind::EventsDropped { .. } => "events-dropped",
                _ => "unknown",
            })
            .collect()
    }

    fn status_of(events: &[CadEvent]) -> &StatusFrame {
        match &events.last().expect("a flush emits events").kind {
            CadEventKind::StatusChanged { status } => status,
            other => panic!("last event is not status-changed: {other:?}"),
        }
    }

    #[test]
    fn a_no_session_flush_emits_the_unsupported_status_and_profile() {
        let shared = shared();
        let first = flush_events_current(&shared, &HostProfile::native_below(), None::<String>);
        assert_eq!(kinds(&first), ["status-changed"]);
        let frame = status_of(&first);
        assert_eq!(frame.profile, HostProfile::native_below());
        assert_eq!(frame.status, project_current_status(&lock(&shared)));
        let status = &frame.status;
        assert!(status.document_label.is_none());
        assert_eq!(status.entity_count, None);

        // An unchanged second tick: exactly one per-frame `status-changed`.
        let second = flush_events_current(&shared, &HostProfile::native_below(), None::<String>);
        assert_eq!(kinds(&second), ["status-changed"]);
        assert_eq!(second[0].revision, 2);
    }

    #[test]
    fn a_creation_failure_text_reaches_the_no_session_event() {
        let shared = shared();
        lock(&shared).shell.cad_status = Some(unsupported_status(Some("open failed".into())));
        let events = flush_events_current(&shared, &HostProfile::native_below(), None::<String>);
        assert_eq!(kinds(&events), ["status-changed"]);
        assert_eq!(
            status_of(&events).status.error.as_deref(),
            Some("open failed")
        );
    }

    #[test]
    #[allow(deprecated)] // the frozen `CadPopup::label` is required to build a popup
    fn a_popup_set_while_no_session_appears_in_the_next_event() {
        let shared = shared();
        flush_events_current(&shared, &HostProfile::native_below(), None::<String>);

        lock(&shared).shell.popup = Some(CadPopup {
            kind: "about".to_string(),
            label: POPUP_LABEL.to_string(),
            modal: true,
        });
        let events = flush_events_current(&shared, &HostProfile::native_below(), None::<String>);
        assert_eq!(kinds(&events), ["popup-changed", "status-changed"]);
        assert!(matches!(
            &events[0].kind,
            CadEventKind::PopupChanged { popup: Some(popup) } if popup.kind == "about"
        ));
        assert_eq!(
            status_of(&events)
                .status
                .popup
                .as_ref()
                .map(|p| p.kind.as_str()),
            Some("about")
        );
    }

    #[test]
    fn a_changed_mirrored_status_emits_the_semantic_event_before_the_status_event() {
        let shared = shared();
        flush_events_current(&shared, &HostProfile::native_below(), None::<String>);

        let mut next = unsupported_status(None);
        next.selection.count = 2;
        next.selection.empty = false;
        next.error = Some("resource: missing SHX".to_string());
        lock(&shared).shell.cad_status = Some(next);
        let events = flush_events_current(&shared, &HostProfile::native_below(), None::<String>);
        assert_eq!(
            kinds(&events),
            ["selection-changed", "error-raised", "status-changed"]
        );
        assert!(events[0].revision < events[1].revision);
        assert!(events[1].revision < events[2].revision);
    }

    #[test]
    fn a_flush_delivers_through_the_registered_sink_after_projecting() {
        use cad_ui_core::session::events::EventSink;

        struct Recording(Arc<Mutex<Vec<u64>>>);
        impl EventSink for Recording {
            fn deliver(&self, event: &CadEvent) {
                self.0.lock().expect("sink lock").push(event.revision);
            }
        }

        let shared = shared();
        let seen = Arc::new(Mutex::new(Vec::new()));
        lock(&shared)
            .events
            .add_sink(Box::new(Recording(Arc::clone(&seen))));
        let events = flush_events_current(&shared, &HostProfile::native_below(), None::<String>);
        assert_eq!(kinds(&events), ["status-changed"]);
        assert_eq!(*seen.lock().expect("sink lock"), vec![1]);
    }

    #[test]
    fn the_sink_is_registered_once_when_a_window_exists() {
        // No window in a unit test: no transport sink is fabricated.
        let shared = shared();
        flush_events_current(&shared, &HostProfile::native_below(), None::<String>);
        assert_eq!(lock(&shared).events.subscribers(), 0);
        assert_eq!(lock(&shared).events.len(), 1);
    }
}
