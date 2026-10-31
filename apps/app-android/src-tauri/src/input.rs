//! Partition-local DOM input → presented-canvas coordinates.
//!
//! The React chrome reports every pointer/wheel position **relative to its
//! `.native-input` partition**, not to the window: `pointerPosition`
//! (`apps/desktop-ui/src/cad/metrics.ts`) subtracts the element's bounding-rect
//! origin before the position goes on the wire. The `native-below` carrier, in
//! contrast, presents into a `SurfaceView` that covers the **whole window**: the
//! session's canvas metrics are the surface's logical extent (`physical / scale`)
//! with `safe_insets = 0`, so canvas space is window space.
//!
//! The two spaces therefore differ by exactly the partition origin, and every
//! position-bearing input must be moved by it before the session sees it — a pick,
//! a hover/crosshair position and the wheel-zoom anchor are otherwise off by the
//! partition's `(x, y)`.
//!
//! The rule, implemented once here:
//!
//! * `pointer_down` / `pointer_move` / `pointer_up` / `wheel` carry a
//!   partition-local `x`/`y`, so the accepted layout rect's origin (the `x`/`y` of
//!   the `CadMetrics` the chrome last reported, in CSS px) is added to both;
//! * `resize` carries the partition **rect** (its `x`/`y` *is* the origin), not a
//!   position, so it is never moved — translating it would corrupt the very
//!   origin every later input is mapped with;
//! * `pointer_cancel` / `blur` / `pointer_leave` carry no position at all.
//!
//! No scale factor is involved: both spaces are CSS/logical pixels.
//! `SurfaceMetrics.safe_insets` is deliberately **not** used here — it is a
//! surface reservation, not a coordinate origin, and adding it would introduce a
//! scale error on top of the offset.
//!
//! # Known limitation (tracked follow-up, engine/viewport scope)
//!
//! The session's canvas is the **full window** while the chrome covers part of it.
//! `fit_camera` frames that whole canvas, so a fitted drawing can sit partly
//! **under the chrome** (ribbon / panels). Input mapping is exact thanks to this
//! module; only the fitted *framing* is off. Fixing it needs a viewport/inset
//! concept inside `cad-engine` (a canvas that is the surface minus the chrome, with
//! the matching projection), which is not a platform-side transform — out of this
//! lane's scope, tracked.
//!
//! # Why this module is target-independent
//!
//! It is pure and free of `jni`/`ndk`/GPU types, so it is compiled (and its unit
//! tests run) on the development host as well as on Android — the mapping is
//! exactly the kind of arithmetic that must not be validated only on a device.

/// Whether one input kind carries a position that is **partition-local**.
///
/// `kind` is the wire spelling of `cad_ui_core::protocol::InputKind`
/// (`InputKind::as_str`). That spelling is the protocol's wire value — the serde
/// representation — and `cad-ui-core`'s own round-trip test pins the two
/// together, so this is the same vocabulary the chrome sends.
pub fn is_partition_positioned(kind: &str) -> bool {
    matches!(
        kind,
        "pointer_down" | "pointer_move" | "pointer_up" | "wheel"
    )
}

/// Move one partition-local position into canvas space, when the kind has one.
///
/// `origin` is the `.native-input` rect origin in CSS px (`(0.0, 0.0)` when no
/// layout rect has been accepted yet, which is also the only case in which there
/// is no session to feed). An absent coordinate stays absent; a zero origin is a
/// no-op.
pub fn canvas_position(
    kind: &str,
    origin: (f32, f32),
    x: Option<f32>,
    y: Option<f32>,
) -> (Option<f32>, Option<f32>) {
    if !is_partition_positioned(kind) || origin == (0.0, 0.0) {
        return (x, y);
    }
    (x.map(|x| x + origin.0), y.map(|y| y + origin.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chrome layout where the CAD partition starts 96 CSS px below the top of
    /// the window (the ribbon sits above it) and spans the full width.
    const RECT_ORIGIN: (f32, f32) = (0.0, 96.0);

    #[test]
    fn a_pointer_is_moved_by_the_partition_origin() {
        // Partition-local (10, 20) on a rect at (0, 96) is window (10, 116).
        assert_eq!(
            canvas_position("pointer_down", RECT_ORIGIN, Some(10.0), Some(20.0)),
            (Some(10.0), Some(116.0))
        );
        // Every position-bearing pointer phase uses the same rule: the move and
        // the release must land where the press landed.
        assert_eq!(
            canvas_position("pointer_move", RECT_ORIGIN, Some(10.0), Some(20.0)),
            (Some(10.0), Some(116.0))
        );
        assert_eq!(
            canvas_position("pointer_up", RECT_ORIGIN, Some(10.0), Some(20.0)),
            (Some(10.0), Some(116.0))
        );
    }

    #[test]
    fn a_rect_at_the_window_origin_leaves_the_position_unchanged() {
        for kind in ["pointer_down", "pointer_move", "pointer_up", "wheel"] {
            assert_eq!(
                canvas_position(kind, (0.0, 0.0), Some(10.0), Some(20.0)),
                (Some(10.0), Some(20.0)),
                "{kind} must not move on a (0, 0) rect"
            );
        }
    }

    #[test]
    fn the_wheel_zoom_anchor_uses_the_same_partition_origin() {
        // `Camera::zoom_at` anchors the wheel zoom on the cursor position, so a
        // wheel event is mapped exactly like a pointer event.
        assert!(is_partition_positioned("wheel"));
        assert_eq!(
            canvas_position("wheel", RECT_ORIGIN, Some(10.0), Some(20.0)),
            (Some(10.0), Some(116.0))
        );
    }

    #[test]
    fn a_rect_carrying_or_positionless_input_is_never_translated() {
        // `resize` reports the partition rect itself: its `x`/`y` is the origin,
        // and moving it would corrupt the rect the next tick maps with.
        for kind in ["resize", "blur", "pointer_cancel", "pointer_leave"] {
            assert!(
                !is_partition_positioned(kind),
                "{kind} does not carry a partition-local position"
            );
            assert_eq!(
                canvas_position(kind, RECT_ORIGIN, Some(10.0), Some(20.0)),
                (Some(10.0), Some(20.0)),
                "{kind} must be passed through unchanged"
            );
        }
        // The resize payload in particular: `x`/`y` is the rect origin (0, 96).
        assert_eq!(
            canvas_position("resize", RECT_ORIGIN, Some(0.0), Some(96.0)),
            (Some(0.0), Some(96.0))
        );
    }

    #[test]
    fn an_absent_position_stays_absent() {
        // A coordinate-free event must not gain a fabricated `(0, 0)`.
        assert_eq!(
            canvas_position("pointer_leave", RECT_ORIGIN, None, None),
            (None, None)
        );
        assert_eq!(
            canvas_position("pointer_move", RECT_ORIGIN, None, None),
            (None, None)
        );
        assert_eq!(
            canvas_position("wheel", RECT_ORIGIN, Some(4.0), None),
            (Some(4.0), None)
        );
    }
}
