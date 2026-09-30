//! Disposable CPU display descriptions; no surfaces or GPU commands.
use cad_db::DbEntity;
use cad_domain::*;
use cad_resources::ResourceKey;
use std::sync::Arc;
pub enum DisplayPrimitive {
    Lines(Arc<[Point3]>), Mesh(Arc<Mesh>),
    Text { text: String, origin: Point3, font: ResourceKey, height: f64 },
    Image { resource: ResourceKey, transform: Transform3 },
    Instance { block: BlockId, transform: Transform3 },
}
pub struct DisplayFragment { pub source: SelectionRef, pub geometry_source: GeometrySource, pub primitive: DisplayPrimitive }
pub struct DisplayRepresentation { pub fragments: Vec<DisplayFragment>, pub completeness: Completeness, pub diagnostics: Vec<Diagnostic> }
pub struct RepresentationContext { pub document: DocumentId, pub tolerance: TolerancePolicy, pub stamp: TaskStamp }
pub trait RepresentationProvider {
    fn registration(&self) -> Registration;
    fn build(&self, entity: &DbEntity, context: &RepresentationContext) -> CadResult<DisplayRepresentation>;
}
#[derive(Default)]
pub struct ProviderRegistry { providers: Vec<Box<dyn RepresentationProvider>> }
impl ProviderRegistry {
    /// Must reject ambiguous same-priority matches rather than choosing randomly.
    pub fn register(&mut self, _provider: Box<dyn RepresentationProvider>) -> CadResult<()> { pending("representation.register") }
    pub fn build(&self, _entity: &DbEntity, _context: &RepresentationContext) -> CadResult<DisplayRepresentation> {
        let _ = &self.providers; pending("representation.dispatch")
    }
}
