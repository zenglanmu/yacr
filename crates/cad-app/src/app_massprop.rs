//! `MASSPROP` and `MEASUREGEOM RADIUS`: the two AutoCAD 2026 inquiry reports.
//!
//! Split out of `app_measure_object.rs` to keep both modules inside the
//! code-size limit. The long-form contracts live in
//! [`crate::command_id::massprop`] and [`crate::command_id::measuregeom`]; this
//! module owns only the arithmetic and the diagnostic formatting. Neither path
//! writes the database, opens a transaction, records history or bumps a
//! revision.

use super::*;
use cad_geometry::{measure_bulged_ring_area, polyline_length, AreaError};

use crate::app_measure_object::geometry_kind;

// --- MASSPROP (2026-11-07) ------------------------------------------------
//
// The contract (input, supported kinds, every refusal, the report shape and
// every un-modelled AutoCAD property) is documented in
// [`crate::command_id::massprop`]; this module owns the arithmetic.

/// One measured region before it is formatted into diagnostics.
struct MassRegion {
    kind: &'static str,
    area: f64,
    /// `(value, approximate)`; the ellipse perimeter is the only approximate
    /// value in v1 (Ramanujan's second approximation).
    perimeter: (f64, bool),
    min: Point3,
    max: Point3,
    centroid: Point3,
    /// The region's own plane normal (unit) and one point on that plane.
    normal: Point3,
    plane_point: Point3,
}

impl Application {
    /// `MASSPROP`: read-only area/perimeter/bounding-box/centroid report of the
    /// selected closed planar regions.
    ///
    /// Every refusal happens before any diagnostic is produced, so a refused
    /// command leaves the session, drawing, history and selection untouched.
    pub(crate) fn mass_prop(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "massprop.invalid_payload: MASSPROP takes no payload; the selection is the input"
                    .into(),
            ));
        }
        let references = session.selection.refs().to_vec();
        if references.is_empty() {
            return Err(CadError::InvalidInput(
                "massprop.selection: MASSPROP measures the selected closed planar regions; \
                 select at least one"
                    .into(),
            ));
        }
        let mut regions = Vec::with_capacity(references.len());
        for reference in &references {
            if reference.document != command.document {
                return Err(CadError::InvalidInput(format!(
                    "massprop.target_missing: the selected ref for entity {} belongs to another \
                     document",
                    reference.entity.0
                )));
            }
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(CadError::Unsupported(
                    "massprop.not_addressable: block-instance members and sub-elements have no \
                     addressable entity identity, so they have no mass properties"
                        .into(),
                ));
            }
            let entity = {
                let document = self
                    .workspace
                    .documents
                    .get(&command.document)
                    .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
                document
                    .drawing
                    .entity(reference.entity)
                    .cloned()
                    .ok_or_else(|| {
                        CadError::InvalidInput(format!(
                            "massprop.target_missing: entity {} does not exist in this drawing",
                            reference.entity.0
                        ))
                    })?
            };
            regions.push((reference.entity, self.mass_region(&entity)?));
        }
        self.ensure_coplanar(&regions)?;

        let mut diagnostics = Vec::with_capacity(regions.len() * 2 + 1);
        for (entity, region) in &regions {
            diagnostics.push(Diagnostic {
                object: Some(ObjectId(entity.0)),
                code: "massprop.object".into(),
                message: format!(
                    "MASSPROP: entity {} ({}) area={:.6} perimeter={}{:.6} bbox min={} max={} \
                     centroid={}",
                    entity.0,
                    region.kind,
                    region.area,
                    if region.perimeter.1 { "~" } else { "" },
                    region.perimeter.0,
                    format_point(region.min),
                    format_point(region.max),
                    format_point(region.centroid),
                ),
            });
            if region.perimeter.1 {
                diagnostics.push(Diagnostic {
                    object: Some(ObjectId(entity.0)),
                    code: "massprop.approx_ellipse_perimeter".into(),
                    message: format!(
                        "MASSPROP: entity {} perimeter is Ramanujan's second approximation of the \
                         complete elliptic integral; it is not an exact value",
                        entity.0
                    ),
                });
            }
        }

        let total_area: f64 = regions.iter().map(|(_, region)| region.area).sum();
        let total_perimeter: f64 = regions.iter().map(|(_, region)| region.perimeter.0).sum();
        let approximate = regions.iter().any(|(_, region)| region.perimeter.1);
        let mut min = regions[0].1.min;
        let mut max = regions[0].1.max;
        for (_, region) in &regions[1..] {
            min = min_point(min, region.min);
            max = max_point(max, region.max);
        }
        let centroid = if total_area.is_finite() && total_area > 0.0 {
            let mut x = 0.0;
            let mut y = 0.0;
            let mut z = 0.0;
            for (_, region) in &regions {
                x += region.centroid.x * region.area;
                y += region.centroid.y * region.area;
                z += region.centroid.z * region.area;
            }
            Point3 {
                x: x / total_area,
                y: y / total_area,
                z: z / total_area,
            }
        } else {
            return Err(CadError::InvalidInput(
                "massprop.degenerate: the summed area is not a positive finite value".into(),
            ));
        };
        diagnostics.push(Diagnostic {
            object: None,
            code: "massprop.not_implemented".into(),
            message: "MASSPROP: moments/products of inertia, radii of gyration, principal moments \
                      and axes, mass/volume/3D solids, density, Add/Subtract region composition and \
                      .mpr output are NOT IMPLEMENTED; no value is reported for them"
                .into(),
        });
        diagnostics.push(Diagnostic {
            object: None,
            code: "massprop.summary".into(),
            message: format!(
                "MASSPROP: {} region(s) area={:.6} perimeter={}{:.6} bbox min={} max={} centroid={} \
                 (area-weighted combination of the per-object results; not a CSG union)",
                regions.len(),
                total_area,
                if approximate { "~" } else { "" },
                total_perimeter,
                format_point(min),
                format_point(max),
                format_point(centroid),
            ),
        });

        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// Measure one selected entity as a closed planar region, or refuse it by
    /// name.
    fn mass_region(&self, entity: &cad_db::DbEntity) -> CadResult<MassRegion> {
        match &entity.geometry {
            SemanticGeometry::Circle {
                center,
                normal,
                radius,
            } => {
                let normal = self.mass_normal(entity, *normal)?;
                if !camera::is_finite_point(*center) {
                    return Err(mass_degenerate(entity, "the circle's centre is not finite"));
                }
                if !radius.is_finite() || *radius <= 0.0 {
                    return Err(mass_degenerate(
                        entity,
                        &format!("the circle's radius {radius} is not a positive finite length"),
                    ));
                }
                let area = std::f64::consts::PI * radius * radius;
                if !area.is_finite() || area <= 0.0 {
                    return Err(mass_degenerate(
                        entity,
                        "the computed circle area is not a positive finite value",
                    ));
                }
                let (u, v) = plane_basis(normal).ok_or_else(|| {
                    mass_degenerate(entity, "the circle's normal does not define a plane basis")
                })?;
                let extents = axis_extents(normal, u, v, *radius);
                Ok(MassRegion {
                    kind: "circle",
                    area,
                    perimeter: (std::f64::consts::TAU * radius, false),
                    min: Point3 {
                        x: center.x - extents.x,
                        y: center.y - extents.y,
                        z: center.z - extents.z,
                    },
                    max: Point3 {
                        x: center.x + extents.x,
                        y: center.y + extents.y,
                        z: center.z + extents.z,
                    },
                    centroid: *center,
                    normal,
                    plane_point: *center,
                })
            }
            SemanticGeometry::Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            } => {
                let normal = self.mass_normal(entity, *normal)?;
                if !camera::is_finite_point(*center) || !camera::is_finite_point(*major_axis) {
                    return Err(mass_degenerate(
                        entity,
                        "the ellipse's centre/major axis is not finite",
                    ));
                }
                if !start.is_finite() || !sweep.is_finite() {
                    return Err(mass_degenerate(
                        entity,
                        "the ellipse's start angle/sweep is not finite",
                    ));
                }
                if sweep.abs() < std::f64::consts::TAU - FULL_TURN_TOLERANCE_RADIANS {
                    return Err(CadError::InvalidInput(format!(
                        "massprop.ellipse_arc: entity {} is an elliptical arc (sweep {sweep} rad \
                         of a full turn); MASSPROP needs a closed region",
                        entity.id.0
                    )));
                }
                let major = camera::length3(*major_axis);
                if !major.is_finite() || major <= 0.0 {
                    return Err(mass_degenerate(
                        entity,
                        &format!("the ellipse's major axis length {major} is not positive"),
                    ));
                }
                if !ratio.is_finite() || *ratio <= 0.0 {
                    return Err(mass_degenerate(
                        entity,
                        &format!("the ellipse's ratio {ratio} is not positive and finite"),
                    ));
                }
                let major_unit = camera::normalize3(*major_axis).ok_or_else(|| {
                    mass_degenerate(entity, "the ellipse's major axis has no direction")
                })?;
                let minor_unit = camera::normalize3(camera::cross(normal, major_unit))
                    .ok_or_else(|| {
                        mass_degenerate(
                            entity,
                            "the ellipse's normal and major axis do not define a plane basis",
                        )
                    })?;
                let minor = major * ratio;
                let area = std::f64::consts::PI * major * minor;
                if !area.is_finite() || area <= 0.0 {
                    return Err(mass_degenerate(
                        entity,
                        "the computed ellipse area is not a positive finite value",
                    ));
                }
                // Ramanujan's second approximation for a full ellipse. This is
                // explicitly approximate (no elliptic integral in this build).
                let h = ((major - minor) / (major + minor)).powi(2);
                let perimeter =
                    std::f64::consts::PI * (major + minor) * (1.0 + 3.0 * h / (10.0 + (4.0 - 3.0 * h).sqrt()));
                if !perimeter.is_finite() {
                    return Err(mass_degenerate(
                        entity,
                        "the approximated ellipse perimeter is not finite",
                    ));
                }
                let extents = Point3 {
                    x: (major * major_unit.x).hypot(minor * minor_unit.x),
                    y: (major * major_unit.y).hypot(minor * minor_unit.y),
                    z: (major * major_unit.z).hypot(minor * minor_unit.z),
                };
                Ok(MassRegion {
                    kind: "ellipse",
                    area,
                    perimeter: (perimeter, true),
                    min: Point3 {
                        x: center.x - extents.x,
                        y: center.y - extents.y,
                        z: center.z - extents.z,
                    },
                    max: Point3 {
                        x: center.x + extents.x,
                        y: center.y + extents.y,
                        z: center.z + extents.z,
                    },
                    centroid: *center,
                    normal,
                    plane_point: *center,
                })
            }
            SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
            } => {
                if !*closed {
                    return Err(CadError::InvalidInput(format!(
                        "massprop.open_polyline: entity {} is an open polyline; MASSPROP needs a \
                         closed region",
                        entity.id.0
                    )));
                }
                if points.iter().any(|p| !camera::is_finite_point(*p))
                    || bulges.iter().any(|b| !b.is_finite())
                {
                    return Err(mass_degenerate(
                        entity,
                        "the polyline has a non-finite vertex or bulge",
                    ));
                }
                let ring = measure_bulged_ring_area(
                    points,
                    bulges,
                    self.measurement.tolerance.computation_world,
                )
                .map_err(|error| mass_polyline_refusal(entity, error))?;
                let normal = camera::normalize3(camera::cross(ring.plane.u, ring.plane.v))
                    .ok_or_else(|| {
                        mass_degenerate(entity, "the polyline's plane normal is not finite")
                    })?;
                let flat: Vec<[f64; 2]> = points
                    .iter()
                    .map(|p| {
                        let d = camera::sub(*p, ring.plane.origin);
                        [camera::dot3(d, ring.plane.u), camera::dot3(d, ring.plane.v)]
                    })
                    .collect();
                let perimeter = bulged_ring_perimeter(points, bulges)
                    .filter(|value| value.is_finite() && *value > 0.0)
                    .ok_or_else(|| {
                        mass_degenerate(entity, "the closed polyline has no finite perimeter")
                    })?;
                let centroid = bulged_ring_centroid(ring.plane, &flat, bulges).ok_or_else(|| {
                    mass_degenerate(
                        entity,
                        "the closed polyline's centroid could not be derived",
                    )
                })?;
                let (min, max) = bulged_ring_bounds(points, bulges, ring.plane);
                Ok(MassRegion {
                    kind: "closed polyline",
                    area: ring.area,
                    perimeter: (perimeter, false),
                    min,
                    max,
                    centroid,
                    normal,
                    plane_point: ring.plane.origin,
                })
            }
            SemanticGeometry::Arc { .. } => Err(CadError::InvalidInput(format!(
                "massprop.arc: entity {} is an ARC (an open curve); MASSPROP needs a closed region",
                entity.id.0
            ))),
            other => Err(CadError::Unsupported(format!(
                "massprop.unsupported_kind: MASSPROP measures closed CIRCLE, full ELLIPSE or closed \
                 POLYLINE; entity {} is {}",
                entity.id.0,
                geometry_kind(other)
            ))),
        }
    }

    /// Normalise one circle/ellipse normal, or refuse a degenerate one.
    fn mass_normal(&self, entity: &cad_db::DbEntity, normal: Point3) -> CadResult<Point3> {
        if !camera::is_finite_point(normal) {
            return Err(mass_degenerate(entity, "the region's normal is not finite"));
        }
        camera::normalize3(normal)
            .ok_or_else(|| mass_degenerate(entity, "the region's normal has zero length"))
    }

    /// Refuse a selection whose regions do not share one plane.
    fn ensure_coplanar(&self, regions: &[(EntityId, MassRegion)]) -> CadResult<()> {
        let tolerance = self.measurement.tolerance.computation_world.max(1e-9);
        let (_, first) = &regions[0];
        let first_offset = camera::dot3(first.normal, first.plane_point);
        for (entity, region) in &regions[1..] {
            let parallel = camera::dot3(first.normal, region.normal).abs() >= 1.0 - 1e-9;
            let same_offset =
                (camera::dot3(region.normal, region.plane_point) - first_offset).abs() <= tolerance;
            if !parallel || !same_offset {
                return Err(CadError::InvalidInput(format!(
                    "massprop.not_coplanar: entity {} is not on the selection's plane; MASSPROP \
                     needs all selected regions to be coplanar",
                    entity.0
                )));
            }
        }
        Ok(())
    }

    /// `MEASUREGEOM`: route the option that reached the command layer.
    ///
    /// The four tool options (`DISTANCE`/`ANGLE`/`AREA`/`POLYLINE`) never travel
    /// this far: the session turns them into the existing `BeginMeasurement`
    /// requests. `RADIUS` is the read-only report and `VOLUME` the explicit
    /// not-implemented refusal. Full contract:
    /// [`crate::command_id::measuregeom`].
    pub(crate) fn measure_geom(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match command.payload {
            CommandPayload::MeasureGeom(MeasureGeomOption::Radius) => {
                self.measure_geom_radius(session, command)
            }
            CommandPayload::MeasureGeom(MeasureGeomOption::Volume) => Err(CadError::Unsupported(
                "measuregeom.volume_unsupported: MEASUREGEOM VOLUME is not implemented (this build \
                 has no 3D-solid or volume model); no value is reported"
                    .into(),
            )),
            CommandPayload::MeasureGeom(
                option @ (MeasureGeomOption::Distance
                | MeasureGeomOption::Angle
                | MeasureGeomOption::Area
                | MeasureGeomOption::Polyline),
            ) => Err(CadError::InvalidInput(format!(
                "measuregeom.invalid_payload: {option:?} starts the corresponding measurement tool \
                 in the session and has no command-layer report"
            ))),
            CommandPayload::None => Err(CadError::InvalidInput(
                "measuregeom.needs_option: MEASUREGEOM needs one of DISTANCE, ANGLE, AREA, \
                 POLYLINE, RADIUS or VOLUME"
                    .into(),
            )),
            _ => Err(CadError::InvalidInput(
                "measuregeom.invalid_payload: MEASUREGEOM takes its option as a \
                 CommandPayload::MeasureGeom value".into(),
            )),
        }
    }

    /// `MEASUREGEOM RADIUS`: report the one selected ARC/CIRCLE's radius family.
    fn measure_geom_radius(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let references = session.selection.refs();
        match references {
            [] => {
                return Err(CadError::InvalidInput(
                    "measuregeom.radius.selection_empty: MEASUREGEOM RADIUS needs exactly one \
                     selected ARC or CIRCLE"
                        .into(),
                ));
            }
            [_] => {}
            many => {
                return Err(CadError::InvalidInput(format!(
                    "measuregeom.radius.selection_multiple: MEASUREGEOM RADIUS measures one \
                     ARC/CIRCLE, got {} selected; nothing was measured",
                    many.len()
                )));
            }
        }
        let reference = &references[0];
        if reference.document != command.document {
            return Err(CadError::InvalidInput(format!(
                "measuregeom.radius.target_missing: the selected ref for entity {} belongs to \
                 another document",
                reference.entity.0
            )));
        }
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(CadError::Unsupported(
                "measuregeom.radius.not_addressable: block-instance members and sub-elements have \
                 no addressable entity identity"
                    .into(),
            ));
        }
        let entity = {
            let document = self
                .workspace
                .documents
                .get(&command.document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            document
                .drawing
                .entity(reference.entity)
                .cloned()
                .ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "measuregeom.radius.target_missing: entity {} does not exist in this \
                         drawing",
                        reference.entity.0
                    ))
                })?
        };
        let (arc_length, center, radius) = match &entity.geometry {
            SemanticGeometry::Circle {
                center,
                normal: _,
                radius,
            } => {
                validate_radius_entity(&entity, *center, *radius)?;
                (None, *center, *radius)
            }
            SemanticGeometry::Arc {
                center,
                normal: _,
                radius,
                start: _,
                sweep,
            } => {
                validate_radius_entity(&entity, *center, *radius)?;
                if !sweep.is_finite() {
                    return Err(CadError::InvalidInput(format!(
                        "measuregeom.radius.degenerate: entity {} has a non-finite sweep",
                        entity.id.0
                    )));
                }
                (Some(radius * sweep.abs()), *center, *radius)
            }
            other => {
                return Err(CadError::Unsupported(format!(
                    "measuregeom.radius.not_arc_circle: MEASUREGEOM RADIUS measures one ARC or \
                     CIRCLE; entity {} is {}",
                    entity.id.0,
                    geometry_kind(other)
                )));
            }
        };
        let diameter = 2.0 * radius;
        let circumference = std::f64::consts::TAU * radius;
        let arc_note = arc_length
            .map(|length| format!(" arc_length={length:.6}"))
            .unwrap_or_default();
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(entity.id.0)),
                code: "measuregeom.radius".into(),
                message: format!(
                    "MEASUREGEOM RADIUS: entity {} centre={} radius={radius:.6} diameter={diameter:.6} \
                     circumference={circumference:.6}{arc_note}",
                    entity.id.0,
                    format_point(center),
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// The normal of a circle/ellipse is a plane normal, not an orientation: `±n`
/// describes the same region. This keeps the reported normal unit-length and
/// deterministic (the stored sign is preserved).
fn plane_basis(normal: Point3) -> Option<(Point3, Point3)> {
    let normal = camera::normalize3(normal)?;
    let helper = if normal.x.abs() < 0.9 {
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        }
    } else {
        Point3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        }
    };
    let u = camera::normalize3(camera::cross(helper, normal))?;
    let v = camera::normalize3(camera::cross(normal, u))?;
    Some((u, v))
}

/// The circle's half-extent per world axis: every point is `c + r(cosθ·u +
/// sinθ·v)`, so the amplitude along an axis is `r·hypot(u_i, v_i)`.
fn axis_extents(_normal: Point3, u: Point3, v: Point3, radius: f64) -> Point3 {
    Point3 {
        x: radius * u.x.hypot(v.x),
        y: radius * u.y.hypot(v.y),
        z: radius * u.z.hypot(v.z),
    }
}

/// Exact perimeter of a closed bulged ring.
///
/// Straight chords use [`cad_geometry::polyline_length`] (the existing single
/// chord-length rule); each nonzero bulge replaces its chord with the exact
/// circular-arc length `L·|θ| / (2·|sin(θ/2)|)`, `θ = 4·atan(bulge)`.
fn bulged_ring_perimeter(points: &[Point3], bulges: &[f64]) -> Option<f64> {
    if points.len() != bulges.len() || points.len() < 3 {
        return None;
    }
    let mut closed = points.to_vec();
    closed.push(points[0]);
    let mut total = polyline_length(&closed);
    for (index, bulge) in bulges.iter().enumerate() {
        if *bulge == 0.0 {
            continue;
        }
        let next = (index + 1) % points.len();
        let chord = polyline_length(&[points[index], points[next]]);
        if !chord.is_finite() || chord <= 0.0 {
            return None;
        }
        let theta = 4.0 * bulge.atan();
        let half = theta * 0.5;
        let arc = if half.abs() < 1e-12 {
            chord
        } else {
            chord * theta.abs() / (2.0 * half.sin().abs())
        };
        if !arc.is_finite() {
            return None;
        }
        total += arc - chord;
    }
    total.is_finite().then_some(total)
}

/// Area-weighted centroid of a closed bulged ring, in the ring's own plane.
///
/// The straight-edge contribution is the polygon first-moment integral
/// `∬x dA = (1/6)Σ(x_i+x_{i+1})·cross_i`; every bulge adds one **circular
/// segment** whose area is `(r²/2)(θ−sinθ)` and whose centroid lies on the apex
/// direction at `y_c = 4r·sin³(θ/2) / (3(θ−sinθ))` from the arc centre. Both
/// small-θ expressions use their series to avoid cancellation; the derivation is
/// pinned by the `massprop` tests (semicircle centroid `4r/(3π)` etc.).
fn bulged_ring_centroid(plane: WorkPlane, flat: &[[f64; 2]], bulges: &[f64]) -> Option<Point3> {
    if flat.len() != bulges.len() || flat.len() < 3 {
        return None;
    }
    let mut signed_area = 0.0_f64;
    let mut moment_x = 0.0_f64;
    let mut moment_y = 0.0_f64;
    for index in 0..flat.len() {
        let a = flat[index];
        let b = flat[(index + 1) % flat.len()];
        let cross = a[0] * b[1] - a[1] * b[0];
        signed_area += 0.5 * cross;
        moment_x += (a[0] + b[0]) * cross / 6.0;
        moment_y += (a[1] + b[1]) * cross / 6.0;
        let bulge = bulges[index];
        if bulge.abs() < 1e-12 {
            continue;
        }
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let chord = dx.hypot(dy);
        if !chord.is_finite() || chord <= 0.0 {
            return None;
        }
        let theta = 4.0 * bulge.atan();
        let half = theta * 0.5;
        let sagitta = bulge * chord * 0.5;
        let radius = (chord * chord * 0.25 + sagitta * sagitta) / (2.0 * sagitta.abs());
        if !radius.is_finite() || radius <= 0.0 {
            return None;
        }
        let side = if bulge > 0.0 { 1.0 } else { -1.0 };
        // A positive bulge bows to the **right** of the chord (the same frame
        // `bulge_arc_geometry` uses); `apex` points from the chord to the arc,
        // so the arc centre is on the opposite side (for |θ| < π).
        let left = [-dy / chord, dx / chord];
        let apex = [-left[0] * side, -left[1] * side];
        let center = [
            (a[0] + b[0]) * 0.5 - apex[0] * radius * half.cos(),
            (a[1] + b[1]) * 0.5 - apex[1] * radius * half.cos(),
        ];
        let segment_area = if theta.abs() < 1e-3 {
            0.5 * radius * radius * theta.powi(3) / 6.0
                * (1.0 - theta * theta / 20.0 + theta.powi(4) / 840.0)
        } else {
            0.5 * radius * radius * (theta - theta.sin())
        };
        let y_c = if theta.abs() < 1e-3 {
            radius * (1.0 - 3.0 * theta * theta / 40.0)
        } else {
            4.0 * radius * half.sin().powi(3) / (3.0 * (theta - theta.sin()))
        };
        signed_area += segment_area;
        moment_x += segment_area * (center[0] + apex[0] * y_c);
        moment_y += segment_area * (center[1] + apex[1] * y_c);
    }
    if !signed_area.is_finite() || signed_area.abs() < 1e-15 {
        return None;
    }
    let x = moment_x / signed_area;
    let y = moment_y / signed_area;
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    Some(camera::add(
        plane.origin,
        camera::add(camera::scale3(plane.u, x), camera::scale3(plane.v, y)),
    ))
}

/// Exact world bounding box of a closed bulged ring: every vertex plus every arc
/// axis-extreme angle that lies inside a segment's sweep.
fn bulged_ring_bounds(points: &[Point3], bulges: &[f64], plane: WorkPlane) -> (Point3, Point3) {
    let mut min = points[0];
    let mut max = points[0];
    for point in points {
        min = min_point(min, *point);
        max = max_point(max, *point);
    }
    let origin = plane.origin;
    let flat: Vec<[f64; 2]> = points
        .iter()
        .map(|p| {
            let d = camera::sub(*p, origin);
            [camera::dot3(d, plane.u), camera::dot3(d, plane.v)]
        })
        .collect();
    for (index, bulge) in bulges.iter().enumerate() {
        if *bulge == 0.0 {
            continue;
        }
        let a = flat[index];
        let b = flat[(index + 1) % flat.len()];
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let chord = dx.hypot(dy);
        if !chord.is_finite() || chord <= 0.0 {
            continue;
        }
        let theta = 4.0 * bulge.atan();
        let half = theta * 0.5;
        let sagitta = bulge * chord * 0.5;
        let radius = (chord * chord * 0.25 + sagitta * sagitta) / (2.0 * sagitta.abs());
        if !radius.is_finite() || radius <= 0.0 {
            continue;
        }
        let side = if *bulge > 0.0 { 1.0 } else { -1.0 };
        // A positive bulge bows to the **right** of the chord (the same frame
        // `bulge_arc_geometry` uses); `apex` points from the chord to the arc,
        // so the arc centre is on the opposite side (for |θ| < π).
        let left = [-dy / chord, dx / chord];
        let apex = [-left[0] * side, -left[1] * side];
        let center = [
            (a[0] + b[0]) * 0.5 - apex[0] * radius * half.cos(),
            (a[1] + b[1]) * 0.5 - apex[1] * radius * half.cos(),
        ];
        let center3 = camera::add(
            origin,
            camera::add(
                camera::scale3(plane.u, center[0]),
                camera::scale3(plane.v, center[1]),
            ),
        );
        let start = (a[1] - center[1]).atan2(a[0] - center[0]);
        for axis in 0..3 {
            let (u_component, v_component) = match axis {
                0 => (plane.u.x, plane.v.x),
                1 => (plane.u.y, plane.v.y),
                _ => (plane.u.z, plane.v.z),
            };
            let amplitude = u_component.hypot(v_component);
            if amplitude < 1e-15 {
                continue;
            }
            let phase = v_component.atan2(u_component);
            let center_component = match axis {
                0 => center3.x,
                1 => center3.y,
                _ => center3.z,
            };
            for (angle, sign) in [(phase, 1.0), (phase + std::f64::consts::PI, -1.0)] {
                if angle_in_sweep(angle, start, theta) {
                    let value = center_component + sign * radius * amplitude;
                    match axis {
                        0 => {
                            min.x = min.x.min(value);
                            max.x = max.x.max(value);
                        }
                        1 => {
                            min.y = min.y.min(value);
                            max.y = max.y.max(value);
                        }
                        _ => {
                            min.z = min.z.min(value);
                            max.z = max.z.max(value);
                        }
                    }
                }
            }
        }
    }
    (min, max)
}

/// Whether `angle` lies on the signed sweep `[start, start + sweep]`.
fn angle_in_sweep(angle: f64, start: f64, sweep: f64) -> bool {
    let tau = std::f64::consts::TAU;
    let delta = (angle - start).rem_euclid(tau);
    if sweep >= 0.0 {
        delta <= sweep + 1e-12
    } else {
        delta - tau >= sweep - 1e-12
    }
}

fn min_point(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x.min(b.x),
        y: a.y.min(b.y),
        z: a.z.min(b.z),
    }
}

fn max_point(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x.max(b.x),
        y: a.y.max(b.y),
        z: a.z.max(b.z),
    }
}

fn format_point(p: Point3) -> String {
    format!("({:.6}, {:.6}, {:.6})", p.x, p.y, p.z)
}

fn mass_degenerate(entity: &cad_db::DbEntity, reason: &str) -> CadError {
    CadError::InvalidInput(format!(
        "massprop.degenerate: entity {} cannot be measured: {reason}",
        entity.id.0
    ))
}

fn validate_radius_entity(entity: &cad_db::DbEntity, center: Point3, radius: f64) -> CadResult<()> {
    if !camera::is_finite_point(center) {
        return Err(CadError::InvalidInput(format!(
            "measuregeom.radius.degenerate: entity {} has a non-finite centre",
            entity.id.0
        )));
    }
    if !radius.is_finite() || radius <= 0.0 {
        return Err(CadError::InvalidInput(format!(
            "measuregeom.radius.degenerate: entity {} has radius {radius}, which is not a positive \
             finite length",
            entity.id.0
        )));
    }
    Ok(())
}

/// Map one [`AreaError`] from the bulge-aware ring measurement to a **MASSPROP**
/// refusal code (the shared `AREA OBJECT` mapper names that command instead).
fn mass_polyline_refusal(entity: &cad_db::DbEntity, error: AreaError) -> CadError {
    let id = entity.id.0;
    match error {
        AreaError::Degenerate => {
            CadError::InvalidInput(format!("massprop.degenerate: entity {id} has zero area"))
        }
        AreaError::TooFewPoints(count) => CadError::InvalidInput(format!(
            "massprop.polyline_plane: entity {id} encloses no ring ({count} distinct vertices)"
        )),
        AreaError::UndeterminedPlane => CadError::InvalidInput(format!(
            "massprop.polyline_plane: entity {id}'s plane cannot be derived from its vertices, so              the bulge sign frame is undetermined"
        )),
        AreaError::NonPlanar => CadError::InvalidInput(format!(
            "massprop.non_planar: entity {id} is not planar within the computation tolerance"
        )),
        AreaError::SelfIntersecting => CadError::InvalidInput(format!(
            "massprop.self_intersecting: entity {id} is self-intersecting; area is undefined"
        )),
        AreaError::BulgesMismatch { points, bulges } => CadError::InvalidInput(format!(
            "massprop.degenerate: entity {id} is a corrupt polyline ({points} vertices, {bulges}              bulges; one bulge per vertex is required)"
        )),
    }
}
