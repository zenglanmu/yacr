//! The wasm event transport: the in-process hub behind `cad_subscribe` and the
//! per-export flush.
//!
//! The wire contract and the pure diff live in
//! `cad_ui_core::session::events`; this module is the browser-side transport:
//!
//! * [`EventHub`] owns the revisioned [`EventBus`] and the previous snapshot, and
//!   turns one status projection into `derive_batch` + (the vendor pass's own
//!   events) + at most one `cursor-changed` (the volatile pair) + at most one
//!   `status-changed` (first flush or a semantic change). Order: semantic, vendor,
//!   cursor, snapshot.
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

use cad_ui_core::session::events::{
    cursor_changed, derive_batch, same_status_ignoring_volatile, CadEvent, CadEventKind, EventBus,
    StatusFrame,
};
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

    /// One flush: semantic events, then at most one `cursor-changed`, then at
    /// most one `status-changed` (first flush or semantic change).
    pub fn flush(
        &mut self,
        status: &CadStatus,
        profile: &HostProfile,
        canvas: Option<String>,
    ) -> Vec<CadEvent> {
        self.flush_with_vendor(status, profile, canvas, Vec::new)
    }

    /// One flush that also carries transport-owned events (the vendor pass).
    ///
    /// Order per flush, and it is the frozen one: the derived semantic events,
    /// then whatever `vendor` takes (at most one `vendor-pending`, then at most
    /// one `vendor-settled`; see [`crate::vendor::take_event_kinds`]), then at
    /// most one `cursor-changed` for the volatile pair, then at most one
    /// `status-changed` (the first flush, or a semantic change). The vendor kinds
    /// are pushed **before** the snapshot so a consumer that reacts to
    /// `vendor-pending` by calling `cad_vendor_records` never sees a stale status
    /// first.
    ///
    /// `vendor` is called once per flush with the hub already borrowed; it must
    /// only take from a store (no re-entrant flush, no bus access).
    pub fn flush_with_vendor(
        &mut self,
        status: &CadStatus,
        profile: &HostProfile,
        canvas: Option<String>,
        vendor: impl FnOnce() -> Vec<CadEventKind>,
    ) -> Vec<CadEvent> {
        let mut emitted = Vec::new();
        for kind in derive_batch(self.prev.as_ref(), status) {
            emitted.extend(self.bus.push(kind));
        }
        for kind in vendor() {
            emitted.extend(self.bus.push(kind));
        }
        let volatile = self
            .prev
            .as_ref()
            .is_some_and(|prev| cursor_changed(prev, status));
        let semantic = self
            .prev
            .as_ref()
            .is_none_or(|prev| !same_status_ignoring_volatile(prev, status));
        if volatile {
            emitted.extend(self.bus.push(CadEventKind::CursorChanged {
                cursor_world: status.cursor_world,
                frame_count: status.frame_count,
                view: status.view.clone(),
            }));
        }
        if semantic {
            emitted.extend(
                self.bus.push_status(
                    StatusFrame::new(status.clone(), profile.clone())
                        .with_canvas(canvas.map(cad_ui_core::session::events::CanvasJson::new)),
                ),
            );
        }
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
        hub.borrow_mut().flush_with_vendor(
            &status,
            &HostProfile::dom_canvas(),
            canvas,
            crate::vendor::take_event_kinds,
        )
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
        // The baseline is recorded: an unchanged first flush emits nothing.
        let flush = hub.flush(&status(), &HostProfile::dom_canvas(), None);
        assert!(flush.is_empty(), "{flush:?}");
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

    #[test]
    fn a_vendor_flush_orders_semantic_then_vendor_then_status() {
        let mut hub = EventHub::new();
        hub.initial(&status(), &HostProfile::dom_canvas(), None);

        let mut next = status();
        next.selection.count = 2;
        let events = hub.flush_with_vendor(&next, &HostProfile::dom_canvas(), None, || {
            vec![
                CadEventKind::VendorPending {
                    token: 7,
                    generation: 7,
                    entities: 3,
                },
                CadEventKind::VendorSettled {
                    token: 7,
                    applied: 0,
                    diagnostics: vec!["vendor.unsupported_layout".to_string()],
                },
            ]
        });
        let kinds: Vec<_> = events
            .iter()
            .map(|event| match &event.kind {
                CadEventKind::SelectionChanged { .. } => "selection-changed",
                CadEventKind::VendorPending { .. } => "vendor-pending",
                CadEventKind::VendorSettled { .. } => "vendor-settled",
                CadEventKind::StatusChanged { .. } => "status-changed",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "selection-changed",
                "vendor-pending",
                "vendor-settled",
                "status-changed"
            ]
        );
        // Exactly one snapshot, and it is last: a consumer reacting to the pending
        // frame never reads a status that predates it.
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.kind, CadEventKind::StatusChanged { .. }))
                .count(),
            1
        );
        // Revisions stay strictly +1 across the whole batch.
        assert_eq!(events.first().expect("events").revision, 2);
        assert_eq!(events.last().expect("events").revision, 5);

        // `flush` itself carries no vendor kinds; the next tick is unchanged, so
        // it emits nothing at all (semantic status was already delivered).
        let plain = hub.flush(&next, &HostProfile::dom_canvas(), None);
        assert!(plain.is_empty(), "{plain:?}");
    }

    #[test]
    fn a_volatile_only_flush_emits_one_cursor_changed_and_no_status() {
        let mut hub = EventHub::new();
        hub.initial(&status(), &HostProfile::dom_canvas(), None);

        let mut next = status();
        next.frame_count = 21;
        next.cursor_world = Some([1.0, -2.5, 0.0]);
        let events = hub.flush(&next, &HostProfile::dom_canvas(), None);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].revision, 2);
        assert!(matches!(
            &events[0].kind,
            CadEventKind::CursorChanged {
                cursor_world: Some([1.0, -2.5, 0.0]),
                frame_count: 21,
                ..
            }
        ));
        let json = serde_json::to_value(&events[0]).expect("serializes");
        assert_eq!(json["type"], serde_json::json!("cursor-changed"));
        assert_eq!(json["cursorWorld"], serde_json::json!([1.0, -2.5, 0.0]));
        assert_eq!(json["frameCount"], serde_json::json!(21));

        // The same volatile pair again: nothing new.
        assert!(hub
            .flush(&next, &HostProfile::dom_canvas(), None)
            .is_empty());
    }

    #[test]
    fn a_semantic_plus_volatile_flush_orders_derived_cursor_then_status() {
        let mut hub = EventHub::new();
        hub.initial(&status(), &HostProfile::dom_canvas(), None);

        let mut next = status();
        next.selection.count = 2;
        next.frame_count = 30;
        next.cursor_world = Some([4.0, 5.0, 0.0]);
        let events = hub.flush(&next, &HostProfile::dom_canvas(), None);
        let kinds: Vec<_> = events
            .iter()
            .map(|event| match &event.kind {
                CadEventKind::SelectionChanged { .. } => "selection-changed",
                CadEventKind::CursorChanged { .. } => "cursor-changed",
                CadEventKind::StatusChanged { .. } => "status-changed",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            ["selection-changed", "cursor-changed", "status-changed"]
        );
        assert_eq!(events[0].revision, 2);
        assert_eq!(events[2].revision, 4);
    }

    #[test]
    fn a_flush_with_vendor_orders_cursor_changed_before_the_optional_status() {
        let mut hub = EventHub::new();
        hub.initial(&status(), &HostProfile::dom_canvas(), None);

        let mut next = status();
        next.frame_count = 7;
        let events = hub.flush_with_vendor(&next, &HostProfile::dom_canvas(), None, || {
            vec![CadEventKind::VendorPending {
                token: 3,
                generation: 3,
                entities: 1,
            }]
        });
        let kinds: Vec<_> = events
            .iter()
            .map(|event| match &event.kind {
                CadEventKind::VendorPending { .. } => "vendor-pending",
                CadEventKind::CursorChanged { .. } => "cursor-changed",
                CadEventKind::StatusChanged { .. } => "status-changed",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["vendor-pending", "cursor-changed"]);
    }
}
