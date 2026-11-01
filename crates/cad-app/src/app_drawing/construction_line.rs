//! `XLINE`/`RAY`: construction lines reduced to a finite stored segment.
//!
//! `cad-db` has **no infinite-line geometry**, so neither command can store the
//! thing AutoCAD calls an XLINE or a RAY. This module performs exactly the
//! reduction the DWG importer performs
//! (`cad_import_acadrust::construct_line::ray_semantics`): the construct line is
//! clipped to the drawing's XY model bounds (or a default 1000-unit box around
//! `base` when the drawing is empty) and stored as one
//! [`SemanticGeometry::Line`]. The entity keeps its source class name in
//! `DbObject::type_key` (`AcDbXline` / `AcDbRay`) and a `drawing.xline` /
//! `drawing.ray` diagnostic states the loss on every success, so nothing here
//! claims an infinite line was stored (export writes a LINE).
//!
//! Both commands create one entity on the session's active layer in model space,
//! in one transaction / one undo step, through the shared typed create funnel.
//! Every refusal happens **before** the transaction: a non-finite point
//! (`xline.invalid_point` / `ray.invalid_point`), a degenerate direction
//! (`xline.degenerate_direction` / `ray.degenerate_direction`) and a clip that
//! misses the box (`xline.outside_bounds` / `ray.outside_bounds`).
//!
//! A straight line crosses an axis-aligned box at most once, so the result is a
//! single segment with exact clipped endpoints — no sampled or stitched
//! approximation.

use super::*;

impl Application {
    /// `XLINE`: one finite segment clipped from the infinite construct line.
    pub(crate) fn create_xline(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        self.create_construction_line(session, command, false)
    }

    /// `RAY`: one finite segment clipped from the **positive half** of the
    /// construct line (from `base` along `+direction`).
    pub(crate) fn create_ray(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        self.create_construction_line(session, command, true)
    }

    /// The shared XLINE/RAY reduction and create path.
    fn create_construction_line(
        &mut self,
        session: &SessionState,
        command: &Command,
        is_ray: bool,
    ) -> CadResult<CommandOutcome> {
        let (base, through) = match (&command.payload, is_ray) {
            (CommandPayload::XLine { base, through }, false)
            | (CommandPayload::Ray { base, through }, true) => (*base, *through),
            _ => {
                return Err(CadError::InvalidInput(format!(
                    "{} needs a {} {{ base, through }} payload",
                    if is_ray { "CreateRay" } else { "CreateXLine" },
                    if is_ray { "Ray" } else { "XLine" }
                )));
            }
        };
        let reason = if is_ray { "ray" } else { "xline" };
        if !is_finite_point(base) || !is_finite_point(through) {
            return Err(CadError::InvalidInput(format!(
                "{reason}.invalid_point: the base and through points must be finite"
            )));
        }
        // The direction is `through − base`; it must be finite and long enough to
        // normalize, exactly like the importer's check.
        let direction = Point3 {
            x: through.x - base.x,
            y: through.y - base.y,
            z: through.z - base.z,
        };
        if !cad_geometry::is_finite(direction) || cad_geometry::length(direction) <= 1e-12 {
            return Err(CadError::InvalidInput(format!(
                "{reason}.degenerate_direction: the base and through points must differ"
            )));
        }
        let direction = cad_geometry::normalize(direction);
        // The clip box is the drawing's model bounds expanded by
        // `(distance * 0.05).max(1.0)`, or the importer's default 1000-unit box
        // around `base` when no geometry has defined bounds yet. Z is the flat
        // work plane, matching the importer.
        let bounds = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .bounds();
        let (min, max, default_box) = match bounds {
            Some((lo, hi)) => {
                let margin = (distance(lo, hi) * 0.05).max(1.0);
                (
                    Point3 {
                        x: lo.x - margin,
                        y: lo.y - margin,
                        z: 0.0,
                    },
                    Point3 {
                        x: hi.x + margin,
                        y: hi.y + margin,
                        z: 0.0,
                    },
                    false,
                )
            }
            None => (
                Point3 {
                    x: base.x - 1.0e3,
                    y: base.y - 1.0e3,
                    z: 0.0,
                },
                Point3 {
                    x: base.x + 1.0e3,
                    y: base.y + 1.0e3,
                    z: 0.0,
                },
                true,
            ),
        };
        let span = ((max.x - min.x).abs().max((max.y - min.y).abs()) * 4.0).max(1.0);
        let a = if is_ray {
            base
        } else {
            cad_geometry::add(base, cad_geometry::scale(direction, -span))
        };
        let b = cad_geometry::add(base, cad_geometry::scale(direction, span));
        let (start, end) =
            match cad_geometry::clip_segment_to_xy_rect(a, b, (min.x, min.y), (max.x, max.y)) {
                Some((start, end)) if cad_geometry::distance(start, end) > 1e-9 => (start, end),
                _ => {
                    return Err(CadError::InvalidInput(format!(
                        "{reason}.outside_bounds: the {reason} does not intersect the \
                         drawing bounds"
                    )));
                }
            };
        let type_key = if is_ray { "AcDbRay" } else { "AcDbXline" };
        let geometry = SemanticGeometry::Line { start, end };
        let mut outcome =
            self.commit_new_entity_typed(session, command, geometry, type_key, reason)?;
        outcome.diagnostics.push(Diagnostic {
            object: outcome.objects.first().copied(),
            code: format!("drawing.{reason}"),
            message: format!(
                "{}: stored as a finite segment ({}, {}, {}) - ({}, {}, {}) clipped to the {}; \
                 the construct line is infinite, so this is an approximation and export writes \
                 a LINE",
                type_key,
                start.x,
                start.y,
                start.z,
                end.x,
                end.y,
                end.z,
                if default_box {
                    "default 1000-unit box around the base point"
                } else {
                    "model bounds"
                }
            ),
        });
        Ok(outcome)
    }
}
