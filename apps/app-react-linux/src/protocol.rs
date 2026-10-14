//! Frontend protocol for the React native Linux probe host.
//!
//! Two commands are exposed to the React frontend:
//! - `probe_status` returns a [`ProbeStatus`] snapshot.
//! - `probe_input` accepts a [`ProbeInput`] and returns a [`ProbeInputAck`].
//!
//! The probe is **not** a CAD feature. Inputs only move the probe marker block
//! and are acknowledged; nothing about CAD entities, tools or measurement is
//! claimed here. An input is never silently dropped: a rejected input is still
//! acknowledged with `accepted = false` and a stable `reason`.

use serde::{Deserialize, Serialize};

/// Marker/offset wrap period: offsets are always kept in `[0, 1)`.
const UNIT_PERIOD: f32 = 1.0;
/// Marker movement per wheel notch, as a fraction of the surface width.
const WHEEL_STEP: f32 = 0.02;

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
}

impl InputKind {
    /// Whether this event ends an interaction.
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
    /// The most recent accepted `resize` rect (CAD viewport, CSS px) for the
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

    /// The most recent accepted `resize` rect (CAD viewport in CSS px), if any.
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
                        // Record the full CSS rect for the host layout tracker.
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
}
