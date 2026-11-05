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
//!   calls. It derives the semantic batch against the previous flush, pushes
//!   those **first**, then (after the vendor events) a small `cursor-changed`
//!   when only the volatile pair changed, then the full `status-changed` only on
//!   the first flush or a semantic change; it releases the lock and only then
//!   delivers every event. It projects the same status `cad_status` answers (host
//!   error + live popup included) even before a session exists, so probe mode, the
//!   lazy-session window and a creation failure all keep the push channel
//!   truthful.
//!
//! # Cadence (ADR 0028 phase 3)
//!
//! The **first** flush emits one full `status-changed` (the wire shape is
//! unchanged: ≈2–25 KB of JSON, command log/selection rows are the variable part).
//! Later flushes emit:
//!
//! * semantic events from [`derive_batch`] (unchanged),
//! * at most one `cursor-changed { cursorWorld, frameCount }` when either
//!   volatile field moved,
//! * at most one `status-changed` only when something **semantic** changed
//!   ([`same_status_ignoring_volatile`]).
//!
//! An idle renderer therefore emits a tiny `cursor-changed` per presented frame,
//! not a full status; a truly static tick emits nothing. The in-process ring
//! retains the last
//! [`EVENT_RING_CAPACITY`](cad_ui_core::session::events::EVENT_RING_CAPACITY)
//! events as a replay window; it is not the webview queue. There is **no polling
//! timer** here — the owner tick is the only producer.
//!
//! The React consumer (`apps/desktop-ui/src/cad/use-cad-events.ts`) applies a
//! semantic status to the shell and feeds `cursorWorld`/`frameCount` from
//! `cursor-changed` to two leaf stores (`cad/status-cadence.ts`).
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
    cursor_changed, derive_batch, same_status_ignoring_volatile, CadEvent, CadEventKind,
    CanvasJson, EventSink, StatusFrame,
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
        // Clone the baseline so the pushes below can borrow `state` mutably.
        let previous = state.events_prev.clone();
        let batch = derive_batch(previous.as_ref(), &status);
        // The volatile pair rides its own small event; a full status is only
        // needed on the first flush or a semantic change (ADR 0028 phase 3).
        let volatile = previous
            .as_ref()
            .is_some_and(|prev| cursor_changed(prev, &status));
        let semantic = previous
            .as_ref()
            .is_none_or(|prev| !same_status_ignoring_volatile(prev, &status));
        let closed = batch
            .iter()
            .any(|kind| matches!(kind, CadEventKind::DocumentClosed));
        let mut emitted = Vec::new();
        for kind in batch {
            emitted.extend(state.events.push_quiet(kind));
        }
        if closed {
            state.vendor.clear();
            state.vendor_pending_representations = None;
            state.vendor_representations_clear = true;
        }
        // The owner tick stages the records copied by the import; the flush only
        // announces them once. A drawing with no TCH records has no pass.
        if let Some((token, generation, entities)) = state.vendor.announce_pending() {
            emitted.extend(state.events.push_quiet(CadEventKind::VendorPending {
                token,
                generation,
                entities,
            }));
        }
        if let Some((token, ack)) = state.vendor.take_unreported() {
            emitted.extend(state.events.push_quiet(CadEventKind::VendorSettled {
                token,
                applied: ack.applied,
                diagnostics: ack.diagnostics,
            }));
        }
        // Order: derived semantic events, vendor events, cursor-changed, status.
        if volatile {
            emitted.extend(state.events.push_quiet(CadEventKind::CursorChanged {
                cursor_world: status.cursor_world,
                frame_count: status.frame_count,
            }));
        }
        if semantic {
            let frame = StatusFrame::new(status.clone(), profile.clone())
                .with_canvas(canvas.map(CanvasJson::new));
            emitted.extend(state.events.push_quiet(CadEventKind::StatusChanged {
                status: Box::new(frame),
            }));
        }
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
                CadEventKind::VendorPending { .. } => "vendor-pending",
                CadEventKind::VendorSettled { .. } => "vendor-settled",
                CadEventKind::EventsDropped { .. } => "events-dropped",
                CadEventKind::CursorChanged { .. } => "cursor-changed",
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

        // An unchanged second tick emits nothing: the first flush already
        // delivered the full status (`frameCount`/`cursorWorld` ride a separate
        // `cursor-changed` only when they actually change).
        let second = flush_events_current(&shared, &HostProfile::native_below(), None::<String>);
        assert!(
            second.is_empty(),
            "an unchanged tick emits no event: {second:?}"
        );
    }

    #[test]
    fn a_volatile_only_tick_emits_one_cursor_changed_and_no_status() {
        let shared = shared();
        flush_events_current(&shared, &HostProfile::native_below(), None::<String>);

        let mut next = unsupported_status(None);
        next.frame_count = 9;
        next.cursor_world = Some([1.5, -2.0, 0.0]);
        lock(&shared).shell.cad_status = Some(next.clone());
        let events = flush_events_current(&shared, &HostProfile::native_below(), None::<String>);
        assert_eq!(kinds(&events), ["cursor-changed"], "{events:?}");
        assert!(matches!(
            &events[0].kind,
            CadEventKind::CursorChanged {
                cursor_world: Some([1.5, -2.0, 0.0]),
                frame_count: 9
            }
        ));
        assert_eq!(events[0].revision, 2);

        // The same volatile pair again: nothing new to report.
        let again = flush_events_current(&shared, &HostProfile::native_below(), None::<String>);
        assert!(again.is_empty(), "{again:?}");
    }

    #[test]
    fn a_semantic_plus_volatile_tick_orders_semantic_then_cursor_then_status() {
        let shared = shared();
        flush_events_current(&shared, &HostProfile::native_below(), None::<String>);

        let mut next = unsupported_status(None);
        next.frame_count = 12;
        next.cursor_world = Some([3.0, 4.0, 5.0]);
        next.selection.count = 2;
        next.selection.empty = false;
        next.error = Some("boom".to_string());
        lock(&shared).shell.cad_status = Some(next.clone());
        let events = flush_events_current(&shared, &HostProfile::native_below(), None::<String>);
        assert_eq!(
            kinds(&events),
            [
                "selection-changed",
                "error-raised",
                "cursor-changed",
                "status-changed"
            ]
        );
        // Revisions are strictly +1 and the full status is last (byte-identical
        // `StatusFrame`, same projection as `cad_status`).
        assert!(events
            .windows(2)
            .all(|pair| pair[0].revision + 1 == pair[1].revision));
        let frame = status_of(&events);
        assert_eq!(frame.status.selection.count, 2);
        assert_eq!(frame.status.error.as_deref(), Some("boom"));
        assert_eq!(frame.status.frame_count, 12);
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
    fn a_staged_pass_is_announced_once_and_a_settled_ack_is_emitted_once() {
        use cad_app::{VendorRecord, VendorSource};

        let shared = shared();
        lock(&shared).vendor.enable();
        flush_events_current(&shared, &HostProfile::native_below(), None::<String>);

        lock(&shared)
            .vendor
            .stage(vec![VendorRecord {
                class_name: "TCH_WALL".to_string(),
                handle: 0x2c,
                dwg_version: 1027,
                source: VendorSource::UnknownEntity,
                main: vec![0x06],
                main_bits: 8,
                text: Vec::new(),
                text_bits: 0,
                handle_stream: Vec::new(),
                handle_bits: 0,
                main_skip_bits: 0,
                text_skip_bits: 0,
                handle_skip_bits: 0,
                references: Vec::new(),
            }])
            .expect("stages");
        let first = flush_events_current(&shared, &HostProfile::native_below(), None::<String>);
        assert!(kinds(&first).contains(&"vendor-pending"), "{first:?}");
        let pending = first
            .iter()
            .find_map(|event| match &event.kind {
                CadEventKind::VendorPending {
                    token,
                    generation,
                    entities,
                } => Some((*token, *generation, *entities)),
                _ => None,
            })
            .expect("one pending event");
        assert_eq!(pending.1, pending.0, "v2 uses the token as the generation");
        assert_eq!(pending.2, 1, "the staged record count");

        // A second flush must not announce the same pass again.
        let second = flush_events_current(&shared, &HostProfile::native_below(), None::<String>);
        assert!(!kinds(&second).contains(&"vendor-pending"), "{second:?}");

        // Settle the pass the way `cad_vendor_apply` does, then let the tick emit
        // exactly one `vendor-settled`.
        let (token, generation) = lock(&shared).vendor.active().expect("active");
        use crate::vendor::{RESULT_MAGIC, VENDOR_ABI_VERSION};
        let mut frame = Vec::new();
        frame.extend_from_slice(&RESULT_MAGIC.to_le_bytes());
        frame.extend_from_slice(&VENDOR_ABI_VERSION.to_le_bytes());
        frame.extend_from_slice(&token.to_le_bytes());
        frame.extend_from_slice(&generation.to_le_bytes());
        frame.extend_from_slice(&1u32.to_le_bytes()); // unsupported
        frame.extend_from_slice(&0u32.to_le_bytes()); // applied
        frame.extend_from_slice(&0u32.to_le_bytes()); // diagnostics
        frame.extend_from_slice(&0u32.to_le_bytes()); // entity groups
        lock(&shared)
            .vendor
            .settle_bytes(token, generation, &frame)
            .expect("settles");
        let third = flush_events_current(&shared, &HostProfile::native_below(), None::<String>);
        assert!(kinds(&third).contains(&"vendor-settled"), "{third:?}");
        let fourth = flush_events_current(&shared, &HostProfile::native_below(), None::<String>);
        assert!(!kinds(&fourth).contains(&"vendor-settled"), "{fourth:?}");
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
