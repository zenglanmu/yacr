//! Wire-protocol unit tests (the frozen `probe_input` contract).
//!
//! Split out of `protocol.rs` so that file stays under the 1000-line ceiling;
//! these tests are still the module's own (`super::*` is the protocol module).

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
        scale: None,
        width: None,
        height: None,
        dpr: None,
        button: None,
        shift: None,
        device: None,
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
        (InputKind::Pinch, false),
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
    F64(f64),
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
            Wire::F64(value) => value.into_deserializer().deserialize_any(visitor),
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
        InputKind::Pinch,
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

// --- the optional `device` payload (frozen wire contract) ----------------

#[test]
fn a_caller_that_omits_device_still_decodes_and_means_mouse() {
    // A pre-existing JS caller sends no `device` member at all; the wire
    // decodes and the session maps the absent field to `mouse`.
    let omitted = decode(vec![
        ("sequence", Wire::U64(1)),
        ("kind", Wire::Text("pointer_down")),
        ("x", Wire::I32(24)),
        ("y", Wire::I32(12)),
    ])
    .expect("an input without `device` decodes");
    assert_eq!(
        omitted.device, None,
        "an absent device stays absent on the wire; `mouse` is the session default"
    );

    // An explicit `null` behaves like the absent member.
    let null = decode(vec![
        ("sequence", Wire::U64(2)),
        ("kind", Wire::Text("pointer_up")),
        ("device", Wire::Null),
    ])
    .expect("a null device decodes");
    assert_eq!(null.device, None);
}

#[test]
fn device_round_trips_the_three_wire_spellings_and_refuses_anything_else() {
    for (wire, expected) in [
        ("mouse", ProbeDevice::Mouse),
        ("touch", ProbeDevice::Touch),
        ("pen", ProbeDevice::Pen),
    ] {
        let input = decode(vec![
            ("sequence", Wire::U64(1)),
            ("kind", Wire::Text("pointer_down")),
            ("device", Wire::Text(wire)),
        ])
        .unwrap_or_else(|error| panic!("device {wire} must decode: {error}"));
        assert_eq!(input.device, Some(expected));
        assert_eq!(expected.as_str(), wire, "as_str is the wire spelling");
    }

    // An unknown device string is refused by the decoder — never silently
    // mapped to `mouse` (that would hide a frontend bug).
    for wire in ["trackpad", "stylus", "", "Mouse"] {
        assert!(
            decode(vec![
                ("sequence", Wire::U64(1)),
                ("kind", Wire::Text("pointer_down")),
                ("device", Wire::Text(wire)),
            ])
            .is_err(),
            "the unknown device {wire:?} must be refused"
        );
    }
}

// --- the continuous `pinch` event ----------------------------------------

#[test]
fn pinch_round_trips_with_midpoint_delta_and_scale() {
    let input = decode(vec![
        ("sequence", Wire::U64(1)),
        ("kind", Wire::Text("pinch")),
        ("x", Wire::I32(100)),
        ("y", Wire::I32(50)),
        ("dx", Wire::I32(-3)),
        ("dy", Wire::I32(4)),
        ("scale", Wire::F64(1.25)),
    ])
    .expect("pinch decodes");
    assert_eq!(input.kind, InputKind::Pinch);
    assert_eq!(input.kind.as_str(), "pinch");
    assert_eq!(input.x, Some(100.0));
    assert_eq!(input.y, Some(50.0));
    assert_eq!(input.dx, Some(-3.0));
    assert_eq!(input.dy, Some(4.0));
    assert_eq!(input.scale, Some(1.25));
    assert!(
        !input.kind.is_terminal(),
        "pinch is continuous: the session ends it with `pointer_cancel`"
    );
}

#[test]
fn pinch_scale_must_be_finite_and_strictly_positive() {
    for bad in [0.0f32, -1.0, f32::NEG_INFINITY, f32::NAN] {
        let mut pinch = input(1, InputKind::Pinch);
        pinch.x = Some(10.0);
        pinch.y = Some(10.0);
        pinch.scale = Some(bad);
        let ack = InputTracker::new().apply(&pinch);
        assert!(!ack.accepted, "scale {bad} must be refused");
        let reason = ack.reason.as_deref().unwrap_or_default();
        if bad.is_finite() {
            assert_eq!(reason, "invalid_pinch_scale");
        } else {
            assert_eq!(reason, "non_finite_parameter");
        }
    }

    // A refusal never consumes the sequence number: the corrected event is
    // still admissible.
    let mut tracker = InputTracker::new();
    let mut bad = input(1, InputKind::Pinch);
    bad.scale = Some(0.0);
    assert!(!tracker.apply(&bad).accepted);
    let mut good = input(1, InputKind::Pinch);
    good.scale = Some(1.0);
    assert!(tracker.apply(&good).accepted);
}

#[test]
fn a_pinch_is_continuous_and_leaves_the_press_state_alone() {
    let mut tracker = InputTracker::new();
    let mut down = input(1, InputKind::PointerDown);
    down.x = Some(25.0);
    down.width = Some(100.0);
    assert!(tracker.apply(&down).accepted);
    assert!(tracker.is_active());
    assert!(tracker.take_dirty());

    let mut pinch = input(2, InputKind::Pinch);
    pinch.x = Some(75.0);
    pinch.y = Some(40.0);
    pinch.width = Some(100.0);
    pinch.dx = Some(4.0);
    pinch.dy = Some(-2.0);
    pinch.scale = Some(1.5);
    let ack = tracker.apply(&pinch);
    assert!(ack.accepted);
    assert!(!ack.terminal, "a pinch event is not terminal");
    assert!(
        (ack.marker_offset - 0.75).abs() < 1e-6,
        "the marker follows the pinch midpoint"
    );
    assert!(tracker.is_active(), "a pinch ends no press");
    assert!(tracker.take_dirty(), "the midpoint moved the marker");
}
