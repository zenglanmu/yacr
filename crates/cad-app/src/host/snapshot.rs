//! Snapshot capture/install and the persistence-state accessors (ADR-0037 D1/D2/D12).
//!
//! `cad-db` owns the authoritative content and `cad-store-zip` owns the archive,
//! but neither may name the other (D1). The host is the only layer that holds
//! both a `Document` (units/identity/resource keys) and a database, so the two
//! conversions live here:
//!
//! * [`HostController::snapshot_document`] — database + document state → snapshot,
//!   for an archive write.
//! * [`HostController::install_snapshot`] — snapshot → a whole new authoritative
//!   document, for an archive read.
//!
//! Both keep the two revisions straight (D2): a snapshot carries
//! `manifest.saved_revision` as its revision, and the import baseline travels
//! separately in [`crate::PersistenceState::open_revision`].

use super::*;

/// The number of export reports [`HostController::record_export_report`] keeps.
///
/// Repeated Save As calls must not grow the document without bound; the table is
/// a bounded recent history, so the oldest rows are dropped first.
pub const MAX_EXPORT_RECORDS: usize = 32;

impl HostController {
    /// Encode the current document as a [`DocumentSnapshot`].
    ///
    /// The unit context, identity and resource keys come from the document slot
    /// (the database does not hold them); the database id and the identifier
    /// counters come from the database itself, so a header can never contradict
    /// the rows it describes.
    ///
    /// A missing document is an explicit failure: there is nothing to encode and
    /// an empty snapshot would be a fabricated document.
    pub fn snapshot_document(&self) -> CadResult<cad_db::DocumentSnapshot> {
        let document = self
            .application
            .workspace
            .documents
            .get(&self.document_id)
            .ok_or_else(|| CadError::InvalidInput("no document is open".into()))?;
        let mut snapshot = document
            .drawing
            .snapshot(
                document.units.clone(),
                document.identity.clone(),
                document.resource_keys.clone(),
                document.drawing.revision(),
            )
            // A marker, not prose: callers match on `ymodel.*` (D13).
            .map_err(|error| {
                CadError::Unsupported(format!("{}: {}", error.marker, error.detail))
            })?;
        // Host-owned metadata (ADR-0037 D11/D14): the database snapshot never
        // produces these rows, so the document slot supplies them here. An empty
        // vector is an empty table, not an absent one: the document really has
        // no report/source rows.
        snapshot
            .tables
            .set(cad_db::TableKind::Compat, document.compat.clone());
        snapshot
            .tables
            .set(cad_db::TableKind::Sources, document.sources.clone());
        // Vendor semantics are host-owned too (ADR-0030 D7), but unlike the two
        // report tables an **empty** table must not be written: the table is the
        // minor-12 tier, so a document with no vendor rows has to stay at the
        // minor its other features imply. `set` only when the document really
        // has rows.
        if !document.persistence.vendor_rows.is_empty() {
            snapshot.tables.set(
                cad_db::TableKind::Vendor,
                document.persistence.vendor_rows.clone(),
            );
        }
        Ok(snapshot)
    }

    /// Install a decoded archive as the authoritative document.
    ///
    /// Mirrors [`HostController::publish_imported`] field by field, except there
    /// is no [`ImportReport`] to publish: the archive *is* the document, so its
    /// header supplies units/identity/resource keys and the name hint comes from
    /// the caller. Every teardown step of the import publish is repeated here —
    /// a missed `reset_for_new_content` leaves an undo record, a selection or a
    /// layer override pointing at the replaced drawing.
    ///
    /// The caller installs the persistence state: `saved_revision` from the
    /// manifest, `open_revision` from the manifest's import baseline (never
    /// derived from the snapshot revision), the document uuid and the foreign
    /// entry list.
    pub fn install_snapshot(
        &mut self,
        snapshot: &cad_db::DocumentSnapshot,
        mut persistence: crate::PersistenceState,
        name_hint: Option<String>,
    ) -> CadResult<()> {
        let drawing = DrawingDatabase::from_snapshot(snapshot).map_err(|error| {
            CadError::Unsupported(format!("{}: {}", error.marker, error.detail))
        })?;
        // Host-owned vendor semantics travel with the archive (ADR-0030 D7).
        // The database hydration above ignores the table by construction; the
        // document slot is their runtime authority. Rows whose entity id is not
        // in the loaded drawing are stale (the entity was deleted before the
        // save); drop them here and name the count below instead of attaching
        // them to an unrelated entity.
        let stored = snapshot
            .tables
            .get(cad_db::TableKind::Vendor)
            .map(<[serde_json::Value]>::to_vec)
            .unwrap_or_default();
        let (vendor_rows, orphan_rows) =
            super::vendor_semantics::drop_orphan_rows(stored, &drawing);
        persistence.vendor_rows = vendor_rows;
        let semantics_pending = !persistence.vendor_rows.is_empty();
        persistence.vendor_semantics_pending = semantics_pending;
        let label = name_hint.unwrap_or_else(|| self.document_name_hint.clone());
        let document = Document {
            id: self.document_id,
            drawing: Arc::new(drawing),
            identity: snapshot.header.identity.clone(),
            units: snapshot.header.units.clone(),
            resource_keys: snapshot.header.resource_keys.clone(),
            // Host-owned metadata travels with the archive (ADR-0037 D11/D14).
            // The database hydration above ignores these rows by construction.
            compat: snapshot
                .tables
                .get(cad_db::TableKind::Compat)
                .map(<[serde_json::Value]>::to_vec)
                .unwrap_or_default(),
            sources: snapshot
                .tables
                .get(cad_db::TableKind::Sources)
                .map(<[serde_json::Value]>::to_vec)
                .unwrap_or_default(),
            persistence,
        };
        self.application
            .workspace
            .documents
            .insert(self.document_id, document);

        // Rebuild everything bound to the previous content identity.
        self.reset_for_new_content();
        self.document_name_hint = label;
        // No connector ran: a `.ymodel` open has no import report and no vendor
        // records. A document with persisted semantics owes a `YANS` pass
        // (ADR-0030 D8); one without any row still stages `Some([])` so the next
        // tick clears the previous pass.
        self.last_import_report = None;
        self.last_vendor_records = Vec::new();
        self.vendor_records_pending = !semantics_pending;
        // Not a localized sentence: the host owns the user-facing status line and
        // reads `document_name_hint` for it (the crate must not hardcode one).
        self.last_status = if orphan_rows == 0 {
            format!("opened {}", self.document_name_hint)
        } else {
            format!(
                "opened {} (vendor.orphan_rows: {orphan_rows})",
                self.document_name_hint
            )
        };
        Ok(())
    }

    /// Replace the document's unit context (ADR-0037 D9).
    ///
    /// This is a **document-header** change, not scene geometry: the units live
    /// on the `Document` slot, so the database learns about it through
    /// [`cad_db::DrawingDatabase::note_document_header_change`] — one revision
    /// that dirties only `document.json`, exactly what the next save's header
    /// write needs. The session generation advances the same way a view/command
    /// outcome does, so a host republishes its snapshot instead of showing the
    /// previous units.
    ///
    /// Identical units are a **no-op**: no revision, no generation bump, no
    /// status. A caller that re-applies the configured units (for example after
    /// an open) must not manufacture an edit.
    pub fn set_units(&mut self, units: cad_domain::UnitContext) -> CadResult<()> {
        let changed = {
            let document = self
                .application
                .workspace
                .documents
                .get_mut(&self.document_id)
                .ok_or_else(|| CadError::InvalidInput("no document is open".into()))?;
            if document.units == units {
                return Ok(());
            }
            document.units = units;
            let drawing = Arc::make_mut(&mut document.drawing);
            drawing.note_document_header_change(
                crate::app_drawing::next_transaction_id(),
                "units changed",
            );
            true
        };
        if changed {
            self.session.generation = self.session.generation.saturating_add(1);
            // The document slot is the authority; this is the host's own status
            // line (not a catalog key), like `install_snapshot`'s "opened …".
            let unit_label = self
                .application
                .workspace
                .documents
                .get(&self.document_id)
                .map(|document| document.units.label().to_string())
                .unwrap_or_else(|| "drawing units".to_string());
            self.last_status = format!("units changed: display in {unit_label}");
        }
        Ok(())
    }

    /// The current document's persistence state, if a document is open.
    pub fn persistence(&self) -> Option<&crate::PersistenceState> {
        self.application
            .workspace
            .documents
            .get(&self.document_id)
            .map(|document| &document.persistence)
    }

    /// Accumulate one commit's [`cad_db::ChangeScope`] into this document's
    /// unsaved dirty set (ADR-0037 D8/D12 Phase 2).
    ///
    /// Merge is a union, so a commit reachable through both the outer outcome and
    /// a traced command may be noted twice without changing the answer. Hosts
    /// call this for publishes that bypass the wrappers below (the engine request
    /// funnel), so there is one accumulated set rather than per-path copies.
    pub fn note_change_scope(&mut self, scope: &cad_db::ChangeScope) {
        self.dirty_scope.merge(scope);
    }

    /// The dirty set accumulated since the last save/open/close.
    ///
    /// Read-only: the saver clones this as the `dirty` half of the archive write.
    /// It is empty right after a durable save, an open, a New or a Close.
    pub fn dirty_scope(&self) -> &cad_db::ChangeScope {
        &self.dirty_scope
    }

    /// Mark the host-owned report/vendor tables dirty after an import publish.
    ///
    /// `tables/compat.json`, `tables/sources.json` and `tables/vendor.json` are
    /// written from the `Document` slot, not from `cad-db`, so no database commit
    /// would otherwise name them. Called **after**
    /// [`HostController::reset_for_new_content`], which clears the accumulated
    /// scope because it described the replaced document.
    pub(super) fn mark_import_metadata_dirty(&mut self) {
        self.dirty_scope.table(cad_db::TableKind::Compat);
        self.dirty_scope.table(cad_db::TableKind::Sources);
        self.dirty_scope.table(cad_db::TableKind::Vendor);
    }

    /// Append one export report to the document's persisted compatibility table.
    ///
    /// Called by the Save As funnel **after** the exported file is durable
    /// (`crates/cad-host/src/save.rs`), so the report of the file just written
    /// travels with the document the next time its `.ymodel` archive is saved.
    /// It is metadata about a projection, not document content: a failure to
    /// record must never fail the save that already wrote its bytes.
    ///
    /// Only the newest [`MAX_EXPORT_RECORDS`] rows are retained: repeated exports
    /// would otherwise grow the document (and its archive) without bound. The
    /// oldest rows are dropped first, so the table stays a bounded recent history
    /// rather than an ever-growing log.
    ///
    /// Returns an explicit error when no document is open: silently dropping the
    /// record would claim the report was persisted.
    pub fn record_export_report(&mut self, report: &CompatReport) -> CadResult<()> {
        let record = report_json::compat_report_to_json(report);
        let document = self
            .application
            .workspace
            .documents
            .get_mut(&self.document_id)
            .ok_or_else(|| CadError::InvalidInput("no document is open".into()))?;
        document.compat.push(record);
        // Bounded **export** history: keep every import record (the document's
        // provenance) and only the newest `MAX_EXPORT_RECORDS` export rows.
        // `retain` walks front to back, so the oldest export rows go first.
        let is_export = |row: &serde_json::Value| {
            row.get("direction").and_then(serde_json::Value::as_str) == Some("export")
        };
        let export_rows = document.compat.iter().filter(|row| is_export(row)).count();
        let mut excess = export_rows.saturating_sub(MAX_EXPORT_RECORDS);
        if excess > 0 {
            document.compat.retain(|row| {
                if excess > 0 && is_export(row) {
                    excess -= 1;
                    false
                } else {
                    true
                }
            });
        }
        self.dirty_scope.table(cad_db::TableKind::Compat);
        Ok(())
    }

    /// Record that the document's file now holds `saved` (D12 step 5).
    ///
    /// Called only after the bytes are durably on disk. The import baseline
    /// (`open_revision`) is deliberately untouched: it never advances across
    /// saves, or the L0 passthrough predicate would become satisfiable again
    /// (D2).
    ///
    /// Clearing the accumulated dirty scope here — together with
    /// [`HostController::record_saved_archive`]'s retained bytes — is what makes
    /// "the file holds this and there is nothing outstanding" one atomic fact;
    /// a write failure must never reach this method (ADR-0037 D8/D12 Phase 2).
    pub fn mark_saved(
        &mut self,
        saved: Revision,
        document_uuid: Option<String>,
        foreign_entries: Vec<String>,
    ) {
        if let Some(document) = self
            .application
            .workspace
            .documents
            .get_mut(&self.document_id)
        {
            document.persistence.saved_revision = saved;
            document.persistence.document_uuid = document_uuid;
            document.persistence.foreign_entries = foreign_entries;
        }
        self.dirty_scope = cad_db::ChangeScope::default();
    }

    /// Retain the bytes just committed to the document's own file.
    ///
    /// They become the base of the next save: its live entries are the ones a
    /// narrowed write keeps and its unknown entries are copied through. Never
    /// the save target — a Save As carries these bytes forward instead of
    /// adopting the destination file's contents (ADR-0037 D8/D11/D12 Phase 2).
    ///
    /// Called only after the same durable write as [`HostController::mark_saved`].
    pub fn record_saved_archive(&mut self, bytes: Arc<[u8]>) {
        if let Some(document) = self
            .application
            .workspace
            .documents
            .get_mut(&self.document_id)
        {
            document.persistence.archive_bytes = Some(bytes);
        }
    }

    /// Record the `sha256` of the bytes the document was last opened from or
    /// saved to, so the next in-place save can detect an external edit.
    pub fn set_source_sha256(&mut self, source_sha256: Option<String>) {
        if let Some(document) = self
            .application
            .workspace
            .documents
            .get_mut(&self.document_id)
        {
            document.persistence.source_sha256 = source_sha256;
        }
    }

    /// Whether the open document changed since its last save (D12).
    ///
    /// A closed document has no persistence state and is never dirty.
    pub fn is_dirty(&self) -> bool {
        let Some(document) = self.application.workspace.documents.get(&self.document_id) else {
            return false;
        };
        document.persistence.is_dirty(document.drawing.revision())
    }
}
