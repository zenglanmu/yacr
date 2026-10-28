//! Input synthesis: the `pan`/`zoom_at` navigation the current chrome speaks,
//! translated into the session's own wire input.
//!
//! The current page and the X11 chrome call `pan(dx, dy)` / `zoom_at(factor, x,
//! y)`, while the real session speaks the wire protocol (`ProbeInput`). This module
//! is the honest translation — the **same** events a mouse produces, not a second
//! navigation path:
//!
//! * `pan` is the middle-button drag the desktop host pans with. The press is
//!   retained across calls (its anchor is the viewport centre, and each call moves
//!   to `anchor + Σdelta`), so the pan is exact and the session's own drag
//!   threshold cannot swallow the first few pixels. The gesture ends at the next
//!   non-pan interaction ([`Live::close_pan_inputs`]).
//! * `zoom_at` is one wheel event. The host's rule is
//!   `factor = 1 + delta_logical[1] × 0.0015`, and the wire's wheel deltaY is the
//!   DOM's (`scroll_delta` inverts the sign once), so the factor is inverted here.
//!   A factor the host would clamp is refused instead of applied approximately.
//!
//! Nothing here decides what the session does with an event: it builds the input
//! and hands it to `CadSession::handle_input`.

use cad_ui_core::protocol::{InputKind, ProbeInput, BUTTON_MIDDLE};

use super::{Failure, Live, STEP_INPUT, STEP_NAVIGATION};
use crate::live::POPUP_MODAL_OPEN;

/// The host's wheel-to-zoom constant (`cad-host`'s `Scroll` handling).
pub const WHEEL_FACTOR_PER_LOGICAL_PX: f64 = 0.0015;
/// The host clamps one wheel event to this factor range; outside it a
/// synthesized event would zoom by something other than the requested factor.
pub const WHEEL_MIN_FACTOR: f64 = 0.2;
pub const WHEEL_MAX_FACTOR: f64 = 5.0;

/// The retained state of the synthetic pointer stream.
#[derive(Debug, Default)]
pub struct InputState {
    /// The next `ProbeInput::sequence` (strictly increasing across the session).
    sequence: u64,
    /// The open synthetic pan press, if any.
    pan: Option<PanGesture>,
}

/// One synthetic middle-button drag: where it went down and how far the
/// pointer has travelled since.
#[derive(Debug, Clone, Copy)]
struct PanGesture {
    anchor: [f64; 2],
    offset: [f64; 2],
}

impl InputState {
    /// The next strictly increasing sequence number.
    pub fn next_sequence(&mut self) -> u64 {
        self.sequence = self.sequence.saturating_add(1);
        self.sequence
    }

    /// Whether a synthetic pan press is open.
    pub fn pan_open(&self) -> bool {
        self.pan.is_some()
    }

    /// The inputs one `pan(dx, dy)` call feeds the session.
    ///
    /// The first call of a gesture presses at `centre`; every call moves to
    /// `anchor + accumulated delta`, so the total pan is the sum of the deltas
    /// and no pixel is lost to the drag threshold.
    pub fn pan(&mut self, delta: [f64; 2], centre: [f64; 2]) -> Vec<ProbeInput> {
        let mut inputs = Vec::with_capacity(2);
        match &mut self.pan {
            None => {
                self.pan = Some(PanGesture {
                    anchor: centre,
                    offset: delta,
                });
                let sequence = self.next_sequence();
                inputs.push(pointer(
                    sequence,
                    InputKind::PointerDown,
                    centre,
                    Some(BUTTON_MIDDLE),
                ));
                let point = [centre[0] + delta[0], centre[1] + delta[1]];
                let sequence = self.next_sequence();
                inputs.push(pointer(sequence, InputKind::PointerMove, point, None));
            }
            Some(gesture) => {
                gesture.offset = [gesture.offset[0] + delta[0], gesture.offset[1] + delta[1]];
                let point = [
                    gesture.anchor[0] + gesture.offset[0],
                    gesture.anchor[1] + gesture.offset[1],
                ];
                let sequence = self.next_sequence();
                inputs.push(pointer(sequence, InputKind::PointerMove, point, None));
            }
        }
        inputs
    }

    /// The release that ends the synthetic pan gesture, if one is open.
    pub fn close_pan(&mut self) -> Option<ProbeInput> {
        let gesture = self.pan.take()?;
        let point = [
            gesture.anchor[0] + gesture.offset[0],
            gesture.anchor[1] + gesture.offset[1],
        ];
        let sequence = self.next_sequence();
        Some(pointer(
            sequence,
            InputKind::PointerUp,
            point,
            Some(BUTTON_MIDDLE),
        ))
    }

    /// The one wheel event that zooms by `factor` about `cursor`.
    pub fn zoom(&mut self, factor: f64, cursor: [f64; 2]) -> Result<ProbeInput, String> {
        if !(factor.is_finite() && factor > 0.0) {
            return Err(format!("zoom factor must be positive and finite: {factor}"));
        }
        if !(WHEEL_MIN_FACTOR..=WHEEL_MAX_FACTOR).contains(&factor) {
            return Err(format!(
                "zoom factor {factor} is outside the wheel range {WHEEL_MIN_FACTOR}..={WHEEL_MAX_FACTOR} \
                 this carrier can synthesize (a wheel event is clamped there, so a different \
                 factor would be applied)"
            ));
        }
        if !(cursor[0].is_finite() && cursor[1].is_finite()) {
            return Err(format!(
                "zoom cursor must be finite: ({}, {})",
                cursor[0], cursor[1]
            ));
        }
        // The host reads `1 + delta_logical[1] * 0.0015`; the wire's wheel
        // delta is the DOM's, whose sign `scroll_delta` inverts exactly once.
        let delta_logical = (factor - 1.0) / WHEEL_FACTOR_PER_LOGICAL_PX;
        let sequence = self.next_sequence();
        Ok(ProbeInput {
            sequence,
            kind: InputKind::Wheel,
            x: Some(cursor[0] as f32),
            y: Some(cursor[1] as f32),
            dx: None,
            dy: Some((-delta_logical) as f32),
            width: None,
            height: None,
            dpr: None,
            button: None,
            shift: None,
        })
    }
}

/// One pointer event at a canvas-local logical position.
pub fn pointer(
    sequence: u64,
    kind: InputKind,
    position: [f64; 2],
    button: Option<i32>,
) -> ProbeInput {
    ProbeInput {
        sequence,
        kind,
        x: Some(position[0] as f32),
        y: Some(position[1] as f32),
        dx: None,
        dy: None,
        width: None,
        height: None,
        dpr: None,
        button,
        shift: None,
    }
}

impl Live {
    /// Pan by a CSS-pixel screen delta, through the session's own pointer path.
    pub fn pan_by(&mut self, delta: [f64; 2]) -> Result<(), Failure> {
        if !(delta[0].is_finite() && delta[1].is_finite()) {
            return Err(Failure::new(
                STEP_NAVIGATION,
                format!("pan delta must be finite: ({}, {})", delta[0], delta[1]),
            ));
        }
        if delta[0] == 0.0 && delta[1] == 0.0 {
            return Ok(());
        }
        let inputs = self.input.pan(delta, self.metrics.centre());
        self.feed(inputs)
    }

    /// Zoom by `factor` about a canvas-local CSS-pixel cursor.
    pub fn zoom_about(&mut self, factor: f64, cursor: [f64; 2]) -> Result<(), Failure> {
        let input = self
            .input
            .zoom(factor, cursor)
            .map_err(|message| Failure::new(STEP_NAVIGATION, message))?;
        self.feed(vec![input])
    }

    /// End the synthetic pan gesture, if one is open (a no-op otherwise).
    pub fn close_pan_inputs(&mut self) {
        if let Some(input) = self.input.close_pan() {
            // A failed release is not swallowed: the session records it in its own
            // error state, which the next status reports.
            if let Err(message) = self.session.handle_input(&input) {
                self.note_failure(Failure::new(STEP_INPUT, message));
            }
        }
    }

    /// Feed one wire pointer input (the desktop `probe_input` channel).
    ///
    /// While a modal popup owns input every CAD interaction is refused by name
    /// (ADR-0021 D2) — the same rule the desktop host enforces on its own queue.
    pub fn probe(&mut self, input: &ProbeInput) -> Result<(), Failure> {
        if self.popups.modal_open() && input.kind != InputKind::Resize {
            return Err(Failure::new(STEP_INPUT, POPUP_MODAL_OPEN));
        }
        // A real pointer event ends a synthetic pan gesture: the two must never own
        // the single press slot at the same time.
        self.close_pan_inputs();
        self.session
            .handle_input(input)
            .map_err(|message| Failure::new(STEP_INPUT, message))
    }

    /// Run one CAD command line through the real router.
    pub fn command(&mut self, text: &str) -> Result<(), Failure> {
        self.close_pan_inputs();
        self.session
            .command(text)
            .map_err(|message| Failure::new(STEP_INPUT, message))
    }

    /// Feed several inputs, keeping the first failure.
    pub(super) fn feed(&mut self, inputs: Vec<ProbeInput>) -> Result<(), Failure> {
        for input in inputs {
            self.session
                .handle_input(&input)
                .map_err(|message| Failure::new(STEP_INPUT, message))?;
        }
        Ok(())
    }
}
