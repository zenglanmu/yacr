//! f64 geometry; display tolerances never control measurement semantics.
use cad_domain::*;
pub trait GeometryEngine {
    fn transform(&self, geometry: &SemanticGeometry, transform: &Transform3) -> CadResult<SemanticGeometry>;
    fn bounds(&self, geometry: &SemanticGeometry) -> CadResult<Bounds3>;
    fn tessellate_curve(&self, geometry: &SemanticGeometry, tolerance: &TolerancePolicy) -> CadResult<Vec<Point3>>;
    fn intersect_local(&self, a: &SemanticGeometry, b: &SemanticGeometry, tolerance: &TolerancePolicy) -> CadResult<Vec<Point3>>;
    fn ray_plane(&self, ray: &Ray3, plane: &WorkPlane, tolerance: &TolerancePolicy) -> CadResult<Option<Point3>>;
}
pub struct PendingGeometryEngine;
impl GeometryEngine for PendingGeometryEngine {
    fn transform(&self, _: &SemanticGeometry, _: &Transform3) -> CadResult<SemanticGeometry> { pending("geometry.transform") }
    fn bounds(&self, _: &SemanticGeometry) -> CadResult<Bounds3> { pending("geometry.bounds") }
    fn tessellate_curve(&self, _: &SemanticGeometry, _: &TolerancePolicy) -> CadResult<Vec<Point3>> { pending("geometry.tessellate_curve") }
    fn intersect_local(&self, _: &SemanticGeometry, _: &SemanticGeometry, _: &TolerancePolicy) -> CadResult<Vec<Point3>> { pending("geometry.intersect_local") }
    fn ray_plane(&self, _: &Ray3, _: &WorkPlane, _: &TolerancePolicy) -> CadResult<Option<Point3>> { pending("geometry.ray_plane") }
}
