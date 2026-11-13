//! The plugin-API **call** surface: one reflected read, served on both carriers.
//!
//! # What this is
//!
//! [`API_CALL_COMMAND`] (`cad_api_call`) is the first *operation* the plugin API
//! performs rather than merely advertises. This wave serves exactly one
//! operation, `ApplicationServices.documentManager.mdiActiveDocument`: the
//! reflected snapshot of the open document (or of "no document").
//!
//! The read is **derived**: the authoritative document stays `cad-db`'s, and the
//! value below is a projection of the host controller's own fields
//! ([`cad_app::PluginDocument`]). Nothing here opens a transaction, mutates the
//! drawing or writes the mirror back.
//!
//! # Refusals are explicit
//!
//! * an unknown/unimplemented operation is
//!   `unsupported-runtime: cad_api.<namespace>.<operation>` — the same
//!   machine-readable marker shape the rest of the wire uses for "this carrier
//!   does not perform that";
//! * **no snapshot yet** (the desktop owner-tick mirror has not been populated)
//!   is [`CAD_WAITING`], the shared "ask again after a tick" word. A snapshot
//!   that exists but holds no document is *not* waiting: that is the real
//!   answer `{ "document": null }`, see [`CadApiSnapshot`].
//!
//! An empty namespace or operation is simply an operation that is not served, so
//! it takes the first refusal. It is **never** answered as an empty success.
//!
//! # The wire shape
//!
//! ```json
//! { "document": { "id": "<DocumentId>", "name": "<label>", "isReadOnly": false,
//!                 "database": { "id": "<DatabaseId>", "version": "<revision>" } } }
//! ```
//!
//! or `{ "document": null }` when the document manager exists but holds no
//! document. The reply envelope is [`CadApiOutcome`]: `ok` plus exactly one of
//! `value`/`error` (the absent half is omitted, never `null`).

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

/// The namespace prefix of the one served operation.
pub const API_CALL_NAMESPACE: &str = "ApplicationServices";

/// The one served operation (part of the PascalCase plugin contract).
pub const API_CALL_MDI_ACTIVE_DOCUMENT: &str = "documentManager.mdiActiveDocument";

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

/// Whether this build performs `namespace.operation`.
fn is_served(namespace: &str, operation: &str) -> bool {
    namespace == API_CALL_NAMESPACE && operation == API_CALL_MDI_ACTIVE_DOCUMENT
}

/// Serve one plugin-API call from an optional reflected snapshot.
///
/// `snapshot == None` means the host has **no snapshot yet** (the desktop mirror
/// before the first owner tick), so the answer is [`CAD_WAITING`] — never a claim
/// about the drawing. `Some(snapshot)` is answered as a matter of fact, including
/// `snapshot.document == None` → `{ "document": null }`.
///
/// An operation outside the served one is the stable `unsupported-runtime`
/// refusal even before a snapshot exists: "ask again later" would be a false
/// promise about an operation this build will never perform.
pub fn dispatch_api_call(
    snapshot: Option<&CadApiSnapshot>,
    namespace: &str,
    operation: &str,
) -> CadApiOutcome {
    if !is_served(namespace, operation) {
        return CadApiOutcome::refusal(api_call_unknown(namespace, operation));
    }
    match snapshot {
        Some(snapshot) => api_call_document(snapshot.document.clone()),
        None => CadApiOutcome::refusal(CAD_WAITING),
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

    #[test]
    fn the_served_operation_reports_the_nested_wire_shape() {
        let outcome = dispatch_api_call(
            Some(&snapshot()),
            API_CALL_NAMESPACE,
            API_CALL_MDI_ACTIVE_DOCUMENT,
        );
        assert!(outcome.ok);
        assert_eq!(outcome.error, None);
        assert_eq!(
            outcome.value.expect("a value"),
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
    fn the_reply_omits_the_absent_half() {
        let ok = serde_json::to_value(api_call_document(Some(document()))).expect("serializes");
        assert_eq!(ok["ok"], json!(true));
        assert!(ok.get("error").is_none(), "the refusal half is omitted");
        let refused = serde_json::to_value(CadApiOutcome::refusal("x")).expect("serializes");
        assert_eq!(refused["ok"], json!(false));
        assert!(refused.get("value").is_none(), "the value half is omitted");
    }

    #[test]
    fn an_unknown_operation_is_the_stable_refusal() {
        for (namespace, operation) in [
            (API_CALL_NAMESPACE, "documentManager.other"),
            ("DatabaseServices", API_CALL_MDI_ACTIVE_DOCUMENT),
            ("", ""),
            ("", API_CALL_MDI_ACTIVE_DOCUMENT),
        ] {
            let outcome = dispatch_api_call(Some(&snapshot()), namespace, operation);
            assert!(!outcome.ok);
            assert_eq!(outcome.value, None);
            assert_eq!(
                outcome.error.as_deref(),
                Some(api_call_unknown(namespace, operation).as_str())
            );
            assert_eq!(
                outcome.error.as_deref(),
                Some(format!("unsupported-runtime: cad_api.{namespace}.{operation}").as_str())
            );
        }
    }

    #[test]
    fn an_unknown_operation_is_refused_before_a_snapshot_exists() {
        let outcome = dispatch_api_call(None, "EditorInput", "getPoint");
        assert!(!outcome.ok, "cad_waiting would promise a later success");
        assert_eq!(
            outcome.error.as_deref(),
            Some("unsupported-runtime: cad_api.EditorInput.getPoint")
        );
    }

    #[test]
    fn no_snapshot_is_cad_waiting() {
        let outcome = dispatch_api_call(None, API_CALL_NAMESPACE, API_CALL_MDI_ACTIVE_DOCUMENT);
        assert!(!outcome.ok);
        assert_eq!(outcome.value, None);
        assert_eq!(outcome.error.as_deref(), Some(CAD_WAITING));
    }

    #[test]
    fn no_document_answers_a_null_document() {
        // A snapshot that exists without a document is a fact, answered as such
        // (never `cad_waiting`, which would claim the host has nothing to say).
        let outcome = dispatch_api_call(
            Some(&CadApiSnapshot { document: None }),
            API_CALL_NAMESPACE,
            API_CALL_MDI_ACTIVE_DOCUMENT,
        );
        assert!(outcome.ok);
        assert_eq!(outcome.error, None);
        assert_eq!(outcome.value.expect("a value"), json!({ "document": null }));
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
            serde_json::to_value(CadApiSnapshot { document: None }).expect("serializes"),
            json!({ "document": null })
        );
    }

    #[test]
    fn the_read_only_flag_travels_unchanged() {
        let mut document = document();
        document.is_read_only = true;
        let outcome = dispatch_api_call(
            Some(&CadApiSnapshot {
                document: Some(document),
            }),
            API_CALL_NAMESPACE,
            API_CALL_MDI_ACTIVE_DOCUMENT,
        );
        let value = outcome.value.expect("a value");
        assert_eq!(value["document"]["isReadOnly"], json!(true));
    }

    #[test]
    fn the_reply_never_echoes_the_call_parameters() {
        // `parameters` exists on the wire command for contract stability and is
        // deliberately unused this wave: the answer is a reflected snapshot, not
        // an echo of the request. The transport-level coverage (an unknown
        // `parameters` object changing nothing) lives in the app-web matrix.
        let value = api_call_document(Some(document())).value.expect("a value");
        let object = value.as_object().expect("one object");
        assert_eq!(object.len(), 1, "only the `document` key is served");
        assert!(object.get("parameters").is_none());
    }
}
