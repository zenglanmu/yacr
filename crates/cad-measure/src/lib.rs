use cad_db::{MeasurementAlgorithm, MeasurementRecord};
use cad_domain::*;
pub enum MeasurementSpace { Plane(WorkPlane), World3d, Paper(LayoutId), ViewportModel { layout: LayoutId, inverse: Transform3 } }
pub struct MeasurementRequest {
    pub algorithm: MeasurementAlgorithm, pub points: Vec<Point3>, pub space: MeasurementSpace,
    pub units: UnitContext, pub source: GeometrySource, pub precision: Precision,
}
pub enum SnapKind { Endpoint, Midpoint, Center, LocalIntersection }
pub struct SnapCandidate { pub kind: SnapKind, pub point: Point3, pub source: SelectionRef, pub logical_pixel_distance: f64 }
pub struct MeasurementEngine { pub tolerance: TolerancePolicy }
impl MeasurementEngine {
    /// Reject nonfinite, degenerate, noncoplanar and self-intersecting inputs.
    pub fn measure(&self, _request: &MeasurementRequest) -> CadResult<MeasurementRecord> { pending("measure.evaluate") }
    pub fn snap(&self, _viewport: ViewportId, _candidates: &[SelectionRef], _ray: Ray3) -> CadResult<Option<SnapCandidate>> { pending("measure.snap_local") }
}
