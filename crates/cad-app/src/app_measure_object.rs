//! `AREA OBJECT`: the exact area of one closed entity, from its own geometry.
//!
//! This is our **selection-based** form of AutoCAD's `AREA` → `Object`. AutoCAD
//! prompts for an interactive object pick; this build reads the session's current
//! selection instead (the same declared divergence `MATCHPROP` records — see
//! [`crate::CommandId::AreaObject`]). Exactly one selected entity is accepted: 0
//! and >1 are named refusals **before** any geometry is read, so there is no
//! partial result and no "first picked wins".
//!
//! ## Exact, never approximated
//!
//! * `Circle` → `π r²` from the stored radius (`[`std::f64::consts::PI`]`).
//! * `Ellipse` → `π a b` with `a = |major_axis|` and `b = a · ratio`, i.e. the
//!   stored `ratio` is honoured. A **partial** ellipse (an elliptical arc) is
//!   refused: reporting a whole ellipse's area for an arc would be a fabricated
//!   number.
//! * closed `Polyline` → the bulge-aware exact area **in the polyline's own plane**
//!   ([`cad_geometry::measure_bulged_ring_area`]). The existing XY-only
//!   [`cad_geometry::measure_polygon_area`] is not reused because it is neither
//!   bulge-aware nor plane-aware: a tilted polyline would be silently measured in
//!   its XY projection, and every arc segment would be treated as a chord.
//!
//! Nothing is sampled, tessellated or flattened, and no tolerance is applied to
//! the value itself. Every other geometry kind is an explicit
//! [`CadError::Unsupported`] refusal naming the kind.
//!
//! ## What the record carries
//!
//! [`cad_db::MeasurementAlgorithm::ObjectArea`] is a variant of its own (the value
//! did not come from a picked polygon ring), and the record follows one documented
//! convention per kind:
//!
//! * `Circle` / `Ellipse`: `inputs: []`, `plane: None` — a closed curve has no
//!   vertices, and a synthesised point list would be a fabrication;
//! * closed `Polyline`: `inputs` are the ring's **own stored vertices** in world
//!   coordinates (never synthesised picks) and `plane` is the ring's own plane
//!   (`cad_geometry::BulgedRingArea::plane`).
//!
//! `source` is [`GeometrySource::Analytic`] (the value comes from the entity's
//! analytic geometry, not from user points) and `precision` is
//! [`Precision::Analytic`].
//!
//! ## Nothing is written
//!
//! A measurement is not an edit: the command opens no transaction, publishes no
//! `ChangeSet`, records no undo entry and does not bump the revision. The only
//! state it touches is the session's "last measurement" record, the same field the
//! interactive measurement paths use. Work-only, like the other measurement
//! commands.

use super::*;
use cad_geometry::{measure_bulged_ring_area, AreaError};

/// How close an ellipse's stored `sweep` must be to one full turn to count as a
/// **closed** ellipse (radians).
///
/// The same rule the tessellator uses (`|sweep| >= TAU - 1e-9` is a full turn), so
/// an entity the display draws as a full ellipse is measured as one, and an
/// elliptical arc is refused instead of being reported as a whole ellipse.
pub const FULL_TURN_TOLERANCE_RADIANS: f64 = 1e-9;

impl Application {
    /// `AREA OBJECT`: measure one selected closed entity's exact area.
    ///
    /// Every refusal happens before anything is recorded, so a refused command
    /// leaves the session (selection, last measurement, drawing, history) untouched.
    pub(crate) fn area_object(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "area.object.invalid_payload: AREA OBJECT takes no payload; the selection is the \
                 input"
                    .into(),
            ));
        }
        let references = session.selection.refs();
        match references {
            [] => {
                return Err(CadError::InvalidInput(
                    "area.object.selection_empty: AREA OBJECT measures one closed object; select \
                     exactly one"
                        .into(),
                ));
            }
            [_] => {}
            many => {
                return Err(CadError::InvalidInput(format!(
                    "area.object.selection_multiple: AREA OBJECT measures one closed object, got \
                     {} selected; nothing was measured",
                    many.len()
                )));
            }
        }
        let reference = &references[0];
        if reference.document != command.document {
            return Err(CadError::InvalidInput(format!(
                "area.object.target_missing: the selected ref for entity {} belongs to another \
                 document",
                reference.entity.0
            )));
        }
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(CadError::Unsupported(
                "area.object.not_addressable: block-instance members and sub-elements have no \
                 addressable entity identity, so they have no stored area"
                    .into(),
            ));
        }

        let document_id = command.document;
        let (entity, units) = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            let entity = document
                .drawing
                .entity(reference.entity)
                .cloned()
                .ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "area.object.target_missing: entity {} does not exist in this drawing",
                        reference.entity.0
                    ))
                })?;
            (entity, document.units.clone())
        };

        let measured = self.measure_entity_area(&entity)?;
        let value = measured.value;
        let record = MeasurementRecord {
            algorithm: MeasurementAlgorithm::ObjectArea,
            inputs: measured.inputs,
            plane: measured.plane,
            value,
            units,
            // The value is the entity's analytic geometry, not a set of picked
            // points: `Analytic`, exactly, with no approximation flag.
            source: GeometrySource::Analytic,
            precision: Precision::Analytic,
        };
        session.set_last_measurement(record.clone());
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(entity.id.0)),
                code: "measure.object_area".into(),
                message: format!(
                    "AREA OBJECT: {} (entity {}) = {:.6}",
                    measured.kind, entity.id.0, value
                ),
            }],
            measurement: Some(record),
            point_report: None,
        })
    }

    /// Compute one entity's exact area, or refuse it by name.
    ///
    /// Split out so the supported-kind table and its refusals exist once (the
    /// contracts in `tests::area_object` drive it through the command).
    fn measure_entity_area(&self, entity: &cad_db::DbEntity) -> CadResult<MeasuredObjectArea> {
        match &entity.geometry {
            SemanticGeometry::Circle {
                center,
                normal,
                radius,
            } => {
                if !camera::is_finite_point(*center) || !camera::is_finite_point(*normal) {
                    return Err(degenerate(
                        entity,
                        "the circle's centre/normal is not finite",
                    ));
                }
                if !radius.is_finite() || *radius <= 0.0 {
                    return Err(degenerate(
                        entity,
                        &format!("the circle's radius {radius} is not a positive finite length"),
                    ));
                }
                let value = std::f64::consts::PI * radius * radius;
                MeasuredObjectArea::curve(entity, "circle", value)
            }
            SemanticGeometry::Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            } => {
                if !camera::is_finite_point(*center)
                    || !camera::is_finite_point(*normal)
                    || !camera::is_finite_point(*major_axis)
                {
                    return Err(degenerate(
                        entity,
                        "the ellipse's centre/normal/major axis is not finite",
                    ));
                }
                if !start.is_finite() || !sweep.is_finite() {
                    return Err(degenerate(
                        entity,
                        "the ellipse's start angle/sweep is not finite",
                    ));
                }
                // Only a full ellipse encloses an area. `TAU - 1e-9` is the
                // tessellator's own full-turn rule, so draw and measure agree.
                if sweep.abs() < std::f64::consts::TAU - FULL_TURN_TOLERANCE_RADIANS {
                    return Err(CadError::InvalidInput(format!(
                        "area.object.ellipse_arc: entity {} is an elliptical arc (sweep {sweep} rad \
                         of a full turn); AREA OBJECT needs a closed object",
                        entity.id.0
                    )));
                }
                let major = camera::length3(*major_axis);
                if !major.is_finite() || major <= 0.0 {
                    return Err(degenerate(
                        entity,
                        &format!("the ellipse's major axis length {major} is not positive"),
                    ));
                }
                if !ratio.is_finite() || *ratio <= 0.0 {
                    return Err(degenerate(
                        entity,
                        &format!(
                            "the ellipse's ratio {ratio} is not a positive finite number, so the \
                             minor semi-axis is undefined"
                        ),
                    ));
                }
                // The stored ratio is used **verbatim**, never swapped: an imported
                // ratio above 1 (a source that named the longer axis "minor") still
                // measures exactly pi * a * (a * ratio), so it is not refused and no
                // axis is silently exchanged.
                let minor = major * ratio;
                let value = std::f64::consts::PI * major * minor;
                MeasuredObjectArea::curve(entity, "ellipse", value)
            }
            SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
            } => {
                if !*closed {
                    return Err(CadError::InvalidInput(format!(
                        "area.object.polyline_open: entity {} is an open polyline; AREA OBJECT \
                         needs a closed object",
                        entity.id.0
                    )));
                }
                if points.iter().any(|p| !camera::is_finite_point(*p))
                    || bulges.iter().any(|b| !b.is_finite())
                {
                    return Err(degenerate(
                        entity,
                        "the polyline has a non-finite vertex or bulge",
                    ));
                }
                let ring = measure_bulged_ring_area(
                    points,
                    bulges,
                    self.measurement.tolerance.computation_world,
                )
                .map_err(|error| polyline_refusal(entity, error))?;
                Ok(MeasuredObjectArea {
                    kind: "closed polyline",
                    value: ring.area,
                    // The ring's own stored vertices, in world coordinates — never
                    // synthesised picks — and its own plane.
                    inputs: points.clone(),
                    plane: Some(ring.plane),
                })
            }
            other => Err(CadError::Unsupported(format!(
                "area.object.unsupported_kind: AREA OBJECT measures one closed CIRCLE, ELLIPSE or \
                 closed POLYLINE; entity {} is {}",
                entity.id.0,
                geometry_kind(other)
            ))),
        }
    }
}

/// One measured object area, before it is wrapped in a [`MeasurementRecord`].
struct MeasuredObjectArea {
    /// The kind named in the diagnostic (`circle`, `ellipse`, `closed polyline`).
    kind: &'static str,
    value: f64,
    /// The ring's own vertices; empty for a closed curve (no vertices exist).
    inputs: Vec<Point3>,
    /// The ring's own plane; `None` for a closed curve (nothing to project).
    plane: Option<WorkPlane>,
}

impl MeasuredObjectArea {
    /// A closed-curve result: no input vertices, no plane, and a finite value.
    fn curve(entity: &cad_db::DbEntity, kind: &'static str, value: f64) -> CadResult<Self> {
        if !value.is_finite() {
            return Err(degenerate(
                entity,
                "the computed area overflowed to a non-finite value",
            ));
        }
        if value <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "area.object.zero_area: entity {} has zero area",
                entity.id.0
            )));
        }
        Ok(Self {
            kind,
            value,
            inputs: Vec::new(),
            plane: None,
        })
    }
}

/// The stable refusal for a degenerate radius, axis or coordinate.
fn degenerate(entity: &cad_db::DbEntity, reason: &str) -> CadError {
    CadError::InvalidInput(format!(
        "area.object.degenerate: entity {} cannot be measured: {reason}",
        entity.id.0
    ))
}

/// Map one [`AreaError`] from the bulge-aware ring measurement to a named refusal.
///
/// The input's finiteness is checked before the call, so a remaining
/// [`AreaError::Degenerate`] is the zero-area case; it is reported as
/// `area.object.zero_area` rather than as a generic failure.
fn polyline_refusal(entity: &cad_db::DbEntity, error: AreaError) -> CadError {
    let id = entity.id.0;
    match error {
        AreaError::Degenerate => {
            CadError::InvalidInput(format!("area.object.zero_area: entity {id} has zero area"))
        }
        AreaError::TooFewPoints(count) => CadError::InvalidInput(format!(
            "area.object.polyline_plane: entity {id} encloses no ring ({count} distinct vertices)"
        )),
        AreaError::UndeterminedPlane => CadError::InvalidInput(format!(
            "area.object.polyline_plane: entity {id}'s plane cannot be derived from its vertices, \
             so the bulge sign frame is undetermined"
        )),
        AreaError::NonPlanar => CadError::InvalidInput(format!(
            "area.object.polyline_non_planar: entity {id} is not planar within the computation \
             tolerance"
        )),
        AreaError::SelfIntersecting => CadError::InvalidInput(format!(
            "area.object.self_intersecting: entity {id} is self-intersecting; area is undefined"
        )),
        AreaError::BulgesMismatch { points, bulges } => CadError::InvalidInput(format!(
            "area.object.degenerate: entity {id} is a corrupt polyline ({points} vertices, \
             {bulges} bulges; one bulge per vertex is required)"
        )),
    }
}

/// The display name of one geometry kind, for the unsupported-kind refusal.
fn geometry_kind(geometry: &SemanticGeometry) -> &'static str {
    match geometry {
        SemanticGeometry::Line { .. } => "a LINE",
        SemanticGeometry::Polyline { .. } => "a POLYLINE",
        SemanticGeometry::Circle { .. } => "a CIRCLE",
        SemanticGeometry::Arc { .. } => "an ARC (open, so it encloses no area)",
        SemanticGeometry::Ellipse { .. } => "an ELLIPSE",
        SemanticGeometry::Spline { .. } => "a SPLINE",
        SemanticGeometry::Point(_) => "a POINT",
        SemanticGeometry::Mesh(_) => "a MESH",
        SemanticGeometry::Insert { .. } => "a block INSERT",
        SemanticGeometry::Text { .. } => "TEXT",
        SemanticGeometry::Shape { .. } => "a SHAPE",
        SemanticGeometry::Opaque { .. } => "an OPAQUE entity",
        SemanticGeometry::Compound(_) => "a COMPOUND entity",
        SemanticGeometry::Dimension(_) => "a DIMENSION",
        SemanticGeometry::Hatch(_) => "a HATCH",
        SemanticGeometry::MultiLeader(_) => "a MULTILEADER",
        SemanticGeometry::Image { .. } => "an IMAGE",
        SemanticGeometry::Mask { .. } => "a MASK",
    }
}
