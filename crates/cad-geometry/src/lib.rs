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

pub use area::{measure_polygon_area, signed_area, AreaError};
pub use clip::{clip_polyline_to_xy_rect, clip_segment_to_xy_rect};
pub use hatch::{pattern_polylines, simplify, triangulate, Loop, PatternLine, MAX_FILL_POINTS};
pub use mesh::{compute_vertex_normals, mesh_bounds};

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
                // Non-uniform scaling turns bulge arcs into ellipses; rather
                // than silently mis-drawing them we keep the points and drop
                // bulge only when the transform is not uniform.
                let bulges = if is_uniform(t) {
                    bulges.clone()
                } else {
                    vec![0.0; points.len()]
                };
                G::Polyline {
                    points: points.iter().map(|p| tp(*p)).collect(),
                    bulges,
                    closed: *closed,
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
                    circle_to_ellipse(*center, *normal, *radius, t)
                }
            }
            G::Arc {
                center,
                normal,
                radius,
                start,
                sweep,
            } => {
                if t.is_uniform_scale(1e-9) {
                    G::Arc {
                        center: tp(*center),
                        normal: normalize(td(*normal)),
                        radius: radius * uniform_scale(t),
                        start: *start,
                        sweep: *sweep,
                    }
                } else {
                    let ellipse = circle_to_ellipse(*center, *normal, *radius, t);
                    match ellipse {
                        G::Ellipse {
                            center: ec,
                            major_axis,
                            ratio,
                            start: es,
                            sweep: _ew,
                        } => G::Ellipse {
                            center: ec,
                            major_axis,
                            ratio,
                            // `circle_to_ellipse` reports the parameter angle of
                            // the original circle's θ=0 image, so the arc's own
                            // start shifts by it and keeps its sweep.
                            start: es + *start,
                            sweep: *sweep,
                        },
                        other => other,
                    }
                }
            }
            G::Ellipse {
                center,
                major_axis,
                ratio,
                start,
                sweep,
            } => {
                // Keep the ellipse's own plane. The domain defines the minor
                // axis as `cross(world_z, major)`, so the transformed minor is
                // the transform of that direction (audit B23).
                let major = td(*major_axis);
                let major_len = length(major);
                let major_unit = normalize(*major_axis);
                let minor_unit = ellipse_minor_dir(major_unit);
                let minor = td(minor_unit);
                if major_len < 1e-12 {
                    G::Ellipse {
                        center: tp(*center),
                        major_axis: major,
                        ratio: *ratio,
                        start: *start,
                        sweep: *sweep,
                    }
                } else {
                    let ratio2 = ratio.abs() * (length(minor) / major_len);
                    G::Ellipse {
                        center: tp(*center),
                        major_axis: major,
                        ratio: ratio2,
                        start: *start,
                        sweep: *sweep,
                    }
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
            let sweep = normalize_sweep(*sweep);
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
            // documented convention `minor = cross(world_z, major)` (audit B23).
            let minor = scale(ellipse_minor_dir(u), major_len * ratio.abs());
            let sweep = if sweep.abs() >= std::f64::consts::TAU - 1e-9 {
                std::f64::consts::TAU
            } else {
                normalize_sweep(*sweep)
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

fn normalize_sweep(sweep: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let mut s = sweep % tau;
    if s <= 0.0 {
        s += tau;
    }
    s
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
/// The source knot vector is authoritative: uniform/periodic/clamped and
/// rational arcs all keep their true shape. Malformed knot vectors (wrong
/// length or non-monotone) fall back to the clamped-uniform convention rather
/// than silently producing a garbled curve.
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
    let k = (degree as usize).max(1).min(n.saturating_sub(1));
    if n <= k {
        return control.to_vec();
    }
    let owned_knots: Vec<f64>;
    let knots: &[f64] = if knots.len() == n + k + 1 && knots.windows(2).all(|w| w[1] >= w[0]) {
        knots
    } else {
        owned_knots = clamped_uniform_knots(n, k);
        &owned_knots
    };
    let weights: Vec<f64> = if weights.len() == n {
        weights
            .iter()
            .map(|w| if w.is_finite() && *w > 0.0 { *w } else { 1.0 })
            .collect()
    } else {
        vec![1.0; n]
    };
    let spans = n - k;
    let per_span = (params.max_segments / spans.max(1)).clamp(8, 64);
    let t_min = knots[k];
    let t_max = knots[n];
    if !t_min.is_finite() || !t_max.is_finite() || t_max <= t_min {
        return control.to_vec();
    }
    let mut out = Vec::with_capacity(spans * per_span + 1);
    for s in 0..=(spans * per_span) {
        let t = t_min + (t_max - t_min) * (s as f64) / ((spans * per_span) as f64);
        out.push(de_boor_rational(control, knots, &weights, k, t));
    }
    out
}

fn clamped_uniform_knots(n: usize, k: usize) -> Vec<f64> {
    let m = n + k + 1;
    let mut knots = vec![0.0; m];
    for i in 0..=k {
        knots[n + i] = 1.0;
    }
    let inner = n.saturating_sub(k + 1);
    for i in 1..=inner {
        knots[k + i] = i as f64 / (inner + 1) as f64;
    }
    knots
}

fn de_boor_rational(pts: &[Point3], knots: &[f64], weights: &[f64], k: usize, t: f64) -> Point3 {
    let n = pts.len();
    let mut span = k;
    while span < n - 1 && t >= knots[span + 1] {
        span += 1;
    }
    // Homogeneous coordinates (w*x, w*y, w*z, w); the rational point is the
    // perspective divide of the de Boor result.
    let hom = |j: usize| {
        let w = weights[span - k + j];
        let p = pts[span - k + j];
        [p.x * w, p.y * w, p.z * w, w]
    };
    let mut d: Vec<[f64; 4]> = (0..=k).map(hom).collect();
    for r in 1..=k {
        for j in (r..=k).rev() {
            let i = span - k + j;
            let denom = knots[i + k + 1 - r] - knots[i];
            let alpha = if denom.abs() > 1e-12 {
                (t - knots[i]) / denom
            } else {
                0.0
            };
            let prev = d[j - 1];
            let cur = d[j];
            let mut out = [0.0; 4];
            for c in 0..4 {
                out[c] = prev[c] * (1.0 - alpha) + cur[c] * alpha;
            }
            d[j] = out;
        }
    }
    let w = d[k][3];
    if w.abs() < 1e-12 {
        Point3 {
            x: d[k][0],
            y: d[k][1],
            z: d[k][2],
        }
    } else {
        Point3 {
            x: d[k][0] / w,
            y: d[k][1] / w,
            z: d[k][2] / w,
        }
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
            major_axis,
            ratio,
            start,
            sweep,
        } => {
            finite(*center)
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

/// Unit minor-axis direction for an ellipse with unit major axis `major_unit`.
fn ellipse_minor_dir(major_unit: Point3) -> Point3 {
    let minor = cross(
        Point3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        major_unit,
    );
    if length(minor) < 1e-9 {
        Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        }
    } else {
        normalize(minor)
    }
}

/// Map a circle through an affine transform as an exact ellipse.
///
/// The transformed plane is spanned by the images of the two in-plane basis
/// vectors; the longer image is the major axis. `ratio` is the axis ratio and
/// `start` is the parameter angle (in radians) of the original circle angle 0,
/// so the affine image of `circle(θ)` matches `ellipse(start + θ)`.
fn circle_to_ellipse(
    center: Point3,
    normal: Point3,
    radius: f64,
    t: &Transform3,
) -> SemanticGeometry {
    let radius = radius.abs();
    let n = normalize(normal);
    let (ax, ay, _) = arbitrary_axis(n);
    let ex = apply_vector(t, ax);
    let ey = apply_vector(t, ay);
    let c = apply_point(t, center);
    let lx = length(ex);
    let ly = length(ey);
    if lx < 1e-12 && ly < 1e-12 {
        // The plane collapsed under a singular/near-singular map.
        return SemanticGeometry::Point(c);
    }
    let (major_axis, ratio, phi) = if lx >= ly {
        let phi = ex.y.atan2(ex.x);
        let ratio = if lx > 1e-12 { ly / lx } else { 0.0 };
        (scale(ex, radius), ratio, phi)
    } else {
        let phi = ey.y.atan2(ey.x);
        let ratio = if ly > 1e-12 { lx / ly } else { 0.0 };
        (scale(ey, radius), ratio, phi)
    };
    SemanticGeometry::Ellipse {
        center: c,
        major_axis,
        ratio,
        start: phi,
        sweep: std::f64::consts::TAU,
    }
}

fn rotation_of(t: &Transform3) -> f64 {
    // Rotation about Z from the transformed X axis.
    t.matrix[1][0].atan2(t.matrix[0][0])
}

fn segment_segment_intersection_3d(
    a1: Point3,
    a2: Point3,
    b1: Point3,
    b2: Point3,
    tol: f64,
) -> Option<Point3> {
    // Project onto XY and intersect, then reject if the Z separation is large.
    let r = sub(a2, a1);
    let s = sub(b2, b1);
    let denom = r.x * s.y - r.y * s.x;
    if denom.abs() < tol.max(1e-12) {
        return None;
    }
    let qp = sub(b1, a1);
    let t = (qp.x * s.y - qp.y * s.x) / denom;
    let u = (qp.x * r.y - qp.y * r.x) / denom;
    if !(0.0..=1.0).contains(&t) || !(0.0..=1.0).contains(&u) {
        return None;
    }
    let p = add(a1, scale(r, t));
    let p2 = add(b1, scale(s, u));
    if (p.z - p2.z).abs() > tol.max(1e-9) * 1000.0 {
        return None;
    }
    Some(Point3 {
        x: p.x,
        y: p.y,
        z: (p.z + p2.z) * 0.5,
    })
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
