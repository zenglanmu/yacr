//! World-space ray hit testing (spec F14 / F05).
//!
//! This is the CPU precise-pick stage that follows the broad phase in
//! [`crate::SpatialIndex`]: given a world [`Ray3`] and a set of
//! [`PickItem`]s (a [`SelectionRef`] plus its geometry and accumulated
//! transform) it returns the **closest** hit and its distance, never the first
//! candidate found.
//!
//! Picking never depends on GPU readback (spec §8.5): it is pure geometry, so
//! it runs identically on the WebGL2 path and in a host without a device.
//!
//! # What is supported
//!
//! * `Line` / `Polyline` / `Point` — analytic segment/point distance;
//! * `Circle` / `Arc` / `Ellipse` / `Spline` / `Text` — discretised through
//!   `cad_geometry::tessellate_geometry` and marked
//!   [`Precision::Approximate`] (see `docs/picking-3d.md`);
//! * `Mesh` — Möller–Trumbore triangle intersection with an explicit
//!   [`BackFacePolicy`];
//! * `Compound` — recurses and keeps the closest child.
//!
//! # What is explicitly *not* supported here
//!
//! * `Opaque` — an unknown payload is [`PickOutcome::Unsupported`]; a caller
//!   must never be handed a fabricated hit for it;
//! * `Insert` — the block graph lives in the database, so expanding an insert
//!   (and attaching its [`InstancePath`]) is the caller's job; a bare insert
//!   here is [`PickOutcome::Unsupported`].
//!
//! Degenerate rays (non-finite or zero-direction) and a non-positive tolerance
//! are refused with [`CadError::InvalidInput`] rather than silently missing.

use cad_domain::*;
use cad_geometry::{tessellate_geometry, TessellationParams};

use crate::PickHit;

/// Ray directions shorter than this are considered degenerate.
pub const MIN_RAY_DIRECTION: f64 = 1e-12;
/// Maximum number of tessellation segments for one curve during picking, so a
/// pathological curve can never make a single pick unbounded.
pub const MAX_PICK_SEGMENTS: usize = 8192;

/// How a ray treats the back face of a triangle.
///
/// The renderer uses `cull_mode = Back` with a mirrored-front-face variant, so
/// a pick that must agree with what the user sees has to state its policy
/// explicitly instead of leaving it implicit in the winding sign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackFacePolicy {
    /// Ignore triangles whose outward side faces away from the ray origin.
    Cull,
    /// Accept both sides of a triangle (for open sheets / single-sided meshes).
    DoubleSided,
}

/// World-space picking parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickOptions {
    /// Maximum world distance from the ray at which a segment/curve counts as
    /// hit. Derived from the [`TolerancePolicy`] by the caller (see
    /// `cad_app::picking::pick_tolerance`).
    pub tolerance: f64,
    /// Back-face treatment for mesh triangles.
    pub back_faces: BackFacePolicy,
}

impl PickOptions {
    /// Validated options with back-face culling (matching the renderer default).
    pub fn new(tolerance: f64) -> CadResult<Self> {
        let options = PickOptions {
            tolerance,
            back_faces: BackFacePolicy::Cull,
        };
        options.validate()?;
        Ok(options)
    }

    /// Override the back-face policy.
    pub fn with_back_faces(mut self, policy: BackFacePolicy) -> Self {
        self.back_faces = policy;
        self
    }

    /// Reject a non-finite or non-positive tolerance (audit: never fake a hit
    /// with a degenerate tolerance).
    pub fn validate(&self) -> CadResult<()> {
        if !self.tolerance.is_finite() || self.tolerance <= 0.0 {
            return Err(CadError::InvalidInput(
                "pick tolerance must be finite and > 0".into(),
            ));
        }
        Ok(())
    }
}

/// One object offered to the precise pick stage.
#[derive(Debug, Clone)]
pub struct PickItem {
    /// The identity a hit reports (carries the `InstancePath` for INSERTs).
    pub source: SelectionRef,
    /// The geometry in its own local coordinates.
    pub geometry: SemanticGeometry,
    /// The accumulated world transform for this item (identity for a
    /// top-level model-space entity; the composed insert chain otherwise).
    pub transform: Transform3,
    /// Where the geometry came from, carried through to the hit.
    pub geometry_source: GeometrySource,
}

/// A precise hit with its depth along the ray.
#[derive(Debug, Clone, PartialEq)]
pub struct GeometryHit {
    /// The world point on the geometry closest to the ray.
    pub point: Point3,
    /// Distance along the (unit) ray direction. Always `>= 0`.
    pub distance: f64,
    /// Perpendicular (screen-space) distance from the ray to the hit point,
    /// used only to break a tie between hits at the same depth.
    pub offset: f64,
    pub precision: Precision,
    pub geometry_source: GeometrySource,
}

/// The verdict of a precise test for one item.
#[derive(Debug, Clone, PartialEq)]
pub enum PickOutcome {
    /// The ray hit the geometry within tolerance.
    Hit(GeometryHit),
    /// The ray did not hit within tolerance.
    Miss,
    /// The geometry cannot be tested precisely; `reason` is a machine key.
    Unsupported(&'static str),
}

/// An item that was skipped because it cannot be picked precisely.
#[derive(Debug, Clone, PartialEq)]
pub struct SkippedGeometry {
    pub source: SelectionRef,
    /// Machine-readable reason (never translated prose).
    pub reason: &'static str,
}

/// The result of a closest-hit query over many items.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PickReport {
    /// The closest hit, or `None` when nothing was hit.
    pub hit: Option<PickHit>,
    /// Items that could not be tested precisely, with the reason each was
    /// skipped. Never conflated with a `Miss`.
    pub skipped: Vec<SkippedGeometry>,
}

/// Validate a ray: finite origin/direction and a non-degenerate direction.
pub fn validate_ray(ray: &Ray3) -> CadResult<()> {
    if !is_finite_point(ray.origin) {
        return Err(CadError::InvalidInput("ray origin is not finite".into()));
    }
    if !is_finite_point(ray.direction) {
        return Err(CadError::InvalidInput("ray direction is not finite".into()));
    }
    if length3(ray.direction) < MIN_RAY_DIRECTION {
        return Err(CadError::InvalidInput("ray direction is degenerate".into()));
    }
    Ok(())
}

/// Closest approach between a ray and a segment.
///
/// Returns `(t, point_on_segment, distance)` where `t` is the parameter along
/// the ray (not clamped, so a caller can reject hits behind the origin) and
/// `point_on_segment` is the nearest point of the segment `a..b`. A degenerate
/// segment is treated as the single point `a`.
pub fn ray_segment_closest(ray: &Ray3, a: Point3, b: Point3) -> Option<(f64, Point3, f64)> {
    let d = ray.direction;
    let a1 = dot3(d, d);
    if !a1.is_finite() || a1 < MIN_RAY_DIRECTION * MIN_RAY_DIRECTION {
        return None;
    }
    let ab = sub(b, a);
    let c1 = dot3(ab, ab);
    let w0 = sub(ray.origin, a);
    let d1 = dot3(d, w0);
    let e1 = dot3(ab, w0);

    let s = if c1 < 1e-24 {
        // Degenerate segment: the nearest point is `a`.
        0.0
    } else {
        let denom = a1 * c1 - dot3(d, ab) * dot3(d, ab);
        let s = if denom.abs() < 1e-18 {
            // Near-parallel: project the ray origin onto the segment.
            e1 / c1
        } else {
            (a1 * e1 - dot3(d, ab) * d1) / denom
        };
        s.clamp(0.0, 1.0)
    };
    let point = add(a, scale3(ab, s));
    // The ray parameter of the closest point on the ray to `point`.
    let t = dot3(sub(point, ray.origin), d) / a1;
    let closest_on_ray = add(ray.origin, scale3(d, t));
    let dist = length3(sub(point, closest_on_ray));
    Some((t, point, dist))
}

/// Möller–Trumbore ray/triangle intersection.
///
/// Returns the ray parameter `t >= 0` of the intersection, or `None` for a
/// parallel/degenerate triangle or a miss. `policy` decides whether a triangle
/// whose world outward normal faces away from the origin is ignored.
pub fn ray_triangle(
    ray: &Ray3,
    v0: Point3,
    v1: Point3,
    v2: Point3,
    policy: BackFacePolicy,
) -> Option<f64> {
    let e1 = sub(v1, v0);
    let e2 = sub(v2, v0);
    let n = cross(e1, e2);
    if length3(n) < 1e-18 {
        // Degenerate triangle (zero area): never a hit.
        return None;
    }
    if policy == BackFacePolicy::Cull && dot3(n, ray.direction) >= 0.0 {
        // The outward normal points away from (or along) the ray: back face.
        return None;
    }
    let pvec = cross(ray.direction, e2);
    let det = dot3(e1, pvec);
    if det.abs() < 1e-15 {
        return None;
    }
    let inv = 1.0 / det;
    let tvec = sub(ray.origin, v0);
    let u = dot3(tvec, pvec) * inv;
    if !(-1e-12..=1.0 + 1e-12).contains(&u) {
        return None;
    }
    let qvec = cross(tvec, e1);
    let v = dot3(ray.direction, qvec) * inv;
    if v < -1e-12 || u + v > 1.0 + 1e-12 {
        return None;
    }
    let t = dot3(e2, qvec) * inv;
    if !t.is_finite() || t < 0.0 {
        return None;
    }
    Some(t)
}

/// Test one geometry item at its accumulated world transform.
///
/// Validates its options and ray, and never reports a hit for geometry it
/// cannot test (opaque payloads and unexpanded inserts are `Unsupported`).
pub fn hit_geometry(
    ray: &Ray3,
    geometry: &SemanticGeometry,
    transform: &Transform3,
    options: &PickOptions,
) -> CadResult<PickOutcome> {
    validate_ray(ray)?;
    options.validate()?;
    let params = TessellationParams {
        tolerance: options.tolerance,
        max_segments: MAX_PICK_SEGMENTS,
        ..TessellationParams::default()
    };
    Ok(match geometry {
        SemanticGeometry::Line { start, end } => match segment_hit(
            ray,
            transform.apply_point(*start),
            transform.apply_point(*end),
            options.tolerance,
        ) {
            Some((t, offset, point)) => hit(
                t,
                offset,
                point,
                Precision::Analytic,
                GeometrySource::Analytic,
            ),
            None => PickOutcome::Miss,
        },
        SemanticGeometry::Point(p) => {
            let world = transform.apply_point(*p);
            match segment_hit(ray, world, world, options.tolerance) {
                Some((t, offset, point)) => hit(
                    t,
                    offset,
                    point,
                    Precision::Analytic,
                    GeometrySource::Analytic,
                ),
                None => PickOutcome::Miss,
            }
        }
        SemanticGeometry::Polyline { .. } => {
            let points = transform_polyline(tessellate_geometry(geometry, params), transform);
            match polyline_hit(ray, &points, options.tolerance) {
                Some((t, offset, point)) => hit(
                    t,
                    offset,
                    point,
                    tessellation_precision(options),
                    GeometrySource::Analytic,
                ),
                None => PickOutcome::Miss,
            }
        }
        SemanticGeometry::Circle { .. }
        | SemanticGeometry::Arc { .. }
        | SemanticGeometry::Ellipse { .. }
        | SemanticGeometry::Spline { .. }
        | SemanticGeometry::Text { .. } => {
            let points = transform_polyline(tessellate_geometry(geometry, params), transform);
            match polyline_hit(ray, &points, options.tolerance) {
                Some((t, offset, point)) => hit(
                    t,
                    offset,
                    point,
                    tessellation_precision(options),
                    GeometrySource::Analytic,
                ),
                None => PickOutcome::Miss,
            }
        }
        SemanticGeometry::Mesh(mesh) => match mesh_hit(ray, mesh, transform, options) {
            Some((t, point)) => hit(
                t,
                0.0,
                point,
                Precision::Analytic,
                GeometrySource::DirectMesh,
            ),
            None => PickOutcome::Miss,
        },
        SemanticGeometry::Compound(children) => {
            let mut best: Option<(f64, f64, Point3, Precision)> = None;
            let mut unsupported: Option<&'static str> = None;
            for child in children {
                match hit_geometry(ray, child, transform, options)? {
                    PickOutcome::Hit(hit) => {
                        let better = best
                            .as_ref()
                            .map(|(t, o, _, _)| (hit.distance, hit.offset) < (*t, *o))
                            .unwrap_or(true);
                        if better {
                            best = Some((hit.distance, hit.offset, hit.point, hit.precision));
                        }
                    }
                    PickOutcome::Miss => {}
                    PickOutcome::Unsupported(reason) => unsupported = Some(reason),
                }
            }
            match best {
                Some((distance, offset, point, precision)) => PickOutcome::Hit(GeometryHit {
                    point,
                    distance,
                    offset,
                    precision,
                    geometry_source: GeometrySource::Analytic,
                }),
                None if unsupported.is_some() => PickOutcome::Unsupported("compound-unsupported"),
                None => PickOutcome::Miss,
            }
        }
        SemanticGeometry::Insert { .. } => PickOutcome::Unsupported("insert-not-expanded"),
        SemanticGeometry::Opaque { .. } => PickOutcome::Unsupported("opaque"),
    })
}

/// Precise closest-hit query over many items.
///
/// Returns the hit with the smallest ray distance, never the first found, plus
/// the list of items that could not be tested. Degenerate rays and tolerances
/// are errors.
pub fn pick_closest(
    ray: &Ray3,
    items: &[PickItem],
    options: &PickOptions,
) -> CadResult<PickReport> {
    validate_ray(ray)?;
    options.validate()?;
    let mut report = PickReport::default();
    let mut best: Option<PickHit> = None;
    for item in items {
        match hit_geometry(ray, &item.geometry, &item.transform, options)? {
            PickOutcome::Hit(hit) => {
                let candidate = PickHit {
                    source: item.source.clone(),
                    point: hit.point,
                    distance: hit.distance,
                    offset: hit.offset,
                    precision: hit.precision,
                    geometry_source: hit.geometry_source,
                };
                // Closest depth wins; at equal depth the smaller perpendicular
                // offset wins, so two candidates on the same plane are ordered
                // by how near the ray actually passes them.
                let better = best
                    .as_ref()
                    .map(|b| (candidate.distance, candidate.offset) < (b.distance, b.offset))
                    .unwrap_or(true);
                if better {
                    best = Some(candidate);
                }
            }
            PickOutcome::Miss => {}
            PickOutcome::Unsupported(reason) => report.skipped.push(SkippedGeometry {
                source: item.source.clone(),
                reason,
            }),
        }
    }
    report.hit = best;
    Ok(report)
}

fn hit(
    distance: f64,
    offset: f64,
    point: Point3,
    precision: Precision,
    geometry_source: GeometrySource,
) -> PickOutcome {
    PickOutcome::Hit(GeometryHit {
        point,
        distance,
        offset,
        precision,
        geometry_source,
    })
}

fn tessellation_precision(options: &PickOptions) -> Precision {
    Precision::Approximate {
        error_bound: Some(options.tolerance),
    }
}

fn transform_polyline(points: Vec<Point3>, transform: &Transform3) -> Vec<Point3> {
    points
        .into_iter()
        .map(|p| transform.apply_point(p))
        .collect()
}

/// Hit test a polyline (already in world coordinates) and return the closest
/// `(t, perpendicular offset, point)`; an empty or single-point list is `None`.
fn polyline_hit(ray: &Ray3, points: &[Point3], tolerance: f64) -> Option<(f64, f64, Point3)> {
    let mut best: Option<(f64, f64, Point3)> = None;
    for window in points.windows(2) {
        if let Some((t, point, dist)) = ray_segment_closest(ray, window[0], window[1]) {
            if t < 0.0 {
                continue;
            }
            if dist <= tolerance {
                let better = best.as_ref().map(|(bt, _, _)| t < *bt).unwrap_or(true);
                if better {
                    best = Some((t, dist, point));
                }
            }
        }
    }
    best
}

/// Hit test a single segment and return `(t, perpendicular offset, point)`,
/// requiring `t >= 0` and the distance within tolerance.
fn segment_hit(ray: &Ray3, a: Point3, b: Point3, tolerance: f64) -> Option<(f64, f64, Point3)> {
    let (t, point, dist) = ray_segment_closest(ray, a, b)?;
    if t < 0.0 || dist > tolerance {
        return None;
    }
    Some((t, dist, point))
}

/// Hit test a mesh and return the closest `(t, world point)`.
fn mesh_hit(
    ray: &Ray3,
    mesh: &Mesh,
    transform: &Transform3,
    options: &PickOptions,
) -> Option<(f64, Point3)> {
    let mut best: Option<(f64, Point3)> = None;
    let vertices: Vec<Point3> = mesh
        .vertices
        .iter()
        .map(|v| transform.apply_point(*v))
        .collect();
    for tri in &mesh.triangles {
        let (ia, ib, ic) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if ia >= vertices.len() || ib >= vertices.len() || ic >= vertices.len() {
            // A malformed index is skipped, never treated as a hit.
            continue;
        }
        if let Some(t) = ray_triangle(
            ray,
            vertices[ia],
            vertices[ib],
            vertices[ic],
            options.back_faces,
        ) {
            let better = best.as_ref().map(|(bt, _)| t < *bt).unwrap_or(true);
            if better {
                best = Some((t, add(ray.origin, scale3(ray.direction, t))));
            }
        }
    }
    best
}

// --- local vector helpers (cad-spatial has no geometry prelude of its own) ---

fn dot3(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

fn length3(v: Point3) -> f64 {
    dot3(v, v).sqrt()
}

fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn scale3(v: Point3, s: f64) -> Point3 {
    Point3 {
        x: v.x * s,
        y: v.y * s,
        z: v.z * s,
    }
}

fn is_finite_point(p: Point3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_domain::{DocumentId, EntityId, InstancePath};

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    fn ray(origin: Point3, direction: Point3) -> Ray3 {
        Ray3 { origin, direction }
    }

    fn source(id: u128, instance: Vec<u128>) -> SelectionRef {
        SelectionRef {
            document: DocumentId(1),
            entity: EntityId(id),
            instance: InstancePath(instance.into_iter().map(EntityId).collect()),
            sub_element: None,
        }
    }

    fn item(id: u128, geometry: SemanticGeometry) -> PickItem {
        PickItem {
            source: source(id, Vec::new()),
            geometry,
            transform: Transform3::identity(),
            geometry_source: GeometrySource::Analytic,
        }
    }

    #[test]
    fn segment_hit_and_miss_with_tolerance() {
        let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
        // A segment crossing the ray (distance 0).
        let hit = segment_hit(&r, p(5.0, -1.0, 0.0), p(5.0, 1.0, 0.0), 1e-6);
        assert!(hit.is_some());
        // A segment running parallel to the ray, offset by 0.001: beyond a
        // 1e-6 tolerance it is a miss.
        let miss = segment_hit(&r, p(5.0, 0.001, -1.0), p(5.0, 0.001, 1.0), 1e-6);
        assert!(miss.is_none());
    }

    #[test]
    fn tolerance_scales_the_segment_hit_window() {
        let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
        let a = p(5.0, 0.25, 0.0);
        let b = p(5.0, 0.25, 1.0);
        // Distance 0.25: missed with a 0.1 tolerance, hit with 0.5.
        assert!(segment_hit(&r, a, b, 0.1).is_none());
        assert!(segment_hit(&r, a, b, 0.5).is_some());
    }

    #[test]
    fn segment_behind_the_origin_is_a_miss() {
        let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
        // The segment is behind the ray origin.
        assert!(segment_hit(&r, p(-5.0, -1.0, 0.0), p(-5.0, 1.0, 0.0), 10.0).is_none());
    }

    #[test]
    fn mesh_triangle_hit_respects_back_face_policy() {
        // Triangle in the z=5 plane, counter-clockwise seen from -z, so its
        // outward normal points -z (toward a ray that travels +z).
        let v0 = p(-1.0, -1.0, 5.0);
        let v1 = p(0.0, 1.0, 5.0);
        let v2 = p(1.0, -1.0, 5.0);
        // A ray travelling +z from the origin sees the front face.
        let front = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
        assert!(ray_triangle(&front, v0, v1, v2, BackFacePolicy::Cull).is_some());
        // A ray travelling -z from beyond the plane sees the back face.
        let back = ray(p(0.0, 0.0, 10.0), p(0.0, 0.0, -1.0));
        assert!(ray_triangle(&back, v0, v1, v2, BackFacePolicy::Cull).is_none());
        assert!(ray_triangle(&back, v0, v1, v2, BackFacePolicy::DoubleSided).is_some());
    }

    #[test]
    fn mesh_pick_returns_the_closest_triangle() {
        let near = p(-1.0, -1.0, 2.0);
        let nb = p(0.0, 1.0, 2.0);
        let nc = p(1.0, -1.0, 2.0);
        let far = p(-1.0, -1.0, 5.0);
        let fb = p(0.0, 1.0, 5.0);
        let fc = p(1.0, -1.0, 5.0);
        let mesh = Mesh {
            vertices: vec![near, nb, nc, far, fb, fc],
            triangles: vec![[0, 1, 2], [3, 4, 5]],
            normals: Vec::new(),
            face_sources: Vec::new(),
        };
        let r = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
        let options = PickOptions::new(1e-6).unwrap();
        match hit_geometry(
            &r,
            &SemanticGeometry::Mesh(mesh),
            &Transform3::identity(),
            &options,
        )
        .unwrap()
        {
            PickOutcome::Hit(hit) => {
                assert!((hit.distance - 2.0).abs() < 1e-9, "closest triangle wins");
                assert!((hit.point.z - 2.0).abs() < 1e-9);
            }
            other => panic!("expected a hit, got {other:?}"),
        }
    }

    #[test]
    fn closest_of_many_is_returned_not_the_first() {
        let items = vec![
            item(
                1,
                SemanticGeometry::Line {
                    start: p(50.0, -1.0, 0.0),
                    end: p(50.0, 1.0, 0.0),
                },
            ),
            item(
                2,
                SemanticGeometry::Line {
                    start: p(5.0, -1.0, 0.0),
                    end: p(5.0, 1.0, 0.0),
                },
            ),
        ];
        let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
        let options = PickOptions::new(1e-6).unwrap();
        let report = pick_closest(&r, &items, &options).unwrap();
        let hit = report.hit.expect("a hit");
        assert_eq!(hit.source.entity, EntityId(2), "closest, not first");
        assert!((hit.distance - 5.0).abs() < 1e-9);
    }

    #[test]
    fn instance_path_is_carried_on_the_hit() {
        let mut a = item(
            20,
            SemanticGeometry::Line {
                start: p(5.0, -1.0, 0.0),
                end: p(5.0, 1.0, 0.0),
            },
        );
        a.source = source(20, vec![3]);
        let mut b = item(
            20,
            SemanticGeometry::Line {
                start: p(5.0, -1.0, 0.0),
                end: p(5.0, 1.0, 0.0),
            },
        );
        b.source = source(20, vec![4]);
        let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
        let options = PickOptions::new(1e-6).unwrap();
        let report = pick_closest(&r, &[a, b], &options).unwrap();
        let hit = report.hit.expect("a hit");
        assert_eq!(hit.source.entity, EntityId(20));
        assert_eq!(hit.source.instance, InstancePath(vec![EntityId(3)]));
    }

    #[test]
    fn opaque_geometry_is_skipped_with_a_reason() {
        let items = vec![item(
            9,
            SemanticGeometry::Opaque {
                type_key: "ACAD_PROXY_ENTITY".into(),
                version: 1,
                payload: vec![1, 2, 3],
            },
        )];
        let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
        let options = PickOptions::new(1e-6).unwrap();
        let report = pick_closest(&r, &items, &options).unwrap();
        assert!(report.hit.is_none());
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].reason, "opaque");
    }

    #[test]
    fn insert_is_not_silently_claimed_as_a_hit() {
        let items = vec![item(
            3,
            SemanticGeometry::Insert {
                block: BlockId(0),
                transform: Transform3::identity(),
            },
        )];
        let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
        let options = PickOptions::new(1e-6).unwrap();
        let report = pick_closest(&r, &items, &options).unwrap();
        assert!(report.hit.is_none());
        assert_eq!(report.skipped[0].reason, "insert-not-expanded");
    }

    #[test]
    fn degenerate_ray_and_tolerance_are_refused() {
        let items = vec![item(
            1,
            SemanticGeometry::Line {
                start: p(5.0, -1.0, 0.0),
                end: p(5.0, 1.0, 0.0),
            },
        )];
        let good_options = PickOptions::new(1e-6).unwrap();
        let zero = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 0.0));
        assert!(pick_closest(&zero, &items, &good_options).is_err());
        let nan = ray(p(0.0, 0.0, 0.0), p(f64::NAN, 0.0, 0.0));
        assert!(pick_closest(&nan, &items, &good_options).is_err());
        let good = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
        // A zero/NaN tolerance must be refused by `validate` and by `new`.
        let zero_tol = PickOptions {
            tolerance: 0.0,
            back_faces: BackFacePolicy::Cull,
        };
        let nan_tol = PickOptions {
            tolerance: f64::NAN,
            back_faces: BackFacePolicy::Cull,
        };
        assert!(zero_tol.validate().is_err());
        assert!(nan_tol.validate().is_err());
        assert!(pick_closest(&good, &items, &zero_tol).is_err());
        assert!(pick_closest(&good, &items, &nan_tol).is_err());
        assert!(PickOptions::new(0.0).is_err());
        assert!(PickOptions::new(-1.0).is_err());
    }

    #[test]
    fn transform_is_applied_before_testing() {
        let mut item = item(
            1,
            SemanticGeometry::Line {
                start: p(0.0, -1.0, 0.0),
                end: p(0.0, 1.0, 0.0),
            },
        );
        item.transform = Transform3::translation(p(5.0, 0.0, 0.0));
        let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
        let report = pick_closest(&r, &[item], &PickOptions::new(1e-6).unwrap()).unwrap();
        let hit = report.hit.expect("translated line is hit at x=5");
        assert!((hit.distance - 5.0).abs() < 1e-9);
    }

    #[test]
    fn circle_is_tessellated_and_marked_approximate() {
        let circle = SemanticGeometry::Circle {
            center: p(10.0, 0.0, 0.0),
            normal: p(0.0, 0.0, 1.0),
            radius: 2.0,
        };
        let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
        let options = PickOptions::new(0.05).unwrap();
        match hit_geometry(&r, &circle, &Transform3::identity(), &options).unwrap() {
            PickOutcome::Hit(hit) => {
                assert!(matches!(hit.precision, Precision::Approximate { .. }));
                assert!((hit.point.x - 8.0).abs() < 0.1, "near side of the circle");
            }
            other => panic!("expected a hit, got {other:?}"),
        }
    }
}
