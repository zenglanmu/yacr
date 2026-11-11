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
//! Pattern type code `2` selects one of the **predefined** patterns by its
//! **1-based** index (`hatchPattern`). The implemented library is the three
//! hand-transcribed `.pat` patterns (`ANSI31`, `ANSI32`, `ANSI37`), stored as
//! AutoCAD `.pat` definition text and parsed by
//! `super::hatch_pattern::PatternLibrary` — the same loader, name resolution
//! and index resolution a user pattern file goes through. Spacing is the pattern
//! scale (`HPSCALE`), angle the hatch-level rotation and the double flag the
//! perpendicular second family; the definition rows come from the library, so
//! these are **our transcribed values, not vendor-verified**. An index outside
//! the library is refused **by name** — never clamped onto a neighbouring
//! pattern. A user `acad.pat`/`acadiso.pat` file is parsed and resolvable by
//! name, but this build has no pattern-name/file tool parameter (`hatchPattern`
//! is a closed 1-based index), so a user entry cannot yet be selected from the
//! command line (declared; nothing is approximated).
//!
//! Pattern type code `3` creates a two-colour **gradient** fill from its own
//! channel (`hatchGradientKind` Linear/Spherical, `hatchGradientAngle` in
//! **degrees**, `hatchGradientColorA`/`B` as ACI indices resolved through the
//! canonical palette). The entity records `Predefined` (`76=1`) because a
//! gradient is written as a solid fill plus the 450-470 block (an unverified
//! vendor convention). The angle is converted to radians exactly once here, while
//! the import path stores group 460 verbatim — the two conventions are pinned by
//! the create/export/reimport test. `reserved` and `shift` are fixed `0`, and the
//! pattern spacing/double/style are not consulted (a stale spacing never refuses
//! a gradient).
//!
//! The implementation is split for the 1000-line gate: this file is the command
//! entry point, [`super::hatch_boundary`] classifies the boundary / stitches it /
//! resolves the plane / writes the loops, [`super::hatch_stitch`] is the pure edge
//! chainer and [`super::hatch_parameters`] owns the typed channel plus the summary
//! diagnostic.

use super::*;

use super::hatch_boundary::{boundary_loops, boundary_shapes, resolve_plane, stitch_boundary};
use super::hatch_parameters::{
    gradient_parameters, hatch_creation_refusal, hatch_diagnostic, hatch_parameters, hatch_style,
    HatchFillMode,
};

/// Relabel a shared HATCH refusal for the GRADIENT command: the boundary rules
/// are the same code, so only the command name in the message changes. HATCH
/// itself passes its errors through unchanged.
impl HatchFillMode {
    fn relabel(self, error: CadError) -> CadError {
        if self != HatchFillMode::Gradient {
            return error;
        }
        match error {
            CadError::InvalidInput(message) => {
                CadError::InvalidInput(message.replace("HATCH", "GRADIENT"))
            }
            other => other,
        }
    }
}

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
    ///   table (refused by name, never clamped), a gradient kind outside
    ///   `{Linear, Spherical}`, a gradient angle that is not finite and a
    ///   gradient ACI index that is non-integral, outside `0..=255` or the ACI
    ///   `0` ByBlock placeholder — and Viewer mode (the command layer's mode
    ///   check).
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
        self.create_hatch_from_selection(session, command, refs, HatchFillMode::Session)
    }

    /// GRADIENT: fill the **currently selected** closed boundary entities with the
    /// session gradient parameters.
    ///
    /// This is the HATCH creation path with the fill forced to
    /// [`HatchFillMode::Gradient`]: the boundary classifier, stitcher, plane
    /// resolver and the `HatchGeometry::new` creation validator are the *same*
    /// functions, so GRADIENT has no second boundary implementation and the same
    /// HATCH refusal surface (relabelled `GRADIENT …`). The fill comes from the
    /// existing gradient channel (`hatchGradientKind`, `hatchGradientAngle` in
    /// degrees, `hatchGradientColorA`/`B`); the default session values are the
    /// two-colour linear ramp. The stale `hatchPatternType`/index/spacing are not
    /// consulted. Full contract: [`crate::command_id::gradient`].
    ///
    /// Additional v1 refusal: a boundary whose resolved plane normal is not `+Z`
    /// is `gradient.not_plus_z` (the gradient bake is defined in that plane).
    /// The command takes no payload (`gradient.payload`) and an empty selection is
    /// `gradient.selection`; both write nothing.
    pub(crate) fn create_gradient(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "gradient.payload: GRADIENT takes no payload; the current selection is the closed \
                 boundary input"
                    .into(),
            ));
        }
        let refs = session.selection.refs();
        if refs.is_empty() {
            return Err(CadError::InvalidInput(
                "gradient.selection: GRADIENT needs at least one selected closed boundary entity"
                    .into(),
            ));
        }
        self.create_hatch_from_selection(session, command, refs, HatchFillMode::Gradient)
    }

    /// The one boundary-to-hatch path shared by HATCH and GRADIENT.
    fn create_hatch_from_selection(
        &mut self,
        session: &SessionState,
        command: &Command,
        refs: &[SelectionRef],
        mode: HatchFillMode,
    ) -> CadResult<CommandOutcome> {
        let gradient = mode == HatchFillMode::Gradient;
        let parameters = if gradient {
            gradient_parameters(session)?
        } else {
            hatch_parameters(session)?
        };

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
                        "{} boundary must be a model-space entity; entity {} is referenced \
                         through a block instance or a sub-element",
                        if gradient { "GRADIENT" } else { "HATCH" },
                        reference.entity.0
                    )));
                }
            }

            let (code, operation, action) = if gradient {
                ("gradient", "GRADIENT", "gradient")
            } else {
                ("hatch", "HATCH", "hatch")
            };
            let (sources, _diagnostics) =
                Self::resolve_selection(drawing, document_id, refs, code, operation, action, false)
                    .map_err(|error| mode.relabel(error))?;
            let shapes = boundary_shapes(&sources).map_err(|error| mode.relabel(error))?;
            let stitched = stitch_boundary(&shapes).map_err(|error| mode.relabel(error))?;
            let plane = resolve_plane(&shapes, &stitched).map_err(|error| mode.relabel(error))?;
            if gradient && plane.normal.z < 1.0 - 1e-9 {
                return Err(CadError::InvalidInput(format!(
                    "gradient.not_plus_z: GRADIENT needs a boundary in the +Z plane; the resolved \
                     normal is ({}, {}, {})",
                    plane.normal.x, plane.normal.y, plane.normal.z
                )));
            }
            let loops =
                boundary_loops(&shapes, &stitched, &plane).map_err(|error| mode.relabel(error))?;
            let edge_count = stitched.edge_count();
            (loops, plane, edge_count)
        };
        let loop_count = loops.len();
        let style = hatch_style(parameters.style)?;
        let (fill, pattern_type) = parameters.fill()?;

        // The unique creation-path constructor: every boundary defect (A.3) is a
        // whole-command refusal here, before a transaction exists.
        let hatch = HatchGeometry::new(loops, plane, style, fill, Vec::new(), None, pattern_type)
            .map_err(|error| mode.relabel(hatch_creation_refusal(error)))?;
        let geometry = SemanticGeometry::Hatch(Box::new(hatch));

        let mut outcome = self.commit_new_entity(
            session,
            command,
            geometry,
            if gradient {
                "create gradient"
            } else {
                "create hatch"
            },
        )?;
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
