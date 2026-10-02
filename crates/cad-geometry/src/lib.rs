//! f64 geometry; display tolerances never control measurement semantics.
//!
//! Spec v2.0 §16.1: there is no single global epsilon. Operations take the
//! [`TolerancePolicy`] and never confuse display discretisation with the
//! geometric predicates used for closure, area or snapping.
//!
//! The OpenCADStudio tessellation path (spec §10.2) is realised by
//! [`GeometryEngine::tessellate_curve`] and [`mesh`]. Kernel and codec types
//! stay outside this crate.

pub mod area;
pub mod clip;
pub mod mesh;
pub mod nurbs;

pub use area::{measure_polygon_area, signed_area, AreaError};
pub use clip::{clip_polyline_to_xy_rect, clip_segment_to_xy_rect};
pub use hatch::{
    fill_rings, pattern_polylines, simplify, triangulate, FillError, FillMesh, Loop, PatternLine,
    MAX_FILL_POINTS, MAX_FILL_TRIANGLES,
};
pub use mesh::{compute_vertex_normals, mesh_bounds};
pub use nurbs::{clamped_uniform_knots, NurbsCurve};

pub mod hatch;

use cad_domain::*;

/// Validate that a [`WorkPlane`] is a usable orthonormal frame.
///
/// A measurement work plane must have finite, non-degenerate and mutually
/// orthogonal `u`/`v`; a skewed or non-finite basis silently distorts projected
/// area, so callers must reject it (audit B24).
pub fn validate_work_plane(plane: &WorkPlane, tolerance: f64) -> CadResult<()> {
    if !is_finite(plane.origin) || !is_finite(plane.u) || !is_finite(plane.v) {
        return Err(CadError::InvalidInput(
            "work plane has non-finite values".to_string(),
        ));
    }
    let lu = length(plane.u);
    let lv = length(plane.v);
    let tol = tolerance.max(1e-12);
    if lu < tol || lv < tol {
        return Err(CadError::InvalidInput(
            "work plane basis vectors are degenerate".to_string(),
        ));
    }
    let ortho = dot(plane.u, plane.v).abs() / (lu * lv);
    if ortho > tol.max(1e-9) {
        return Err(CadError::InvalidInput(
            "work plane basis is not orthogonal".to_string(),
        ));
    }
    Ok(())
}

/// Unsigned distance of `p` from the plane through `origin` with unit `normal`.
pub fn distance_to_plane(p: Point3, origin: Point3, normal: Point3) -> f64 {
    let n = normalize(normal);
    dot(sub(p, origin), n).abs()
}

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

/// Discretise any curve-like geometry into a point polyline.
///
/// Meshes, inserts and opaque payloads yield an empty vector: they are handled
/// by the representation layer, not as polylines.
pub fn tessellate_geometry(geometry: &SemanticGeometry, params: TessellationParams) -> Vec<Point3> {
    use SemanticGeometry as G;
    match geometry {
        G::Line { start, end } => vec![*start, *end],
        G::Polyline {
            points,
            bulges,
            closed,
        } => polyline_with_bulges(points, bulges, *closed, params),
        G::Circle {
            center,
            normal,
            radius,
        } => {
            let (ax, ay, _) = arbitrary_axis(*normal);
            let mut out = Vec::new();
            let n = arc_segments_for_tolerance(*radius, std::f64::consts::TAU, params);
            for i in 0..=n {
                let t = std::f64::consts::TAU * (i as f64) / (n as f64);
                out.push(add(
                    *center,
                    add(scale(ax, t.cos() * radius), scale(ay, t.sin() * radius)),
                ));
            }
            out
        }
        G::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            let (ax, ay, _) = arbitrary_axis(*normal);
            // Preserve the sweep's sign: a reflected/clockwise arc (for example
            // from a mirror) must not be re-drawn as its complement (audit B23).
            let sweep = if sweep.abs() >= std::f64::consts::TAU - 1e-9 {
                std::f64::consts::TAU
            } else {
                *sweep
            };
            let n = arc_segments_for_tolerance(*radius, sweep, params);
            let mut out = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let t = start + sweep * (i as f64) / (n as f64);
                out.push(add(
                    *center,
                    add(scale(ax, t.cos() * radius), scale(ay, t.sin() * radius)),
                ));
            }
            out
        }
        G::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            let major_len = length(*major_axis);
            if major_len < 1e-12 {
                return vec![*center];
            }
            let u = scale(*major_axis, 1.0 / major_len);
            // The minor axis lies in the ellipse's own plane, defined by the
            // stored extrusion normal: `minor = cross(normal, major)` (audit
            // B23). This keeps an ellipse on an arbitrary OCS plane in that
            // plane instead of folding it onto world XY.
            let minor = scale(ellipse_minor_dir(*normal, u), major_len * ratio.abs());
            let sweep = if sweep.abs() >= std::f64::consts::TAU - 1e-9 {
                std::f64::consts::TAU
            } else {
                *sweep
            };
            let r = major_len.max(major_len * ratio.abs());
            let n = arc_segments_for_tolerance(r, sweep, params);
            let mut out = Vec::with_capacity(n + 1);
            for i in 0..=n {
                let t = start + sweep * (i as f64) / (n as f64);
                out.push(add(
                    *center,
                    add(scale(*major_axis, t.cos()), scale(minor, t.sin())),
                ));
            }
            out
        }
        G::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => tessellate_spline(control_points, knots, weights, *degree, params),
        G::Point(p) => vec![*p, *p],
        G::Text {
            position,
            height,
            text,
            ..
        } => {
            // A text placeholder box edge, sufficient for picking bounds.
            let h = height.abs().max(1e-9);
            vec![
                *position,
                Point3 {
                    x: position.x + h * text.chars().count() as f64,
                    y: position.y + h,
                    z: position.z,
                },
            ]
        }
        G::Mesh(_) | G::Insert { .. } | G::Opaque { .. } => Vec::new(),
        G::Compound(children) => {
            let mut out = Vec::new();
            for child in children {
                out.extend(tessellate_geometry(child, params));
            }
            out
        }
    }
}

/// Segments needed so an arc keeps its sagitta under the tolerance.
pub fn arc_segments_for_tolerance(radius: f64, sweep: f64, params: TessellationParams) -> usize {
    let sweep = sweep.abs();
    if sweep <= 1e-12 {
        return 1;
    }
    let radius = radius.abs().max(1e-12);
    let ratio = 1.0 - (params.tolerance / radius);
    let step = if ratio <= -1.0 {
        std::f64::consts::FRAC_PI_2
    } else {
        2.0 * ratio.clamp(-1.0, 1.0).acos()
    };
    let step = step.max(1e-6);
    let n = (sweep / step).ceil() as usize;
    n.clamp(params.min_segments.max(1), params.max_segments.max(1))
}

fn polyline_with_bulges(
    points: &[Point3],
    bulges: &[f64],
    closed: bool,
    params: TessellationParams,
) -> Vec<Point3> {
    if points.is_empty() {
        return Vec::new();
    }
    // Bulge arcs lie in the polyline's own plane, which need not be world XY
    // (an OCS/tilted polyline from the importer). Derive that plane from the
    // vertices so a tilted bulge is not silently flattened (audit B23).
    let plane_normal = polyline_plane_normal(points);
    let mut out: Vec<Point3> = Vec::with_capacity(points.len() * 4);
    let count = points.len();
    let last = if closed {
        count
    } else {
        count.saturating_sub(1)
    };
    for i in 0..last {
        let a = points[i];
        let b = points[(i + 1) % count];
        let bulge = bulges.get(i).copied().unwrap_or(0.0);
        // `a` is always the previous segment's endpoint, so the first point of
        // each segment is pushed by whichever branch runs (audit B23).
        if bulge.abs() < 1e-12 {
            if out.is_empty() {
                out.push(a);
            }
            out.push(b);
        } else {
            append_bulge_arc(a, b, bulge, plane_normal, params, &mut out);
        }
    }
    out
}

/// The best-fit plane normal of a polyline, using Newell's method.
///
/// The closing edge is always included so the polygon normal is well defined
/// even for an open polyline. Falls back to world Z for degenerate input
/// (fewer than three points, or collinear points).
fn polyline_plane_normal(points: &[Point3]) -> Point3 {
    let world_z = Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    let n = points.len();
    if n < 3 {
        return world_z;
    }
    let mut acc = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        acc.x += (a.y - b.y) * (a.z + b.z);
        acc.y += (a.z - b.z) * (a.x + b.x);
        acc.z += (a.x - b.x) * (a.y + b.y);
    }
    if length(acc) < 1e-12 {
        world_z
    } else {
        normalize(acc)
    }
}

/// Append the arc described by a bulge (`bulge = tan(theta/4)`).
///
/// The point `a` is always emitted first so the returned polyline is
/// continuous even when a straight segment precedes this arc (audit B23). The
/// arc is computed in the polyline's own plane (`plane_normal`); world-Z
/// polylines reduce to the original XY math.
fn append_bulge_arc(
    a: Point3,
    b: Point3,
    bulge: f64,
    plane_normal: Point3,
    params: TessellationParams,
    out: &mut Vec<Point3>,
) {
    let chord = sub(b, a);
    let chord_len = length(chord);
    if chord_len < 1e-12 {
        if !ends_at(out, b) {
            out.push(b);
        }
        return;
    }
    let theta = 4.0 * bulge.atan();
    let half = theta * 0.5;
    let half_chord = chord_len * 0.5;
    let sagitta = bulge * half_chord;
    if sagitta.abs() < 1e-12 {
        if !ends_at(out, a) {
            out.push(a);
        }
        out.push(b);
        return;
    }
    let radius_abs = (half_chord * half_chord + sagitta * sagitta) / (2.0 * sagitta.abs());
    // Orthonormal in-plane axes; `a` is the 2D origin, the chord is `d2`.
    let (ax, ay, _) = arbitrary_axis(plane_normal);
    let d2 = [dot(chord, ax), dot(chord, ay)];
    let chord2_len = (d2[0] * d2[0] + d2[1] * d2[1]).sqrt();
    if chord2_len < 1e-12 {
        // The chord is parallel to the plane normal: no in-plane arc exists.
        if !ends_at(out, a) {
            out.push(a);
        }
        out.push(b);
        return;
    }
    let dir2 = [d2[0] / chord2_len, d2[1] / chord2_len];
    let left2 = [-dir2[1], dir2[0]];
    let u2 = if sagitta >= 0.0 {
        left2
    } else {
        [-left2[0], -left2[1]]
    };
    let mid2 = [d2[0] * 0.5, d2[1] * 0.5];
    let center2 = [
        mid2[0] - u2[0] * radius_abs * half.cos(),
        mid2[1] - u2[1] * radius_abs * half.cos(),
    ];
    // `a` is the origin, so its angle is measured against the reflected centre.
    let start_angle = (-center2[1]).atan2(-center2[0]);
    let n = arc_segments_for_tolerance(radius_abs, theta, params);
    if !ends_at(out, a) {
        out.push(a);
    }
    for i in 1..=n {
        let t = start_angle + theta * (i as f64) / (n as f64);
        let px = center2[0] + t.cos() * radius_abs;
        let py = center2[1] + t.sin() * radius_abs;
        out.push(add(a, add(scale(ax, px), scale(ay, py))));
    }
    // Snap the final sample exactly onto the arc endpoint so a polyline that
    // closes on itself shares the vertex exactly (audit B23).
    if let Some(last) = out.last_mut() {
        *last = b;
    }
}

/// Whether the polyline `out` already ends at `p` (within a small tolerance).
fn ends_at(out: &[Point3], p: Point3) -> bool {
    out.last().map(|l| distance(*l, p) < 1e-9).unwrap_or(false)
}

/// Tessellate a clamped uniform B-spline from control points.
///
/// Convenience wrapper used by hatch boundaries that carry no source knots;
/// it synthesises clamped uniform knots. Source splines with explicit knots or
/// weights go through [`tessellate_spline`] instead (audit B23).
pub fn tessellate_bspline(
    control: &[Point3],
    degree: u32,
    params: TessellationParams,
) -> Vec<Point3> {
    let n = control.len();
    if n == 0 {
        return Vec::new();
    }
    let k = (degree as usize).max(1).min(n.saturating_sub(1));
    let knots = clamped_uniform_knots(n, k);
    let weights = vec![1.0; n];
    tessellate_spline(control, &knots, &weights, degree, params)
}

/// Tessellate a rational B-spline using its source knots and weights.
///
/// The source knot vector and weights are authoritative: uniform/periodic/
/// clamped and rational arcs all keep their true shape, and the sampling is
/// adaptive to the chord tolerance. Malformed input (wrong-length or
/// non-monotone knots, bad degree) falls back to the clamped-uniform
/// convention rather than silently producing a garbled curve (audit B23).
pub fn tessellate_spline(
    control: &[Point3],
    knots: &[f64],
    weights: &[f64],
    degree: u32,
    params: TessellationParams,
) -> Vec<Point3> {
    let n = control.len();
    if n == 0 {
        return Vec::new();
    }
    let k = (degree as usize).max(1);
    if n <= k {
        return control.to_vec();
    }
    let curve =
        NurbsCurve::new(k, control.to_vec(), knots.to_vec(), weights.to_vec()).or_else(|_| {
            NurbsCurve::new(
                k.min(n - 1),
                control.to_vec(),
                clamped_uniform_knots(n, k.min(n - 1)),
                if weights.len() == n {
                    weights.to_vec()
                } else {
                    Vec::new()
                },
            )
        });
    match curve {
        Ok(curve) => curve.discretize(params.tolerance, &params),
        Err(_) => control.to_vec(),
    }
}

/// The AutoCAD arbitrary axis algorithm.
pub fn arbitrary_axis(normal: Point3) -> (Point3, Point3, Point3) {
    let n = if length(normal) < 1e-24 {
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        }
    } else {
        normalize(normal)
    };
    const ONE_64TH: f64 = 1.0 / 64.0;
    let ax = if n.x.abs() < ONE_64TH && n.y.abs() < ONE_64TH {
        cross(
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            n,
        )
    } else {
        cross(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            n,
        )
    };
    let ax = if length(ax) < 1e-24 {
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        }
    } else {
        normalize(ax)
    };
    let ay = normalize(cross(n, ax));
    (ax, ay, n)
}

/// Accumulates an axis-aligned bounding box.
pub struct BoundsAccumulator {
    min: Point3,
    max: Point3,
    any: bool,
}

impl Default for BoundsAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl BoundsAccumulator {
    pub fn new() -> Self {
        BoundsAccumulator {
            min: Point3 {
                x: f64::INFINITY,
                y: f64::INFINITY,
                z: f64::INFINITY,
            },
            max: Point3 {
                x: f64::NEG_INFINITY,
                y: f64::NEG_INFINITY,
                z: f64::NEG_INFINITY,
            },
            any: false,
        }
    }

    pub fn add_point(&mut self, p: Point3) {
        if !is_finite(p) {
            return;
        }
        self.min = Point3 {
            x: self.min.x.min(p.x),
            y: self.min.y.min(p.y),
            z: self.min.z.min(p.z),
        };
        self.max = Point3 {
            x: self.max.x.max(p.x),
            y: self.max.y.max(p.y),
            z: self.max.z.max(p.z),
        };
        self.any = true;
    }

    pub fn add_geometry(&mut self, geometry: &SemanticGeometry) {
        for p in tessellate_geometry(
            geometry,
            TessellationParams {
                tolerance: 0.1,
                max_segments: 256,
                min_segments: 4,
            },
        ) {
            self.add_point(p);
        }
        if let SemanticGeometry::Mesh(m) = geometry {
            for p in &m.vertices {
                self.add_point(*p);
            }
        }
    }

    pub fn union_point(&mut self, p: Point3) {
        self.add_point(p);
    }

    pub fn finish(&self) -> Option<Bounds3> {
        if self.any {
            Some(Bounds3 {
                min: self.min,
                max: self.max,
            })
        } else {
            None
        }
    }
}

// ---- Point3 helpers (kept local so no external math crate leaks) ----

pub fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

pub fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

pub fn scale(a: Point3, s: f64) -> Point3 {
    Point3 {
        x: a.x * s,
        y: a.y * s,
        z: a.z * s,
    }
}

pub fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

pub fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

pub fn length(a: Point3) -> f64 {
    dot(a, a).sqrt()
}

pub fn distance(a: Point3, b: Point3) -> f64 {
    length(sub(a, b))
}

pub fn normalize(a: Point3) -> Point3 {
    let l = length(a);
    if l < 1e-24 {
        a
    } else {
        scale(a, 1.0 / l)
    }
}

pub fn is_finite(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

/// Whether every scalar and point in `geometry` is finite.
///
/// Used after a transform to refuse poisoned output (audit B12/B23).
pub fn geometry_is_finite(geometry: &SemanticGeometry) -> bool {
    use SemanticGeometry as G;
    let finite = |p: Point3| is_finite(p);
    let all_points = |pts: &[Point3]| pts.iter().all(|p| finite(*p));
    match geometry {
        G::Line { start, end } => finite(*start) && finite(*end),
        G::Polyline { points, bulges, .. } => {
            all_points(points) && bulges.iter().all(|b| b.is_finite())
        }
        G::Circle {
            center,
            normal,
            radius,
        } => finite(*center) && finite(*normal) && radius.is_finite(),
        G::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => {
            finite(*center)
                && finite(*normal)
                && radius.is_finite()
                && start.is_finite()
                && sweep.is_finite()
        }
        G::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            finite(*center)
                && finite(*normal)
                && finite(*major_axis)
                && ratio.is_finite()
                && start.is_finite()
                && sweep.is_finite()
        }
        G::Spline {
            knots,
            control_points,
            weights,
            ..
        } => {
            all_points(control_points)
                && knots.iter().all(|k| k.is_finite())
                && weights.iter().all(|w| w.is_finite())
        }
        G::Point(p) => finite(*p),
        G::Mesh(m) => {
            all_points(&m.vertices)
                && m.normals.iter().all(|n| finite(*n))
                && m.triangles
                    .iter()
                    .all(|t| t.iter().all(|i| (*i as usize) < m.vertices.len()))
        }
        G::Insert { transform, .. } => transform.matrix.iter().flatten().all(|v| v.is_finite()),
        G::Text {
            position,
            height,
            rotation,
            ..
        } => finite(*position) && height.is_finite() && rotation.is_finite(),
        G::Opaque { .. } => true,
        G::Compound(children) => children.iter().all(geometry_is_finite),
    }
}

fn apply_point(t: &Transform3, p: Point3) -> Point3 {
    let m = &t.matrix;
    Point3 {
        x: m[0][0] * p.x + m[0][1] * p.y + m[0][2] * p.z + m[0][3],
        y: m[1][0] * p.x + m[1][1] * p.y + m[1][2] * p.z + m[1][3],
        z: m[2][0] * p.x + m[2][1] * p.y + m[2][2] * p.z + m[2][3],
    }
}

fn apply_vector(t: &Transform3, v: Point3) -> Point3 {
    let m = &t.matrix;
    Point3 {
        x: m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
        y: m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
        z: m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
    }
}

fn uniform_scale(t: &Transform3) -> f64 {
    // Mean column length; used only after `is_uniform` has accepted the matrix.
    let m = &t.matrix;
    let sx = (m[0][0] * m[0][0] + m[1][0] * m[1][0] + m[2][0] * m[2][0]).sqrt();
    let sy = (m[0][1] * m[0][1] + m[1][1] * m[1][1] + m[2][1] * m[2][1]).sqrt();
    let sz = (m[0][2] * m[0][2] + m[1][2] * m[1][2] + m[2][2] * m[2][2]).sqrt();
    ((sx + sy + sz) / 3.0).max(1e-12)
}

/// Whether a transform keeps circles circular (similarity, no shear).
fn is_uniform(t: &Transform3) -> bool {
    t.is_uniform_scale(1e-9)
}

fn world_z_axis() -> Point3 {
    Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    }
}

/// A mirror (negative determinant) reverses the OCS handedness, so a traced
/// curve keeps its image only if its parameter range is reversed as well.
fn oriented_after_reflection(t: &Transform3, start: f64, sweep: f64) -> (f64, f64) {
    if t.determinant() < 0.0 {
        (-start, -sweep)
    } else {
        (start, sweep)
    }
}

/// Unit minor-axis direction for an ellipse with plane normal `normal` and unit
/// major axis `major_unit`: the in-plane `+90°` rotation `cross(normal, major)`.
fn ellipse_minor_dir(normal: Point3, major_unit: Point3) -> Point3 {
    let n = normalize(normal);
    let m = normalize(major_unit);
    let minor = cross(n, m);
    if length(minor) >= 1e-9 {
        return normalize(minor);
    }
    // The normal is parallel to the major axis: the ellipse is degenerate. Pick
    // any perpendicular so callers still get a usable frame.
    let fallback = cross(world_z_axis(), m);
    if length(fallback) >= 1e-9 {
        normalize(fallback)
    } else {
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        }
    }
}

/// Map a circle through an affine transform as an exact ellipse.
fn circle_to_geometry(
    center: Point3,
    normal: Point3,
    radius: f64,
    t: &Transform3,
) -> SemanticGeometry {
    let radius = radius.abs();
    if radius < 1e-12 {
        return SemanticGeometry::Point(apply_point(t, center));
    }
    let n = normalize(normal);
    let (ax, _, _) = arbitrary_axis(n);
    affine_ellipse_arc(
        center,
        n,
        scale(ax, radius),
        1.0,
        0.0,
        std::f64::consts::TAU,
        t,
    )
}

/// Map a circular arc through an affine transform as an exact elliptical arc.
fn arc_to_geometry(
    center: Point3,
    normal: Point3,
    radius: f64,
    start: f64,
    sweep: f64,
    t: &Transform3,
) -> SemanticGeometry {
    let radius = radius.abs();
    if radius < 1e-12 {
        return SemanticGeometry::Point(apply_point(t, center));
    }
    let n = normalize(normal);
    let (ax, _, _) = arbitrary_axis(n);
    affine_ellipse_arc(center, n, scale(ax, radius), 1.0, start, sweep, t)
}

/// The exact affine image of an elliptical arc.
///
/// An affine map sends the parameterisation
/// `E(t) = c + u·cos t + v·sin t` to `c' + (A u)·cos t + (A v)·sin t`. Writing
/// `M = [A u, A v]`, the 2×2 eigen-decomposition of `MᵀM` yields the principal
/// axes (`s1 ≥ s2`), a rotation angle `θ` and an orthonormal frame `(w1, w2)`
/// with `A u·cos t + A v·sin t = s1 w1·cos(t−θ) + s2 w2·sin(t−θ)`. The result is
/// reported in the domain convention `minor = cross(normal, major)`, with the
/// parameter shifted by `−θ` (audit B23).
#[allow(clippy::too_many_arguments)]
fn affine_ellipse_arc(
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
    start: f64,
    sweep: f64,
    t: &Transform3,
) -> SemanticGeometry {
    let a = length(major_axis);
    if a < 1e-12 {
        return SemanticGeometry::Point(apply_point(t, center));
    }
    let n = normalize(normal);
    let major_unit = scale(major_axis, 1.0 / a);
    let minor_vec = scale(ellipse_minor_dir(n, major_unit), a * ratio.abs().max(0.0));
    let u = apply_vector(t, major_axis);
    let v = apply_vector(t, minor_vec);
    let c = apply_point(t, center);

    let uu = dot(u, u);
    let vv = dot(v, v);
    let uv = dot(u, v);
    let trace = uu + vv;
    let diff = uu - vv;
    let disc = (diff * diff + 4.0 * uv * uv).sqrt();
    let lam1 = 0.5 * (trace + disc);
    let lam2 = (0.5 * (trace - disc)).max(0.0);
    let s1 = lam1.max(0.0).sqrt();
    let s2 = lam2.sqrt();
    if s1 < 1e-300 {
        // The whole plane collapsed to a point.
        return SemanticGeometry::Point(c);
    }
    // Eigenvector of the larger eigenvalue.
    let (ex, ey) = if uv.abs() > 1e-300 {
        let len = (uv * uv + (lam1 - uu) * (lam1 - uu)).sqrt();
        if len > 1e-300 {
            (uv / len, (lam1 - uu) / len)
        } else {
            (1.0, 0.0)
        }
    } else if uu >= vv {
        (1.0, 0.0)
    } else {
        (0.0, 1.0)
    };
    let (e2x, e2y) = (-ey, ex);
    let w1 = scale(add(scale(u, ex), scale(v, ey)), 1.0 / s1);
    let w2 = if s2 > 1e-300 {
        scale(add(scale(u, e2x), scale(v, e2y)), 1.0 / s2)
    } else {
        Point3::default()
    };
    let theta = ey.atan2(ex);
    let normal_out = if s2 > 1e-300 {
        let nn = cross(w1, w2);
        if length(nn) >= 1e-12 {
            normalize(nn)
        } else {
            affine_normal_fallback(t, n)
        }
    } else {
        affine_normal_fallback(t, n)
    };
    SemanticGeometry::Ellipse {
        center: c,
        normal: normal_out,
        major_axis: scale(w1, s1),
        ratio: s2 / s1,
        start: start - theta,
        sweep,
    }
}

fn affine_normal_fallback(t: &Transform3, n: Point3) -> Point3 {
    let mapped = apply_vector(t, n);
    if length(mapped) >= 1e-12 {
        normalize(mapped)
    } else {
        world_z_axis()
    }
}

/// Recover a bulge segment as an exact circular arc in the polyline's plane.
///
/// `bulge = tan(θ/4)`; the arc is the one bowing to the sign of the bulge. This
/// mirrors the tessellator's construction so the analytic and sampled forms are
/// the same curve (audit B23).
fn bulge_arc_geometry(
    a: Point3,
    b: Point3,
    bulge: f64,
    plane_normal: Point3,
) -> Option<SemanticGeometry> {
    let chord = sub(b, a);
    let chord_len = length(chord);
    if chord_len < 1e-12 {
        return None;
    }
    let theta = 4.0 * bulge.atan();
    let half = theta * 0.5;
    let half_chord = chord_len * 0.5;
    let sagitta = bulge * half_chord;
    if sagitta.abs() < 1e-12 {
        return None;
    }
    let radius_abs = (half_chord * half_chord + sagitta * sagitta) / (2.0 * sagitta.abs());
    let (ax, ay, _) = arbitrary_axis(plane_normal);
    let d2 = [dot(chord, ax), dot(chord, ay)];
    let chord2_len = (d2[0] * d2[0] + d2[1] * d2[1]).sqrt();
    if chord2_len < 1e-12 {
        // The chord is parallel to the plane normal: no in-plane arc exists.
        return None;
    }
    let dir2 = [d2[0] / chord2_len, d2[1] / chord2_len];
    let left2 = [-dir2[1], dir2[0]];
    let u2 = if sagitta >= 0.0 {
        left2
    } else {
        [-left2[0], -left2[1]]
    };
    let mid2 = [d2[0] * 0.5, d2[1] * 0.5];
    let center2 = [
        mid2[0] - u2[0] * radius_abs * half.cos(),
        mid2[1] - u2[1] * radius_abs * half.cos(),
    ];
    let start_angle = (-center2[1]).atan2(-center2[0]);
    let center = add(a, add(scale(ax, center2[0]), scale(ay, center2[1])));
    Some(SemanticGeometry::Arc {
        center,
        normal: plane_normal,
        radius: radius_abs,
        start: start_angle,
        sweep: theta,
    })
}

fn rotation_of(t: &Transform3) -> f64 {
    // Rotation about Z from the transformed X axis.
    t.matrix[1][0].atan2(t.matrix[0][0])
}

/// Exact or near-exact intersection of two 3D segments.
///
/// Works for segments in any plane (the old version projected onto XY and only
/// accepted lines that crossed there). `tol` is an absolute world tolerance.
fn segment_segment_intersection_3d(
    a1: Point3,
    a2: Point3,
    b1: Point3,
    b2: Point3,
    tol: f64,
) -> Option<Point3> {
    let d1 = sub(a2, a1);
    let d2 = sub(b2, b1);
    // Closest-point formula from Ericson, *Real-Time Collision Detection*:
    // r must go from the second segment's start to the first's.
    let r = sub(a1, b1);
    let aa = dot(d1, d1);
    let ee = dot(d2, d2);
    let bb = dot(d1, d2);
    let cc = dot(d1, r);
    let ff = dot(d2, r);
    let denom = aa * ee - bb * bb;
    let len_scale = (aa * ee).sqrt().max(1.0);
    let (s, t) = if denom.abs() > 1e-15 * len_scale {
        ((bb * ff - cc * ee) / denom, (aa * ff - bb * cc) / denom)
    } else {
        // Parallel: pick the projection of b1 onto a.
        let s = if aa > 0.0 { cc / aa } else { 0.0 };
        let t = if ee > 0.0 { ff / ee } else { 0.0 };
        (s, t)
    };
    let slack = if len_scale > 0.0 {
        (tol.max(1e-12) / len_scale.sqrt()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    if s < -slack || s > 1.0 + slack || t < -slack || t > 1.0 + slack {
        return None;
    }
    let p = add(a1, scale(d1, s.clamp(0.0, 1.0)));
    let q = add(b1, scale(d2, t.clamp(0.0, 1.0)));
    if distance(p, q) <= tol.max(1e-12) {
        Some(scale(add(p, q), 0.5))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Analytic curve/curve intersection (audit B23/F06).
//
// Measurement and snapping must not depend on the display tessellation. Where
// both operands are conic primitives the intersection is solved on the analytic
// curve; anything else falls back to adaptive sampling at the world-space
// predicate tolerance.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Conic {
    Segment {
        a: Point3,
        b: Point3,
    },
    Circle {
        center: Point3,
        normal: Point3,
        radius: f64,
    },
    Arc {
        center: Point3,
        normal: Point3,
        radius: f64,
        start: f64,
        sweep: f64,
    },
    Ellipse {
        center: Point3,
        normal: Point3,
        major_axis: Point3,
        ratio: f64,
        start: f64,
        sweep: f64,
    },
}

fn conic_of(geometry: &SemanticGeometry) -> Option<Conic> {
    match geometry {
        SemanticGeometry::Line { start, end } => Some(Conic::Segment { a: *start, b: *end }),
        SemanticGeometry::Circle {
            center,
            normal,
            radius,
        } => Some(Conic::Circle {
            center: *center,
            normal: *normal,
            radius: radius.abs(),
        }),
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => Some(Conic::Arc {
            center: *center,
            normal: *normal,
            radius: radius.abs(),
            start: *start,
            sweep: *sweep,
        }),
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => Some(Conic::Ellipse {
            center: *center,
            normal: *normal,
            major_axis: *major_axis,
            ratio: ratio.abs(),
            start: *start,
            sweep: *sweep,
        }),
        _ => None,
    }
}

/// Whether a parameter angle lies on the (possibly signed) arc.
fn angle_on_arc(angle: f64, start: f64, sweep: f64, ang_tol: f64) -> bool {
    let tau = std::f64::consts::TAU;
    if sweep.abs() >= tau - 1e-9 {
        return true;
    }
    let norm = |mut x: f64| {
        x %= tau;
        if x < 0.0 {
            x += tau;
        }
        x
    };
    let s = norm(start);
    let a = norm(angle);
    if sweep >= 0.0 {
        norm(a - s) <= sweep + ang_tol
    } else {
        norm(s - a) <= -sweep + ang_tol
    }
}

fn push_unique(out: &mut Vec<Point3>, p: Point3, tol: f64) {
    if !out.iter().any(|q| distance(*q, p) <= tol) {
        out.push(p);
    }
}

fn segment_circle_points(
    a: Point3,
    b: Point3,
    center: Point3,
    normal: Point3,
    radius: f64,
    tol: f64,
) -> Vec<Point3> {
    let mut out = Vec::new();
    let n = normalize(normal);
    let d = sub(b, a);
    let dn = dot(d, n);
    let ac = sub(a, center);
    let a_off = dot(ac, n);
    if dn.abs() > tol {
        // The segment pierces the circle's plane at a single parameter.
        let t = -a_off / dn;
        if t >= -tol && t <= 1.0 + tol {
            let p = add(a, scale(d, t.clamp(0.0, 1.0)));
            if (distance(p, center) - radius).abs() <= tol {
                push_unique(&mut out, p, tol);
            }
        }
        return out;
    }
    // Parallel to the plane: only an in-plane segment can meet the circle.
    if a_off.abs() > tol {
        return out;
    }
    let qa = dot(d, d);
    if qa <= tol * tol {
        if (distance(a, center) - radius).abs() <= tol {
            push_unique(&mut out, a, tol);
        }
        return out;
    }
    let qb = 2.0 * dot(ac, d);
    let qc = dot(ac, ac) - radius * radius;
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 {
        return out;
    }
    let sq = disc.sqrt();
    for t in [(-qb - sq) / (2.0 * qa), (-qb + sq) / (2.0 * qa)] {
        if (-tol..=1.0 + tol).contains(&t) {
            push_unique(&mut out, add(a, scale(d, t.clamp(0.0, 1.0))), tol);
        }
    }
    out
}

fn circle_circle_points(
    c1: Point3,
    n1: Point3,
    r1: f64,
    c2: Point3,
    n2: Point3,
    r2: f64,
    tol: f64,
) -> Option<Vec<Point3>> {
    let u1 = normalize(n1);
    let u2 = normalize(n2);
    if length(cross(u1, u2)) > 1e-9 {
        // Non-parallel planes: not handled analytically (caller samples).
        return None;
    }
    if dot(sub(c2, c1), u1).abs() > tol {
        return None;
    }
    let mut out = Vec::new();
    let d_vec = sub(c2, c1);
    let d = length(d_vec);
    if d <= tol {
        // Concentric: either the same circle (infinitely many points) or none.
        return Some(out);
    }
    if d > r1 + r2 + tol || d < (r1 - r2).abs() - tol {
        return Some(out);
    }
    let aa = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let hh = r1 * r1 - aa * aa;
    if hh < -tol {
        return Some(out);
    }
    let h = hh.max(0.0).sqrt();
    let dir = scale(d_vec, 1.0 / d);
    let base = add(c1, scale(dir, aa));
    let perp = normalize(cross(dir, u1));
    push_unique(&mut out, add(base, scale(perp, h)), tol);
    if h > tol {
        push_unique(&mut out, sub(base, scale(perp, h)), tol);
    }
    Some(out)
}

#[allow(clippy::too_many_arguments)]
fn segment_ellipse_points(
    a: Point3,
    b: Point3,
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
    start: f64,
    sweep: f64,
    tol: f64,
) -> Vec<Point3> {
    let mut out = Vec::new();
    let n = normalize(normal);
    let ma = length(major_axis);
    if ma < 1e-12 {
        return out;
    }
    let major_unit = scale(major_axis, 1.0 / ma);
    let mb = ma * ratio;
    let minor_dir = ellipse_minor_dir(n, major_unit);
    let d = sub(b, a);
    let ac = sub(a, center);
    let du = dot(d, major_unit);
    let dv = dot(d, minor_dir);
    let au = dot(ac, major_unit);
    let av = dot(ac, minor_dir);
    if mb < 1e-12 {
        // Degenerate ellipse (a segment): the minor coordinate must be zero.
        if dv.abs() > tol {
            return out;
        }
        for u in [-ma, ma] {
            let t = (u - au) / du;
            if !(-tol..=1.0 + tol).contains(&t) {
                continue;
            }
            let p = add(a, scale(d, t.clamp(0.0, 1.0)));
            if dot(sub(p, center), n).abs() <= tol {
                push_unique(&mut out, p, tol);
            }
        }
        return out;
    }
    let qa = (du / ma).powi(2) + (dv / mb).powi(2);
    if qa <= 1e-300 {
        return out;
    }
    let qb = 2.0 * (au * du / (ma * ma) + av * dv / (mb * mb));
    let qc = (au / ma).powi(2) + (av / mb).powi(2) - 1.0;
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 {
        return out;
    }
    let sq = disc.sqrt();
    let ang_tol = (tol / ma.max(mb)).max(1e-12);
    for t in [(-qb - sq) / (2.0 * qa), (-qb + sq) / (2.0 * qa)] {
        if !(-tol..=1.0 + tol).contains(&t) {
            continue;
        }
        let p = add(a, scale(d, t.clamp(0.0, 1.0)));
        if dot(sub(p, center), n).abs() > tol {
            continue;
        }
        let u = dot(sub(p, center), major_unit) / ma;
        let v = dot(sub(p, center), minor_dir) / mb;
        if angle_on_arc(v.atan2(u), start, sweep, ang_tol) {
            push_unique(&mut out, p, tol);
        }
    }
    out
}

/// Analytic intersection when both operands are conic primitives.
///
/// Returns `None` when the pair is not solved analytically, so the caller can
/// fall back to sampling.
fn analytic_intersections(a: &Conic, b: &Conic, tol: f64) -> Option<Vec<Point3>> {
    use Conic::*;
    let mut out = Vec::new();
    match (a, b) {
        (Segment { a: a1, b: a2 }, Segment { a: b1, b: b2 }) => {
            if let Some(p) = segment_segment_intersection_3d(*a1, *a2, *b1, *b2, tol) {
                out.push(p);
            }
            Some(out)
        }
        (
            Segment { a: a1, b: a2 },
            Circle {
                center,
                normal,
                radius,
            },
        )
        | (
            Circle {
                center,
                normal,
                radius,
            },
            Segment { a: a1, b: a2 },
        ) => Some(segment_circle_points(
            *a1, *a2, *center, *normal, *radius, tol,
        )),
        (
            Segment { a: a1, b: a2 },
            Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            },
        )
        | (
            Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            },
            Segment { a: a1, b: a2 },
        ) => {
            let hits = segment_circle_points(*a1, *a2, *center, *normal, *radius, tol);
            let (ax, ay, _) = arbitrary_axis(*normal);
            let ang_tol = (tol / radius.abs().max(1e-12)).max(1e-12);
            for p in hits {
                let v = sub(p, *center);
                if angle_on_arc(dot(v, ay).atan2(dot(v, ax)), *start, *sweep, ang_tol) {
                    push_unique(&mut out, p, tol);
                }
            }
            Some(out)
        }
        (
            Segment { a: a1, b: a2 },
            Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            },
        )
        | (
            Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                start,
                sweep,
            },
            Segment { a: a1, b: a2 },
        ) => Some(segment_ellipse_points(
            *a1,
            *a2,
            *center,
            *normal,
            *major_axis,
            *ratio,
            *start,
            *sweep,
            tol,
        )),
        (
            Circle {
                center: c1,
                normal: n1,
                radius: r1,
            },
            Circle {
                center: c2,
                normal: n2,
                radius: r2,
            },
        ) => circle_circle_points(*c1, *n1, *r1, *c2, *n2, *r2, tol),
        (
            Circle {
                center: c1,
                normal: n1,
                radius: r1,
            },
            Arc {
                center: c2,
                normal: n2,
                radius: r2,
                start,
                sweep,
            },
        )
        | (
            Arc {
                center: c2,
                normal: n2,
                radius: r2,
                start,
                sweep,
            },
            Circle {
                center: c1,
                normal: n1,
                radius: r1,
            },
        ) => {
            let hits = circle_circle_points(*c1, *n1, *r1, *c2, *n2, *r2, tol)?;
            let (ax, ay, _) = arbitrary_axis(*n2);
            let ang_tol = (tol / r2.abs().max(1e-12)).max(1e-12);
            for p in hits {
                let v = sub(p, *c2);
                if angle_on_arc(dot(v, ay).atan2(dot(v, ax)), *start, *sweep, ang_tol) {
                    push_unique(&mut out, p, tol);
                }
            }
            Some(out)
        }
        (
            Arc {
                center: c1,
                normal: n1,
                radius: r1,
                start: s1,
                sweep: w1,
            },
            Arc {
                center: c2,
                normal: n2,
                radius: r2,
                start: s2,
                sweep: w2,
            },
        ) => {
            let hits = circle_circle_points(*c1, *n1, *r1, *c2, *n2, *r2, tol)?;
            let (ax1, ay1, _) = arbitrary_axis(*n1);
            let (ax2, ay2, _) = arbitrary_axis(*n2);
            for p in hits {
                let v1 = sub(p, *c1);
                let v2 = sub(p, *c2);
                let a1 = dot(v1, ay1).atan2(dot(v1, ax1));
                let a2 = dot(v2, ay2).atan2(dot(v2, ax2));
                if angle_on_arc(a1, *s1, *w1, 1e-9) && angle_on_arc(a2, *s2, *w2, 1e-9) {
                    push_unique(&mut out, p, tol);
                }
            }
            Some(out)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    #[test]
    fn line_bounds_are_exact() {
        let g = SemanticGeometry::Line {
            start: p(1.0, 2.0),
            end: p(5.0, -3.0),
        };
        let b = DefaultGeometryEngine.bounds(&g).unwrap();
        assert_eq!(b.min, p(1.0, -3.0));
        assert_eq!(b.max, p(5.0, 2.0));
    }

    #[test]
    fn circle_tessellation_respects_tolerance() {
        let g = SemanticGeometry::Circle {
            center: p(0.0, 0.0),
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            radius: 100.0,
        };
        let coarse = tessellate_geometry(
            &g,
            TessellationParams {
                tolerance: 1.0,
                max_segments: 4096,
                min_segments: 8,
            },
        );
        let fine = tessellate_geometry(
            &g,
            TessellationParams {
                tolerance: 0.01,
                max_segments: 4096,
                min_segments: 8,
            },
        );
        assert!(fine.len() > coarse.len());
        // All points lie on the circle.
        for q in &fine {
            assert!((length(*q) - 100.0).abs() < 1e-6);
        }
    }

    #[test]
    fn bulge_semicircle_hits_lower_apex_for_ccw() {
        let pts = vec![p(0.0, 0.0), p(2.0, 0.0)];
        let out = polyline_with_bulges(&pts, &[1.0, 0.0], false, TessellationParams::default());
        let apex = out.iter().find(|q| (q.x - 1.0).abs() < 1e-6).unwrap();
        assert!((apex.y + 1.0).abs() < 1e-6, "apex y = {}", apex.y);
    }

    #[test]
    fn transform_round_trips_translation() {
        let mut m = [[0.0f64; 4]; 4];
        m[0][0] = 1.0;
        m[1][1] = 1.0;
        m[2][2] = 1.0;
        m[3][3] = 1.0;
        m[0][3] = 10.0;
        m[1][3] = 20.0;
        let t = Transform3 { matrix: m };
        let g = SemanticGeometry::Point(p(1.0, 1.0));
        let out = DefaultGeometryEngine.transform(&g, &t).unwrap();
        assert_eq!(out, SemanticGeometry::Point(p(11.0, 21.0)));
    }

    #[test]
    fn ray_hits_work_plane() {
        let plane = WorkPlane {
            origin: p(0.0, 0.0),
            u: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            v: Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        };
        let ray = Ray3 {
            origin: Point3 {
                x: 0.0,
                y: 0.0,
                z: 5.0,
            },
            direction: Point3 {
                x: 0.0,
                y: 0.0,
                z: -1.0,
            },
        };
        let hit = DefaultGeometryEngine
            .ray_plane(&ray, &plane, &TolerancePolicy::default())
            .unwrap();
        assert!(hit.is_some());
        assert!(distance(hit.unwrap(), p(0.0, 0.0)) < 1e-9);
    }

    #[test]
    fn local_intersection_finds_crossing_lines() {
        let a = SemanticGeometry::Line {
            start: p(-1.0, 0.0),
            end: p(1.0, 0.0),
        };
        let b = SemanticGeometry::Line {
            start: p(0.0, -1.0),
            end: p(0.0, 1.0),
        };
        let hits = DefaultGeometryEngine
            .intersect_local(&a, &b, &TolerancePolicy::default())
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(distance(hits[0], p(0.0, 0.0)) < 1e-9);
    }
}
