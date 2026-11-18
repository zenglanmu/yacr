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
        document
            .drawing
            .snapshot(
                document.units.clone(),
                document.identity.clone(),
                document.resource_keys.clone(),
                document.drawing.revision(),
            )
            // A marker, not prose: callers match on `ymodel.*` (D13).
            .map_err(|error| CadError::Unsupported(format!("{}: {}", error.marker, error.detail)))
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
        persistence: crate::PersistenceState,
        name_hint: Option<String>,
    ) -> CadResult<()> {
        let drawing = DrawingDatabase::from_snapshot(snapshot).map_err(|error| {
            CadError::Unsupported(format!("{}: {}", error.marker, error.detail))
        })?;
        let label = name_hint.unwrap_or_else(|| self.document_name_hint.clone());
        let document = Document {
            id: self.document_id,
            drawing: Arc::new(drawing),
            identity: snapshot.header.identity.clone(),
            units: snapshot.header.units.clone(),
            resource_keys: snapshot.header.resource_keys.clone(),
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
        // records. `Some([])` on the next tick clears any previous vendor pass.
        self.last_import_report = None;
        self.last_vendor_records = Vec::new();
        self.vendor_records_pending = true;
        // Not a localized sentence: the host owns the user-facing status line and
        // reads `document_name_hint` for it (the crate must not hardcode one).
        self.last_status = format!("opened {}", self.document_name_hint);
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

    /// Record that the document's file now holds `saved` (D12 step 5).
    ///
    /// Called only after the bytes are durably on disk. The import baseline
    /// (`open_revision`) is deliberately untouched: it never advances across
    /// saves, or the L0 passthrough predicate would become satisfiable again
    /// (D2).
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
