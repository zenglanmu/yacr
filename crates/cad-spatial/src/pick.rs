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

/// Machine key: a mesh carried no `face_sources` list at all, so a triangle
/// index cannot be mapped to a stable sub-element.
pub const REASON_FACE_SOURCES_ABSENT: &str = "mesh-face-sources-absent";
/// Machine key: the mesh has a `face_sources` list but it is shorter than its
/// triangle list, so the hit triangle has no corresponding entry.
pub const REASON_FACE_SOURCE_OUT_OF_RANGE: &str = "mesh-face-source-index-out-of-range";
/// Machine key: `face_sources[triangle]` is explicitly `None`: the producer did
/// not attach a stable id to that face.
pub const REASON_FACE_SOURCE_MISSING: &str = "mesh-face-source-missing";

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
    /// The stable sub-element of the hit geometry, when the geometry carries
    /// one. Only meshes are sub-element-addressable here: the hit triangle is
    /// mapped through [`Mesh::face_sources`]. Lines/curves/points always yield
    /// `None` and are not sub-element-addressable.
    pub sub_element: Option<SubElementId>,
    /// Machine key explaining `sub_element == None` for a mesh face that has no
    /// stable source (`face_sources` absent / short / entry `None`). `None`
    /// when a sub-element was resolved or when the geometry has no faces.
    pub sub_element_reason: Option<&'static str>,
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
    hit_validated(ray, geometry, transform, options, params)
}

fn hit_validated(
    ray: &Ray3,
    geometry: &SemanticGeometry,
    transform: &Transform3,
    options: &PickOptions,
    params: TessellationParams,
) -> CadResult<PickOutcome> {
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
        SemanticGeometry::Shape { position, .. } => {
            // The glyph outline needs the font; pick the anchor point.
            let world = transform.apply_point(*position);
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
        | SemanticGeometry::Text { .. }
        // A mask tessellates to its closed boundary polygon, so it picks like a
        // polyline. A raster image has no analytic pick outline in this build
        // (the representation layer draws it), so it tessellates to no points
        // and honestly misses here rather than fabricating a hit.
        | SemanticGeometry::Mask { .. }
        | SemanticGeometry::Image { .. } => {
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
            Some((t, point, triangle)) => {
                let (sub_element, sub_element_reason) = face_sub_element(mesh, triangle);
                PickOutcome::Hit(GeometryHit {
                    point,
                    distance: t,
                    offset: 0.0,
                    precision: Precision::Analytic,
                    geometry_source: GeometrySource::DirectMesh,
                    sub_element,
                    sub_element_reason,
                })
            }
            None => PickOutcome::Miss,
        },
        SemanticGeometry::Compound(children) => {
            let mut best: Option<GeometryHit> = None;
            let mut unsupported: Option<&'static str> = None;
            for child in children {
                match hit_validated(ray, child, transform, options, params)? {
                    PickOutcome::Hit(hit) => {
                        let better = best
                            .as_ref()
                            .map(|b| (hit.distance, hit.offset) < (b.distance, b.offset))
                            .unwrap_or(true);
                        if better {
                            best = Some(hit);
                        }
                    }
                    PickOutcome::Miss => {}
                    PickOutcome::Unsupported(reason) => unsupported = Some(reason),
                }
            }
            match best {
                // A compound keeps the winning child's sub-element/reason so a
                // mesh face inside a HATCH/compound selection stays addressable.
                Some(hit) => PickOutcome::Hit(GeometryHit {
                    geometry_source: GeometrySource::Analytic,
                    ..hit
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
    pick_closest_borrowed(
        ray,
        items
            .iter()
            .map(|item| (&item.source, &item.geometry, &item.transform)),
        options,
    )
}

/// Identical precise picking over borrowed geometry; callers need not deep-copy
/// large compounds just to perform a read-only query. Iterator order defines
/// the same stable tie handling as [`pick_closest`].
pub fn pick_closest_borrowed<'a>(
    ray: &Ray3,
    items: impl IntoIterator<Item = (&'a SelectionRef, &'a SemanticGeometry, &'a Transform3)>,
    options: &PickOptions,
) -> CadResult<PickReport> {
    validate_ray(ray)?;
    options.validate()?;
    let mut report = PickReport::default();
    let mut best: Option<PickHit> = None;
    let params = TessellationParams {
        tolerance: options.tolerance,
        max_segments: MAX_PICK_SEGMENTS,
        ..TessellationParams::default()
    };
    for (item_source, geometry, transform) in items {
        match hit_validated(ray, geometry, transform, options, params)? {
            PickOutcome::Hit(hit) => {
                // Identity is `entity + instance + sub-element`. The item names
                // the entity/instance; the geometry resolves the sub-element. A
                // caller that already narrowed the item to a sub-element keeps
                // its exact identity (the geometry cannot silently re-target it).
                let mut source = item_source.clone();
                let mut sub_element_reason = hit.sub_element_reason;
                if source.sub_element.is_none() {
                    source.sub_element = hit.sub_element;
                }
                if source.sub_element.is_some() {
                    // A resolved identity has no unresolved-sub-element reason.
                    sub_element_reason = None;
                }
                let candidate = PickHit {
                    source,
                    point: hit.point,
                    distance: hit.distance,
                    offset: hit.offset,
                    precision: hit.precision,
                    geometry_source: hit.geometry_source,
                    sub_element_reason,
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
                source: item_source.clone(),
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
        // Sub-elements belong to meshes; every other primitive is whole-object.
        sub_element: None,
        sub_element_reason: None,
    })
}

/// Map a hit triangle to its stable sub-element.
///
/// The mapping is a pure read of [`Mesh::face_sources`], indexed by triangle.
/// It never invents an id: a missing/short/`None` source yields `None` plus a
/// machine reason.
fn face_sub_element(mesh: &Mesh, triangle: usize) -> (Option<SubElementId>, Option<&'static str>) {
    if mesh.face_sources.is_empty() {
        return (None, Some(REASON_FACE_SOURCES_ABSENT));
    }
    match mesh.face_sources.get(triangle) {
        Some(Some(id)) => (Some(id.clone()), None),
        Some(None) => (None, Some(REASON_FACE_SOURCE_MISSING)),
        None => (None, Some(REASON_FACE_SOURCE_OUT_OF_RANGE)),
    }
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

/// Hit test a mesh and return the closest `(t, world point, triangle index)`.
///
/// The triangle index is the index into [`Mesh::triangles`] (and therefore into
/// [`Mesh::face_sources`]), so the caller can map it to a stable sub-element.
fn mesh_hit(
    ray: &Ray3,
    mesh: &Mesh,
    transform: &Transform3,
    options: &PickOptions,
) -> Option<(f64, Point3, usize)> {
    let mut best: Option<(f64, Point3, usize)> = None;
    let vertices: Vec<Point3> = mesh
        .vertices
        .iter()
        .map(|v| transform.apply_point(*v))
        .collect();
    for (triangle, tri) in mesh.triangles.iter().enumerate() {
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
            let better = best.as_ref().map(|(bt, _, _)| t < *bt).unwrap_or(true);
            if better {
                best = Some((t, add(ray.origin, scale3(ray.direction, t)), triangle));
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

    #[test]
    fn borrowed_picking_matches_owned_single_hit() {
        let source = SelectionRef {
            document: DocumentId(1),
            entity: EntityId(1),
            instance: InstancePath::default(),
            sub_element: None,
        };
        let items = vec![PickItem {
            source: source.clone(),
            geometry: SemanticGeometry::Line {
                start: p(-1.0, 0.0, 0.0),
                end: p(1.0, 0.0, 0.0),
            },
            transform: Transform3::identity(),
            geometry_source: GeometrySource::Analytic,
        }];
        let ray = Ray3 {
            origin: p(0.0, 0.0, 1.0),
            direction: p(0.0, 0.0, -1.0),
        };
        let options = PickOptions::new(0.1).unwrap();
        let owned = pick_closest(&ray, &items, &options).unwrap();
        let borrowed = pick_closest_borrowed(
            &ray,
            items
                .iter()
                .map(|item| (&item.source, &item.geometry, &item.transform)),
            &options,
        )
        .unwrap();
        assert_eq!(borrowed, owned);
        assert!(borrowed.hit.is_some());
    }
    use cad_domain::{DocumentId, EntityId, InstancePath, Revision, SubElementId};

    #[test]
    fn borrowed_pick_matches_owning_hits_ties_and_unsupported_reports() {
        let source = SelectionRef {
            document: DocumentId(1),
            entity: EntityId(1),
            instance: InstancePath::default(),
            sub_element: None,
        };
        let mut items = vec![PickItem {
            source,
            geometry: SemanticGeometry::Line {
                start: p(-1.0, 0.0, 0.0),
                end: p(1.0, 0.0, 0.0),
            },
            transform: Transform3::identity(),
            geometry_source: GeometrySource::Analytic,
        }];
        let mut second = items[0].clone();
        second.source.entity = EntityId(2);
        items.push(second);
        items.push(item(
            3,
            SemanticGeometry::Opaque {
                type_key: "PROXY".into(),
                version: 1,
                payload: vec![1],
            },
        ));
        let ray = Ray3 {
            origin: p(0.0, 0.0, 1.0),
            direction: p(0.0, 0.0, -1.0),
        };
        let options = PickOptions::new(0.1).unwrap();
        let owned = pick_closest(&ray, &items, &options).unwrap();
        let borrowed = pick_closest_borrowed(
            &ray,
            items
                .iter()
                .map(|item| (&item.source, &item.geometry, &item.transform)),
            &options,
        )
        .unwrap();
        assert_eq!(owned, borrowed);
        assert_eq!(borrowed.skipped.len(), 1);
        assert_eq!(borrowed.hit.unwrap().source.entity, EntityId(1));
    }

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
            colors: Vec::new(),
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

    fn face(key: &str) -> SubElementId {
        SubElementId {
            source_key: key.into(),
            topology_revision: Revision(0),
        }
    }

    /// Two coplanar triangles at z = 2, side by side along x, each carrying a
    /// distinct stable face source.
    fn two_face_mesh() -> Mesh {
        Mesh {
            vertices: vec![
                p(-2.0, -1.0, 2.0),
                p(-1.0, 1.0, 2.0),
                p(0.0, -1.0, 2.0),
                p(0.0, -1.0, 2.0),
                p(1.0, 1.0, 2.0),
                p(2.0, -1.0, 2.0),
            ],
            triangles: vec![[0, 1, 2], [3, 4, 5]],
            normals: Vec::new(),
            face_sources: vec![Some(face("face-a")), Some(face("face-b"))],
            colors: Vec::new(),
        }
    }

    #[test]
    fn mesh_face_hit_resolves_the_stable_sub_element() {
        let options = PickOptions::new(1e-6).unwrap();
        let mesh = SemanticGeometry::Mesh(two_face_mesh());
        let transform = Transform3::identity();

        let left = ray(p(-1.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
        match hit_geometry(&left, &mesh, &transform, &options).unwrap() {
            PickOutcome::Hit(hit) => {
                assert_eq!(
                    hit.sub_element.as_ref().map(|s| s.source_key.as_str()),
                    Some("face-a")
                );
                assert!(hit.sub_element_reason.is_none());
                assert!((hit.point.z - 2.0).abs() < 1e-9);
            }
            other => panic!("expected a hit, got {other:?}"),
        }

        let right = ray(p(1.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
        match hit_geometry(&right, &mesh, &transform, &options).unwrap() {
            PickOutcome::Hit(hit) => assert_eq!(
                hit.sub_element.as_ref().map(|s| s.source_key.as_str()),
                Some("face-b"),
                "each triangle maps to its own face source"
            ),
            other => panic!("expected a hit, got {other:?}"),
        }
    }

    #[test]
    fn mesh_face_without_a_source_is_unresolved_with_a_reason() {
        let options = PickOptions::new(1e-6).unwrap();
        let transform = Transform3::identity();
        let r = ray(p(-1.0, 0.0, 0.0), p(0.0, 0.0, 1.0));

        // No `face_sources` at all.
        let mut absent = two_face_mesh();
        absent.face_sources = Vec::new();
        match hit_geometry(&r, &SemanticGeometry::Mesh(absent), &transform, &options).unwrap() {
            PickOutcome::Hit(hit) => {
                assert!(hit.sub_element.is_none());
                assert_eq!(hit.sub_element_reason, Some(REASON_FACE_SOURCES_ABSENT));
            }
            other => panic!("expected a hit, got {other:?}"),
        }

        // The hit triangle's entry is explicitly `None`.
        let mut missing = two_face_mesh();
        missing.face_sources = vec![None, Some(face("face-b"))];
        match hit_geometry(&r, &SemanticGeometry::Mesh(missing), &transform, &options).unwrap() {
            PickOutcome::Hit(hit) => {
                assert!(hit.sub_element.is_none());
                assert_eq!(hit.sub_element_reason, Some(REASON_FACE_SOURCE_MISSING));
            }
            other => panic!("expected a hit, got {other:?}"),
        }

        // A short `face_sources` list puts the hit triangle out of range.
        let short_right = ray(p(1.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
        let mut short = two_face_mesh();
        short.face_sources = vec![Some(face("face-a"))];
        match hit_geometry(
            &short_right,
            &SemanticGeometry::Mesh(short),
            &transform,
            &options,
        )
        .unwrap()
        {
            PickOutcome::Hit(hit) => {
                assert!(hit.sub_element.is_none());
                assert_eq!(
                    hit.sub_element_reason,
                    Some(REASON_FACE_SOURCE_OUT_OF_RANGE)
                );
            }
            other => panic!("expected a hit, got {other:?}"),
        }
    }

    #[test]
    fn pick_closest_puts_the_resolved_sub_element_on_the_hit_source() {
        let item = PickItem {
            source: source(7, Vec::new()),
            geometry: SemanticGeometry::Mesh(two_face_mesh()),
            transform: Transform3::identity(),
            geometry_source: GeometrySource::DirectMesh,
        };
        let r = ray(p(-1.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
        let report = pick_closest(&r, &[item], &PickOptions::new(1e-6).unwrap()).unwrap();
        let hit = report.hit.expect("a hit");
        assert_eq!(hit.source.entity, EntityId(7));
        assert_eq!(
            hit.source
                .sub_element
                .as_ref()
                .map(|s| s.source_key.as_str()),
            Some("face-a")
        );
        assert!(hit.sub_element_reason.is_none());
    }

    #[test]
    fn two_faces_of_one_mesh_are_distinct_identities() {
        let options = PickOptions::new(1e-6).unwrap();
        let mesh = two_face_mesh();
        let left = PickItem {
            source: source(7, Vec::new()),
            geometry: SemanticGeometry::Mesh(mesh.clone()),
            transform: Transform3::identity(),
            geometry_source: GeometrySource::DirectMesh,
        };
        let right = PickItem {
            source: source(7, Vec::new()),
            geometry: SemanticGeometry::Mesh(mesh),
            transform: Transform3::identity(),
            geometry_source: GeometrySource::DirectMesh,
        };
        let a = pick_closest(
            &ray(p(-1.0, 0.0, 0.0), p(0.0, 0.0, 1.0)),
            std::slice::from_ref(&left),
            &options,
        )
        .unwrap()
        .hit
        .unwrap();
        let b = pick_closest(
            &ray(p(1.0, 0.0, 0.0), p(0.0, 0.0, 1.0)),
            std::slice::from_ref(&right),
            &options,
        )
        .unwrap()
        .hit
        .unwrap();
        assert_ne!(
            a.source.sub_element, b.source.sub_element,
            "two faces of one mesh select differently"
        );
        assert_ne!(a.source, b.source);
    }

    #[test]
    fn line_hits_have_no_sub_element_and_no_reason() {
        let items = vec![item(
            1,
            SemanticGeometry::Line {
                start: p(5.0, -1.0, 0.0),
                end: p(5.0, 1.0, 0.0),
            },
        )];
        let r = ray(p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0));
        let hit = pick_closest(&r, &items, &PickOptions::new(1e-6).unwrap())
            .unwrap()
            .hit
            .unwrap();
        assert!(hit.source.sub_element.is_none());
        assert!(
            hit.sub_element_reason.is_none(),
            "lines are not face-addressable"
        );
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
