//! Frontend wire protocol shared by the UI hosts.
//!
//! Two commands are exposed to the frontend:
//! - `probe_status` returns a [`ProbeStatus`] snapshot.
//! - `probe_input` accepts a [`ProbeInput`] and returns a [`ProbeInputAck`].
//!
//! The probe is **not** a CAD feature. Inputs only move the probe marker block
//! and are acknowledged; nothing about CAD entities, tools or measurement is
//! claimed here. An input is never silently dropped: a rejected input is still
//! acknowledged with `accepted = false` and a stable `reason`.
//!
//! These types are a frozen wire contract: the field names, serde renames and
//! omission rules are byte-stable so the JavaScript frontend keeps working.
//!
//! ## `cad_open_popup` acknowledgement addendum (2026-10-22, WP3)
//!
//! Popups are in-DOM overlays in the single UI tree, so `cad_open_popup` no
//! longer creates a window. Its acknowledgement keeps the frozen fields
//! `{ accepted, kind, popup_label, existing }` and **adds** one optional field:
//!
//! ```json
//! { "accepted": true, "kind": "about", "popup_label": "popup-1",
//!   "existing": false,
//!   "payload": { "kind": "about", "version": "0.1.2", "build": "release" } }
//! ```
//!
//! * `payload` is present only on a **fresh open** of a kind that carries one
//!   (`about`, `textEdit`); it is omitted (`skip_serializing_if`) for `options`
//!   and for `existing: true`, so a consumer that ignores it sees the old shape
//!   byte-for-byte.
//! * its own `kind` tag repeats the requested kind; the UI rejects a payload
//!   whose tag does not match instead of rendering guessed content.
//! * `textEdit`'s payload is host-derived from the current single selection
//!   (`{kind:"textEdit",text,multiline}`); the caller still supplies no content.
//!
//! `cad_status.popup` remains the open/closed source of truth
//! (`{kind,label,modal}` or `null`); the acknowledgement only carries the
//! content the newly rendered overlay needs.

use serde::{Deserialize, Serialize};

/// Marker/offset wrap period: offsets are always kept in `[0, 1)`.
const UNIT_PERIOD: f32 = 1.0;
/// Marker movement per wheel notch, as a fraction of the surface width.
const WHEEL_STEP: f32 = 0.02;

/// Wire encoding of [`ProbeInput::button`]: the DOM `MouseEvent.button` numbers,
/// so a frontend forwards the event value unchanged.
///
/// * [`BUTTON_PRIMARY`] (`0`) — primary/left.
/// * [`BUTTON_MIDDLE`] (`1`) — middle/wheel.
/// * [`BUTTON_SECONDARY`] (`2`) — secondary/right.
///
/// Any other value (including the DOM's `-1` "no button changed") is **refused
/// explicitly** by the session's mapping; it is never silently treated as the
/// primary button. A caller with no button information omits the field, which
/// means primary.
pub const BUTTON_PRIMARY: i32 = 0;
/// See [`BUTTON_PRIMARY`]: middle/wheel button.
pub const BUTTON_MIDDLE: i32 = 1;
/// See [`BUTTON_PRIMARY`]: secondary/right button.
pub const BUTTON_SECONDARY: i32 = 2;

/// The probe status returned by `probe_status`.
///
/// `backend`/`adapter` are `None` until the GPU probe has initialized.
/// `error` is present only while the probe is in an error state; it is never
/// cleared by faking a successful frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct ProbeStatus {
    /// Stable lowercase graphics backend (`"vulkan"`) once initialized.
    pub backend: Option<String>,
    /// Adapter/driver name (`"Quadro P620"`) once initialized.
    pub adapter: Option<String>,
    /// Last surface width in physical pixels (`0` while minimized/unmapped).
    pub width: u32,
    /// Last surface height in physical pixels (`0` while minimized/unmapped).
    pub height: u32,
    /// Number of frames actually presented.
    pub frames: u64,
    /// `true` once the probe has been stopped (destroy/exit) and released.
    pub stopped: bool,
    /// Current error, when the probe is not healthy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The kind of a frontend input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    /// Primary pointer pressed.
    PointerDown,
    /// Pointer moved (only moves the marker while the pointer is down).
    PointerMove,
    /// Pointer released; a terminal event.
    PointerUp,
    /// Pointer interaction cancelled; a terminal event.
    PointerCancel,
    /// Wheel/scroll delta.
    Wheel,
    /// Surface size / device-pixel-ratio update.
    Resize,
    /// Window lost focus; a terminal event.
    Blur,
    /// Pointer left the surface.
    ///
    /// A **transient** event, not a terminal one (see [`InputKind::is_terminal`]):
    /// it clears the cursor overlay for a surface the pointer is no longer over,
    /// but it ends no interaction — the pointer may re-enter and continue. The
    /// React host emits it only while no button is held (`pointerleave` with
    /// `active === null`), so it never arrives mid-drag; the session nonetheless
    /// leaves any in-flight press untouched (`session::plan_input`).
    PointerLeave,
}

impl InputKind {
    /// Whether this event ends an interaction.
    ///
    /// A **terminal** kind closes the input marker/sequence (`pointer_up`,
    /// `pointer_cancel`, `blur`). `pointer_leave` is deliberately **not**
    /// terminal: leaving the surface is transient and ends nothing, so the
    /// marker/sequence policy stays open across it.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::PointerUp | Self::PointerCancel | Self::Blur)
    }

    /// Stable lowercase name, matching the wire value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PointerDown => "pointer_down",
            Self::PointerMove => "pointer_move",
            Self::PointerUp => "pointer_up",
            Self::PointerCancel => "pointer_cancel",
            Self::Wheel => "wheel",
            Self::Resize => "resize",
            Self::Blur => "blur",
            Self::PointerLeave => "pointer_leave",
        }
    }
}

/// A frontend input event.
///
/// Every geometric field is optional; only the fields relevant to `kind` are
/// consumed. All present values must be finite.
#[derive(Debug, Clone, Deserialize)]
pub struct ProbeInput {
    /// Monotonic per-interaction sequence number; must strictly increase.
    pub sequence: u64,
    /// The event kind.
    pub kind: InputKind,
    /// Pointer X, in CSS/device pixels depending on the caller's convention.
    #[serde(default)]
    pub x: Option<f32>,
    /// Pointer Y. Carried for completeness; the probe marker is 1D horizontally.
    #[serde(default)]
    pub y: Option<f32>,
    /// Wheel delta X. Carried but not consumed (the marker is horizontal via `dy`).
    #[serde(default)]
    pub dx: Option<f32>,
    /// Wheel delta Y, used to advance the marker.
    #[serde(default)]
    pub dy: Option<f32>,
    /// Surface width, used to normalize `x` into a marker offset.
    #[serde(default)]
    pub width: Option<f32>,
    /// Surface height, carried for `resize`.
    #[serde(default)]
    pub height: Option<f32>,
    /// Device pixel ratio, carried for `resize`.
    #[serde(default)]
    pub dpr: Option<f32>,
    /// The pressed button of a pointer event, in the DOM `MouseEvent.button`
    /// numbers ([`BUTTON_PRIMARY`] `0` / [`BUTTON_MIDDLE`] `1` /
    /// [`BUTTON_SECONDARY`] `2`).
    ///
    /// The field is optional: an omitted `button` (or an explicit `null`) means
    /// the primary button, so a frontend that does not report buttons keeps its
    /// existing primary-button behavior. The session **prefers the button the
    /// press went down with** for `pointer_move`/`pointer_up`, so a caller may
    /// omit the field on a move.
    ///
    /// A value outside the three defined numbers — including the DOM's `-1`
    /// ("no button changed") — is refused with a stable reason by the session's
    /// mapping; it is never silently treated as primary.
    #[serde(default)]
    pub button: Option<i32>,
    /// Whether Shift was held on a pointer event (ADR 0014 B.4).
    ///
    /// Optional like [`ProbeInput::button`]: an omitted `shift` (or an explicit
    /// `null`) means "not held", so a frontend that does not report modifiers
    /// keeps its existing `Replace` behaviour. The React host sends it on
    /// `pointer_down`/`pointer_up` only; a move does not re-report a modifier
    /// change.
    ///
    /// A value of any other JSON type (for example the string `"true"` or the
    /// number `1`) is **refused** by the decoder, never coerced to `false` or
    /// `true`. `shift` is the only modifier on the wire this round; there is no
    /// general modifier bundle.
    #[serde(default)]
    pub shift: Option<bool>,
}

/// The acknowledgement for `probe_input`.
///
/// `accepted = false` always carries a `reason`; a rejection is never an empty
/// success.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProbeInputAck {
    /// Whether the input was accepted.
    pub accepted: bool,
    /// Echo of the input sequence number.
    pub sequence: u64,
    /// The marker offset after applying the input, in `[0, 1)`.
    pub marker_offset: f32,
    /// Whether this input was a terminal event.
    pub terminal: bool,
    /// Stable rejection reason, present only when `accepted = false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Tracks probe input: sequence ordering, the marker offset and interaction
/// state. Pure and GPU-free, so it is fully unit-testable.
#[derive(Debug, Clone)]
pub struct InputTracker {
    last_sequence: Option<u64>,
    active: bool,
    marker_offset: f32,
    dirty: bool,
    surface: Option<(f32, f32)>,
    dpr: f32,
    /// The most recent accepted `resize` rect (CAD viewport, logical px) for the
    /// host's layout tracker.
    rect: Option<crate::layout::CadMetrics>,
}

impl Default for InputTracker {
    fn default() -> Self {
        Self {
            last_sequence: None,
            active: false,
            marker_offset: 0.0,
            dirty: false,
            surface: None,
            dpr: 1.0,
            rect: None,
        }
    }
}

impl InputTracker {
    /// A fresh tracker with the marker at the left edge and DPR `1.0`.
    pub fn new() -> Self {
        Self::default()
    }

    /// The current marker offset, always in `[0, 1)`.
    pub fn marker_offset(&self) -> f32 {
        self.marker_offset
    }

    /// The last reported device pixel ratio.
    pub fn dpr(&self) -> f32 {
        self.dpr
    }

    /// The most recent accepted `resize` rect (CAD viewport in logical px), if
    /// any.
    pub fn last_rect(&self) -> Option<crate::layout::CadMetrics> {
        self.rect
    }

    /// Whether the pointer is currently down.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Consume the "input set the marker since the last frame" flag.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Apply a fixed animation step to the marker (bounded, wrapping).
    pub fn advance_animation(&mut self, step: f32) {
        self.marker_offset = wrap01(self.marker_offset + step);
    }

    /// Validate and apply an input, returning an explicit acknowledgement.
    pub fn apply(&mut self, input: &ProbeInput) -> ProbeInputAck {
        let terminal = input.kind.is_terminal();
        let reject = |tracker: &Self, reason: &str| ProbeInputAck {
            accepted: false,
            sequence: input.sequence,
            marker_offset: tracker.marker_offset,
            terminal,
            reason: Some(reason.to_string()),
        };

        if let Some(reason) = first_non_finite(input) {
            return reject(self, reason);
        }
        if let Some(last) = self.last_sequence {
            if input.sequence <= last {
                return reject(self, "out_of_order_sequence");
            }
        }

        self.last_sequence = Some(input.sequence);
        match input.kind {
            InputKind::PointerDown => {
                self.active = true;
                self.set_marker_from_pointer(input);
            }
            InputKind::PointerMove => {
                if self.active {
                    self.set_marker_from_pointer(input);
                }
            }
            InputKind::PointerUp => {
                self.set_marker_from_pointer(input);
                self.active = false;
            }
            InputKind::PointerCancel | InputKind::Blur => {
                self.active = false;
            }
            // A transient, non-terminal event: the pointer left the surface. It
            // moves no marker and ends no press (the contract has the UI send it
            // only with no button held), so it is a state-preserving no-op here.
            // The session resolves it to the overlay-only `ClearCursor` request.
            InputKind::PointerLeave => {}
            InputKind::Wheel => {
                if let Some(dy) = input.dy {
                    self.marker_offset = wrap01(self.marker_offset + dy * WHEEL_STEP);
                    self.dirty = true;
                }
            }
            InputKind::Resize => {
                if let (Some(width), Some(height)) = (input.width, input.height) {
                    if width > 0.0 && height > 0.0 {
                        self.surface = Some((width, height));
                        if let Some(dpr) = input.dpr {
                            if dpr > 0.0 {
                                self.dpr = dpr;
                            }
                        }
                        // Record the full logical rect for the host layout tracker.
                        self.rect = Some(crate::layout::CadMetrics {
                            x: input.x.unwrap_or(0.0),
                            y: input.y.unwrap_or(0.0),
                            width,
                            height,
                            dpr: self.dpr,
                            sequence: input.sequence,
                        });
                    }
                }
            }
        }

        ProbeInputAck {
            accepted: true,
            sequence: input.sequence,
            marker_offset: self.marker_offset,
            terminal,
            reason: None,
        }
    }

    fn set_marker_from_pointer(&mut self, input: &ProbeInput) {
        let width = input.width.or(self.surface.map(|(width, _)| width));
        if let (Some(x), Some(width)) = (input.x, width) {
            if width > 0.0 {
                self.marker_offset = clamp01(x / width);
                self.dirty = true;
            }
        }
    }
}

/// Return the reason for the first non-finite present parameter, if any.
fn first_non_finite(input: &ProbeInput) -> Option<&'static str> {
    let values = [
        input.x,
        input.y,
        input.dx,
        input.dy,
        input.width,
        input.height,
        input.dpr,
    ];
    if values.into_iter().flatten().any(|value| !value.is_finite()) {
        Some("non_finite_parameter")
    } else {
        None
    }
}

fn clamp01(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn wrap01(value: f32) -> f32 {
    if value.is_finite() {
        value.rem_euclid(UNIT_PERIOD)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::{Deserializer, IntoDeserializer, Visitor};

    fn input(sequence: u64, kind: InputKind) -> ProbeInput {
        ProbeInput {
            sequence,
            kind,
            x: None,
            y: None,
            dx: None,
            dy: None,
            width: None,
            height: None,
            dpr: None,
            button: None,
            shift: None,
        }
    }

    #[test]
    fn out_of_order_sequences_are_rejected_and_never_regress() {
        let mut tracker = InputTracker::new();
        let first = tracker.apply(&input(2, InputKind::PointerDown));
        assert!(first.accepted, "first input is accepted");

        let stale = tracker.apply(&input(1, InputKind::PointerDown));
        assert!(!stale.accepted);
        assert_eq!(stale.reason.as_deref(), Some("out_of_order_sequence"));

        let duplicate = tracker.apply(&input(2, InputKind::PointerMove));
        assert!(!duplicate.accepted, "an equal sequence is out of order");
        assert_eq!(duplicate.reason.as_deref(), Some("out_of_order_sequence"));

        let next = tracker.apply(&input(3, InputKind::PointerMove));
        assert!(next.accepted);
    }

    #[test]
    fn non_finite_parameters_are_rejected() {
        let mut tracker = InputTracker::new();
        let mut bad = input(1, InputKind::PointerDown);
        bad.x = Some(f32::NAN);
        bad.width = Some(100.0);
        let ack = tracker.apply(&bad);
        assert!(!ack.accepted);
        assert_eq!(ack.reason.as_deref(), Some("non_finite_parameter"));

        let mut bad_wheel = input(2, InputKind::Wheel);
        bad_wheel.dy = Some(f32::INFINITY);
        let ack = tracker.apply(&bad_wheel);
        assert!(!ack.accepted);
        assert_eq!(ack.reason.as_deref(), Some("non_finite_parameter"));

        // A rejection must not consume the sequence number.
        let ok = tracker.apply(&input(1, InputKind::PointerDown));
        assert!(ok.accepted);
    }

    #[test]
    fn terminal_events_report_terminal() {
        for (kind, expect_terminal) in [
            (InputKind::PointerUp, true),
            (InputKind::PointerCancel, true),
            (InputKind::Blur, true),
            (InputKind::PointerDown, false),
            (InputKind::PointerMove, false),
            (InputKind::Wheel, false),
            (InputKind::Resize, false),
            (InputKind::PointerLeave, false),
        ] {
            let mut tracker = InputTracker::new();
            let ack = tracker.apply(&input(1, kind));
            assert!(ack.accepted, "{kind:?} is accepted");
            assert_eq!(ack.terminal, expect_terminal, "{kind:?} terminal flag");
        }
    }

    #[test]
    fn pointer_down_and_move_set_the_marker_from_x_and_width() {
        let mut tracker = InputTracker::new();
        let mut down = input(1, InputKind::PointerDown);
        down.x = Some(25.0);
        down.width = Some(100.0);
        let ack = tracker.apply(&down);
        assert!(ack.accepted);
        assert!((ack.marker_offset - 0.25).abs() < 1e-6);
        assert!(tracker.take_dirty(), "input set the marker");
        assert!(!tracker.take_dirty(), "dirty flag is consumed once");

        // A move without width falls back to the last resize width.
        let mut resize = input(2, InputKind::Resize);
        resize.width = Some(200.0);
        resize.height = Some(100.0);
        assert!(tracker.apply(&resize).accepted);
        let mut moving = input(3, InputKind::PointerMove);
        moving.x = Some(100.0);
        assert!(tracker.apply(&moving).accepted);
        assert!((tracker.marker_offset() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn pointer_move_without_a_press_does_not_move_the_marker() {
        let mut tracker = InputTracker::new();
        let mut moving = input(1, InputKind::PointerMove);
        moving.x = Some(50.0);
        moving.width = Some(100.0);
        assert!(tracker.apply(&moving).accepted);
        assert!((tracker.marker_offset() - 0.0).abs() < 1e-6);
        assert!(!tracker.take_dirty());
    }

    #[test]
    fn wheel_and_animation_stay_in_the_unit_range() {
        let mut tracker = InputTracker::new();
        let mut wheel = input(1, InputKind::Wheel);
        wheel.dy = Some(1000.0);
        let ack = tracker.apply(&wheel);
        assert!(ack.accepted);
        assert!((0.0..1.0).contains(&ack.marker_offset));

        for _ in 0..1000 {
            tracker.advance_animation(0.03);
            assert!((0.0..1.0).contains(&tracker.marker_offset()));
        }
    }

    #[test]
    fn resize_ignores_zero_and_negative_dimensions() {
        let mut tracker = InputTracker::new();
        let mut resize = input(1, InputKind::Resize);
        resize.width = Some(0.0);
        resize.height = Some(100.0);
        assert!(tracker.apply(&resize).accepted);
        // Falls back to no stored surface → pointer move cannot set the marker.
        let mut moving = input(2, InputKind::PointerMove);
        moving.x = Some(50.0);
        assert!(tracker.apply(&moving).accepted);
        assert!((tracker.marker_offset() - 0.0).abs() < 1e-6);
    }

    // --- the optional `button` payload (frozen wire contract) ----------------

    /// One value of a decoded field map: just enough to drive `ProbeInput`'s
    /// derive the way a JSON object does (an absent key takes the serde default,
    /// a present one is decoded with its real type). Keeps this crate free of a
    /// JSON dependency while still testing the wire contract itself.
    enum Wire {
        Text(&'static str),
        U64(u64),
        I32(i32),
        Bool(bool),
        Null,
    }

    struct WireDeserializer(Wire);

    impl<'de> Deserializer<'de> for WireDeserializer {
        type Error = serde::de::value::Error;

        fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            match self.0 {
                Wire::Text(value) => value.into_deserializer().deserialize_any(visitor),
                Wire::U64(value) => value.into_deserializer().deserialize_any(visitor),
                Wire::I32(value) => value.into_deserializer().deserialize_any(visitor),
                Wire::Bool(value) => value.into_deserializer().deserialize_any(visitor),
                Wire::Null => ().into_deserializer().deserialize_any(visitor),
            }
        }

        fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            match self.0 {
                Wire::Null => visitor.visit_none(),
                other => visitor.visit_some(WireDeserializer(other)),
            }
        }

        fn deserialize_enum<V: Visitor<'de>>(
            self,
            name: &'static str,
            variants: &'static [&'static str],
            visitor: V,
        ) -> Result<V::Value, Self::Error> {
            match self.0 {
                Wire::Text(value) => value
                    .into_deserializer()
                    .deserialize_enum(name, variants, visitor),
                other => WireDeserializer(other).deserialize_any(visitor),
            }
        }

        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
            bytes byte_buf unit unit_struct newtype_struct seq tuple tuple_struct
            map struct identifier ignored_any
        }
    }

    impl<'de> IntoDeserializer<'de, serde::de::value::Error> for Wire {
        type Deserializer = WireDeserializer;

        fn into_deserializer(self) -> WireDeserializer {
            WireDeserializer(self)
        }
    }

    /// Decode one input exactly like the JSON IPC layer: a field map where an
    /// absent key is a missing JSON member.
    fn decode(fields: Vec<(&'static str, Wire)>) -> Result<ProbeInput, serde::de::value::Error> {
        ProbeInput::deserialize(serde::de::value::MapDeserializer::new(fields.into_iter()))
    }

    #[test]
    fn a_caller_that_omits_button_still_decodes_and_defaults_to_primary() {
        // A pre-existing JS caller sends no `button` member at all.
        let omitted = decode(vec![
            ("sequence", Wire::U64(1)),
            ("kind", Wire::Text("pointer_down")),
            ("x", Wire::I32(24)),
            ("y", Wire::I32(12)),
            ("width", Wire::I32(100)),
        ])
        .expect("an input without `button` decodes");
        assert_eq!(omitted.kind, InputKind::PointerDown);
        assert_eq!(omitted.button, None, "an absent button is no button report");
        assert_eq!(omitted.x, Some(24.0));

        // An explicit `null` behaves like the absent member.
        let null = decode(vec![
            ("sequence", Wire::U64(2)),
            ("kind", Wire::Text("pointer_up")),
            ("button", Wire::Null),
        ])
        .expect("a null button decodes");
        assert_eq!(null.button, None);
    }

    #[test]
    fn button_numbers_round_trip_through_the_input_payload() {
        for (wire, expected) in [
            (BUTTON_PRIMARY, 0),
            (BUTTON_MIDDLE, 1),
            (BUTTON_SECONDARY, 2),
        ] {
            let input = decode(vec![
                ("sequence", Wire::U64(1)),
                ("kind", Wire::Text("pointer_down")),
                ("button", Wire::I32(wire)),
            ])
            .unwrap_or_else(|error| panic!("button {wire} must decode: {error}"));
            assert_eq!(input.button, Some(expected));
        }
    }

    #[test]
    fn the_tracker_accepts_a_pointer_press_with_any_button_report() {
        // The tracker owns sequence/marker state, not button policy: it must
        // accept the event and keep its marker semantics unchanged. The session
        // is the layer that refuses an undefined button explicitly.
        for button in [None, Some(0), Some(1), Some(2), Some(7)] {
            let mut tracker = InputTracker::new();
            let mut down = input(1, InputKind::PointerDown);
            down.x = Some(50.0);
            down.width = Some(100.0);
            down.button = button;
            let ack = tracker.apply(&down);
            assert!(ack.accepted, "button {button:?} is accepted by the tracker");
            assert!((ack.marker_offset - 0.5).abs() < 1e-6);
            assert!(tracker.is_active());
            assert!(tracker.take_dirty());
        }
    }

    // --- the optional `shift` payload (frozen wire contract) -----------------

    #[test]
    fn a_caller_that_omits_shift_still_decodes_and_defaults_to_not_held() {
        // A pre-existing JS caller sends no `shift` member at all.
        let omitted = decode(vec![
            ("sequence", Wire::U64(1)),
            ("kind", Wire::Text("pointer_down")),
            ("x", Wire::I32(24)),
            ("y", Wire::I32(12)),
            ("button", Wire::I32(BUTTON_PRIMARY)),
        ])
        .expect("an input without `shift` decodes");
        assert_eq!(omitted.kind, InputKind::PointerDown);
        assert_eq!(omitted.shift, None, "an absent shift is no modifier report");

        // An explicit `null` behaves like the absent member.
        let null = decode(vec![
            ("sequence", Wire::U64(2)),
            ("kind", Wire::Text("pointer_up")),
            ("shift", Wire::Null),
        ])
        .expect("a null shift decodes");
        assert_eq!(null.shift, None);
    }

    #[test]
    fn shift_booleans_round_trip_and_any_non_boolean_is_refused() {
        for (wire, expected) in [(true, true), (false, false)] {
            let input = decode(vec![
                ("sequence", Wire::U64(1)),
                ("kind", Wire::Text("pointer_down")),
                ("shift", Wire::Bool(wire)),
            ])
            .unwrap_or_else(|error| panic!("shift {wire} must decode: {error}"));
            assert_eq!(input.shift, Some(expected));
        }

        // A non-boolean value is never coerced: `"true"`, `1` and `0` all fail.
        for wire in [Wire::Text("true"), Wire::I32(1), Wire::U64(0)] {
            assert!(
                decode(vec![
                    ("sequence", Wire::U64(1)),
                    ("kind", Wire::Text("pointer_down")),
                    ("shift", wire),
                ])
                .is_err(),
                "a non-boolean shift must be refused, never coerced"
            );
        }
    }

    #[test]
    fn the_tracker_accepts_a_shift_report_without_disturbing_its_marker() {
        let mut tracker = InputTracker::new();
        let mut down = input(1, InputKind::PointerDown);
        down.x = Some(25.0);
        down.width = Some(100.0);
        down.shift = Some(true);
        let ack = tracker.apply(&down);
        assert!(ack.accepted, "the tracker accepts a shift report");
        assert!((ack.marker_offset - 0.25).abs() < 1e-6);
    }

    // --- the transient `pointer_leave` event --------------------------------

    #[test]
    fn pointer_leave_round_trips_and_is_not_terminal() {
        // The wire string is the frozen contract the React lane sends.
        let input = decode(vec![
            ("sequence", Wire::U64(1)),
            ("kind", Wire::Text("pointer_leave")),
        ])
        .expect("pointer_leave decodes");
        assert_eq!(input.kind, InputKind::PointerLeave);
        assert_eq!(input.kind.as_str(), "pointer_leave");
        assert!(
            !input.kind.is_terminal(),
            "leaving the surface is transient: it ends no interaction"
        );
        // The stable lowercase spelling matches the wire value exactly.
        for kind in [
            InputKind::PointerDown,
            InputKind::PointerMove,
            InputKind::PointerUp,
            InputKind::PointerCancel,
            InputKind::Wheel,
            InputKind::Resize,
            InputKind::Blur,
            InputKind::PointerLeave,
        ] {
            let decoded = decode(vec![
                ("sequence", Wire::U64(1)),
                ("kind", Wire::Text(kind.as_str())),
            ])
            .unwrap_or_else(|error| panic!("{:?} must decode: {error}", kind.as_str()));
            assert_eq!(decoded.kind, kind, "{} round-trips", kind.as_str());
        }
    }

    #[test]
    fn the_tracker_accepts_a_pointer_leave_without_disturbing_its_state() {
        let mut tracker = InputTracker::new();
        let mut down = input(1, InputKind::PointerDown);
        down.x = Some(25.0);
        down.width = Some(100.0);
        assert!(tracker.apply(&down).accepted);
        assert!((tracker.marker_offset() - 0.25).abs() < 1e-6);
        assert!(tracker.is_active());
        assert!(tracker.take_dirty(), "the press set the marker");

        // A leave in the middle of the stream is accepted, advances the
        // sequence and is not terminal: it moves no marker, ends no press and
        // marks nothing dirty.
        let ack = tracker.apply(&input(2, InputKind::PointerLeave));
        assert!(ack.accepted, "the tracker accepts a pointer leave");
        assert!(!ack.terminal, "a leave is not a terminal event");
        assert!(
            (ack.marker_offset - 0.25).abs() < 1e-6,
            "the marker is untouched"
        );
        assert!(tracker.is_active(), "the press state is untouched");
        assert!(!tracker.take_dirty(), "a leave sets no dirty flag");

        // Sequence ordering is upheld: the consumed sequence is respected, so a
        // stale/duplicate event after it is refused and the stream continues.
        let stale = tracker.apply(&input(2, InputKind::PointerUp));
        assert!(!stale.accepted);
        assert_eq!(stale.reason.as_deref(), Some("out_of_order_sequence"));
        let next = tracker.apply(&input(3, InputKind::PointerUp));
        assert!(next.accepted);
        assert!(
            !tracker.is_active(),
            "the press still closes normally after the leave"
        );
    }
}
