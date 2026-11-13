//! Native (carrier-observed) trackpad pinch intake: the shared conversion that
//! turns a toolkit pinch sample into the same `Pinch` request the DOM
//! two-contact path produces.
//!
//! The sample is expressed in the carrier's own coordinates and the owner tick
//! supplies its layout metrics, so these tests pin the arithmetic (rect origin +
//! CSS↔toolkit ratio), the named refusals and the `sequence == 0` rule that says
//! this path never orders against `InputTracker`.

use super::super::input::{
    native_pinch_input, NativePinch, PinchViewport, NATIVE_PINCH_INVALID_RATIO,
    NATIVE_PINCH_INVALID_SCALE, NATIVE_PINCH_NOT_FINITE,
};
use crate::protocol::InputKind;

/// A widget-local pinch sample and the partition mapping it must be read with.
fn native_pinch(x: f64, y: f64, scale: f64) -> NativePinch {
    NativePinch { x, y, scale }
}

fn viewport(rect_x: f64, rect_y: f64, ratio: f64) -> PinchViewport {
    PinchViewport {
        rect_x,
        rect_y,
        ratio,
    }
}

/// The stable refusal reason for a sample that must be rejected.
fn native_pinch_refusal(pinch: &NativePinch, viewport: PinchViewport) -> String {
    match native_pinch_input(pinch, viewport) {
        Ok(input) => panic!("the sample must be refused, got {input:?}"),
        Err(reason) => reason,
    }
}

#[test]
fn native_pinch_maps_widget_logical_px_into_canvas_css_px() {
    // ratio 2.0 means one CSS px covers two widget-logical px (dpr 2, parent
    // scale 1). The partition starts at CSS (40, 12), so widget (200, 100) is
    // CSS (100, 50) in the widget and canvas-local (60, 38).
    let input = native_pinch_input(&native_pinch(200.0, 100.0, 1.25), viewport(40.0, 12.0, 2.0))
        .expect("a finite sample with a positive ratio is accepted");
    assert_eq!(input.kind, InputKind::Pinch);
    assert_eq!(input.x, Some(60.0));
    assert_eq!(input.y, Some(38.0));
    assert_eq!(input.scale, Some(1.25));
    // This path never orders against `InputTracker`: the session applies the
    // batch in slice order.
    assert_eq!(input.sequence, 0);
    // A native pinch anchors the zoom: it carries no midpoint movement.
    assert_eq!(input.dx, None);
    assert_eq!(input.dy, None);

    // A fractional ratio keeps the same formula (CSS = widget / ratio - origin).
    let scaled = native_pinch_input(&native_pinch(150.0, 75.0, 0.5), viewport(10.0, 5.0, 1.5))
        .expect("a fractional ratio is a legal mapping");
    assert_eq!(scaled.x, Some(90.0));
    assert_eq!(scaled.y, Some(45.0));

    // A partition at the widget origin is the identity under ratio 1.
    let identity = native_pinch_input(&native_pinch(7.0, 9.0, 1.0), viewport(0.0, 0.0, 1.0))
        .expect("ratio 1 with no origin offset is the identity");
    assert_eq!(identity.x, Some(7.0));
    assert_eq!(identity.y, Some(9.0));
}

#[test]
fn native_pinch_refuses_a_non_finite_sample_by_name() {
    for broken in [
        native_pinch(f64::NAN, 10.0, 1.1),
        native_pinch(10.0, f64::INFINITY, 1.1),
        native_pinch(10.0, 10.0, f64::NAN),
    ] {
        assert_eq!(
            native_pinch_refusal(&broken, viewport(0.0, 0.0, 1.0)),
            NATIVE_PINCH_NOT_FINITE,
            "a non-finite {broken:?} is refused, never clamped"
        );
    }
}

#[test]
fn native_pinch_refuses_a_non_positive_scale_by_name() {
    for broken in [0.0, -1.0, f64::NEG_INFINITY] {
        // `-inf` is non-finite first, so only the finite non-positive values
        // reach the scale refusal.
        let expected = if broken.is_finite() {
            NATIVE_PINCH_INVALID_SCALE
        } else {
            NATIVE_PINCH_NOT_FINITE
        };
        assert_eq!(
            native_pinch_refusal(&native_pinch(1.0, 1.0, broken), viewport(0.0, 0.0, 1.0)),
            expected,
            "scale {broken} is refused by its own name"
        );
    }
}

#[test]
fn native_pinch_refuses_a_degenerate_ratio_by_name() {
    for ratio in [0.0, -2.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            native_pinch_refusal(&native_pinch(1.0, 1.0, 1.1), viewport(0.0, 0.0, ratio)),
            NATIVE_PINCH_INVALID_RATIO,
            "ratio {ratio} cannot map CSS px, so the sample is refused"
        );
    }
}
