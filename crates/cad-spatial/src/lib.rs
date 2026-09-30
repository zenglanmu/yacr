//! CPU candidate lookup remains available to the WebGL2 path.
use cad_domain::*;
pub struct SpatialEntry { pub source: SelectionRef, pub bounds: Bounds3 }
pub struct PickHit { pub source: SelectionRef, pub point: Point3, pub precision: Precision, pub geometry_source: GeometrySource }
pub trait SpatialIndex {
    fn rebuild(&mut self, entries: &[SpatialEntry]) -> CadResult<()>;
    fn update(&mut self, inserted: &[SpatialEntry], removed: &[SelectionRef]) -> CadResult<()>;
    fn query_bounds(&self, bounds: &Bounds3) -> CadResult<Vec<SelectionRef>>;
    fn ray_candidates(&self, ray: &Ray3) -> CadResult<Vec<SelectionRef>>;
}
pub struct PendingSpatialIndex;
impl SpatialIndex for PendingSpatialIndex {
    fn rebuild(&mut self, _: &[SpatialEntry]) -> CadResult<()> { pending("spatial.rebuild") }
    fn update(&mut self, _: &[SpatialEntry], _: &[SelectionRef]) -> CadResult<()> { pending("spatial.update") }
    fn query_bounds(&self, _: &Bounds3) -> CadResult<Vec<SelectionRef>> { pending("spatial.query_bounds") }
    fn ray_candidates(&self, _: &Ray3) -> CadResult<Vec<SelectionRef>> { pending("spatial.ray_candidates") }
}
