//! CPU scene cache, separate from authoritative data and device resources.
use cad_db::ChangeSet;
use cad_domain::*;
use cad_representation::DisplayRepresentation;
pub struct CacheKey {
    pub identity: DocumentIdentity, pub entity: EntityId, pub revision: Revision,
    pub style_version: u64, pub font_version: u64, pub lod: u32, pub space: SpaceId,
}
pub struct RenderBatch { pub local_origin: Point3, pub vertices: Vec<[f32; 3]>, pub sources: Vec<SelectionRef>, pub draw_order: i64 }
pub struct SceneDelta { pub stamp: TaskStamp, pub added: Vec<RenderBatch>, pub removed_chunks: Vec<u64> }
pub struct SceneBudget { pub cpu_bytes: usize, pub queued_tasks: usize, pub upload_bytes_per_frame: usize }
pub struct SceneCache { pub budget: SceneBudget }
impl SceneCache {
    pub fn apply_changes(&mut self, _changes: &ChangeSet) -> CadResult<()> { pending("scene.apply_changes") }
    pub fn build(&mut self, _representation: &DisplayRepresentation, _stamp: TaskStamp) -> CadResult<SceneDelta> { pending("scene.build") }
    pub fn publish(&mut self, delta: SceneDelta, current: &TaskStamp) -> CadResult<()> { delta.stamp.validate(current)?; pending("scene.publish") }
    pub fn evict(&mut self, _required_bytes: usize) -> CadResult<()> { pending("scene.evict") }
}
