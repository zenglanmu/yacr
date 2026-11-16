//! `MASSPROP` and `MEASUREGEOM RADIUS`: the two AutoCAD 2026 inquiry reports.
//!
//! Split out of `app_measure_object.rs` to keep both modules inside the
//! code-size limit. The long-form contracts live in
//! [`crate::command_id::massprop`] and [`crate::command_id::measuregeom`]; this
//! module owns only the arithmetic and the diagnostic formatting. Neither path
//! writes the database, opens a transaction, records history or bumps a
//! revision.

use super::*;
use cad_geometry::{
    arbitrary_axis, measure_bulged_ring_area, polyline_length, AreaError, PlanarMoments,
};

use crate::app_measure_object::geometry_kind;

// The sampled closed-SPLINE and HATCH-loop region arithmetic lives in a sibling
// module (this file is at the code-size ceiling). `#[path]` keeps it a sibling
// file rather than a directory.
#[path = "app_massprop_regions.rs"]
mod regions;

// The `MEASUREGEOM` half (option routing plus the `RADIUS` report) is the other
// sibling: same reason, and the same `impl Application` funnel.
#[path = "app_massprop_measuregeom.rs"]
mod measuregeom;

// --- MASSPROP (2026-11-07) ------------------------------------------------
//
// The contract (input, supported kinds, every refusal, the report shape and
// every un-modelled AutoCAD property) is documented in
// [`crate::command_id::massprop`]; this module owns the arithmetic.

/// Stable refusal reason: a 3D body's mass properties need the exact B-rep
/// kernel, which this build does not link. Its display tessellation is never
/// substituted for the solid (a mesh volume is not the body's volume).
pub const REASON_MASSPROP_BODY_REQUIRES_KERNEL: &str = "massprop.body_requires_kernel";

/// One measured region before it is formatted into diagnostics.
struct MassRegion {
    kind: &'static str,
    /// The region's exact area (a magnitude: every kind is a positive region; a
    /// hatch's holes have already been subtracted inside its own loops).
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
    /// The region's own in-plane orthonormal basis. `plane_point` is its origin
    /// and `moments` is integrated in it, so `(u, v, plane_point)` is the frame
    /// every second moment below is expressed in.
    u: Point3,
    v: Point3,
    /// `∫x² dA`, `∫y² dA`, `∫xy dA` (plus the area/first moments) about
    /// `plane_point` in the `(u, v)` frame, in the geometric convention (a
    /// clockwise winding is normalised away). `area` repeats the value above.
    moments: PlanarMoments,
    /// Whether `moments` is a closed-form integral of the entity's own geometry.
    /// A sampled kind (a closed SPLINE's sample ring, an imported HATCH boundary
    /// the importer flattened) is `false`: its second moments are the sample
    /// polygon's, exactly like its area and perimeter.
    exact: bool,
    /// Extra per-object diagnostics, emitted right after `massprop.object`.
    ///
    /// Only the sampled/loop-derived kinds carry notes: a closed SPLINE's values
    /// come from an adaptive sample ring, and an imported HATCH's boundary may be
    /// a sampled approximation. The exact analytic kinds (circle, full ellipse,
    /// closed bulged polyline) leave this empty.
    notes: Vec<(&'static str, String)>,
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

        // --- 2D mass properties -------------------------------------------
        //
        // One common frame for the whole selection: the first selected region's
        // plane origin and the AutoCAD arbitrary axis of its (unit) normal. Every
        // region's moments were integrated in *its own* plane basis — which is
        // what makes a bulge's sign mean the right thing — so they are
        // re-expressed in the selection frame here, exactly (a rotation or
        // reflection plus the parallel-axis translation). The per-region lines
        // and the selection line are therefore directly comparable.
        let frame = selection_frame(&regions[0].1);
        let mut resolved = Vec::with_capacity(regions.len());
        for (entity, region) in &regions {
            let moments = cad_geometry::reexpress(
                region.moments,
                WorkPlane {
                    origin: region.plane_point,
                    u: region.u,
                    v: region.v,
                },
                frame,
            )
            .ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "massprop.inertia_degenerate: entity {}'s region frame is not an orthonormal \
                     basis of the selection plane, so its second moments cannot be placed in the \
                     selection frame; no value is reported",
                    entity.0
                ))
            })?
            .with_positive_area();
            let inertia = moments.centroidal().ok_or_else(|| {
                CadError::InvalidInput(format!(
                    "massprop.degenerate: entity {}'s area is zero or underflowed, so it has no \
                     centroid to shift its second moments onto",
                    entity.0
                ))
            })?;
            if !inertia.is_finite_and_valid() {
                return Err(mass_inertia_degenerate(
                    "the region's second-moment tensor is not finite or not positive \
                     semi-definite, so it cannot be a real region's inertia",
                ));
            }
            resolved.push((moments, inertia));
        }

        let mut diagnostics = Vec::with_capacity(regions.len() * 3 + 3);
        let mut total_moments = PlanarMoments::ZERO;
        for ((entity, region), (moments, inertia)) in regions.iter().zip(&resolved) {
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
            for (code, message) in &region.notes {
                diagnostics.push(Diagnostic {
                    object: Some(ObjectId(entity.0)),
                    code: (*code).into(),
                    message: message.clone(),
                });
            }
            // `Ix` is the second moment about the centroidal axis parallel to the
            // frame's `u` direction, i.e. `∫v² dA` in the frame's own
            // coordinates, and `Iy = ∫u² dA`; both are exact for every analytic
            // kind and the sample ring's for a sampled one.
            let (kx, ky) = match (inertia.gyration_x(), inertia.gyration_y()) {
                (Some(kx), Some(ky)) => (kx, ky),
                _ => {
                    return Err(mass_inertia_degenerate(
                        "the region's radii of gyration are not finite",
                    ))
                }
            };
            diagnostics.push(Diagnostic {
                object: Some(ObjectId(entity.0)),
                code: "massprop.inertia".into(),
                message: format!(
                    "MASSPROP: entity {} ({}) second moments about its own centroid in the \
                     selection frame: Ix={:.6} Iy={:.6} Ixy={:.6} J={:.6} kx={:.6} ky={:.6} \
                     (Ix=∫v²dA, Iy=∫u²dA, Ixy=∫uv dA; {})",
                    entity.0,
                    region.kind,
                    inertia.inertia_x,
                    inertia.inertia_y,
                    inertia.product_xy,
                    inertia.polar(),
                    kx,
                    ky,
                    if region.exact {
                        "exact closed form"
                    } else {
                        "the sample polygon's values, not closed-form"
                    },
                ),
            });
            total_moments = total_moments + *moments;
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
        // The selection's centroidal tensor: the summed raw moments are about the
        // selection frame origin, so one parallel-axis shift moves them to the
        // combined centroid — the same `A = Σaᵢ`, `c = Σaᵢcᵢ/A` combination the
        // area line reports.
        let total_inertia = total_moments.centroidal().ok_or_else(|| {
            CadError::InvalidInput(
                "massprop.degenerate: the summed area is not a positive finite value, so no \
                 centroid can be derived"
                    .into(),
            )
        })?;
        if !total_inertia.is_finite_and_valid() {
            return Err(mass_inertia_degenerate(
                "the selection's summed second-moment tensor is not finite or not positive \
                 semi-definite",
            ));
        }
        let (kx, ky, kz) = match (
            total_inertia.gyration_x(),
            total_inertia.gyration_y(),
            total_inertia.gyration_polar(),
        ) {
            (Some(kx), Some(ky), Some(kz)) => (kx, ky, kz),
            _ => {
                return Err(mass_inertia_degenerate(
                    "the selection's radii of gyration are not finite",
                ))
            }
        };
        let principal = total_inertia.principal().ok_or_else(|| {
            mass_inertia_degenerate("the selection's principal axes cannot be derived")
        })?;
        let centroid = camera::add(
            frame.origin,
            camera::add(
                camera::scale3(frame.u, total_inertia.centroid[0]),
                camera::scale3(frame.v, total_inertia.centroid[1]),
            ),
        );
        // A principal axis is a direction in the plane: lift it back to world.
        let axis_major = camera::add(
            camera::scale3(frame.u, principal.major_axis[0]),
            camera::scale3(frame.v, principal.major_axis[1]),
        );
        let minor_axis = principal.minor_axis();
        let axis_minor = camera::add(
            camera::scale3(frame.u, minor_axis[0]),
            camera::scale3(frame.v, minor_axis[1]),
        );
        let normal = camera::normalize3(camera::cross(frame.u, frame.v))
            .ok_or_else(|| mass_inertia_degenerate("the selection frame has no normal"))?;

        diagnostics.push(Diagnostic {
            object: None,
            code: "massprop.summary_inertia".into(),
            message: format!(
                "MASSPROP: {} region(s) about the combined centroid {}: Ix={:.6} Iy={:.6} \
                 Ixy={:.6} J={:.6} kx={:.6} ky={:.6} kz={:.6} principal I1={:.6} axis1={} \
                 I2={:.6} axis2={} principal_angle_deg={:.6}; frame origin={} u={} v={} n={} \
                 (the first selected region's plane; Ix is about the u-parallel axis, Iy about \
                 the v-parallel one, and principal_angle_deg is measured from u in the plane)",
                regions.len(),
                format_point(centroid),
                total_inertia.inertia_x,
                total_inertia.inertia_y,
                total_inertia.product_xy,
                total_inertia.polar(),
                kx,
                ky,
                kz,
                principal.major,
                format_point(axis_major),
                principal.minor,
                format_point(axis_minor),
                principal.angle.to_degrees(),
                format_point(frame.origin),
                format_point(frame.u),
                format_point(frame.v),
                format_point(normal),
            ),
        });
        diagnostics.push(Diagnostic {
            object: None,
            code: "massprop.solid_unsupported".into(),
            message: "MASSPROP: volume, mass, density and 3D moments of inertia are NOT REPORTED \
                      (this build has no linked B-rep kernel); no value is computed or invented \
                      for them"
                .into(),
        });
        diagnostics.push(Diagnostic {
            object: None,
            code: "massprop.not_implemented".into(),
            message: "MASSPROP: Add/Subtract region composition and .mpr output are NOT \
                      IMPLEMENTED; the reported values are the area-weighted combination of the \
                      selected regions, not a CSG union"
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
                // A disc's second moments are the same about every centroidal
                // axis in its plane: ∫x² = ∫y² = πr⁴/4, ∫xy = 0.
                let second = std::f64::consts::FRAC_PI_4 * radius.powi(4);
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
                    u,
                    v,
                    moments: PlanarMoments {
                        area,
                        first_x: 0.0,
                        first_y: 0.0,
                        second_x: second,
                        second_y: second,
                        second_xy: 0.0,
                    },
                    exact: true,
                    notes: Vec::new(),
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
                let minor_unit =
                    camera::normalize3(camera::cross(normal, major_unit)).ok_or_else(|| {
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
                let perimeter = std::f64::consts::PI
                    * (major + minor)
                    * (1.0 + 3.0 * h / (10.0 + (4.0 - 3.0 * h).sqrt()));
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
                let (u, v) = plane_basis(normal).ok_or_else(|| {
                    mass_degenerate(entity, "the ellipse's normal does not define a plane basis")
                })?;
                // The ellipse's own principal second moments: with ξ along the
                // major axis and η along the minor one, ∫ξ² = πa³b/4 and
                // ∫η² = πab³/4, and ∫ξη = 0. Rotate that diagonal tensor into
                // the region frame `(u, v)`.
                let along_major = std::f64::consts::FRAC_PI_4 * major.powi(3) * minor;
                let along_minor = std::f64::consts::FRAC_PI_4 * major * minor.powi(3);
                let e1 = [camera::dot3(major_unit, u), camera::dot3(major_unit, v)];
                let e2 = [camera::dot3(minor_unit, u), camera::dot3(minor_unit, v)];
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
                    u,
                    v,
                    moments: PlanarMoments {
                        area,
                        first_x: 0.0,
                        first_y: 0.0,
                        second_x: along_major * e1[0] * e1[0] + along_minor * e2[0] * e2[0],
                        second_y: along_major * e1[1] * e1[1] + along_minor * e2[1] * e2[1],
                        second_xy: along_major * e1[0] * e1[1] + along_minor * e2[0] * e2[1],
                    },
                    exact: true,
                    // The perimeter above is Ramanujan's second approximation, so the
                    // ellipse — and only the ellipse — carries this note. Its area and
                    // second moments are exact and do not depend on it.
                    notes: vec![(
                        "massprop.approx_ellipse_perimeter",
                        format!(
                            "MASSPROP: entity {} perimeter is Ramanujan's second approximation of \
                             the complete elliptic integral; it is not an exact value (the area, \
                             centroid and second moments are exact)",
                            entity.id.0
                        ),
                    )],
                })
            }
            SemanticGeometry::Polyline {
                points,
                bulges,
                closed,
                ..
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
                let centroid =
                    bulged_ring_centroid(ring.plane, &flat, bulges).ok_or_else(|| {
                        mass_degenerate(
                            entity,
                            "the closed polyline's centroid could not be derived",
                        )
                    })?;
                let (min, max) = bulged_ring_bounds(points, bulges, ring.plane);
                let moments = cad_geometry::bulged_ring_moments(&flat, bulges)
                    .ok_or_else(|| {
                        mass_degenerate(
                            entity,
                            "the closed polyline's second moments could not be derived",
                        )
                    })?
                    .with_positive_area();
                Ok(MassRegion {
                    kind: "closed polyline",
                    area: ring.area,
                    perimeter: (perimeter, false),
                    min,
                    max,
                    centroid,
                    normal,
                    plane_point: ring.plane.origin,
                    u: ring.plane.u,
                    v: ring.plane.v,
                    moments,
                    exact: true,
                    notes: Vec::new(),
                })
            }
            SemanticGeometry::Arc { .. } => Err(CadError::InvalidInput(format!(
                "massprop.arc: entity {} is an ARC (an open curve); MASSPROP needs a closed region",
                entity.id.0
            ))),
            SemanticGeometry::Spline {
                degree,
                knots,
                control_points,
                weights,
                ..
            } => regions::mass_spline_region(
                entity,
                *degree,
                knots,
                control_points,
                weights,
                self.measurement.tolerance.computation_world,
            ),
            SemanticGeometry::Hatch(hatch) => regions::mass_hatch_region(
                entity,
                hatch,
                self.measurement.tolerance.computation_world,
            ),
            // A body's mass properties are never derived from its display
            // tessellation: a mesh volume is not the solid's volume. No kernel is
            // linked in this build, so the refusal is explicit and stable.
            SemanticGeometry::Body3d(body) => Err(CadError::Unsupported(format!(
                "{}: entity {} ({}) is a 3D BODY (kind {:?}); its mass properties need the exact \
                 B-rep kernel, which is not linked in this build, so no display-mesh volume is \
                 reported",
                REASON_MASSPROP_BODY_REQUIRES_KERNEL,
                entity.id.0,
                entity.object.type_key,
                body.kind
            ))),
            other => Err(CadError::Unsupported(format!(
                "massprop.unsupported_kind: MASSPROP measures closed CIRCLE, full ELLIPSE, closed \
                 POLYLINE, closed SPLINE or a HATCH region; entity {} is {}",
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
}

/// The one frame the whole selection's second moments are reported in.
///
/// The origin is the first selected region's plane point and the axes are the
/// AutoCAD arbitrary axis of its normal, i.e. the OCS basis a hatch already
/// stores, so a region in a world-XY plane reports `u = +X`, `v = +Y` and its
/// `Ix` is the familiar `∫y² dA`. Every region's own-frame moments are re-expressed
/// in this frame before they are summed, so the per-object lines and the
/// selection line are comparable.
fn selection_frame(first: &MassRegion) -> WorkPlane {
    let (u, v, _) = arbitrary_axis(first.normal);
    WorkPlane {
        origin: first.plane_point,
        u,
        v,
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
/// This is [`cad_geometry::bulged_ring_moments`]'s first moment divided by its
/// area: the straight-edge polygon integral plus one circular segment per bulge
/// (whose first moments are the exact `sector − triangle` expressions). There is
/// exactly **one** implementation of that arithmetic, in `cad-geometry`, so the
/// centroid and the second moments reported next to it cannot drift apart.
fn bulged_ring_centroid(plane: WorkPlane, flat: &[[f64; 2]], bulges: &[f64]) -> Option<Point3> {
    let [x, y] = cad_geometry::bulged_ring_moments(flat, bulges)?.centroid()?;
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

/// Refuse a second-moment report whose accumulated tensor is not a real region's.
///
/// Every exact path already refuses a zero/near-zero area before this point; this
/// code exists so a numerically impossible tensor (non-finite, negative, or a
/// product of inertia beyond `sqrt(Ix·Iy)`) is named rather than printed.
fn mass_inertia_degenerate(reason: &str) -> CadError {
    CadError::InvalidInput(format!(
        "massprop.inertia_degenerate: {reason}; no second moment is reported"
    ))
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
