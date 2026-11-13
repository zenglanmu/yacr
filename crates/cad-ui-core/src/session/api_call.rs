//! The plugin-API **call** surface: reflected reads, served on both carriers.
//!
//! # What this is
//!
//! [`API_CALL_COMMAND`] (`cad_api_call`) is the plugin API *performing*, not
//! merely advertising. This wave serves five reflected reads from one snapshot:
//!
//! | namespace.operation | answer |
//! |---|---|
//! | `ApplicationServices.documentManager.mdiActiveDocument` | `{ "document": … or null }` |
//! | `ApplicationServices.documentManager.documents` | `{ "documents": [ … ] }` (0 or 1) |
//! | `DatabaseServices.database.revision` (`{ "database": "<id>" }`) | `{ "revision": <number> }` |
//! | `DatabaseServices.database.symbolTables` (`{ "database": "<id>" }`) | `{ "blockTable": …, "layerTable": …, "clayer": … }` |
//! | `EditorInput.editor.currentSelection` | `{ "objectIds": [ "<EntityId>", … ] }` |
//!
//! Every read is **derived**: the authoritative document stays `cad-db`'s, and
//! each answer is a projection of the host controller's own fields
//! ([`cad_app::PluginDocument`], its `SelectionSet`). Nothing here opens a
//! transaction, mutates the drawing or writes the mirror back.
//!
//! `editor.currentSelection` has no parameters. Its `objectIds` are the current
//! selection's `EntityId`s in selection order, as decimal strings. A selection
//! ref also carries an `InstancePath` and an optional `SubElementId`, but this
//! read can only name the entity: every selected ref is included (never dropped
//! silently) and repeated ids are deduplicated, first occurrence first. Instance
//! and sub-element identity is therefore **not representable** in this read yet —
//! the list is the distinct `EntityId`s, not the distinct refs.
//!
//! The descriptor rows in `api/plugin-api.json` are **not** updated in this wave
//! (they stay `pending`); the wire answer above is what this build performs.
//!
//! # Refusals are explicit
//!
//! * an unknown/unimplemented operation is
//!   `unsupported-runtime: cad_api.<namespace>.<operation>` — the same
//!   machine-readable marker shape the rest of the wire uses for "this carrier
//!   does not perform that". It is checked **first**, before any snapshot or
//!   argument check;
//! * **no snapshot yet** (the desktop owner-tick mirror has not been populated)
//!   is [`CAD_WAITING`], the shared "ask again after a tick" word. A snapshot
//!   that exists but holds no document is *not* waiting: the document reads
//!   answer their real "nothing open" value (`{ "document": null }`,
//!   `{ "documents": [] }`) and the document-scoped reads (`revision`,
//!   `symbolTables`, `editor.currentSelection`) refuse
//!   [`API_CALL_NO_DOCUMENT`];
//! * a database request (`revision`, `symbolTables`) whose `parameters.database`
//!   is missing, not a string, or not the snapshot's database id is
//!   [`API_CALL_INVALID_ARGUMENT_DATABASE`].
//!
//! An empty namespace or operation is simply an operation that is not served, so
//! it takes the first refusal. It is **never** answered as an empty success.
//!
//! # The wire shapes
//!
//! ```json
//! { "document": { "id": "<DocumentId>", "name": "<label>", "isReadOnly": false,
//!                 "database": { "id": "<DatabaseId>", "version": "<revision>" } } }
//! { "documents": [ <the same descriptor> ] }
//! { "revision": <number> }
//! { "blockTable": { "name": "blocks", "records": [ { "name": "…", "isLayout": false,
//!                    "origin": [x, y, z] } ] },
//!   "layerTable": { "name": "layers", "records": [ { "name": "0",
//!                    "color": { "aci": 7, "rgb": "#ffffff" }, "visible": true,
//!                    "locked": false, "frozen": false, "linetype": "Continuous" } ] },
//!   "clayer": "0" }
//! { "objectIds": [ "12", "13" ] }
//! ```
//!
//! `mdiActiveDocument` is `{ "document": null }` when the document manager exists
//! but holds no document; `documents` is then `{ "documents": [] }`. An empty
//! selection is `{ "objectIds": [] }` — a fact, not a refusal. The reply
//! envelope is [`CadApiOutcome`]: `ok` plus exactly one of `value`/`error` (the
//! absent half is omitted, never `null`).

use serde::Serialize;
use serde_json::{json, Value};

use super::wire::CAD_WAITING;

/// The wire command name that performs one plugin-API call.
pub const API_CALL_COMMAND: &str = "cad_api_call";

/// The stable refusal prefix for an operation this build does not perform.
///
/// The full marker is [`API_CALL_UNKNOWN`] + `<namespace>.<operation>`, built by
/// [`api_call_unknown`].
pub const API_CALL_UNKNOWN: &str = "unsupported-runtime: cad_api.";

/// The `ApplicationServices` namespace (documents, the MDI document manager).
pub const API_CALL_NAMESPACE: &str = "ApplicationServices";

/// The `DatabaseServices` namespace (database-level reads).
pub const API_CALL_DATABASE_NAMESPACE: &str = "DatabaseServices";

/// The `EditorInput` namespace (the current selection and editor prompts).
pub const API_CALL_EDITOR_NAMESPACE: &str = "EditorInput";

/// The reflected read of the active document.
pub const API_CALL_MDI_ACTIVE_DOCUMENT: &str = "documentManager.mdiActiveDocument";

/// The reflected read of every open document (single-document host: 0 or 1).
pub const API_CALL_DOCUMENTS: &str = "documentManager.documents";

/// The reflected read of the open database's revision.
pub const API_CALL_REVISION: &str = "database.revision";

/// The reflected read of the open database's block and layer symbol tables.
pub const API_CALL_SYMBOL_TABLES: &str = "database.symbolTables";

/// The reflected read of the current selection's entity ids (no parameters).
pub const API_CALL_CURRENT_SELECTION: &str = "editor.currentSelection";

/// Stable refusal of a database read while the session holds no document.
///
/// Distinct from [`CAD_WAITING`]: the snapshot exists and says there is no
/// document, so asking again after a tick changes nothing until a document opens.
pub const API_CALL_NO_DOCUMENT: &str = "cad_api.no_document";

/// Stable refusal of a served operation whose `parameters` are unusable.
pub const API_CALL_INVALID_ARGUMENT_DATABASE: &str = "api.invalid_argument: database";

/// Stable refusal of a snapshot whose revision is not a decimal number.
///
/// This build always renders [`CadApiDocument::database_version`] from the
/// database's revision counter, so it is an internal invariant; a violated one is
/// this named refusal rather than a fabricated number.
const API_CALL_INVALID_SNAPSHOT: &str = "cad_api.invalid_snapshot: databaseVersion";

/// Stable refusal of a snapshot that claims a document but carries no symbol-table
/// projection (an inconsistent snapshot, only reachable from a hand-built value).
///
/// [`CadSession::api_snapshot`] builds both halves together, so this names a
/// violated invariant instead of fabricating empty tables.
const API_CALL_INCOMPLETE_SNAPSHOT: &str = "cad_api.incomplete_snapshot: symbolTables";

/// The stable `unsupported-runtime` marker for an unserved API call.
pub fn api_call_unknown(namespace: &str, operation: &str) -> String {
    format!("{API_CALL_UNKNOWN}{namespace}.{operation}")
}

/// One queued plugin-API **mutation** request (`cad_api_submit`).
///
/// The wire carries `requestId`; the submission is owned by whichever queue
/// answers it (the desktop's owner tick) or by the session that applies it
/// synchronously (`dom-canvas`). The outcome is polled through
/// [`CadApiResults`] — this type never carries an answer.
#[derive(Debug, Clone, PartialEq)]
pub struct CadApiSubmission {
    /// The caller's correlation id (see [`validate_request_id`]).
    pub request_id: String,
    /// The API namespace, e.g. `DatabaseServices`.
    pub namespace: String,
    /// The operation inside the namespace, e.g. `transaction.commit`.
    pub operation: String,
    /// The operation's argument object (`None` when the caller sent none).
    pub parameters: Option<Value>,
}

/// The `cad_api_poll` answer: every completed outcome, FIFO.
///
/// An empty list is a valid answer (nothing completed since the last poll).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CadApiResults {
    /// The drained outcomes, oldest first.
    pub results: Vec<CadApiOutcome>,
}

/// Bound on queued submissions a desktop host holds before refusing.
pub const CAD_API_SUBMIT_CAP: usize = 16;

/// Bound on retained outcomes before the oldest is dropped.
///
/// Both carriers apply it (the desktop's shared result queue and the session's
/// own queue on a synchronous carrier), so an unpolled client cannot grow the
/// host without limit; the dropped answer is gone, never silently replayed.
pub const CAD_API_RESULT_CAP: usize = 32;

/// Stable refusal of a submission this carrier's queue cannot accept.
pub const API_SUBMIT_QUEUE_FULL: &str = "cad_api.queue_full";

/// Stable refusal of a submission whose `requestId` is still queued.
pub const API_SUBMIT_DUPLICATE: &str = "cad_api.duplicate_request";

/// Stable refusal of an unusable `requestId`.
pub const API_SUBMIT_INVALID_REQUEST: &str = "cad_api.invalid_request: requestId";

/// The longest accepted request id.
const REQUEST_ID_MAX_LEN: usize = 64;

/// The one mutating operation this build performs.
pub const API_CALL_TRANSACTION_COMMIT: &str = "transaction.commit";

/// Stable refusal of a commit whose `expectedRevision` is not the open revision.
pub const API_CALL_REVISION_CONFLICT: &str = "api.revision-conflict";

/// The stable `api.invalid_argument: <field>` marker.
pub fn api_call_invalid_argument(field: &str) -> String {
    format!("api.invalid_argument: {field}")
}

/// The stable `cad_api.error: <text>` marker for a host-side commit failure.
pub fn api_call_error(text: impl std::fmt::Display) -> String {
    format!("cad_api.error: {text}")
}

/// Whether a submission id is usable: 1..=64 characters from `[A-Za-z0-9._-]`.
///
/// The grammar is deliberately narrow: the id travels through an IPC envelope,
/// a log line and this crate's own result queue, and a client that reuses an id
/// must be able to recognise its answer exactly. Anything else is refused by
/// name ([`API_SUBMIT_INVALID_REQUEST`]) rather than normalized.
pub fn validate_request_id(request_id: &str) -> Result<(), String> {
    let len = request_id.len();
    if len == 0 || len > REQUEST_ID_MAX_LEN {
        return Err(API_SUBMIT_INVALID_REQUEST.to_string());
    }
    if !request_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(API_SUBMIT_INVALID_REQUEST.to_string());
    }
    Ok(())
}

/// One document as the plugin API reports it (flat, host-side projection).
///
/// Its [`Serialize`] implementation is the **nested** wire object
/// (`{"id", "name", "isReadOnly", "database": {"id", "version"}}`), so the JSON
/// contract lives in exactly one place: [`CadApiSnapshot`] serializes as
/// `{"document": <that object>|null}` and [`api_call_document`] composes the
/// same value into the reply envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CadApiDocument {
    /// The open document's id, in decimal.
    pub id: String,
    /// The session's own label for the document (never a synthesized path).
    pub name: String,
    /// Whether the session is in read-only (`Viewer`) mode.
    pub is_read_only: bool,
    /// The open database's id, in decimal.
    pub database_id: String,
    /// The open database's revision, in decimal.
    pub database_version: String,
}

impl CadApiDocument {
    /// The exact `mdiActiveDocument` JSON object (the database is nested).
    pub fn wire_value(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "isReadOnly": self.is_read_only,
            "database": {
                "id": self.database_id,
                "version": self.database_version,
            },
        })
    }
}

impl Serialize for CadApiDocument {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.wire_value().serialize(serializer)
    }
}

impl From<cad_app::PluginDocument> for CadApiDocument {
    fn from(document: cad_app::PluginDocument) -> Self {
        Self {
            id: document.id,
            name: document.name,
            is_read_only: document.is_read_only,
            database_id: document.database_id,
            database_version: document.database_version,
        }
    }
}

/// The **reflected snapshot** one host answers `cad_api_call` from.
///
/// This is the type that separates the two states a bare `Option<CadApiDocument>`
/// used to conflate:
///
/// * the host has **no snapshot yet** — the whole value is `None` at the read
///   seam ([`super::wire::ControlReads::api_snapshot`]), which is
///   [`CAD_WAITING`];
/// * the host **has a snapshot** — `Some(CadApiSnapshot { document: ... })`.
///   `document: None` is then a *fact about the drawing* ("the document manager
///   exists but no document is open") and is answered `{ "document": null }`,
///   never as "ask again later".
///
/// The wire shape of the document half is exactly `{"document": ...}`; the
/// `Option` serializes `None` as JSON `null`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CadApiSnapshot {
    /// The open document, or `None` when the manager holds no document.
    pub document: Option<CadApiDocument>,
    /// The document's symbol-table projection, or the projection's own refusal.
    ///
    /// `None` exactly when [`Self::document`] is `None`: the session always builds
    /// both halves together ([`super::CadSession::api_snapshot`]). `Some(Err(..))` relays
    /// the layer projection's real refusal instead of serving invented rows.
    ///
    /// Not part of the serialized snapshot: the document envelope is built by
    /// [`api_call_document`], and this projection is served by
    /// `DatabaseServices.database.symbolTables`.
    #[serde(skip)]
    pub symbol_tables: Option<Result<CadApiSymbolTables, String>>,
    /// Every selected ref's `EntityId`, in selection order, as decimal strings.
    ///
    /// One entry per selected **ref**, so an entity selected through two distinct
    /// `InstancePath`s (or two sub-elements) appears twice here; the
    /// `EditorInput.editor.currentSelection` read deduplicates by id, first
    /// occurrence first ([`api_call_selection`]), because instance and sub-element
    /// identity is not representable in that answer yet.
    ///
    /// Empty is a real fact (nothing selected), not "waiting", and it says nothing
    /// about whether a document is open. Not part of the serialized document
    /// envelope; served by [`API_CALL_CURRENT_SELECTION`].
    #[serde(skip)]
    pub selection: Vec<String>,
}

/// One named block definition, as the symbol-table read reports it.
///
/// Only blocks the database carries **metadata** for are listed
/// ([`cad_db::DrawingDatabase::block_metadata_entries`]); an unnamed block has no
/// honest name or base point, so it is never invented here.
#[derive(Debug, Clone, PartialEq)]
pub struct CadApiBlockRecord {
    /// The block table name.
    pub name: String,
    /// The block's insertion base point, in block space.
    pub origin: [f64; 3],
}

/// One layer-table row: the stored attributes plus the current visibility.
#[derive(Debug, Clone, PartialEq)]
pub struct CadApiLayerRecord {
    /// The layer name.
    pub name: String,
    /// The source ACI index, or `None` for a true-colour layer (wire `null`).
    pub aci: Option<u8>,
    /// The resolved sRGB value, lowercase `#rrggbb`.
    pub rgb: String,
    /// The visibility the scene honours right now (database flags + overrides).
    pub visible: bool,
    /// The stored lock flag.
    pub locked: bool,
    /// The stored frozen flag.
    pub frozen: bool,
    /// The linetype table entry's name.
    pub linetype: String,
}

/// The symbol-table projection of the open document.
///
/// A derived read of `cad-db`: the block table comes from the database's named
/// block metadata and the layer table from the session's own layer projection
/// (the same rows `cad_layers` serves). Nothing is written back.
#[derive(Debug, Clone, PartialEq)]
pub struct CadApiSymbolTables {
    /// Every named block definition, sorted by name.
    pub blocks: Vec<CadApiBlockRecord>,
    /// Every layer row, in the layer-panel's own order.
    pub layers: Vec<CadApiLayerRecord>,
    /// The active layer's name (`""` when the projection reports none).
    pub clayer: String,
}

/// The reply envelope of one API call.
///
/// `value` and `error` are mutually exclusive and the absent half is omitted
/// from the JSON, so a caller reads exactly one of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CadApiOutcome {
    /// The submission id this answer belongs to, on the `cad_api_submit` lane.
    ///
    /// `None` for the synchronous read lane ([`API_CALL_COMMAND`]), so that
    /// answer's JSON is unchanged; every submit outcome carries the id its
    /// request supplied.
    #[serde(rename = "requestId", skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Whether the call was performed.
    pub ok: bool,
    /// The reflected answer, when `ok`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// The stable refusal marker, when not `ok`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl CadApiOutcome {
    /// A performed call with its reflected value.
    pub fn value(value: Value) -> Self {
        Self {
            request_id: None,
            ok: true,
            value: Some(value),
            error: None,
        }
    }

    /// A call this carrier does not perform, with its stable marker.
    pub fn refusal(error: impl Into<String>) -> Self {
        Self {
            request_id: None,
            ok: false,
            value: None,
            error: Some(error.into()),
        }
    }

    /// Attach the submission id this answer belongs to.
    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    /// The `cad_waiting` answer for one submitted request (no session yet).
    pub fn waiting(request_id: impl Into<String>) -> Self {
        Self::refusal(CAD_WAITING).with_request_id(request_id)
    }
}

/// Build the `mdiActiveDocument` answer for an optional document.
///
/// `Some(document)` is the reflected document; `None` is the **real** "no
/// document is open" answer (`{ "document": null }`) for a document manager that
/// exists without a document. Only a caller with **no snapshot at all** must
/// answer [`CAD_WAITING`] instead (see [`dispatch_api_call`]).
pub fn api_call_document(document: Option<CadApiDocument>) -> CadApiOutcome {
    match document {
        Some(document) => CadApiOutcome::value(json!({ "document": document.wire_value() })),
        None => CadApiOutcome::value(json!({ "document": null })),
    }
}

/// Build the `documentManager.documents` answer: 0 or 1 descriptors.
///
/// The descriptor is the **same object** `mdiActiveDocument` reports (one shape,
/// one projection). A session with no document answers the honest empty list.
pub fn api_call_documents(snapshot: &CadApiSnapshot) -> CadApiOutcome {
    let documents: Vec<Value> = snapshot
        .document
        .iter()
        .map(CadApiDocument::wire_value)
        .collect();
    CadApiOutcome::value(json!({ "documents": documents }))
}

/// Build the `EditorInput.editor.currentSelection` answer.
///
/// `objectIds` are the snapshot selection's entity ids, in selection order, as
/// decimal strings. Duplicates are removed by id (first occurrence wins) so a
/// selection holding the same entity through two `InstancePath`s or sub-elements
/// is still included — never dropped — but named once: this read has no field for
/// that identity yet.
///
/// An empty selection is `{ "objectIds": [] }` (a fact). A snapshot without a
/// document refuses [`API_CALL_NO_DOCUMENT`]: there is no drawing whose selection
/// could be reported. The read takes no parameters; any supplied are ignored.
pub fn api_call_selection(snapshot: &CadApiSnapshot) -> CadApiOutcome {
    if snapshot.document.is_none() {
        return CadApiOutcome::refusal(API_CALL_NO_DOCUMENT);
    }
    let mut object_ids: Vec<&str> = Vec::with_capacity(snapshot.selection.len());
    for id in &snapshot.selection {
        if !object_ids.contains(&id.as_str()) {
            object_ids.push(id.as_str());
        }
    }
    CadApiOutcome::value(json!({ "objectIds": object_ids }))
}

/// Build the `DatabaseServices.database.revision` answer.
///
/// The requested `parameters.database` must be the snapshot document's database
/// id (`api.invalid_argument: database` otherwise — missing, not a string, or a
/// different id). The revision is the decimal number the snapshot already carries
/// as `database.version`; the string is parsed, never guessed.
pub fn api_call_revision(snapshot: &CadApiSnapshot, parameters: Option<&Value>) -> CadApiOutcome {
    let Some(document) = snapshot.document.as_ref() else {
        return CadApiOutcome::refusal(API_CALL_NO_DOCUMENT);
    };
    if requested_database(parameters) != Some(document.database_id.as_str()) {
        return CadApiOutcome::refusal(API_CALL_INVALID_ARGUMENT_DATABASE);
    }
    match document.database_version.parse::<u64>() {
        Ok(revision) => CadApiOutcome::value(json!({ "revision": revision })),
        Err(_) => CadApiOutcome::refusal(API_CALL_INVALID_SNAPSHOT),
    }
}

/// The `parameters.database` selector, when it is a string.
fn requested_database(parameters: Option<&Value>) -> Option<&str> {
    parameters
        .and_then(|parameters| parameters.get("database"))
        .and_then(Value::as_str)
}

/// Build the `DatabaseServices.database.symbolTables` answer.
///
/// Same argument rule as [`api_call_revision`] (`parameters.database` must be the
/// snapshot document's database id). The block table lists the database's named
/// block definitions (`isLayout` is always `false`: paper/model spaces are not
/// blocks in `cad-db`), the layer table lists the same rows `cad_layers` serves,
/// and `clayer` is the projection's active layer. A failed layer projection is
/// relayed as its own refusal rather than replaced by empty tables.
pub fn api_call_symbol_tables(
    snapshot: &CadApiSnapshot,
    parameters: Option<&Value>,
) -> CadApiOutcome {
    let Some(document) = snapshot.document.as_ref() else {
        return CadApiOutcome::refusal(API_CALL_NO_DOCUMENT);
    };
    if requested_database(parameters) != Some(document.database_id.as_str()) {
        return CadApiOutcome::refusal(API_CALL_INVALID_ARGUMENT_DATABASE);
    }
    match snapshot.symbol_tables.as_ref() {
        Some(Ok(tables)) => CadApiOutcome::value(symbol_tables_value(tables)),
        Some(Err(reason)) => CadApiOutcome::refusal(reason.clone()),
        None => CadApiOutcome::refusal(API_CALL_INCOMPLETE_SNAPSHOT),
    }
}

/// The frozen `symbolTables` JSON object.
fn symbol_tables_value(tables: &CadApiSymbolTables) -> Value {
    let blocks: Vec<Value> = tables
        .blocks
        .iter()
        .map(|record| {
            json!({
                "name": record.name,
                // `cad-db` spaces are not blocks: a metadata block is never a layout.
                "isLayout": false,
                "origin": record.origin,
            })
        })
        .collect();
    let layers: Vec<Value> = tables
        .layers
        .iter()
        .map(|record| {
            json!({
                "name": record.name,
                "color": { "aci": record.aci, "rgb": record.rgb },
                "visible": record.visible,
                "locked": record.locked,
                "frozen": record.frozen,
                "linetype": record.linetype,
            })
        })
        .collect();
    json!({
        "blockTable": { "name": "blocks", "records": blocks },
        "layerTable": { "name": "layers", "records": layers },
        "clayer": tables.clayer,
    })
}

/// The operations this build serves, resolved from `namespace.operation`.
enum ServedOperation {
    /// `ApplicationServices.documentManager.mdiActiveDocument`
    ActiveDocument,
    /// `ApplicationServices.documentManager.documents`
    Documents,
    /// `DatabaseServices.database.revision`
    Revision,
    /// `DatabaseServices.database.symbolTables`
    SymbolTables,
    /// `EditorInput.editor.currentSelection`
    CurrentSelection,
}

/// Resolve a namespaced operation, or `None` when this build does not serve it.
fn served_operation(namespace: &str, operation: &str) -> Option<ServedOperation> {
    match (namespace, operation) {
        (API_CALL_NAMESPACE, API_CALL_MDI_ACTIVE_DOCUMENT) => Some(ServedOperation::ActiveDocument),
        (API_CALL_NAMESPACE, API_CALL_DOCUMENTS) => Some(ServedOperation::Documents),
        (API_CALL_DATABASE_NAMESPACE, API_CALL_REVISION) => Some(ServedOperation::Revision),
        (API_CALL_DATABASE_NAMESPACE, API_CALL_SYMBOL_TABLES) => {
            Some(ServedOperation::SymbolTables)
        }
        (API_CALL_EDITOR_NAMESPACE, API_CALL_CURRENT_SELECTION) => {
            Some(ServedOperation::CurrentSelection)
        }
        _ => None,
    }
}

/// Serve one plugin-API call from an optional reflected snapshot.
///
/// `snapshot == None` means the host has **no snapshot yet** (the desktop mirror
/// before the first owner tick), so every served operation answers
/// [`CAD_WAITING`] — never a claim about the drawing. `Some(snapshot)` is
/// answered as a matter of fact, including `snapshot.document == None`:
/// `mdiActiveDocument` → `{ "document": null }`, `documents` →
/// `{ "documents": [] }`, and the document-scoped reads (`revision`,
/// `symbolTables`, `editor.currentSelection`) → [`API_CALL_NO_DOCUMENT`].
///
/// `parameters` carries the operation's arguments (the database reads require
/// `{ "database": "<id>" }`). The document and selection reads ignore them, so a
/// caller that sends none answers exactly as before.
///
/// An operation outside the served set is the stable `unsupported-runtime`
/// refusal **first** — before any snapshot or argument check — because "ask again
/// later" or "bad argument" would both be false promises about an operation this
/// build never performs.
pub fn dispatch_api_call(
    snapshot: Option<&CadApiSnapshot>,
    namespace: &str,
    operation: &str,
    parameters: Option<&Value>,
) -> CadApiOutcome {
    let Some(served) = served_operation(namespace, operation) else {
        return CadApiOutcome::refusal(api_call_unknown(namespace, operation));
    };
    let Some(snapshot) = snapshot else {
        return CadApiOutcome::refusal(CAD_WAITING);
    };
    match served {
        ServedOperation::ActiveDocument => api_call_document(snapshot.document.clone()),
        ServedOperation::Documents => api_call_documents(snapshot),
        ServedOperation::Revision => api_call_revision(snapshot, parameters),
        ServedOperation::SymbolTables => api_call_symbol_tables(snapshot, parameters),
        ServedOperation::CurrentSelection => api_call_selection(snapshot),
    }
}

impl super::CadSession {
    /// The reflected plugin-API snapshot of this session's document slot.
    ///
    /// Always answers: the session *is* the document manager, so the snapshot
    /// exists even when no document is open (`document: None`, a fact, not
    /// "waiting"). Only a host with no session at all has no snapshot. The value
    /// is a read-only projection of the host controller — this never opens a
    /// transaction and never touches `cad-db`'s authority.
    pub fn api_snapshot(&self) -> CadApiSnapshot {
        let (document, selection) = {
            let controller = self.core.controller.borrow();
            let document = controller.plugin_document().map(CadApiDocument::from);
            // One decimal entry per selected ref, in selection order. Instance and
            // sub-element identity stays behind: the read dedupes by entity id.
            let selection = controller
                .selection()
                .refs()
                .iter()
                .map(|reference| reference.entity.0.to_string())
                .collect();
            (document, selection)
        };
        // The two halves are built together: no document ⇒ no symbol tables, so
        // the `database.symbolTables` read can tell "nothing open" from "the
        // projection failed" (which is relayed, never replaced by empty tables).
        let symbol_tables = if document.is_some() {
            Some(self.api_symbol_tables())
        } else {
            None
        };
        CadApiSnapshot {
            document,
            symbol_tables,
            selection,
        }
    }

    /// The symbol-table projection of the open document (derived, read-only).
    ///
    /// Blocks come from the database's own named-block metadata (sorted by name),
    /// layers from the session's layer projection — the exact rows `cad_layers`
    /// serves — and `clayer` from that projection's active layer.
    fn api_symbol_tables(&self) -> Result<CadApiSymbolTables, String> {
        let blocks = {
            let controller = self.core.controller.borrow();
            let Some(drawing) = controller.drawing() else {
                return Err(API_CALL_NO_DOCUMENT.to_string());
            };
            let mut blocks: Vec<CadApiBlockRecord> = drawing
                .block_metadata_entries()
                .map(|(_, metadata)| CadApiBlockRecord {
                    name: metadata.name.clone(),
                    origin: [
                        metadata.base_point.x,
                        metadata.base_point.y,
                        metadata.base_point.z,
                    ],
                })
                .collect();
            // Stable table order for the caller; the database itself is keyed by id.
            blocks.sort_by(|a, b| a.name.cmp(&b.name));
            blocks
        };
        let layers = self.layers()?;
        Ok(CadApiSymbolTables {
            blocks,
            layers: layers
                .layers
                .into_iter()
                .map(|layer| CadApiLayerRecord {
                    name: layer.name,
                    aci: layer.color.aci,
                    rgb: layer.color.rgb,
                    visible: layer.visible,
                    locked: layer.locked,
                    frozen: layer.frozen,
                    linetype: layer.linetype,
                })
                .collect(),
            // The projection reports no active layer only for an empty projection;
            // then the honest `clayer` is the empty string, never a invented name.
            clayer: layers.active.unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests;
