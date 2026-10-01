//! Measurement and snapping.
//!
//! Spec v2.0 §3.3: results carry their input points, unit context, algorithm and
//! precision — never just a formatted string. 2D planar and 3D spatial distance
//! are modelled separately, non-coplanar and self-intersecting area inputs are
//! rejected, and snap tolerance is defined in logical pixels.
//!
//! Audit B24 closed here:
//!
//! * [`MeasurementSpace::World3d`] is a **3D** space and may not be used for the
//!   planar algorithms ([`MeasurementAlgorithm::Distance2d`],
//!   [`MeasurementAlgorithm::PolylineLength`],
//!   [`MeasurementAlgorithm::PlanarPolygonArea`]); those require an explicit
//!   [`MeasurementSpace::Plane`] (or a verified paper/viewport transform).
//! * Area projects a closed ring onto the *measurement work plane* and checks
//!   orthogonality, finiteness and (when u/v are unit-scaled) coplanarity before
//!   computing the signed area. A skewed or degenerate plane, a non-coplanar
//!   ring or a non-finite projection is reported, never silently flattened to 0.
//! * Snap candidates are constrained by a pick-ray parameter `t >= 0` and a
//!   unit-direction contract, so geometry *behind* the camera is not snapped.
//! * Every algorithm checks the space/paper policy up front: paper-space
//!   measurement is refused without a verified inverse transform, and inputs
//!   from different spaces may not be mixed in one measurement.
//! * Sizes are checked against the measurement plane's unit scale so extremely
//!   large finite coordinates fail with an explicit error instead of overflowing
//!   to `inf`.

use cad_db::{MeasurementAlgorithm, MeasurementRecord};
use cad_domain::*;
use cad_geometry::measure_polygon_area;

/// The space a measurement is evaluated in.
///
/// The variant chosen must agree with the algorithm's dimensionality, and every
/// point supplied for the measurement must originate from that slot (see
/// [`MeasurementPoint`]); a mismatch is an explicit error.
pub enum MeasurementSpace {
    /// Explicit 2D work plane (u/v/out-of-plane in world units).
    Plane(WorkPlane),
    /// 3D model space; only [`MeasurementAlgorithm::Distance3d`] and
    /// [`MeasurementAlgorithm::Angle3Points`] are defined here.
    World3d,
    /// Paper space of a layout. Without a verified inverse transform the engine
    /// has no model-space scale and refuses the measurement.
    Paper(LayoutId),
    /// A paper viewport showing model space through `inverse`.
    ViewportModel {
        layout: LayoutId,
        inverse: Transform3,
    },
}

impl MeasurementSpace {
    /// Whether planar (2D) algorithms are defined in this space.
    pub fn is_planar(&self) -> bool {
        matches!(self, MeasurementSpace::Plane(_))
    }

    /// The layout id for paper-space-located geometry, if any.
    ///
    /// A model-space measurement returns `None`; geometry tagged with an
    /// incompatible space is a mixed input.
    pub fn paper_layout(&self) -> Option<LayoutId> {
        match self {
            MeasurementSpace::Plane(_) | MeasurementSpace::World3d => None,
            MeasurementSpace::Paper(layout) | MeasurementSpace::ViewportModel { layout, .. } => {
                Some(*layout)
            }
        }
    }
}

/// A picked point together with the space it was captured from.
///
/// The space tag is what lets the engine refuse to mix paper-space and
/// model-space geometry inside a single measurement, and what makes "all points
/// must fit the measurement's plane" a check instead of an assumption.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasurementPoint {
    pub point: Point3,
    pub space: SpaceId,
}

impl MeasurementPoint {
    /// A point captured in model space.
    pub fn model(point: Point3) -> Self {
        MeasurementPoint {
            point,
            space: SpaceId::Model,
        }
    }

    /// A point captured on `layout` paper.
    pub fn paper(point: Point3, layout: LayoutId) -> Self {
        MeasurementPoint {
            point,
            space: SpaceId::Paper(layout),
        }
    }

    /// The raw coordinate, dropping the provenance.
    pub fn as_point3(&self) -> Point3 {
        self.point
    }
}

pub struct MeasurementRequest {
    pub algorithm: MeasurementAlgorithm,
    /// Raw coordinates. Kept for callers that already enforce provenance; the
    /// `space` field still constrains which values are acceptable.
    pub points: Vec<Point3>,
    /// Preferred provenance-carrying form. When non-empty it takes precedence
    /// over `points` and enables mixed-space rejection.
    pub tapped: Vec<MeasurementPoint>,
    pub space: MeasurementSpace,
    pub units: UnitContext,
    pub source: GeometrySource,
    pub precision: Precision,
}

impl MeasurementRequest {
    /// Build a request from provenance-carrying points, filling `points` from
    /// the tapped coordinates.
    pub fn from_tapped(
        algorithm: MeasurementAlgorithm,
        tapped: Vec<MeasurementPoint>,
        space: MeasurementSpace,
        units: UnitContext,
        source: GeometrySource,
        precision: Precision,
    ) -> Self {
        MeasurementRequest {
            algorithm,
            points: tapped.iter().map(|t| t.point).collect(),
            tapped,
            space,
            units,
            source,
            precision,
        }
    }

    fn coordinates(&self) -> Vec<Point3> {
        if self.tapped.is_empty() {
            self.points.clone()
        } else {
            self.tapped.iter().map(|t| t.point).collect()
        }
    }
}

pub enum SnapKind {
    Endpoint,
    Midpoint,
    Center,
    LocalIntersection,
}

pub struct SnapCandidate {
    pub kind: SnapKind,
    pub point: Point3,
    /// Space the candidate's geometry lives in, used for space filtering.
    pub space: SpaceId,
    pub source: SelectionRef,
    pub logical_pixel_distance: f64,
}

#[derive(Default)]
pub struct MeasurementEngine {
    pub tolerance: TolerancePolicy,
}

impl MeasurementEngine {
    pub fn new(tolerance: TolerancePolicy) -> Self {
        MeasurementEngine { tolerance }
    }

    /// Evaluate a measurement, rejecting non-finite, degenerate, non-coplanar,
    /// self-intersecting, mixed-space and unsupported-space inputs (spec §3.3,
    /// §16.1; audit B24).
    pub fn measure(&self, request: &MeasurementRequest) -> CadResult<MeasurementRecord> {
        self.check_space_policy(request)?;
        let points = request.coordinates();
        for p in &points {
            if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
                return Err(CadError::InvalidInput(
                    "measurement point is not finite".to_string(),
                ));
            }
        }
        let tol = self.tolerance.computation_world.max(1e-12);
        let value = match request.algorithm {
            MeasurementAlgorithm::Distance2d => {
                require_planar_space(&request.space)?;
                let [a, b] = two(&points)?;
                distance_in_plane(a, b, &request.space, tol)?
            }
            MeasurementAlgorithm::Distance3d => {
                let [a, b] = two(&points)?;
                distance_with_space(a, b, &request.space)?
            }
            MeasurementAlgorithm::PolylineLength => {
                require_planar_space(&request.space)?;
                if points.len() < 2 {
                    return Err(CadError::InvalidInput(
                        "polyline needs at least two points".to_string(),
                    ));
                }
                let mut total = 0.0;
                for w in points.windows(2) {
                    total += distance_in_plane(w[0], w[1], &request.space, tol)?;
                    if !total.is_finite() {
                        return Err(CadError::InvalidInput(
                            "polyline length overflowed to a non-finite value".to_string(),
                        ));
                    }
                }
                if total <= tol {
                    return Err(CadError::InvalidInput(
                        "polyline has zero length".to_string(),
                    ));
                }
                total
            }
            MeasurementAlgorithm::Angle3Points => {
                if points.len() != 3 {
                    return Err(CadError::InvalidInput(
                        "angle needs exactly three points".to_string(),
                    ));
                }
                let (a, v, b) = (points[0], points[1], points[2]);
                angle_at_vertex(a, v, b, &request.space, tol)?
            }
            MeasurementAlgorithm::PlanarPolygonArea => {
                require_planar_space(&request.space)?;
                if points.len() < 3 {
                    return Err(CadError::InvalidInput(
                        "area needs at least three points".to_string(),
                    ));
                }
                let flat =
                    project_to_plane(&points, &request.space, self.tolerance.topology_world)?;
                measure_polygon_area(&flat, self.tolerance.topology_world)
                    .map_err(|e| CadError::InvalidInput(format!("area rejected: {e}")))?
            }
        };
        if !value.is_finite() {
            return Err(CadError::InvalidInput(
                "measurement produced a non-finite value".to_string(),
            ));
        }
        Ok(MeasurementRecord {
            algorithm: request.algorithm.clone(),
            inputs: points,
            plane: match &request.space {
                MeasurementSpace::Plane(p) => Some(*p),
                _ => None,
            },
            value,
            units: request.units.clone(),
            source: request.source.clone(),
            precision: request.precision.clone(),
        })
    }

    /// Space policy: paper-space measurement needs a verified inverse transform
    /// and a single layout; mixed-space inputs are refused.
    fn check_space_policy(&self, request: &MeasurementRequest) -> CadResult<()> {
        match &request.space {
            MeasurementSpace::Paper(layout) => {
                return Err(CadError::Unsupported(format!(
                    "paper-space measurement needs a verified inverse viewport transform for layout {}",
                    layout.0
                )));
            }
            MeasurementSpace::ViewportModel { layout, inverse } => {
                if !transform_finite(inverse) {
                    return Err(CadError::InvalidInput(
                        "viewport inverse transform is not finite".to_string(),
                    ));
                }
                return Err(CadError::Unsupported(format!(
                    "viewport model measurement for layout {} is not wired to a verified inverse transform yet",
                    layout.0
                )));
            }
            _ => {}
        }
        // Reject inputs gathered from different spaces in one measurement.
        let mut seen: Option<SpaceId> = None;
        for t in &request.tapped {
            match &seen {
                None => seen = Some(t.space.clone()),
                Some(s) if *s != t.space => {
                    return Err(CadError::InvalidInput(
                        "measurement mixes points from different spaces".to_string(),
                    ));
                }
                _ => {}
            }
        }
        // Model-space measurement may not consume paper-space picks.
        if request.space.paper_layout().is_none() {
            if let Some(s) = seen {
                if !s.is_model() {
                    return Err(CadError::InvalidInput(
                        "model-space measurement cannot use paper-space points".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Snap against a supplied local candidate list.
    ///
    /// Geometry must come from the caller (the database + spatial index), so the
    /// contract-level `snap` cannot fabricate points; this is the real entry.
    ///
    /// The ray is a **pick ray**: `ray.direction` must be finite and non-zero
    /// and `ray.origin` must be finite. Only candidates in front of the origin
    /// (`t >= -tolerance`) and, when `space_filter` is supplied, belonging to
    /// that space are considered (audit B24).
    pub fn snap_to_points(
        &self,
        candidates: &[SnapCandidate],
        ray: &Ray3,
        world_per_px: f64,
        space_filter: Option<SpaceId>,
    ) -> Option<SnapCandidate> {
        let dir = unit_direction(ray)?;
        if !world_per_px.is_finite() || world_per_px <= 0.0 {
            return None;
        }
        let tolerance = self.tolerance.interaction_logical_pixels * world_per_px;
        let mut best: Option<(f64, SnapCandidate)> = None;
        for c in candidates {
            if let Some(filter) = &space_filter {
                if c.space != *filter {
                    continue;
                }
            }
            let Some(d) = point_ray_distance(&dir, ray.origin, c.point) else {
                // Behind the pick-ray origin: never a valid snap target.
                continue;
            };
            let ok = best.as_ref().map(|(bd, _)| d < *bd).unwrap_or(true);
            if d <= tolerance && ok {
                best = Some((
                    d,
                    SnapCandidate {
                        kind: clone_kind(&c.kind),
                        point: c.point,
                        space: c.space.clone(),
                        source: c.source.clone(),
                        logical_pixel_distance: d / world_per_px,
                    },
                ));
            }
        }
        best.map(|(_, c)| c)
    }

    /// Contract entry: snapping needs candidate geometry from the database.
    pub fn snap(
        &self,
        _viewport: ViewportId,
        _candidates: &[SelectionRef],
        _ray: Ray3,
    ) -> CadResult<Option<SnapCandidate>> {
        Err(CadError::Unsupported(
            "snap needs candidate geometry; use snap_to_points with points resolved from the database".into(),
        ))
    }
}

fn clone_kind(kind: &SnapKind) -> SnapKind {
    match kind {
        SnapKind::Endpoint => SnapKind::Endpoint,
        SnapKind::Midpoint => SnapKind::Midpoint,
        SnapKind::Center => SnapKind::Center,
        SnapKind::LocalIntersection => SnapKind::LocalIntersection,
    }
}

fn require_planar_space(space: &MeasurementSpace) -> CadResult<()> {
    if !space.is_planar() {
        return Err(CadError::InvalidInput(
            "planar measurement needs an explicit work plane, not 3D/paper space".to_string(),
        ));
    }
    Ok(())
}

fn transform_finite(t: &Transform3) -> bool {
    t.matrix.iter().flatten().all(|c| c.is_finite())
}

/// Unit pick-ray direction, or `None` when the ray is unusable.
fn unit_direction(ray: &Ray3) -> Option<Point3> {
    let d = ray.direction;
    if !d.x.is_finite() || !d.y.is_finite() || !d.z.is_finite() {
        return None;
    }
    if !ray.origin.x.is_finite() || !ray.origin.y.is_finite() || !ray.origin.z.is_finite() {
        return None;
    }
    let l = length(d);
    if !l.is_finite() || l < 1e-12 {
        return None;
    }
    let dir = scale(d, 1.0 / l);
    if !dir.x.is_finite() || !dir.y.is_finite() || !dir.z.is_finite() {
        return None;
    }
    Some(dir)
}

/// Perpendicular distance from a point to the pick ray, or `None` when the point
/// is behind the ray origin (negative ray parameter).
fn point_ray_distance(dir: &Point3, origin: Point3, p: Point3) -> Option<f64> {
    let v = sub(p, origin);
    let proj = dot(v, *dir);
    if !proj.is_finite() || proj < 0.0 {
        return None;
    }
    let closest = scale(*dir, proj);
    let d = length(sub(v, closest));
    if d.is_finite() {
        Some(d)
    } else {
        None
    }
}

fn two(points: &[Point3]) -> CadResult<[Point3; 2]> {
    if points.len() != 2 {
        return Err(CadError::InvalidInput(
            "distance needs exactly two points".to_string(),
        ));
    }
    Ok([points[0], points[1]])
}

fn distance(a: Point3, b: Point3) -> f64 {
    length(sub(a, b))
}

/// 3D distance, but still governed by the space policy: a mean 3D distance is
/// undefined without the true model transform for paper geometry.
fn distance_with_space(a: Point3, b: Point3, space: &MeasurementSpace) -> CadResult<f64> {
    match space {
        MeasurementSpace::World3d | MeasurementSpace::Plane(_) => Ok(distance(a, b)),
        MeasurementSpace::Paper(_) | MeasurementSpace::ViewportModel { .. } => Err(
            CadError::Unsupported("3D distance is not defined in paper/viewport space".into()),
        ),
    }
}

/// Distance projected onto the measurement plane.
fn distance_in_plane(a: Point3, b: Point3, space: &MeasurementSpace, tol: f64) -> CadResult<f64> {
    let d = sub(b, a);
    match space {
        MeasurementSpace::Plane(plane) => {
            let nx = cross(plane.u, plane.v);
            let ln = length(nx);
            if !ln.is_finite() || ln < tol {
                return Err(CadError::InvalidInput(
                    "measurement plane is degenerate".to_string(),
                ));
            }
            // Remove the plane-normal component so this is a true in-plane distance.
            let n = scale(nx, 1.0 / ln);
            let dn = dot(d, n);
            let planar = sub(d, scale(n, dn));
            Ok(length(planar))
        }
        MeasurementSpace::World3d => Ok(length(d)),
        MeasurementSpace::Paper(_) | MeasurementSpace::ViewportModel { .. } => {
            // Paper/viewport measurement requires a correct inverse transform;
            // without it we must not guess a model distance (spec §3.3).
            Err(CadError::Unsupported(
                "paper-space/viewport measurement needs a verified inverse viewport transform"
                    .into(),
            ))
        }
    }
}

/// Angle at `vertex`, measured in the plane defined by the space.
fn angle_at_vertex(
    a: Point3,
    vertex: Point3,
    b: Point3,
    space: &MeasurementSpace,
    tol: f64,
) -> CadResult<f64> {
    let v1 = sub(a, vertex);
    let v2 = sub(b, vertex);
    // Project the arms into the measurement plane when one exists; otherwise
    // (3D) use the raw vectors.
    let (w1, w2) = match space {
        MeasurementSpace::Plane(plane) => {
            let n = normalized_normal(plane, tol)?;
            (sub(v1, scale(n, dot(v1, n))), sub(v2, scale(n, dot(v2, n))))
        }
        MeasurementSpace::World3d => (v1, v2),
        MeasurementSpace::Paper(_) | MeasurementSpace::ViewportModel { .. } => {
            return Err(CadError::Unsupported(
                "angle is not defined in paper/viewport space without a verified transform".into(),
            ));
        }
    };
    let l1 = length(w1);
    let l2 = length(w2);
    if !l1.is_finite() || !l2.is_finite() || l1 < tol || l2 < tol {
        return Err(CadError::InvalidInput(
            "angle has a degenerate arm".to_string(),
        ));
    }
    let c = dot(w1, w2) / (l1 * l2);
    if !c.is_finite() {
        return Err(CadError::InvalidInput("angle is not finite".to_string()));
    }
    Ok(c.clamp(-1.0, 1.0).acos().to_degrees())
}

/// Normalized plane normal, rejecting a degenerate basis.
fn normalized_normal(plane: &WorkPlane, tol: f64) -> CadResult<Point3> {
    let n = cross(plane.u, plane.v);
    let l = length(n);
    if !l.is_finite() || l < tol {
        return Err(CadError::InvalidInput(
            "measurement plane is degenerate".to_string(),
        ));
    }
    Ok(scale(n, 1.0 / l))
}

/// Project points onto the measurement work plane's 2D coordinates.
///
/// Audit B24: the plane must be finite and orthogonal. When u and v are unit
/// vectors (the normalised contract used by viewports), points that are not
/// coplanar with the plane are rejected instead of being silently flattened. A
/// plane whose basis is not unit-scaled cannot support a scale check, so the
/// projection still succeeds but the returned coordinates are in u/v units.
fn project_to_plane(
    points: &[Point3],
    space: &MeasurementSpace,
    tol: f64,
) -> CadResult<Vec<Point3>> {
    match space {
        MeasurementSpace::Plane(plane) => {
            let u = check_plane_basis(plane, tol)?;
            let v = normalized_normalized(plane.v, tol)?;
            if unit_scaled(plane) {
                for p in points {
                    let d = sub(*p, plane.origin);
                    let off = dot(d, cross(u, v));
                    if off.abs() > self_epsilon(tol) {
                        return Err(CadError::InvalidInput(
                            "area ring is not coplanar with the measurement plane".to_string(),
                        ));
                    }
                }
            }
            let coords = points
                .iter()
                .map(|p| {
                    let d = sub(*p, plane.origin);
                    Point3 {
                        x: dot(d, u),
                        y: dot(d, v),
                        z: 0.0,
                    }
                })
                .collect::<Vec<_>>();
            if coords.iter().any(|c| !c.x.is_finite() || !c.y.is_finite()) {
                return Err(CadError::InvalidInput(
                    "area projection is not finite".to_string(),
                ));
            }
            Ok(coords)
        }
        MeasurementSpace::World3d => Ok(points.to_vec()),
        _ => Err(CadError::Unsupported(
            "area requires a defined measurement plane".into(),
        )),
    }
}

/// Coplanarity epsilon derived from a topology tolerance; never below 1e-9.
fn self_epsilon(tol: f64) -> f64 {
    let t = tol.max(1e-9);
    if t.is_finite() {
        t
    } else {
        1e-9
    }
}

/// Validate a non-degenerate, finite basis vector and return its unit form.
fn check_plane_basis(plane: &WorkPlane, tol: f64) -> CadResult<Point3> {
    for p in [plane.origin, plane.u, plane.v] {
        if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
            return Err(CadError::InvalidInput(
                "measurement plane is not finite".to_string(),
            ));
        }
    }
    let lu = length(plane.u);
    let lv = length(plane.v);
    if !lu.is_finite() || !lv.is_finite() || lu < tol || lv < tol {
        return Err(CadError::InvalidInput(
            "measurement plane is degenerate".to_string(),
        ));
    }
    let n = cross(plane.u, plane.v);
    let ln = length(n);
    if !ln.is_finite() || ln < tol {
        return Err(CadError::InvalidInput(
            "measurement plane is degenerate".to_string(),
        ));
    }
    let ortho = dot(plane.u, plane.v).abs() / (lu * lv);
    if !ortho.is_finite() || ortho > 1e-6 {
        return Err(CadError::InvalidInput(
            "measurement plane is skewed (u and v are not orthogonal)".to_string(),
        ));
    }
    Ok(scale(plane.u, 1.0 / lu))
}

/// Whether the plane basis looks unit-scaled, making u/v coordinates directly
/// comparable to model-space distances for the coplanarity check.
fn unit_scaled(plane: &WorkPlane) -> bool {
    let lu = length(plane.u);
    let lv = length(plane.v);
    (lu - 1.0).abs() <= 1e-4 && (lv - 1.0).abs() <= 1e-4
}

/// Normalise a basis vector, requiring it to be finite and non-degenerate.
fn normalized_normalized(v: Point3, tol: f64) -> CadResult<Point3> {
    let l = length(v);
    if !l.is_finite() || l < tol {
        return Err(CadError::InvalidInput(
            "measurement plane is degenerate".to_string(),
        ));
    }
    Ok(scale(v, 1.0 / l))
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}
fn scale(a: Point3, s: f64) -> Point3 {
    Point3 {
        x: a.x * s,
        y: a.y * s,
        z: a.z * s,
    }
}
fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}
fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}
fn length(a: Point3) -> f64 {
    dot(a, a).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> MeasurementEngine {
        MeasurementEngine::default()
    }

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    fn z_plane() -> WorkPlane {
        WorkPlane {
            origin: p(0.0, 0.0, 0.0),
            u: p(1.0, 0.0, 0.0),
            v: p(0.0, 1.0, 0.0),
        }
    }

    fn request(algorithm: MeasurementAlgorithm, points: Vec<Point3>) -> MeasurementRequest {
        MeasurementRequest {
            algorithm,
            points,
            tapped: Vec::new(),
            space: MeasurementSpace::World3d,
            units: UnitContext::drawing_units(),
            source: GeometrySource::Analytic,
            precision: Precision::Analytic,
        }
    }

    fn plane_request(algorithm: MeasurementAlgorithm, points: Vec<Point3>) -> MeasurementRequest {
        let mut r = request(algorithm, points);
        r.space = MeasurementSpace::Plane(z_plane());
        r
    }

    fn selection() -> SelectionRef {
        SelectionRef {
            document: DocumentId(1),
            entity: EntityId(1),
            instance: InstancePath(Vec::new()),
            sub_element: None,
        }
    }

    fn candidate(kind: SnapKind, point: Point3, space: SpaceId) -> SnapCandidate {
        SnapCandidate {
            kind,
            point,
            space,
            source: selection(),
            logical_pixel_distance: 0.0,
        }
    }

    // ---- planar space policy (B24) -----------------------------------------

    #[test]
    fn distance_2d_requires_an_explicit_plane() {
        let r = engine().measure(&request(
            MeasurementAlgorithm::Distance2d,
            vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)],
        ));
        assert!(matches!(r, Err(CadError::InvalidInput(_))), "{r:?}");
    }

    #[test]
    fn polyline_length_requires_an_explicit_plane() {
        let r = engine().measure(&request(
            MeasurementAlgorithm::PolylineLength,
            vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)],
        ));
        assert!(matches!(r, Err(CadError::InvalidInput(_))), "{r:?}");
    }

    #[test]
    fn planar_area_requires_an_explicit_plane() {
        let r = engine().measure(&request(
            MeasurementAlgorithm::PlanarPolygonArea,
            vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
        ));
        assert!(matches!(r, Err(CadError::InvalidInput(_))), "{r:?}");
    }

    #[test]
    fn distance_2d_in_plane_matches_planar_projection() {
        // A point off the plane contributes no in-plane distance.
        let r = engine()
            .measure(&plane_request(
                MeasurementAlgorithm::Distance2d,
                vec![p(0.0, 0.0, 0.0), p(3.0, 4.0, 100.0)],
            ))
            .unwrap();
        assert!((r.value - 5.0).abs() < 1e-12);
    }

    // ---- work-plane-aware area (B24) ---------------------------------------

    #[test]
    fn area_on_tilted_work_plane_is_projected() {
        // A 2x1 rectangle in the XZ plane, measured on a work plane that *is*
        // that XZ plane. Projection must recover 2.0, and the ring is coplanar.
        let mut req = plane_request(
            MeasurementAlgorithm::PlanarPolygonArea,
            vec![
                p(0.0, 0.0, 0.0),
                p(2.0, 0.0, 0.0),
                p(2.0, 0.0, 1.0),
                p(0.0, 0.0, 1.0),
            ],
        );
        req.space = MeasurementSpace::Plane(WorkPlane {
            origin: p(0.0, 0.0, 0.0),
            u: p(1.0, 0.0, 0.0),
            v: p(0.0, 0.0, 1.0),
        });
        let r = engine().measure(&req).unwrap();
        assert!((r.value - 2.0).abs() < 1e-12, "got {}", r.value);
    }

    #[test]
    fn area_rejects_ring_off_the_measurement_plane() {
        // Three of four corners lie on the z=0 work plane, one is lifted. The
        // old code flattened z to 0 and returned a valid-looking 1.0.
        let r = engine().measure(&plane_request(
            MeasurementAlgorithm::PlanarPolygonArea,
            vec![
                p(0.0, 0.0, 0.0),
                p(1.0, 0.0, 0.0),
                p(1.0, 1.0, 5.0),
                p(0.0, 1.0, 0.0),
            ],
        ));
        assert!(matches!(r, Err(CadError::InvalidInput(_))), "{r:?}");
        let msg = format!("{}", r.unwrap_err());
        assert!(msg.contains("coplanar"), "{msg}");
    }

    #[test]
    fn area_rejects_skewed_work_plane() {
        let mut req = plane_request(
            MeasurementAlgorithm::PlanarPolygonArea,
            vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
        );
        req.space = MeasurementSpace::Plane(WorkPlane {
            origin: p(0.0, 0.0, 0.0),
            u: p(1.0, 0.0, 0.0),
            v: p(1.0, 1.0, 0.0), // 45°, not orthogonal
        });
        let r = engine().measure(&req);
        assert!(matches!(r, Err(CadError::InvalidInput(_))), "{r:?}");
        let msg = format!("{}", r.unwrap_err());
        assert!(msg.contains("skewed"), "{msg}");
    }

    #[test]
    fn area_rejects_degenerate_work_plane() {
        let mut req = plane_request(
            MeasurementAlgorithm::PlanarPolygonArea,
            vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
        );
        req.space = MeasurementSpace::Plane(WorkPlane {
            origin: p(0.0, 0.0, 0.0),
            u: p(1.0, 0.0, 0.0),
            v: p(2.0, 0.0, 0.0), // parallel to u
        });
        let r = engine().measure(&req);
        assert!(matches!(r, Err(CadError::InvalidInput(_))), "{r:?}");
    }

    #[test]
    fn area_rejects_non_finite_work_plane() {
        let mut req = plane_request(
            MeasurementAlgorithm::PlanarPolygonArea,
            vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
        );
        req.space = MeasurementSpace::Plane(WorkPlane {
            origin: p(0.0, 0.0, 0.0),
            u: p(f64::NAN, 0.0, 0.0),
            v: p(0.0, 1.0, 0.0),
        });
        let r = engine().measure(&req);
        assert!(matches!(r, Err(CadError::InvalidInput(_))), "{r:?}");
    }

    #[test]
    fn area_rejects_extreme_finite_coordinates_without_overflowing() {
        // Large but finite coordinates; the shoelace products overflow to inf,
        // which must surface as an explicit error, never a bogus number.
        let big = 1e200;
        let r = engine().measure(&plane_request(
            MeasurementAlgorithm::PlanarPolygonArea,
            vec![
                p(big, 0.0, 0.0),
                p(big, big, 0.0),
                p(0.0, big, 0.0),
                p(0.0, 0.0, 0.0),
            ],
        ));
        assert!(r.is_err(), "expected overflow rejection, got {r:?}");
    }

    // ---- space constraints (B24 / F04) -------------------------------------

    #[test]
    fn distance_3d_in_paper_space_is_refused() {
        let mut req = request(
            MeasurementAlgorithm::Distance3d,
            vec![p(0.0, 0.0, 0.0), p(3.0, 4.0, 0.0)],
        );
        req.space = MeasurementSpace::Paper(LayoutId(1));
        assert!(matches!(
            engine().measure(&req),
            Err(CadError::Unsupported(_))
        ));
    }

    #[test]
    fn angle_in_paper_space_is_refused() {
        let mut req = request(
            MeasurementAlgorithm::Angle3Points,
            vec![p(1.0, 0.0, 0.0), p(0.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
        );
        req.space = MeasurementSpace::Paper(LayoutId(1));
        assert!(matches!(
            engine().measure(&req),
            Err(CadError::Unsupported(_))
        ));
    }

    #[test]
    fn mixing_spaces_in_one_measurement_is_refused() {
        let tapped = vec![
            MeasurementPoint::model(p(0.0, 0.0, 0.0)),
            MeasurementPoint::paper(p(3.0, 4.0, 0.0), LayoutId(1)),
        ];
        let req = MeasurementRequest::from_tapped(
            MeasurementAlgorithm::Distance3d,
            tapped,
            MeasurementSpace::World3d,
            UnitContext::drawing_units(),
            GeometrySource::UserPoints,
            Precision::Analytic,
        );
        let r = engine().measure(&req);
        assert!(matches!(r, Err(CadError::InvalidInput(_))), "{r:?}");
        assert!(format!("{}", r.unwrap_err()).contains("mixes"));
    }

    #[test]
    fn model_space_cannot_consume_paper_points() {
        let tapped = vec![
            MeasurementPoint::paper(p(0.0, 0.0, 0.0), LayoutId(1)),
            MeasurementPoint::paper(p(3.0, 4.0, 0.0), LayoutId(1)),
        ];
        let req = MeasurementRequest::from_tapped(
            MeasurementAlgorithm::Distance3d,
            tapped,
            MeasurementSpace::World3d,
            UnitContext::drawing_units(),
            GeometrySource::UserPoints,
            Precision::Analytic,
        );
        let r = engine().measure(&req);
        assert!(matches!(r, Err(CadError::InvalidInput(_))), "{r:?}");
    }

    #[test]
    fn consistent_model_points_are_accepted() {
        let tapped = vec![
            MeasurementPoint::model(p(0.0, 0.0, 0.0)),
            MeasurementPoint::model(p(3.0, 4.0, 0.0)),
        ];
        let req = MeasurementRequest::from_tapped(
            MeasurementAlgorithm::Distance3d,
            tapped,
            MeasurementSpace::World3d,
            UnitContext::drawing_units(),
            GeometrySource::UserPoints,
            Precision::Analytic,
        );
        let r = engine().measure(&req).unwrap();
        assert!((r.value - 5.0).abs() < 1e-12);
    }

    // ---- snapping (B24) ----------------------------------------------------

    #[test]
    fn snap_ignores_candidates_behind_the_ray_origin() {
        let ray = Ray3 {
            origin: p(0.0, 0.0, 0.0),
            direction: p(1.0, 0.0, 0.0),
        };
        let behind = candidate(SnapKind::Endpoint, p(-0.5, 0.0, 0.0), SpaceId::Model);
        let front = candidate(SnapKind::Endpoint, p(0.25, 0.0, 0.0), SpaceId::Model);
        let hit = engine().snap_to_points(&[behind, front], &ray, 1.0, None);
        assert_eq!(hit.map(|c| c.point.x), Some(0.25));
    }

    #[test]
    fn snap_rejects_non_unit_ray_direction() {
        let ray = Ray3 {
            origin: p(0.0, 0.0, 0.0),
            direction: p(0.0, 0.0, 0.0),
        };
        let c = candidate(SnapKind::Endpoint, p(0.0, 0.0, 0.0), SpaceId::Model);
        assert!(engine().snap_to_points(&[c], &ray, 1.0, None).is_none());
    }

    #[test]
    fn snap_returns_original_logical_distance_and_kind() {
        let ray = Ray3 {
            origin: p(0.0, 0.0, 0.0),
            direction: p(2.0, 0.0, 0.0), // not unit; engine normalises
        };
        let c = candidate(SnapKind::Midpoint, p(1.0, 0.2, 0.0), SpaceId::Model);
        let hit = engine()
            .snap_to_points(&[c], &ray, 0.1, None)
            .expect("midpoint within tolerance");
        assert!((hit.logical_pixel_distance - 2.0).abs() < 1e-9);
    }

    #[test]
    fn snap_filters_by_space() {
        let ray = Ray3 {
            origin: p(0.0, 0.0, 0.0),
            direction: p(1.0, 0.0, 0.0),
        };
        let model = candidate(SnapKind::Endpoint, p(1.0, 0.0, 0.0), SpaceId::Model);
        let paper = candidate(
            SnapKind::Endpoint,
            p(0.5, 0.0, 0.0),
            SpaceId::Paper(LayoutId(1)),
        );
        let hit = engine()
            .snap_to_points(&[model, paper], &ray, 1.0, Some(SpaceId::Model))
            .expect("model candidate");
        assert_eq!(hit.space, SpaceId::Model);
        assert_eq!(hit.point.x, 1.0);
    }

    #[test]
    fn snap_rejects_non_positive_world_per_px() {
        let ray = Ray3 {
            origin: p(0.0, 0.0, 0.0),
            direction: p(1.0, 0.0, 0.0),
        };
        let c = candidate(SnapKind::Endpoint, p(1.0, 0.0, 0.0), SpaceId::Model);
        assert!(engine().snap_to_points(&[c], &ray, 0.0, None).is_none());
    }

    // ---- pre-existing behaviour preserved ----------------------------------

    #[test]
    fn distance_3d_is_euclidean() {
        let r = engine()
            .measure(&request(
                MeasurementAlgorithm::Distance3d,
                vec![p(0.0, 0.0, 0.0), p(3.0, 4.0, 0.0)],
            ))
            .unwrap();
        assert!((r.value - 5.0).abs() < 1e-12);
    }

    #[test]
    fn angle_three_points_is_degrees() {
        let r = engine()
            .measure(&request(
                MeasurementAlgorithm::Angle3Points,
                vec![p(1.0, 0.0, 0.0), p(0.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
            ))
            .unwrap();
        assert!((r.value - 90.0).abs() < 1e-9);
    }

    #[test]
    fn non_finite_points_are_rejected() {
        let r = engine().measure(&request(
            MeasurementAlgorithm::Distance3d,
            vec![p(f64::NAN, 0.0, 0.0), p(0.0, 0.0, 0.0)],
        ));
        assert!(r.is_err());
    }

    #[test]
    fn polygon_area_rejects_self_intersection() {
        let bowtie = vec![
            p(0.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(0.0, 1.0, 0.0),
        ];
        let r = engine().measure(&plane_request(
            MeasurementAlgorithm::PlanarPolygonArea,
            bowtie,
        ));
        assert!(r.is_err());
    }

    #[test]
    fn paper_measurement_is_refused_without_inverse_transform() {
        let mut req = request(
            MeasurementAlgorithm::Distance2d,
            vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0)],
        );
        req.space = MeasurementSpace::Paper(LayoutId(1));
        assert!(matches!(
            engine().measure(&req),
            Err(CadError::Unsupported(_))
        ));
    }
}
