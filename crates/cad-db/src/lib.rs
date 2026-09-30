//! Authoritative storage; import builder and transactions are the only write paths.
use cad_domain::*;
use std::collections::BTreeMap;
#[derive(Debug, Clone)]
pub struct DbObject { pub id: ObjectId, pub type_key: String, pub revision: Revision, pub source_handle: Option<String> }
#[derive(Debug, Clone)]
pub struct DbEntity {
    pub object: DbObject, pub id: EntityId, pub layer: LayerId, pub space: SpaceId,
    pub geometry: SemanticGeometry, pub draw_order: i64,
}
#[derive(Debug, Clone)]
pub struct Layer { pub id: LayerId, pub name: String, pub visible: bool }
#[derive(Debug, Clone)]
pub struct BlockDefinition { pub id: BlockId, pub entities: Vec<EntityId> }
#[derive(Debug, Clone)]
pub struct Layout { pub id: LayoutId, pub name: String, pub viewports: Vec<PaperViewport> }
#[derive(Debug, Clone)]
pub struct PaperViewport { pub clip: Vec<Point3>, pub model_to_paper: Transform3, pub completeness: Completeness }
#[derive(Debug, Clone)]
pub struct Style { pub id: StyleId, pub name: String, pub resource_keys: Vec<String> }
#[derive(Debug, Clone)]
pub enum AnchorStatus { Valid, Stale, Unresolved }
#[derive(Debug, Clone)]
pub struct EntityAnchor {
    pub source_handle: String, pub instance: InstancePath, pub sub_element: Option<SubElementId>,
    pub fallback: Point3, pub status: AnchorStatus,
}
#[derive(Debug, Clone)]
pub enum AnnotationGeometry {
    Text(Point3), Leader(Vec<Point3>), Rectangle([Point3; 2]),
    Ellipse { center: Point3, axis_u: Point3, axis_v: Point3 },
    Freehand(Vec<Point3>), Cloud(Vec<Point3>), Measurement(MeasurementRecord),
}
#[derive(Debug, Clone)]
pub enum MeasurementAlgorithm { Distance2d, Distance3d, PolylineLength, Angle3Points, PlanarPolygonArea }
#[derive(Debug, Clone)]
pub struct MeasurementRecord {
    pub algorithm: MeasurementAlgorithm, pub inputs: Vec<Point3>, pub plane: Option<WorkPlane>,
    pub value: f64, pub units: UnitContext, pub source: GeometrySource, pub precision: Precision,
}
#[derive(Debug, Clone)]
pub struct AnnotationStyle { pub rgba: [u8; 4], pub logical_width: f64, pub text_height: f64 }
#[derive(Debug, Clone)]
pub struct Annotation {
    /// UUID supplied by the host; never a GPU index.
    pub id: AnnotationId, pub space: SpaceId, pub geometry: AnnotationGeometry,
    pub text: String, pub style: AnnotationStyle, pub created_unix_ms: i64,
    pub modified_unix_ms: i64, pub anchor: Option<EntityAnchor>, pub precision: Precision,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeMask(pub u8);
impl ChangeMask {
    pub const GEOMETRY: Self = Self(1);
    pub const STYLE: Self = Self(2);
    pub const TRANSFORM: Self = Self(4);
    pub const REFERENCES: Self = Self(8);
    pub const METADATA: Self = Self(16);
}
#[derive(Debug, Clone)]
pub enum ObjectChange { Insert(ObjectId), Update(ObjectId, ChangeMask), Delete(ObjectId) }
#[derive(Debug, Clone)]
pub struct ChangeSet {
    pub database: DatabaseId, pub before: Revision, pub after: Revision,
    pub transaction: TransactionId, pub reason: String, pub changes: Vec<ObjectChange>,
}
impl ChangeSet {
    pub fn follows(&self, database: DatabaseId, revision: Revision) -> bool {
        self.database == database && self.before == revision && self.after.0.checked_sub(1) == Some(revision.0)
    }
}
#[derive(Debug, Clone)]
pub struct DrawingDatabase {
    id: DatabaseId, revision: Revision, entities: BTreeMap<EntityId, DbEntity>,
    layers: BTreeMap<LayerId, Layer>, layouts: BTreeMap<LayoutId, Layout>,
}
impl DrawingDatabase {
    pub fn id(&self) -> DatabaseId { self.id }
    pub fn revision(&self) -> Revision { self.revision }
    pub fn entity(&self, id: EntityId) -> Option<&DbEntity> { self.entities.get(&id) }
    pub fn entities(&self) -> impl Iterator<Item = &DbEntity> { self.entities.values() }
    pub fn layers(&self) -> impl Iterator<Item = &Layer> { self.layers.values() }
    pub fn layouts(&self) -> impl Iterator<Item = &Layout> { self.layouts.values() }
}
pub struct DrawingDatabaseBuilder { database: DrawingDatabase }
impl DrawingDatabaseBuilder {
    pub fn new(id: DatabaseId) -> Self {
        Self { database: DrawingDatabase { id, revision: Revision(0), entities: BTreeMap::new(), layers: BTreeMap::new(), layouts: BTreeMap::new() } }
    }
    pub fn insert_entity(&mut self, _entity: DbEntity) -> CadResult<()> { pending("db.builder.insert_entity") }
    pub fn insert_layer(&mut self, _layer: Layer) -> CadResult<()> { pending("db.builder.insert_layer") }
    pub fn insert_layout(&mut self, _layout: Layout) -> CadResult<()> { pending("db.builder.insert_layout") }
    pub fn finish(self) -> CadResult<DrawingDatabase> { let _ = self.database; pending("db.builder.validate_and_finish") }
}
#[derive(Debug)]
pub struct AnnotationDatabase {
    id: DatabaseId, revision: Revision, annotations: BTreeMap<AnnotationId, Annotation>, saved_revision: Revision,
}
impl AnnotationDatabase {
    pub fn new(id: DatabaseId) -> Self { Self { id, revision: Revision(0), annotations: BTreeMap::new(), saved_revision: Revision(0) } }
    pub fn id(&self) -> DatabaseId { self.id }
    pub fn revision(&self) -> Revision { self.revision }
    pub fn is_dirty(&self) -> bool { self.revision != self.saved_revision }
    pub fn annotations(&self) -> impl Iterator<Item = &Annotation> { self.annotations.values() }
    pub fn get(&self, id: AnnotationId) -> Option<&Annotation> { self.annotations.get(&id) }
    /// Called only after durable export succeeds, with the exported snapshot revision.
    pub fn mark_exported(&mut self, _revision: Revision) -> CadResult<()> { pending("db.mark_exported") }
    pub fn begin(&mut self, _reason: &str, _id: TransactionId) -> CadResult<AnnotationTransaction<'_>> { pending("db.begin_transaction") }
}
pub struct AnnotationTransaction<'a> { database: &'a mut AnnotationDatabase }
impl AnnotationTransaction<'_> {
    pub fn insert_annotation(&mut self, _annotation: Annotation) -> CadResult<AnnotationId> { pending("db.tx.insert") }
    pub fn update_annotation(&mut self, _annotation: Annotation) -> CadResult<()> { pending("db.tx.update") }
    pub fn delete_annotation(&mut self, _id: AnnotationId) -> CadResult<()> { pending("db.tx.delete") }
    pub fn commit(self) -> CadResult<ChangeSet> { let _ = self.database; pending("db.tx.atomic_commit") }
    /// Uncommitted staged changes must be discarded on drop.
    pub fn rollback(self) { }
}
