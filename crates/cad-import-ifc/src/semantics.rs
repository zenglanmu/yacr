//! Second-pass IFC BIM semantics scan (ADR-0042 D1/D3/D5/D7).
//!
//! The geometry path is unchanged: [`crate::IfcImporter`] still schedules one
//! job per geometry-bearing product and meshes it with `ifc-lite-geometry`. This
//! module runs **after** that schedule exists and reads only what the geometry
//! path ignores — `IfcRoot` identity, the spatial/type/material relations and the
//! property sets — into [`cad_db::BimElement`] and
//! [`cad_db::BimDefinitions`].
//!
//! # What is read
//!
//! | IFC records | Stored as |
//! |---|---|
//! | `IfcRoot.GlobalId` / `Name`, `IfcElement.Tag` | `bim.source.id` / `bim.name` / `bim.source.native_id` |
//! | `IfcBuildingStorey` + `IfcRelAggregates` / `IfcRelContainedInSpatialStructure` | `level` definition + `bim.level` |
//! | `IfcTypeObject` family + `IfcRelDefinesByType` | `type` definition + `bim.element_type` |
//! | `IfcPropertySet` + `IfcRelDefinesByProperties` + `IfcPropertySingleValue` | `bim.psets` / type `psets` |
//! | `IfcElementQuantity` + `IfcRelDefinesByProperties` | `bim.psets` under the source's own `Qto_*` name |
//! | `IfcMaterial` + `IfcRelAssociatesMaterial` | `material` definition + `bim.material` |
//! | `IfcRelAggregates` / `IfcRelNests` / `IfcRelContainedInSpatialStructure` / `IfcRelAssignsToGroup` / `IfcRelConnectsElements` | `tables/bim-relations.json` via [`cad_db::BimRelations`] |
//!
//! # What is deliberately **not** read
//!
//! Complex, enumerated, bounded and table property values have no
//! [`cad_db::BimValue`] form (ADR-0042 D2). They are counted under stable
//! diagnostics and left out; nothing is flattened into a string that looks like
//! the source's value. The same holds for an aggregate
//! `IfcPhysicalComplexQuantity`: it is reported, not flattened into a number.
//! `evidence` is always [`cad_db::BimEvidence::Declared`]: every stored field is
//! spelled in the file. There is no heuristic path here.
//!
//! # Budgets
//!
//! Every `cad_db::BIM_*` budget is a **drop with a report**, never a silent
//! truncation and never a dangling reference: a definition that does not fit is
//! not stored, and every element field that would have pointed at it is left
//! `None`. The shared table's encoded-size ceiling is enforced by
//! `cad_db` at `DrawingDatabaseBuilder::finish` (a refusal, not a truncation).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use cad_db::{
    BimDefinition, BimDefinitions, BimElement, BimEvidence, BimRelations, BimSource,
    BIM_MAX_DEFINITIONS, BIM_MAX_STRING_BYTES,
};
use cad_domain::Diagnostic;
use cad_import_core::MeshImport;
use ifc_lite_core::{slot, DecodedEntity, EntityDecoder, EntityScanner, IfcType, ProjectUnits};

use crate::{
    DIAG_BIM_BUDGET_EXCEEDED, DIAG_BIM_ELEVATION_UNRESOLVED, DIAG_BIM_MATERIAL_UNRESOLVED,
    DIAG_BIM_PROPERTY_SET_UNSUPPORTED, DIAG_BIM_PROPERTY_VALUE_UNSUPPORTED,
    DIAG_BIM_QUANTITY_SET_NOT_STORED, DIAG_BIM_QUANTITY_SET_STORED,
    DIAG_BIM_RELATION_BUDGET_EXCEEDED, DIAG_BIM_RELATION_ENDPOINT_MISSING,
    DIAG_BIM_SOURCE_ID_MISSING, DIAG_BIM_STOREY_UNRESOLVED, DIAG_BIM_TYPE_UNRESOLVED,
};

mod property;
mod quantity;
mod relations;
use property::{collect_element_psets, collect_psets};

/// The connector id every IFC BIM source row carries.
pub const CONNECTOR: &str = "ifc";

/// Deepest spatial/aggregate walk before a storey is declared unresolved.
///
/// The value is ADR-0042 D7's "最多向上 16 层": at most sixteen parent edges. A
/// cyclic or unreasonably deep chain is reported, never followed forever.
const MAX_STOREY_WALK_DEPTH: usize = 16;

/// Why a relations scan ended without a storey (for the diagnostic count).
#[derive(Default)]
struct Stats {
    source_id_missing: usize,
    storey_unresolved: usize,
    elevation_unresolved: usize,
    type_unresolved: usize,
    material_unresolved: usize,
    property_set_unsupported: usize,
    property_value_unsupported: usize,
    /// `IfcElementQuantity` sets stored as property sets (lossless count).
    quantity_sets_stored: usize,
    /// Quantity sets/entries that could not be read and were left out.
    quantity_not_stored: usize,
    /// Structural edges skipped because an endpoint had no readable GlobalId.
    relation_endpoint_missing: usize,
    /// The structural relation table was refused at `BIM_MAX_RELATIONS`.
    relation_budget_exceeded: bool,
    budget_exceeded: usize,
}

/// The relation edges one scan pass collects.
///
/// Every map is keyed by IFC step id. A malformed file can name the same child
/// from several parents or point a cycle; the readers below bound their walks
/// and dedupe with a visited set, so the scan never recurses on file input.
#[derive(Default)]
struct Relations {
    /// `IfcRelAggregates`, inverted for an upward walk: child -> parent
    /// object definitions.
    ancestors: BTreeMap<u32, Vec<u32>>,
    /// `IfcRelContainedInSpatialStructure`: element -> relating structure.
    containment: BTreeMap<u32, u32>,
    /// `IfcRelDefinesByType`: occurrence -> relating type object.
    types: BTreeMap<u32, u32>,
    /// `IfcRelDefinesByProperties`: object (occurrence or type) -> property definitions.
    properties: BTreeMap<u32, Vec<u32>>,
    /// `IfcRelAssociatesMaterial`: object -> relating material definition.
    materials: BTreeMap<u32, u32>,
}

/// The BIM result of one IFC import.
pub(crate) struct IfcSemantics {
    definitions: BimDefinitions,
    /// Structural edges (`tables/bim-relations.json`), empty when the file
    /// declares none or the table was refused at its row budget.
    relations: BimRelations,
    /// One logical BIM block per geometry-bearing IFC product, keyed by step id.
    elements: BTreeMap<u32, BimElement>,
    stats: Stats,
    /// Whether any relation-level data (level/type/material/pset) was stored.
    ///
    /// A product that only gains `source` + `category` does not set this: those
    /// fields are the entity's own class and root id, not a relation reading, and
    /// a geometry-only IFC keeps its existing completeness verdict.
    relations_stored: bool,
}

impl IfcSemantics {
    /// Read the file's BIM relations for `products` (geometry-bearing step ids).
    ///
    /// `length_scale` is metres per file length unit when the project declared a
    /// usable one; `None` leaves storey elevations unconverted and reported,
    /// exactly like the geometry path's honest drawing-unit behaviour.
    pub(crate) fn scan(
        content: &[u8],
        decoder: &mut EntityDecoder<'_>,
        products: &[u32],
        length_scale: Option<f64>,
    ) -> IfcSemantics {
        let relations = scan_relations(content, decoder);
        let units = decoder
            .project_id()
            .map(|project| ProjectUnits::resolve(decoder, project))
            .unwrap_or_default();

        let mut semantics = IfcSemantics {
            definitions: BimDefinitions::new(),
            relations: BimRelations::new(),
            elements: BTreeMap::new(),
            stats: Stats::default(),
            relations_stored: false,
        };
        let document_id = match project_global_id(decoder) {
            Some(id) if id.len() <= BIM_MAX_STRING_BYTES => Some(id),
            Some(_) => {
                // An over-budget document id would make every definition id
                // invalid; count it and fall back to the `-` key component.
                semantics.stats.budget_exceeded += 1;
                None
            }
            None => None,
        };
        // The structural edges are a separate bounded table; they reuse the
        // project GlobalId as the endpoint document id. A refused table stores
        // nothing and reports its own loss rather than truncating.
        let relation_scan = relations::collect(content, decoder, document_id.as_deref());
        semantics.relations = relation_scan.relations;
        semantics.stats.relation_endpoint_missing = relation_scan.endpoint_missing;
        semantics.stats.relation_budget_exceeded = relation_scan.budget_exceeded;
        for &product_id in products {
            let Ok(product) = decoder.decode_by_id(product_id) else {
                continue;
            };
            let Some(element) = build_element(
                &product,
                &relations,
                &document_id,
                length_scale,
                decoder,
                &units,
                &mut semantics,
            ) else {
                continue;
            };
            semantics.elements.insert(product_id, element);
        }
        semantics
    }

    /// Take the definition table for the builder (which owns it at `finish`).
    pub(crate) fn take_definitions(&mut self) -> BimDefinitions {
        std::mem::take(&mut self.definitions)
    }

    /// Take the structural relation table for the builder (which owns it at
    /// `finish`, where the encoded-size ceiling is enforced).
    pub(crate) fn take_relations(&mut self) -> BimRelations {
        std::mem::take(&mut self.relations)
    }

    /// The BIM block for one geometry-bearing product, when it has one.
    pub(crate) fn element(&self, product_id: u32) -> Option<&BimElement> {
        self.elements.get(&product_id)
    }

    /// Write every reading loss onto the import's own report channel.
    ///
    /// Counts are aggregated per stable code (the way the geometry lane reports
    /// its drops) so a hostile file cannot turn one import into thousands of
    /// diagnostics. Every non-zero count marks the import `Partial`.
    pub(crate) fn record(&self, import: &mut MeshImport) {
        let stats = &self.stats;
        if stats.source_id_missing > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_SOURCE_ID_MISSING.into(),
                    message: format!(
                        "{} IFC product(s) carry no IfcRoot.GlobalId; their BIM source id is \
                         empty and they can only be grouped by their own EntityId",
                        stats.source_id_missing
                    ),
                },
                Some(format!(
                    "{} IFC product(s) had no GlobalId for their BIM source",
                    stats.source_id_missing
                )),
            );
        }
        if stats.storey_unresolved > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_STOREY_UNRESOLVED.into(),
                    message: format!(
                        "{} IFC product(s) name a spatial/aggregate parent chain with no usable \
                         IfcBuildingStorey (undecodable, unnamed, cyclic or deeper than \
                         {MAX_STOREY_WALK_DEPTH} levels); their level is left empty",
                        stats.storey_unresolved
                    ),
                },
                Some(format!(
                    "{} IFC product(s) had a spatial relation with no resolvable storey",
                    stats.storey_unresolved
                )),
            );
        }
        if stats.elevation_unresolved > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_ELEVATION_UNRESOLVED.into(),
                    message: format!(
                        "{} IfcBuildingStorey elevation(s) could not be converted to metres \
                         because the project length unit did not resolve; the level is stored \
                         without an elevation",
                        stats.elevation_unresolved
                    ),
                },
                Some(format!(
                    "{} storey elevation(s) stayed unconverted: no usable length unit",
                    stats.elevation_unresolved
                )),
            );
        }
        if stats.type_unresolved > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_TYPE_UNRESOLVED.into(),
                    message: format!(
                        "{} IfcRelDefinesByType link(s) name a type object that is missing, not \
                         an IfcTypeObject, or has no name; the element type is left empty",
                        stats.type_unresolved
                    ),
                },
                Some(format!(
                    "{} IFC type relation(s) could not be resolved",
                    stats.type_unresolved
                )),
            );
        }
        if stats.material_unresolved > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_MATERIAL_UNRESOLVED.into(),
                    message: format!(
                        "{} IfcRelAssociatesMaterial link(s) name a material that is missing, not \
                         a direct IfcMaterial, or has no name; the element material is left empty",
                        stats.material_unresolved
                    ),
                },
                Some(format!(
                    "{} IFC material relation(s) could not be resolved",
                    stats.material_unresolved
                )),
            );
        }
        if stats.property_set_unsupported > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_PROPERTY_SET_UNSUPPORTED.into(),
                    message: format!(
                        "{} IFC property definition(s) are not an IfcPropertySet (or have no \
                         usable name) and were not stored",
                        stats.property_set_unsupported
                    ),
                },
                Some(format!(
                    "{} IFC property set(s) were not representable",
                    stats.property_set_unsupported
                )),
            );
        }
        if stats.property_value_unsupported > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_PROPERTY_VALUE_UNSUPPORTED.into(),
                    message: format!(
                        "{} IFC property value(s) are complex, enumerated, bounded or otherwise \
                         not representable by the BIM value model; they were counted and left \
                         out, not flattened into strings",
                        stats.property_value_unsupported
                    ),
                },
                Some(format!(
                    "{} IFC property value(s) had no BIM value representation",
                    stats.property_value_unsupported
                )),
            );
        }
        if stats.quantity_sets_stored > 0 {
            // Lossless: the quantities are in the document. This says what was
            // read (so a consumer knows `Qto_*` rows came from the file), and
            // deliberately does not mark the import `Partial` on its own.
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_QUANTITY_SET_STORED.into(),
                    message: format!(
                        "{} IfcElementQuantity set(s) were stored as property sets under their \
                         own source name, one property per quantity",
                        stats.quantity_sets_stored
                    ),
                },
                None,
            );
        }
        if stats.quantity_not_stored > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_QUANTITY_SET_NOT_STORED.into(),
                    message: format!(
                        "{} IFC quantity set(s)/quantit(ies) could not be read (unknown physical \
                         quantity entity, missing name, or an invalid/non-finite value) and were \
                         left out; nothing was invented in their place",
                        stats.quantity_not_stored
                    ),
                },
                Some(format!(
                    "{} IFC quantity set(s)/quantit(ies) could not be read",
                    stats.quantity_not_stored
                )),
            );
        }
        if stats.relation_endpoint_missing > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_RELATION_ENDPOINT_MISSING.into(),
                    message: format!(
                        "{} structural relation edge(s) name a parent or child without a \
                         readable IfcRoot.GlobalId; the edge was not stored rather than keyed \
                         by a file-local step id",
                        stats.relation_endpoint_missing
                    ),
                },
                Some(format!(
                    "{} IFC structural relation edge(s) had no GlobalId endpoint",
                    stats.relation_endpoint_missing
                )),
            );
        }
        if stats.relation_budget_exceeded {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_RELATION_BUDGET_EXCEEDED.into(),
                    message: format!(
                        "the file declares more than {} structural relations; the relation table \
                         was refused rather than truncated, so no partial edge set is stored",
                        cad_db::BIM_MAX_RELATIONS
                    ),
                },
                Some(format!(
                    "IFC structural relations exceeded the {} row budget and were not stored",
                    cad_db::BIM_MAX_RELATIONS
                )),
            );
        }
        if stats.budget_exceeded > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_BUDGET_EXCEEDED.into(),
                    message: format!(
                        "{} BIM row(s)/property(ies) exceeded a cad_db BIM_* budget and were \
                         dropped; references to a dropped definition were cleared, so the \
                         document holds no dangling level/type/material reference",
                        stats.budget_exceeded
                    ),
                },
                Some(format!(
                    "{} BIM row(s) exceeded a cad-db budget and were dropped",
                    stats.budget_exceeded
                )),
            );
        }
        if self.relations_stored {
            // The values are read from the file (Declared), but the mapping itself
            // is a code contract: no vendor IFC tool has cross-checked this import.
            import.note_partial(
                "IFC BIM semantics are read from the file's own relations and stored as \
                 contract evidence; no vendor tool cross-validation exists",
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Relation scan.
// ---------------------------------------------------------------------------

fn scan_relations(content: &[u8], decoder: &mut EntityDecoder<'_>) -> Relations {
    let mut relations = Relations::default();
    let mut scanner = EntityScanner::new(content);
    while let Some((id, type_name, start, end)) = scanner.next_entity() {
        if !is_wanted_relation(type_name) {
            continue;
        }
        let Ok(entity) = decoder.decode_at_with_id(id, start, end) else {
            continue;
        };
        match entity.ifc_type.as_str() {
            "IFCRELAGGREGATES" => {
                let parent = entity.get_ref(slot::IfcRelAggregates::RELATING_OBJECT.index());
                let children = entity.get_refs(slot::IfcRelAggregates::RELATED_OBJECTS.index());
                if let (Some(parent), Some(children)) = (parent, children) {
                    // Store child -> parent: the level walk goes up from the
                    // product (or its containing space) to a storey.
                    for child in children {
                        relations.ancestors.entry(child).or_default().push(parent);
                    }
                }
            }
            "IFCRELCONTAINEDINSPATIALSTRUCTURE" => {
                let structure = entity
                    .get_ref(slot::IfcRelContainedInSpatialStructure::RELATING_STRUCTURE.index());
                let elements = entity
                    .get_refs(slot::IfcRelContainedInSpatialStructure::RELATED_ELEMENTS.index());
                if let (Some(structure), Some(elements)) = (structure, elements) {
                    for element in elements {
                        // The first containment wins: a second relation for one
                        // element is malformed, and picking the file's first is
                        // deterministic (never a silent merge of two levels).
                        relations.containment.entry(element).or_insert(structure);
                        // The upward walk also needs this edge: a wall contained
                        // in an IfcSpace must still reach the space's own storey.
                        relations
                            .ancestors
                            .entry(element)
                            .or_default()
                            .push(structure);
                    }
                }
            }
            "IFCRELDEFINESBYTYPE" => {
                let relating = entity.get_ref(slot::IfcRelDefinesByType::RELATING_TYPE.index());
                let objects = entity.get_refs(slot::IfcRelDefinesByType::RELATED_OBJECTS.index());
                if let (Some(relating), Some(objects)) = (relating, objects) {
                    for object in objects {
                        relations.types.entry(object).or_insert(relating);
                    }
                }
            }
            "IFCRELDEFINESBYPROPERTIES" => {
                let definitions = entity.get_refs(
                    slot::IfcRelDefinesByProperties::RELATING_PROPERTY_DEFINITION.index(),
                );
                let objects =
                    entity.get_refs(slot::IfcRelDefinesByProperties::RELATED_OBJECTS.index());
                if let (Some(definitions), Some(objects)) = (definitions, objects) {
                    for object in objects {
                        relations
                            .properties
                            .entry(object)
                            .or_default()
                            .extend(definitions.iter().copied());
                    }
                }
            }
            "IFCRELASSOCIATESMATERIAL" => {
                let material =
                    entity.get_ref(slot::IfcRelAssociatesMaterial::RELATING_MATERIAL.index());
                let objects =
                    entity.get_refs(slot::IfcRelAssociatesMaterial::RELATED_OBJECTS.index());
                if let (Some(material), Some(objects)) = (material, objects) {
                    for object in objects {
                        relations.materials.entry(object).or_insert(material);
                    }
                }
            }
            _ => {}
        }
    }
    relations
}

/// The one cheap gate before decoding: only these keywords carry BIM relations.
fn is_wanted_relation(type_name: &str) -> bool {
    type_name.eq_ignore_ascii_case("IFCRELAGGREGATES")
        || type_name.eq_ignore_ascii_case("IFCRELCONTAINEDINSPATIALSTRUCTURE")
        || type_name.eq_ignore_ascii_case("IFCRELDEFINESBYTYPE")
        || type_name.eq_ignore_ascii_case("IFCRELDEFINESBYPROPERTIES")
        || type_name.eq_ignore_ascii_case("IFCRELASSOCIATESMATERIAL")
}

/// The `IfcProject.GlobalId`, when the file has a readable project.
fn project_global_id(decoder: &mut EntityDecoder<'_>) -> Option<String> {
    let project = decoder.project_id()?;
    let entity = decoder.decode_by_id(project).ok()?;
    non_empty(entity.get_string(slot::IfcProject::GLOBAL_ID.index())).map(str::to_string)
}

// ---------------------------------------------------------------------------
// Per-product element.
// ---------------------------------------------------------------------------

fn build_element(
    product: &DecodedEntity,
    relations: &Relations,
    document_id: &Option<String>,
    length_scale: Option<f64>,
    decoder: &mut EntityDecoder<'_>,
    units: &ProjectUnits,
    semantics: &mut IfcSemantics,
) -> Option<BimElement> {
    // `IfcRoot.GlobalId` / `Name` are the same slots on every root subtype (0/2).
    let global_id = bounded_text(
        product.get_string(slot::IfcRoot::GLOBAL_ID.index()),
        &mut semantics.stats,
    );
    if global_id.is_none() {
        semantics.stats.source_id_missing += 1;
    }
    let name = bounded_text(
        product.get_string(slot::IfcRoot::NAME.index()),
        &mut semantics.stats,
    );
    // Only an IfcElement carries Tag at attribute 7. An IfcSpatialElement's slot 7
    // is LongName, so reading it by number would invent a native id.
    let native_id = if product.ifc_type.is_subtype_of(IfcType::IfcElement) {
        bounded_text(
            product.get_string(slot::IfcElement::TAG.index()),
            &mut semantics.stats,
        )
    } else {
        None
    };

    let category = product.ifc_type.name().to_string();
    let level = resolve_level(
        product.id,
        relations,
        decoder,
        length_scale,
        document_id,
        &mut semantics.definitions,
        &mut semantics.stats,
    );
    let element_type = resolve_type(
        product,
        &category,
        relations,
        decoder,
        document_id,
        units,
        &mut semantics.definitions,
        &mut semantics.stats,
    );
    let material = resolve_material(
        product.id,
        relations,
        decoder,
        document_id,
        &mut semantics.definitions,
        &mut semantics.stats,
    );
    let psets = collect_element_psets(product.id, relations, decoder, units, &mut semantics.stats);

    if level.is_some() || element_type.is_some() || material.is_some() || !psets.is_empty() {
        semantics.relations_stored = true;
    }

    Some(BimElement {
        category: Some(category),
        name,
        source: BimSource {
            connector: CONNECTOR.to_string(),
            document_id: document_id.clone(),
            id: global_id,
            native_id,
        },
        level,
        element_type,
        material,
        psets,
        evidence: BimEvidence::Declared,
    })
}

// ---------------------------------------------------------------------------
// Levels (IfcBuildingStorey).
// ---------------------------------------------------------------------------

fn resolve_level(
    product_id: u32,
    relations: &Relations,
    decoder: &mut EntityDecoder<'_>,
    length_scale: Option<f64>,
    document_id: &Option<String>,
    definitions: &mut BimDefinitions,
    stats: &mut Stats,
) -> Option<String> {
    // A product reaches its storey either through containment or, in an
    // aggregate-authored model, through `IfcRelAggregates` parents.
    let mut starts = Vec::new();
    let mut related = false;
    if let Some(structure) = relations.containment.get(&product_id) {
        related = true;
        starts.push((*structure, 0usize));
    }
    if relations.ancestors.contains_key(&product_id) {
        related = true;
        starts.push((product_id, 0usize));
    }
    if !related {
        // No spatial or aggregate relation was declared at all: there is nothing
        // to resolve, and no loss to report.
        return None;
    }

    let mut queue: VecDeque<(u32, usize)> = starts.into();
    let mut visited = BTreeSet::new();
    while let Some((node, depth)) = queue.pop_front() {
        if !visited.insert(node) {
            // A cycle: the node was already walked on this product's chain.
            continue;
        }
        match decoder.decode_by_id(node) {
            Ok(entity) if entity.ifc_type == IfcType::IfcBuildingStorey => {
                return storey_definition(&entity, length_scale, document_id, definitions, stats);
            }
            Ok(_) => {}
            // The relation named a record that does not decode: the chain is
            // broken, which is a reported loss rather than a silent `None`.
            Err(_) => continue,
        }
        if let Some(parents) = relations.ancestors.get(&node) {
            for parent in parents {
                // "最多向上 16 层": a chain that would need a seventeenth edge is
                // not followed. If it held the storey, the product is reported
                // unresolved rather than walked forever.
                if depth < MAX_STOREY_WALK_DEPTH {
                    queue.push_back((*parent, depth + 1));
                }
            }
        }
    }
    stats.storey_unresolved += 1;
    None
}

fn storey_definition(
    storey: &DecodedEntity,
    length_scale: Option<f64>,
    document_id: &Option<String>,
    definitions: &mut BimDefinitions,
    stats: &mut Stats,
) -> Option<String> {
    let name = bounded_text(storey.get_string(slot::IfcRoot::NAME.index()), stats);
    let Some(name) = name else {
        stats.storey_unresolved += 1;
        return None;
    };
    let global_id = bounded_text(storey.get_string(slot::IfcRoot::GLOBAL_ID.index()), stats);
    let key = source_key(global_id.as_deref(), storey.id);
    let id = definition_id("level", document_id.as_deref(), &key);
    if definitions.get(&id).is_some() {
        return Some(id);
    }
    let elevation = match storey.get_float(slot::IfcBuildingStorey::ELEVATION.index()) {
        Some(raw) if !raw.is_finite() => {
            // `cad_db` refuses a non-finite elevation as invalid; drop the
            // elevation, not the whole import.
            stats.elevation_unresolved += 1;
            None
        }
        Some(raw) => match length_scale {
            Some(scale) => Some(raw * scale),
            None => {
                // No usable length unit: the file's raw number is not metres, so
                // it is not written as an elevation.
                stats.elevation_unresolved += 1;
                None
            }
        },
        None => None,
    };
    let row = BimDefinition::Level {
        id: id.clone(),
        name,
        elevation,
        source: definition_source(document_id, &key),
        evidence: BimEvidence::Declared,
    };
    insert_definition(definitions, stats, row).then_some(id)
}

// ---------------------------------------------------------------------------
// Types (IfcTypeObject family).
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn resolve_type(
    occurrence: &DecodedEntity,
    occurrence_category: &str,
    relations: &Relations,
    decoder: &mut EntityDecoder<'_>,
    document_id: &Option<String>,
    units: &ProjectUnits,
    definitions: &mut BimDefinitions,
    stats: &mut Stats,
) -> Option<String> {
    let type_id = relations.types.get(&occurrence.id)?;
    let Ok(entity) = decoder.decode_by_id(*type_id) else {
        stats.type_unresolved += 1;
        return None;
    };
    if !entity.ifc_type.is_subtype_of(IfcType::IfcTypeObject) {
        stats.type_unresolved += 1;
        return None;
    }
    let Some(name) = bounded_text(entity.get_string(slot::IfcTypeObject::NAME.index()), stats)
    else {
        stats.type_unresolved += 1;
        return None;
    };
    let global_id = bounded_text(entity.get_string(slot::IfcRoot::GLOBAL_ID.index()), stats);
    let key = source_key(global_id.as_deref(), entity.id);
    let id = definition_id("type", document_id.as_deref(), &key);
    if definitions.get(&id).is_some() {
        return Some(id);
    }

    // Type property sets are declared both by `IfcTypeObject.HasPropertySets` and
    // by an `IfcRelDefinesByProperties` whose related object is the type.
    let mut pset_refs = entity
        .get_refs(slot::IfcTypeObject::HAS_PROPERTY_SETS.index())
        .unwrap_or_default();
    if let Some(defined) = relations.properties.get(&entity.id) {
        pset_refs.extend(defined.iter().copied());
    }
    let psets = collect_psets(pset_refs, decoder, units, stats);

    let row = BimDefinition::Type {
        id: id.clone(),
        name,
        class: Some(entity.ifc_type.name().to_string()),
        category: Some(occurrence_category.to_string()),
        psets,
        source: definition_source(document_id, &key),
        evidence: BimEvidence::Declared,
    };
    insert_definition(definitions, stats, row).then_some(id)
}

// ---------------------------------------------------------------------------
// Materials.
// ---------------------------------------------------------------------------

fn resolve_material(
    product_id: u32,
    relations: &Relations,
    decoder: &mut EntityDecoder<'_>,
    document_id: &Option<String>,
    definitions: &mut BimDefinitions,
    stats: &mut Stats,
) -> Option<String> {
    let material_id = relations.materials.get(&product_id)?;
    let Ok(entity) = decoder.decode_by_id(*material_id) else {
        stats.material_unresolved += 1;
        return None;
    };
    // Only a direct `IfcMaterial` is read. A layer set, profile set or usage is a
    // different shape; picking one layer out of it would be a guess (D1).
    if entity.ifc_type != IfcType::IfcMaterial {
        stats.material_unresolved += 1;
        return None;
    }
    let Some(name) = bounded_text(entity.get_string(slot::IfcMaterial::NAME.index()), stats) else {
        stats.material_unresolved += 1;
        return None;
    };
    // `IfcMaterial` is not an `IfcRoot` and has no GlobalId; its step id is the
    // file-local identity the reader can reproduce.
    let key = format!("#{}", entity.id);
    let id = definition_id("material", document_id.as_deref(), &key);
    if definitions.get(&id).is_some() {
        return Some(id);
    }
    let row = BimDefinition::Material {
        id: id.clone(),
        name,
        source: BimSource {
            connector: CONNECTOR.to_string(),
            document_id: document_id.clone(),
            id: Some(key),
            native_id: None,
        },
        evidence: BimEvidence::Declared,
    };
    insert_definition(definitions, stats, row).then_some(id)
}

// ---------------------------------------------------------------------------
// Sharing helpers.
// ---------------------------------------------------------------------------

fn definition_id(kind: &str, document_id: Option<&str>, key: &str) -> String {
    // The document token is `-` when the file has no readable project GlobalId;
    // it is a key component, not a claim about the document's name.
    format!("{kind}:{CONNECTOR}:{}:{key}", document_id.unwrap_or("-"))
}

fn source_key(global_id: Option<&str>, step_id: u32) -> String {
    match global_id {
        Some(id) => id.to_string(),
        // A non-root definition still has a file-local identity; the step id is
        // written in the file, not guessed.
        None => format!("#{step_id}"),
    }
}

fn definition_source(document_id: &Option<String>, key: &str) -> BimSource {
    BimSource {
        connector: CONNECTOR.to_string(),
        document_id: document_id.clone(),
        id: Some(key.to_string()),
        native_id: None,
    }
}

/// Insert one row unless the count budget is exhausted.
///
/// A dropped row returns `false`, and the caller leaves every element reference
/// to it `None` — the document never points at a definition that is not there.
fn insert_definition(
    definitions: &mut BimDefinitions,
    stats: &mut Stats,
    row: BimDefinition,
) -> bool {
    let id = row.id().to_string();
    if definitions.get(&id).is_some() {
        return true;
    }
    if definitions.len() >= BIM_MAX_DEFINITIONS {
        stats.budget_exceeded += 1;
        return false;
    }
    match definitions.insert(row) {
        Ok(()) => true,
        Err(_) => {
            // A duplicate id is the only refusal `insert` documents; the check
            // above should have caught it. Count it rather than lose the row
            // silently.
            stats.budget_exceeded += 1;
            false
        }
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|text| !text.trim().is_empty())
}

/// A non-empty, per-string-budgeted text field (a name, an id, a tag).
fn bounded_text(value: Option<&str>, stats: &mut Stats) -> Option<String> {
    let value = non_empty(value)?;
    if value.len() > BIM_MAX_STRING_BYTES {
        stats.budget_exceeded += 1;
        return None;
    }
    Some(value.to_string())
}

/// A text value that may legitimately be empty; only its length is bounded.
fn bounded_string(value: &str, stats: &mut Stats) -> Option<String> {
    if value.len() > BIM_MAX_STRING_BYTES {
        stats.budget_exceeded += 1;
        return None;
    }
    Some(value.to_string())
}

#[cfg(test)]
#[path = "tests_semantics.rs"]
mod tests_semantics;

#[cfg(test)]
#[path = "tests_semantics_quantity.rs"]
mod tests_semantics_quantity;
