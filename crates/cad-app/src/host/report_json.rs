//! JSON rows for the two persisted report tables (ADR-0037 D11/D14).
//!
//! `tables/compat.json` holds object-level compatibility report records and
//! `tables/sources.json` holds one import-provenance row per import. Both are
//! **host-owned document metadata**: `cad-db` carries the rows as opaque JSON
//! and never interprets them, so this module is the single place the shapes are
//! written.
//!
//! The JSON is the persistence contract, not a UI projection:
//!
//! ```text
//! compat: {"connector","direction","level","counts"|null,
//!          "completeness":{"kind","reasons"}, "capabilities":[...], "entries":[...]}
//! source: {"source_id","identifier","kind","checksum","retained",
//!          "connector","open_revision"}
//! ```
//!
//! Rules this module enforces rather than leaving to convention:
//!
//! * an **import** record stores `counts: null`, never fabricated zeros — an
//!   import has no exported-geometry tally, and `0/0/0/0` would read as one;
//! * `checksum` is a real SHA-256 hex or `null`. A temporary document identity
//!   must not be hashed into anything that looks like a source checksum;
//! * no raw source bytes are ever written here (D11 keeps them out of the
//!   report tables entirely).

use serde_json::{json, Value};

use cad_connector_core::{CompatReport, FidelityLevel, ImportReport, ReportDirection};
use cad_domain::{Completeness, DocumentIdentity, SupportStatus};

/// The two persisted rows one successful import publishes.
///
/// Attribution comes from the registry wrapper, which stamps
/// [`ImportReport::connector`] after a successful import. A direct adapter call
/// (tests, CLI probes) leaves it `None`; the rows then say `"unknown"` instead
/// of inventing a format.
pub(crate) fn import_rows(report: &ImportReport, identifier: &str) -> (Vec<Value>, Vec<Value>) {
    let connector = report.connector.unwrap_or("unknown");
    let compat = compat_report_to_json(&report.compat(
        connector,
        FidelityLevel::L1,
        ReportDirection::Import,
    ));
    let source = source_row_to_json(report, identifier, connector);
    (vec![compat], vec![source])
}

/// One persisted compatibility record.
pub(crate) fn compat_report_to_json(report: &CompatReport) -> Value {
    json!({
        "connector": report.connector,
        "direction": direction_label(report.direction),
        "level": level_label(report.level),
        // `null` for an import: there is no exported-geometry tally to report,
        // and zeros would be a fabricated count (ADR-0037 D11).
        "counts": match report.direction {
            ReportDirection::Import => Value::Null,
            ReportDirection::Export => json!({
                "exact": report.counts.exact,
                "converted": report.counts.converted,
                "dropped": report.counts.dropped,
                "bytes": report.counts.bytes,
            }),
        },
        "completeness": completeness_json(&report.completeness),
        "capabilities": report
            .capabilities
            .iter()
            .map(|capability| json!({
                "type_key": capability.type_key,
                "read": status_label(capability.read),
                "semantic": status_label(capability.semantic),
                "render": status_label(capability.render),
                "pick": status_label(capability.pick),
                "measure": status_label(capability.measure),
            }))
            .collect::<Vec<_>>(),
        "entries": report
            .entries
            .iter()
            .map(|entry| json!({
                "object": entry.object.map(|id| id.0 as u64),
                "type_key": entry.type_key,
                "status": status_label(entry.status),
                "code": entry.code,
                "message": entry.message,
            }))
            .collect::<Vec<_>>(),
    })
}

/// One persisted import-provenance row.
///
/// `identifier` is the open label (never a path: the host owns that and the
/// catalogue does not need it). `connector` is the registry id the import was
/// dispatched through.
pub(crate) fn source_row_to_json(
    report: &ImportReport,
    identifier: &str,
    connector: &'static str,
) -> Value {
    let (source_id, checksum) = match &report.identity {
        // A content identity: the digest is both the row id and the checksum.
        DocumentIdentity::Sha256(digest) => {
            let hex = hex(digest);
            (Value::from(hex.clone()), Value::from(hex))
        }
        // A temporary identity has no content digest. Reuse its token as the row
        // id rather than hashing it (that would fabricate a checksum).
        DocumentIdentity::Temporary(serial) => (Value::from(format!("uuid:{serial}")), Value::Null),
    };
    json!({
        "source_id": source_id,
        "identifier": identifier,
        "kind": "drawing",
        "checksum": checksum,
        "retained": false,
        "connector": connector,
        // An import has no archive baseline yet: the `.ymodel` is written later,
        // so there is no revision to name here.
        "open_revision": Value::Null,
    })
}

fn completeness_json(completeness: &Completeness) -> Value {
    let (kind, reasons) = match completeness {
        Completeness::Complete => ("complete", Vec::new()),
        Completeness::Partial(reasons) => ("partial", reasons.clone()),
        Completeness::Missing(reasons) => ("missing", reasons.clone()),
        Completeness::Unverified => ("unverified", Vec::new()),
    };
    json!({"kind": kind, "reasons": reasons})
}

/// Wire-stable spelling of one support column (same vocabulary as the report).
fn status_label(status: SupportStatus) -> &'static str {
    match status {
        SupportStatus::Verified => "verified",
        SupportStatus::Partial => "partial",
        SupportStatus::Unverified => "unverified",
        SupportStatus::NotImplemented => "not_implemented",
        SupportStatus::Unsupported => "unsupported",
    }
}

fn level_label(level: FidelityLevel) -> &'static str {
    match level {
        FidelityLevel::L0 => "L0",
        FidelityLevel::L1 => "L1",
    }
}

fn direction_label(direction: ReportDirection) -> &'static str {
    match direction {
        ReportDirection::Import => "import",
        ReportDirection::Export => "export",
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_domain::{Diagnostic, EntityCapability, ObjectId};

    fn capability(type_key: &str, read: SupportStatus, measure: SupportStatus) -> EntityCapability {
        EntityCapability {
            type_key: type_key.into(),
            read,
            semantic: SupportStatus::Partial,
            render: SupportStatus::Verified,
            pick: SupportStatus::Verified,
            measure,
        }
    }

    fn report(identity: DocumentIdentity) -> ImportReport {
        ImportReport {
            identity,
            capabilities: vec![capability(
                "AcDbSpline",
                SupportStatus::Verified,
                SupportStatus::Unsupported,
            )],
            completeness: Completeness::Partial(vec!["spline flags dropped".into()]),
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(7)),
                code: "import.spline.flags_dropped".into(),
                message: "flags dropped".into(),
            }],
            connector: Some("dxf"),
            parse_ms: None,
            object_xdata: Vec::new(),
        }
    }

    #[test]
    fn an_import_record_keeps_capabilities_and_entry_markers() {
        let compat = report(DocumentIdentity::Sha256([0; 32])).compat(
            "dxf",
            FidelityLevel::L1,
            ReportDirection::Import,
        );
        let value = compat_report_to_json(&compat);
        assert_eq!(value["connector"], "dxf");
        assert_eq!(value["direction"], "import");
        assert_eq!(value["level"], "L1");
        // An import has no export tally; zeros would be a fabricated count.
        assert!(value["counts"].is_null());
        assert_eq!(value["completeness"]["kind"], "partial");
        assert_eq!(value["completeness"]["reasons"][0], "spline flags dropped");
        assert_eq!(value["capabilities"][0]["type_key"], "AcDbSpline");
        assert_eq!(value["capabilities"][0]["measure"], "unsupported");
        // One entry per capability row, then one per diagnostic.
        assert!(value["entries"][0]["object"].is_null());
        assert_eq!(value["entries"][0]["code"], "compat.capability");
        assert_eq!(value["entries"][1]["object"], 7);
        assert_eq!(value["entries"][1]["code"], "import.spline.flags_dropped");
        // The report is partial and the code does not use the `.dropped`
        // namespace, so the entry inherits `partial` (never a clean success).
        assert_eq!(value["entries"][1]["status"], "partial");
    }

    #[test]
    fn a_sha256_identity_becomes_the_source_id_and_the_checksum() {
        let row = source_row_to_json(
            &report(DocumentIdentity::Sha256([0xab; 32])),
            "plan.dxf",
            "dxf",
        );
        let hex = "ab".repeat(32);
        assert_eq!(row["source_id"], hex);
        assert_eq!(row["checksum"], hex);
        assert_eq!(row["identifier"], "plan.dxf");
        assert_eq!(row["kind"], "drawing");
        assert_eq!(row["retained"], false);
        assert_eq!(row["connector"], "dxf");
        assert!(row["open_revision"].is_null());
    }

    #[test]
    fn a_temporary_identity_never_fabricates_a_checksum() {
        let row = source_row_to_json(
            &report(DocumentIdentity::Temporary(3)),
            "untitled.dwg",
            "dwg",
        );
        assert_eq!(row["source_id"], "uuid:3");
        assert!(row["checksum"].is_null());
    }
}
