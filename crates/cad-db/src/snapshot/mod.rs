//! The pure-data view of a document that the persistence layer reads and writes
//! (ADR-0037 D1/D11).
//!
//! A [`DocumentSnapshot`] is the only thing `cad-store-zip` ever sees: a header,
//! a revision, a list of element shards and a set of table rows. The store turns
//! it into archive bytes and back; it never touches [`DrawingDatabase`].
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

pub(crate) mod build;
pub(crate) mod elements;
pub(crate) mod tables;
pub(crate) mod variables;

use std::collections::BTreeMap;
use std::fmt;

use cad_domain::ymodel::{DocumentHeader, ModelKey, PersistentId, Shard};
use cad_domain::{EntityId, Revision};

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
#[derive(Debug, Clone, PartialEq)]
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
