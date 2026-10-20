use cad_domain::*;
use cad_geometry::{tessellate_geometry, TessellationParams};

use crate::PickHit;

use super::bounds::{
    bounds_culling_enabled, cached_local_pick_bounds, conservative_world_bounds,
    ray_intersects_expanded_bounds, PickBoundsCache,
};
use super::math::{add, scale3};
use super::ray::{ray_segment_closest, ray_triangle, validate_ray};
use super::types::{
    GeometryHit, PickItem, PickOptions, PickOutcome, PickReport, SkippedGeometry,
    MAX_PICK_SEGMENTS, REASON_FACE_SOURCES_ABSENT, REASON_FACE_SOURCE_MISSING,
    REASON_FACE_SOURCE_OUT_OF_RANGE,
};

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
        // A dimension is picked like a compound: recurse into its display
        // children (ADR 0017 A.8).
        SemanticGeometry::Dimension(_) => FAMILY_COMPOUND,
        // A hatch is picked like a compound: its display holds the boundary
        // polylines and the fill mesh, so clicking inside the fill hits the
        // mesh (ADR 0018 A.2/E).
        SemanticGeometry::Hatch(_) => FAMILY_COMPOUND,
        // A multileader is picked like the compound it used to be: any leader
        // path, arrowhead, dogleg or text hit selects the whole entity
        // (ADR 0019 decision G).
        SemanticGeometry::MultiLeader(_) => FAMILY_COMPOUND,
        SemanticGeometry::Text { .. } => FAMILY_TEXT,
        SemanticGeometry::Shape { .. } => FAMILY_SHAPE,
        SemanticGeometry::Mask { .. } => FAMILY_MASK,
        SemanticGeometry::Image { .. } => FAMILY_IMAGE,
    }
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
                super::CULLED_BY_PICK_BOUNDS.with(|count| count.set(count.get() + 1));
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
    super::PICK_NARROW_CALLS.with(|count| count.set(count.get() + 1));
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
        // A dimension picks through its display children, exactly like a
        // compound (ADR 0017 A.8).
        SemanticGeometry::Dimension(dimension) => {
            let mut best: Option<GeometryHit> = None;
            let mut unsupported: Option<&'static str> = None;
            for child in dimension.display() {
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
                Some(hit) => PickOutcome::Hit(GeometryHit {
                    geometry_source: GeometrySource::Analytic,
                    ..hit
                }),
                None if unsupported.is_some() => PickOutcome::Unsupported("dimension-unsupported"),
                None => PickOutcome::Miss,
            }
        }
        // A hatch picks through its display children exactly like a compound,
        // so the fill mesh (and therefore the interior) stays selectable
        // (ADR 0018 E).
        SemanticGeometry::Hatch(hatch) => {
            let mut best: Option<GeometryHit> = None;
            let mut unsupported: Option<&'static str> = None;
            for child in hatch.display() {
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
                Some(hit) => PickOutcome::Hit(GeometryHit {
                    geometry_source: GeometrySource::Analytic,
                    ..hit
                }),
                None if unsupported.is_some() => PickOutcome::Unsupported("hatch-unsupported"),
                None => PickOutcome::Miss,
            }
        }
        // A multileader picks through its display children exactly like the
        // compound it used to be, so any leader path/arrowhead/dogleg/text hit
        // selects the whole entity (ADR 0019 decision G).
        SemanticGeometry::MultiLeader(multileader) => {
            let mut best: Option<GeometryHit> = None;
            let mut unsupported: Option<&'static str> = None;
            for child in multileader.display() {
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
                Some(hit) => PickOutcome::Hit(GeometryHit {
                    geometry_source: GeometrySource::Analytic,
                    ..hit
                }),
                None if unsupported.is_some() => {
                    PickOutcome::Unsupported("multileader-unsupported")
                }
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
pub(super) fn segment_hit(
    ray: &Ray3,
    a: Point3,
    b: Point3,
    tolerance: f64,
) -> Option<(f64, f64, Point3)> {
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
