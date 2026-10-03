//! engine module.

use super::*;

/// Curve and geometry operations over [`SemanticGeometry`].
pub trait GeometryEngine {
    fn transform(
        &self,
        geometry: &SemanticGeometry,
        transform: &Transform3,
    ) -> CadResult<SemanticGeometry>;
    fn bounds(&self, geometry: &SemanticGeometry) -> CadResult<Bounds3>;
    fn tessellate_curve(
        &self,
        geometry: &SemanticGeometry,
        tolerance: &TolerancePolicy,
    ) -> CadResult<Vec<Point3>>;
    fn intersect_local(
        &self,
        a: &SemanticGeometry,
        b: &SemanticGeometry,
        tolerance: &TolerancePolicy,
    ) -> CadResult<Vec<Point3>>;
    fn ray_plane(
        &self,
        ray: &Ray3,
        plane: &WorkPlane,
        tolerance: &TolerancePolicy,
    ) -> CadResult<Option<Point3>>;
}

/// Parameters controlling curve discretisation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TessellationParams {
    /// Maximum allowed chord deviation in world units.
    pub tolerance: f64,
    pub max_segments: usize,
    pub min_segments: usize,
}

impl Default for TessellationParams {
    fn default() -> Self {
        TessellationParams {
            tolerance: 0.01,
            max_segments: 4096,
            min_segments: 8,
        }
    }
}

impl TessellationParams {
    /// Derive from the tolerance policy and the current zoom.
    ///
    /// The policy's `display_pixels` is a screen-space budget; `world_per_px`
    /// converts it to the world-space chord error used here.
    pub fn from_policy(policy: &TolerancePolicy, world_per_px: f64) -> Self {
        let tol = (policy.display_pixels * world_per_px).max(1e-12);
        TessellationParams {
            tolerance: tol,
            ..Self::default()
        }
    }
}

/// The production geometry engine.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultGeometryEngine;

impl GeometryEngine for DefaultGeometryEngine {
    fn transform(
        &self,
        geometry: &SemanticGeometry,
        t: &Transform3,
    ) -> CadResult<SemanticGeometry> {
        use SemanticGeometry as G;
        let tp = |p: Point3| apply_point(t, p);
        let td = |v: Point3| apply_vector(t, v);
        let ok = match geometry {
            G::Line { start, end } => G::Line {
                start: tp(*start),
                end: tp(*end),
            },
            G::Polyline {
                points,
                bulges,
                closed,
            } => {
                let has_bulge = bulges.iter().any(|b| b.abs() > 1e-12);
                if is_uniform(t) || !has_bulge {
                    // A similarity keeps a bulge a circular arc; a mirror flips
                    // which side of the chord it bows to, so the sign follows
                    // the determinant (audit B23).
                    let flip = t.determinant() < 0.0;
                    G::Polyline {
                        points: points.iter().map(|p| tp(*p)).collect(),
                        bulges: if flip {
                            bulges.iter().map(|b| -b).collect()
                        } else {
                            bulges.clone()
                        },
                        closed: *closed,
                    }
                } else {
                    // A non-uniform scale turns every bulge arc into an
                    // elliptical arc. Dropping the bulge would draw a straight
                    // segment, so the polyline becomes an explicit compound of
                    // its exact line and elliptical-arc segments (audit B23).
                    self.transform_polyline_affine(points, bulges, *closed, t)?
                }
            }
            G::Circle {
                center,
                normal,
                radius,
            } => {
                // A circle only stays a circle under a similarity transform;
                // under a non-uniform scale or shear it is a genuine ellipse
                // and must be reported as one rather than mis-drawn as a
                // circle (audit B23).
                if t.is_uniform_scale(1e-9) {
                    G::Circle {
                        center: tp(*center),
                        normal: normalize(td(*normal)),
                        radius: radius * uniform_scale(t),
                    }
                } else {
                    circle_to_geometry(*center, *normal, *radius, t)
                }
            }
            G::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            } => {
                if t.is_uniform_scale(1e-9) && t.determinant() > 0.0 {
                    // A proper similarity rotates the OCS frame: the arc stays
                    // an arc, but its start angle shifts by the in-plane
                    // rotation between the old and new `arbitrary_axis` frames
                    // (audit B23).
                    let n_out = normalize(td(*normal));
                    let (ax_in, _, _) = arbitrary_axis(*normal);
                    let (ax_out, ay_out, _) = arbitrary_axis(n_out);
                    let ax_img = td(ax_in);
                    let phi = dot(ax_img, ay_out).atan2(dot(ax_img, ax_out));
                    G::Arc {
                        center: tp(*center),
                        normal: n_out,
                        radius: radius * uniform_scale(t),
                        start: start + phi,
                        sweep: *sweep,
                    }
                } else {
                    // A mirror or a non-uniform affine maps the arc to an
                    // elliptical arc; map it through the general conic path
                    // (audit B23).
                    arc_to_geometry(*center, *normal, *radius, *start, *sweep, t)
                }
            }
            G::Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            } => {
                if t.is_uniform_scale(1e-9) {
                    // Similarity: the axis directions follow the linear map and
                    // the ratio is unchanged; a mirror flips the parameter
                    // direction.
                    let (start, sweep) = oriented_after_reflection(t, *start, *sweep);
                    G::Ellipse {
                        center: tp(*center),
                        normal: normalize(td(*normal)),
                        major_axis: td(*major_axis),
                        ratio: ratio.abs(),
                        start,
                        sweep,
                    }
                } else {
                    // General affine image: re-derive the principal axes so the
                    // result is still an ellipse with a true normal, never the
                    // old circle with an averaged radius (audit B23).
                    affine_ellipse_arc(
                        *center,
                        *normal,
                        *major_axis,
                        ratio.abs(),
                        *start,
                        *sweep,
                        t,
                    )
                }
            }
            G::Spline {
                degree,
                knots,
                control_points,
                weights,
            } => G::Spline {
                degree: *degree,
                // Source knots and weights are geometry: transforming a spline
                // must not re-invent them. A non-finite control point is
                // reported rather than silently produced (audit B23).
                knots: knots.clone(),
                control_points: control_points.iter().map(|p| tp(*p)).collect::<Vec<_>>(),
                weights: weights.clone(),
            },
            G::Point(p) => G::Point(tp(*p)),
            G::Mesh(m) => {
                let mut m2 = m.clone();
                m2.vertices = m.vertices.iter().map(|p| tp(*p)).collect();
                if is_uniform(t) {
                    m2.normals = m.normals.iter().map(|n| td(*n)).collect();
                } else {
                    // Normals must be recomputed under non-uniform scaling.
                    m2.normals = mesh::compute_vertex_normals(&m2);
                }
                G::Mesh(m2)
            }
            G::Insert {
                block,
                transform: inner,
            } => G::Insert {
                block: *block,
                transform: t.matrix_mul(inner),
            },
            G::Text {
                text,
                position,
                style,
                height,
                rotation,
                font,
                h_align,
                v_align,
            } => G::Text {
                text: text.clone(),
                position: tp(*position),
                style: *style,
                height: height * uniform_scale(t),
                rotation: rotation + rotation_of(t),
                font: font.clone(),
                h_align: *h_align,
                v_align: *v_align,
            },
            G::Shape {
                shape_name,
                code,
                position,
                size,
                rotation,
                font,
            } => G::Shape {
                shape_name: shape_name.clone(),
                code: *code,
                position: tp(*position),
                size: size * uniform_scale(t),
                rotation: rotation + rotation_of(t),
                font: font.clone(),
            },
            G::Opaque {
                type_key,
                version,
                payload,
            } => G::Opaque {
                type_key: type_key.clone(),
                version: *version,
                payload: payload.clone(),
            },
            G::Compound(children) => {
                let mut out = Vec::with_capacity(children.len());
                for child in children {
                    out.push(self.transform(child, t)?);
                }
                G::Compound(out)
            }
        };
        // A transform that maps a finite source to a non-finite point is not a
        // usable result; report it instead of returning poisoned geometry
        // (audit B12/B23).
        if !geometry_is_finite(&ok) {
            return Err(CadError::InvalidInput(
                "transform produced non-finite geometry".to_string(),
            ));
        }
        Ok(ok)
    }

    fn bounds(&self, geometry: &SemanticGeometry) -> CadResult<Bounds3> {
        let mut acc = BoundsAccumulator::new();
        acc.add_geometry(geometry);
        acc.finish().ok_or(CadError::InvalidInput(
            "geometry has no finite extent".to_string(),
        ))
    }

    fn tessellate_curve(
        &self,
        geometry: &SemanticGeometry,
        tolerance: &TolerancePolicy,
    ) -> CadResult<Vec<Point3>> {
        // Discretisation is presentation only: the chord tolerance is derived
        // from the world-space computation tolerance, never from the display
        // pixel budget, so a change of display LOD cannot change measurement or
        // snapping semantics (audit B23). Callers that want screen-relative
        // density use `TessellationParams::from_policy` with a zoom factor.
        let params = TessellationParams {
            tolerance: tolerance.computation_world.max(1e-12),
            ..Default::default()
        };
        Ok(tessellate_geometry(geometry, params))
    }

    fn intersect_local(
        &self,
        a: &SemanticGeometry,
        b: &SemanticGeometry,
        tolerance: &TolerancePolicy,
    ) -> CadResult<Vec<Point3>> {
        // The intersection is a measured point, so its tolerance must come from
        // the world-space predicate policy, not the display LOD (audit B23).
        let tol = tolerance.computation_world.max(1e-12);
        // For two conic primitives solve on the analytic curve first; this is
        // independent of any sampling and therefore of the display budget
        // (audit B23/F06).
        if let (Some(ca), Some(cb)) = (conic_of(a), conic_of(b)) {
            if let Some(mut hits) = analytic_intersections(&ca, &cb, tol) {
                let dedup = tolerance.topology_world.max(1e-12);
                let mut unique: Vec<Point3> = Vec::with_capacity(hits.len());
                for p in hits.drain(..) {
                    push_unique(&mut unique, p, dedup);
                }
                return Ok(unique);
            }
        }
        // The chord tolerance is deliberately coarse relative to the predicate
        // tolerance so a curved entity is approximated by segments while the
        // final acceptance test stays geometric.
        let chord = tolerance.computation_world.max(1e-9);
        let pa = tessellate_geometry(
            a,
            TessellationParams {
                tolerance: chord,
                min_segments: 4,
                ..Default::default()
            },
        );
        let pb = tessellate_geometry(
            b,
            TessellationParams {
                tolerance: chord,
                min_segments: 4,
                ..Default::default()
            },
        );
        let mut out = Vec::new();
        for i in 0..pa.len().saturating_sub(1) {
            for j in 0..pb.len().saturating_sub(1) {
                if let Some(p) = segment_segment_intersection_3d(
                    pa[i],
                    pa[i + 1],
                    pb[j],
                    pb[j + 1],
                    tolerance.computation_world,
                ) {
                    if !out
                        .iter()
                        .any(|q: &Point3| distance(*q, p) < tolerance.topology_world.max(1e-9))
                    {
                        out.push(p);
                    }
                }
            }
        }
        Ok(out)
    }

    fn ray_plane(
        &self,
        ray: &Ray3,
        plane: &WorkPlane,
        tolerance: &TolerancePolicy,
    ) -> CadResult<Option<Point3>> {
        if !is_finite(plane.origin) || !is_finite(plane.u) || !is_finite(plane.v) {
            return Err(CadError::InvalidInput(
                "work plane has non-finite values".to_string(),
            ));
        }
        if !is_finite(ray.origin) || !is_finite(ray.direction) {
            return Err(CadError::InvalidInput(
                "ray has non-finite values".to_string(),
            ));
        }
        let normal = cross(plane.u, plane.v);
        let nlen = length(normal);
        if nlen < tolerance.computation_world.max(1e-12) {
            return Err(CadError::InvalidInput(
                "work plane basis is degenerate".to_string(),
            ));
        }
        let normal = scale(normal, 1.0 / nlen);
        // The ray only reaches behind the origin at t < 0; the direction's
        // magnitude is irrelevant, so judge (and parametrise) against the unit
        // direction (audit B24/B23).
        let dir_len = length(ray.direction);
        if dir_len < tolerance.computation_world.max(1e-12) {
            return Err(CadError::InvalidInput(
                "ray direction is degenerate".to_string(),
            ));
        }
        let dir = scale(ray.direction, 1.0 / dir_len);
        let denom = dot(dir, normal);
        if denom.abs() < tolerance.computation_world.max(1e-12) {
            return Ok(None);
        }
        let t = dot(sub(plane.origin, ray.origin), normal) / denom;
        if !t.is_finite() || t < 0.0 {
            return Ok(None);
        }
        Ok(Some(add(ray.origin, scale(dir, t))))
    }
}

impl DefaultGeometryEngine {
    /// Transform a bulge polyline through a non-uniform affine as an explicit
    /// compound of its exact line and elliptical-arc segments.
    ///
    /// Dropping the bulges would silently straighten the arcs, so each arc is
    /// reconstructed as a circular arc in the source plane and mapped through
    /// the general conic path (audit B23).
    fn transform_polyline_affine(
        &self,
        points: &[Point3],
        bulges: &[f64],
        closed: bool,
        t: &Transform3,
    ) -> CadResult<SemanticGeometry> {
        if points.is_empty() {
            return Ok(SemanticGeometry::Polyline {
                points: Vec::new(),
                bulges: Vec::new(),
                closed,
            });
        }
        let plane_normal = polyline_plane_normal(points);
        let count = points.len();
        let last = if closed {
            count
        } else {
            count.saturating_sub(1)
        };
        let mut children = Vec::with_capacity(last);
        for i in 0..last {
            let a = points[i];
            let b = points[(i + 1) % count];
            let bulge = bulges.get(i).copied().unwrap_or(0.0);
            let segment = if bulge.abs() <= 1e-12 {
                SemanticGeometry::Line { start: a, end: b }
            } else {
                bulge_arc_geometry(a, b, bulge, plane_normal)
                    .unwrap_or(SemanticGeometry::Line { start: a, end: b })
            };
            children.push(self.transform(&segment, t)?);
        }
        Ok(SemanticGeometry::Compound(children))
    }
}
