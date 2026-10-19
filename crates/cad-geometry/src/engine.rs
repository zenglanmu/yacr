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

// ---------------------------------------------------------------------------
// Bounded sampling for the *approximate* intersection fallback.
//
// Only pairs without an analytic solution reach this code (a bulge polyline is
// not a conic primitive). The two operands are flattened and tested pairwise,
// so the cost is `O(n*m)` and both `n` and `m` must be tied to the geometry
// rather than to an absolute tolerance.
// ---------------------------------------------------------------------------

/// Chord error of the fallback, as a fraction of the operand's own local
/// extent (see [`fallback_scale`]).
///
/// 1e-4 keeps every sampled chord within 0.01% of the entity's overall size:
/// well below the interaction/measurement resolution for snapping, while the
/// sample count becomes scale-free (a full circle needs ~157 segments at any
/// radius instead of always saturating the cap). The final acceptance test
/// still uses `tolerance.computation_world`; only the candidate search is
/// coarsened.
pub(crate) const FALLBACK_CHORD_FRACTION: f64 = 1e-4;

/// Hard cap on the number of tessellated segments per operand in the fallback.
///
/// The pair loop is `O(n*m)`, so this cap is what makes the cost predictable:
/// at most `256 * 256 = 65_536` segment tests per pair regardless of drawing
/// scale, arc count or spline wiggliness. A full circle or ellipse at
/// [`FALLBACK_CHORD_FRACTION`] needs ~157 segments, so ordinary curves are not
/// clipped; only pathological multi-arc polylines or very wiggly splines are
/// uniformly decimated (see [`bound_fallback_samples`]) and are already
/// approximate by construction.
pub(crate) const FALLBACK_MAX_SEGMENTS: usize = 256;

/// Tessellation parameters for one operand of the approximate fallback.
///
/// The chord is `(extent * FALLBACK_CHORD_FRACTION).max(computation_world)`,
/// where `extent` is the geometry's own local extent ([`fallback_scale`]). It
/// therefore follows the entity, not the drawing units or the view: scaling a
/// probe by 1e3 does not change its sample count.
pub(crate) fn fallback_tessellation_params(
    geometry: &SemanticGeometry,
    tolerance: &TolerancePolicy,
) -> TessellationParams {
    let extent = fallback_scale(geometry);
    let chord = (extent * FALLBACK_CHORD_FRACTION).max(tolerance.computation_world.max(1e-12));
    TessellationParams {
        tolerance: chord,
        max_segments: FALLBACK_MAX_SEGMENTS,
        min_segments: 4,
    }
}

/// A cheap local world-scale estimate for the fallback chord.
///
/// This deliberately does **not** call `GeometryEngine::bounds`: bounds
/// tessellates, and doing that per pair on the per-pointer-move path would
/// reintroduce the very cost this fallback is bounded against (a multi-arc
/// polyline would be flattened at up to 256 samples per arc). The estimate is
/// taken from vertices, radii and control points instead, so it is `O(n)` in
/// the entity's own description and needs no allocation of samples.
///
/// Over-estimating the scale is safe: it only *coarsens* the chord and the
/// fallback is approximate by construction.
pub(crate) fn fallback_scale(geometry: &SemanticGeometry) -> f64 {
    use SemanticGeometry as G;
    match geometry {
        G::Line { start, end } => distance(*start, *end),
        G::Polyline { points, bulges, .. } => {
            if points.is_empty() {
                return 0.0;
            }
            let mut sagitta = 0.0f64;
            for (i, a) in points.iter().enumerate() {
                let b = points[(i + 1) % points.len()];
                let chord = distance(*a, b);
                sagitta = sagitta.max(bulges.get(i).copied().unwrap_or(0.0).abs() * 0.5 * chord);
            }
            // An arc's apex can lie outside the vertex box; add the largest
            // sagitta on both sides as a cheap upper bound.
            bbox_extent(points) + 2.0 * sagitta
        }
        G::Circle { radius, .. } | G::Arc { radius, .. } => 2.0 * radius.abs(),
        G::Ellipse { major_axis, .. } => 2.0 * length(*major_axis),
        G::Spline { control_points, .. } => bbox_extent(control_points),
        G::Mask { boundary, .. } => bbox_extent(boundary),
        G::Compound(children) => children.iter().map(fallback_scale).fold(0.0f64, f64::max),
        // A dimension's fallback extent is the largest of its display children,
        // exactly like a compound (ADR 0017 A.8).
        G::Dimension(dimension) => dimension
            .display()
            .iter()
            .map(fallback_scale)
            .fold(0.0f64, f64::max),
        // Point/Text have no closed extent; Mesh/Insert/Opaque/Shape/Image do
        // not tessellate to a polyline, so the fallback yields nothing anyway.
        _ => 0.0,
    }
}

/// Diagonal of the axis-aligned box spanned by `points` (0.0 when empty).
fn bbox_extent(points: &[Point3]) -> f64 {
    let mut acc = BoundsAccumulator::new();
    for p in points {
        acc.add_point(*p);
    }
    acc.finish()
        .map(|b| length(sub(b.max, b.min)))
        .unwrap_or(0.0)
}

/// Truncate a tessellation to the fallback's per-operand point budget.
///
/// A single arc or curve is already capped by `TessellationParams::max_segments`,
/// but a polyline made of many bulged segments can exceed it in total. In that
/// case the samples are uniformly decimated (first/last vertices kept). This is
/// deliberately a *bound*, not a refinement: the fallback is approximate, and a
/// pair whose operands are this dense is outside the precision it promises.
pub(crate) fn bound_fallback_samples(points: Vec<Point3>) -> Vec<Point3> {
    let max_points = FALLBACK_MAX_SEGMENTS + 1;
    if points.len() <= max_points || max_points < 2 {
        return points;
    }
    let stride = (points.len() - 1).div_ceil(max_points - 1);
    let last = match points.last() {
        Some(p) => *p,
        None => return points,
    };
    let mut out = Vec::with_capacity(max_points);
    let mut i = 0;
    while i < points.len() {
        out.push(points[i]);
        i += stride;
    }
    if out.last() != Some(&last) {
        if out.len() == max_points {
            out.pop();
        }
        out.push(last);
    }
    out
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
            // A raster image is a rigid placement: `origin` transforms as a
            // point, `u`/`v` as direction vectors; the pixel size and resource
            // key are not geometry and are preserved.
            G::Image {
                origin,
                u,
                v,
                pixels,
                file,
                clip,
                visible,
            } => G::Image {
                origin: tp(*origin),
                u: td(*u),
                v: td(*v),
                pixels: *pixels,
                file: file.clone(),
                clip: clip.clone(),
                visible: *visible,
            },
            // A mask is a world-space polygon: every boundary point moves.
            G::Mask { boundary, inverted } => G::Mask {
                boundary: boundary.iter().map(|p| tp(*p)).collect(),
                inverted: *inverted,
            },
            G::Compound(children) => {
                let mut out = Vec::with_capacity(children.len());
                for child in children {
                    out.push(self.transform(child, t)?);
                }
                G::Compound(out)
            }
            // A dimension moves its definition points, display children and
            // measurement together (ADR 0017 A.5). This is the second transform
            // surface (the snap path); `cad-db::transform_geometry` is the other
            // and both delegate to the same domain method.
            G::Dimension(dimension) => G::Dimension(Box::new(dimension.transformed(t)?)),
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
        // Everything else (a bulge polyline is not one conic, for example) is
        // **approximate**: the two entities are flattened and the crossing is
        // found between sampled segments. The result is only as accurate as the
        // chord below, and it is a candidate point, not an analytic solution.
        //
        // The sampling must stay scale-relative and bounded. An absolute chord
        // (this used to be a fixed 1e-9) is finer than any display or
        // measurement need, always saturates the segment cap at any drawing
        // scale, and makes the `O(n*m)` pair test quadratic in the cap: the
        // circle x bulge-polyline probe measured ~210 ms per pair in release
        // before this change (tests/intersection_fallback_cost.rs).
        let pa = bound_fallback_samples(tessellate_geometry(
            a,
            fallback_tessellation_params(a, tolerance),
        ));
        let pb = bound_fallback_samples(tessellate_geometry(
            b,
            fallback_tessellation_params(b, tolerance),
        ));
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
