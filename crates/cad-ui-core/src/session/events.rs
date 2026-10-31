//! The **cad-event wire**: one envelope per host event, a revisioned bus and the
//! pure status-diff that derives the semantic events.
//!
//! The frozen wire shape (frontend contract) is:
//!
//! ```json
//! {"revision": 7, "type": "status-changed", "status": { ...CadStatus..., "hostProfile": { ... } }}
//! {"revision": 8, "type": "command-admitted", "command": "LINE"}
//! {"revision": 9, "type": "command-rejected", "command": "ZOOM", "code": "zoom.argument_required"}
//! {"revision": 10, "type": "selection-changed", "count": 2, "empty": false}
//! {"revision": 11, "type": "tool-step", "tool": "draw.line", "step": 2}
//! {"revision": 12, "type": "document-opened", "label": "plan.dwg"}
//! {"revision": 13, "type": "document-closed"}
//! {"revision": 14, "type": "error-raised", "error": "<verbatim host string>"}
//! {"revision": 15, "type": "popup-changed", "popup": null}
//! {"revision": 16, "type": "events-dropped", "count": 3}
//! ```
//!
//! [`CadEvent`] flattens [`CadEventKind`], which is internally tagged by `type`
//! (kebab-case), so the envelope is exactly `revision` + the variant's own keys.
//! The enum is `#[non_exhaustive]`: a consumer must ignore an unknown `type`
//! instead of failing on it, which is what lets a later event be additive.
//!
//! # `status-changed` is per rendered frame, not a diff
//!
//! Per the frozen decision, every flush emits **one** `status-changed` carrying
//! the complete status envelope ([`StatusFrame`]: the flattened [`CadStatus`] plus
//! `hostProfile` and the optional `canvasSession`, byte-identical to `cad_status`).
//! There is deliberately no diff gate: `frameCount`/`cursorWorld` ride this event,
//! so no separate `frame-advanced`/`cursor-moved` events exist.
//!
//! [`derive_batch`] is the *other* half: it derives the semantic/lifecycle events
//! (command, selection, tool, document, error, popup) from two status snapshots.
//! It never touches session internals and never assigns a revision — the
//! [`EventBus`] does that.
//!
//! # Ordering
//!
//! A flush pushes every derived semantic event first and the `status-changed`
//! last (see [`EventBus`] docs). Consumers that only need the snapshot can ignore
//! everything else; consumers that react to a command see it before the snapshot
//! that already contains its result.
//!
//! # Command-log diffing and truncation
//!
//! The status carries the host's **bounded** command log
//! (`cad_host::COMMAND_LOG_CAPACITY` = 200, oldest dropped, cleared on a document
//! replacement). [`derive_batch`] therefore finds the longest suffix of `prev`'s
//! log that is a prefix of `next`'s log and treats the remainder as appended:
//!
//! ```text
//! prev [a, b, c]   next [b, c, d, e]   -> longest common suffix/prefix [b, c] -> appended [d, e]
//! ```
//!
//! Limitation, stated explicitly: when the two logs have **no** overlapping entry
//! (a document replacement cleared the log, or more than the whole retained window
//! was produced between two flushes), every entry of `next` is reported as new.
//! The log is observational, so a duplicated semantic event is noise, never a
//! lost state change; the authoritative state still rides `status-changed`.
//!
//! # What the log proves, and what it does not
//!
//! A `Command` entry means the host **admitted** a selector — not that its
//! operation finished. The derived event is therefore `command-admitted`; there
//! is no `command-completed` event. An `Error` entry carries only a diagnostic
//! `code`, never a command name (`cad_host::CommandLog::push_error`), so
//! attribution is strictly batch-local:
//!
//! * `command-rejected { command, code }` is emitted only when the `Error` entry
//!   and the admitted `Command` entry it **directly follows** are both in the
//!   appended batch (error index > 0, `appended[error_index - 1]`, both fields
//!   non-empty).
//! * An `Error` whose preceding `Command` entry predates the appended batch is
//!   **never** attributed. An `OPEN` admitted in an earlier flush followed later
//!   by an internal failure does **not** become
//!   `command-rejected { command: "OPEN" }`; the error still reaches the
//!   consumer through `status.error` and the log.
//!
//! The event stream is deliberately not a complete error feed. An error that is
//! set and cleared entirely inside one flush window produces no `error-raised`
//! (the diff only compares two snapshots), and at least one refusal path
//! (`CadSession::zoom_step` with no active viewport) currently records neither a
//! log entry nor `status.error`. Absence of an event is therefore not proof that
//! nothing was refused; the `status-changed` snapshot remains authoritative.

use std::collections::VecDeque;
use std::sync::Arc;

use serde::ser::Error as _;
use serde::{Serialize, Serializer};

use super::command_log::{CadCommandLogEntry, CadCommandLogKind};
use super::profile::HostProfile;
use super::status::{CadPopup, CadStatus};

/// Capacity of the in-process event ring.
///
/// The ring is a *replay window for in-process subscribers*, not the webview
/// queue (the webview is never replayed). Overflow drops the oldest event and
/// emits one [`CadEventKind::EventsDropped`] marker per push that overflowed.
pub const EVENT_RING_CAPACITY: usize = 256;

/// A serialized `canvasSession` descriptor, embedded as raw JSON.
///
/// The descriptor lives in the host crates (`app-linux`, `app-web`), so this
/// layer stores the host's own serialization instead of defining a look-alike
/// type. Keeping the **raw** bytes (rather than a `serde_json::Value`, which
/// would sort keys) is what makes `status-changed.status` byte-identical to
/// `cad_status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasJson(String);

impl CanvasJson {
    /// Take an already serialized descriptor object.
    pub fn new(json: String) -> Self {
        Self(json)
    }

    /// The raw JSON text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for CanvasJson {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match serde_json::value::RawValue::from_string(self.0.clone()) {
            Ok(raw) => raw.serialize(serializer),
            Err(error) => Err(S::Error::custom(error)),
        }
    }
}

/// The `status` payload of `status-changed`: the `WireStatus` envelope.
///
/// `CadStatus` is flattened **first**, so its frozen field order is unchanged;
/// then `hostProfile`, then the optional `canvasSession`. This is the exact
/// serialization `cad_status` performs, without a `Value` round-trip.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StatusFrame {
    /// The frozen status fields, first and in order.
    #[serde(flatten)]
    pub status: CadStatus,
    /// The always-present typed carrier description.
    #[serde(rename = "hostProfile")]
    pub profile: HostProfile,
    /// The wasm/`dom-canvas` session descriptor, or absent when undeclared.
    #[serde(rename = "canvasSession", skip_serializing_if = "Option::is_none")]
    pub canvas: Option<CanvasJson>,
}

impl StatusFrame {
    /// A frame with no `canvasSession` key.
    pub fn new(status: CadStatus, profile: HostProfile) -> Self {
        Self {
            status,
            profile,
            canvas: None,
        }
    }

    /// Attach a host-serialized `canvasSession` descriptor.
    pub fn with_canvas(mut self, canvas: Option<CanvasJson>) -> Self {
        self.canvas = canvas;
        self
    }
}

/// The kind of one host event (the flattened body of [`CadEvent`]).
///
/// Internally tagged by `type`, kebab-case. `#[non_exhaustive]`: consumers must
/// ignore unknown variants.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum CadEventKind {
    /// One rendered frame / flush: the **complete** status envelope.
    StatusChanged {
        /// The `WireStatus`-shaped status (flattened `CadStatus` + `hostProfile`
        /// + optional `canvasSession`).
        status: Box<StatusFrame>,
    },
    /// A named selector was admitted (recorded in the host command log).
    ///
    /// Admission is not completion: this says the log recorded the selector, not
    /// that its operation finished.
    CommandAdmitted {
        /// The stable selector name (`LINE`, `DIST`, …).
        command: String,
    },
    /// A command was refused; `code` is the host's stable diagnostic code.
    ///
    /// Emitted only when the refusal `Error` entry and the admitted `Command`
    /// entry it directly follows are both in the same appended batch; see the
    /// module docs for the exact rule.
    CommandRejected {
        /// The admitted selector the refusal directly follows (never empty).
        command: String,
        /// The stable machine diagnostic code (never empty).
        code: String,
    },
    /// The selection count/emptiness changed.
    SelectionChanged {
        /// Real selected-object count.
        count: u64,
        /// Whether nothing is selected (`count == 0`).
        empty: bool,
    },
    /// The active tool or its captured-point step changed.
    ToolStep {
        /// The tool's stable machine key (`draw.line`, `measure.distance`, …).
        tool: String,
        /// Points captured so far.
        step: u32,
    },
    /// A document label was installed (a real label change).
    DocumentOpened {
        /// The newly open document's label.
        label: String,
    },
    /// The document was closed (label became `null`).
    DocumentClosed,
    /// A new non-null host error appeared.
    ErrorRaised {
        /// The host's verbatim technical error string.
        error: String,
    },
    /// The open modal popup changed (`null` = none open).
    PopupChanged {
        /// The projected popup, or `null`.
        popup: Option<CadPopup>,
    },
    /// The in-process ring dropped events (bounded-buffer marker).
    EventsDropped {
        /// How many events were dropped since the previous marker.
        count: u32,
    },
}

/// One revisioned event on the wire.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CadEvent {
    /// Per-host monotonic revision, incremented by exactly 1 per emitted event.
    pub revision: u64,
    /// The event body; flattened so `type` sits beside `revision`.
    #[serde(flatten)]
    pub kind: CadEventKind,
}

/// A transport-side event consumer.
///
/// `cad-ui-core` is Tauri/platform-free, so the trait only says "deliver this
/// event". The desktop shell plugs in a `tauri::Emitter` implementation and the
/// wasm carrier plugs in its JS callback; neither type appears here.
pub trait EventSink: Send + Sync {
    /// Deliver one event. Implementations must not block or re-enter the bus.
    fn deliver(&self, event: &CadEvent);
}

/// The revision-assigning event bus plus the bounded in-process ring.
///
/// * [`Self::push`] assigns the next revision, stores the event in a
///   [`EVENT_RING_CAPACITY`]-entry ring and delivers it to every registered
///   [`EventSink`]. When the ring overflows it drops the oldest event and emits
///   one `events-dropped` marker (which is itself revisioned and delivered).
/// * [`Self::subscribers`] counts registered sinks.
/// * The bus is `Send + Sync` whenever its sinks are; the desktop shell keeps it
///   inside `Shared`.
#[derive(Default)]
pub struct EventBus {
    next_revision: u64,
    ring: VecDeque<CadEvent>,
    dropped: u32,
    sinks: Vec<Arc<dyn EventSink>>,
}

impl std::fmt::Debug for EventBus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EventBus")
            .field("next_revision", &self.next_revision)
            .field("ring_len", &self.ring.len())
            .field("dropped", &self.dropped)
            .field("subscribers", &self.sinks.len())
            .finish()
    }
}

impl EventBus {
    /// A bus with no events, no sinks and revision `0` (the first event is `1`).
    pub fn new() -> Self {
        Self::default()
    }

    /// The last assigned revision (`0` before the first event).
    pub fn revision(&self) -> u64 {
        self.next_revision
    }

    /// Number of registered transport sinks.
    pub fn subscribers(&self) -> usize {
        self.sinks.len()
    }

    /// Register a transport sink. Delivery is synchronous, in registration order.
    pub fn add_sink(&mut self, sink: Box<dyn EventSink>) {
        self.sinks.push(Arc::from(sink));
    }

    /// A snapshot of the registered sinks for **out-of-lock** delivery.
    ///
    /// Used by [`Self::push_quiet`] callers: the handles are cheap clones, and
    /// delivering through them after the owner lock is released keeps a transport
    /// `emit` off the lock path.
    pub fn sink_handles(&self) -> Vec<Arc<dyn EventSink>> {
        self.sinks.clone()
    }

    /// Remove every registered sink (the ring keeps its replay window).
    pub fn clear_sinks(&mut self) {
        self.sinks.clear();
    }

    /// The retained ring, oldest first.
    pub fn events(&self) -> impl DoubleEndedIterator<Item = &CadEvent> {
        self.ring.iter()
    }

    /// Number of retained events (at most [`EVENT_RING_CAPACITY`]).
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// Whether the ring is empty.
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    /// Assign a revision to `kind`, retain and deliver it.
    ///
    /// Returns every event this call emitted: the event itself, plus an
    /// `events-dropped` marker when the ring overflowed. Revisions strictly
    /// increase by 1 across the returned sequence.
    pub fn push(&mut self, kind: CadEventKind) -> Vec<CadEvent> {
        let mut emitted = vec![self.emit(kind)];
        if self.dropped > 0 {
            let count = std::mem::take(&mut self.dropped);
            // The marker is delivered but not retained: retaining it would drop
            // another data event merely to record a drop. A subscriber that reads
            // the ring later (instead of receiving the delivery) sees the gap
            // through the revision numbers.
            emitted.push(self.emit_unretained(CadEventKind::EventsDropped { count }));
        }
        emitted
    }

    /// Push one `status-changed` carrying `status`.
    pub fn push_status(&mut self, status: StatusFrame) -> Vec<CadEvent> {
        self.push(CadEventKind::StatusChanged {
            status: Box::new(status),
        })
    }

    /// Assign a revision and retain `kind`, but do **not** deliver it.
    ///
    /// Returns exactly what [`Self::push`] would return (the event plus any
    /// `events-dropped` marker). The caller must deliver the sequence after it
    /// releases its own lock, through [`Self::sink_handles`]. The desktop flush
    /// uses this so a transport `emit` never runs with `Shared` locked; ring and
    /// revision behaviour are identical.
    pub fn push_quiet(&mut self, kind: CadEventKind) -> Vec<CadEvent> {
        let mut emitted = vec![self.emit_quiet(kind, true)];
        if self.dropped > 0 {
            let count = std::mem::take(&mut self.dropped);
            emitted.push(self.emit_quiet(CadEventKind::EventsDropped { count }, false));
        }
        emitted
    }

    fn emit_quiet(&mut self, kind: CadEventKind, retain: bool) -> CadEvent {
        let event = self.assemble(kind);
        if retain {
            if self.ring.len() >= EVENT_RING_CAPACITY {
                self.ring.pop_front();
                self.dropped = self.dropped.saturating_add(1);
            }
            self.ring.push_back(event.clone());
        }
        event
    }

    fn emit(&mut self, kind: CadEventKind) -> CadEvent {
        let event = self.assemble(kind);
        if self.ring.len() >= EVENT_RING_CAPACITY {
            self.ring.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.ring.push_back(event.clone());
        self.deliver(&event);
        event
    }

    /// Deliver an event without retaining it (the drop marker).
    fn emit_unretained(&mut self, kind: CadEventKind) -> CadEvent {
        let event = self.assemble(kind);
        self.deliver(&event);
        event
    }

    fn assemble(&mut self, kind: CadEventKind) -> CadEvent {
        self.next_revision = self.next_revision.saturating_add(1);
        CadEvent {
            revision: self.next_revision,
            kind,
        }
    }

    fn deliver(&self, event: &CadEvent) {
        for sink in &self.sinks {
            sink.deliver(event);
        }
    }
}

/// Derive the semantic/lifecycle events from two status snapshots.
///
/// `prev: None` is the first snapshot of a host: no semantic event is invented
/// (the `status-changed` carries the full state). Revisions are **not** assigned
/// here; [`EventBus`] does that.
///
/// Every comparison is over public status fields only, so this never touches
/// session internals. A tool becoming idle is **not** an event: `tool-step` is
/// emitted only while an active tool's key or captured-point count changes.
pub fn derive_batch(prev: Option<&CadStatus>, next: &CadStatus) -> Vec<CadEventKind> {
    let Some(prev) = prev else {
        return Vec::new();
    };
    let mut events = command_events(prev, next);

    if prev.selection.count != next.selection.count || prev.selection.empty != next.selection.empty
    {
        events.push(CadEventKind::SelectionChanged {
            count: next.selection.count,
            empty: next.selection.empty,
        });
    }

    let prev_tool = prev
        .tool
        .as_ref()
        .map(|tool| (tool.key.as_str(), tool.points));
    let next_tool = next
        .tool
        .as_ref()
        .map(|tool| (tool.key.as_str(), tool.points));
    if let Some((key, points)) = next_tool {
        if prev_tool != next_tool {
            events.push(CadEventKind::ToolStep {
                tool: key.to_string(),
                step: points as u32,
            });
        }
    }

    match (
        prev.document_label.as_deref(),
        next.document_label.as_deref(),
    ) {
        (None, Some(label)) => events.push(CadEventKind::DocumentOpened {
            label: label.to_string(),
        }),
        (Some(old), Some(label)) if old != label => events.push(CadEventKind::DocumentOpened {
            label: label.to_string(),
        }),
        (Some(_), None) => events.push(CadEventKind::DocumentClosed),
        _ => {}
    }

    if let Some(error) = &next.error {
        if prev.error.as_deref() != Some(error.as_str()) {
            events.push(CadEventKind::ErrorRaised {
                error: error.clone(),
            });
        }
    }

    if prev.popup != next.popup {
        events.push(CadEventKind::PopupChanged {
            popup: next.popup.clone(),
        });
    }

    events
}

/// The `command-*` events of one flush.
///
/// The bounded log makes "appended" ambiguous; see the module docs for the
/// longest-suffix-of-`prev`/prefix-of-`next` rule and its documented limitation.
fn command_events(prev: &CadStatus, next: &CadStatus) -> Vec<CadEventKind> {
    let prev_log = prev.command_log.as_slice();
    let next_log = next.command_log.as_slice();
    let appended = appended_entries(prev_log, next_log);

    let mut events = Vec::new();
    for (offset, entry) in appended.iter().enumerate() {
        match entry.kind {
            CadCommandLogKind::Command => {
                if let Some(command) = non_empty(entry.command.as_deref()) {
                    events.push(CadEventKind::CommandAdmitted {
                        command: command.to_string(),
                    });
                }
            }
            CadCommandLogKind::Error => {
                // A refusal is attributable only to the selector it **directly
                // follows inside this batch**. An error whose predecessor is not
                // in the batch (it predates the flush) or is not a named
                // admission is never labelled; `status.error` and the log still
                // carry it.
                let selector = offset
                    .checked_sub(1)
                    .and_then(|index| appended.get(index))
                    .filter(|predecessor| predecessor.kind == CadCommandLogKind::Command)
                    .and_then(|predecessor| non_empty(predecessor.command.as_deref()));
                let code = non_empty(entry.code.as_deref());
                if let (Some(command), Some(code)) = (selector, code) {
                    events.push(CadEventKind::CommandRejected {
                        command: command.to_string(),
                        code: code.to_string(),
                    });
                }
            }
            // A measurement result is a typed answer, not a lifecycle event.
            CadCommandLogKind::Result => {}
        }
    }
    events
}

/// The value only when it is a real, non-empty token.
fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}

/// The entries appended to a bounded log.
///
/// Finds the longest suffix of `prev` that is also a prefix of `next`; everything
/// after that overlap in `next` is appended. `next` wins when there is no overlap
/// (see the module docs for the limitation).
fn appended_entries<'a>(
    prev: &[CadCommandLogEntry],
    next: &'a [CadCommandLogEntry],
) -> &'a [CadCommandLogEntry] {
    let max = prev.len().min(next.len());
    for len in (0..=max).rev() {
        if prev[prev.len() - len..] == next[..len] {
            return &next[len..];
        }
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::unsupported_status;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct RecordingSink {
        events: Arc<Mutex<Vec<CadEvent>>>,
    }

    impl EventSink for RecordingSink {
        fn deliver(&self, event: &CadEvent) {
            self.events.lock().expect("sink lock").push(event.clone());
        }
    }

    fn status() -> CadStatus {
        unsupported_status(None)
    }

    fn entry(
        sequence: u64,
        kind: CadCommandLogKind,
        command: Option<&str>,
        code: Option<&str>,
    ) -> CadCommandLogEntry {
        CadCommandLogEntry {
            sequence,
            kind,
            command: command.map(str::to_string),
            code: code.map(str::to_string),
            measurement: None,
            point: None,
        }
    }

    fn kinds(events: &[CadEventKind]) -> Vec<&'static str> {
        events
            .iter()
            .map(|event| match event {
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
            })
            .collect()
    }

    #[test]
    fn revisions_increase_by_one_per_emitted_event() {
        let mut bus = EventBus::new();
        assert_eq!(bus.revision(), 0);
        let first = bus.push(CadEventKind::CommandAdmitted {
            command: "LINE".to_string(),
        });
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].revision, 1);
        let second = bus.push_status(StatusFrame::new(status(), HostProfile::native_below()));
        assert_eq!(second[0].revision, 2);
        assert_eq!(bus.revision(), 2);
    }

    #[test]
    fn semantic_events_are_pushed_before_the_status_frame() {
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let mut bus = EventBus::new();
        bus.add_sink(Box::new(RecordingSink {
            events: Arc::clone(&recorded),
        }));
        assert_eq!(bus.subscribers(), 1);

        bus.push(CadEventKind::CommandAdmitted {
            command: "LINE".to_string(),
        });
        bus.push_status(StatusFrame::new(status(), HostProfile::native_below()));

        let events = recorded.lock().expect("sink lock");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].revision, 1);
        assert_eq!(events[1].revision, 2);
        assert!(matches!(
            events[0].kind,
            CadEventKind::CommandAdmitted { .. }
        ));
        assert!(matches!(events[1].kind, CadEventKind::StatusChanged { .. }));
    }

    #[test]
    fn the_ring_is_bounded_and_reports_dropped_events() {
        let mut bus = EventBus::new();
        for index in 0..EVENT_RING_CAPACITY {
            bus.push(CadEventKind::CommandAdmitted {
                command: format!("C{index}"),
            });
        }
        assert_eq!(bus.len(), EVENT_RING_CAPACITY);
        assert!(!bus.is_empty());

        let emitted = bus.push(CadEventKind::CommandAdmitted {
            command: "OVERFLOW".to_string(),
        });
        assert_eq!(
            kinds(&emitted.iter().map(|e| e.kind.clone()).collect::<Vec<_>>()),
            ["command-admitted", "events-dropped"]
        );
        assert_eq!(bus.len(), EVENT_RING_CAPACITY);
        let newest: Vec<_> = bus.events().rev().take(1).collect();
        assert!(matches!(
            newest[0].kind,
            CadEventKind::CommandAdmitted { .. }
        ));
        assert!(
            !bus.events()
                .any(|event| matches!(event.kind, CadEventKind::EventsDropped { .. })),
            "the marker is delivered, not retained"
        );
    }

    #[test]
    fn push_quiet_overflows_and_reports_dropped_events_like_push() {
        let mut bus = EventBus::new();
        for index in 0..EVENT_RING_CAPACITY {
            let emitted = bus.push_quiet(CadEventKind::CommandAdmitted {
                command: format!("C{index}"),
            });
            assert_eq!(emitted.len(), 1, "the ring is not full yet: no marker");
        }
        assert_eq!(bus.len(), EVENT_RING_CAPACITY);

        let emitted = bus.push_quiet(CadEventKind::CommandAdmitted {
            command: "OVERFLOW".to_string(),
        });
        assert_eq!(
            kinds(
                &emitted
                    .iter()
                    .map(|event| event.kind.clone())
                    .collect::<Vec<_>>()
            ),
            ["command-admitted", "events-dropped"]
        );
        assert_eq!(emitted[0].revision, EVENT_RING_CAPACITY as u64 + 1);
        assert_eq!(emitted[1].revision, emitted[0].revision + 1);
        assert_eq!(bus.len(), EVENT_RING_CAPACITY);
        let newest: Vec<_> = bus.events().rev().take(1).collect();
        assert!(matches!(
            newest[0].kind,
            CadEventKind::CommandAdmitted { .. }
        ));
        assert!(
            !bus.events()
                .any(|event| matches!(event.kind, CadEventKind::EventsDropped { .. })),
            "push_quiet keeps the same ring contract as push"
        );
    }

    #[test]
    fn no_event_is_derived_from_the_first_snapshot() {
        assert!(derive_batch(None, &status()).is_empty());
    }

    #[test]
    fn command_entries_become_admitted_and_batch_local_errors_become_rejected() {
        let prev = status();
        let mut next = status();
        next.command_log = vec![
            entry(1, CadCommandLogKind::Command, Some("ZOOM"), None),
            entry(
                2,
                CadCommandLogKind::Error,
                None,
                Some("zoom.argument_required"),
            ),
            entry(3, CadCommandLogKind::Command, Some("LINE"), None),
        ];
        let events = derive_batch(Some(&prev), &next);
        assert_eq!(
            kinds(&events),
            ["command-admitted", "command-rejected", "command-admitted"]
        );
        assert!(matches!(
            &events[1],
            CadEventKind::CommandRejected { command, code }
                if command == "ZOOM" && code == "zoom.argument_required"
        ));
    }

    #[test]
    fn a_refusal_without_a_selector_or_code_emits_no_rejection_event() {
        // No admitted selector at all: nothing to name, so no event is invented.
        let prev = status();
        let mut next = status();
        next.command_log = vec![entry(
            1,
            CadCommandLogKind::Error,
            None,
            Some("zoom.argument_required"),
        )];
        assert!(derive_batch(Some(&prev), &next).is_empty());

        // A selector with no stable code: the code is not invented either.
        let mut next = status();
        next.command_log = vec![
            entry(1, CadCommandLogKind::Command, Some("ZOOM"), None),
            entry(2, CadCommandLogKind::Error, None, None),
        ];
        assert_eq!(
            kinds(&derive_batch(Some(&prev), &next)),
            ["command-admitted"]
        );
    }

    #[test]
    fn an_error_is_not_attributed_to_a_command_from_an_earlier_batch() {
        // OPEN was admitted in a previous flush. A later internal failure appends
        // only an `Error` entry, so the batch-local rule refuses to label it as
        // an OPEN rejection; the error still rides `status.error`.
        let mut prev = status();
        prev.command_log = vec![entry(1, CadCommandLogKind::Command, Some("OPEN"), None)];
        let mut next = status();
        next.command_log = vec![
            entry(1, CadCommandLogKind::Command, Some("OPEN"), None),
            entry(
                2,
                CadCommandLogKind::Error,
                None,
                Some("open.internal_failure"),
            ),
        ];
        next.error = Some("open failed".to_string());

        let events = derive_batch(Some(&prev), &next);
        assert_eq!(kinds(&events), ["error-raised"]);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, CadEventKind::CommandRejected { .. })),
            "an error with no batch-local admission is never attributed: {events:?}"
        );
    }

    #[test]
    fn a_second_error_is_not_attributed_to_the_previous_command() {
        // The first error is attributable to ZOOM; the second directly follows
        // an error, so it has no selector and is skipped (never re-labelled).
        let prev = status();
        let mut next = status();
        next.command_log = vec![
            entry(1, CadCommandLogKind::Command, Some("ZOOM"), None),
            entry(2, CadCommandLogKind::Error, None, Some("zoom.first")),
            entry(3, CadCommandLogKind::Error, None, Some("zoom.second")),
        ];
        let events = derive_batch(Some(&prev), &next);
        assert_eq!(kinds(&events), ["command-admitted", "command-rejected"]);
        assert!(matches!(
            &events[1],
            CadEventKind::CommandRejected { command, code }
                if command == "ZOOM" && code == "zoom.first"
        ));
    }

    #[test]
    fn a_bounded_log_reports_only_the_appended_tail() {
        let capacity = cad_host::COMMAND_LOG_CAPACITY;
        let mut prev = status();
        prev.command_log = (0..capacity as u64)
            .map(|index| {
                entry(
                    index + 1,
                    CadCommandLogKind::Command,
                    Some(&format!("C{index}")),
                    None,
                )
            })
            .collect();
        let mut next = status();
        next.command_log = prev.command_log[1..].to_vec();
        next.command_log.push(entry(
            capacity as u64 + 1,
            CadCommandLogKind::Command,
            Some("NEWEST"),
            None,
        ));

        let events = derive_batch(Some(&prev), &next);
        assert_eq!(kinds(&events), ["command-admitted"]);
        assert!(matches!(
            &events[0],
            CadEventKind::CommandAdmitted { command } if command == "NEWEST"
        ));
    }

    #[test]
    fn a_cleared_log_reports_its_current_entries() {
        // A document replacement clears the host log; no overlap proves what was
        // there before, so the current entries are reported (documented limitation).
        let mut prev = status();
        prev.command_log = vec![entry(1, CadCommandLogKind::Command, Some("OLD"), None)];
        let mut next = status();
        next.command_log = vec![entry(2, CadCommandLogKind::Command, Some("NEW"), None)];

        let events = derive_batch(Some(&prev), &next);
        assert_eq!(kinds(&events), ["command-admitted"]);
    }

    #[test]
    #[allow(deprecated)] // the frozen `CadPopup::label` is required to build a popup at all
    fn selection_tool_document_error_and_popup_are_derived() {
        let mut prev = status();
        prev.document_label = Some("plan.dwg".to_string());
        prev.selection.count = 1;
        prev.selection.empty = false;
        prev.tool = None;
        prev.error = None;
        prev.popup = None;

        let mut next = status();
        next.document_label = None;
        next.selection.count = 2;
        next.selection.empty = false;
        next.tool = Some(cad_tool("draw.line", 2));
        next.error = Some("resource: missing SHX".to_string());
        next.popup = Some(CadPopup {
            kind: "about".to_string(),
            label: crate::popup::POPUP_LABEL.to_string(),
            modal: true,
        });

        let events = derive_batch(Some(&prev), &next);
        assert_eq!(
            kinds(&events),
            [
                "selection-changed",
                "tool-step",
                "document-closed",
                "error-raised",
                "popup-changed",
            ]
        );
        assert!(matches!(
            &events[1],
            CadEventKind::ToolStep { tool, step } if tool == "draw.line" && *step == 2
        ));
    }

    #[test]
    fn a_document_label_transition_emits_opened_then_closed() {
        let mut prev = status();
        prev.document_label = None;
        let mut next = status();
        next.document_label = Some("plan.dwg".to_string());
        let events = derive_batch(Some(&prev), &next);
        assert_eq!(kinds(&events), ["document-opened"]);
        assert!(matches!(
            &events[0],
            CadEventKind::DocumentOpened { label } if label == "plan.dwg"
        ));

        let closed = derive_batch(Some(&next), &prev);
        assert_eq!(kinds(&closed), ["document-closed"]);
    }

    #[test]
    fn an_unchanged_status_derives_nothing() {
        let status = status();
        assert!(derive_batch(Some(&status), &status).is_empty());
    }

    #[test]
    fn the_status_frame_serializes_exactly_like_cad_status() {
        let mut status = status();
        status.document_label = Some("plan.dwg".to_string());
        let frame = StatusFrame::new(status, HostProfile::dom_canvas()).with_canvas(Some(
            CanvasJson::new(r#"{"label":"plan.dwg","batches":2,"draws":0}"#.to_string()),
        ));
        let event = CadEvent {
            revision: 7,
            kind: CadEventKind::StatusChanged {
                status: Box::new(frame),
            },
        };
        let json = serde_json::to_string(&event).expect("serializes");
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(value["revision"], serde_json::json!(7));
        assert_eq!(value["type"], serde_json::json!("status-changed"));
        assert_eq!(
            value["status"]["documentLabel"],
            serde_json::json!("plan.dwg")
        );
        assert_eq!(
            value["status"]["hostProfile"],
            serde_json::to_value(HostProfile::dom_canvas()).expect("profile")
        );
        assert_eq!(
            value["status"]["canvasSession"],
            serde_json::json!({"label": "plan.dwg", "batches": 2, "draws": 0})
        );
        // The flattened status fields precede the profile, exactly like
        // `wire_status_json` (no `Value` key sorting).
        let label = json.find("\"documentLabel\"").expect("documentLabel");
        let profile = json.find("\"hostProfile\"").expect("hostProfile");
        let canvas = json.find("\"canvasSession\"").expect("canvasSession");
        assert!(label < profile && profile < canvas, "{json}");
    }

    #[test]
    fn a_status_frame_without_a_canvas_omits_the_key() {
        let event = CadEvent {
            revision: 1,
            kind: CadEventKind::StatusChanged {
                status: Box::new(StatusFrame::new(status(), HostProfile::native_below())),
            },
        };
        let json = serde_json::to_string(&event).expect("serializes");
        assert!(!json.contains("canvasSession"), "{json}");
    }

    #[test]
    fn event_types_are_kebab_case_and_the_envelope_is_frozen() {
        let cases = [
            (
                CadEventKind::CommandAdmitted {
                    command: "LINE".to_string(),
                },
                serde_json::json!({"revision": 1, "type": "command-admitted", "command": "LINE"}),
            ),
            (
                CadEventKind::CommandRejected {
                    command: "ZOOM".to_string(),
                    code: "zoom.argument_required".to_string(),
                },
                serde_json::json!({"revision": 1, "type": "command-rejected", "command": "ZOOM", "code": "zoom.argument_required"}),
            ),
            (
                CadEventKind::SelectionChanged {
                    count: 2,
                    empty: false,
                },
                serde_json::json!({"revision": 1, "type": "selection-changed", "count": 2, "empty": false}),
            ),
            (
                CadEventKind::ToolStep {
                    tool: "line".to_string(),
                    step: 2,
                },
                serde_json::json!({"revision": 1, "type": "tool-step", "tool": "line", "step": 2}),
            ),
            (
                CadEventKind::DocumentOpened {
                    label: "plan.dwg".to_string(),
                },
                serde_json::json!({"revision": 1, "type": "document-opened", "label": "plan.dwg"}),
            ),
            (
                CadEventKind::DocumentClosed,
                serde_json::json!({"revision": 1, "type": "document-closed"}),
            ),
            (
                CadEventKind::ErrorRaised {
                    error: "boom".to_string(),
                },
                serde_json::json!({"revision": 1, "type": "error-raised", "error": "boom"}),
            ),
            (
                CadEventKind::PopupChanged { popup: None },
                serde_json::json!({"revision": 1, "type": "popup-changed", "popup": null}),
            ),
            (
                CadEventKind::EventsDropped { count: 3 },
                serde_json::json!({"revision": 1, "type": "events-dropped", "count": 3}),
            ),
        ];
        for (kind, expected) in cases {
            let event = CadEvent { revision: 1, kind };
            assert_eq!(serde_json::to_value(&event).expect("serializes"), expected);
        }
    }

    fn cad_tool(key: &str, points: usize) -> super::super::status::CadTool {
        super::super::status::CadTool {
            key: key.to_string(),
            points,
            ready: false,
            open_ended: false,
            awaiting_text: false,
            text_prefill: None,
            params: super::super::status::CadToolParams {
                sides: None,
                fillet_radius: None,
                chamfer_distance_a: None,
                chamfer_distance_b: None,
                array_rows: None,
                array_columns: None,
                array_row_spacing: None,
                array_column_spacing: None,
                array_items: None,
                array_fill_angle: None,
                array_rotate_items: None,
                align_scale: None,
                hatch_pattern_type: None,
                hatch_pattern: None,
                hatch_spacing: None,
                hatch_angle: None,
                hatch_double: None,
                hatch_style: None,
                hatch_gradient_kind: None,
                hatch_gradient_angle: None,
                hatch_gradient_color_a: None,
                hatch_gradient_color_b: None,
                text_height: None,
                text_rotation: None,
                mtext_attachment: None,
            },
        }
    }
}
