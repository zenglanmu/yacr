//! Measurement and snapping.
//!
//! Spec v2.0 §3.3: results carry their input points, unit context, algorithm and
//! precision — never just a formatted string. 2D planar and 3D spatial distance
//! are modelled separately, non-coplanar and self-intersecting area inputs are
//! rejected, and snap tolerance is defined in logical pixels.

use cad_db::{MeasurementAlgorithm, MeasurementRecord};
use cad_domain::*;
use cad_geometry::measure_polygon_area;

pub enum MeasurementSpace {
    Plane(WorkPlane),
    World3d,
    Paper(LayoutId),
    ViewportModel {
        layout: LayoutId,
        inverse: Transform3,
    },
}

pub struct MeasurementRequest {
    pub algorithm: MeasurementAlgorithm,
    pub points: Vec<Point3>,
    pub space: MeasurementSpace,
    pub units: UnitContext,
    pub source: GeometrySource,
    pub precision: Precision,
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

    /// Evaluate a measurement, rejecting non-finite, degenerate, non-coplanar
    /// and self-intersecting inputs (spec §3.3, §16.1).
    pub fn measure(&self, request: &MeasurementRequest) -> CadResult<MeasurementRecord> {
        for p in &request.points {
            if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
                return Err(CadError::InvalidInput(
                    "measurement point is not finite".to_string(),
                ));
            }
        }
        let tol = self.tolerance.computation_world.max(1e-12);
        let value = match request.algorithm {
            MeasurementAlgorithm::Distance2d => {
                let [a, b] = two(&request.points)?;
                distance_in_plane(a, b, &request.space, tol)?
            }
            MeasurementAlgorithm::Distance3d => {
                let [a, b] = two(&request.points)?;
                distance(a, b)
            }
            MeasurementAlgorithm::PolylineLength => {
                if request.points.len() < 2 {
                    return Err(CadError::InvalidInput(
                        "polyline needs at least two points".to_string(),
                    ));
                }
                let mut total = 0.0;
                for w in request.points.windows(2) {
                    total += match &request.space {
                        MeasurementSpace::World3d => distance(w[0], w[1]),
                        _ => distance_in_plane(w[0], w[1], &request.space, tol)?,
                    };
                }
                if total <= tol {
                    return Err(CadError::InvalidInput(
                        "polyline has zero length".to_string(),
                    ));
                }
                total
            }
            MeasurementAlgorithm::Angle3Points => {
                if request.points.len() != 3 {
                    return Err(CadError::InvalidInput(
                        "angle needs exactly three points".to_string(),
                    ));
                }
                let (a, v, b) = (request.points[0], request.points[1], request.points[2]);
                let v1 = sub(a, v);
                let v2 = sub(b, v);
                let l1 = length(v1);
                let l2 = length(v2);
                if l1 < tol || l2 < tol {
                    return Err(CadError::InvalidInput(
                        "angle has a degenerate arm".to_string(),
                    ));
                }
                (dot(v1, v2) / (l1 * l2))
                    .clamp(-1.0, 1.0)
                    .acos()
                    .to_degrees()
            }
            MeasurementAlgorithm::PlanarPolygonArea => {
                if request.points.len() < 3 {
                    return Err(CadError::InvalidInput(
                        "area needs at least three points".to_string(),
                    ));
                }
                let flat = project_to_plane(
                    &request.points,
                    &request.space,
                    self.tolerance.topology_world,
                )?;
                measure_polygon_area(&flat, self.tolerance.topology_world)
                    .map_err(|e| CadError::InvalidInput(format!("area rejected: {e}")))?
            }
        };
        Ok(MeasurementRecord {
            algorithm: request.algorithm.clone(),
            inputs: request.points.clone(),
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

    /// Snap against a supplied local candidate list.
    ///
    /// Geometry must come from the caller (the database + spatial index), so the
    /// contract-level `snap` cannot fabricate points; this is the real entry.
    pub fn snap_to_points(
        &self,
        candidates: &[SnapCandidate],
        ray: &Ray3,
        world_per_px: f64,
    ) -> Option<SnapCandidate> {
        let tolerance = self.tolerance.interaction_logical_pixels * world_per_px;
        let mut best: Option<(f64, SnapCandidate)> = None;
        for c in candidates {
            let d = point_ray_distance(ray, c.point);
            let ok = best.as_ref().map(|(bd, _)| d < *bd).unwrap_or(true);
            if d <= tolerance && ok {
                best = Some((
                    d,
                    SnapCandidate {
                        kind: match c.kind {
                            SnapKind::Endpoint => SnapKind::Endpoint,
                            SnapKind::Midpoint => SnapKind::Midpoint,
                            SnapKind::Center => SnapKind::Center,
                            SnapKind::LocalIntersection => SnapKind::LocalIntersection,
                        },
                        point: c.point,
                        source: c.source.clone(),
                        logical_pixel_distance: d / world_per_px.max(1e-12),
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

/// Perpendicular distance from a point to a ray's infinite line.
fn point_ray_distance(ray: &Ray3, p: Point3) -> f64 {
    let d = ray.direction;
    let dl = length(d);
    if dl < 1e-24 {
        return distance(ray.origin, p);
    }
    let dir = scale(d, 1.0 / dl);
    let v = sub(p, ray.origin);
    let proj = dot(v, dir);
    let closest = scale(dir, proj);
    length(sub(v, closest))
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

/// Distance projected onto the measurement plane.
fn distance_in_plane(a: Point3, b: Point3, space: &MeasurementSpace, tol: f64) -> CadResult<f64> {
    let d = sub(b, a);
    match space {
        MeasurementSpace::Plane(plane) => {
            let nx = normalize(cross(plane.u, plane.v));
            if length(nx) < tol {
                return Err(CadError::InvalidInput(
                    "measurement plane is degenerate".to_string(),
                ));
            }
            // Remove the plane-normal component so this is a true in-plane distance.
            let n = nx;
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

/// Project points into the measurement plane's 2D coordinates.
fn project_to_plane(
    points: &[Point3],
    space: &MeasurementSpace,
    tol: f64,
) -> CadResult<Vec<Point3>> {
    match space {
        MeasurementSpace::Plane(plane) => {
            let u = normalize(plane.u);
            let v = normalize(plane.v);
            if length(cross(u, v)) < tol {
                return Err(CadError::InvalidInput(
                    "measurement plane is degenerate".to_string(),
                ));
            }
            Ok(points
                .iter()
                .map(|p| {
                    let d = sub(*p, plane.origin);
                    Point3 {
                        x: dot(d, u),
                        y: dot(d, v),
                        z: 0.0,
                    }
                })
                .collect())
        }
        MeasurementSpace::World3d => Ok(points.to_vec()),
        _ => Err(CadError::Unsupported(
            "area requires a defined measurement plane".into(),
        )),
    }
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
fn normalize(a: Point3) -> Point3 {
    let l = length(a);
    if l < 1e-24 {
        a
    } else {
        scale(a, 1.0 / l)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> MeasurementEngine {
        MeasurementEngine::default()
    }

    fn request(algorithm: MeasurementAlgorithm, points: Vec<Point3>) -> MeasurementRequest {
        MeasurementRequest {
            algorithm,
            points,
            space: MeasurementSpace::World3d,
            units: UnitContext::drawing_units(),
            source: GeometrySource::Analytic,
            precision: Precision::Analytic,
        }
    }

    #[test]
    fn distance_3d_is_euclidean() {
        let r = engine()
            .measure(&request(
                MeasurementAlgorithm::Distance3d,
                vec![
                    Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    Point3 {
                        x: 3.0,
                        y: 4.0,
                        z: 0.0,
                    },
                ],
            ))
            .unwrap();
        assert!((r.value - 5.0).abs() < 1e-12);
    }

    #[test]
    fn angle_three_points_is_degrees() {
        let r = engine()
            .measure(&request(
                MeasurementAlgorithm::Angle3Points,
                vec![
                    Point3 {
                        x: 1.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    Point3 {
                        x: 0.0,
                        y: 1.0,
                        z: 0.0,
                    },
                ],
            ))
            .unwrap();
        assert!((r.value - 90.0).abs() < 1e-9);
    }

    #[test]
    fn non_finite_points_are_rejected() {
        let r = engine().measure(&request(
            MeasurementAlgorithm::Distance3d,
            vec![
                Point3 {
                    x: f64::NAN,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
            ],
        ));
        assert!(r.is_err());
    }

    #[test]
    fn polygon_area_rejects_self_intersection() {
        let bowtie = vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 1.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ];
        let r = engine().measure(&request(MeasurementAlgorithm::PlanarPolygonArea, bowtie));
        assert!(r.is_err());
    }

    #[test]
    fn paper_measurement_is_refused_without_inverse_transform() {
        let mut req = request(
            MeasurementAlgorithm::Distance2d,
            vec![
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            ],
        );
        req.space = MeasurementSpace::Paper(LayoutId(1));
        assert!(matches!(
            engine().measure(&req),
            Err(CadError::Unsupported(_))
        ));
    }
}
