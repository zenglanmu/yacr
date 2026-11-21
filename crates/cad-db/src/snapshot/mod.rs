//! The pure-data view of a document that the persistence layer reads and writes
//! (ADR-0037 D1/D11).
//!
//! A [`DocumentSnapshot`] is the only thing `cad-store-zip` ever sees: a header,
//! a revision, a list of element shards, a set of table rows and the document's
//! base-colour texture bytes (ADR-0038 D3). The store turns it into archive bytes
//! and back; it never touches [`DrawingDatabase`].
//!
//! The direction of the dependency matters:
//!
//! * the **store** owns bytes ⇄ `Value` (container layout, hashing, compression);
//! * **`cad-db`** owns `Value` ⇄ rows ([`DrawingDatabase::snapshot`] and
//!   [`DrawingDatabase::from_snapshot`]).
//!
//! So the row codecs in [`elements`] and [`tables`] stay crate-private while the
//! snapshot types are public. A store cannot invent a payload shape the database
//! does not understand, and the database cannot depend on the archive library.
//!
//! Row payloads are `serde_json::Value` on purpose: the archive is JSON (D11),
//! and keeping the boundary at JSON means neither side needs a second
//! serialization format. Field-level rules (the `$f64` escape, canonical key
//! order) live in `cad_domain::ymodel::json` / `payload` and are shared.

pub(crate) mod bim;
pub(crate) mod build;
pub(crate) mod elements;
pub(crate) mod resources;
pub(crate) mod tables;
pub(crate) mod variables;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use cad_domain::ymodel::{DocumentHeader, ModelKey, PersistentId, Shard};
use cad_domain::{BundledKind, EntityId, Revision};

use crate::scope::TableKind;

/// One element entry: `elements/<model>-<shard>.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct ShardSnapshot {
    /// The Model this shard belongs to.
    pub model: ModelKey,
    /// Shard index inside the Model (`seq / 2048`).
    pub shard: Shard,
    /// Element payloads, ordered by [`ElementSnapshot::id`].
    pub elements: Vec<ElementSnapshot>,
}

/// One element row as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementSnapshot {
    /// The element's identity, also present inside `payload`.
    ///
    /// Kept outside the payload so the store can sort and shard entries without
    /// interpreting them.
    pub id: EntityId,
    /// The `cad:Element` envelope plus its geometry, already encoded.
    pub payload: serde_json::Value,
}

/// Model metadata (`tables/models.json`).
///
/// Models are derived from the in-memory spaces rather than allocated (ADR-0037
/// D7): model space, one per paper layout, one per block, plus the document-level
/// definition model. The record exists so a reader can rebuild an empty model and
/// recover its name without scanning every element shard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRecord {
    pub key: ModelKey,
    pub kind: ModelKind,
    /// The layout/block name, when there is one.
    pub name: Option<String>,
    /// The element that owns this model (`ModelModelsElement`), when there is one.
    pub owner: Option<PersistentId>,
}

/// What a [`ModelKey`] denotes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModelKind {
    /// `cad:ModelSpace`; may hold 2D and 3D elements at once (D4 relaxation).
    ModelSpace,
    /// `cad:PaperSpace`; 2D only.
    PaperSpace,
    /// `cad:BlockModel`; the members of one block definition.
    Block,
    /// The document-level `cad:DefinitionModel`.
    Definitions,
}

impl ModelKind {
    /// The persisted spelling. Part of the format.
    pub const fn as_str(self) -> &'static str {
        match self {
            ModelKind::ModelSpace => "model",
            ModelKind::PaperSpace => "paper",
            ModelKind::Block => "block",
            ModelKind::Definitions => "definitions",
        }
    }

    /// Parse the persisted spelling.
    ///
    /// Not `FromStr`: the format's spellings are a closed set and callers must
    /// treat an unknown one as a malformed document, not as an invalid input.
    pub fn parse_name(value: &str) -> Option<ModelKind> {
        match value {
            "model" => Some(ModelKind::ModelSpace),
            "paper" => Some(ModelKind::PaperSpace),
            "block" => Some(ModelKind::Block),
            "definitions" => Some(ModelKind::Definitions),
            _ => None,
        }
    }
}

/// Table rows, keyed by the closed [`TableKind`] set.
///
/// A table that is absent was not written; an empty vector is a table that exists
/// with no rows. The distinction matters on rewrite: absent means "not this
/// document has no such table yet" only for tables that never existed, so the
/// store writes every present key and never invents one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableSnapshot {
    pub rows: BTreeMap<TableKind, Vec<serde_json::Value>>,
}

impl TableSnapshot {
    /// Append one row payload.
    pub fn push(&mut self, kind: TableKind, row: serde_json::Value) {
        self.rows.entry(kind).or_default().push(row);
    }

    /// Replace every row of one table.
    pub fn set(&mut self, kind: TableKind, rows: Vec<serde_json::Value>) {
        self.rows.insert(kind, rows);
    }

    /// The rows of one table, if the table is present.
    pub fn get(&self, kind: TableKind) -> Option<&[serde_json::Value]> {
        self.rows.get(&kind).map(Vec::as_slice)
    }
}

/// The archive entry that carries one table.
///
/// The mapping lives here rather than on [`TableKind`] so the dirty-scope type
/// stays free of file-layout knowledge; adding a table still requires touching
/// both, and the store's dispatch is exhaustive over [`TableKind`].
pub const fn table_entry(kind: TableKind) -> &'static str {
    match kind {
        TableKind::Layers => "tables/layers.json",
        TableKind::Layouts => "tables/layouts.json",
        TableKind::Styles => "tables/styles.json",
        TableKind::DimStyles => "tables/dimstyles.json",
        TableKind::MLeaderStyles => "tables/mleaderstyles.json",
        TableKind::Linetypes => "tables/linetypes.json",
        TableKind::Scales => "tables/scales.json",
        TableKind::PlotSettings => "tables/plot-settings.json",
        TableKind::ExternalReferences => "tables/xrefs.json",
        TableKind::Blocks => "tables/blocks.json",
        // The logical name ⇄ archive-key mapping (ADR-0040 D4). A plain row
        // array like every other table; the bytes are separate
        // `resources/<kind>/<key>` entries.
        TableKind::Resources => "tables/resources.json",
        // Host-owned document metadata (ADR-0037 D11): written from the
        // `Document` slot, never encoded from the database itself.
        TableKind::Sources => "tables/sources.json",
        TableKind::Compat => "tables/compat.json",
        // Shared BIM definitions (ADR-0042 D3). Written only when the table
        // holds rows, so a document without BIM data stays at minor 4/5.
        TableKind::BimDefinitions => "tables/bim-definitions.json",
    }
}

/// The archive entry carrying model metadata.
pub const MODELS_ENTRY: &str = "tables/models.json";

/// The archive entry carrying the document header.
pub const DOCUMENT_ENTRY: &str = "document.json";

/// The archive entry carrying the class registry.
pub const CLASSES_ENTRY: &str = "schema/classes.json";

/// A document's complete persisted state.
///
/// One snapshot is one save: it is built from a `&DrawingDatabase` (plus the
/// dirty scope, which decides what may be copied rather than rewritten) and it is
/// consumed to hydrate a new database on open.
#[derive(Debug, Clone)]
pub struct DocumentSnapshot {
    pub header: DocumentHeader,
    /// The revision this snapshot represents. On save this is
    /// `manifest.saved_revision`; on load it seeds `DrawingDatabase::revision`
    /// (ADR-0037 D2 — without this an edited-then-reopened document would satisfy
    /// the L0 passthrough predicate and export divergent content as "original").
    pub revision: Revision,
    pub models: Vec<ModelRecord>,
    pub shards: Vec<ShardSnapshot>,
    pub tables: TableSnapshot,
    /// Document-owned base-colour texture bytes, keyed by
    /// [`cad_domain::is_valid_texture_key`]'s canonical form (ADR-0038 D3/D4).
    ///
    /// The store writes one `textures/<key>` binary entry per item and reads them
    /// back here; the snapshot is the authority for what exists, so a key this map
    /// does not hold is never written (no orphan assets in the archive). Empty for
    /// every document without textures, which is every document this build wrote
    /// before minor 3.
    pub textures: BTreeMap<String, Arc<[u8]>>,
    /// Document-bundled external resource bytes (fonts, xref sources and
    /// `IMAGE` rasters), keyed by `(kind, key)` (ADR-0040 D5).
    ///
    /// The store writes one `resources/<kind>/<key>` binary entry per item and
    /// reads them back here; `tables/resources.json` carries the logical names
    /// that point at these keys. Empty for every document that packs nothing,
    /// which is every document written before minor 5.
    pub resources: BTreeMap<(BundledKind, String), Arc<[u8]>>,
    /// The logical name each bundled resource is referenced by (ADR-0040 D4).
    ///
    /// Keyed like [`Self::resources`] so the mapping and the payloads cannot be
    /// reordered independently; the name is what the entity/style/xref row
    /// actually spells.
    pub resource_names: BTreeMap<(BundledKind, String), String>,
    /// The collection-time source path of each bundled resource, when one was
    /// recorded (ADR-0040 D4: written as `source`, diagnostics only).
    ///
    /// Preserved rather than dropped so a save-after-open does not silently
    /// rewrite a file with the provenance removed. `None` is "the writer knew no
    /// path", which the table codec writes as `source: null`.
    pub resource_sources: BTreeMap<(BundledKind, String), Option<String>>,
}

impl PartialEq for DocumentSnapshot {
    /// Equality that does **not** compare the binary payloads byte for byte.
    ///
    /// `textures` and `resources` carry up to tens of MiB each, and the fixed
    /// point test (`snapshot → database → snapshot`) compares whole snapshots:
    /// a full memcmp there answers a question nobody asked. The key set plus each
    /// payload's length are compared instead, so an added, dropped or resized
    /// asset still makes two snapshots unequal. A caller that really means
    /// "identical bytes" calls [`DocumentSnapshot::bytes_equal`].
    fn eq(&self, other: &Self) -> bool {
        self.header == other.header
            && self.revision == other.revision
            && self.models == other.models
            && self.shards == other.shards
            && self.tables == other.tables
            && binary_keys_and_lengths_match(&self.textures, &other.textures)
            && binary_keys_and_lengths_match(&self.resources, &other.resources)
            && self.resource_names == other.resource_names
            && self.resource_sources == other.resource_sources
    }
}

/// Same keys, same payload **lengths**; contents are not compared (see
/// [`DocumentSnapshot`]'s `PartialEq`).
fn binary_keys_and_lengths_match<K: Ord>(
    left: &BTreeMap<K, Arc<[u8]>>,
    right: &BTreeMap<K, Arc<[u8]>>,
) -> bool {
    left.len() == right.len()
        && left.iter().all(|(key, bytes)| {
            right
                .get(key)
                .is_some_and(|other| other.len() == bytes.len())
        })
}

/// Same keys and identical payload bytes.
fn binary_contents_match<K: Ord>(
    left: &BTreeMap<K, Arc<[u8]>>,
    right: &BTreeMap<K, Arc<[u8]>>,
) -> bool {
    left.len() == right.len()
        && left.iter().all(|(key, bytes)| {
            right
                .get(key)
                .is_some_and(|other| other.as_ref() == bytes.as_ref())
        })
}

impl DocumentSnapshot {
    /// Whether both snapshots carry byte-identical textures and resources.
    ///
    /// `PartialEq` deliberately compares only keys and lengths (see above); a
    /// round-trip test that wants to prove the bytes survived calls this.
    pub fn bytes_equal(&self, other: &DocumentSnapshot) -> bool {
        binary_contents_match(&self.textures, &other.textures)
            && binary_contents_match(&self.resources, &other.resources)
    }

    /// Whether this document carries BIM data (ADR-0042 D6).
    ///
    /// The store needs this to pick `manifest.format.minor` but must not know
    /// the `bim` element field or the definition table by name (D6: `has_bim`
    /// lives here, not in `cad-store-zip`). It is true when the definition table
    /// is present **or** any element payload carries a `bim` block — an element
    /// can hold source/category/psets without referencing a definition, and
    /// writing that document at minor 5 would let an older build refuse it for
    /// a reason that no longer describes its content.
    ///
    /// The element scan reads the payload key the element codec writes; it is
    /// the one place outside the codec that names it, and it is still inside
    /// `cad-db` where the shape is defined.
    pub fn has_bim(&self) -> bool {
        self.tables.get(TableKind::BimDefinitions).is_some()
            || self
                .shards
                .iter()
                .flat_map(|shard| shard.elements.iter())
                .any(|element| element.payload.get("bim").is_some())
    }
}

/// Why a snapshot could not be built or hydrated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotError {
    /// Stable marker (see `cad_domain::ymodel::markers`).
    pub marker: &'static str,
    /// What specifically failed.
    pub detail: String,
}

impl SnapshotError {
    pub fn new(marker: &'static str, detail: impl Into<String>) -> SnapshotError {
        SnapshotError {
            marker,
            detail: detail.into(),
        }
    }

    /// A row that could not be encoded or decoded.
    pub fn invalid(detail: impl Into<String>) -> SnapshotError {
        SnapshotError::new(cad_domain::ymodel::markers::INVALID_DOCUMENT, detail)
    }
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.marker, self.detail)
    }
}

impl std::error::Error for SnapshotError {}
