//! IFC4 export writer: neutral `cad-db` meshes to `IfcTriangulatedFaceSet`.
//!
//! The writer is a thin adapter from the shared [`cad_import_core::Exporter`]
//! contract onto `ifc-lite-export`'s export-plan executor:
//!
//! 1. every model-space mesh entity is collected (a [`cad_domain::Mesh`] body,
//!    or a [`cad_db::DbEntity`]'s 3D body display cache);
//! 2. one empty IFC4 file is built with [`ifc_lite_export::plan::skeleton`]
//!    (project / site / building, SI metres);
//! 3. each mesh is added as an `IfcBuildingElementProxy` contained in the
//!    building, with its triangles as an `IfcTriangulatedFaceSet`;
//! 4. the executor's output is returned with a [`cad_import_core::CompatReport`]
//!    that names every written mesh and every drop.
//!
//! # Units
//!
//! The skeleton is SI metres, so `cad-db` coordinates are converted **once**,
//! at this boundary, by the job's `UnitContext::source` factor
//! (`mm` 0.001 … `ft` 0.3048). A missing context, or `Unit::DrawingUnits`, is a
//! named refusal ([`UNITS_UNKNOWN`]): the writer never assumes millimetres.
//!
//! # Determinism and evidence
//!
//! No clock, no RNG: the skeleton's GlobalIds and every added object's GlobalId
//! come from fixed role tags and the `cad-db` `ObjectId`, and the plan header
//! carries no timestamp. The same database always produces byte-identical
//! output, which the crate's tests pin.
//!
//! The only validation of the produced bytes is a **self-consistency**
//! roundtrip through `cad-import-ifc` (the pinned reader reads back the same
//! triangle count). That is not vendor acceptance, not a golden file and not a
//! certification: no external IFC checker or vendor application was run.
//!
//! What the writer does not represent: per-vertex colours, normals, materials,
//! property sets, layers, blocks, paper space and analytic (non-mesh) geometry.
//! Every source entity it cannot write is reported, never silently skipped; an
//! attribute that cannot ride along with a written mesh is reported once by kind
//! (`export.ifc.colors_dropped`, `export.ifc.normals_dropped`,
//! `export.ifc.layers_dropped`) and moves the verdict to `Partial`.
//!
//! An empty database is not an error: the writer still emits the valid IFC4
//! skeleton and reports `Complete`, because there was no geometry to lose.
//!
//! `WriteReport::findings` from the executor are provenance notes (the skeleton
//! writes no `IfcApplication`, so added objects name a deterministic synthesized
//! owner history). They are not geometry losses and are not copied into the
//! compatibility report; refusals are.

use std::collections::{BTreeMap, BTreeSet};

use cad_domain::{
    CadError, CadResult, Completeness, EntityId, ObjectId, SemanticGeometry, SpaceId,
    SupportStatus, Unit, UnitContext,
};
use cad_import_core::{
    CompatCounts, CompatEntry, CompatReport, Connector, ConnectorDescriptor, ExportJob,
    ExportOutput, Exporter, FidelityLevel, ReportDirection,
};
use ifc_lite_export::plan::{
    execute, skeleton, Disposition, ExportPlan, GlobalId, IfcClass, NewObject, NewPlacement,
    ObjectRef, PlanContext, Representation, Skeleton, SkeletonRoot, TriangleMesh, IDENTITY,
};

mod bim;
mod bim_value;

pub use bim::BimExportPolicy;

/// The writer's one registry key (the descriptor's extension spelling).
pub const FORMAT_ID: &str = "ifc";

/// Stable refusal code: the job carries no known source length unit.
pub const UNITS_UNKNOWN: &str = "export.ifc.units_unknown";
/// Stable drop code: the entity's geometry is not a mesh.
pub const UNSUPPORTED_GEOMETRY: &str = "export.ifc.unsupported_geometry";
/// Stable drop code: the entity's mesh has no triangle.
pub const EMPTY_MESH: &str = "export.ifc.empty_mesh";
/// Stable drop code: the entity is outside model space (paper layout or block
/// definition), which this writer does not project.
pub const SPACE_NOT_EXPORTED: &str = "export.ifc.space_not_exported";
/// Stable conversion code: the source mesh carries per-vertex colours, which an
/// `IfcTriangulatedFaceSet` cannot represent.
pub const COLORS_DROPPED: &str = "export.ifc.colors_dropped";
/// Stable conversion code: the source mesh carries per-vertex normals, which the
/// written face set does not carry.
pub const NORMALS_DROPPED: &str = "export.ifc.normals_dropped";
/// Stable conversion code: the entity carries a layer (or display attribute),
/// which the IFC skeleton does not model.
pub const LAYERS_DROPPED: &str = "export.ifc.layers_dropped";
/// Stable conversion code: a written mesh references a base-colour texture, which
/// the IFC skeleton does not project.
pub const TEXTURE_NOT_WRITTEN: &str = "export.ifc.texture_not_written";
/// Stable drop code: the plan executor refused the object's `Add`.
pub const ADD_REFUSED: &str = "export.ifc.add_refused";
/// Stable refusal code: the source has more meshes than the job's entity bound.
pub const ENTITY_LIMIT: &str = "export.ifc.entity_limit";
/// Stable refusal code: the source has more triangles than the job's bound.
pub const TRIANGLE_LIMIT: &str = "export.ifc.triangle_limit";
/// Stable refusal code: the executor could not write the file at all.
pub const EXECUTE_FAILED: &str = "export.ifc.execute_failed";
/// Stable refusal code: the IFC4 skeleton could not be built.
pub const SKELETON_FAILED: &str = "export.ifc.skeleton_failed";

/// The IFC4 writer behind the shared [`Exporter`] contract.
///
/// It holds only the caller's descriptor: the writer keeps no document state,
/// and every export reads the `cad-db` database it is handed.
pub struct IfcExporter {
    descriptor: &'static ConnectorDescriptor,
    /// How BIM-side property sets are written (ADR-0042 D10). Defaults to
    /// excluding `heuristic` evidence; the only place the caller can opt in is
    /// [`IfcExporter::with_bim_policy`], never a global constant.
    bim_policy: BimExportPolicy,
}

impl IfcExporter {
    /// A writer that reports `descriptor` as its connector.
    pub fn new(descriptor: &'static ConnectorDescriptor) -> Self {
        Self {
            descriptor,
            bim_policy: BimExportPolicy::default(),
        }
    }

    /// Write BIM property sets the document only marked `heuristic`.
    ///
    /// The default (`false`) keeps unverified values out of the file and
    /// reports them (`export.ifc.bim.unverified_skipped`): a guessed value in
    /// an IFC file would be read as if the author had declared it.
    pub fn with_bim_policy(mut self, policy: BimExportPolicy) -> Self {
        self.bim_policy = policy;
        self
    }
}

impl Connector for IfcExporter {
    fn descriptor(&self) -> &'static ConnectorDescriptor {
        self.descriptor
    }
}

impl Exporter for IfcExporter {
    fn descriptor(&self) -> &'static ConnectorDescriptor {
        self.descriptor
    }

    fn format_id(&self) -> &'static str {
        FORMAT_ID
    }

    fn export(&self, format_id: &'static str, job: &ExportJob<'_>) -> CadResult<ExportOutput> {
        // The registry key must name what this writer actually writes: another
        // key would stamp a descriptor into the report that does not describe
        // the bytes, so the mismatch is refused by name.
        if format_id != FORMAT_ID {
            return Err(CadError::Unsupported(format!(
                "export.{format_id}.mismatched_descriptor: exporter writes {FORMAT_ID}"
            )));
        }
        let units = job.units.ok_or_else(|| {
            CadError::Unsupported(format!(
                "{UNITS_UNKNOWN}: the export job carries no unit context, and the IFC4 skeleton is \
                 SI metres; the source length unit must be known rather than assumed"
            ))
        })?;
        let factor = metre_factor(units)?;

        let mut drops = Vec::<DropRecord>::new();
        let mut attributes = AttributeLoss::default();
        let meshes = collect_meshes(job, factor, &mut drops, &mut attributes)?;
        // Zero meshes is a valid outcome: the skeleton below is a complete IFC4
        // project on its own, so an empty database writes a skeleton-only file
        // and reports `Complete` (there is no geometry to lose). That is
        // deliberately different from the SKP writer, which refuses an empty
        // model because a `.skp` with no geometry says nothing.
        //
        // A database with no BIM data takes the original path below: same plan,
        // same bytes, same report. Only a database that actually carries BIM
        // data goes through `bim`, so existing output is untouched (ADR-0042
        // D10).
        let planned = if job.database.bim_elements().is_empty() {
            plain_plan(&meshes)?
        } else {
            bim::plan(job.database, &meshes, self.bim_policy)?
        };

        let mut bytes = Vec::new();
        let written = execute(&planned.skeleton_bytes, &planned.plan, &mut bytes)
            .map_err(|error| CadError::Unsupported(format!("{EXECUTE_FAILED}: {error}")))?;

        // One refusal is one mesh the executor did not write. It is recorded by
        // name, never dropped silently.
        let mut gid_index = BTreeMap::new();
        for (index, global_id) in planned.element_ids.iter().enumerate() {
            gid_index.insert(global_id.encode(), index);
        }
        let mut refused_meshes = 0usize;
        for refusal in &written.refusals {
            let object = match &refusal.object {
                ObjectRef::Guid(guid) => gid_index.get(guid).map(|&index| &meshes[index]),
                // `ObjectRef` is `#[non_exhaustive]`; the keys this writer
                // passes are always GlobalIds, and an id-keyed refusal is not
                // tied to a source entity here.
                _ => None,
            };
            if object.is_some() {
                refused_meshes += 1;
            }
            drops.push(DropRecord {
                object: object.map(|mesh| mesh.object),
                type_key: object.map(|mesh| mesh.type_key.clone()),
                code: ADD_REFUSED,
                message: format!(
                    "IFC plan executor refused {}: {}",
                    object.map_or("an added object", |mesh| mesh.type_key.as_str()),
                    refusal.reason
                ),
            });
        }

        // A BIM plan also adds storeys, which are not source meshes: counting
        // `written.added` would report them as exact geometry. `exact` is
        // therefore the number of meshes the executor wrote.
        let exact = if planned.bim {
            meshes.len().saturating_sub(refused_meshes)
        } else {
            written.added
        };
        let report = compat_report(
            exact,
            &drops,
            &attributes,
            &planned.bim_entries,
            bytes.len(),
        );
        Ok(ExportOutput { bytes, report })
    }
}

/// One source entity the writer projects as an IFC mesh.
pub(crate) struct CollectedMesh {
    pub(crate) object: ObjectId,
    /// The element row's id: where its BIM side data is looked up.
    pub(crate) entity: EntityId,
    /// The `cad-db` type key, written as the object's `Name` when non-empty.
    pub(crate) type_key: String,
    /// Positions in metres, already converted at this boundary.
    pub(crate) positions: Vec<[f64; 3]>,
    pub(crate) triangles: Vec<[u32; 3]>,
}

/// One entity that is not in the output, and why.
struct DropRecord {
    /// The source object, when the drop is tied to one.
    object: Option<ObjectId>,
    type_key: Option<String>,
    code: &'static str,
    message: String,
}

/// Source attributes the IFC skeleton cannot carry.
///
/// Unlike a [`DropRecord`], the geometry itself **is** written: only the
/// attribute is lost, so each non-zero count becomes one `Partial` conversion
/// entry and moves the report to `Completeness::Partial` (never `Missing`).
#[derive(Default)]
struct AttributeLoss {
    /// Meshes whose `Mesh::colors` is non-empty.
    colours: usize,
    /// Meshes whose `Mesh::normals` is non-empty.
    normals: usize,
    /// Entities on a non-zero layer or carrying a display attribute.
    layers: usize,
    /// Written meshes that reference a document base-colour texture.
    ///
    /// Kept per entity because the message names both the source entity and the
    /// texture key; the loss itself is one conversion per affected mesh.
    textures: Vec<TextureLoss>,
}

/// One written mesh whose base-colour texture is not in the IFC file.
struct TextureLoss {
    object: ObjectId,
    type_key: String,
    key: String,
}

impl AttributeLoss {
    /// One conversion entry per kind that actually occurred: an absent attribute
    /// never invents an entry.
    ///
    /// `dropped` holds the entities the file does not contain at all: a texture
    /// on one of them is part of that drop and is never reported twice.
    fn entries(&self, dropped: &BTreeSet<ObjectId>) -> Vec<CompatEntry> {
        let mut entries = Vec::new();
        if self.colours > 0 {
            entries.push(CompatEntry {
                connector: "ifc",
                direction: ReportDirection::Export,
                level: FidelityLevel::L1,
                object: None,
                type_key: None,
                status: SupportStatus::Partial,
                code: COLORS_DROPPED.to_string(),
                message: format!(
                    "per-vertex colours on {} mesh(es) are not written: an \
                     IfcTriangulatedFaceSet carries positions and triangles only",
                    self.colours
                ),
            });
        }
        if self.normals > 0 {
            entries.push(CompatEntry {
                connector: "ifc",
                direction: ReportDirection::Export,
                level: FidelityLevel::L1,
                object: None,
                type_key: None,
                status: SupportStatus::Partial,
                code: NORMALS_DROPPED.to_string(),
                message: format!(
                    "per-vertex normals on {} mesh(es) are not written: the face set's \
                     normals are derived by the reader",
                    self.normals
                ),
            });
        }
        if self.layers > 0 {
            entries.push(CompatEntry {
                connector: "ifc",
                direction: ReportDirection::Export,
                level: FidelityLevel::L1,
                object: None,
                type_key: None,
                status: SupportStatus::Partial,
                code: LAYERS_DROPPED.to_string(),
                message: format!(
                    "layers or display attributes on {} entity(ies) are not written: the \
                     IFC skeleton has no layer or style projection",
                    self.layers
                ),
            });
        }
        // One entry per written mesh that references a texture: the face set is
        // written, the texture is not. An entity the file does not contain at all
        // was already reported as a drop, so its texture is not reported again.
        for loss in &self.textures {
            if dropped.contains(&loss.object) {
                continue;
            }
            entries.push(CompatEntry {
                connector: "ifc",
                direction: ReportDirection::Export,
                level: FidelityLevel::L1,
                object: Some(loss.object),
                type_key: Some(loss.type_key.clone()),
                status: SupportStatus::Partial,
                code: TEXTURE_NOT_WRITTEN.to_string(),
                message: format!(
                    "entity #{} ({}) references base-colour texture \"{}\", which the IFC \
                     skeleton has no texture projection for; the mesh geometry was written \
                     without it",
                    loss.object.0, loss.type_key, loss.key
                ),
            });
        }
        entries
    }
}

/// Whether an entity carries layer/display information the IFC skeleton drops.
///
/// The comparison is on the display attributes themselves, not the whole record:
/// `geometry_source`/`linetype_scale` are storage provenance, not a layer
/// attribute a user chose.
fn carries_layer_attributes(entity: &cad_db::DbEntity, database: &cad_db::DrawingDatabase) -> bool {
    if entity.layer.0 != 0 {
        return true;
    }
    let attributes = database.entity_render_attributes(entity.id);
    let defaults = cad_db::EntityRenderAttributes::default();
    attributes.color != defaults.color
        || attributes.transparency != defaults.transparency
        || attributes.lineweight != defaults.lineweight
        || attributes.linetype != defaults.linetype
}

/// The metre factor of the job's source unit, or a named refusal.
///
/// `Unit::DrawingUnits` is not a length unit: nothing states how large one unit
/// is, so it is refused rather than guessed.
fn metre_factor(units: &UnitContext) -> CadResult<f64> {
    match &units.source {
        Unit::Millimeter => Ok(0.001),
        Unit::Centimeter => Ok(0.01),
        Unit::Meter => Ok(1.0),
        Unit::Inch => Ok(0.0254),
        Unit::Foot => Ok(0.3048),
        Unit::DrawingUnits => Err(CadError::Unsupported(format!(
            "{UNITS_UNKNOWN}: the source unit is drawing units, which state no length; the IFC4 \
             skeleton is SI metres, so the source unit must be known rather than assumed"
        ))),
    }
}

/// Every model-space mesh the writer projects, with everything else reported.
fn collect_meshes(
    job: &ExportJob<'_>,
    factor: f64,
    drops: &mut Vec<DropRecord>,
    attributes: &mut AttributeLoss,
) -> CadResult<Vec<CollectedMesh>> {
    let mut meshes = Vec::new();
    let mut triangles = 0usize;
    for entity in job.database.entities() {
        let object = entity.object.id;
        let type_key = entity.object.type_key.clone();
        if entity.space != SpaceId::Model {
            let message = format!(
                "entity #{} is outside model space ({:?}); the IFC writer projects model \
                 space only",
                object.0, entity.space
            );
            drops.push(DropRecord {
                object: Some(object),
                type_key: Some(type_key),
                code: SPACE_NOT_EXPORTED,
                message,
            });
            continue;
        }
        let mesh = match &entity.geometry {
            SemanticGeometry::Mesh(mesh) => mesh,
            // A 3D body's derived display mesh is the drawable geometry; the
            // exact B-rep is not written (and the report says so when the cache
            // is empty).
            SemanticGeometry::Body3d(body) => &body.display.mesh,
            _ => {
                let message = format!(
                    "entity #{} ({}) is not mesh geometry; the IFC writer writes meshes as \
                     IfcTriangulatedFaceSet only",
                    object.0, type_key
                );
                drops.push(DropRecord {
                    object: Some(object),
                    type_key: Some(type_key),
                    code: UNSUPPORTED_GEOMETRY,
                    message,
                });
                continue;
            }
        };
        if mesh.triangles.is_empty() {
            let message = format!(
                "entity #{} ({}) has no triangle; an empty mesh has no IfcTriangulatedFaceSet \
                 to write",
                object.0, type_key
            );
            drops.push(DropRecord {
                object: Some(object),
                type_key: Some(type_key),
                code: EMPTY_MESH,
                message,
            });
            continue;
        }
        // The geometry is written; these attributes are not. Each is reported
        // once by kind, and only when a source entity actually carries it.
        if !mesh.colors.is_empty() {
            attributes.colours += 1;
        }
        if !mesh.normals.is_empty() {
            attributes.normals += 1;
        }
        if carries_layer_attributes(entity, job.database) {
            attributes.layers += 1;
        }
        if let Some(key) = mesh.texture.as_deref() {
            attributes.textures.push(TextureLoss {
                object,
                type_key: type_key.clone(),
                key: key.to_string(),
            });
        }
        meshes.push(CollectedMesh {
            object,
            entity: entity.id,
            type_key,
            positions: mesh
                .vertices
                .iter()
                .map(|point| [point.x * factor, point.y * factor, point.z * factor])
                .collect(),
            triangles: mesh.triangles.clone(),
        });
        triangles = triangles.saturating_add(mesh.triangles.len());
        if meshes.len() > job.limits.max_output_entities {
            return Err(CadError::Unsupported(format!(
                "{ENTITY_LIMIT}: the source has more than {} mesh entities, over the export \
                 limit",
                job.limits.max_output_entities
            )));
        }
        if triangles > job.limits.max_output_triangles {
            return Err(CadError::Unsupported(format!(
                "{TRIANGLE_LIMIT}: the source has more than {} triangles, over the export limit",
                job.limits.max_output_triangles
            )));
        }
    }
    Ok(meshes)
}

/// One export plan plus what the compatibility report needs about it.
pub(crate) struct PlannedExport {
    /// The skeleton the plan is executed against.
    pub(crate) skeleton_bytes: Vec<u8>,
    pub(crate) plan: ExportPlan,
    /// The GlobalId of each collected mesh, in mesh order.
    pub(crate) element_ids: Vec<GlobalId>,
    /// BIM-side conversion entries (`export.ifc.bim.*`); empty without BIM.
    pub(crate) bim_entries: Vec<CompatEntry>,
    /// Whether this plan wrote BIM semantics (storeys and friends).
    pub(crate) bim: bool,
}

/// The project / site / building GlobalIds and the skeleton bytes.
///
/// Shared by the plain and the BIM plan so both write the same spatial root
/// (the BIM path only adds storeys and semantics under it).
pub(crate) fn skeleton_roots() -> CadResult<([GlobalId; 3], Vec<u8>)> {
    let gids = [
        stable_global_id("yacr/project", 0),
        stable_global_id("yacr/site", 0),
        stable_global_id("yacr/building", 0),
    ];
    let mut ids = BTreeSet::new();
    for id in gids {
        if !ids.insert(id.0) {
            return Err(CadError::Invariant(
                "IFC skeleton GlobalIds collided".into(),
            ));
        }
    }

    let source = Skeleton::new(
        SkeletonRoot::new(gids[0]).with_name("yacr project"),
        SkeletonRoot::new(gids[1]).with_name("yacr site"),
        SkeletonRoot::new(gids[2]).with_name("yacr building"),
    )
    .with_file_name("export.ifc");
    let skeleton_bytes = skeleton(&source)
        .map_err(|reason| CadError::Unsupported(format!("{SKELETON_FAILED}: {reason}")))?;
    Ok((gids, skeleton_bytes))
}

/// The original plan: every mesh as an `IfcBuildingElementProxy` in the
/// building, with no BIM semantics (byte-for-byte what this writer always
/// produced, ADR-0042 D10).
pub(crate) fn plain_plan(meshes: &[CollectedMesh]) -> CadResult<PlannedExport> {
    let (gids, skeleton_bytes) = skeleton_roots()?;
    let building = gids[2];
    let mut ids = BTreeSet::new();
    for id in gids {
        ids.insert(id.0);
    }

    let container = ObjectRef::Guid(building.encode());
    let mut dispositions = Vec::with_capacity(meshes.len());
    let mut element_ids = Vec::with_capacity(meshes.len());
    for mesh in meshes {
        let global_id = mesh_global_id(mesh.object);
        if !ids.insert(global_id.0) {
            return Err(CadError::Invariant(format!(
                "IFC export GlobalId collision for object #{}",
                mesh.object.0
            )));
        }
        let mut object = NewObject::new(
            global_id,
            IfcClass::BuildingElementProxy,
            NewPlacement::new(IDENTITY),
        )
        .with_container(container.clone())
        .with_representation(Representation::Mesh(TriangleMesh::new(
            mesh.positions.clone(),
            mesh.triangles.clone(),
        )));
        if !mesh.type_key.trim().is_empty() {
            object = object.with_name(mesh.type_key.clone());
        } else {
            object = object.with_tag(format!("object:{}", mesh.object.0));
        }
        dispositions.push((
            ObjectRef::Guid(global_id.encode()),
            Disposition::Add(Box::new(object)),
        ));
        element_ids.push(global_id);
    }

    let plan =
        ExportPlan::new(dispositions).with_context(PlanContext::new().with_file_name("export.ifc"));
    Ok(PlannedExport {
        skeleton_bytes,
        plan,
        element_ids,
        bim_entries: Vec::new(),
        bim: false,
    })
}

/// The compatibility report for one export.
fn compat_report(
    exact: usize,
    drops: &[DropRecord],
    attributes: &AttributeLoss,
    bim_entries: &[CompatEntry],
    bytes: usize,
) -> CompatReport {
    let mut entries = Vec::with_capacity(drops.len() + 3 + bim_entries.len());
    let mut reasons: Vec<String> = Vec::new();
    for drop in drops {
        entries.push(CompatEntry {
            connector: "ifc",
            direction: ReportDirection::Export,
            level: FidelityLevel::L1,
            object: drop.object,
            type_key: drop.type_key.clone(),
            // Every drop is a named loss: the entity is not in the output.
            status: SupportStatus::Unsupported,
            code: drop.code.to_string(),
            message: drop.message.clone(),
        });
        if !reasons.contains(&drop.message) {
            reasons.push(drop.message.clone());
        }
    }
    // An entity the file does not contain at all is already reported as a drop,
    // so a texture on it must not be reported a second time.
    let dropped_objects: BTreeSet<ObjectId> = drops.iter().filter_map(|drop| drop.object).collect();
    let attribute_entries = attributes.entries(&dropped_objects);
    reasons.extend(attribute_entries.iter().map(|entry| entry.message.clone()));
    // Each attribute entry is one conversion (one per attribute kind actually
    // lost, or one per textured mesh), matching the `Partial` entries above.
    let mut converted = attribute_entries.len();
    entries.extend(attribute_entries);
    // BIM-side losses are conversions too: the geometry is written, a storey,
    // type, property set or GlobalId could not be. Empty without BIM.
    converted += bim_entries.len();
    reasons.extend(bim_entries.iter().map(|entry| entry.message.clone()));
    entries.extend_from_slice(bim_entries);
    CompatReport {
        connector: "ifc",
        direction: ReportDirection::Export,
        level: FidelityLevel::L1,
        counts: CompatCounts {
            exact,
            converted,
            dropped: drops.len(),
            bytes,
        },
        entries,
        capabilities: Vec::new(),
        completeness: if !drops.is_empty() {
            Completeness::Missing(reasons)
        } else if !reasons.is_empty() {
            // Some geometry was written but an attribute on it was not: the
            // report is partial, never a clean `Complete`.
            Completeness::Partial(reasons)
        } else {
            Completeness::Complete
        },
    }
}

/// A deterministic `GlobalId` for one role and index: no clock, no RNG.
///
/// FNV-1a over the role tag and the 128-bit index. The same database therefore
/// always produces the same GlobalIds and byte-identical output.
pub(crate) fn stable_global_id(role: &str, index: u128) -> GlobalId {
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    let mut hash = OFFSET;
    for byte in role.as_bytes().iter().chain(&[0xff]) {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    let mut remaining = index;
    for _ in 0..16 {
        hash ^= remaining & 0xff;
        hash = hash.wrapping_mul(PRIME);
        remaining >>= 8;
    }
    GlobalId(hash)
}

/// The GlobalId of one exported mesh entity.
pub(crate) fn mesh_global_id(object: ObjectId) -> GlobalId {
    stable_global_id("yacr/mesh", object.0)
}

#[cfg(test)]
mod tests;
