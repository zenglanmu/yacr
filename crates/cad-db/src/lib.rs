//! Authoritative storage; import builder and transactions are the only write paths.
//!
//! Spec v2.0 §4.3, §4.6: the drawing database is read-only after a controlled
//! import; the annotation database is only mutated through transactions. A
//! transaction is either fully committed or leaves no partial state, and every
//! commit raises the database revision and publishes an ordered [`ChangeSet`].

use cad_domain::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
pub struct DbObject {
    pub id: ObjectId,
    pub type_key: String,
    pub revision: Revision,
    pub source_handle: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DbEntity {
    pub object: DbObject,
    pub id: EntityId,
    pub layer: LayerId,
    pub space: SpaceId,
    pub geometry: SemanticGeometry,
    pub draw_order: i64,
}

impl DbEntity {
    pub fn entity_style_key(&self) -> &str {
        &self.object.type_key
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockDefinition {
    pub id: BlockId,
    pub entities: Vec<EntityId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub id: LayoutId,
    pub name: String,
    pub viewports: Vec<PaperViewport>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaperViewport {
    pub clip: Vec<Point3>,
    pub model_to_paper: Transform3,
    pub completeness: Completeness,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub id: StyleId,
    pub name: String,
    pub resource_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorStatus {
    Valid,
    Stale,
    Unresolved,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntityAnchor {
    pub source_handle: String,
    pub instance: InstancePath,
    pub sub_element: Option<SubElementId>,
    pub fallback: Point3,
    pub status: AnchorStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnnotationGeometry {
    Text(Point3),
    Leader(Vec<Point3>),
    Rectangle([Point3; 2]),
    Ellipse {
        center: Point3,
        axis_u: Point3,
        axis_v: Point3,
    },
    Freehand(Vec<Point3>),
    Cloud(Vec<Point3>),
    Measurement(MeasurementRecord),
}

impl AnnotationGeometry {
    /// All world points, used for bounds and cache invalidation.
    pub fn points(&self) -> Vec<Point3> {
        match self {
            AnnotationGeometry::Text(p) => vec![*p],
            AnnotationGeometry::Leader(v)
            | AnnotationGeometry::Freehand(v)
            | AnnotationGeometry::Cloud(v) => v.clone(),
            AnnotationGeometry::Rectangle(pair) => pair.to_vec(),
            AnnotationGeometry::Ellipse {
                center,
                axis_u,
                axis_v,
            } => vec![
                add(*center, *axis_u),
                add(*center, *axis_v),
                sub(*center, *axis_u),
                sub(*center, *axis_v),
            ],
            AnnotationGeometry::Measurement(m) => m.inputs.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MeasurementAlgorithm {
    Distance2d,
    Distance3d,
    PolylineLength,
    Angle3Points,
    PlanarPolygonArea,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeasurementRecord {
    pub algorithm: MeasurementAlgorithm,
    pub inputs: Vec<Point3>,
    pub plane: Option<WorkPlane>,
    pub value: f64,
    pub units: UnitContext,
    pub source: GeometrySource,
    pub precision: Precision,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationStyle {
    pub rgba: [u8; 4],
    pub logical_width: f64,
    pub text_height: f64,
}

impl Default for AnnotationStyle {
    fn default() -> Self {
        AnnotationStyle {
            rgba: [0xE5, 0x39, 0x35, 0xFF],
            logical_width: 2.0,
            text_height: 2.5,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    /// UUID supplied by the host; never a GPU index.
    pub id: AnnotationId,
    pub space: SpaceId,
    pub geometry: AnnotationGeometry,
    pub text: String,
    pub style: AnnotationStyle,
    pub created_unix_ms: i64,
    pub modified_unix_ms: i64,
    pub anchor: Option<EntityAnchor>,
    pub precision: Precision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeMask(pub u8);

impl ChangeMask {
    pub const GEOMETRY: Self = Self(1);
    pub const STYLE: Self = Self(2);
    pub const TRANSFORM: Self = Self(4);
    pub const REFERENCES: Self = Self(8);
    pub const METADATA: Self = Self(16);

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether this change requires display representations to be rebuilt.
    pub fn invalidates_representation(self) -> bool {
        self.contains(ChangeMask::GEOMETRY)
            || self.contains(ChangeMask::STYLE)
            || self.contains(ChangeMask::TRANSFORM)
            || self.contains(ChangeMask::REFERENCES)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObjectChange {
    Insert(ObjectId),
    Update(ObjectId, ChangeMask),
    Delete(ObjectId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChangeSet {
    pub database: DatabaseId,
    pub before: Revision,
    pub after: Revision,
    pub transaction: TransactionId,
    pub reason: String,
    pub changes: Vec<ObjectChange>,
}

impl ChangeSet {
    /// True when this change set is exactly the successor of `revision`.
    pub fn follows(&self, database: DatabaseId, revision: Revision) -> bool {
        self.database == database
            && self.before == revision
            && self.after.0.checked_sub(1) == Some(revision.0)
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// The authoritative, read-only-after-import drawing database.
#[derive(Debug, Clone)]
pub struct DrawingDatabase {
    id: DatabaseId,
    revision: Revision,
    entities: BTreeMap<EntityId, DbEntity>,
    layers: BTreeMap<LayerId, Layer>,
    blocks: BTreeMap<BlockId, BlockDefinition>,
    layouts: BTreeMap<LayoutId, Layout>,
    styles: BTreeMap<StyleId, Style>,
}

impl DrawingDatabase {
    pub fn id(&self) -> DatabaseId {
        self.id
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Content identity used to decide whether a render/scene cache is stale.
    ///
    /// A bare `DatabaseId` is not enough: hosts may reuse the same id for a
    /// different drawing, which would leave old GPU batches on screen (audit
    /// B04). The identity therefore mixes the id, revision, structural counts
    /// and model-space bounds.
    pub fn scene_identity(&self) -> SceneIdentity {
        let (min, max) = self
            .bounds()
            .unwrap_or((Point3::default(), Point3::default()));
        SceneIdentity {
            database: self.id,
            revision: self.revision,
            entities: self.entities.len() as u64,
            layers: self.layers.len() as u64,
            bounds: [min, max],
        }
    }

    pub fn entity(&self, id: EntityId) -> Option<&DbEntity> {
        self.entities.get(&id)
    }

    pub fn entities(&self) -> impl Iterator<Item = &DbEntity> {
        self.entities.values()
    }

    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.get(&id)
    }

    pub fn layers(&self) -> impl Iterator<Item = &Layer> {
        self.layers.values()
    }

    pub fn block(&self, id: BlockId) -> Option<&BlockDefinition> {
        self.blocks.get(&id)
    }

    pub fn blocks(&self) -> impl Iterator<Item = &BlockDefinition> {
        self.blocks.values()
    }

    pub fn layout(&self, id: LayoutId) -> Option<&Layout> {
        self.layouts.get(&id)
    }

    pub fn layouts(&self) -> impl Iterator<Item = &Layout> {
        self.layouts.values()
    }

    pub fn style(&self, id: StyleId) -> Option<&Style> {
        self.styles.get(&id)
    }

    /// Entities in model space, in draw order.
    pub fn model_space(&self) -> Vec<&DbEntity> {
        let mut v: Vec<&DbEntity> = self
            .entities
            .values()
            .filter(|e| matches!(e.space, SpaceId::Model))
            .collect();
        v.sort_by_key(|e| e.draw_order);
        v
    }

    /// Entities of a block definition, in draw order.
    pub fn block_entities(&self, id: BlockId) -> Vec<&DbEntity> {
        let Some(block) = self.blocks.get(&id) else {
            return Vec::new();
        };
        let mut v: Vec<&DbEntity> = block
            .entities
            .iter()
            .filter_map(|e| self.entities.get(e))
            .collect();
        v.sort_by_key(|e| e.draw_order);
        v
    }

    /// Entities belonging to a layout, in draw order.
    pub fn layout_entities(&self, id: LayoutId) -> Vec<&DbEntity> {
        let mut v: Vec<&DbEntity> = self
            .entities
            .values()
            .filter(|e| matches!(&e.space, SpaceId::Paper(l) if *l == id))
            .collect();
        v.sort_by_key(|e| e.draw_order);
        v
    }

    /// World bounds of model-space geometry, expanding INSERT instances.
    pub fn bounds(&self) -> Option<(Point3, Point3)> {
        let mut acc = BoundsAccumulator::new();
        let mut stack: Vec<BlockId> = Vec::new();
        for e in self.model_space() {
            self.accumulate_bounds(&mut acc, e, &Transform3::identity(), 0, &mut stack);
        }
        acc.finish()
    }

    /// Recursively fold one model-space entity (and any inserts) into `acc`.
    ///
    /// Nesting is bounded and cycles are cut so a malformed block graph cannot
    /// loop forever; a truncated/cyclic branch simply contributes no bounds.
    fn accumulate_bounds(
        &self,
        acc: &mut BoundsAccumulator,
        entity: &DbEntity,
        transform: &Transform3,
        depth: usize,
        stack: &mut Vec<BlockId>,
    ) {
        if let SemanticGeometry::Insert {
            block,
            transform: insert,
        } = &entity.geometry
        {
            if depth >= MAX_INSTANCE_DEPTH || stack.contains(block) {
                return;
            }
            let composed = transform.matrix_mul(insert);
            stack.push(*block);
            for child in self.block_entities(*block) {
                self.accumulate_bounds(acc, child, &composed, depth + 1, stack);
            }
            stack.pop();
        } else {
            acc.add_geometry_transformed(&entity.geometry, transform);
        }
    }
}

/// Accumulates an axis-aligned box over geometry.
pub struct BoundsAccumulator {
    min: Point3,
    max: Point3,
    any: bool,
}

impl Default for BoundsAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl BoundsAccumulator {
    pub fn new() -> Self {
        BoundsAccumulator {
            min: Point3 {
                x: f64::INFINITY,
                y: f64::INFINITY,
                z: f64::INFINITY,
            },
            max: Point3 {
                x: f64::NEG_INFINITY,
                y: f64::NEG_INFINITY,
                z: f64::NEG_INFINITY,
            },
            any: false,
        }
    }

    pub fn add_point(&mut self, p: Point3) {
        if !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite() {
            return;
        }
        self.min = Point3 {
            x: self.min.x.min(p.x),
            y: self.min.y.min(p.y),
            z: self.min.z.min(p.z),
        };
        self.max = Point3 {
            x: self.max.x.max(p.x),
            y: self.max.y.max(p.y),
            z: self.max.z.max(p.z),
        };
        self.any = true;
    }

    pub fn add_geometry(&mut self, geometry: &SemanticGeometry) {
        self.add_geometry_transformed(geometry, &Transform3::identity());
    }

    /// Accumulate bounds of `geometry` after applying `transform`.
    ///
    /// Curved bounds are expanded conservatively by the transform's largest
    /// scale so a rotated/scaled instance still fits the box.
    pub fn add_geometry_transformed(
        &mut self,
        geometry: &SemanticGeometry,
        transform: &Transform3,
    ) {
        match geometry {
            SemanticGeometry::Line { start, end } => {
                self.add_point(transform.apply_point(*start));
                self.add_point(transform.apply_point(*end));
            }
            SemanticGeometry::Polyline { points, .. } => {
                for p in points {
                    self.add_point(transform.apply_point(*p));
                }
            }
            SemanticGeometry::Circle { center, radius, .. }
            | SemanticGeometry::Arc { center, radius, .. } => {
                self.add_sphere_transformed(*center, *radius, transform);
            }
            SemanticGeometry::Ellipse {
                center,
                major_axis,
                ratio,
                ..
            } => {
                let r = length(*major_axis).max(length(*major_axis) * ratio.abs());
                self.add_sphere_transformed(*center, r, transform);
            }
            SemanticGeometry::Spline { control_points, .. } => {
                for p in control_points {
                    self.add_point(transform.apply_point(*p));
                }
            }
            SemanticGeometry::Point(p) => self.add_point(transform.apply_point(*p)),
            SemanticGeometry::Mesh(mesh) => {
                for p in &mesh.vertices {
                    self.add_point(transform.apply_point(*p));
                }
            }
            // Inserts are expanded by `DrawingDatabase::bounds`, which owns the
            // block definitions; a bare insert contributes no bounds here.
            SemanticGeometry::Insert { .. } => {}
            SemanticGeometry::Text {
                position,
                height,
                text,
                ..
            } => {
                let p = transform.apply_point(*position);
                let h = height.abs().max(1e-9) * transform_scale(transform);
                self.add_point(p);
                self.add_point(Point3 {
                    x: p.x + h * text.chars().count() as f64,
                    y: p.y + h,
                    z: p.z,
                });
            }
            SemanticGeometry::Opaque { .. } => {}
            SemanticGeometry::Compound(children) => {
                for child in children {
                    self.add_geometry_transformed(child, transform);
                }
            }
        }
    }

    fn add_sphere_transformed(&mut self, center: Point3, radius: f64, transform: &Transform3) {
        let c = transform.apply_point(center);
        let r = radius.abs() * transform_scale(transform);
        self.add_point(Point3 {
            x: c.x - r,
            y: c.y - r,
            z: c.z - r,
        });
        self.add_point(Point3 {
            x: c.x + r,
            y: c.y + r,
            z: c.z + r,
        });
    }

    pub fn finish(&self) -> Option<(Point3, Point3)> {
        if self.any {
            Some((self.min, self.max))
        } else {
            None
        }
    }
}

/// The only sanctioned way to construct a [`DrawingDatabase`].
///
/// Validation is deferred to [`Self::finish`], which rejects dangling layer,
/// block and layout references so a partially-built database never escapes.
pub struct DrawingDatabaseBuilder {
    database: DrawingDatabase,
    errors: Vec<String>,
}

impl DrawingDatabaseBuilder {
    pub fn new(id: DatabaseId) -> Self {
        Self {
            database: DrawingDatabase {
                id,
                revision: Revision(0),
                entities: BTreeMap::new(),
                layers: BTreeMap::new(),
                blocks: BTreeMap::new(),
                layouts: BTreeMap::new(),
                styles: BTreeMap::new(),
            },
            errors: Vec::new(),
        }
    }

    pub fn insert_entity(&mut self, entity: DbEntity) -> CadResult<()> {
        if self.database.entities.contains_key(&entity.id) {
            return Err(CadError::Invariant(format!(
                "duplicate entity id {:?} during import",
                entity.id
            )));
        }
        self.database.entities.insert(entity.id, entity);
        Ok(())
    }

    pub fn insert_layer(&mut self, layer: Layer) -> CadResult<()> {
        self.database.layers.insert(layer.id, layer);
        Ok(())
    }

    pub fn insert_block(&mut self, block: BlockDefinition) -> CadResult<()> {
        self.database.blocks.insert(block.id, block);
        Ok(())
    }

    pub fn insert_layout(&mut self, layout: Layout) -> CadResult<()> {
        self.database.layouts.insert(layout.id, layout);
        Ok(())
    }

    pub fn insert_style(&mut self, style: Style) -> CadResult<()> {
        self.database.styles.insert(style.id, style);
        Ok(())
    }

    /// Validate references and produce the immutable database.
    pub fn finish(mut self) -> CadResult<DrawingDatabase> {
        // Every entity's layer must exist.
        for entity in self.database.entities.values() {
            if !self.database.layers.contains_key(&entity.layer) {
                self.errors.push(format!(
                    "entity {:?} references missing layer {:?}",
                    entity.id, entity.layer
                ));
            }
            if let SpaceId::Paper(layout) = &entity.space {
                if !self.database.layouts.contains_key(layout) {
                    self.errors.push(format!(
                        "entity {:?} references missing layout {:?}",
                        entity.id, layout
                    ));
                }
            }
        }
        // Block definitions must not reference unknown entities.
        for block in self.database.blocks.values() {
            for e in &block.entities {
                if !self.database.entities.contains_key(e) {
                    self.errors.push(format!(
                        "block {:?} references missing entity {:?}",
                        block.id, e
                    ));
                }
            }
        }
        if !self.errors.is_empty() {
            let joined = self.errors.join("; ");
            return Err(CadError::Invariant(format!(
                "import validation failed: {joined}"
            )));
        }
        Ok(self.database)
    }
}

/// The mutable annotation database; all writes go through transactions.
#[derive(Debug, Clone)]
pub struct AnnotationDatabase {
    id: DatabaseId,
    revision: Revision,
    annotations: BTreeMap<AnnotationId, Annotation>,
    saved_revision: Revision,
}

impl AnnotationDatabase {
    pub fn new(id: DatabaseId) -> Self {
        Self {
            id,
            revision: Revision(0),
            annotations: BTreeMap::new(),
            saved_revision: Revision(0),
        }
    }

    pub fn id(&self) -> DatabaseId {
        self.id
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn len(&self) -> usize {
        self.annotations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.annotations.is_empty()
    }

    pub fn annotations(&self) -> impl Iterator<Item = &Annotation> {
        self.annotations.values()
    }

    pub fn get(&self, id: AnnotationId) -> Option<&Annotation> {
        self.annotations.get(&id)
    }

    /// Called only after durable export succeeds, with the exported revision.
    pub fn mark_exported(&mut self, revision: Revision) -> CadResult<()> {
        if revision.0 > self.revision.0 {
            return Err(CadError::Invariant(
                "cannot mark a revision that has not happened".to_string(),
            ));
        }
        self.saved_revision = revision;
        Ok(())
    }

    pub fn begin(
        &mut self,
        reason: &str,
        id: TransactionId,
    ) -> CadResult<AnnotationTransaction<'_>> {
        AnnotationTransaction::new(self, reason, id)
    }

    /// Single validated write path used by both transactions and history.
    ///
    /// `changes` maps an annotation id to its new value (`None` = delete).
    /// Applies atomically: on any validation failure nothing is modified.
    pub fn apply_annotation_changes(
        &mut self,
        reason: &str,
        transaction: TransactionId,
        changes: Vec<(AnnotationId, Option<Annotation>)>,
    ) -> CadResult<ChangeSet> {
        // Validate first; do not touch the map until every change is legal.
        let mut seen = BTreeSet::new();
        for (id, change) in &changes {
            if !seen.insert(*id) {
                return Err(CadError::Invariant(format!(
                    "transaction {transaction:?} modifies annotation {id:?} twice"
                )));
            }
            if change.is_none() && !self.annotations.contains_key(id) {
                return Err(CadError::Invariant(format!(
                    "annotation {id:?} does not exist"
                )));
            }
        }

        let before = self.revision;
        if changes.is_empty() {
            // Nothing staged: do not advance the revision or claim a change.
            return Ok(ChangeSet {
                database: self.id,
                before,
                after: before,
                transaction,
                reason: reason.to_string(),
                changes: Vec::new(),
            });
        }
        let mut ordered = Vec::with_capacity(changes.len());
        for (id, change) in changes {
            match change {
                Some(annotation) => {
                    let mask = ChangeMask::GEOMETRY;
                    if self.annotations.insert(id, annotation).is_some() {
                        ordered.push(ObjectChange::Update(ObjectId(id.0), mask));
                    } else {
                        ordered.push(ObjectChange::Insert(ObjectId(id.0)));
                    }
                }
                None => {
                    self.annotations.remove(&id);
                    ordered.push(ObjectChange::Delete(ObjectId(id.0)));
                }
            }
        }
        let after = Revision(before.0 + 1);
        self.revision = after;
        Ok(ChangeSet {
            database: self.id,
            before,
            after,
            transaction,
            reason: reason.to_string(),
            changes: ordered,
        })
    }
}

/// A staged, all-or-nothing annotation transaction.
pub struct AnnotationTransaction<'a> {
    database: &'a mut AnnotationDatabase,
    reason: String,
    transaction: TransactionId,
    staged: BTreeMap<AnnotationId, Option<Annotation>>,
}

impl<'a> AnnotationTransaction<'a> {
    fn new(
        database: &'a mut AnnotationDatabase,
        reason: &str,
        transaction: TransactionId,
    ) -> CadResult<Self> {
        if reason.trim().is_empty() {
            return Err(CadError::InvalidInput(
                "transaction reason is required".to_string(),
            ));
        }
        Ok(AnnotationTransaction {
            database,
            reason: reason.to_string(),
            transaction,
            staged: BTreeMap::new(),
        })
    }

    /// Number of staged changes (used by tests and the UI status).
    pub fn staged_len(&self) -> usize {
        self.staged.len()
    }

    pub fn insert_annotation(&mut self, annotation: Annotation) -> CadResult<AnnotationId> {
        let id = annotation.id;
        if self.database.annotations.contains_key(&id) {
            return Err(CadError::Invariant(format!(
                "annotation {id:?} already exists"
            )));
        }
        self.staged.insert(id, Some(annotation));
        Ok(id)
    }

    pub fn update_annotation(&mut self, annotation: Annotation) -> CadResult<()> {
        let id = annotation.id;
        if !self.database.annotations.contains_key(&id) {
            return Err(CadError::Invariant(format!(
                "annotation {id:?} does not exist"
            )));
        }
        self.staged.insert(id, Some(annotation));
        Ok(())
    }

    pub fn delete_annotation(&mut self, id: AnnotationId) -> CadResult<()> {
        if !self.database.annotations.contains_key(&id) {
            return Err(CadError::Invariant(format!(
                "annotation {id:?} does not exist"
            )));
        }
        self.staged.insert(id, None);
        Ok(())
    }

    pub fn commit(mut self) -> CadResult<ChangeSet> {
        let staged = std::mem::take(&mut self.staged);
        let changes: Vec<(AnnotationId, Option<Annotation>)> = staged.into_iter().collect();
        self.database
            .apply_annotation_changes(&self.reason, self.transaction, changes)
    }

    /// Uncommitted staged changes are discarded on drop.
    pub fn rollback(self) {
        // Dropping `staged` is the rollback; the database was never touched.
    }
}

fn add(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn length(v: Point3) -> f64 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

/// Maximum INSERT nesting the database will expand while computing bounds.
pub const MAX_INSTANCE_DEPTH: usize = 32;

/// Conservative upper bound on a transform's linear scale (Frobenius norm of
/// the 3×3 part), used to grow circular bounds under scale/rotation.
fn transform_scale(t: &Transform3) -> f64 {
    let m = &t.matrix;
    let mut sum = 0.0;
    for row in m.iter().take(3) {
        for value in row.iter().take(3) {
            sum += value * value;
        }
    }
    sum.sqrt().max(1e-12)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ann(id: u128) -> Annotation {
        Annotation {
            id: AnnotationId(id),
            space: SpaceId::Model,
            geometry: AnnotationGeometry::Text(Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }),
            text: "note".to_string(),
            style: AnnotationStyle::default(),
            created_unix_ms: 0,
            modified_unix_ms: 0,
            anchor: None,
            precision: Precision::Analytic,
        }
    }

    #[test]
    fn transaction_commit_raises_revision_and_is_ordered() {
        let mut db = AnnotationDatabase::new(DatabaseId(1));
        let tx = db.begin("create text", TransactionId(7)).unwrap();
        let mut tx = tx;
        tx.insert_annotation(ann(10)).unwrap();
        let changes = tx.commit().unwrap();
        assert_eq!(changes.before, Revision(0));
        assert_eq!(changes.after, Revision(1));
        assert_eq!(changes.changes, vec![ObjectChange::Insert(ObjectId(10))]);
        assert_eq!(db.revision(), Revision(1));
        assert!(db.is_dirty());
        assert!(changes.follows(DatabaseId(1), Revision(0)));
    }

    #[test]
    fn dropped_transaction_leaves_no_state() {
        let mut db = AnnotationDatabase::new(DatabaseId(1));
        {
            let mut tx = db.begin("edit", TransactionId(1)).unwrap();
            tx.insert_annotation(ann(10)).unwrap();
            tx.rollback();
        }
        assert_eq!(db.len(), 0);
        assert_eq!(db.revision(), Revision(0));
    }

    #[test]
    fn failed_transaction_does_not_change_revision() {
        let mut db = AnnotationDatabase::new(DatabaseId(1));
        let mut tx = db.begin("delete missing", TransactionId(1)).unwrap();
        assert!(tx.delete_annotation(AnnotationId(99)).is_err());
        // Nothing valid was staged, so commit is a no-op rather than a bump.
        let changes = tx.commit().unwrap();
        assert!(changes.is_empty());
        assert_eq!(changes.before, changes.after);
        assert_eq!(db.revision(), Revision(0));
        assert_eq!(db.len(), 0);
    }

    #[test]
    fn double_modification_in_one_transaction_is_rejected() {
        let mut db = AnnotationDatabase::new(DatabaseId(1));
        let mut tx = db.begin("x", TransactionId(1)).unwrap();
        tx.insert_annotation(ann(1)).unwrap();
        // Second staged change for the same id is allowed at stage time but the
        // commit rejects it as an invariant violation.
        tx.staged.insert(AnnotationId(1), None);
        assert!(tx.commit().is_err());
        assert_eq!(db.len(), 0);
        assert_eq!(db.revision(), Revision(0));
    }

    #[test]
    fn export_marking_requires_a_real_revision() {
        let mut db = AnnotationDatabase::new(DatabaseId(1));
        assert!(db.mark_exported(Revision(5)).is_err());
        db.apply_annotation_changes("a", TransactionId(1), vec![(AnnotationId(1), Some(ann(1)))])
            .unwrap();
        assert!(db.is_dirty());
        db.mark_exported(Revision(1)).unwrap();
        assert!(!db.is_dirty());
    }

    #[test]
    fn builder_rejects_dangling_layer_reference() {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(42),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Line {
                start: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                end: Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            },
            draw_order: 0,
        })
        .unwrap();
        assert!(matches!(b.finish(), Err(CadError::Invariant(_))));
    }

    #[test]
    fn builder_accepts_consistent_database_and_computes_bounds() {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbLine".into(),
                revision: Revision(0),
                source_handle: Some("1A".into()),
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Line {
                start: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                end: Point3 {
                    x: 3.0,
                    y: 4.0,
                    z: 0.0,
                },
            },
            draw_order: 0,
        })
        .unwrap();
        let db = b.finish().unwrap();
        assert_eq!(db.entity_count(), 1);
        let (min, max) = db.bounds().unwrap();
        assert_eq!(min.x, 0.0);
        assert_eq!(max.x, 3.0);
        assert_eq!(max.y, 4.0);
    }

    fn point(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    fn raw_entity(id: u128, space: SpaceId, geometry: SemanticGeometry) -> DbEntity {
        DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbEntity".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(id),
            layer: LayerId(0),
            space,
            geometry,
            draw_order: id as i64,
        }
    }

    fn insert_at(block: u128, dx: f64) -> SemanticGeometry {
        SemanticGeometry::Insert {
            block: BlockId(block),
            transform: Transform3::translation(point(dx, 0.0)),
        }
    }

    #[test]
    fn block_entities_are_not_model_space_and_inserts_expand_in_bounds() {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        // Block 0 owns a unit line; two model inserts place it at x=10 and x=20.
        b.insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(2)],
        })
        .unwrap();
        b.insert_entity(raw_entity(
            2,
            SpaceId::Block(BlockId(0)),
            SemanticGeometry::Line {
                start: point(0.0, 0.0),
                end: point(1.0, 0.0),
            },
        ))
        .unwrap();
        b.insert_entity(raw_entity(1, SpaceId::Model, insert_at(0, 10.0)))
            .unwrap();
        b.insert_entity(raw_entity(3, SpaceId::Model, insert_at(0, 20.0)))
            .unwrap();
        let db = b.finish().unwrap();

        // The block definition is not drawn top-level (audit B15).
        assert_eq!(db.model_space().len(), 2);
        assert_eq!(db.block_entities(BlockId(0)).len(), 1);
        // Bounds expand both instances: 10 .. 21.
        let (min, max) = db.bounds().unwrap();
        assert_eq!(min.x, 10.0);
        assert_eq!(max.x, 21.0);
    }

    #[test]
    fn cyclic_block_reference_does_not_loop_bounds() {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        // Block 0 inserts itself.
        b.insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(2)],
        })
        .unwrap();
        b.insert_entity(raw_entity(2, SpaceId::Block(BlockId(0)), insert_at(0, 1.0)))
            .unwrap();
        b.insert_entity(raw_entity(1, SpaceId::Model, insert_at(0, 0.0)))
            .unwrap();
        let db = b.finish().unwrap();
        // Must terminate; a self-referential block contributes no bounds.
        assert!(db.bounds().is_none());
    }

    #[test]
    fn scene_identity_distinguishes_same_id_different_content() {
        // Two databases that reuse DatabaseId(1) but hold different drawings
        // must not compare equal, or a host would keep stale GPU batches on
        // screen after opening the second one (audit B04).
        let make = |end_x: f64| {
            let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
            b.insert_layer(Layer {
                id: LayerId(0),
                name: "0".into(),
                visible: true,
            })
            .unwrap();
            b.insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(1),
                    type_key: "AcDbLine".into(),
                    revision: Revision(0),
                    source_handle: Some("1A".into()),
                },
                id: EntityId(1),
                layer: LayerId(0),
                space: SpaceId::Model,
                geometry: SemanticGeometry::Line {
                    start: Point3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    end: Point3 {
                        x: end_x,
                        y: 0.0,
                        z: 0.0,
                    },
                },
                draw_order: 0,
            })
            .unwrap();
            b.finish().unwrap()
        };
        let a = make(10.0);
        let b = make(20.0);
        assert_eq!(a.id(), b.id(), "same DatabaseId");
        assert_ne!(a.scene_identity(), b.scene_identity());
        assert_eq!(a.scene_identity(), a.scene_identity());
    }
}
