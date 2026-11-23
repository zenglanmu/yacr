//! Revit -> BIM semantics from the `IfcModel` the GLB is made from
//! (ADR-0042 D8).
//!
//! [`crate::decode`] already runs `rvt::ifc::run_export`, which produces an
//! [`rvt::ifc::IfcModel`]; before this module existed the model was dropped
//! after [`rvt::ifc::gltf::model_to_glb`] read it. The model is the only place
//! Revit's storeys, types, materials and property sets survive: the
//! intermediate GLB's node `extras` carry only `entityIndex`/`ifcType`
//! (`vendor/rvt/src/ifc/gltf.rs`), and rebuilding the rest from them would be
//! a lossy guess (ADR-0042 D8).
//!
//! # What is read, and where it goes
//!
//! | `IfcModel` | Stored as |
//! |---|---|
//! | `IfcEntity::BuildingElement.ifc_type` | `bim.category`, normalized to IFC spelling |
//! | `IfcEntity::BuildingElement.name` | `bim.name` |
//! | `global_ids.elements[i]` | `bim.source.id` (`native_id` is the ElementId) |
//! | `building_storeys[storey_index]` | `level` definition (`elevation_feet * 0.3048`) + `bim.level` |
//! | `element_type_ids` + the element name's `Family:Type` tail | `type` definition + `bim.element_type` |
//! | `type_property_sets` | the type definition's `psets` |
//! | `materials[material_index]` | `material` definition + `bim.material` |
//! | `BuildingElement.property_set` + `ElementPropertySet` | `bim.psets` |
//!
//! `IfcEntity::Aggregate`'s `parts` are **not** grouped into their `whole`
//! (Phase 3, ADR-0042 D8); the number of aggregates and parts is counted so the
//! omission is reported rather than silent.
//!
//! # Identity is not ownership
//!
//! `RevitGlobalIds::document` is the file's `Unique Document GUID`, which the
//! upstream documentation says **changes on every save**. It is therefore never
//! used as `BimSource::document_id` (ADR-0042 D5): RVT stores
//! `document_id: None` and takes the element's own IFC GlobalId as `id`. A file
//! that yields no element GlobalId falls back to the ElementId as `native_id`
//! and reports [`crate::DIAG_BIM_SOURCE_ID_MISSING`].
//!
//! # Status ceiling
//!
//! Everything here is **declared by the file but unverified as a whole**: the
//! mapping is a code contract built on `rvt` 0.25.0's model, no real Revit
//! project has been validated, and `rvt` itself says its output must not be used
//! for measurement or coordination. The importer reports
//! [`crate::DIAG_BIM_UNVERIFIED`] once; `evidence` stays
//! [`cad_db::BimEvidence::Declared`] because each stored value is spelled in
//! the file.

use std::collections::BTreeMap;

use cad_connector_core::MeshImport;
use cad_db::{
    BimDefinition, BimDefinitions, BimElement, BimEvidence, BimProperty, BimPropertySet, BimSource,
    BimValue, BIM_MAX_DEFINITIONS, BIM_MAX_PROPERTIES_PER_SET, BIM_MAX_PSETS_PER_ELEMENT,
    BIM_MAX_STRING_BYTES,
};
use cad_domain::Diagnostic;
use rvt::ifc::entities::{IfcEntity, PropertySet, PropertyValue};
use rvt::ifc::IfcModel;

use crate::{
    DIAG_BIM_AGGREGATE_PARTS_NOT_GROUPED, DIAG_BIM_BUDGET_EXCEEDED, DIAG_BIM_MATERIAL_UNRESOLVED,
    DIAG_BIM_PROPERTY_VALUE_UNSUPPORTED, DIAG_BIM_SOURCE_ID_MISSING, DIAG_BIM_STOREY_UNRESOLVED,
    DIAG_BIM_TYPE_NAME_MISSING, DIAG_BIM_UNVERIFIED,
};

/// The connector id every RVT BIM row carries.
pub const CONNECTOR: &str = "rvt";

/// Feet per metre divider: Revit elevations are feet, the database stores metres.
const FEET_TO_METRES: f64 = 0.3048;

/// Reading losses, aggregated per stable diagnostic code.
///
/// A pure [`map_model`] cannot record diagnostics itself; the counters travel
/// with the result and [`RvtSemantics::record`] turns each non-zero one into
/// one `import.rvt.bim.*` diagnostic.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SemanticsCounts {
    /// Building elements whose `global_ids.elements[i]` entry was missing or
    /// unusable; `source.id` is `None` and `native_id` is the ElementId.
    pub source_id_missing: usize,
    /// Elements whose `storey_index` names no storey in `building_storeys`.
    pub storey_unresolved: usize,
    /// Elements with a Revit type but an element name that is not `Family:Type`,
    /// so no honest type name exists.
    pub type_name_missing: usize,
    /// `material_index` names no material, or the material has no name.
    pub material_unresolved: usize,
    /// Property values with no [`BimValue`] form (enumerated, list, unknown
    /// logical, non-finite real). Counted, never flattened.
    pub property_value_unsupported: usize,
    /// `IfcEntity::Aggregate` records whose parts were not grouped (Phase 3).
    pub aggregates_not_grouped: usize,
    /// The sum of those aggregates' `parts` lengths.
    pub aggregate_parts: usize,
    /// A row/property/string exceeded a `cad_db::BIM_*` budget and was dropped;
    /// every reference to a dropped definition is left `None`.
    pub budget_exceeded: usize,
}

/// The BIM result of one `IfcModel`, aligned to `model.entities`.
#[derive(Debug, Clone, Default)]
pub struct RvtSemantics {
    definitions: BimDefinitions,
    /// One entry per `model.entities` index: the element's BIM block, or `None`
    /// for a record that is not a geometry-bearing building element. The GLB's
    /// `extras.entityIndex` indexes straight into this vector.
    elements: Vec<Option<BimElement>>,
    counts: SemanticsCounts,
}

impl RvtSemantics {
    /// The BIM block of one `model.entities` index, if that record is a
    /// building element.
    ///
    /// `entity_index` is the GLB node's `extras.entityIndex`; `None` (no extras)
    /// resolves to `None`, never to a guessed element.
    pub fn element_for_index(&self, entity_index: Option<usize>) -> Option<&BimElement> {
        self.elements.get(entity_index?)?.as_ref()
    }

    /// Take the shared definition table for the builder (which owns it until
    /// `finish`).
    pub fn take_definitions(&mut self) -> BimDefinitions {
        std::mem::take(&mut self.definitions)
    }

    /// The shared definition table as mapped so far.
    pub fn definitions(&self) -> &BimDefinitions {
        &self.definitions
    }

    /// The counters, for diagnostics and contract tests.
    pub fn counts(&self) -> &SemanticsCounts {
        &self.counts
    }

    /// Whether any element got a BIM block (and therefore the document holds
    /// BIM data at all).
    pub fn is_empty(&self) -> bool {
        !self.elements.iter().any(Option::is_some)
    }

    /// Write every reading loss, and the status-ceiling note, onto the import's
    /// report channel.
    ///
    /// Counts are aggregated per stable code, the way the geometry lane reports
    /// its own drops, so a hostile file cannot turn one import into thousands of
    /// diagnostics. This never changes the geometry verdict: an RVT import is
    /// already `Partial` for preview geometry, and a document with no BIM gets
    /// no BIM diagnostic at all.
    pub(crate) fn record(&self, import: &mut MeshImport) {
        if self.is_empty() {
            return;
        }
        let counts = &self.counts;
        if counts.source_id_missing > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_SOURCE_ID_MISSING.into(),
                    message: format!(
                        "{} Revit building element(s) carry no IFC GlobalId in the export; their \
                         BIM source id is empty and only the Revit ElementId (native id) is \
                         stored, so they can be grouped only by their own EntityId",
                        counts.source_id_missing
                    ),
                },
                Some(format!(
                    "{} Revit element(s) had no GlobalId for their BIM source",
                    counts.source_id_missing
                )),
            );
        }
        if counts.storey_unresolved > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_STOREY_UNRESOLVED.into(),
                    message: format!(
                        "{} Revit building element(s) name a storey index that is out of range \
                         for the exported storey list (or a storey with no name); their level is \
                         left empty rather than pointed at an invented storey",
                        counts.storey_unresolved
                    ),
                },
                Some(format!(
                    "{} Revit element(s) had an unresolvable storey",
                    counts.storey_unresolved
                )),
            );
        }
        if counts.type_name_missing > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_TYPE_NAME_MISSING.into(),
                    message: format!(
                        "{} Revit building element(s) have a type but a name that is not \
                         `Family:Type`; no type definition is created, because the type's name \
                         cannot be read without guessing",
                        counts.type_name_missing
                    ),
                },
                Some(format!(
                    "{} Revit type name(s) could not be read from the element name",
                    counts.type_name_missing
                )),
            );
        }
        if counts.material_unresolved > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_MATERIAL_UNRESOLVED.into(),
                    message: format!(
                        "{} Revit building element(s) name a material index that is out of range \
                         or a material with no name; their material is left empty",
                        counts.material_unresolved
                    ),
                },
                Some(format!(
                    "{} Revit material reference(s) could not be resolved",
                    counts.material_unresolved
                )),
            );
        }
        if counts.property_value_unsupported > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_PROPERTY_VALUE_UNSUPPORTED.into(),
                    message: format!(
                        "{} Revit property value(s) are enumerated, list-valued, an unknown \
                         logical or a non-finite real; the BIM value model cannot represent them, \
                         so they were counted and left out, not flattened into strings",
                        counts.property_value_unsupported
                    ),
                },
                Some(format!(
                    "{} Revit property value(s) had no BIM value representation",
                    counts.property_value_unsupported
                )),
            );
        }
        if counts.aggregates_not_grouped > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_AGGREGATE_PARTS_NOT_GROUPED.into(),
                    message: format!(
                        "{} Revit aggregate record(s) decompose elements into {} part(s) \
                         (`IfcEntity::Aggregate`); the parts stay as their own BIM elements \
                         (grouping is Phase 3, ADR-0042 D8)",
                        counts.aggregates_not_grouped, counts.aggregate_parts
                    ),
                },
                Some(format!(
                    "{} Revit aggregate(s) with {} part(s) were not grouped",
                    counts.aggregates_not_grouped, counts.aggregate_parts
                )),
            );
        }
        if counts.budget_exceeded > 0 {
            import.record(
                Diagnostic {
                    object: None,
                    code: DIAG_BIM_BUDGET_EXCEEDED.into(),
                    message: format!(
                        "{} RVT BIM row(s)/property(ies)/string(s) exceeded a cad-db BIM_* \
                         budget and were dropped; every element reference to a dropped \
                         definition is empty, so the document holds no dangling \
                         level/type/material reference",
                        counts.budget_exceeded
                    ),
                },
                Some(format!(
                    "{} RVT BIM row(s) exceeded a cad-db budget and were dropped",
                    counts.budget_exceeded
                )),
            );
        }
        import.record(
            Diagnostic {
                object: None,
                code: DIAG_BIM_UNVERIFIED.into(),
                message: "RVT BIM semantics (category/name/source/level/type/material/property \
                          sets) are read from rvt 0.25.0's IfcModel of the file, not from the \
                          intermediate GLB; the mapping is a code contract — no real Revit \
                          project has been verified, and rvt's own output must not be used for \
                          measurement or coordination"
                    .into(),
            },
            None,
        );
    }
}

/// Map one exported [`IfcModel`] into shared definitions and per-element blocks.
pub fn map_model(model: &IfcModel) -> RvtSemantics {
    // `rvt` writes an element's own set as the element's `property_set` and
    // every further declared set as a later `ElementPropertySet` record; both
    // describe the same element and both belong in its `bim.psets`.
    let mut further_psets: BTreeMap<usize, Vec<&PropertySet>> = BTreeMap::new();
    for entity in &model.entities {
        if let IfcEntity::ElementPropertySet { element, set } = entity {
            further_psets.entry(*element).or_default().push(set);
        }
    }
    let mut semantics = RvtSemantics {
        definitions: BimDefinitions::new(),
        elements: Vec::with_capacity(model.entities.len()),
        counts: SemanticsCounts::default(),
    };
    for (index, entity) in model.entities.iter().enumerate() {
        semantics.elements.push(map_entity(
            index,
            entity,
            model,
            further_psets.get(&index).map(Vec::as_slice).unwrap_or(&[]),
            &mut semantics.definitions,
            &mut semantics.counts,
        ));
    }
    semantics
}

/// One `model.entities` entry: a BIM block for a building element, `None` for
/// every other variant (whose own BIM-relevant content is counted here).
fn map_entity(
    index: usize,
    entity: &IfcEntity,
    model: &IfcModel,
    further_psets: &[&PropertySet],
    definitions: &mut BimDefinitions,
    counts: &mut SemanticsCounts,
) -> Option<BimElement> {
    let IfcEntity::BuildingElement {
        ifc_type,
        name,
        type_guid,
        storey_index,
        material_index,
        property_set,
        ..
    } = entity
    else {
        if let IfcEntity::Aggregate { parts, .. } = entity {
            // ADR-0042 D8: the parts are elements of their own in this phase.
            counts.aggregates_not_grouped += 1;
            counts.aggregate_parts += parts.len();
        }
        return None;
    };

    let category = normalize_ifc_category(ifc_type);
    let name = bounded(name, counts);
    let global_id = model
        .global_ids
        .elements
        .get(&index)
        .and_then(|global_id| bounded(global_id, counts));
    if global_id.is_none() {
        counts.source_id_missing += 1;
    }
    let native_id = bounded_opt(type_guid.as_deref(), counts);
    let element_id = native_id.as_deref().and_then(|tag| tag.parse::<u32>().ok());

    let level = resolve_level(model, *storey_index, definitions, counts);
    let element_type = resolve_type(
        model,
        element_id,
        name.as_deref(),
        category.as_deref(),
        definitions,
        counts,
    );
    let material = resolve_material(model, *material_index, definitions, counts);

    let mut psets = Vec::new();
    if let Some(set) = property_set {
        if let Some(set) = convert_pset(set, counts) {
            psets.push(set);
        }
    }
    for set in further_psets {
        if psets.len() >= BIM_MAX_PSETS_PER_ELEMENT {
            counts.budget_exceeded += 1;
            continue;
        }
        if let Some(set) = convert_pset(set, counts) {
            psets.push(set);
        }
    }

    Some(BimElement {
        category,
        name,
        source: BimSource {
            connector: CONNECTOR.to_string(),
            // The document's `Unique Document GUID` changes on every save
            // (upstream), so it is provenance at best and never an identity.
            document_id: None,
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
// Levels.
// ---------------------------------------------------------------------------

fn resolve_level(
    model: &IfcModel,
    storey_index: Option<usize>,
    definitions: &mut BimDefinitions,
    counts: &mut SemanticsCounts,
) -> Option<String> {
    let index = storey_index?;
    let Some(storey) = model.building_storeys.get(index) else {
        counts.storey_unresolved += 1;
        return None;
    };
    let Some(name) = bounded(&storey.name, counts) else {
        counts.storey_unresolved += 1;
        return None;
    };
    // A Level's own GlobalId is the only stable per-storey identity the export
    // carries; a file without one keeps the name as the key.
    let global_id = model
        .global_ids
        .storeys
        .get(&index)
        .and_then(|global_id| bounded(global_id, counts));
    let key = global_id.clone().unwrap_or_else(|| name.clone());
    let elevation = storey
        .elevation_feet
        .is_finite()
        .then_some(storey.elevation_feet * FEET_TO_METRES);

    let id = level_id(definitions, index, &key, elevation);
    if definitions.get(&id).is_some() {
        return Some(id);
    }
    let row = BimDefinition::Level {
        id: id.clone(),
        name,
        elevation,
        source: BimSource {
            connector: CONNECTOR.to_string(),
            document_id: None,
            id: Some(key.to_string()),
            native_id: Some(index.to_string()),
        },
        evidence: BimEvidence::Declared,
    };
    insert_definition(definitions, counts, row).then_some(id)
}

/// `level:rvt:<GlobalId-or-name>`, with `#<index>` only when two same-named
/// storeys carry no GlobalId and different elevations.
fn level_id(
    definitions: &BimDefinitions,
    index: usize,
    key: &str,
    elevation: Option<f64>,
) -> String {
    let base = format!("level:{CONNECTOR}:{key}");
    match definitions.get(&base) {
        None => base,
        Some(BimDefinition::Level {
            elevation: existing,
            ..
        }) if *existing == elevation => base,
        _ => format!("{base}#{index}"),
    }
}

// ---------------------------------------------------------------------------
// Types.
// ---------------------------------------------------------------------------

fn resolve_type(
    model: &IfcModel,
    element_id: Option<u32>,
    element_name: Option<&str>,
    category: Option<&str>,
    definitions: &mut BimDefinitions,
    counts: &mut SemanticsCounts,
) -> Option<String> {
    let type_element_id = model.element_type_ids.get(&element_id?)?.to_owned();
    // Revit names an instance `Family:Type`; only the Type half names the type
    // object. No colon means no honest type name: no definition is invented.
    let Some(type_name) = element_name
        .and_then(|name| name.rsplit_once(':'))
        .map(|(_, tail)| tail.trim())
        .filter(|tail| !tail.is_empty())
    else {
        counts.type_name_missing += 1;
        return None;
    };
    let id = format!("type:{CONNECTOR}:{type_element_id}");
    if definitions.get(&id).is_some() {
        return Some(id);
    }
    let psets = model
        .type_property_sets
        .get(&type_element_id)
        .map(|sets| convert_psets(sets, counts))
        .unwrap_or_default();
    let row = BimDefinition::Type {
        id: id.clone(),
        name: type_name.to_string(),
        // `IfcModel` exposes the type's ElementId, not the IFC type-object
        // class it is written as; deriving `IfcWallType` from the category
        // would be a guess, so the class stays unknown.
        class: None,
        category: category.map(str::to_string),
        psets,
        source: BimSource {
            connector: CONNECTOR.to_string(),
            document_id: None,
            id: Some(type_element_id.to_string()),
            native_id: None,
        },
        evidence: BimEvidence::Declared,
    };
    insert_definition(definitions, counts, row).then_some(id)
}

// ---------------------------------------------------------------------------
// Materials.
// ---------------------------------------------------------------------------

fn resolve_material(
    model: &IfcModel,
    material_index: Option<usize>,
    definitions: &mut BimDefinitions,
    counts: &mut SemanticsCounts,
) -> Option<String> {
    let index = material_index?;
    let Some(material) = model.materials.get(index) else {
        counts.material_unresolved += 1;
        return None;
    };
    let Some(name) = bounded(&material.name, counts) else {
        counts.material_unresolved += 1;
        return None;
    };
    let id = format!("material:{CONNECTOR}:{name}");
    if definitions.get(&id).is_some() {
        return Some(id);
    }
    let row = BimDefinition::Material {
        id: id.clone(),
        name,
        source: BimSource {
            connector: CONNECTOR.to_string(),
            document_id: None,
            id: Some(format!("#{index}")),
            native_id: Some(index.to_string()),
        },
        evidence: BimEvidence::Declared,
    };
    insert_definition(definitions, counts, row).then_some(id)
}

// ---------------------------------------------------------------------------
// Property sets.
// ---------------------------------------------------------------------------

fn convert_psets(sets: &[PropertySet], counts: &mut SemanticsCounts) -> Vec<BimPropertySet> {
    let mut converted = Vec::new();
    for set in sets {
        if converted.len() >= BIM_MAX_PSETS_PER_ELEMENT {
            counts.budget_exceeded += 1;
            continue;
        }
        if let Some(set) = convert_pset(set, counts) {
            converted.push(set);
        }
    }
    converted
}

fn convert_pset(set: &PropertySet, counts: &mut SemanticsCounts) -> Option<BimPropertySet> {
    let name = bounded(&set.name, counts)?;
    let mut properties = Vec::new();
    for property in &set.properties {
        if properties.len() >= BIM_MAX_PROPERTIES_PER_SET {
            counts.budget_exceeded += 1;
            continue;
        }
        let Some(property_name) = bounded(&property.name, counts) else {
            continue;
        };
        match convert_value(&property.value) {
            Some(value) => properties.push(BimProperty {
                name: property_name,
                value,
            }),
            None => counts.property_value_unsupported += 1,
        }
    }
    Some(BimPropertySet { name, properties })
}

/// Convert one declared IFC value into the closed [`BimValue`] model.
///
/// `None` is a value the model cannot represent (ADR-0042 D2): enumerated,
/// list-valued, an unknown logical, or a non-finite real. The caller counts it;
/// nothing is flattened into a string that would look like the source's value.
fn convert_value(value: &PropertyValue) -> Option<BimValue> {
    Some(match value {
        PropertyValue::Text(text)
        | PropertyValue::Label(text)
        | PropertyValue::Identifier(text) => BimValue::Text(text.clone()),
        PropertyValue::Integer(number) => BimValue::Integer(*number),
        PropertyValue::Real(number) => BimValue::Real {
            value: finite(*number)?,
            ifc_type: None,
            unit: None,
        },
        PropertyValue::Boolean(value) => BimValue::Bool(*value),
        // A three-valued logical with no value is unknown, not `false` and not
        // an explicit null; the BIM value model has no "unknown" form.
        PropertyValue::Logical(Some(value)) => BimValue::Bool(*value),
        PropertyValue::Logical(None) => return None,
        // Revit stores these in feet / square feet / cubic feet / pounds; the
        // BIM property model stores metres / SI, so the conversion happens here
        // and the measure is named on the value.
        PropertyValue::LengthFeet(feet) => {
            measure(*feet * FEET_TO_METRES, "IfcLengthMeasure", "m")?
        }
        PropertyValue::PositiveLengthFeet(feet) => {
            measure(*feet * FEET_TO_METRES, "IfcPositiveLengthMeasure", "m")?
        }
        PropertyValue::AngleRadians(radians) => measure(*radians, "IfcPlaneAngleMeasure", "rad")?,
        PropertyValue::AreaSquareFeet(square_feet) => {
            measure(*square_feet * 0.092_903_04, "IfcAreaMeasure", "m2")?
        }
        PropertyValue::VolumeCubicFeet(cubic_feet) => {
            measure(*cubic_feet * 0.028_316_846_592, "IfcVolumeMeasure", "m3")?
        }
        PropertyValue::CountValue(count) => BimValue::Integer(*count),
        PropertyValue::TimeSeconds(seconds) => measure(*seconds, "IfcTimeMeasure", "s")?,
        PropertyValue::MassPounds(pounds) => {
            measure(*pounds * 0.453_592_37, "IfcMassMeasure", "kg")?
        }
        PropertyValue::ThermalTransmittance(value) => {
            measure(*value, "IfcThermalTransmittanceMeasure", "W/(m2.K)")?
        }
        PropertyValue::VolumetricFlowRate(value) => {
            measure(*value, "IfcVolumetricFlowRateMeasure", "m3/s")?
        }
        PropertyValue::Pressure(value) => measure(*value, "IfcPressureMeasure", "Pa")?,
        PropertyValue::Power(value) => measure(*value, "IfcPowerMeasure", "W")?,
        PropertyValue::ElectricVoltage(value) => measure(*value, "IfcElectricVoltageMeasure", "V")?,
        PropertyValue::ElectricCurrent(value) => measure(*value, "IfcElectricCurrentMeasure", "A")?,
        PropertyValue::Ratio(value) => measure(*value, "IfcRatioMeasure", "")?,
        // Enumerated sets and lists have no BIM value form (D2).
        PropertyValue::List(_) | PropertyValue::Enumerated(_) => return None,
        // `PropertyValue` is `#[non_exhaustive]`: a future variant has no form
        // here either.
        _ => return None,
    })
}

/// A named measure as a [`BimValue::Real`], or `None` for a non-finite number.
fn measure(value: f64, ifc_type: &str, unit: &str) -> Option<BimValue> {
    Some(BimValue::Real {
        value: finite(value)?,
        ifc_type: Some(ifc_type.to_string()),
        unit: (!unit.is_empty()).then(|| unit.to_string()),
    })
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

// ---------------------------------------------------------------------------
// Categories.
// ---------------------------------------------------------------------------

/// Normalize `rvt`'s uppercase STEP entity name to IFC's spelling.
///
/// `"IFCWALL"` becomes `"IfcWall"`, `"IFCCURTAINWALL"` `"IfcCurtainWall"`. The
/// canonical spellings of the entities `rvt`'s category map emits are spelled
/// out; anything else becomes `Ifc` + the tail with its first letter upper-cased
/// and the rest lower-cased, which is deterministic but not claimed to be the
/// IFC spelling. An empty name yields `None`.
pub fn normalize_ifc_category(ifc_type: &str) -> Option<String> {
    let trimmed = ifc_type.trim();
    if trimmed.is_empty() {
        return None;
    }
    let upper = trimmed.to_ascii_uppercase();
    if let Some((_, canonical)) = CANONICAL_IFC_TYPES
        .iter()
        .find(|(step_name, _)| *step_name == upper)
    {
        return Some((*canonical).to_string());
    }
    let tail = upper.strip_prefix("IFC").unwrap_or(&upper);
    let mut chars = tail.chars();
    let first = chars.next()?;
    Some(format!(
        "Ifc{}{}",
        first.to_ascii_uppercase(),
        chars.as_str().to_ascii_lowercase()
    ))
}

/// The IFC4 spellings of the entities `vendor/rvt/src/ifc/category_map.rs` maps
/// Revit classes to. Sorted by STEP name; looked up with `==` (the list is
/// small and the scan happens once per element).
const CANONICAL_IFC_TYPES: &[(&str, &str)] = &[
    ("IFCAIRTERMINAL", "IfcAirTerminal"),
    ("IFCALARM", "IfcAlarm"),
    ("IFCBEAM", "IfcBeam"),
    ("IFCBUILDINGELEMENTPROXY", "IfcBuildingElementProxy"),
    ("IFCBUILDINGSTOREY", "IfcBuildingStorey"),
    ("IFCCABLECARRIERFITTING", "IfcCableCarrierFitting"),
    ("IFCCABLECARRIERSEGMENT", "IfcCableCarrierSegment"),
    ("IFCCOLUMN", "IfcColumn"),
    ("IFCCOVERING", "IfcCovering"),
    ("IFCCURTAINWALL", "IfcCurtainWall"),
    ("IFCDOOR", "IfcDoor"),
    ("IFCDUCTFITTING", "IfcDuctFitting"),
    ("IFCDUCTSEGMENT", "IfcDuctSegment"),
    ("IFCELECTRICAPPLIANCE", "IfcElectricAppliance"),
    ("IFCFIRESUPPRESSIONTERMINAL", "IfcFireSuppressionTerminal"),
    ("IFCFLOWSEGMENT", "IfcFlowSegment"),
    ("IFCFOOTING", "IfcFooting"),
    ("IFCFURNITURE", "IfcFurniture"),
    ("IFCGEOGRAPHICELEMENT", "IfcGeographicElement"),
    ("IFCGRID", "IfcGrid"),
    ("IFCLIGHTFIXTURE", "IfcLightFixture"),
    ("IFCMEMBER", "IfcMember"),
    ("IFCPIPEFITTING", "IfcPipeFitting"),
    ("IFCPIPESEGMENT", "IfcPipeSegment"),
    ("IFCPLATE", "IfcPlate"),
    ("IFCRAILING", "IfcRailing"),
    ("IFCRAMP", "IfcRamp"),
    ("IFCREINFORCINGBAR", "IfcReinforcingBar"),
    ("IFCROOF", "IfcRoof"),
    ("IFCSANITARYTERMINAL", "IfcSanitaryTerminal"),
    ("IFCSHADINGDEVICE", "IfcShadingDevice"),
    ("IFCSLAB", "IfcSlab"),
    ("IFCSOLARDEVICE", "IfcSolarDevice"),
    ("IFCSPACE", "IfcSpace"),
    ("IFCSTAIR", "IfcStair"),
    ("IFCSTAIRFLIGHT", "IfcStairFlight"),
    ("IFCSYSTEMFURNITUREELEMENT", "IfcSystemFurnitureElement"),
    ("IFCTRANSPORTELEMENT", "IfcTransportElement"),
    ("IFCWALL", "IfcWall"),
    ("IFCWASTETERMINAL", "IfcWasteTerminal"),
    ("IFCWINDOW", "IfcWindow"),
];

// ---------------------------------------------------------------------------
// Sharing helpers.
// ---------------------------------------------------------------------------

/// Insert one row unless the count budget is exhausted.
///
/// A dropped row returns `false`, and the caller leaves every element reference
/// to it `None` — the document never points at a definition that is not there.
fn insert_definition(
    definitions: &mut BimDefinitions,
    counts: &mut SemanticsCounts,
    row: BimDefinition,
) -> bool {
    let id = row.id().to_string();
    if definitions.get(&id).is_some() {
        return true;
    }
    if definitions.len() >= BIM_MAX_DEFINITIONS {
        counts.budget_exceeded += 1;
        return false;
    }
    match definitions.insert(row) {
        Ok(()) => true,
        Err(_) => {
            // A duplicate id is the only refusal `insert` documents; the check
            // above should have caught it. Count it rather than lose the row
            // silently.
            counts.budget_exceeded += 1;
            false
        }
    }
}

/// A non-empty, per-string-budgeted text field (a name, an id, a category).
fn bounded(value: &str, counts: &mut SemanticsCounts) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() > BIM_MAX_STRING_BYTES {
        counts.budget_exceeded += 1;
        return None;
    }
    Some(trimmed.to_string())
}

fn bounded_opt(value: Option<&str>, counts: &mut SemanticsCounts) -> Option<String> {
    bounded(value?, counts)
}

#[cfg(test)]
#[path = "tests_semantics.rs"]
mod tests_semantics;
