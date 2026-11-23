//! Host-side vendor semantics rows (ADR-0030 D7/D8).
//!
//! The WebView plugin extracts version-independent `TCH_*` semantics at import.
//! This module is the only place that turns those rows into document data:
//!
//! * resolve the plugin's `source.handle` strings to `EntityId`s and write the
//!   persisted `entity` number into each row (and each `refs[]` entry);
//! * keep the rows on `PersistenceState` (they are written to
//!   `tables/vendor.json`, not to `cad-db`);
//! * hand the rows back to the owner tick for one `YANS` pass when a `.ymodel`
//!   is opened.
//!
//! It never stores primitives and never writes geometry. A handle that the
//! loaded drawing does not contain stays unresolved with a stable diagnostic;
//! a row whose `entity` is not in the drawing is dropped on load (orphan) and
//! counted in the status line rather than silently attached to another entity.

use std::collections::HashMap;

use cad_db::DrawingDatabase;
use cad_domain::ymodel::{PersistentId, PersistentIdOf};
use cad_domain::EntityId;
use serde_json::{Map, Value};

use super::HostController;

/// Upper bound of persisted semantics rows in one document.
pub const MAX_VENDOR_SEMANTICS_ROWS: usize = 200_000;
/// Upper bound of one row's serialized JSON.
pub const MAX_VENDOR_SEMANTICS_ROW_BYTES: usize = 1024 * 1024;
/// Upper bound of the serialized rows a pass may carry.
pub const MAX_VENDOR_SEMANTICS_BYTES: usize = 32 * 1024 * 1024;
/// Highest vendor semantics schema this build accepts.
pub const VENDOR_SEMANTICS_SCHEMA_MAX: u64 = 1;

impl HostController {
    /// Take the vendor records copied by the most recent successful import.
    ///
    /// The owner tick calls this exactly once per published document and hands
    /// the bytes to the WebView vendor pass. `Some([])` is a real answer — the new
    /// document carries no `TCH_*` objects — and the caller must clear the
    /// previous pass so a stale token cannot settle into it. `None` means "no new
    /// document since the last call", or that a `.ymodel` owes a `YANS` pass
    /// instead (the two are mutually exclusive for one document, ADR-0030 D8).
    pub fn take_vendor_records(&mut self) -> Option<Vec<crate::VendorRecord>> {
        if self
            .application
            .workspace
            .documents
            .get(&self.document_id)
            .is_some_and(|document| document.persistence.vendor_semantics_pending)
        {
            return None;
        }
        if !self.vendor_records_pending {
            return None;
        }
        self.vendor_records_pending = false;
        Some(std::mem::take(&mut self.last_vendor_records))
    }

    /// Take the rows loaded from a `.ymodel`, once, for the owner tick's `YANS`
    /// pass (ADR-0030 D8).
    ///
    /// `None` is "this document owes no semantics pass": an import (the plugin
    /// already produced the rows) or an archive without any vendor row. The
    /// rows stay on the document slot, so a save before the pass settles still
    /// preserves them.
    pub fn take_vendor_semantics(&mut self) -> Option<Vec<Value>> {
        let document = self
            .application
            .workspace
            .documents
            .get_mut(&self.document_id)?;
        if !document.persistence.vendor_semantics_pending {
            return None;
        }
        document.persistence.vendor_semantics_pending = false;
        Some(document.persistence.vendor_rows.clone())
    }

    /// Persist one validated `YANE` semantics batch for the current document.
    ///
    /// The plugin speaks source handles; the host is the only layer that knows
    /// `EntityId`, so it rewrites every row and reference here. The returned
    /// count is the number of rows stored (unresolved rows are still stored with
    /// a diagnostic; an unresolved reference simply has no `entity`).
    pub fn apply_vendor_semantics(&mut self, rows: Vec<Value>) -> Result<usize, String> {
        let drawing = self
            .drawing()
            .ok_or_else(|| "vendor.no_document".to_string())?;
        let index = source_handle_index(&drawing);
        let resolved = resolve_rows(rows, &index)?;
        let count = resolved.len();
        let document = self
            .application
            .workspace
            .documents
            .get_mut(&self.document_id)
            .ok_or_else(|| "vendor.no_document".to_string())?;
        document.persistence.vendor_rows = resolved;
        document.persistence.vendor_semantics_pending = false;
        self.dirty_scope.table(cad_db::TableKind::Vendor);
        Ok(count)
    }

    /// Whether a freshly installed document still owes a vendor input pass.
    ///
    /// The host uses this to set `documentReady = false` at publish, before the
    /// owner tick can stage the pass, so a SAVE cannot slip into the gap.
    pub fn vendor_input_pending(&self) -> bool {
        let semantics = self
            .application
            .workspace
            .documents
            .get(&self.document_id)
            .is_some_and(|document| document.persistence.vendor_semantics_pending);
        semantics || (self.vendor_records_pending && !self.last_vendor_records.is_empty())
    }

    /// The rows the next save should write, or an empty slice.
    pub fn vendor_rows(&self) -> &[Value] {
        self.application
            .workspace
            .documents
            .get(&self.document_id)
            .map(|document| document.persistence.vendor_rows.as_slice())
            .unwrap_or(&[])
    }
}

/// Build `source_handle (DWG u64) → EntityId` for the loaded drawing.
///
/// A duplicated source handle (a copied block member, a re-imported entity)
/// keeps the first entity; the store is still keyed by source handle, so this is
/// the documented P1 limitation (ADR-0030 D8), not a guessed identity.
fn source_handle_index(drawing: &DrawingDatabase) -> HashMap<u64, EntityId> {
    let mut index = HashMap::new();
    for entity in drawing.entities() {
        if let Some(handle) = entity
            .object
            .source_handle
            .as_deref()
            .and_then(parse_handle)
        {
            index.entry(handle).or_insert(entity.id);
        }
    }
    index
}

fn parse_handle(text: &str) -> Option<u64> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > 16 {
        return None;
    }
    u64::from_str_radix(trimmed, 16).ok()
}

/// Validate and resolve one batch in place.
fn resolve_rows(
    mut rows: Vec<Value>,
    index: &HashMap<u64, EntityId>,
) -> Result<Vec<Value>, String> {
    if rows.len() > MAX_VENDOR_SEMANTICS_ROWS {
        return Err("vendor.semantics.too_many_rows".to_string());
    }
    let mut total = 0usize;
    for row in &mut rows {
        let object = row
            .as_object_mut()
            .ok_or_else(|| "vendor.semantics.invalid_row".to_string())?;
        let schema = object
            .get("schema")
            .and_then(Value::as_u64)
            .ok_or_else(|| "vendor.semantics.invalid_schema".to_string())?;
        if schema > VENDOR_SEMANTICS_SCHEMA_MAX {
            return Err(format!("vendor.semantics.unsupported_schema: {schema}"));
        }
        if object.get("kind").and_then(Value::as_str) != Some("tch") {
            return Err("vendor.semantics.invalid_kind".to_string());
        }
        if object.get("class").and_then(Value::as_str).is_none() {
            return Err("vendor.semantics.invalid_class".to_string());
        }

        let handle = object
            .get("source")
            .and_then(|source| source.get("handle"))
            .and_then(Value::as_str)
            .and_then(parse_handle);
        match handle.and_then(|handle| index.get(&handle).copied()) {
            Some(id) => {
                object.insert("entity".to_string(), Value::from(id.persistent().raw()));
            }
            None => push_diagnostic(object, "vendor.source_handle_unresolved"),
        }

        // Resolve refs without borrowing `object` twice: clone the array, rewrite
        // it, then store it back and attach the diagnostics collected for it.
        let mut ref_reasons = Vec::new();
        if let Some(Value::Array(items)) = object.get("refs").cloned() {
            let mut rewritten = Vec::with_capacity(items.len());
            for mut item in items {
                // A block ref points at a block definition, not an entity; its
                // handle is the renderer's key (`block_by_source_handle`), so it
                // is kept as-is rather than reported unresolved.
                let is_block = item.get("kind").and_then(Value::as_str) == Some("block");
                let is_style = item.get("role").and_then(Value::as_str) == Some("style");
                if !is_block && !is_style {
                    if let Some(reference) = item.as_object_mut() {
                        let handle = reference
                            .get("handle")
                            .and_then(Value::as_str)
                            .and_then(parse_handle);
                        match handle.and_then(|handle| index.get(&handle).copied()) {
                            Some(id) => {
                                reference.insert(
                                    "entity".to_string(),
                                    Value::from(id.persistent().raw()),
                                );
                            }
                            None => ref_reasons.push("vendor.ref.unresolved"),
                        }
                    }
                }
                rewritten.push(item);
            }
            object.insert("refs".to_string(), Value::Array(rewritten));
        }
        for reason in ref_reasons {
            push_diagnostic(object, reason);
        }

        let bytes = serde_json::to_vec(row).map_err(|error| error.to_string())?;
        if bytes.len() > MAX_VENDOR_SEMANTICS_ROW_BYTES {
            return Err("vendor.semantics.row_too_large".to_string());
        }
        total = total.saturating_add(bytes.len());
        if total > MAX_VENDOR_SEMANTICS_BYTES {
            return Err("vendor.semantics.too_large".to_string());
        }
    }
    Ok(rows)
}

fn push_diagnostic(object: &mut Map<String, Value>, code: &str) {
    let entry = object
        .entry("diagnostics".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Value::Array(list) = entry else {
        return;
    };
    if !list.iter().any(|value| value.as_str() == Some(code)) {
        list.push(Value::String(code.to_string()));
    }
}

/// Drop rows whose `entity` no longer exists in `drawing`.
///
/// Returns the kept rows and the number dropped. A row without `entity` is an
/// unresolved import row; it cannot be attached to anything, so it is dropped
/// too and counted in the same orphan total.
pub(super) fn drop_orphan_rows(rows: Vec<Value>, drawing: &DrawingDatabase) -> (Vec<Value>, usize) {
    let mut kept = Vec::with_capacity(rows.len());
    let mut dropped = 0usize;
    for row in rows {
        let entity = row
            .get("entity")
            .and_then(Value::as_i64)
            .and_then(PersistentId::from_raw)
            .filter(|id| id.kind() == cad_domain::ymodel::IdKind::Entity)
            .map(|id| EntityId::from_seq(id.seq()));
        match entity {
            Some(id) if drawing.entity(id).is_some() => kept.push(row),
            _ => dropped = dropped.saturating_add(1),
        }
    }
    (kept, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_handle_resolves_and_a_missing_one_is_named() {
        let index = HashMap::from([(0x2f, EntityId(7))]);
        let rows = resolve_rows(
            vec![
                json!({
                    "kind": "tch", "class": "TCH_WALL", "schema": 1,
                    "source": {"handle": "2F"},
                    "refs": [{"role": "wall", "handle": "30"}]
                }),
                json!({
                    "kind": "tch", "class": "TCH_TEXT", "schema": 1,
                    "source": {"handle": "FF"}
                }),
            ],
            &index,
        )
        .expect("resolves");
        assert_eq!(rows[0]["entity"], json!(EntityId(7).persistent().raw()));
        assert_eq!(rows[0]["refs"][0]["handle"], json!("30"));
        assert!(rows[0]["refs"][0].get("entity").is_none());
        assert_eq!(rows[0]["diagnostics"][0], json!("vendor.ref.unresolved"));
        assert!(rows[1].get("entity").is_none());
        assert_eq!(
            rows[1]["diagnostics"][0],
            json!("vendor.source_handle_unresolved")
        );
    }

    #[test]
    fn a_row_schema_above_the_build_is_refused() {
        let error = resolve_rows(
            vec![json!({
                "kind": "tch", "class": "TCH_WALL", "schema": 99,
                "source": {"handle": "2F"}
            })],
            &HashMap::new(),
        )
        .expect_err("schema is refused");
        assert!(error.starts_with("vendor.semantics.unsupported_schema"));
    }
}
