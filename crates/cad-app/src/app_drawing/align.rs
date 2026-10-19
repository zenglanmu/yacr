//! ALIGN edit command (ADR 0016 decision F).
//!
//! ALIGN moves and rotates the current selection from **two point pairs**, and
//! (only when the session `AlignScale` flag is true) applies a uniform scale. The
//! transform is applied **in place** (no copies) in one transaction with one undo
//! step, reusing the shared in-place transform shape
//! ([`Application::transform_selection`]) that ROTATE/SCALE use.
//!
//! Every input is validated before any write, so a refusal changes nothing (the
//! database transaction is all-or-nothing and no history is recorded).
//!
//! NOT IMPLEMENTED: three-pair 3D alignment, non-uniform scaling, and the
//! one-pair "move only" case (its effect is exactly MOVE).

use super::*;

use crate::draw_tool::{rotation_transform, scale_transform, ToolParameter, ToolValue};

/// The minimum XY length of a source/destination pair for it to define a
/// direction (and, for the source pair, a non-zero scale denominator).
///
/// A shorter pair has no well-defined angle and is refused rather than
/// fabricating a direction or dividing by zero.
const MIN_ALIGN_PAIR_EXTENT: f64 = 1e-12;

impl Application {
    /// ALIGN the selected entities from two point pairs (ADR 0016 decision F).
    ///
    /// The composed transform is `translate(dest1 − source1) ∘ rotate(θ, about
    /// source1) ∘ [uniform scale(s, about source1)]`, where `θ` is the angle from
    /// `(source2 − source1)` to `(dest2 − dest1)` and `s = |dest2 − dest1| /
    /// |source2 − source1|`. The scale factor is applied **only** when the session
    /// [`ToolParameter::AlignScale`] flag is true (AutoCAD's default answer to the
    /// scale prompt is No). Rotation and uniform scale are both about `source1`,
    /// so they commute and the order above is the documented
    /// translate∘rotate∘scale about the first source point. This is the flat
    /// `+Z` similarity subset: the angle and the pair magnitudes are measured in
    /// the XY work plane (for planar input that is the plain Euclidean distance),
    /// and the rotation is about `+Z`.
    ///
    /// The whole selection is transformed **in place** (no copies) in one
    /// transaction with one undo step.
    ///
    /// Refusals (nothing written): any non-finite point; a degenerate pair (a
    /// source or destination pair whose points coincide, so no direction exists —
    /// this also rules out the zero-length destination pair that would divide by
    /// zero when scaling); an empty selection; and geometry the transform surface
    /// cannot handle (the existing `Unsupported`).
    ///
    /// NOT IMPLEMENTED: three-pair 3D alignment, non-uniform scaling, and the
    /// one-pair "move only" case (equivalent to MOVE).
    pub(crate) fn align_entities(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Align {
            refs,
            source1,
            dest1,
            source2,
            dest2,
        } = &command.payload
        else {
            return Err(CadError::InvalidInput(
                "AlignEntities needs an Align { refs, source1, dest1, source2, dest2 } payload"
                    .into(),
            ));
        };
        if refs.is_empty() {
            return Err(CadError::InvalidInput(
                "AlignEntities needs at least one selected reference".into(),
            ));
        }
        if !is_finite_point(*source1)
            || !is_finite_point(*dest1)
            || !is_finite_point(*source2)
            || !is_finite_point(*dest2)
        {
            return Err(CadError::InvalidInput(
                "AlignEntities points must be finite".into(),
            ));
        }
        // A pair whose points coincide has no direction. The source pair also
        // supplies the scale denominator, so both pairs must be non-degenerate
        // (a zero-length destination pair would otherwise divide by zero).
        let source_pair = pair_vector(*source1, *source2);
        let dest_pair = pair_vector(*dest1, *dest2);
        if pair_length(source_pair) <= MIN_ALIGN_PAIR_EXTENT {
            return Err(CadError::InvalidInput(
                "ALIGN source pair needs two distinct points".into(),
            ));
        }
        if pair_length(dest_pair) <= MIN_ALIGN_PAIR_EXTENT {
            return Err(CadError::InvalidInput(
                "ALIGN destination pair needs two distinct points".into(),
            ));
        }
        let angle = (source_pair.x * dest_pair.y - source_pair.y * dest_pair.x)
            .atan2(source_pair.x * dest_pair.x + source_pair.y * dest_pair.y);
        if !angle.is_finite() {
            return Err(CadError::InvalidInput(
                "ALIGN source and destination pairs must define a finite angle".into(),
            ));
        }
        let align_scale = match session.tool_parameter(ToolParameter::AlignScale) {
            ToolValue::Flag(value) => value,
            // The store validates on write, so `AlignScale` is always a flag; the
            // number arm is unreachable and falls back to the documented default.
            ToolValue::Number(_) => false,
        };
        let rotate = rotation_transform(*source1, angle);
        let translate = Transform3::translation(Point3 {
            x: dest1.x - source1.x,
            y: dest1.y - source1.y,
            z: dest1.z - source1.z,
        });
        // `matrix_mul` applies the right-hand side first, so this composes
        // translate ∘ rotate ∘ scale (scale applied to the point first).
        let transform = if align_scale {
            let factor = pair_length(dest_pair) / pair_length(source_pair);
            let scale = scale_transform(*source1, factor);
            translate.matrix_mul(&rotate).matrix_mul(&scale)
        } else {
            translate.matrix_mul(&rotate)
        };
        self.transform_selection(
            command,
            refs,
            &transform,
            "align entities",
            "align",
            "AlignEntities",
            "ALIGN",
            // The diagnostic verb is ASCII: the ADR fence forbids new CJK literals
            // on the Rust side, and this action slot only reaches the pre-existing
            // sub-element/instance diagnostic templates.
            "align",
        )
    }
}

/// The XY direction vector from `a` to `b` (the `+Z` construction's plane).
fn pair_vector(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: b.x - a.x,
        y: b.y - a.y,
        z: b.z - a.z,
    }
}

/// The XY length of a pair vector (the rotation and scale are `+Z` constructions).
fn pair_length(v: Point3) -> f64 {
    v.x.hypot(v.y)
}
