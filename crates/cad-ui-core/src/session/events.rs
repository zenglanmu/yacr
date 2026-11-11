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
//! {"revision": 17, "type": "cursor-changed", "cursorWorld": [1, 2, 0], "frameCount": 42, "view": null}
//! ```
//!
//! [`CadEvent`] flattens [`CadEventKind`], which is internally tagged by `type`
//! (kebab-case), so the envelope is exactly `revision` + the variant's own keys.
//! The enum is `#[non_exhaustive]`: a consumer must ignore an unknown `type`
//! instead of failing on it, which is what lets a later event be additive.
//!
//! # `status-changed` is semantic; `cursor-changed` is the volatile cadence
//!
//! The first flush (subscribe / first owner tick) emits **one full
//! `status-changed`** carrying the complete status envelope ([`StatusFrame`]: the
//! flattened [`CadStatus`] plus `hostProfile` and the optional `canvasSession`,
//! byte-identical to `cad_status`). Every later flush emits a full status only
//! when something **other than** the volatile fields changed (see
//! [`same_status_ignoring_volatile`]); `cursor_world`, `frame_count` and the
//! orbit `view` basis ride a small [`CadEventKind::CursorChanged`] event instead.
//! That keeps the wire shape of `status-changed` unchanged while a purely visual
//! move no longer re-renders a whole shell.
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
use super::status::{CadPopup, CadStatus, CadViewOrientation};

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
    /// The two per-frame volatile fields changed (`cursorWorld` / `frameCount`).
    ///
    /// Emitted after the first full status and **instead of** a `status-changed`
    /// when nothing semantic changed. Consumers update their leaf stores (cursor
    /// readout / FPS sampler); it never pumps a frame and never replaces the shell
    /// status. Never produced by [`derive_batch`].
    CursorChanged {
        /// The pointer's world position, or `null` when it is off the surface.
        #[serde(rename = "cursorWorld")]
        cursor_world: Option<[f64; 3]>,
        /// The renderer's monotonic frame counter.
        #[serde(rename = "frameCount")]
        frame_count: u64,
        /// The current view orientation (see [`CadViewOrientation`]), or `null`.
        ///
        /// An orbit is a **volatile** view change: it must reach a consumer per
        /// frame without re-rendering the whole shell, so it rides this event
        /// instead of a full `status-changed`. `null` is the same honest "no
        /// authoritative camera" the status reports.
        view: Option<CadViewOrientation>,
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
    /// A vendor-decoding pass was armed for the newly opened document.
    ///
    /// The frontend fetches the binary records with `cad_vendor_records(token)`,
    /// decodes them in the WebView plugin, and posts the result with
    /// `cad_vendor_apply`. `entities` is the number of vendor records the host
    /// found (zero while the record extractor is not wired).
    VendorPending {
        /// Monotonic pass token; a newer document always gets a new token.
        token: u64,
        /// The document epoch the pass belongs to (echoed by the result frame).
        generation: u64,
        /// Vendor records available (bounded by the host).
        entities: u32,
    },
    /// A vendor pass settled (accepted or rejected) and is ready for the chrome.
    VendorSettled {
        /// The pass token this result belongs to.
        token: u64,
        /// Primitives the host actually wrote (zero until geometry insertion is
        /// wired; a non-zero claim is rejected before this event).
        applied: u32,
        /// Stable diagnostics, bounded by the host.
        diagnostics: Vec<String>,
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

/// Whether `previous` and `next` are identical apart from the per-frame volatile
/// fields (`frame_count`, `cursor_world`, `view`).
///
/// This is the Rust mirror of the frontend's `sameStatusIgnoringVolatile`: the
/// shell/proxy uses it to decide whether a full `status-changed` is still needed
/// after the first flush. A normalized clone is compared (no JSON round-trip), so
/// field additions participate automatically.
///
/// `view` is volatile **by design**: an orbit changes the camera basis every
/// frame, and that must reach the chrome through the `cursor-changed` frame
/// instead of a semantic status change.
pub fn same_status_ignoring_volatile(previous: &CadStatus, next: &CadStatus) -> bool {
    let mut previous = previous.clone();
    let mut next = next.clone();
    previous.frame_count = 0;
    next.frame_count = 0;
    previous.cursor_world = None;
    next.cursor_world = None;
    previous.view = None;
    next.view = None;
    previous == next
}

/// Whether any per-frame volatile field changed (`cursor_world`, `frame_count`
/// or the view orientation).
pub fn cursor_changed(previous: &CadStatus, next: &CadStatus) -> bool {
    previous.cursor_world != next.cursor_world
        || previous.frame_count != next.frame_count
        || previous.view != next.view
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
mod tests;
