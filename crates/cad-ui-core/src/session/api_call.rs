//! The plugin-API **call** surface: reflected reads, served on both carriers.
//!
//! # What this is
//!
//! [`API_CALL_COMMAND`] (`cad_api_call`) is the plugin API *performing*, not
//! merely advertising. This wave serves three reflected reads from one snapshot:
//!
//! | namespace.operation | answer |
//! |---|---|
//! | `ApplicationServices.documentManager.mdiActiveDocument` | `{ "document": … or null }` |
//! | `ApplicationServices.documentManager.documents` | `{ "documents": [ … ] }` (0 or 1) |
//! | `DatabaseServices.database.revision` (`{ "database": "<id>" }`) | `{ "revision": <number> }` |
//!
//! Every read is **derived**: the authoritative document stays `cad-db`'s, and
//! each answer is a projection of the host controller's own fields
//! ([`cad_app::PluginDocument`]). Nothing here opens a transaction, mutates the
//! drawing or writes the mirror back.
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
//!   `{ "documents": [] }`) and the database read refuses
//!   [`API_CALL_NO_DOCUMENT`];
//! * a `database.revision` request whose `parameters.database` is missing, not a
//!   string, or not the snapshot's database id is
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
//! ```
//!
//! `mdiActiveDocument` is `{ "document": null }` when the document manager exists
//! but holds no document; `documents` is then `{ "documents": [] }`. The reply
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

/// The reflected read of the active document.
pub const API_CALL_MDI_ACTIVE_DOCUMENT: &str = "documentManager.mdiActiveDocument";

/// The reflected read of every open document (single-document host: 0 or 1).
pub const API_CALL_DOCUMENTS: &str = "documentManager.documents";

/// The reflected read of the open database's revision.
pub const API_CALL_REVISION: &str = "database.revision";

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

/// The stable `unsupported-runtime` marker for an unserved API call.
pub fn api_call_unknown(namespace: &str, operation: &str) -> String {
    format!("{API_CALL_UNKNOWN}{namespace}.{operation}")
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
/// The wire shape is exactly `{"document": ...}`; the `Option` serializes `None`
/// as JSON `null`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CadApiSnapshot {
    /// The open document, or `None` when the manager holds no document.
    pub document: Option<CadApiDocument>,
}

/// The reply envelope of one API call.
///
/// `value` and `error` are mutually exclusive and the absent half is omitted
/// from the JSON, so a caller reads exactly one of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CadApiOutcome {
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
            ok: true,
            value: Some(value),
            error: None,
        }
    }

    /// A call this carrier does not perform, with its stable marker.
    pub fn refusal(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            value: None,
            error: Some(error.into()),
        }
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
    let requested = parameters
        .and_then(|parameters| parameters.get("database"))
        .and_then(Value::as_str);
    if requested != Some(document.database_id.as_str()) {
        return CadApiOutcome::refusal(API_CALL_INVALID_ARGUMENT_DATABASE);
    }
    match document.database_version.parse::<u64>() {
        Ok(revision) => CadApiOutcome::value(json!({ "revision": revision })),
        Err(_) => CadApiOutcome::refusal(API_CALL_INVALID_SNAPSHOT),
    }
}

/// The operations this build serves, resolved from `namespace.operation`.
enum ServedOperation {
    /// `ApplicationServices.documentManager.mdiActiveDocument`
    ActiveDocument,
    /// `ApplicationServices.documentManager.documents`
    Documents,
    /// `DatabaseServices.database.revision`
    Revision,
}

/// Resolve a namespaced operation, or `None` when this build does not serve it.
fn served_operation(namespace: &str, operation: &str) -> Option<ServedOperation> {
    match (namespace, operation) {
        (API_CALL_NAMESPACE, API_CALL_MDI_ACTIVE_DOCUMENT) => Some(ServedOperation::ActiveDocument),
        (API_CALL_NAMESPACE, API_CALL_DOCUMENTS) => Some(ServedOperation::Documents),
        (API_CALL_DATABASE_NAMESPACE, API_CALL_REVISION) => Some(ServedOperation::Revision),
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
/// `{ "documents": [] }`, and `database.revision` → [`API_CALL_NO_DOCUMENT`].
///
/// `parameters` carries the operation's arguments (currently only
/// `database.revision` reads them). The active-document read ignores them, so a
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
        CadApiSnapshot {
            document: self
                .core
                .controller
                .borrow()
                .plugin_document()
                .map(CadApiDocument::from),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> CadApiDocument {
        CadApiDocument {
            id: "1".to_string(),
            name: "plan.dwg".to_string(),
            is_read_only: false,
            database_id: "7".to_string(),
            database_version: "12".to_string(),
        }
    }

    fn snapshot() -> CadApiSnapshot {
        CadApiSnapshot {
            document: Some(document()),
        }
    }

    fn no_document_snapshot() -> CadApiSnapshot {
        CadApiSnapshot { document: None }
    }

    /// One call without parameters (the active-document read's shape).
    fn call(snapshot: Option<&CadApiSnapshot>, namespace: &str, operation: &str) -> CadApiOutcome {
        dispatch_api_call(snapshot, namespace, operation, None)
    }

    fn value_of(outcome: CadApiOutcome) -> Value {
        assert!(outcome.ok, "expected a value, got {outcome:?}");
        outcome.value.expect("a value")
    }

    fn error_of(outcome: CadApiOutcome) -> String {
        assert!(!outcome.ok, "expected a refusal, got {outcome:?}");
        assert_eq!(outcome.value, None, "a refusal carries no value");
        outcome.error.expect("a refusal marker")
    }

    #[test]
    fn the_active_document_reports_the_nested_wire_shape() {
        let outcome = call(
            Some(&snapshot()),
            API_CALL_NAMESPACE,
            API_CALL_MDI_ACTIVE_DOCUMENT,
        );
        assert_eq!(outcome.error, None);
        assert_eq!(
            value_of(outcome),
            json!({
                "document": {
                    "id": "1",
                    "name": "plan.dwg",
                    "isReadOnly": false,
                    "database": { "id": "7", "version": "12" },
                }
            })
        );
    }

    #[test]
    fn the_documents_list_reports_the_same_descriptor() {
        let listed = value_of(call(
            Some(&snapshot()),
            API_CALL_NAMESPACE,
            API_CALL_DOCUMENTS,
        ));
        let active = value_of(call(
            Some(&snapshot()),
            API_CALL_NAMESPACE,
            API_CALL_MDI_ACTIVE_DOCUMENT,
        ));
        let documents = listed["documents"].as_array().expect("an array");
        assert_eq!(documents.len(), 1, "this host holds one document");
        assert_eq!(
            documents[0], active["document"],
            "one descriptor shape for both reads"
        );
    }

    #[test]
    fn no_document_reports_an_empty_document_list() {
        let outcome = call(
            Some(&no_document_snapshot()),
            API_CALL_NAMESPACE,
            API_CALL_DOCUMENTS,
        );
        assert!(outcome.ok, "no document is a fact, not a wait");
        assert_eq!(value_of(outcome), json!({ "documents": [] }));
    }

    #[test]
    fn the_revision_matches_the_snapshot_database_id() {
        let parameters = json!({ "database": "7" });
        let outcome = dispatch_api_call(
            Some(&snapshot()),
            API_CALL_DATABASE_NAMESPACE,
            API_CALL_REVISION,
            Some(&parameters),
        );
        assert_eq!(outcome.error, None);
        assert_eq!(value_of(outcome), json!({ "revision": 12 }));
    }

    #[test]
    fn the_revision_refuses_a_mismatched_database() {
        for parameters in [
            json!({ "database": "999" }),
            json!({ "database": "" }),
            json!({ "database": "07" }),
        ] {
            let outcome = dispatch_api_call(
                Some(&snapshot()),
                API_CALL_DATABASE_NAMESPACE,
                API_CALL_REVISION,
                Some(&parameters),
            );
            assert_eq!(
                error_of(outcome),
                API_CALL_INVALID_ARGUMENT_DATABASE,
                "parameters {parameters} must not resolve another database"
            );
        }
    }

    #[test]
    fn the_revision_refuses_missing_or_non_string_arguments() {
        for parameters in [
            Value::Null,
            json!({}),
            json!({ "other": "7" }),
            json!({ "database": 7 }),
            json!({ "database": null }),
            json!(["7"]),
        ] {
            let outcome = dispatch_api_call(
                Some(&snapshot()),
                API_CALL_DATABASE_NAMESPACE,
                API_CALL_REVISION,
                Some(&parameters),
            );
            assert_eq!(
                error_of(outcome),
                API_CALL_INVALID_ARGUMENT_DATABASE,
                "parameters {parameters} must be refused by name"
            );
        }
        // An absent `parameters` object is the same refusal.
        assert_eq!(
            error_of(call(
                Some(&snapshot()),
                API_CALL_DATABASE_NAMESPACE,
                API_CALL_REVISION
            )),
            API_CALL_INVALID_ARGUMENT_DATABASE
        );
    }

    #[test]
    fn the_revision_without_a_document_is_no_document() {
        // The no-document fact wins over argument validation: there is no
        // database to name, so every parameter set gets the same answer.
        for parameters in [None, Some(json!({ "database": "7" }))] {
            let outcome = dispatch_api_call(
                Some(&no_document_snapshot()),
                API_CALL_DATABASE_NAMESPACE,
                API_CALL_REVISION,
                parameters.as_ref(),
            );
            assert_eq!(error_of(outcome), API_CALL_NO_DOCUMENT);
        }
    }

    #[test]
    fn the_revision_refuses_an_unparseable_snapshot_version() {
        // This build always renders the revision decimal; a violated invariant is
        // a named refusal, never a fabricated number.
        let broken = CadApiSnapshot {
            document: Some(CadApiDocument {
                database_version: "v12".to_string(),
                ..document()
            }),
        };
        let parameters = json!({ "database": "7" });
        let outcome = dispatch_api_call(
            Some(&broken),
            API_CALL_DATABASE_NAMESPACE,
            API_CALL_REVISION,
            Some(&parameters),
        );
        assert_eq!(error_of(outcome), API_CALL_INVALID_SNAPSHOT);
    }

    #[test]
    fn no_snapshot_is_cad_waiting_for_every_served_operation() {
        for (namespace, operation) in [
            (API_CALL_NAMESPACE, API_CALL_MDI_ACTIVE_DOCUMENT),
            (API_CALL_NAMESPACE, API_CALL_DOCUMENTS),
            (API_CALL_DATABASE_NAMESPACE, API_CALL_REVISION),
        ] {
            let outcome = call(None, namespace, operation);
            assert_eq!(error_of(outcome), CAD_WAITING, "{namespace}.{operation}");
        }
    }

    #[test]
    fn an_unknown_operation_is_the_stable_refusal() {
        for (namespace, operation) in [
            (API_CALL_NAMESPACE, "documentManager.other"),
            (API_CALL_DATABASE_NAMESPACE, API_CALL_MDI_ACTIVE_DOCUMENT),
            (API_CALL_NAMESPACE, API_CALL_REVISION),
            (API_CALL_DATABASE_NAMESPACE, API_CALL_DOCUMENTS),
            ("", ""),
            ("", API_CALL_MDI_ACTIVE_DOCUMENT),
        ] {
            let outcome = call(Some(&snapshot()), namespace, operation);
            assert_eq!(
                error_of(outcome),
                api_call_unknown(namespace, operation),
                "{namespace}.{operation} is not served"
            );
            assert_eq!(
                error_of(call(Some(&snapshot()), namespace, operation)),
                format!("unsupported-runtime: cad_api.{namespace}.{operation}")
            );
        }
    }

    #[test]
    fn an_unknown_operation_is_refused_before_snapshot_and_arguments() {
        let parameters = json!({ "database": "nope" });
        let outcome = dispatch_api_call(None, "EditorInput", "getPoint", Some(&parameters));
        assert_eq!(
            error_of(outcome),
            "unsupported-runtime: cad_api.EditorInput.getPoint",
            "cad_waiting/argument refusals would promise a later success"
        );
    }

    #[test]
    fn the_active_document_ignores_parameters() {
        let without = value_of(call(
            Some(&snapshot()),
            API_CALL_NAMESPACE,
            API_CALL_MDI_ACTIVE_DOCUMENT,
        ));
        for parameters in [json!({}), json!({ "database": "7", "unused": [1, 2] })] {
            let with = value_of(dispatch_api_call(
                Some(&snapshot()),
                API_CALL_NAMESPACE,
                API_CALL_MDI_ACTIVE_DOCUMENT,
                Some(&parameters),
            ));
            assert_eq!(with, without, "this read takes no arguments");
        }
    }

    #[test]
    fn no_document_answers_a_null_document() {
        // A snapshot that exists without a document is a fact, answered as such
        // (never `cad_waiting`, which would claim the host has nothing to say).
        let outcome = call(
            Some(&no_document_snapshot()),
            API_CALL_NAMESPACE,
            API_CALL_MDI_ACTIVE_DOCUMENT,
        );
        assert!(outcome.ok);
        assert_eq!(outcome.error, None);
        assert_eq!(value_of(outcome), json!({ "document": null }));
        assert_eq!(
            api_call_document(None).value.expect("a value"),
            json!({ "document": null })
        );
    }

    #[test]
    fn the_snapshot_serializes_as_the_frozen_wire_shape() {
        assert_eq!(
            serde_json::to_value(snapshot()).expect("serializes"),
            json!({
                "document": {
                    "id": "1",
                    "name": "plan.dwg",
                    "isReadOnly": false,
                    "database": { "id": "7", "version": "12" },
                }
            })
        );
        assert_eq!(
            serde_json::to_value(no_document_snapshot()).expect("serializes"),
            json!({ "document": null })
        );
    }

    #[test]
    fn the_read_only_flag_travels_unchanged() {
        let mut document = document();
        document.is_read_only = true;
        let outcome = call(
            Some(&CadApiSnapshot {
                document: Some(document),
            }),
            API_CALL_NAMESPACE,
            API_CALL_MDI_ACTIVE_DOCUMENT,
        );
        let value = value_of(outcome);
        assert_eq!(value["document"]["isReadOnly"], json!(true));
    }

    #[test]
    fn the_reply_omits_the_absent_half() {
        let ok = serde_json::to_value(api_call_document(Some(document()))).expect("serializes");
        assert_eq!(ok["ok"], json!(true));
        assert!(ok.get("error").is_none(), "the refusal half is omitted");
        let refused = serde_json::to_value(CadApiOutcome::refusal("x")).expect("serializes");
        assert_eq!(refused["ok"], json!(false));
        assert!(refused.get("value").is_none(), "the value half is omitted");
    }
}
