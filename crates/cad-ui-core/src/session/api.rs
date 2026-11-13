//! The plugin / secondary-development API **capability descriptor**.
//!
//! This module is the shared JSON authority for what a plugin can discover about
//! the API this build offers. [`API_DESCRIPTOR`] is the checked-in
//! `api/plugin-api.json` document (one version, one namespace list, one row per
//! operation); [`status`] parses it and pins the version, and the
//! [`API_STATUS_COMMAND`] wire command serves that value identically on both
//! transports.
//!
//! # What this is, and what it is not
//!
//! The descriptor is **negotiation**, not implementation. A row marked
//! [`CadApiOperationStatus::Pending`] names an operation the plugin contract
//! reserves *and this build does not perform*; its `reason` is the stable
//! `api.pending: <Namespace>.<operation>` marker a host or adapter can surface
//! verbatim.
//!
//! A row exists only for an operation the **host** serves (or reserves) over the
//! wire, so a descriptor can legitimately list zero
//! [`CadApiOperationStatus::Supported`] rows: that is the case in this build —
//! every row is `Pending`. Package-level features that never cross the wire (the
//! `Geometry`/`Colors` value helpers and the manifest parser in
//! `packages/cad-plugin-api`) are **not** capabilities and therefore have no row;
//! advertising them here would promise host state that does not exist. Nothing in
//! this module reads or writes a document: the document, transaction and editor
//! operations stay pending until a later round implements them through
//! `cad-db`/host commands.
//!
//! The three status values are the same vocabulary the rest of the wire uses: a
//! supported row omits `reason`, while a pending or unsupported row must carry
//! one (never an empty success, never a silently missing capability).

use serde::{Deserialize, Serialize};

/// The plugin API contract version this build advertises.
///
/// Bumped only when the descriptor's *shape or meaning* changes incompatibly; a
/// row moving from `pending` to `supported` is additive and does not bump it.
pub const PLUGIN_API_VERSION: u32 = 1;

/// The checked-in descriptor document (`api/plugin-api.json`), byte for byte.
///
/// A single authority: the Rust answer, the TypeScript contract test and the
/// documentation all derive from this file rather than restating the row list.
pub const API_DESCRIPTOR: &str = include_str!("../../api/plugin-api.json");

/// The wire command name that answers the descriptor.
pub const API_STATUS_COMMAND: &str = "cad_api_status";

/// One API namespace of the plugin contract.
///
/// The wire spelling is the PascalCase variant name itself; the rename is pinned
/// so a variant rename cannot silently change the JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum CadApiNamespace {
    /// Application-level services: documents, the MDI document manager.
    ApplicationServices,
    /// Database-level services: object ids, symbol tables, transactions.
    DatabaseServices,
    /// Editor input services: point/entity/selection/string prompts.
    EditorInput,
    /// Plugin runtime: command registration, load/unload, manifest.
    Runtime,
    /// Geometry value types.
    Geometry,
    /// Colour value type.
    Colors,
}

impl CadApiNamespace {
    /// The PascalCase wire spelling of this namespace.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApplicationServices => "ApplicationServices",
            Self::DatabaseServices => "DatabaseServices",
            Self::EditorInput => "EditorInput",
            Self::Runtime => "Runtime",
            Self::Geometry => "Geometry",
            Self::Colors => "Colors",
        }
    }
}

/// How much of one operation this build actually offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CadApiOperationStatus {
    /// Implemented and callable in this build.
    Supported,
    /// Reserved by the contract, not implemented in this build.
    Pending,
    /// Deliberately not offered by this carrier/contract.
    Unsupported,
}

/// One row of the descriptor: a namespaced operation and its availability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CadApiOperation {
    /// The namespace the operation belongs to.
    pub namespace: CadApiNamespace,
    /// The operation's dotted name inside its namespace.
    pub operation: String,
    /// This build's availability for the operation.
    pub status: CadApiOperationStatus,
    /// The stable refusal marker for a non-supported row
    /// (`api.pending: <Namespace>.<operation>`); absent for `supported`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The whole `cad_api_status` answer (camelCase on the wire).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CadApiStatus {
    /// The contract version the rows below are written against.
    pub api_version: u32,
    /// Every namespace the contract declares, in descriptor order.
    pub namespaces: Vec<CadApiNamespace>,
    /// Every operation row, in descriptor order.
    pub operations: Vec<CadApiOperation>,
}

/// Parse one descriptor document into the wire value.
///
/// The parse is the only validation step here; the structural invariants
/// (unique rows, pinned pending reasons) are covered by the module tests. A
/// malformed document is a named error, never an empty descriptor.
pub fn parse_descriptor(text: &str) -> Result<CadApiStatus, String> {
    serde_json::from_str(text).map_err(|err| format!("api.descriptor_invalid: {err}"))
}

/// The descriptor this build advertises.
///
/// Parses the checked-in document and pins [`CadApiStatus::api_version`] to
/// [`PLUGIN_API_VERSION`], so a stale JSON file is an explicit error rather than
/// a version the transport reports without owning.
pub fn status() -> Result<CadApiStatus, String> {
    let descriptor = parse_descriptor(API_DESCRIPTOR)?;
    if descriptor.api_version != PLUGIN_API_VERSION {
        return Err("api.descriptor_invalid: apiVersion".to_string());
    }
    Ok(descriptor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending_reason(namespace: CadApiNamespace, operation: &str) -> String {
        format!("api.pending: {}.{operation}", namespace.as_str())
    }

    #[test]
    fn descriptor_parses_with_the_pinned_version() {
        let descriptor = status().expect("the checked-in descriptor parses");
        assert_eq!(descriptor.api_version, PLUGIN_API_VERSION);
        assert_eq!(descriptor.api_version, 1);
        assert!(
            !descriptor.operations.is_empty(),
            "the descriptor advertises at least one operation"
        );
    }

    #[test]
    fn operation_rows_are_unique() {
        let descriptor = status().expect("the checked-in descriptor parses");
        let mut seen: Vec<(CadApiNamespace, &str)> = Vec::new();
        for row in &descriptor.operations {
            let key = (row.namespace, row.operation.as_str());
            assert!(
                !seen.contains(&key),
                "duplicate API operation row: {}.{}",
                row.namespace.as_str(),
                row.operation
            );
            seen.push(key);
        }
    }

    #[test]
    fn pending_rows_carry_their_pinned_reason() {
        let descriptor = status().expect("the checked-in descriptor parses");
        for row in &descriptor.operations {
            if row.status != CadApiOperationStatus::Pending {
                continue;
            }
            assert_eq!(
                row.reason.as_deref(),
                Some(pending_reason(row.namespace, &row.operation).as_str()),
                "pending row {}.{} carries its stable marker",
                row.namespace.as_str(),
                row.operation
            );
        }
    }

    #[test]
    fn supported_rows_omit_their_reason() {
        let descriptor = status().expect("the checked-in descriptor parses");
        let mut supported = 0;
        for row in &descriptor.operations {
            if row.status == CadApiOperationStatus::Supported {
                supported += 1;
                assert_eq!(
                    row.reason,
                    None,
                    "supported row {}.{} must not carry a reason",
                    row.namespace.as_str(),
                    row.operation
                );
            }
        }
        // Three operations are served in this build: the two reflected document
        // reads and the database revision (M2.2/M2.3). Pure value helpers and the
        // manifest parser are package features, not descriptor rows, so they never
        // appear here.
        assert_eq!(
            supported, 3,
            "this build serves exactly three host operations"
        );
    }

    #[test]
    fn namespaces_are_unique() {
        let descriptor = status().expect("the checked-in descriptor parses");
        let mut seen: Vec<CadApiNamespace> = Vec::new();
        for namespace in &descriptor.namespaces {
            assert!(
                !seen.contains(namespace),
                "duplicate namespace: {}",
                namespace.as_str()
            );
            seen.push(*namespace);
        }
        assert!(!descriptor.namespaces.is_empty());
    }
}
