//! BIM semantics attached to elements (ADR-0042 D2/D3/D5/D6).
//!
//! Three pieces of document state live here, all of them **optional**: a
//! document with no BIM data is byte-identical to a minor-4/5 archive.
//!
//! * [`BimElement`] — one element's placement in the BIM world: its source
//!   identity, category, name, the level/type/material definition it points at,
//!   its property sets and how the data was obtained ([`BimEvidence`]). It is
//!   stored *beside* the entity row (a side table, like render attributes) and
//!   written as the optional `bim` block of that element's shard row, so a save
//!   that touches no element still raw-copies the shard (D2).
//! * [`BimDefinitions`] — the shared definition rows (`level`/`type`/`material`)
//!   whose ids are **readable composite strings**, not [`PersistentId`]s: they
//!   are document-local keys like `textures/<key>`, so they never touch
//!   [`cad_domain::ymodel::IdKind`] or `document.json` (D3/D12).
//! * [`BimKey`] — the logical-component equivalence class
//!   `(connector, document_id, id)`, falling back to the element id when the
//!   source gave no identifier (D5). Counting and matching go through it so one
//!   source element split into several geometry rows counts once.
//!
//! # No silent truncation
//!
//! Every limit below is a **refusal**, never a truncation: an importer that
//! exceeds one gets a [`BimError`] it must report, and the reader refuses an
//! archive whose rows exceed them (`ymodel.bim.budget_exceeded`). A value that
//! cannot be represented is not written as a lossy string (D2: complex /
//! enumerated / bounded IFC values are counted and reported by the importer, not
//! flattened here).

use std::collections::BTreeMap;
use std::fmt;

use cad_domain::EntityId;

/// Most rows `tables/bim-definitions.json` may hold (ADR-0042 D3).
pub const BIM_MAX_DEFINITIONS: usize = 100_000;

/// Most bytes the encoded definition table may occupy (ADR-0042 D3).
pub const BIM_MAX_DEFINITION_BYTES: usize = 32 * 1024 * 1024;

/// Most property sets one element may carry (ADR-0042 D7).
pub const BIM_MAX_PSETS_PER_ELEMENT: usize = 64;

/// Most properties one property set may carry (ADR-0042 D7).
pub const BIM_MAX_PROPERTIES_PER_SET: usize = 256;

/// Longest single string a `bim` block or definition row may carry (ADR-0042 D7).
pub const BIM_MAX_STRING_BYTES: usize = 4 * 1024;

/// A BIM block or definition row whose shape is not representable.
pub const BIM_INVALID: &str = "ymodel.bim.invalid";

/// A BIM row set exceeds a budget; nothing was truncated.
pub const BIM_BUDGET_EXCEEDED: &str = "ymodel.bim.budget_exceeded";

/// An element points at a definition that is not in the table.
pub const BIM_DANGLING_REFERENCE: &str = "ymodel.bim.dangling_reference";

/// How the BIM data was obtained (ADR-0042 D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BimEvidence {
    /// The source file stated it (a real `IfcPropertySet`, an Item Type, …).
    Declared,
    /// A heuristic produced it; only ever with an explicit import option.
    Heuristic,
}

impl BimEvidence {
    /// The persisted spelling. Part of the format.
    pub const fn as_str(self) -> &'static str {
        match self {
            BimEvidence::Declared => "declared",
            BimEvidence::Heuristic => "heuristic",
        }
    }

    /// Parse the persisted spelling; an unknown tag is a malformed document.
    pub fn parse(value: &str) -> Option<BimEvidence> {
        match value {
            "declared" => Some(BimEvidence::Declared),
            "heuristic" => Some(BimEvidence::Heuristic),
            _ => None,
        }
    }
}

/// One BIM property value.
///
/// Deliberately closed (ADR-0042 D2): IFC complex/enumerated/bounded values and
/// quantities are **not** flattened into [`Self::Text`] here. The importer
/// counts them and reports `*.bim.property_value_unsupported` instead; a value
/// stored here is a value the source actually declared in that form.
///
/// `ifc_type` and `unit` are carried on [`Self::Real`] only: they describe the
/// measure (for example `IfcPositiveLengthMeasure` / `m`) and are meaningless on
/// a boolean.
#[derive(Debug, Clone, PartialEq)]
pub enum BimValue {
    /// A string value, as declared.
    Text(String),
    /// A real number, with its optional IFC measure type and unit token.
    Real {
        value: f64,
        ifc_type: Option<String>,
        unit: Option<String>,
    },
    /// An integer value, as declared.
    Integer(i64),
    /// A boolean value, as declared.
    Bool(bool),
    /// An explicit null value (a property whose source value was null).
    Null,
}

/// One named property inside a set.
#[derive(Debug, Clone, PartialEq)]
pub struct BimProperty {
    pub name: String,
    pub value: BimValue,
}

/// One property set (`Pset_WallCommon`, a Revit type PSet, DGN Item Type, …).
#[derive(Debug, Clone, PartialEq)]
pub struct BimPropertySet {
    pub name: String,
    pub properties: Vec<BimProperty>,
}

/// Where an element's BIM data came from (ADR-0042 D5).
///
/// The source identifier is **not** an identity: RVT's document GUID changes on
/// every save and an IFC `GlobalId` is unique only inside its file, so nothing
/// here promises cross-file merging. `document_id`/`id` are provenance and the
/// grouping key inside one import batch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BimSource {
    /// Connector id (`"ifc"`, `"rvt"`, `"dgn"`). Never empty for a real source.
    pub connector: String,
    /// The source document's own identifier, when it has one.
    pub document_id: Option<String>,
    /// The element's identifier inside that document (`GlobalId`, …).
    pub id: Option<String>,
    /// The source-native id (`Tag`, a Revit element id, …) for diagnostics.
    pub native_id: Option<String>,
}

/// The logical component a set of geometry rows belongs to (ADR-0042 D5).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BimKey {
    /// `(connector, document_id, id)` when both source strings exist.
    Source {
        connector: String,
        document_id: String,
        id: String,
    },
    /// No usable source identifier: the element itself is the component.
    Entity(EntityId),
}

/// The BIM block of one element row (ADR-0042 D2).
#[derive(Debug, Clone, PartialEq)]
pub struct BimElement {
    /// The IFC entity spelling / RVT `ifc_type` / an explicitly tagged DGN
    /// category. An open token, never a table (D3).
    pub category: Option<String>,
    /// The element's display name, when the source gave one.
    pub name: Option<String>,
    /// Provenance and the grouping key.
    pub source: BimSource,
    /// Reference to a `level` definition row.
    pub level: Option<String>,
    /// Reference to a `type` definition row. Persisted as `"type"`, which is a
    /// Rust keyword, hence the field name.
    pub element_type: Option<String>,
    /// Reference to a `material` definition row.
    pub material: Option<String>,
    /// Inline property sets. Inline (not a definition) because they describe
    /// this instance and are not shared (D4).
    pub psets: Vec<BimPropertySet>,
    /// How the data was obtained.
    pub evidence: BimEvidence,
}

impl BimElement {
    /// The logical-component key of this element (ADR-0042 D5).
    pub fn key(&self, entity: EntityId) -> BimKey {
        match (&self.source.document_id, &self.source.id) {
            (Some(document_id), Some(id)) if !document_id.is_empty() && !id.is_empty() => {
                BimKey::Source {
                    connector: self.source.connector.clone(),
                    document_id: document_id.clone(),
                    id: id.clone(),
                }
            }
            _ => BimKey::Entity(entity),
        }
    }
}

/// Which shared definition table a row belongs to (ADR-0042 D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BimDefinitionKind {
    /// A building storey.
    Level,
    /// An element type (`IfcWallType`, a Revit family:type, …).
    Type,
    /// A material.
    Material,
}

impl BimDefinitionKind {
    /// The persisted `kind` tag and the id prefix. Part of the format.
    pub const fn as_str(self) -> &'static str {
        match self {
            BimDefinitionKind::Level => "level",
            BimDefinitionKind::Type => "type",
            BimDefinitionKind::Material => "material",
        }
    }

    /// Parse the persisted tag; unknown tags are a malformed document.
    pub fn parse(value: &str) -> Option<BimDefinitionKind> {
        match value {
            "level" => Some(BimDefinitionKind::Level),
            "type" => Some(BimDefinitionKind::Type),
            "material" => Some(BimDefinitionKind::Material),
            _ => None,
        }
    }

    /// The prefix every id of this kind must carry (`"level:"`, …).
    pub const fn id_prefix(self) -> &'static str {
        match self {
            BimDefinitionKind::Level => "level:",
            BimDefinitionKind::Type => "type:",
            BimDefinitionKind::Material => "material:",
        }
    }
}

/// One shared definition row (ADR-0042 D3).
///
/// The id is a readable composite string (`level:ifc:<doc>:<id>`), not a
/// [`cad_domain::ymodel::PersistentId`]: it is a document-local directory key,
/// and the element's `level`/`type`/`material` fields reference it verbatim.
#[derive(Debug, Clone, PartialEq)]
pub enum BimDefinition {
    /// A storey.
    Level {
        id: String,
        name: String,
        /// Elevation in document units when the source declared one.
        elevation: Option<f64>,
        source: BimSource,
        evidence: BimEvidence,
    },
    /// An element type.
    Type {
        id: String,
        name: String,
        /// The source type object's class (`IfcWallType`, …).
        class: Option<String>,
        /// The element category the type applies to, when declared.
        category: Option<String>,
        psets: Vec<BimPropertySet>,
        source: BimSource,
        evidence: BimEvidence,
    },
    /// A material.
    Material {
        id: String,
        name: String,
        source: BimSource,
        evidence: BimEvidence,
    },
}

impl BimDefinition {
    /// The composite id elements reference.
    pub fn id(&self) -> &str {
        match self {
            BimDefinition::Level { id, .. }
            | BimDefinition::Type { id, .. }
            | BimDefinition::Material { id, .. } => id,
        }
    }

    /// Which table row kind this is.
    pub const fn kind(&self) -> BimDefinitionKind {
        match self {
            BimDefinition::Level { .. } => BimDefinitionKind::Level,
            BimDefinition::Type { .. } => BimDefinitionKind::Type,
            BimDefinition::Material { .. } => BimDefinitionKind::Material,
        }
    }
}

/// `tables/bim-definitions.json`: the shared rows, keyed by their composite id.
///
/// A map, not a vector, so the writer is deterministic and a duplicate id is a
/// refusal at [`Self::insert`] rather than two rows the reader has to choose
/// between. Empty for every document without BIM data, and the writer emits no
/// table at all then (which is what keeps such a document at minor 4/5).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BimDefinitions {
    rows: BTreeMap<String, BimDefinition>,
}

impl BimDefinitions {
    /// An empty table.
    pub fn new() -> BimDefinitions {
        BimDefinitions::default()
    }

    /// Insert one row.
    ///
    /// A duplicate id is refused: two `level:` rows with one id would make the
    /// elements that reference it ambiguous, and the writer cannot merge them.
    pub fn insert(&mut self, row: BimDefinition) -> Result<(), BimError> {
        let id = row.id().to_string();
        if self.rows.contains_key(&id) {
            return Err(BimError::invalid(format!("duplicate definition id {id:?}")));
        }
        self.rows.insert(id, row);
        Ok(())
    }

    /// The row with this composite id.
    pub fn get(&self, id: &str) -> Option<&BimDefinition> {
        self.rows.get(id)
    }

    /// Every row, in id order.
    pub fn iter(&self) -> impl Iterator<Item = &BimDefinition> {
        self.rows.values()
    }

    /// How many rows the table holds.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the table is empty (and therefore not written).
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Adopt an already-unique row set (the decoder's path).
    pub(crate) fn from_rows(rows: BTreeMap<String, BimDefinition>) -> BimDefinitions {
        BimDefinitions { rows }
    }
}

impl<'a> IntoIterator for &'a BimDefinitions {
    type Item = &'a BimDefinition;
    type IntoIter = std::collections::btree_map::Values<'a, String, BimDefinition>;

    fn into_iter(self) -> Self::IntoIter {
        self.rows.values()
    }
}

/// Why BIM data could not be stored or loaded.
///
/// `marker` is the stable contract (`ymodel.bim.*`); the detail says what row or
/// field failed. Carriers report the marker, they do not parse the detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BimError {
    pub marker: &'static str,
    pub detail: String,
}

impl BimError {
    pub fn new(marker: &'static str, detail: impl Into<String>) -> BimError {
        BimError {
            marker,
            detail: detail.into(),
        }
    }

    /// A row or block whose shape is not representable.
    pub fn invalid(detail: impl Into<String>) -> BimError {
        BimError::new(BIM_INVALID, detail)
    }

    /// A budget was exceeded; nothing was truncated.
    pub fn budget(detail: impl Into<String>) -> BimError {
        BimError::new(BIM_BUDGET_EXCEEDED, detail)
    }

    /// An element references a definition that is not in the table.
    pub fn dangling(detail: impl Into<String>) -> BimError {
        BimError::new(BIM_DANGLING_REFERENCE, detail)
    }
}

impl fmt::Display for BimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.marker, self.detail)
    }
}

impl std::error::Error for BimError {}

/// Check one string against the per-string budget and the non-empty rule.
fn check_text(what: &str, value: &str, allow_empty: bool) -> Result<(), BimError> {
    if !allow_empty && value.trim().is_empty() {
        return Err(BimError::invalid(format!("{what}: empty")));
    }
    if value.len() > BIM_MAX_STRING_BYTES {
        return Err(BimError::budget(format!(
            "{what}: {} bytes exceeds the {BIM_MAX_STRING_BYTES}-byte string budget",
            value.len()
        )));
    }
    Ok(())
}

fn check_opt_text(what: &str, value: Option<&String>) -> Result<(), BimError> {
    match value {
        Some(text) => check_text(what, text, false),
        None => Ok(()),
    }
}

fn check_source(what: &str, source: &BimSource) -> Result<(), BimError> {
    check_text(
        &format!("{what}.source.connector"),
        &source.connector,
        false,
    )?;
    check_opt_text(
        &format!("{what}.source.document_id"),
        source.document_id.as_ref(),
    )?;
    check_opt_text(&format!("{what}.source.id"), source.id.as_ref())?;
    check_opt_text(
        &format!("{what}.source.native_id"),
        source.native_id.as_ref(),
    )
}

fn check_psets(what: &str, psets: &[BimPropertySet]) -> Result<(), BimError> {
    if psets.len() > BIM_MAX_PSETS_PER_ELEMENT {
        return Err(BimError::budget(format!(
            "{what}: {} property sets exceeds the {BIM_MAX_PSETS_PER_ELEMENT} budget",
            psets.len()
        )));
    }
    for (set_index, set) in psets.iter().enumerate() {
        let path = format!("{what}.psets[{set_index}]");
        check_text(&format!("{path}.name"), &set.name, false)?;
        if set.properties.len() > BIM_MAX_PROPERTIES_PER_SET {
            return Err(BimError::budget(format!(
                "{path}: {} properties exceeds the {BIM_MAX_PROPERTIES_PER_SET} budget",
                set.properties.len()
            )));
        }
        for (property_index, property) in set.properties.iter().enumerate() {
            let property_path = format!("{path}[{property_index}]");
            check_text(&format!("{property_path}.name"), &property.name, false)?;
            check_value(&property_path, &property.value)?;
        }
    }
    Ok(())
}

fn check_value(what: &str, value: &BimValue) -> Result<(), BimError> {
    match value {
        // A text value may legitimately be empty; only its length is bounded.
        BimValue::Text(text) => check_text(&format!("{what}.value"), text, true),
        BimValue::Real {
            value,
            ifc_type,
            unit,
        } => {
            if !value.is_finite() {
                return Err(BimError::invalid(format!(
                    "{what}.value: a real property must be finite"
                )));
            }
            check_opt_text(&format!("{what}.ifc_type"), ifc_type.as_ref())?;
            check_opt_text(&format!("{what}.unit"), unit.as_ref())
        }
        BimValue::Integer(_) | BimValue::Bool(_) | BimValue::Null => Ok(()),
    }
}

/// Check one element's own shape and per-element budgets (no cross-references).
pub fn validate_element(element: &BimElement) -> Result<(), BimError> {
    check_opt_text("bim.category", element.category.as_ref())?;
    check_opt_text("bim.name", element.name.as_ref())?;
    check_source("bim", &element.source)?;
    check_opt_text("bim.level", element.level.as_ref())?;
    check_opt_text("bim.type", element.element_type.as_ref())?;
    check_opt_text("bim.material", element.material.as_ref())?;
    check_psets("bim", &element.psets)
}

/// Check one definition row's own shape and budgets (no cross-references).
pub fn validate_definition(row: &BimDefinition) -> Result<(), BimError> {
    let kind = row.kind();
    let id = row.id();
    check_text("definition.id", id, false)?;
    if !id.starts_with(kind.id_prefix()) || id.len() == kind.id_prefix().len() {
        return Err(BimError::invalid(format!(
            "definition.id: {id:?} does not name a {} definition (expected {}{{…}})",
            kind.as_str(),
            kind.id_prefix()
        )));
    }
    match row {
        BimDefinition::Level {
            name,
            elevation,
            source,
            ..
        } => {
            check_text("level.name", name, false)?;
            if let Some(elevation) = elevation {
                if !elevation.is_finite() {
                    return Err(BimError::invalid(
                        "level.elevation: a level elevation must be finite",
                    ));
                }
            }
            check_source("level", source)
        }
        BimDefinition::Type {
            name,
            class,
            category,
            psets,
            source,
            ..
        } => {
            check_text("type.name", name, false)?;
            check_opt_text("type.class", class.as_ref())?;
            check_opt_text("type.category", category.as_ref())?;
            check_psets("type", psets)?;
            check_source("type", source)
        }
        BimDefinition::Material { name, source, .. } => {
            check_text("material.name", name, false)?;
            check_source("material", source)
        }
    }
}

/// Check a whole document's BIM state: budgets, shapes and element references.
///
/// This is the builder's `finish()` gate. A reference to a missing definition
/// (or to one of the wrong kind) is [`BIM_DANGLING_REFERENCE`]: the element would
/// otherwise name a level that does not exist, which no reader can repair.
pub fn validate(
    definitions: &BimDefinitions,
    elements: &BTreeMap<EntityId, BimElement>,
) -> Result<(), BimError> {
    if definitions.len() > BIM_MAX_DEFINITIONS {
        return Err(BimError::budget(format!(
            "{} definition rows exceeds the {BIM_MAX_DEFINITIONS} budget",
            definitions.len()
        )));
    }
    for row in definitions.iter() {
        validate_definition(row)?;
    }
    let encoded = crate::snapshot::bim::definitions_encoded_len(definitions)
        .map_err(|error| BimError::invalid(error.detail))?;
    if encoded > BIM_MAX_DEFINITION_BYTES {
        return Err(BimError::budget(format!(
            "the definition table encodes to {encoded} bytes, over the \
             {BIM_MAX_DEFINITION_BYTES}-byte budget"
        )));
    }
    for (entity, element) in elements {
        validate_element(element)?;
        for (field, reference, expected) in [
            ("level", element.level.as_deref(), BimDefinitionKind::Level),
            (
                "type",
                element.element_type.as_deref(),
                BimDefinitionKind::Type,
            ),
            (
                "material",
                element.material.as_deref(),
                BimDefinitionKind::Material,
            ),
        ] {
            let Some(reference) = reference else {
                continue;
            };
            match definitions.get(reference) {
                Some(row) if row.kind() == expected => {}
                Some(row) => {
                    return Err(BimError::dangling(format!(
                        "entity {} {field} {reference:?} names a {} definition, not a {}",
                        entity.0,
                        row.kind().as_str(),
                        expected.as_str()
                    )))
                }
                None => {
                    return Err(BimError::dangling(format!(
                        "entity {} {field} {reference:?} is not in tables/bim-definitions.json",
                        entity.0
                    )))
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Builder path.
//
// These two methods live here, not in `builder.rs`, because that file is at the
// 1000-line gate (ADR-0042 risk 6): the builder exposes its `database`/`errors`
// fields crate-wide and this module owns the BIM side of the import contract.
// ---------------------------------------------------------------------------

impl crate::DrawingDatabaseBuilder {
    /// Record one element's BIM data (ADR-0042 D2).
    ///
    /// The entity must already have been inserted (mirroring
    /// [`Self::set_entity_render_attributes`]): BIM data for an id no element row
    /// owns could never be written, and dropping it would be a silent loss.
    ///
    /// Cross-references (`level`/`type`/`material`) are checked at
    /// [`Self::finish`], so an importer may set elements before or after the
    /// definition table; only the finished document must be consistent.
    pub fn set_entity_bim(
        &mut self,
        id: cad_domain::EntityId,
        bim: BimElement,
    ) -> cad_domain::CadResult<()> {
        if !self.database.entities.contains_key(&id) {
            return Err(cad_domain::CadError::InvalidInput(format!(
                "{BIM_INVALID}: bim for unknown entity {id:?}"
            )));
        }
        validate_element(&bim).map_err(to_cad_error)?;
        self.database.bim_elements.insert(id, bim);
        Ok(())
    }

    /// Replace the shared BIM definition table (ADR-0042 D3).
    ///
    /// Every row is shape-checked here; the count/encoded-size budgets and the
    /// element references are checked once at [`Self::finish`], where the whole
    /// document exists. Nothing is truncated: an over-budget table is refused.
    pub fn set_bim_definitions(
        &mut self,
        definitions: BimDefinitions,
    ) -> cad_domain::CadResult<()> {
        for row in &definitions {
            validate_definition(row).map_err(to_cad_error)?;
        }
        self.database.bim_definitions = definitions;
        Ok(())
    }
}

fn to_cad_error(error: BimError) -> cad_domain::CadError {
    cad_domain::CadError::InvalidInput(error.to_string())
}

#[cfg(test)]
mod tests;
