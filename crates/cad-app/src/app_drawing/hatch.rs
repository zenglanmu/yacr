//! HATCH creation from the **pre-selection** (ADR 0018 decision D, P6b).
//!
//! The boundary is the selection snapshot: `DrawToolKind::Hatch` captures no
//! points (the zero-point exact complement, like ARRAYRECT) and takes no
//! in-command entity picks, so the refs arrive in `CommandPayload::Hatch`. Each
//! accepted closed entity becomes **one loop** in the hatch plane's OCS 2D frame,
//! and the selected `LINE`/`ARC` edges are chained into one loop per closed chain:
//!
//! * `Circle` → an **exact** arc loop: four quadrant vertices with a
//!   `tan(pi/8)` bulge on every segment, i.e. four quarter-circle arcs. The
//!   ADR's cheap win stores a circle as two bulge-1 segments; this build uses
//!   four because the creation-path validator (`HatchGeometry::new`) requires at
//!   least three vertices and measures a chord ring, so a two-vertex loop is a
//!   degenerate chord set there. Four quarter arcs are still an exact circle and
//!   still export as an exact bulged polyline boundary.
//! * full `Ellipse` → a **sampled** chord loop (`ELLIPSE_SEGMENTS` chords per
//!   turn). A true ellipse is not a circular arc, so a bulge loop would be a
//!   lie; this loss is declared here and in the command docs.
//! * closed `Polyline` → its own points and bulges, mapped into the hatch plane.
//! * `Line`/`Arc` edges (any mix with the kinds above) → **stitched** into one or
//!   more closed loops by [`super::hatch_stitch`] and emitted as a closed
//!   **bulged** polyline: an `Arc` becomes one exact bulge segment
//!   (`bulge = tan(sweep / 4)`, matching the circle loop's quarter-arc
//!   `tan(pi/8)`) and a `Line` a bulge-0 segment — the arcs are never sampled.
//!   Endpoints chain within the absolute [`STITCH_TOLERANCE`] of 1e-6 world
//!   units; an edge may be consumed in either direction (mixed drawing
//!   orientations stitch, the traversal direction fixes the bulge sign); a node
//!   where more than two edges meet is refused (`hatch.boundary_branch`) instead
//!   of pairing guessed, and edges left outside every loop are refused
//!   (`hatch.boundary_open`) naming how many. A zero-length LINE or zero-sweep
//!   ARC is refused before stitching (`hatch.boundary_degenerate`).
//!
//! Every selected entity must lie in one plane with a single consistent normal
//! (circles/ellipses declare their normal; a polyline's is implied by its
//! points); anything else is refused **by name** before any write. The geometry
//! goes through `HatchGeometry::new`, so the ADR A.3 creation-path rejections
//! (self-intersecting, tangent/shared vertex, duplicate, partially overlapping,
//! unclosed, degenerate, non-finite) fail the whole command with no transaction.
//! **Declared limitation**: that validator works on the loop's **chords**, so for
//! a bulged loop (our circle loop included) a tangency that falls between the
//! sampled chords of two loops is not detected. An exact-circle contact whose
//! inscribed squares touch *is* refused (pinned by the tests); the residual gap
//! is a chord-sampling blind spot, not a silent acceptance of a crossing.
//!
//! The selected boundary entities are **retained**: the hatch is a derived
//! annotation over them (no associativity, no `DELOBJ` gating).
//!
//! The typed parameters are read from the session (the POLYGON precedent): the
//! fill kind, spacing, angle (degrees on the channel, radians in the domain), the
//! `HPDOUBLE` flag and the style. A user-defined pattern is synthesized from
//! angle/spacing/double by the same one-row definition the import path generates
//! for a `76=0`/`78=0` hatch, so creation and import share one recipe; the
//! generated line geometry remains **our implementation** (ADR U-1), not a
//! documented AutoCAD formula.
//!
//! Pattern type code `2` selects one of the **predefined** patterns from the
//! small hand-transcribed table in `cad_domain::hatch::patterns` (`ANSI31`,
//! `ANSI32`, `ANSI37`) by its **1-based** index (`hatchPattern`). Spacing is the
//! pattern scale (`HPSCALE`), angle the hatch-level rotation and the double flag
//! the perpendicular second family; the definition rows come from the table, so
//! these are **our transcribed values, not vendor-verified**, and `.pat` file
//! loading is NOT IMPLEMENTED. An index outside the table is refused **by name**
//! — never clamped onto a neighbouring pattern.
//!
//! The implementation is split for the 1000-line gate: this file is the command
//! entry point, [`super::hatch_boundary`] classifies the boundary / stitches it /
//! resolves the plane / writes the loops, [`super::hatch_stitch`] is the pure edge
//! chainer and [`super::hatch_parameters`] owns the typed channel plus the summary
//! diagnostic.

use super::*;

use super::hatch_boundary::{boundary_loops, boundary_shapes, resolve_plane, stitch_boundary};
use super::hatch_parameters::{
    hatch_creation_refusal, hatch_diagnostic, hatch_parameters, hatch_style,
};

impl Application {
    /// HATCH the pre-selected boundary entities into one entity (ADR 0018 D).
    ///
    /// One transaction / one undo step through the shared create funnel, on the
    /// active layer, with `type_key`/`class_name` = `AcDbHatch`. The selected
    /// boundary entities are **retained**: the hatch is a derived annotation over
    /// them, exactly like an AutoCAD "select objects" hatch (no associativity).
    ///
    /// Refusals (nothing written, no transaction):
    ///
    /// * an empty selection;
    /// * a selection entry referenced through a block instance or a sub-element
    ///   (the boundary must be a model-space entity);
    /// * any kind other than `Circle`, a full `Ellipse`, a closed `Polyline` or
    ///   `Line`/`Arc` edges (open polyline/spline/text/block/…);
    /// * an ellipse that is not a full turn (an ellipse arc is not a closed loop);
    /// * a `Line`/`Arc` edge that is degenerate (zero length, zero sweep — refused
    ///   by name before stitching), a stitched node where more than two edges meet
    ///   (`hatch.boundary_branch`) and stitched edges left in an open chain
    ///   (`hatch.boundary_open`);
    /// * a selection that is not coplanar with a single consistent normal;
    /// * every `HatchGeometry::new` creation-path rejection (self-intersecting,
    ///   tangent/shared vertex, duplicate, partially overlapping, unclosed,
    ///   degenerate, non-finite);
    /// * a missing active layer, an out-of-range typed parameter — including a
    ///   `hatchPattern` index that does not resolve in the implemented predefined
    ///   table (refused by name, never clamped) — and Viewer mode (the command
    ///   layer's mode check).
    pub(crate) fn create_hatch(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Hatch { refs } = &command.payload else {
            return Err(CadError::InvalidInput(
                "Hatch needs a Hatch { refs } payload".into(),
            ));
        };
        let parameters = hatch_parameters(session)?;

        let document_id = command.document;
        let (loops, plane, stitched_edges) = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let drawing = document.drawing.as_ref();

            for reference in refs {
                if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                    return Err(CadError::InvalidInput(format!(
                        "HATCH boundary must be a model-space entity; entity {} is referenced \
                         through a block instance or a sub-element",
                        reference.entity.0
                    )));
                }
            }

            let (sources, _diagnostics) = Self::resolve_selection(
                drawing,
                document_id,
                refs,
                "hatch",
                "HATCH",
                "hatch",
                false,
            )?;
            let shapes = boundary_shapes(&sources)?;
            let stitched = stitch_boundary(&shapes)?;
            let plane = resolve_plane(&shapes, &stitched)?;
            let loops = boundary_loops(&shapes, &stitched, &plane)?;
            let edge_count = stitched.edge_count();
            (loops, plane, edge_count)
        };
        let loop_count = loops.len();
        let style = hatch_style(parameters.style)?;
        let (fill, pattern_type) = parameters.fill()?;

        // The unique creation-path constructor: every boundary defect (A.3) is a
        // whole-command refusal here, before a transaction exists.
        let hatch = HatchGeometry::new(loops, plane, style, fill, Vec::new(), None, pattern_type)
            .map_err(hatch_creation_refusal)?;
        let geometry = SemanticGeometry::Hatch(Box::new(hatch));

        let mut outcome = self.commit_new_entity(session, command, geometry, "create hatch")?;
        if let Some(object) = outcome.objects.first().copied() {
            outcome.diagnostics.push(hatch_diagnostic(
                object,
                loop_count,
                stitched_edges,
                style,
                &parameters,
            ));
        }
        Ok(outcome)
    }
}
