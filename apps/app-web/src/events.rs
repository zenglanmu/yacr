//! The wasm event transport: the in-process hub behind `cad_subscribe` and the
//! per-export flush.
//!
//! The wire contract and the pure diff live in
//! `cad_ui_core::session::events`; this module is the browser-side transport:
//!
//! * [`EventHub`] owns the revisioned [`EventBus`] and the previous snapshot, and
//!   turns one status projection into `derive_batch` + **one** `status-changed`
//!   (semantic events first, snapshot last — the frozen ordering).
//! * `cad_subscribe(callback)` registers the JS callback and returns the initial
//!   `status-changed` frame JSON for the current snapshot/profile/revision.
//! * `cad_unsubscribe()` drops the callback.
//! * `flush_live()` runs at the end of every state-mutating canvas export
//!   (`start`/`open_bytes`/`frame`/`resize`/`pan`/`zoom_at`/`fit_view`/
//!   `probe_input`/`invoke`). It builds the snapshot in a short `page` borrow,
//!   **drops it**, then pushes + invokes JS.
//!
//! # No threads, no timers
//!
//! wasm is single-threaded; the bus and callback live in `thread_local`s. Nothing
//! here starts a timer — the host's `requestAnimationFrame` calls `frame()`, and
//! that call is the flush point.
//!
//! # Reentrancy
//!
//! Subscribers **must not re-enter** a canvas export from inside the callback.
//! A flush is guarded: a re-entrant flush returns without touching the bus (it
//! cannot recurse into `RefCell` borrows or JS). The guard is panic-safe (a
//! dropped [`FlushPermit`] clears it).

use std::cell::{Cell, RefCell};

use cad_ui_core::session::events::{derive_batch, CadEvent, EventBus, StatusFrame};
use cad_ui_core::session::{CadStatus, HostProfile};

/// The pure half of the wasm transport: revisioned bus + previous snapshot.
pub struct EventHub {
    bus: EventBus,
    prev: Option<CadStatus>,
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new()
    }
}

impl EventHub {
    /// An empty hub (revision 0, no previous snapshot).
    pub fn new() -> Self {
        Self {
            bus: EventBus::new(),
            prev: None,
        }
    }

    /// The last revision the bus assigned.
    pub fn revision(&self) -> u64 {
        self.bus.revision()
    }

    /// The retained ring (oldest first), for in-process replay/tests.
    pub fn events(&self) -> impl DoubleEndedIterator<Item = &CadEvent> {
        self.bus.events()
    }

    /// The initial `status-changed` frame for a fresh subscription.
    ///
    /// It assigns the next revision and records the snapshot as the diff
    /// baseline, but does not replay the ring. The callback receives later
    /// flushes only; this frame is the call's return value.
    pub fn initial(
        &mut self,
        status: &CadStatus,
        profile: &HostProfile,
        canvas: Option<String>,
    ) -> CadEvent {
        self.prev = Some(status.clone());
        self.bus
            .push_status(
                StatusFrame::new(status.clone(), profile.clone())
                    .with_canvas(canvas.map(cad_ui_core::session::events::CanvasJson::new)),
            )
            .into_iter()
            .next()
            .expect("one status event is pushed")
    }

    /// One flush: semantic events first, then exactly one `status-changed`.
    pub fn flush(
        &mut self,
        status: &CadStatus,
        profile: &HostProfile,
        canvas: Option<String>,
    ) -> Vec<CadEvent> {
        let mut emitted = Vec::new();
        for kind in derive_batch(self.prev.as_ref(), status) {
            emitted.extend(self.bus.push(kind));
        }
        emitted.extend(
            self.bus.push_status(
                StatusFrame::new(status.clone(), profile.clone())
                    .with_canvas(canvas.map(cad_ui_core::session::events::CanvasJson::new)),
            ),
        );
        self.prev = Some(status.clone());
        emitted
    }
}

/// A held flush guard; dropping it clears the reentrancy flag.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // native: tests only
pub(crate) struct FlushPermit;

thread_local! {
    static FLUSHING: Cell<bool> = const { Cell::new(false) };
    static HUB: RefCell<EventHub> = RefCell::new(EventHub::new());
    #[cfg(target_arch = "wasm32")]
    static CALLBACK: RefCell<Option<js_sys::Function>> = const { RefCell::new(None) };
}

/// Enter a flush, or `None` when one is already in progress (re-entrant call).
///
/// Documented contract: **subscribers must not re-enter** a canvas export from
/// inside the callback. The guard makes such a call a no-op instead of a panic or
/// a recursive JS dispatch.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))] // native: tests only
pub(crate) fn begin_flush() -> Option<FlushPermit> {
    FLUSHING.with(|flag| {
        if flag.replace(true) {
            None
        } else {
            Some(FlushPermit)
        }
    })
}

impl Drop for FlushPermit {
    fn drop(&mut self) {
        FLUSHING.with(|flag| flag.set(false));
    }
}

/// Flush the live page session, if any.
///
/// Builds the status in a short `page` borrow, drops it, then pushes events and
/// invokes the JS callback. No `RefCell` borrow of the session state survives into
/// the callback.
#[cfg(target_arch = "wasm32")]
pub(crate) fn flush_live() {
    let Some((status, canvas)) = crate::page::event_snapshot() else {
        return;
    };
    let Some(_permit) = begin_flush() else {
        return;
    };
    let events = HUB.with(|hub| {
        hub.borrow_mut()
            .flush(&status, &HostProfile::dom_canvas(), canvas)
    });
    let callback = CALLBACK.with(|callback| callback.borrow().clone());
    let Some(callback) = callback else {
        return;
    };
    for event in &events {
        let Ok(json) = serde_json::to_string(event) else {
            continue;
        };
        // A throwing subscriber is not a host failure: the event was already
        // retained in the ring and its revision stays observable.
        let _ = callback.call1(&wasm_bindgen::JsValue::NULL, &json.into());
    }
}

/// Subscribe one JS callback to the cad-event stream.
///
/// Returns the **initial** `status-changed` frame JSON (with the current
/// snapshot, `hostProfile` and revision) so the caller can seed its state without
/// polling `cad_status`. Later events are delivered as one JSON string argument
/// per callback invocation.
///
/// One callback per wasm instance (the page carrier's single session). Calling it
/// again replaces the callback; revisions stay monotonic.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn cad_subscribe(callback: js_sys::Function) -> String {
    let (status, canvas) = crate::page::event_snapshot_or_empty();
    let frame = HUB.with(|hub| {
        hub.borrow_mut()
            .initial(&status, &HostProfile::dom_canvas(), canvas)
    });
    CALLBACK.with(|slot| *slot.borrow_mut() = Some(callback));
    serde_json::to_string(&frame).expect("CadEvent is a plain JSON value")
}

/// Drop the JS subscriber. Idempotent; revisions stay monotonic for the next
/// subscription.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn cad_unsubscribe() {
    CALLBACK.with(|slot| *slot.borrow_mut() = None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_ui_core::session::events::CadEventKind;
    use cad_ui_core::session::unsupported_status;

    fn status() -> CadStatus {
        unsupported_status(None)
    }

    #[test]
    fn subscribe_returns_a_status_frame_with_the_dom_canvas_profile() {
        let mut hub = EventHub::new();
        let event = hub.initial(&status(), &HostProfile::dom_canvas(), None);
        assert_eq!(event.revision, 1);
        let json = serde_json::to_string(&event).expect("serializes");
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(value["type"], serde_json::json!("status-changed"));
        assert_eq!(
            value["status"]["hostProfile"]["surfaceMode"],
            serde_json::json!("dom-canvas")
        );
        // The baseline is recorded: an unchanged first flush derives nothing.
        let flush = hub.flush(&status(), &HostProfile::dom_canvas(), None);
        assert_eq!(flush.len(), 1);
        assert_eq!(flush[0].revision, 2);
        assert!(matches!(flush[0].kind, CadEventKind::StatusChanged { .. }));
    }

    #[test]
    fn a_flush_delivers_semantic_events_before_the_status_frame() {
        let mut hub = EventHub::new();
        hub.initial(&status(), &HostProfile::dom_canvas(), None);

        let mut next = status();
        next.selection.count = 2;
        next.error = Some("boom".to_string());
        let events = hub.flush(&next, &HostProfile::dom_canvas(), None);
        let kinds: Vec<_> = events
            .iter()
            .map(|event| match &event.kind {
                CadEventKind::SelectionChanged { .. } => "selection-changed",
                CadEventKind::ErrorRaised { .. } => "error-raised",
                CadEventKind::StatusChanged { .. } => "status-changed",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            ["selection-changed", "error-raised", "status-changed"]
        );
        assert_eq!(events[0].revision, 2);
        assert_eq!(events[1].revision, 3);
        assert_eq!(events[2].revision, 4);
    }

    #[test]
    fn a_reentrant_flush_is_refused_by_the_guard() {
        let first = begin_flush().expect("first flush enters");
        assert!(
            begin_flush().is_none(),
            "a subscribe callback must not re-enter a flush"
        );
        drop(first);
        let again = begin_flush().expect("the guard clears on drop");
        drop(again);
    }
}
