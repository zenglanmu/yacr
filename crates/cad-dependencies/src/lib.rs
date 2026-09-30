use cad_db::ChangeSet;
use cad_domain::*;
#[derive(Debug, Clone)]
pub struct Invalidation {
    pub representations: Vec<ObjectId>, pub bounds: Vec<ObjectId>,
    pub spatial: Vec<ObjectId>, pub query: Vec<ObjectId>, pub rebuild_snapshot: bool,
}
pub struct DependencyIndex { pub max_depth: usize, pub max_affected: usize }
impl DependencyIndex {
    pub fn register(&mut self, _consumer: ObjectId, _dependency: ObjectId) -> CadResult<()> { pending("dependencies.register") }
    pub fn invalidate(&mut self, _changes: &ChangeSet) -> CadResult<Invalidation> { pending("dependencies.invalidate") }
}
