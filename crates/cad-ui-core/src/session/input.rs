//! DOM input planning: pointer/wheel classification and typed request mapping.
//!
//! Pure decision code: it maps the wire protocol onto the typed engine
//! interaction requests and owns the click/gesture classification, so the
//! session only applies the outcome. A click inside a draw/measurement capture
//! is a point capture; `Enter` confirms and `Esc` cancels through the command
//! funnel.

use cad_app::input::DRAG_THRESHOLD_LOGICAL_PX;
use cad_app::interaction::{
    InteractionRequest, Modifiers, PointInput, PointerButton, PointerId, PointerKind, PointerPhase,
    SelectionMode,
};

use super::diagnostics::cad_error_text;
#[allow(unused_imports)]
use super::CadSession;
use crate::protocol::{
    InputKind, ProbeDevice, ProbeInput, BUTTON_MIDDLE, BUTTON_PRIMARY, BUTTON_SECONDARY,
};

/// The single pointer stream identity (the DOM protocol carries no id).
pub(super) const POINTER_ID: PointerId = PointerId(1);

/// The retained press state of the single DOM pointer stream.
///
/// The wire protocol carries no pointer id, so the session keeps exactly one
/// press: the button it went down with and where, so a release can be
/// classified as a click (select) or as the end of a gesture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PointerPress {
    /// The button the press went down with.
    pub(super) button: PointerButton,
    /// Canvas-local logical position of the press.
    pub(super) down: [f64; 2],
    /// Whether an intervening move already crossed the drag threshold. A move
    /// that travels beyond the threshold and returns still counts as a drag, so
    /// a round trip never fabricates a click.
    pub(super) dragged: bool,
}

impl PointerPress {
    /// Whether releasing `released` at `up` is a click (select) rather than the
    /// end of a navigation gesture.
    ///
    /// Click-select is the **primary** button only: the middle button is the pan
    /// gesture (and the secondary button is reserved for a future context menu),
    /// so releasing any other button takes the plain pointer path and leaves the
    /// selection alone. The release must name the same button the press went
    /// down with, and the total displacement must stay under the drag threshold.
    fn is_click(&self, released: PointerButton, up: [f64; 2]) -> bool {
        self.button == PointerButton::Primary
            && released == PointerButton::Primary
            && !self.dragged
            && logical_distance(self.down, up) < DRAG_THRESHOLD_LOGICAL_PX
    }
}

/// Map one wire button report to the typed pointer button.
///
/// An omitted report (`None`) means the primary button, so an existing frontend
/// that does not send `button` keeps working. Any other number — including the
/// DOM's `-1` "no button changed" — is refused with a stable reason; it is never
/// silently treated as primary.
pub(super) fn pointer_button(report: Option<i32>) -> Result<PointerButton, String> {
    match report {
        None | Some(BUTTON_PRIMARY) => Ok(PointerButton::Primary),
        Some(BUTTON_MIDDLE) => Ok(PointerButton::Middle),
        Some(BUTTON_SECONDARY) => Ok(PointerButton::Secondary),
        Some(other) => Err(format!(
            "unsupported pointer button: {other} (expected 0, 1 or 2, or omit it for primary)"
        )),
    }
}

/// The finite canvas-local logical position of a pointer input.
fn pointer_logical(input: &ProbeInput) -> Result<[f64; 2], String> {
    match (input.x, input.y) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Ok([f64::from(x), f64::from(y)]),
        _ => Err("pointer position requires finite x and y".to_string()),
    }
}

/// Euclidean distance between two canvas-local logical positions.
fn logical_distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    (dx * dx + dy * dy).sqrt()
}

/// The decoded modifiers of one pointer input.
///
/// Only `shift` travels on the wire this round (ADR 0014 B.4): a plain click
/// selects with `Replace` while a Shift click adds with `Add`, the same rule
/// Shift+box drag already uses. An omitted `shift` (or an explicit `null`)
/// means "not held", so every other modifier stays at its default rather than
/// being inferred from the event stream, and a `pointer_move` never re-reports
/// a modifier change.
fn modifiers(input: &ProbeInput) -> Modifiers {
    Modifiers {
        shift: input.shift.unwrap_or(false),
        ..Modifiers::default()
    }
}

/// Map the optional wire device report to the typed pointer kind.
///
/// An omitted report (`None`) means [`PointerKind::Mouse`], so every existing
/// frontend/host keeps its exact mouse behaviour. The decoder has already
/// refused an unknown device string, so this mapping is total and never guesses.
pub(super) fn pointer_kind(device: Option<ProbeDevice>) -> PointerKind {
    match device.unwrap_or(ProbeDevice::Mouse) {
        ProbeDevice::Mouse => PointerKind::Mouse,
        ProbeDevice::Touch => PointerKind::Touch,
        ProbeDevice::Pen => PointerKind::Pen,
    }
}

/// The typed pointer request for one phase at the input's position.
fn pointer_request(
    input: &ProbeInput,
    revision: u64,
    phase: PointerPhase,
    button: Option<PointerButton>,
) -> Result<InteractionRequest, String> {
    Ok(InteractionRequest::Pointer {
        id: POINTER_ID,
        kind: pointer_kind(input.device),
        phase,
        button,
        position: canvas_point(input, revision, true)?,
        modifiers: modifiers(input),
    })
}

/// What a `CANCEL` selector means, given whether a draw/measure capture owns
/// input right now.
///
/// `CANCEL` is two affordances behind one selector: it ends an active tool, and
/// it is the host-owned import/open cancel. A capture must win, otherwise an
/// `Esc` inside a tool would also abort an unrelated in-flight import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CancelTarget {
    /// A capture owns input: cancel the tool and stop there.
    Tool,
    /// No capture: cancel the tool (idle-safe) *and* forward the host cancel.
    ToolAndHost,
}

/// Resolve [`CancelTarget`] from the capture state **sampled before the cancel
/// request is submitted**.
///
/// Ordering matters: submitting the typed cancel resets the tool to `Idle`
/// through the host, so sampling afterwards would always report "no capture"
/// and forward the host cancel unconditionally.
pub(super) fn cancel_target(capturing: bool) -> CancelTarget {
    if capturing {
        CancelTarget::Tool
    } else {
        CancelTarget::ToolAndHost
    }
}

/// Classify one DOM input against the retained press state.
///
/// Pure: it returns the request(s) to admit, in order, plus the press state that
/// must be committed once every request was admitted. It owns the click/gesture
/// decision so the session only applies the outcome.
///
/// * `pointer_down` starts a press and reports it as an ordinary pointer phase;
/// * `pointer_move` prefers the button the press went down with (the DOM reports
///   no button change on a move), and marks a press as dragged once the pointer
///   leaves the drag-threshold disc around the press position;
/// * `pointer_up` of the same primary button without an intervening drag is a
///   click: it emits the ordinary release **and** a typed point at the release
///   position — [`InteractionRequest::CapturePoint`] while a draw/measurement
///   capture owns the pointer (`capturing`), otherwise
///   [`InteractionRequest::SelectAt`]; its mode is `Add` when the release
///   carries Shift and `Replace` otherwise. Never both;
/// * a drag or a non-primary button never captures a point and never selects;
/// * `pointer_cancel`/`blur` end any press without capturing or selecting;
/// * every pointer phase carries the device kind from the optional wire
///   `device` field (absent ⇒ mouse; `pen` is mouse-like). The kind is not
///   interpreted here — the host arbitrates the gesture from it, so one touch
///   drag pans and a touch tap still selects;
/// * `pinch` maps to [`InteractionRequest::Pinch`] with the midpoint movement
///   and the incremental scale; it is continuous and ends no press;
/// * `pointer_leave` clears the cursor overlay (the same
///   [`InteractionRequest::ClearCursor`] the existing clear path uses) without
///   ending or disturbing any press — it is transient and non-terminal, so a
///   later release still resolves normally;
/// * a `resize` is refused (it is applied through
///   [`CadSession::set_canvas_metrics`]).
pub(super) fn plan_input(
    press: Option<PointerPress>,
    input: &ProbeInput,
    revision: u64,
    capturing: bool,
) -> Result<(Vec<InteractionRequest>, Option<PointerPress>), String> {
    match input.kind {
        InputKind::PointerDown => {
            let button = pointer_button(input.button)?;
            let down = pointer_logical(input)?;
            let request = pointer_request(input, revision, PointerPhase::Down, Some(button))?;
            Ok((
                vec![request],
                Some(PointerPress {
                    button,
                    down,
                    dragged: false,
                }),
            ))
        }
        InputKind::PointerMove => {
            // The reported value is validated even when the retained press owns
            // the button, so an undefined number is never silently ignored.
            let reported = pointer_button(input.button)?;
            let logical = pointer_logical(input)?;
            let hover = press.is_none();
            let (button, next) = match press {
                Some(press) => (
                    press.button,
                    Some(PointerPress {
                        dragged: press.dragged
                            || logical_distance(press.down, logical) >= DRAG_THRESHOLD_LOGICAL_PX,
                        ..press
                    }),
                ),
                // Without a press a move is a hover: not a gesture, and no press
                // state is invented.
                None => (reported, None),
            };
            let mut requests = vec![pointer_request(
                input,
                revision,
                PointerPhase::Move,
                Some(button),
            )?];
            if hover {
                // An idle move also asks the host what is under the cursor: the
                // typed cursor-position request tagged with the **current**
                // metrics revision, which the host refuses when stale and which
                // it resolves as the transient hover highlight. While a press is
                // active the gesture (pan/orbit/drag) owns the move, so no hover
                // is claimed (ADR-0012 §10 S2).
                requests.push(InteractionRequest::MoveCursor(canvas_point(
                    input, revision, true,
                )?));
            }
            Ok((requests, next))
        }
        InputKind::PointerUp => {
            let reported = pointer_button(input.button)?;
            let up = pointer_logical(input)?;
            let click = press.is_some_and(|press| press.is_click(reported, up));
            // The release always travels the ordinary pointer path, so the
            // host's gesture life is closed even if the selection is refused.
            let button = press.map_or(reported, |press| press.button);
            let mut requests = vec![pointer_request(
                input,
                revision,
                PointerPhase::Up,
                Some(button),
            )?];
            if click {
                // The click is either a point capture (a tool owns the pointer)
                // or a selection, never both. Shift+click adds to the current
                // selection (like Shift+box drag); a plain click replaces it.
                // Both carry the **current** metrics revision: a revision the
                // host no longer has is its explicit refusal, never a
                // fabricated pass.
                let point = canvas_point(input, revision, true)?;
                if capturing {
                    requests.push(InteractionRequest::CapturePoint(point));
                } else {
                    requests.push(InteractionRequest::SelectAt {
                        point,
                        mode: if input.shift.unwrap_or(false) {
                            SelectionMode::Add
                        } else {
                            SelectionMode::Replace
                        },
                    });
                }
            }
            Ok((requests, None))
        }
        // A cancel/blur is terminal: it ends any press and never selects.
        InputKind::PointerCancel | InputKind::Blur => {
            Ok((vec![interaction_request(input, revision)?], None))
        }
        // A pointer leave is transient and non-terminal: it clears the cursor
        // overlay and **leaves the press untouched** (`press` is returned
        // unchanged), so it never ends or wedges an in-flight gesture. It
        // carries no coordinates, so it demands no metrics revision it cannot
        // satisfy. The host classifies the resulting `ClearCursor` as
        // overlay-only, so leaving the surface schedules no full-state rebuild.
        InputKind::PointerLeave => Ok((vec![InteractionRequest::ClearCursor], press)),
        // A pinch is continuous navigation, like a wheel: it never ends the
        // retained press (the frontend sends `pointer_cancel` before it starts)
        // and never selects/captures.
        InputKind::Pinch => Ok((vec![pinch_request(input, revision)?], press)),
        InputKind::Wheel | InputKind::Resize => {
            Ok((vec![interaction_request(input, revision)?], press))
        }
    }
}

/// Map one continuous pinch event to its typed request.
///
/// Sign convention (documented so the wire cannot drift): `dx`/`dy` are the
/// **midpoint movement in DOM logical pixels, y down** — exactly the pointer /
/// multi-touch pan convention the host consumes with `pan_by_logical_delta`, so
/// they are passed through unchanged. Only the wheel's `deltaY` is sign-flipped
/// (`scroll_delta`), because the host turns *that* field into a zoom factor
/// rather than a pan. `scale` is already the host's zoom factor (`> 1.0` zooms
/// in), so it is passed through unchanged too.
///
/// `dx`/`dy` default to `0` and `scale` to `1.0` (no-op) when omitted; a present
/// value must be finite, and `scale` strictly positive — a zero/negative or
/// non-finite scale is refused by name, never silently applied or dropped.
fn pinch_request(input: &ProbeInput, revision: u64) -> Result<InteractionRequest, String> {
    let dx = input.dx.unwrap_or(0.0);
    let dy = input.dy.unwrap_or(0.0);
    if !dx.is_finite() || !dy.is_finite() {
        return Err("pinch midpoint delta must be finite".to_string());
    }
    let scale = input.scale.unwrap_or(1.0);
    if !scale.is_finite() || scale <= 0.0 {
        return Err("pinch scale must be finite and greater than zero".to_string());
    }
    Ok(InteractionRequest::Pinch {
        delta_logical: [f64::from(dx), f64::from(dy)],
        scale: f64::from(scale),
        position: canvas_point(input, revision, true)?,
    })
}

/// One **native** (carrier-observed) trackpad pinch sample.
///
/// This is the carrier-agnostic intake for a pinch that never appears as a DOM
/// event: a GTK `GtkGestureZoom` sample on Linux, a future toolkit pinch on
/// another native carrier. It is deliberately **not** a [`ProbeInput`] yet —
/// the carrier reports toolkit coordinates, and only the owner thread knows the
/// CAD partition rect and the CSS↔toolkit ratio at that instant.
///
/// * `x`,`y` — the pinch anchor in **widget-local GTK logical px** (the widget
///   that receives the gesture, i.e. the surface the DOM overlay covers).
/// * `scale` — the **incremental** multiplier for this sample (`> 1.0` zooms
///   in), the same convention as [`ProbeInput::scale`] for
///   [`InputKind::Pinch`]. Samples are never merged or summed on this path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NativePinch {
    /// Anchor X in widget-local GTK logical px.
    pub x: f64,
    /// Anchor Y in widget-local GTK logical px.
    pub y: f64,
    /// Incremental zoom multiplier for this sample.
    pub scale: f64,
}

/// The CAD partition's mapping from widget-local GTK logical px to CSS px.
///
/// `rect_x`/`rect_y` are the partition rect's origin in CSS px (the same rect
/// the layout tracker accepted), and `ratio` is `devicePixelRatio /
/// parent_scale` — the host's single CSS→GTK-logical ratio (see the Linux
/// host's `css_to_gtk`). A carrier that cannot produce this mapping must refuse
/// the sample rather than guess.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PinchViewport {
    /// Partition rect origin X, in CSS px.
    pub rect_x: f64,
    /// Partition rect origin Y, in CSS px.
    pub rect_y: f64,
    /// CSS → widget-logical ratio (`devicePixelRatio / parent_scale`).
    pub ratio: f64,
}

/// Refusal the native pinch intake returns when a coordinate is not finite.
pub const NATIVE_PINCH_NOT_FINITE: &str =
    "pinch.not_finite: native pinch needs finite x, y and scale";
/// Refusal for a non-positive incremental scale.
pub const NATIVE_PINCH_INVALID_SCALE: &str =
    "pinch.invalid_scale: native pinch scale must be greater than zero";
/// Refusal for a missing/degenerate CSS↔toolkit ratio.
pub const NATIVE_PINCH_INVALID_RATIO: &str = "pinch.invalid_ratio";

/// Convert one carrier-observed [`NativePinch`] into the session's wire input.
///
/// This is the single shared bridge between a toolkit pinch and the CAD pinch
/// semantics: the anchor is mapped from widget-local GTK logical px to
/// canvas-local CSS px with the caller's own metrics — `x_css = x / ratio -
/// rect_x` (and the same for `y`) — and the sample keeps its incremental
/// `scale`, so it ends in exactly the [`InteractionRequest::Pinch`] a DOM
/// two-contact pinch produces.
///
/// `sequence` is `0`: this path never touches [`crate::InputTracker`] (only the
/// wire admission path orders sequences), and [`CadSession::handle_inputs`]
/// ignores the field — it admits and applies inputs in slice order. Native
/// samples are appended after the tick's DOM inputs, so their relative order is
/// the queue order.
///
/// Refusals are stable, named strings and **never** a clamp: a broken sample is
/// surfaced, not silently coerced into a plausible one.
pub fn native_pinch_input(
    pinch: &NativePinch,
    viewport: PinchViewport,
) -> Result<ProbeInput, String> {
    if !pinch.x.is_finite() || !pinch.y.is_finite() || !pinch.scale.is_finite() {
        return Err(NATIVE_PINCH_NOT_FINITE.to_string());
    }
    if pinch.scale <= 0.0 {
        return Err(NATIVE_PINCH_INVALID_SCALE.to_string());
    }
    if !viewport.ratio.is_finite() || viewport.ratio <= 0.0 {
        return Err(NATIVE_PINCH_INVALID_RATIO.to_string());
    }
    Ok(ProbeInput {
        sequence: 0,
        kind: InputKind::Pinch,
        x: Some((pinch.x / viewport.ratio - viewport.rect_x) as f32),
        y: Some((pinch.y / viewport.ratio - viewport.rect_y) as f32),
        dx: None,
        dy: None,
        scale: Some(pinch.scale as f32),
        width: None,
        height: None,
        dpr: None,
        button: None,
        shift: None,
        device: None,
    })
}

/// Map one DOM input to a typed interaction request (the stateless mapping).
///
/// `resize` is deliberately refused: the caller applies it through
/// [`CadSession::set_canvas_metrics`], so an input that is not a CAD interaction
/// is never silently accepted.
pub(super) fn interaction_request(
    input: &ProbeInput,
    revision: u64,
) -> Result<InteractionRequest, String> {
    match input.kind {
        InputKind::PointerDown | InputKind::PointerMove | InputKind::PointerUp => {
            let phase = match input.kind {
                InputKind::PointerDown => PointerPhase::Down,
                InputKind::PointerMove => PointerPhase::Move,
                _ => PointerPhase::Up,
            };
            // The host owns the navigation decisions: a primary or middle drag
            // pans the 2D view (the primary button orbits a 3D one).
            pointer_request(input, revision, phase, Some(pointer_button(input.button)?))
        }
        // A cancel/blur is a terminal event and carries no coordinates: a zero
        // position is supplied only to satisfy the typed request; the host's
        // cancel path ignores it.
        InputKind::PointerCancel | InputKind::Blur => Ok(InteractionRequest::Pointer {
            id: POINTER_ID,
            kind: pointer_kind(input.device),
            phase: PointerPhase::Cancel,
            button: None,
            position: canvas_point(input, revision, false)?,
            modifiers: Modifiers::default(),
        }),
        InputKind::Wheel => {
            let dx = input.dx.unwrap_or(0.0);
            let dy = input.dy.unwrap_or(0.0);
            if !dx.is_finite() || !dy.is_finite() {
                return Err("wheel delta must be finite".to_string());
            }
            Ok(InteractionRequest::Scroll {
                delta_logical: scroll_delta(f64::from(dx), f64::from(dy)),
                position: canvas_point(input, revision, true)?,
                modifiers: Modifiers::default(),
            })
        }
        InputKind::Resize => Err("resize is applied through set_canvas_metrics".to_string()),
        // A pinch is continuous navigation: one typed `Pinch` carrying the
        // midpoint delta and the incremental zoom multiplier.
        InputKind::Pinch => pinch_request(input, revision),
        // A pointer leave is a non-terminal, coordinate-free event: it clears
        // the cursor overlay. Any coordinates it happens to carry are ignored
        // (exactly like cancel/blur), so it makes no metrics-revision demand.
        InputKind::PointerLeave => Ok(InteractionRequest::ClearCursor),
    }
}

/// A canvas-local position tagged with the current metrics revision.
///
/// `required` selects the pointer/wheel path (finite `x` and `y` mandatory) from
/// the cancel/blur path (coordinates absent, so the origin is used).
fn canvas_point(input: &ProbeInput, revision: u64, required: bool) -> Result<PointInput, String> {
    match (input.x, input.y) {
        (Some(x), Some(y)) => {
            if !x.is_finite() || !y.is_finite() {
                return Err("pointer position must be finite".to_string());
            }
            Ok(PointInput::CanvasLocal {
                logical_position: [f64::from(x), f64::from(y)],
                metrics_revision: revision,
            })
        }
        _ if required => Err("pointer position requires finite x and y".to_string()),
        _ => Ok(PointInput::CanvasLocal {
            logical_position: [0.0, 0.0],
            metrics_revision: revision,
        }),
    }
}

/// Invert the DOM wheel deltaY sign to the host convention.
///
/// The DOM reports a wheel-up as a negative `deltaY`; the host's scroll handler
/// zooms in on a positive `delta_logical[1]`, so the sign is flipped exactly
/// once (matching the web host's `normalized_scroll_delta`).
pub(super) fn scroll_delta(dx: f64, dy: f64) -> [f64; 2] {
    [dx, -dy]
}

impl CadSession {
    /// Apply one input, publishing the derived render state before returning.
    ///
    /// This is the per-input body; [`CadSession::handle_input`] keeps the
    /// one-publication-per-input behaviour, while [`CadSession::handle_inputs`]
    /// brackets a whole tick's inputs and publishes once.
    pub(super) fn handle_input_impl(&self, input: &ProbeInput) -> Result<(), String> {
        let capturing = self.capturing();
        let (requests, next_press) = plan_input(
            self.press.borrow().as_ref().copied(),
            input,
            self.metrics_revision(),
            capturing,
        )?;
        self.submit_interactions(requests)?;
        *self.press.borrow_mut() = next_press;
        Ok(())
    }

    /// Apply an ordered tick's worth of inputs under one render publication.
    ///
    /// Every input is still admitted and executed in order (wheel deltas are
    /// never summed here, no pointer phase is dropped and no command ordering
    /// changes); only the derived render publication is deferred, so N inputs in
    /// one tick produce one `sync_render`/`request_preparation` instead of N.
    ///
    /// The first refusal is returned after the batch is closed, so a rejected
    /// input never leaves the batch depth set. If every input was admitted but
    /// the single flush failed, that flush error is returned instead (the next
    /// tick's `sync_view` retries the publication).
    pub fn handle_inputs(&self, inputs: &[ProbeInput]) -> Result<(), String> {
        self.core.begin_render_batch();
        let mut first_error = None;
        for input in inputs {
            if let Err(error) = self.handle_input_impl(input) {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        let flush = self
            .core
            .end_render_batch()
            .map_err(|error| cad_error_text(&error));
        match first_error {
            Some(error) => Err(error),
            None => flush,
        }
    }
}
