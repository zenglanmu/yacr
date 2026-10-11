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
use std::collections::HashMap;

use crate::PickHit;

#[cfg(test)]
thread_local! {
    static CULLED_BY_PICK_BOUNDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static BYPASS_PICK_BOUNDS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PICK_BOUNDS_COMPUTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PICK_NARROW_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

type PickBoundsCache = HashMap<*const SemanticGeometry, Option<(Point3, Point3)>>;

/// Ray directions shorter than this are considered degenerate.
pub const MIN_RAY_DIRECTION: f64 = 1e-12;
/// Maximum number of tessellation segments for one curve during picking, so a
/// pathological curve can never make a single pick unbounded.
pub const MAX_PICK_SEGMENTS: usize = 8192;

const FAMILY_LINE: usize = 0;
const FAMILY_POLYLINE: usize = 1;
const FAMILY_CIRCLE: usize = 2;
const FAMILY_ARC: usize = 3;
const FAMILY_ELLIPSE: usize = 4;
const FAMILY_SPLINE: usize = 5;
const FAMILY_POINT: usize = 6;
const FAMILY_MESH: usize = 7;
const FAMILY_INSERT: usize = 8;
const FAMILY_OPAQUE: usize = 9;
const FAMILY_COMPOUND: usize = 10;
const FAMILY_TEXT: usize = 11;
const FAMILY_SHAPE: usize = 12;
const FAMILY_MASK: usize = 13;
const FAMILY_IMAGE: usize = 14;
const PICK_FAMILIES: [&str; 15] = [
    "line", "polyline", "circle", "arc", "ellipse", "spline", "point", "mesh", "insert", "opaque",
    "compound", "text", "shape", "mask", "image",
];

struct PickProfile {
    visits: [usize; PICK_FAMILIES.len()],
    culls: [usize; PICK_FAMILIES.len()],
    narrow: [usize; PICK_FAMILIES.len()],
    total_visits: usize,
    bounds_samples: usize,
    bounds_ns: u128,
    precise_samples: usize,
    precise_ns: u128,
}

impl Default for PickProfile {
    fn default() -> Self {
        Self {
            visits: [0; PICK_FAMILIES.len()],
            culls: [0; PICK_FAMILIES.len()],
            narrow: [0; PICK_FAMILIES.len()],
            total_visits: 0,
            bounds_samples: 0,
            bounds_ns: 0,
            precise_samples: 0,
            precise_ns: 0,
        }
    }
}

impl PickProfile {
    fn visit(&mut self, family: usize) -> bool {
        self.visits[family] += 1;
        self.total_visits += 1;
        self.total_visits % 64 == 1
    }

    fn bounds_sample(&mut self, elapsed: std::time::Duration) {
        self.bounds_samples += 1;
        self.bounds_ns += elapsed.as_nanos();
    }

    fn precise_sample(&mut self, elapsed: std::time::Duration) {
        self.precise_samples += 1;
        self.precise_ns += elapsed.as_nanos();
    }

    fn log(&self) {
        let families = PICK_FAMILIES
            .iter()
            .enumerate()
            .filter(|(index, _)| self.visits[*index] != 0)
            .map(|(index, name)| {
                format!(
                    "{name}={}/{}/{}",
                    self.visits[index], self.culls[index], self.narrow[index]
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        eprintln!(
            "YACR_PROFILE_PICK spatial families=visits/culls/narrow[{families}] sample_every=64 bounds_samples={} bounds_sample_ms={:.3} precise_samples={} precise_sample_ms={:.3}",
            self.bounds_samples,
            self.bounds_ns as f64 / 1_000_000.0,
            self.precise_samples,
            self.precise_ns as f64 / 1_000_000.0,
        );
    }
}

fn pick_profile_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("YACR_PROFILE_PICK").is_some())
}

fn geometry_family(geometry: &SemanticGeometry) -> usize {
    match geometry {
        SemanticGeometry::Line { .. } => FAMILY_LINE,
        SemanticGeometry::Polyline { .. } => FAMILY_POLYLINE,
        SemanticGeometry::Circle { .. } => FAMILY_CIRCLE,
        SemanticGeometry::Arc { .. } => FAMILY_ARC,
        SemanticGeometry::Ellipse { .. } => FAMILY_ELLIPSE,
        SemanticGeometry::Spline { .. } => FAMILY_SPLINE,
        SemanticGeometry::Point(_) => FAMILY_POINT,
        SemanticGeometry::Mesh(_) => FAMILY_MESH,
        SemanticGeometry::Insert { .. } => FAMILY_INSERT,
        SemanticGeometry::Opaque { .. } => FAMILY_OPAQUE,
        SemanticGeometry::Compound(_) => FAMILY_COMPOUND,
        SemanticGeometry::Text { .. } => FAMILY_TEXT,
        SemanticGeometry::Shape { .. } => FAMILY_SHAPE,
        SemanticGeometry::Mask { .. } => FAMILY_MASK,
        SemanticGeometry::Image { .. } => FAMILY_IMAGE,
    }
}

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
    let mut bounds_cache = PickBoundsCache::new();
    hit_validated(
        ray,
        geometry,
        transform,
        options,
        params,
        None,
        &mut bounds_cache,
    )
}

fn hit_validated(
    ray: &Ray3,
    geometry: &SemanticGeometry,
    transform: &Transform3,
    options: &PickOptions,
    params: TessellationParams,
    mut profile: Option<&mut PickProfile>,
    bounds_cache: &mut PickBoundsCache,
) -> CadResult<PickOutcome> {
    let (family, sample) = if let Some(profile) = profile.as_deref_mut() {
        let family = geometry_family(geometry);
        (Some(family), profile.visit(family))
    } else {
        (None, false)
    };
    let bounds_started = sample.then(std::time::Instant::now);
    // Cheap conservative rejection before curve tessellation or mesh vertex
    // transformation. Unknown/invalid bounds deliberately fall through.
    if bounds_culling_enabled() {
        if let Some((min, max)) =
            conservative_world_bounds(cached_local_pick_bounds(geometry, bounds_cache), transform)
        {
            if !ray_intersects_expanded_bounds(ray, min, max, options.tolerance) {
                if let Some(started) = bounds_started {
                    if let Some(profile) = profile.as_deref_mut() {
                        profile.bounds_sample(started.elapsed());
                    }
                }
                if let (Some(profile), Some(family)) = (profile.as_deref_mut(), family) {
                    profile.culls[family] += 1;
                }
                #[cfg(test)]
                CULLED_BY_PICK_BOUNDS.with(|count| count.set(count.get() + 1));
                return Ok(PickOutcome::Miss);
            }
        }
    }
    if let Some(started) = bounds_started {
        if let Some(profile) = profile.as_deref_mut() {
            profile.bounds_sample(started.elapsed());
        }
    }
    if let (Some(profile), Some(family)) = (profile.as_deref_mut(), family) {
        profile.narrow[family] += 1;
    }
    #[cfg(test)]
    PICK_NARROW_CALLS.with(|count| count.set(count.get() + 1));
    let narrow_started = (sample && family != Some(FAMILY_COMPOUND)).then(std::time::Instant::now);
    let outcome = match geometry {
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
                match hit_validated(
                    ray,
                    child,
                    transform,
                    options,
                    params,
                    profile.as_deref_mut(),
                    bounds_cache,
                )? {
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
    };
    if let Some(started) = narrow_started {
        if let Some(profile) = profile {
            profile.precise_sample(started.elapsed());
        }
    }
    Ok(outcome)
}

#[cfg(not(test))]
fn bounds_culling_enabled() -> bool {
    true
}

#[cfg(test)]
fn bounds_culling_enabled() -> bool {
    !BYPASS_PICK_BOUNDS.with(std::cell::Cell::get)
}

/// A deliberately conservative, low-cost local bound for families whose
/// geometry is known to remain within it. Returns `None` for uncertain data.
fn local_pick_bounds(geometry: &SemanticGeometry) -> Option<(Point3, Point3)> {
    let mut bounds: Option<(Point3, Point3)> = None;
    let mut radius = 0.0;
    match geometry {
        SemanticGeometry::Line { start, end } => {
            if !include_bound_point(&mut bounds, *start) || !include_bound_point(&mut bounds, *end)
            {
                return None;
            }
        }
        SemanticGeometry::Point(point) => {
            if !include_bound_point(&mut bounds, *point) {
                return None;
            }
        }
        SemanticGeometry::Shape { position, .. } => {
            if !include_bound_point(&mut bounds, *position) {
                return None;
            }
        }
        // The triangle predicate intentionally permits 1e-12 barycentric
        // slack; for very large triangles that can accept points outside even
        // a tolerance-expanded vertex bound. Preserve that established result.
        SemanticGeometry::Mesh(_) => return None,
        SemanticGeometry::Polyline {
            points: vertices,
            bulges,
            ..
        } if bulges.iter().all(|bulge| *bulge == 0.0) => {
            for point in vertices {
                if !include_bound_point(&mut bounds, *point) {
                    return None;
                }
            }
        }
        SemanticGeometry::Mask { boundary, .. } => {
            for point in boundary {
                if !include_bound_point(&mut bounds, *point) {
                    return None;
                }
            }
        }
        SemanticGeometry::Circle {
            center, radius: r, ..
        }
        | SemanticGeometry::Arc {
            center, radius: r, ..
        } => {
            if !include_bound_point(&mut bounds, *center) {
                return None;
            }
            radius = r.abs();
        }
        SemanticGeometry::Ellipse {
            center,
            major_axis,
            ratio,
            ..
        } => {
            if !include_bound_point(&mut bounds, *center) {
                return None;
            }
            radius = length3(*major_axis) * (1.0 + ratio.abs());
        }
        // Rational splines can leave the control-point hull for invalid or
        // non-positive weights; text bounds depend on shaping/font metrics.
        // Bulged polylines likewise need their true arc extrema.
        _ => return None,
    }
    let (min, max) = bounds?;
    if !radius.is_finite() {
        return None;
    }
    Some((
        Point3 {
            x: min.x - radius,
            y: min.y - radius,
            z: min.z - radius,
        },
        Point3 {
            x: max.x + radius,
            y: max.y + radius,
            z: max.z + radius,
        },
    ))
}

fn include_bound_point(bounds: &mut Option<(Point3, Point3)>, point: Point3) -> bool {
    if !is_finite_point(point) {
        return false;
    }
    match bounds {
        Some((min, max)) => {
            min.x = min.x.min(point.x);
            min.y = min.y.min(point.y);
            min.z = min.z.min(point.z);
            max.x = max.x.max(point.x);
            max.y = max.y.max(point.y);
            max.z = max.z.max(point.z);
        }
        None => *bounds = Some((point, point)),
    }
    true
}

fn cached_local_pick_bounds(
    geometry: &SemanticGeometry,
    cache: &mut PickBoundsCache,
) -> Option<(Point3, Point3)> {
    let SemanticGeometry::Compound(children) = geometry else {
        // Leaf bounds are cheap and common. Avoid hashing and inserting millions
        // of one-shot leaf pointers; only reusable compound aggregates belong
        // in the query cache.
        return local_pick_bounds(geometry);
    };
    let key = geometry as *const SemanticGeometry;
    if let Some(bounds) = cache.get(&key) {
        return *bounds;
    }
    #[cfg(test)]
    PICK_BOUNDS_COMPUTED.with(|count| count.set(count.get() + 1));

    let mut union: Option<(Point3, Point3)> = None;
    let mut all_bounded = !children.is_empty();
    for child in children {
        let Some((child_min, child_max)) = cached_local_pick_bounds(child, cache) else {
            all_bounded = false;
            break;
        };
        union = Some(match union {
            Some((mut min, mut max)) => {
                min.x = min.x.min(child_min.x);
                min.y = min.y.min(child_min.y);
                min.z = min.z.min(child_min.z);
                max.x = max.x.max(child_max.x);
                max.y = max.y.max(child_max.y);
                max.z = max.z.max(child_max.z);
                (min, max)
            }
            None => (child_min, child_max),
        });
    }
    let bounds = if all_bounded { union } else { None };
    cache.insert(key, bounds);
    bounds
}

fn conservative_world_bounds(
    local_bounds: Option<(Point3, Point3)>,
    transform: &Transform3,
) -> Option<(Point3, Point3)> {
    let (min, max) = local_bounds?;
    let mut world_min = Point3 {
        x: f64::INFINITY,
        y: f64::INFINITY,
        z: f64::INFINITY,
    };
    let mut world_max = Point3 {
        x: f64::NEG_INFINITY,
        y: f64::NEG_INFINITY,
        z: f64::NEG_INFINITY,
    };
    for x in [min.x, max.x] {
        for y in [min.y, max.y] {
            for z in [min.z, max.z] {
                let point = transform.apply_point(Point3 { x, y, z });
                if !is_finite_point(point) {
                    return None;
                }
                world_min.x = world_min.x.min(point.x);
                world_min.y = world_min.y.min(point.y);
                world_min.z = world_min.z.min(point.z);
                world_max.x = world_max.x.max(point.x);
                world_max.y = world_max.y.max(point.y);
                world_max.z = world_max.z.max(point.z);
            }
        }
    }
    Some((world_min, world_max))
}

/// Slab test against an AABB expanded by the precise pick tolerance. Invalid
/// arithmetic returns `true` (exhaustive fallback), never a false miss.
fn ray_intersects_expanded_bounds(ray: &Ray3, min: Point3, max: Point3, tolerance: f64) -> bool {
    let mut near: f64 = 0.0;
    let mut far = f64::INFINITY;
    for (origin, direction, low, high) in [
        (
            ray.origin.x,
            ray.direction.x,
            min.x - tolerance,
            max.x + tolerance,
        ),
        (
            ray.origin.y,
            ray.direction.y,
            min.y - tolerance,
            max.y + tolerance,
        ),
        (
            ray.origin.z,
            ray.direction.z,
            min.z - tolerance,
            max.z + tolerance,
        ),
    ] {
        if ![origin, direction, low, high].iter().all(|v| v.is_finite()) {
            return true;
        }
        if direction == 0.0 {
            if origin < low || origin > high {
                return false;
            }
        } else {
            let a = (low - origin) / direction;
            let b = (high - origin) / direction;
            if !a.is_finite() || !b.is_finite() {
                return true;
            }
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return false;
            }
        }
    }
    far >= 0.0
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
    let mut profile = pick_profile_enabled().then(PickProfile::default);
    let mut bounds_cache = PickBoundsCache::new();
    for (item_source, geometry, transform) in items {
        match hit_validated(
            ray,
            geometry,
            transform,
            options,
            params,
            profile.as_mut(),
            &mut bounds_cache,
        )? {
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
    if let Some(profile) = profile {
        profile.log();
    }
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
    #[cfg(test)]
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

    fn local_pick_bounds_reference(geometry: &SemanticGeometry) -> Option<(Point3, Point3)> {
        let mut points = Vec::new();
        let mut radius = 0.0;
        match geometry {
            SemanticGeometry::Line { start, end } => points.extend([*start, *end]),
            SemanticGeometry::Point(point) => points.push(*point),
            SemanticGeometry::Shape { position, .. } => points.push(*position),
            SemanticGeometry::Mesh(_) => return None,
            SemanticGeometry::Polyline {
                points: vertices,
                bulges,
                ..
            } if bulges.iter().all(|bulge| *bulge == 0.0) => {
                points.extend(vertices.iter().copied())
            }
            SemanticGeometry::Mask { boundary, .. } => points.extend(boundary.iter().copied()),
            SemanticGeometry::Circle {
                center, radius: r, ..
            }
            | SemanticGeometry::Arc {
                center, radius: r, ..
            } => {
                points.push(*center);
                radius = r.abs();
            }
            SemanticGeometry::Ellipse {
                center,
                major_axis,
                ratio,
                ..
            } => {
                points.push(*center);
                radius = length3(*major_axis) * (1.0 + ratio.abs());
            }
            _ => return None,
        }
        if points.is_empty() || !radius.is_finite() || points.iter().any(|p| !is_finite_point(*p)) {
            return None;
        }
        let mut min = points[0];
        let mut max = points[0];
        for point in points.iter().skip(1) {
            min.x = min.x.min(point.x);
            min.y = min.y.min(point.y);
            min.z = min.z.min(point.z);
            max.x = max.x.max(point.x);
            max.y = max.y.max(point.y);
            max.z = max.z.max(point.z);
        }
        Some((
            p(min.x - radius, min.y - radius, min.z - radius),
            p(max.x + radius, max.y + radius, max.z + radius),
        ))
    }

    #[test]
    fn allocation_free_pick_bounds_match_the_vec_reference() {
        let cases = vec![
            SemanticGeometry::Line {
                start: p(-2.0, 1.0, 3.0),
                end: p(4.0, -1.0, 2.0),
            },
            SemanticGeometry::Point(p(1.0, 2.0, 3.0)),
            SemanticGeometry::Shape {
                shape_name: "shape".into(),
                code: 1,
                position: p(3.0, 2.0, 1.0),
                size: 1.0,
                rotation: 0.0,
                font: None,
            },
            SemanticGeometry::Polyline {
                points: vec![p(1.0, 2.0, 3.0), p(-2.0, 4.0, 0.0)],
                bulges: vec![0.0],
                closed: false,
            },
            SemanticGeometry::Polyline {
                points: vec![p(1.0, 2.0, 3.0), p(-2.0, 4.0, 0.0)],
                bulges: vec![1.0],
                closed: false,
            },
            SemanticGeometry::Mask {
                boundary: vec![p(1.0, 2.0, 3.0), p(-2.0, 4.0, 0.0)],
                inverted: false,
            },
            SemanticGeometry::Circle {
                center: p(1.0, 2.0, 3.0),
                normal: p(0.0, 0.0, 1.0),
                radius: -4.0,
            },
            SemanticGeometry::Arc {
                center: p(1.0, 2.0, 3.0),
                normal: p(0.0, 0.0, 1.0),
                radius: 4.0,
                start: 0.0,
                sweep: 1.0,
            },
            SemanticGeometry::Ellipse {
                center: p(1.0, 2.0, 3.0),
                normal: p(0.0, 0.0, 1.0),
                major_axis: p(2.0, 0.0, 0.0),
                ratio: 0.5,
                start: 0.0,
                sweep: 1.0,
            },
            SemanticGeometry::Polyline {
                points: vec![p(f64::NAN, 0.0, 0.0)],
                bulges: vec![],
                closed: false,
            },
            SemanticGeometry::Mesh(Mesh {
                vertices: vec![p(0.0, 0.0, 0.0)],
                triangles: vec![],
                normals: vec![],
                face_sources: vec![],
                colors: vec![],
            }),
            SemanticGeometry::Opaque {
                type_key: "x".into(),
                version: 1,
                payload: vec![],
            },
        ];
        for geometry in &cases {
            assert_eq!(
                local_pick_bounds(geometry),
                local_pick_bounds_reference(geometry)
            );
        }
    }

    #[test]
    fn bounded_and_unculled_pick_reports_are_identical() {
        let mut first_tie = item(
            10,
            SemanticGeometry::Line {
                start: p(-1.0, 0.0, 3.0),
                end: p(1.0, 0.0, 3.0),
            },
        );
        first_tie.source.instance = InstancePath(vec![EntityId(101)]);
        let mut second_tie = first_tie.clone();
        second_tie.source.instance = InstancePath(vec![EntityId(102)]);
        let mut transformed = item(11, SemanticGeometry::Point(p(0.0, 0.0, 5.0)));
        transformed.transform.matrix[0][3] = 0.05;
        let items = vec![
            item(
                1,
                SemanticGeometry::Circle {
                    center: p(100.0, 0.0, 20.0),
                    normal: p(0.0, 0.0, 1.0),
                    radius: 2.0,
                },
            ),
            first_tie,
            second_tie,
            transformed,
            item(
                12,
                SemanticGeometry::Mask {
                    boundary: vec![p(-1.0, 0.0, 7.0), p(1.0, 0.0, 7.0)],
                    inverted: false,
                },
            ),
            item(
                13,
                SemanticGeometry::Opaque {
                    type_key: "UNKNOWN".into(),
                    version: 1,
                    payload: vec![],
                },
            ),
            item(
                14,
                SemanticGeometry::Mesh(Mesh {
                    vertices: vec![p(0.0, 0.0, 10.0), p(1e9, 0.0, 10.0), p(0.0, 1e9, 10.0)],
                    triangles: vec![[0, 1, 2]],
                    normals: vec![],
                    face_sources: vec![],
                    colors: vec![],
                }),
            ),
        ];
        let ray = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
        let options = PickOptions::new(0.1)
            .unwrap()
            .with_back_faces(BackFacePolicy::DoubleSided);
        BYPASS_PICK_BOUNDS.with(|bypass| bypass.set(false));
        let bounded = pick_closest(&ray, &items, &options).unwrap();
        BYPASS_PICK_BOUNDS.with(|bypass| bypass.set(true));
        let exhaustive = pick_closest(&ray, &items, &options).unwrap();
        BYPASS_PICK_BOUNDS.with(|bypass| bypass.set(false));
        assert_eq!(bounded, exhaustive);
        assert_eq!(
            bounded.hit.unwrap().source.instance,
            InstancePath(vec![EntityId(101)])
        );
    }

    #[test]
    fn bounded_compound_cull_skips_children_and_shared_placements_reuse_local_bounds() {
        let ray = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
        let options = PickOptions::new(0.01).unwrap();
        let far = SemanticGeometry::Compound(vec![
            SemanticGeometry::Polyline {
                points: vec![p(100.0, -2.0, 5.0), p(100.0, 2.0, 5.0)],
                bulges: vec![0.0],
                closed: false,
            },
            SemanticGeometry::Circle {
                center: p(100.0, 0.0, 10.0),
                normal: p(0.0, 0.0, 1.0),
                radius: 1.0,
            },
        ]);
        let far_source = source(20, Vec::new());
        let identity = Transform3::identity();
        PICK_NARROW_CALLS.with(|count| count.set(0));
        PICK_BOUNDS_COMPUTED.with(|count| count.set(0));
        CULLED_BY_PICK_BOUNDS.with(|count| count.set(0));
        assert!(
            pick_closest_borrowed(&ray, [(&far_source, &far, &identity)], &options,)
                .unwrap()
                .hit
                .is_none()
        );
        PICK_BOUNDS_COMPUTED.with(|count| assert_eq!(count.get(), 1));
        PICK_NARROW_CALLS.with(|count| assert_eq!(count.get(), 0));
        CULLED_BY_PICK_BOUNDS.with(|count| assert_eq!(count.get(), 1));

        // The same immutable geometry is offered at two placements in one
        // query. First placement culls the compound; the second hits and must
        // reuse bounds for the root and both descendants.
        let shared = SemanticGeometry::Compound(vec![
            SemanticGeometry::Compound(vec![SemanticGeometry::Polyline {
                points: vec![p(-1.0, 0.0, 2.0), p(1.0, 0.0, 2.0)],
                bulges: vec![0.0],
                closed: false,
            }]),
            SemanticGeometry::Circle {
                center: p(0.0, 0.0, 4.0),
                normal: p(0.0, 0.0, 1.0),
                radius: 0.5,
            },
        ]);
        let mut aggregate_cache = PickBoundsCache::new();
        assert!(cached_local_pick_bounds(&shared, &mut aggregate_cache).is_some());
        assert_eq!(
            aggregate_cache.len(),
            2,
            "only the two compounds are cached"
        );
        assert!(cached_local_pick_bounds(&shared, &mut aggregate_cache).is_some());
        assert_eq!(aggregate_cache.len(), 2, "a repeated aggregate is reused");
        let mut displaced = Transform3::identity();
        displaced.matrix[0][3] = 100.0;
        let source_a = source(21, vec![1]);
        let source_b = source(21, vec![2]);
        PICK_NARROW_CALLS.with(|count| count.set(0));
        PICK_BOUNDS_COMPUTED.with(|count| count.set(0));
        CULLED_BY_PICK_BOUNDS.with(|count| count.set(0));
        let report = pick_closest_borrowed(
            &ray,
            [
                (&source_a, &shared, &displaced),
                (&source_b, &shared, &identity),
            ],
            &options,
        )
        .unwrap();
        assert!(report.hit.is_some());
        PICK_BOUNDS_COMPUTED.with(|count| assert_eq!(count.get(), 2));
        PICK_NARROW_CALLS.with(|count| assert_eq!(count.get(), 4));
        CULLED_BY_PICK_BOUNDS.with(|count| assert_eq!(count.get(), 1));
    }

    #[test]
    fn cached_compound_picks_match_unculled_shared_nested_and_mixed_reports() {
        let ray = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
        let options = PickOptions::new(0.1).unwrap();
        let shared = SemanticGeometry::Compound(vec![SemanticGeometry::Compound(vec![
            SemanticGeometry::Polyline {
                points: vec![p(-1.0, 0.0, 2.0), p(1.0, 0.0, 2.0)],
                bulges: vec![0.0],
                closed: false,
            },
        ])]);
        let mut displaced = Transform3::identity();
        displaced.matrix[1][3] = 0.05;
        let mut far = Transform3::identity();
        far.matrix[0][3] = 100.0;
        let source_far = source(30, vec![200]);
        let source_offset = source(30, vec![201]);
        let source_first_tie = source(30, vec![202]);
        let source_second_tie = source(30, vec![203]);
        let identity = Transform3::identity();
        let opaque_mixed = SemanticGeometry::Compound(vec![
            SemanticGeometry::Line {
                start: p(100.0, -1.0, 4.0),
                end: p(100.0, 1.0, 4.0),
            },
            SemanticGeometry::Opaque {
                type_key: "UNKNOWN".into(),
                version: 1,
                payload: vec![],
            },
        ]);
        let source_opaque = source(31, vec![]);
        let items = [
            (&source_far, &shared, &far),
            (&source_offset, &shared, &displaced),
            (&source_first_tie, &shared, &identity),
            (&source_second_tie, &shared, &identity),
            (&source_opaque, &opaque_mixed, &identity),
        ];
        BYPASS_PICK_BOUNDS.with(|bypass| bypass.set(false));
        let bounded = pick_closest_borrowed(&ray, items, &options).unwrap();
        BYPASS_PICK_BOUNDS.with(|bypass| bypass.set(true));
        let exhaustive = pick_closest_borrowed(&ray, items, &options).unwrap();
        BYPASS_PICK_BOUNDS.with(|bypass| bypass.set(false));
        assert_eq!(bounded, exhaustive);
        assert_eq!(
            bounded.hit.as_ref().unwrap().source.instance,
            InstancePath(vec![EntityId(202)])
        );
        assert_eq!(bounded.skipped.len(), 1);
        assert_eq!(bounded.skipped[0].reason, "compound-unsupported");
    }

    #[test]
    fn cached_mixed_mesh_compound_preserves_face_identity_vs_unculled_pick() {
        let geometry = SemanticGeometry::Compound(vec![
            SemanticGeometry::Mesh(two_face_mesh()),
            SemanticGeometry::Opaque {
                type_key: "UNKNOWN".into(),
                version: 1,
                payload: vec![],
            },
        ]);
        let source = source(40, vec![401]);
        let transform = Transform3::identity();
        let ray = ray(p(-1.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
        let options = PickOptions::new(1e-6)
            .unwrap()
            .with_back_faces(BackFacePolicy::DoubleSided);
        let items = [(&source, &geometry, &transform)];
        BYPASS_PICK_BOUNDS.with(|bypass| bypass.set(false));
        let bounded = pick_closest_borrowed(&ray, items, &options).unwrap();
        BYPASS_PICK_BOUNDS.with(|bypass| bypass.set(true));
        let exhaustive = pick_closest_borrowed(&ray, items, &options).unwrap();
        BYPASS_PICK_BOUNDS.with(|bypass| bypass.set(false));
        assert_eq!(bounded, exhaustive);
        assert_eq!(
            bounded.hit.as_ref().unwrap().source.sub_element,
            Some(face("face-a"))
        );
    }

    #[test]
    fn conservative_bounds_cull_far_curves_but_preserve_nested_unsupported_children() {
        CULLED_BY_PICK_BOUNDS.with(|count| count.set(0));
        let ray = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
        let options = PickOptions::new(0.01).unwrap();
        let far_circle = SemanticGeometry::Circle {
            center: p(100.0, 0.0, 10.0),
            normal: p(0.0, 0.0, 1.0),
            radius: 20.0,
        };
        let compound = SemanticGeometry::Compound(vec![
            far_circle,
            SemanticGeometry::Opaque {
                type_key: "UNKNOWN".into(),
                version: 1,
                payload: Vec::new(),
            },
        ]);
        assert_eq!(
            hit_geometry(&ray, &compound, &Transform3::identity(), &options).unwrap(),
            PickOutcome::Unsupported("compound-unsupported")
        );
        CULLED_BY_PICK_BOUNDS.with(|count| assert_eq!(count.get(), 1));
    }

    #[test]
    fn conservative_bounds_keep_tolerance_hits_and_fall_back_for_bulged_polylines() {
        let ray = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
        let options = PickOptions::new(0.1).unwrap();
        let near = SemanticGeometry::Circle {
            center: p(0.15, 0.0, 3.0),
            normal: p(0.0, 0.0, 1.0),
            radius: 0.05,
        };
        assert!(matches!(
            hit_geometry(&ray, &near, &Transform3::identity(), &options).unwrap(),
            PickOutcome::Hit(_)
        ));
        let bulged = SemanticGeometry::Polyline {
            points: vec![p(100.0, 0.0, 0.0), p(101.0, 0.0, 0.0)],
            bulges: vec![1.0],
            closed: false,
        };
        assert!(local_pick_bounds(&bulged).is_none());
        assert!(local_pick_bounds(&SemanticGeometry::Opaque {
            type_key: "UNKNOWN".into(),
            version: 1,
            payload: Vec::new()
        })
        .is_none());
    }

    #[test]
    fn conservative_bounds_account_for_transforms_and_mesh_vertices() {
        let mut transform = Transform3::identity();
        transform.matrix[0][3] = 20.0;
        let point = SemanticGeometry::Point(p(0.0, 0.0, 2.0));
        let ray = ray(p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0));
        let options = PickOptions::new(0.01).unwrap();
        assert_eq!(
            hit_geometry(&ray, &point, &transform, &options).unwrap(),
            PickOutcome::Miss
        );

        let mesh = SemanticGeometry::Mesh(Mesh {
            vertices: vec![p(30.0, 0.0, 1.0)],
            triangles: Vec::new(),
            normals: Vec::new(),
            face_sources: Vec::new(),
            colors: Vec::new(),
        });
        assert_eq!(
            hit_geometry(&ray, &mesh, &Transform3::identity(), &options).unwrap(),
            PickOutcome::Miss
        );
    }

    #[test]
    fn huge_mesh_preserves_barycentric_slack_outside_tolerance_bounds() {
        // The existing triangle test allows u down to -1e-12. Across an edge
        // 1e9 units long, this accepts a point 5e-7 units outside the triangle,
        // even though the pick tolerance is only 1e-9.
        let mesh = SemanticGeometry::Mesh(Mesh {
            vertices: vec![p(0.0, 0.0, 1.0), p(1e9, 0.0, 1.0), p(0.0, 1e9, 1.0)],
            triangles: vec![[0, 1, 2]],
            normals: Vec::new(),
            face_sources: Vec::new(),
            colors: Vec::new(),
        });
        assert!(local_pick_bounds(&mesh).is_none());
        let ray = ray(p(-5e-7, 1.0, 0.0), p(0.0, 0.0, 1.0));
        let options = PickOptions::new(1e-9)
            .unwrap()
            .with_back_faces(BackFacePolicy::DoubleSided);
        assert!(matches!(
            hit_geometry(&ray, &mesh, &Transform3::identity(), &options).unwrap(),
            PickOutcome::Hit(_)
        ));
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
