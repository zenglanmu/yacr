//! The measurement engine: space policy, algorithm dispatch and snapping.

use cad_db::{MeasurementAlgorithm, MeasurementRecord};
use cad_domain::*;
use cad_geometry::measure_polygon_area;

use crate::algorithms::{
    angle_at_vertex, distance_in_plane, distance_with_space, point_ray_distance, project_to_plane,
    two, unit_direction,
};
use crate::snap;
use crate::snap::{SnapCandidate, SnapProvenance, SnapTarget, SnappedMeasurement};
use crate::space::{
    effective_plane, measure_space_points, require_planar_space, require_plane, valid_inverse,
    MeasurementRequest, MeasurementSpace,
};

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
    /// §16.1; audit B24, F04/F06).
    ///
    /// Points are brought into the *measurement space* first: paper picks inside
    /// a viewport are mapped to model coordinates through the verified inverse
    /// transform, so a viewport measurement reports a model distance rather than
    /// the paper-pixel distance. Paper space measures the sheet directly.
    pub fn measure(&self, request: &MeasurementRequest) -> CadResult<MeasurementRecord> {
        self.check_space_policy(request)?;
        let requested = request.coordinates();
        for p in &requested {
            if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
                return Err(CadError::InvalidInput(
                    "measurement point is not finite".to_string(),
                ));
            }
        }
        let points = measure_space_points(&request.space, &requested);
        for p in &points {
            if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
                return Err(CadError::InvalidInput(
                    "measurement point is not finite after the space transform".to_string(),
                ));
            }
        }
        let plane = effective_plane(&request.space);
        let tol = self.tolerance.computation_world.max(1e-12);
        let value = match request.algorithm {
            MeasurementAlgorithm::Distance2d => {
                require_planar_space(&request.space)?;
                let plane = require_plane(plane)?;
                let [a, b] = two(&points)?;
                distance_in_plane(a, b, &plane, tol)?
            }
            MeasurementAlgorithm::Distance3d => {
                let [a, b] = two(&points)?;
                distance_with_space(a, b, &request.space)?
            }
            MeasurementAlgorithm::PolylineLength => {
                require_planar_space(&request.space)?;
                let plane = require_plane(plane)?;
                if points.len() < 2 {
                    return Err(CadError::InvalidInput(
                        "polyline needs at least two points".to_string(),
                    ));
                }
                let mut total = 0.0;
                for w in points.windows(2) {
                    total += distance_in_plane(w[0], w[1], &plane, tol)?;
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
                angle_at_vertex(a, v, b, &request.space, plane.as_ref(), tol)?
            }
            MeasurementAlgorithm::PlanarPolygonArea => {
                require_planar_space(&request.space)?;
                let plane = require_plane(plane)?;
                if points.len() < 3 {
                    return Err(CadError::InvalidInput(
                        "area needs at least three points".to_string(),
                    ));
                }
                let flat = project_to_plane(&points, &plane, self.tolerance.topology_world)?;
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
            // Inputs are recorded in the measurement space (model for a viewport
            // measurement, paper for a paper measurement) so the record is
            // self-consistent with `plane` and `value`.
            inputs: points,
            plane,
            value,
            units: request.units.clone(),
            source: request.source.clone(),
            precision: request.precision.clone(),
        })
    }

    /// Space policy.
    ///
    /// Paper-space measurement is defined directly on the sheet. A
    /// `ViewportModel` measurement needs a **valid inverse transform** (finite
    /// and invertible); without one model measurement is explicitly disabled
    /// rather than guessed from paper pixels. Mixed-space inputs are refused.
    fn check_space_policy(&self, request: &MeasurementRequest) -> CadResult<()> {
        if let MeasurementSpace::ViewportModel { layout, inverse } = &request.space {
            if !valid_inverse(inverse) {
                return Err(CadError::Unsupported(format!(
                    "viewport model measurement for layout {} has no valid inverse viewport transform",
                    layout.0
                )));
            }
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
                        kind: c.kind,
                        point: c.point,
                        space: c.space.clone(),
                        source: c.source.clone(),
                        secondary: c.secondary.clone(),
                        precision: c.precision.clone(),
                        logical_pixel_distance: d / world_per_px,
                    },
                ));
            }
        }
        best.map(|(_, c)| c)
    }

    /// Compute snap candidates from a caller-supplied **local** target set.
    ///
    /// This is the real snapping entry: the database + spatial index decide
    /// which entities are near the cursor, and this method resolves their snap
    /// points (endpoint, midpoint, center, quadrant, perpendicular and local
    /// intersection) with a logical-pixel tolerance converted to world units.
    /// Input is rejected for a non-finite/degenerate ray or a non-positive
    /// tolerance; geometry behind the pick-ray origin is never returned.
    pub fn snap_targets(
        &self,
        targets: &[SnapTarget],
        ray: &Ray3,
        plane: Option<&WorkPlane>,
        world_per_px: f64,
        space_filter: Option<SpaceId>,
    ) -> CadResult<Vec<SnapCandidate>> {
        snap::collect_candidates(
            targets,
            ray,
            plane,
            world_per_px,
            space_filter.as_ref(),
            &self.tolerance,
        )
    }

    /// Best local snap candidate by logical pixel distance.
    pub fn snap_best(
        &self,
        targets: &[SnapTarget],
        ray: &Ray3,
        plane: Option<&WorkPlane>,
        world_per_px: f64,
        space_filter: Option<SpaceId>,
    ) -> CadResult<Option<SnapCandidate>> {
        snap::best_candidate(
            targets,
            ray,
            plane,
            world_per_px,
            space_filter.as_ref(),
            &self.tolerance,
        )
    }

    /// Measure and attach the snap provenance of the input points (spec §3.3).
    ///
    /// The provenance must correspond one-to-one with the recorded inputs;
    /// a mismatched count is an explicit error rather than a silently dropped
    /// source.
    pub fn measure_snapped(
        &self,
        request: &MeasurementRequest,
        snaps: Vec<SnapProvenance>,
    ) -> CadResult<SnappedMeasurement> {
        let record = self.measure(request)?;
        if snaps.len() != record.inputs.len() {
            return Err(CadError::InvalidInput(format!(
                "snap provenance has {} entries for {} measurement inputs",
                snaps.len(),
                record.inputs.len()
            )));
        }
        Ok(SnappedMeasurement { record, snaps })
    }

    /// Contract entry: snapping needs candidate geometry from the database.
    pub fn snap(
        &self,
        _viewport: ViewportId,
        _candidates: &[SelectionRef],
        _ray: Ray3,
    ) -> CadResult<Option<SnapCandidate>> {
        Err(CadError::Unsupported(
            "snap needs candidate geometry; use snap_targets with geometry resolved from the database".into(),
        ))
    }
}
