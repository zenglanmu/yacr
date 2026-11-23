//! BIM semantics → IFC4 (ADR-0042 Phase 2, D10).
//!
//! The plain writer (`super::plain_plan`) is untouched: a database with no
//! `bim_elements` still writes one `IfcBuildingElementProxy` per mesh and
//! produces **byte-identical** output. This module is used only when the
//! document actually carries BIM data, and then it maps that data one-way:
//!
//! | `cad-db` | IFC4 |
//! |---|---|
//! | every `BimDefinition::Level` | `IfcBuildingStorey` (`Elevation` in metres), sorted by `(elevation, name, id)` |
//! | `BimElement.level` | the element's container (a storey), else the building |
//! | `BimElement.category` | `IfcClass::Named(category)` when the token is a declared concrete product; else `IfcBuildingElementProxy` |
//! | `BimElement.element_type` | `TypeLink` to an `IfcTypeObject` (`<Category>Type` whitelist only) |
//! | `BimElement.material` | `MaterialLink::New` (one `IfcMaterial` per distinct name) |
//! | `BimElement.psets` | `IfcPropertySet` + `IfcRelDefinesByProperties` |
//! | `BimElement.source.id` | the element's `GlobalId` when it is a valid IFC id |
//! | `BimRelationKind::Contains` | the element's `IfcRelContainedInSpatialStructure` container (overrides its level) |
//! | `BimRelationKind::Nests` | `NewObject::with_nest_in` (`IfcRelNests`) |
//!
//! # Relations
//!
//! `tables/bim-relations.json` is read in `(kind, parent, child)` order and only
//! two families are writable with the pinned plan API
//! (`ifc-lite-export` 28.3.0): one spatial container and one nesting host per
//! written object. `aggregates`, `group`, `connects` and every `other:<token>`
//! edge have no plan-level expression (there is no way to relate two products),
//! so they are **reported per kind** (`export.ifc.bim.relation_not_written`) and
//! never faked into a container or a nest. An endpoint this export did not
//! write is reported (`export.ifc.bim.relation_endpoint_missing`), and the
//! relation is skipped — no panic, no invented object.
//!
//! # No guessing
//!
//! The writable categories are an explicit, finite table of **concrete IFC4
//! `IfcProduct` classes** ([`BIM_CATEGORIES`]); a token outside it is not
//! guessed into a nearby class, it is written as `IfcBuildingElementProxy` and
//! reported (`export.ifc.bim.category_not_written`). Types are only derived for
//! the categories in [`BIM_TYPE_CLASSES`]; a type whose category or declared
//! class is outside that table is skipped and reported
//! (`export.ifc.bim.type_not_written`), never given an invented class.
//!
//! # GlobalIds
//!
//! IFC's 22-character `IfcGloballyUniqueId` is not decoded by
//! `ifc-lite-core` 28.3.0's public surface (only the encoder is re-exported),
//! so [`decode_global_id`] is this crate's own inverse of
//! [`ifc_lite_export::plan::GlobalId::encode`], pinned by known vectors. A
//! decoded source id is reused **only** when it is valid and not already used
//! by the skeleton, a storey/type or another mesh; otherwise the deterministic
//! `stable_global_id`/`mesh_global_id` derivation is used and the fallback is
//! reported (`export.ifc.bim.global_id_invalid|global_id_collision`). An IFC
//! `IfcMaterial` is an unrooted entity (no `GlobalId` at all), so materials are
//! identified by name and de-duplicated by the executor.
//!
//! # Evidence
//!
//! A property set whose element/type is marked [`BimEvidence::Heuristic`] is
//! **not** written by default: an IFC file cannot express "this is a guess".
//! [`BimExportPolicy::include_unverified`] opts in explicitly; the skip is
//! always counted (`export.ifc.bim.unverified_skipped`).
//!
//! Every conversion loss below moves the report to `Partial` (the geometry is
//! still written) and never changes a failure into an empty success.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use cad_connector_core::{CompatEntry, FidelityLevel, ReportDirection};
use cad_db::{BimDefinition, BimEvidence, BimKey, BimPropertySet, BimSource, DrawingDatabase};
use cad_domain::{CadError, CadResult, ObjectId, SupportStatus};
use ifc_lite_export::plan::{
    Disposition, ExportPlan, GlobalId, IfcClass, MaterialLink, NewMaterial, NewObject,
    NewPlacement, NewPropertySet, NewType, ObjectRef, PlanContext, Representation, TriangleMesh,
    TypeLink, IDENTITY,
};

use super::{mesh_global_id, skeleton_roots, stable_global_id, CollectedMesh, PlannedExport};
use crate::export::bim_relations::{plan_relations, RelationInputs};
use crate::export::bim_value::{convert_psets, decode_global_id, hash_text};

/// Stable conversion code: an element names a storey the writer could not
/// place it in; it stays in the building.
pub const STOREY_NOT_WRITTEN: &str = "export.ifc.bim.storey_not_written";
/// Stable conversion code: a category token is not a concrete IFC4 product
/// class this writer declares; the element is written as a proxy.
pub const CATEGORY_NOT_WRITTEN: &str = "export.ifc.bim.category_not_written";
/// Stable conversion code: a type definition has no writable `IfcTypeObject`
/// class; the element is written without a type.
pub const TYPE_NOT_WRITTEN: &str = "export.ifc.bim.type_not_written";
/// Stable conversion code: a material reference does not resolve to a material
/// definition, or the written class cannot carry one; the element is written
/// without a material.
pub const MATERIAL_NOT_WRITTEN: &str = "export.ifc.bim.material_not_written";
/// Stable conversion code: one or more BIM property values are not written as
/// declared (aggregated into one entry).
pub const PROPERTY_NOT_WRITTEN: &str = "export.ifc.bim.property_not_written";
/// Stable conversion code: heuristic property sets were skipped.
pub const UNVERIFIED_SKIPPED: &str = "export.ifc.bim.unverified_skipped";
/// Stable conversion code: a source `GlobalId` is not valid IFC; a derived id
/// was written instead.
pub const GLOBAL_ID_INVALID: &str = "export.ifc.bim.global_id_invalid";
/// Stable conversion code: a source `GlobalId` is already used by another
/// written record; a derived id was written instead.
pub const GLOBAL_ID_COLLISION: &str = "export.ifc.bim.global_id_collision";
/// Stable conversion code: relation edges of one kind are not written (the plan
/// API has no expression for them, or an extra edge conflicts with the one
/// already chosen). One entry per kind, with the count.
pub const RELATION_NOT_WRITTEN: &str = "export.ifc.bim.relation_not_written";
/// Stable conversion code: relation edges name an endpoint this export did not
/// write; those edges are skipped. One entry per kind, with the count.
pub const RELATION_ENDPOINT_MISSING: &str = "export.ifc.bim.relation_endpoint_missing";

/// Whether property sets the document marks as heuristic are written.
///
/// Defaults to `false`: a guessed value would be read back as if the author had
/// declared it. Set through [`super::IfcExporter::with_bim_policy`], never a
/// global constant, so callers state the decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BimExportPolicy {
    /// Write [`BimEvidence::Heuristic`] property sets (default: skip + report).
    pub include_unverified: bool,
}

/// The concrete IFC4 product classes this writer will name as an element class.
///
/// Every entry was checked against the IFC4 EXPRESS registry that
/// `ifc-lite-core` 28.3.0 carries (`entity_info_for_schema`): declared, not
/// abstract, and a subtype of `IfcProduct`/`IfcElement` (`IfcSpace` is a
/// spatial element, aggregated under the storey instead of contained).
/// Anything else is an explicit `IfcBuildingElementProxy` fallback.
const BIM_CATEGORIES: &[&str] = &[
    "IfcAirTerminal",
    "IfcAirTerminalBox",
    "IfcBeam",
    "IfcBeamStandardCase",
    "IfcBuildingElementPart",
    "IfcBuildingElementProxy",
    "IfcCableCarrierFitting",
    "IfcCableCarrierSegment",
    "IfcCableFitting",
    "IfcCableSegment",
    "IfcChimney",
    "IfcCivilElement",
    "IfcColumn",
    "IfcColumnStandardCase",
    "IfcCovering",
    "IfcCurtainWall",
    "IfcDistributionControlElement",
    "IfcDistributionElement",
    "IfcDistributionFlowElement",
    "IfcDoor",
    "IfcDoorStandardCase",
    "IfcDuctFitting",
    "IfcDuctSegment",
    "IfcElectricAppliance",
    "IfcElementAssembly",
    "IfcEnergyConversionDevice",
    "IfcFlowController",
    "IfcFlowFitting",
    "IfcFlowMovingDevice",
    "IfcFlowSegment",
    "IfcFlowStorageDevice",
    "IfcFlowTerminal",
    "IfcFlowTreatmentDevice",
    "IfcFooting",
    "IfcFurnishingElement",
    "IfcFurniture",
    "IfcGeographicElement",
    "IfcLightFixture",
    "IfcMember",
    "IfcMemberStandardCase",
    "IfcOutlet",
    "IfcPile",
    "IfcPipeFitting",
    "IfcPipeSegment",
    "IfcPlate",
    "IfcPlateStandardCase",
    "IfcRailing",
    "IfcRamp",
    "IfcRampFlight",
    "IfcRoof",
    "IfcSanitaryTerminal",
    "IfcShadingDevice",
    "IfcSlab",
    "IfcSlabElementedCase",
    "IfcSlabStandardCase",
    "IfcSpace",
    "IfcStair",
    "IfcStairFlight",
    "IfcSystemFurnitureElement",
    "IfcTransportElement",
    "IfcWall",
    "IfcWallElementedCase",
    "IfcWallStandardCase",
    "IfcWindow",
    "IfcWindowStandardCase",
];

/// The type class each category may be typed by (ADR-0042 D10 white list).
///
/// `(<type class>, <element category>)`. The `*StandardCase` / `*ElementedCase`
/// aliases resolve to their base type, which is exactly what
/// `ifc-lite-export`'s `type_class_for` asks for. A category outside this table
/// gets no type at all.
const BIM_TYPE_CLASSES: &[(&str, &str)] = &[
    ("IfcBeamType", "IfcBeam"),
    ("IfcBeamType", "IfcBeamStandardCase"),
    ("IfcColumnType", "IfcColumn"),
    ("IfcColumnType", "IfcColumnStandardCase"),
    ("IfcCoveringType", "IfcCovering"),
    ("IfcDoorType", "IfcDoor"),
    ("IfcDoorType", "IfcDoorStandardCase"),
    ("IfcMemberType", "IfcMember"),
    ("IfcMemberType", "IfcMemberStandardCase"),
    ("IfcPlateType", "IfcPlate"),
    ("IfcPlateType", "IfcPlateStandardCase"),
    ("IfcRailingType", "IfcRailing"),
    ("IfcRoofType", "IfcRoof"),
    ("IfcSlabType", "IfcSlab"),
    ("IfcSlabType", "IfcSlabElementedCase"),
    ("IfcSlabType", "IfcSlabStandardCase"),
    ("IfcSpaceType", "IfcSpace"),
    ("IfcStairType", "IfcStair"),
    ("IfcWallType", "IfcWall"),
    ("IfcWallType", "IfcWallElementedCase"),
    ("IfcWallType", "IfcWallStandardCase"),
    ("IfcWindowType", "IfcWindow"),
    ("IfcWindowType", "IfcWindowStandardCase"),
];

/// The concrete category spelling for a token, case-insensitively.
///
/// Case is normalised (`IFCWALL` → `IfcWall`); nothing else is inferred.
fn category_token(raw: &str) -> Option<&'static str> {
    let raw = raw.trim();
    BIM_CATEGORIES
        .iter()
        .copied()
        .find(|category| category.eq_ignore_ascii_case(raw))
}

/// The type class a category may be typed by.
fn type_class_for(category: &str) -> Option<&'static str> {
    BIM_TYPE_CLASSES
        .iter()
        .find(|(_, element)| element.eq_ignore_ascii_case(category))
        .map(|(type_class, _)| *type_class)
}

/// The class a type is written with: the declared class when it is the one the
/// category takes, else the derived `<Category>Type`; `None` when the category
/// has no whitelisted type or the declared class is inconsistent.
fn writable_type_class(category: &str, declared: Option<&str>) -> Option<&'static str> {
    let expected = type_class_for(category)?;
    match declared {
        None => Some(expected),
        Some(raw) => {
            let canonical = BIM_TYPE_CLASSES
                .iter()
                .map(|(type_class, _)| *type_class)
                .find(|type_class| type_class.eq_ignore_ascii_case(raw.trim()))?;
            (canonical == expected).then_some(canonical)
        }
    }
}

/// Conversion counts, one aggregated entry per non-zero kind.
#[derive(Default)]
pub(crate) struct Counts {
    pub(crate) storeys: usize,
    pub(crate) categories: usize,
    pub(crate) types: usize,
    pub(crate) properties: usize,
    pub(crate) materials: usize,
    pub(crate) unverified: usize,
    pub(crate) global_id_invalid: usize,
    pub(crate) global_id_collisions: usize,
    /// Relation edges not written, by kind token (ADR-0042 D10).
    pub(crate) relation_not_written: BTreeMap<String, usize>,
    /// Relation edges skipped for a missing endpoint, by kind token.
    pub(crate) relation_endpoint_missing: BTreeMap<String, usize>,
}

impl Counts {
    pub(crate) fn entries(&self) -> Vec<CompatEntry> {
        let mut entries = Vec::new();
        let mut push = |count: usize, code: &str, message: String| {
            if count == 0 {
                return;
            }
            entries.push(CompatEntry {
                connector: "ifc",
                direction: ReportDirection::Export,
                level: FidelityLevel::L1,
                object: None,
                type_key: None,
                status: SupportStatus::Partial,
                code: code.to_string(),
                message,
            });
        };
        push(
            self.storeys,
            STOREY_NOT_WRITTEN,
            format!(
                "{count} element(s) reference a storey that is not a level definition; \
                 they are written in the building instead",
                count = self.storeys
            ),
        );
        push(
            self.categories,
            CATEGORY_NOT_WRITTEN,
            format!(
                "{count} element category token(s) are not a concrete IFC4 product class this \
                 writer declares; those elements are written as IfcBuildingElementProxy",
                count = self.categories
            ),
        );
        push(
            self.types,
            TYPE_NOT_WRITTEN,
            format!(
                "{count} BIM type definition(s) have no writable IfcTypeObject class (the \
                 category is not in the derived-type table, or the declared class does not \
                 match it); those elements are written without a type",
                count = self.types
            ),
        );
        push(
            self.properties,
            PROPERTY_NOT_WRITTEN,
            format!(
                "{count} BIM property value(s) are not written as declared (an explicit null, a \
                 unit this writer cannot convert, or a measure type the value does not satisfy); \
                 no value is relabelled",
                count = self.properties
            ),
        );
        push(
            self.materials,
            MATERIAL_NOT_WRITTEN,
            format!(
                "{count} element material reference(s) are not written: the reference does not \
                 resolve to a material definition, or the element's class cannot carry a material \
                 in IFC4",
                count = self.materials
            ),
        );
        push(
            self.unverified,
            UNVERIFIED_SKIPPED,
            format!(
                "{count} property set(s) marked `heuristic` are not written: an IFC file cannot \
                 say the value is unverified (set the BIM export policy to include them)",
                count = self.unverified
            ),
        );
        push(
            self.global_id_invalid,
            GLOBAL_ID_INVALID,
            format!(
                "{count} source GlobalId(s) are not a valid 22-character IFC GlobalId; a \
                 deterministic id was written instead",
                count = self.global_id_invalid
            ),
        );
        push(
            self.global_id_collisions,
            GLOBAL_ID_COLLISION,
            format!(
                "{count} source GlobalId(s) are already used by another written record; a \
                 deterministic id was written instead",
                count = self.global_id_collisions
            ),
        );
        // One entry per relation kind: the count plus the kind's own token. A
        // kind that was fully written never invents an entry.
        for (kind, count) in &self.relation_not_written {
            entries.push(CompatEntry {
                connector: "ifc",
                direction: ReportDirection::Export,
                level: FidelityLevel::L1,
                object: None,
                type_key: None,
                status: SupportStatus::Partial,
                code: RELATION_NOT_WRITTEN.to_string(),
                message: format!(
                    "{count} relation edge(s) of kind `{kind}` are not written: the export \
                     plan can express one spatial container and one nesting host per object \
                     (IfcRelContainedInSpatialStructure / IfcRelNests) and nothing else, so \
                     the edge is reported rather than faked into either",
                ),
            });
        }
        for (kind, count) in &self.relation_endpoint_missing {
            entries.push(CompatEntry {
                connector: "ifc",
                direction: ReportDirection::Export,
                level: FidelityLevel::L1,
                object: None,
                type_key: None,
                status: SupportStatus::Partial,
                code: RELATION_ENDPOINT_MISSING.to_string(),
                message: format!(
                    "{count} relation edge(s) of kind `{kind}` are skipped: an endpoint is \
                     not an object this export writes (it has no geometry row, or it was \
                     not written); no endpoint is invented",
                ),
            });
        }
        entries
    }
}

/// One level row, copied out so the definition table is not borrowed while the
/// plan is built.
struct LevelRow {
    id: String,
    name: String,
    elevation: Option<f64>,
    source_id: Option<String>,
    /// The `(connector, document_id, id)` key a relation endpoint uses, when
    /// the source declared both halves.
    key: Option<BimKey>,
}

/// One type row (same reason).
struct TypeRow {
    name: String,
    class: Option<String>,
    category: Option<String>,
    psets: Vec<BimPropertySet>,
    evidence: BimEvidence,
    source_id: Option<String>,
}

/// A type that will be created by the first user written in the disposition
/// order.
struct TypePlan {
    global_id: GlobalId,
    class: IfcClass,
    name: String,
    psets: Vec<NewPropertySet>,
    /// The meshes that name this type, in mesh order. The one written first in
    /// the disposition order creates it (see `type_creators`).
    users: Vec<usize>,
}

/// A storey's GlobalId, keyed by definition id.
type Storeys = BTreeMap<String, GlobalId>;

/// Build the BIM-aware plan (storeys, types, properties, materials).
pub(crate) fn plan(
    database: &DrawingDatabase,
    meshes: &[CollectedMesh],
    policy: BimExportPolicy,
) -> CadResult<PlannedExport> {
    let (gids, skeleton_bytes) = skeleton_roots()?;
    let building = gids[2];
    let mut used: BTreeSet<u128> = gids.iter().map(|gid| gid.0).collect();
    let mut counts = Counts::default();
    let mut dispositions: Vec<(ObjectRef, Disposition)> = Vec::new();

    // ---- storeys -----------------------------------------------------------
    let mut levels: Vec<LevelRow> = database
        .bim_definitions()
        .iter()
        .filter_map(|row| match row {
            BimDefinition::Level {
                id,
                name,
                elevation,
                source,
                ..
            } => Some(LevelRow {
                id: id.clone(),
                name: name.clone(),
                elevation: *elevation,
                source_id: source.id.clone(),
                key: source_key(source),
            }),
            _ => None,
        })
        .collect();
    levels.sort_by(level_order);
    let mut storeys: Storeys = BTreeMap::new();
    // The same storeys keyed the way a relation endpoint names them.
    let mut storey_keys: BTreeMap<BimKey, GlobalId> = BTreeMap::new();
    for level in &levels {
        let global_id = definition_id(
            "yacr/bim/storey",
            &level.id,
            level.source_id.as_deref(),
            &mut used,
            &mut counts,
        )?;
        let mut object = NewObject::new(
            global_id,
            IfcClass::BuildingStorey,
            NewPlacement::new(IDENTITY),
        )
        .with_container(ObjectRef::Guid(building.encode()))
        .with_name(level.name.clone());
        if let Some(elevation) = level.elevation {
            object = object.with_elevation(elevation);
        }
        dispositions.push((
            ObjectRef::Guid(global_id.encode()),
            Disposition::Add(Box::new(object)),
        ));
        storeys.insert(level.id.clone(), global_id);
        if let Some(key) = level.key.clone() {
            storey_keys.entry(key).or_insert(global_id);
        }
    }

    // ---- types -------------------------------------------------------------
    let type_rows: BTreeMap<String, TypeRow> = database
        .bim_definitions()
        .iter()
        .filter_map(|row| match row {
            BimDefinition::Type {
                id,
                name,
                class,
                category,
                psets,
                source,
                evidence,
            } => Some((
                id.clone(),
                TypeRow {
                    name: name.clone(),
                    class: class.clone(),
                    category: category.clone(),
                    psets: psets.clone(),
                    evidence: *evidence,
                    source_id: source.id.clone(),
                },
            )),
            _ => None,
        })
        .collect();

    // The type each mesh uses, in mesh order, so the first occurrence can
    // create the type and the rest can name it.
    let mut type_users: Vec<(String, usize)> = Vec::new();
    for (index, mesh) in meshes.iter().enumerate() {
        if let Some(element) = database.bim_element(mesh.entity) {
            if let Some(type_id) = &element.element_type {
                type_users.push((type_id.clone(), index));
            }
        }
    }
    let mut type_plans: BTreeMap<String, TypePlan> = BTreeMap::new();
    for type_id in distinct(&type_users) {
        let Some(row) = type_rows.get(&type_id) else {
            counts.types += 1;
            continue;
        };
        let users: Vec<usize> = type_users
            .iter()
            .filter(|(id, _)| id == &type_id)
            .map(|(_, index)| *index)
            .collect();
        let Some(category) = type_category(database, row, &users, meshes) else {
            counts.types += 1;
            continue;
        };
        let Some(class) = writable_type_class(category, row.class.as_deref()) else {
            counts.types += 1;
            continue;
        };
        let global_id = definition_id(
            "yacr/bim/type",
            &type_id,
            row.source_id.as_deref(),
            &mut used,
            &mut counts,
        )?;
        let psets = convert_psets(&row.psets, row.evidence, policy, &mut counts);
        type_plans.insert(
            type_id,
            TypePlan {
                global_id,
                class: IfcClass::Named(class.to_string()),
                name: row.name.clone(),
                psets,
                users,
            },
        );
    }

    // ---- element identities and relation endpoints -------------------------
    //
    // Ids are allocated in mesh order before any object is built: the relation
    // pass below may name a host owned by a later mesh. The allocation sequence
    // (storeys, types, meshes) is the one this writer always used, so a
    // document with no relations keeps its exact bytes.
    let mut element_ids = Vec::with_capacity(meshes.len());
    let mut categories = Vec::with_capacity(meshes.len());
    let mut element_by_key: BTreeMap<BimKey, usize> = BTreeMap::new();
    let mut spatial: BTreeSet<usize> = BTreeSet::new();
    for (index, mesh) in meshes.iter().enumerate() {
        let element = database.bim_element(mesh.entity);
        let global_id = element_id(
            element.and_then(|bim| bim.source.id.as_deref()),
            mesh.object,
            &mut used,
            &mut counts,
        )?;
        let category = element
            .and_then(|bim| bim.category.as_deref())
            .and_then(|raw| match category_token(raw) {
                Some(category) => Some(category),
                None => {
                    counts.categories += 1;
                    None
                }
            });
        if category == Some("IfcSpace") {
            spatial.insert(index);
        }
        // A relation endpoint names a component by source key (`BimKey::Source`)
        // or by the row itself (`BimKey::Entity`); both spellings resolve, and
        // the first mesh in source order wins a duplicated source key.
        element_by_key
            .entry(BimKey::Entity(mesh.entity))
            .or_insert(index);
        if let Some(element) = element {
            element_by_key
                .entry(element.key(mesh.entity))
                .or_insert(index);
        }
        element_ids.push(global_id);
        categories.push(category);
    }

    let relations = plan_relations(
        database,
        &RelationInputs {
            elements: &element_by_key,
            element_ids: &element_ids,
            storeys: &storey_keys,
            building,
            spatial: &spatial,
            mesh_count: meshes.len(),
        },
        &mut counts,
    );

    // The disposition slot of each mesh, and the first mesh each type is
    // written by. A nesting order may differ from mesh order, so the type is
    // created by whichever user is written first; an `Existing` reference to a
    // type that is not written yet would be refused. Without reordering these
    // are the mesh-order values the writer always used.
    let mut position = vec![0usize; meshes.len()];
    for (slot, index) in relations.order.iter().enumerate() {
        position[*index] = slot;
    }
    let type_creators: BTreeMap<&str, usize> = type_plans
        .iter()
        .map(|(id, plan)| {
            let first = plan
                .users
                .iter()
                .copied()
                .min_by_key(|index| position[*index])
                .expect("a type plan has at least one user");
            (id.as_str(), first)
        })
        .collect();

    // ---- elements ----------------------------------------------------------
    let mut objects: Vec<Option<NewObject>> = Vec::with_capacity(meshes.len());
    for (index, mesh) in meshes.iter().enumerate() {
        let element = database.bim_element(mesh.entity);
        let global_id = element_ids[index];
        let category = categories[index];
        let class = category.map_or(IfcClass::BuildingElementProxy, |category| {
            IfcClass::Named(category.to_string())
        });

        // A level reference that does not resolve leaves the element in the
        // building: it is never silently dropped and never placed in a storey
        // that was not written. (A validated database cannot hold a dangling
        // reference; the branch is defensive and unit-tested directly.)
        //
        // A `contains` relation the document declares overrides that default:
        // the source's own spatial containment is the authority, and the plan
        // API takes exactly one container per object.
        let level = element.and_then(|bim| bim.level.as_deref());
        let container = relations
            .contains
            .get(&index)
            .cloned()
            .unwrap_or_else(|| storey_container(level, &storeys, &building, &mut counts));

        let mut object = NewObject::new(global_id, class, NewPlacement::new(IDENTITY))
            .with_container(container)
            .with_representation(Representation::Mesh(TriangleMesh::new(
                mesh.positions.clone(),
                mesh.triangles.clone(),
            )));

        // A `nests` relation is the object's one nesting host; `order` was
        // arranged so the host is added before its children.
        if let Some(&host) = relations.nests.get(&index) {
            object = object.with_nest_in(ObjectRef::Guid(element_ids[host].encode()));
        }

        if let Some(type_id) = element.and_then(|bim| bim.element_type.as_deref()) {
            if let Some(type_plan) = type_plans.get(type_id) {
                let link = if type_creators.get(type_id) == Some(&index) {
                    TypeLink::New(
                        NewType::new(type_plan.class.clone())
                            .with_global_id(type_plan.global_id)
                            .with_name(type_plan.name.clone())
                            .with_psets(type_plan.psets.clone()),
                    )
                } else {
                    TypeLink::Existing(ObjectRef::Guid(type_plan.global_id.encode()))
                };
                object = object.with_type(link);
            }
        }

        if let Some(element) = element {
            match element.material.as_deref().map(|material_id| {
                match database.bim_definition(material_id) {
                    Some(BimDefinition::Material { name, .. }) => Some(name.as_str()),
                    _ => None,
                }
            }) {
                Some(Some(name)) if category != Some("IfcSpace") => {
                    object = object.with_material(MaterialLink::New(NewMaterial::new(name)));
                }
                Some(_) => counts.materials += 1,
                None => {}
            }
            let psets = convert_psets(&element.psets, element.evidence, policy, &mut counts);
            if !psets.is_empty() {
                object = object.with_psets(psets);
            }
            if let Some(name) = element
                .name
                .as_deref()
                .filter(|name| !name.trim().is_empty())
            {
                object = object.with_name(name);
            } else if !mesh.type_key.trim().is_empty() {
                object = object.with_name(mesh.type_key.clone());
            } else {
                object = object.with_tag(format!("object:{}", mesh.object.0));
            }
            if let Some(native_id) = element
                .source
                .native_id
                .as_deref()
                .filter(|native_id| !native_id.trim().is_empty())
            {
                object = object.with_tag(native_id);
            }
        } else if !mesh.type_key.trim().is_empty() {
            object = object.with_name(mesh.type_key.clone());
        } else {
            object = object.with_tag(format!("object:{}", mesh.object.0));
        }

        objects.push(Some(object));
    }

    // Adds are validated in this order: a `nests` child must follow its host,
    // and `order` is the source order unless a nesting edge forces another
    // (deterministic in both cases). Ids stay in mesh order above.
    for index in relations.order.iter().copied() {
        let object = objects[index].take().ok_or_else(|| {
            CadError::Invariant(format!(
                "IFC export relation order visits mesh #{index} more than once"
            ))
        })?;
        dispositions.push((
            ObjectRef::Guid(element_ids[index].encode()),
            Disposition::Add(Box::new(object)),
        ));
    }

    let plan =
        ExportPlan::new(dispositions).with_context(PlanContext::new().with_file_name("export.ifc"));
    Ok(PlannedExport {
        skeleton_bytes,
        plan,
        element_ids,
        bim_entries: counts.entries(),
        bim: true,
    })
}

/// The container an element belongs to: its storey when the reference
/// resolves, else the building.
///
/// A reference the definition table does not hold is counted
/// ([`STOREY_NOT_WRITTEN`]) and never fabricates a storey: the element stays
/// in the building, exactly where a writer without BIM would have put it.
fn storey_container(
    level: Option<&str>,
    storeys: &Storeys,
    building: &GlobalId,
    counts: &mut Counts,
) -> ObjectRef {
    match level.and_then(|level| storeys.get(level)) {
        Some(storey) => ObjectRef::Guid(storey.encode()),
        None => {
            if level.is_some() {
                counts.storeys += 1;
            }
            ObjectRef::Guid(building.encode())
        }
    }
}

/// The source `(connector, document_id, id)` key of a definition, when the
/// source declared both halves (the shape a relation endpoint uses).
fn source_key(source: &BimSource) -> Option<BimKey> {
    match (&source.document_id, &source.id) {
        (Some(document_id), Some(id))
            if !document_id.trim().is_empty() && !id.trim().is_empty() =>
        {
            Some(BimKey::Source {
                connector: source.connector.clone(),
                document_id: document_id.clone(),
                id: id.clone(),
            })
        }
        _ => None,
    }
}

/// Level order: `(elevation, name, id)`, with a level that declared no
/// elevation before the ones that did (there is no invented value).
fn level_order(left: &LevelRow, right: &LevelRow) -> Ordering {
    match (left.elevation, right.elevation) {
        (Some(a), Some(b)) => a.total_cmp(&b),
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
    .then_with(|| left.name.cmp(&right.name))
    .then_with(|| left.id.cmp(&right.id))
}

/// The distinct type ids, in first-use order.
fn distinct(users: &[(String, usize)]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    users
        .iter()
        .filter(|(id, _)| seen.insert(id.clone()))
        .map(|(id, _)| id.clone())
        .collect()
}

/// The one writable category all users of a type agree on.
fn type_category(
    database: &DrawingDatabase,
    row: &TypeRow,
    users: &[usize],
    meshes: &[CollectedMesh],
) -> Option<&'static str> {
    let declared = match row.category.as_deref() {
        Some(raw) => Some(category_token(raw)?),
        None => None,
    };
    let mut agreed: Option<&'static str> = None;
    for index in users {
        let category = database
            .bim_element(meshes[*index].entity)
            .and_then(|element| element.category.as_deref())
            .and_then(category_token)?;
        match agreed {
            Some(previous) if previous != category => return None,
            Some(_) => {}
            None => agreed = Some(category),
        }
    }
    let users_category = agreed?;
    match declared {
        Some(declared) if declared != users_category => None,
        _ => Some(users_category),
    }
}

/// A definition's GlobalId: the source's own when it is a valid, unused id;
/// otherwise a deterministic id derived from the definition id.
fn definition_id(
    role: &str,
    definition_id: &str,
    source_id: Option<&str>,
    used: &mut BTreeSet<u128>,
    counts: &mut Counts,
) -> CadResult<GlobalId> {
    if let Some(token) = source_id {
        match decode_global_id(token) {
            Some(bits) if used.insert(bits) => return Ok(GlobalId(bits)),
            Some(_) => counts.global_id_collisions += 1,
            None => counts.global_id_invalid += 1,
        }
    }
    let global_id = stable_global_id(role, hash_text(definition_id));
    if !used.insert(global_id.0) {
        return Err(CadError::Invariant(format!(
            "IFC export GlobalId collision for BIM definition {definition_id:?}"
        )));
    }
    Ok(global_id)
}

/// An element's GlobalId: the source's when valid and unused, else the
/// deterministic mesh id.
fn element_id(
    source_id: Option<&str>,
    object: ObjectId,
    used: &mut BTreeSet<u128>,
    counts: &mut Counts,
) -> CadResult<GlobalId> {
    if let Some(token) = source_id {
        match decode_global_id(token) {
            Some(bits) if used.insert(bits) => return Ok(GlobalId(bits)),
            Some(_) => counts.global_id_collisions += 1,
            None => counts.global_id_invalid += 1,
        }
    }
    let global_id = mesh_global_id(object);
    if !used.insert(global_id.0) {
        return Err(CadError::Invariant(format!(
            "IFC export GlobalId collision for object #{}",
            object.0
        )));
    }
    Ok(global_id)
}

#[cfg(test)]
#[path = "tests_bim.rs"]
mod tests;

#[cfg(test)]
#[path = "tests_relations.rs"]
mod relation_tests;
