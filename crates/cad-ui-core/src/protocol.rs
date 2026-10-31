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
    /// Two-finger pinch: midpoint pan plus incremental zoom.
    ///
    /// **Continuous**, like [`InputKind::Wheel`]: there is no terminal pinch
    /// event. A pinch session starts when the second touch contact lands and ends
    /// when the frontend sends `pointer_cancel` (the UI sends one once, when it
    /// drops to fewer than two contacts).
    ///
    /// Fields (all in logical CSS pixels for `dx`/`dy`):
    /// * `x`,`y` — the midpoint of the two contacts (canvas-local), the zoom
    ///   anchor;
    /// * `dx`,`dy` — the midpoint movement since the **previous pinch event**;
    /// * `scale` — the **incremental** zoom multiplier since the previous event
    ///   (`1.0` = no zoom, `> 1.0` = zoom in).
    ///
    /// Coalescing rule (if a carrier ever merges two consecutive pinch events it
    /// must use this, and nothing may simply drop one): midpoint deltas **add**
    /// and scale multipliers **multiply** —
    /// `dx' = dx1 + dx2`, `dy' = dy1 + dy2`, `scale' = scale1 * scale2`. The
    /// product, never the sum: applying the merged event must zoom exactly as
    /// much as applying both in order (two `1.1` events are `1.21`, not `2.2`).
    Pinch,
}

/// The physical device a pointer event came from.
///
/// Wire spellings are `mouse`, `touch` and `pen` ([`Self::as_str`]). The field is
/// optional on [`ProbeInput`]; an absent `device` means [`Self::Mouse`], so every
/// pre-existing frontend keeps mouse semantics byte-for-byte. `pen` behaves like
/// `mouse` everywhere in the host (the contract's default rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeDevice {
    /// Mouse (or trackpad/trackpoint): the default when `device` is omitted.
    #[default]
    Mouse,
    /// Direct touch (finger): one-finger drag pans, a tap selects.
    Touch,
    /// Stylus/pen: mouse semantics.
    Pen,
}

impl ProbeDevice {
    /// Stable lowercase name, matching the wire value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mouse => "mouse",
            Self::Touch => "touch",
            Self::Pen => "pen",
        }
    }
}

impl InputKind {
    /// Whether this event ends an interaction.
    ///
    /// A **terminal** kind closes the input marker/sequence (`pointer_up`,
    /// `pointer_cancel`, `blur`). `pointer_leave` is deliberately **not**
    /// terminal: leaving the surface is transient and ends nothing, so the
    /// marker/sequence policy stays open across it. `pinch` is continuous and is
    /// not terminal either: a pinch session ends with `pointer_cancel`, never
    /// with a pinch event of its own.
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
            Self::Pinch => "pinch",
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
    /// Pinch scale: the **incremental** zoom multiplier since the previous pinch
    /// event (`1.0` = no zoom, `> 1.0` = zoom in). Consumed by
    /// [`InputKind::Pinch`] only; an absent value means `1.0` (no zoom).
    ///
    /// A present value must be finite and strictly positive; `0`, a negative
    /// value and a non-finite value are refused explicitly (never treated as
    /// "no zoom"), so a malformed pinch can never invert or destroy the view.
    #[serde(default)]
    pub scale: Option<f32>,
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
    /// The physical device of a pointer event (see [`ProbeDevice`]).
    ///
    /// Optional and additive: an omitted `device` (or an explicit `null`) means
    /// [`ProbeDevice::Mouse`], so an old frontend/host keeps its exact mouse
    /// behaviour. The wire values are `"mouse"`, `"touch"` and `"pen"`; any other
    /// string is **refused by the decoder** (an unknown enum variant), never
    /// silently mapped to a device.
    ///
    /// The value travels on every pointer phase (`pointer_down|pointer_move|
    /// pointer_up|pointer_cancel`); the host records it at press time and uses
    /// the recorded kind for the gesture, so a move that omits it cannot change
    /// the device mid-gesture. Other kinds ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<ProbeDevice>,
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
        // A pinch's incremental scale must be strictly positive. Zero and
        // negative multipliers are refused by name rather than treated as "no
        // zoom": they would silently discard a user gesture (or invert it).
        if input.kind == InputKind::Pinch {
            if let Some(scale) = input.scale {
                if scale <= 0.0 {
                    return reject(self, "invalid_pinch_scale");
                }
            }
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
            // A pinch is continuous, never terminal and never a press: it moves
            // the marker to the midpoint of the two contacts (like a pointer
            // position) and leaves the press state exactly as it found it. Its
            // `dx`/`dy` are logical-pixel deltas (not part of the 1-D marker),
            // and `scale` is the real zoom multiplier the session maps; the
            // tracker validates the value and applies no zoom of its own.
            InputKind::Pinch => {
                self.set_marker_from_pointer(input);
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
        input.scale,
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
mod tests;
