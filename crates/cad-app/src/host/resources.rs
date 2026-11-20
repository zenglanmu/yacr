//! The controller write path for document-bundled external resources and the
//! self-containment flag (ADR-0040 D3/D5/D13).
//!
//! `cad-db` exposes those fields only through
//! [`cad_db::DrawingDatabaseBuilder`] (construction), not through a `&mut
//! DrawingDatabase` setter, because a bundled resource is **content-addressed
//! document state** whose insert is idempotent per key. The host therefore
//! applies them through the one path the database already sanctions for a whole
//! document: snapshot → edit the two document-level fields → hydrate a new
//! database with [`cad_db::DrawingDatabase::from_snapshot`].
//!
//! That keeps the two invariants the save funnel relies on:
//!
//! * the bytes and the `tables/resources.json` mapping live in `cad-db` (D5:
//!   the archive is a serialization of the document, never a second source);
//! * the mapping rows are encoded by `cad-db` itself. A throwaway database that
//!   holds exactly the newly accepted resources is snapshotted to obtain the
//!   canonical rows, so this module never re-implements the row shape (D4).
//!
//! Every rewrite is a **document revision**: the archive's `saved_revision`
//! advances exactly as an ordinary edit, and the accumulated dirty scope names
//! the entries that changed. A second save that finds nothing new rewrites
//! nothing (content addressing makes the insert a no-op), which is what keeps a
//! repeated save from bumping the revision again.

use std::sync::Arc;

use cad_db::{DocumentSnapshot, DrawingDatabase, DrawingDatabaseBuilder, TableKind};
use cad_domain::{BundledKind, CadError, CadResult, Revision};

use super::HostController;

/// One external resource a save decided to embed (ADR-0040 D13's pure input).
///
/// `key` is the content-addressed archive key (`<sha256 前 32 hex>.<ext>`,
/// D1) — computed by the collector in `cad-host`, which owns the file I/O — and
/// `name` is the logical reference the document already uses, verbatim.
#[derive(Debug, Clone)]
pub struct BundledResource {
    pub kind: BundledKind,
    pub key: String,
    pub name: String,
    /// Collection-time path, diagnostics only (D4): never used for resolution.
    pub source: Option<String>,
    pub bytes: Arc<[u8]>,
}

impl HostController {
    /// Whether the open document packs external resources into its archive
    /// (ADR-0040 D3). `false` when no document is open.
    pub fn embed_resources(&self) -> bool {
        self.application
            .workspace
            .documents
            .get(&self.document_id)
            .is_some_and(|document| document.drawing.embed_resources())
    }

    /// Store the self-containment flag on the document (ADR-0040 D3).
    ///
    /// Returns `true` when the flag actually changed (one revision, one dirty
    /// `document.json`); an unchanged value is a documented no-op so
    /// re-applying a save-dialog choice never manufactures an edit.
    pub fn set_embed_resources(&mut self, embed: bool) -> CadResult<bool> {
        if self.embed_resources() == embed {
            return Ok(false);
        }
        let mut scope = cad_db::ChangeScope::default();
        // The policy is a `document.json` scalar (D3), not a table row.
        scope.header();
        self.rewrite_document(
            move |snapshot| {
                snapshot.header.embed_resources = embed;
                Ok(())
            },
            &scope,
        )?;
        Ok(true)
    }

    /// Insert the collected resources into the authoritative database.
    ///
    /// Returns how many `(kind, key)` entries were **new**. A key the document
    /// already carries is skipped without a write: the key is content-addressed,
    /// so the same key is the same asset (D1/D7), and skipping is what makes the
    /// second save of an unchanged document a no-op instead of a revision bump.
    ///
    /// A name or key the database would refuse (`resource.name_conflict`, a key
    /// whose bytes differ) is returned as an error; the caller reports it and
    /// keeps saving, never fails the file.
    pub fn insert_bundled_resources(&mut self, incoming: &[BundledResource]) -> CadResult<usize> {
        if incoming.is_empty() {
            return Ok(0);
        }
        let snapshot = self.snapshot_document()?;
        let fresh: Vec<&BundledResource> = incoming
            .iter()
            .filter(|resource| {
                !snapshot
                    .resources
                    .contains_key(&(resource.kind, resource.key.clone()))
            })
            .collect();
        if fresh.is_empty() {
            return Ok(0);
        }
        // `cad-db`'s own encoder produces the `tables/resources.json` rows for
        // exactly these resources (see the module docs).
        let rows = self.resource_table_rows(&fresh)?;
        let inserted = fresh.len();
        let mut scope = cad_db::ChangeScope::default();
        scope.table(TableKind::Resources);
        self.rewrite_document(
            move |snapshot| {
                for resource in &fresh {
                    let id = (resource.kind, resource.key.clone());
                    snapshot
                        .resources
                        .insert(id.clone(), Arc::clone(&resource.bytes));
                    snapshot
                        .resource_names
                        .insert(id.clone(), resource.name.clone());
                    snapshot
                        .resource_sources
                        .insert(id, resource.source.clone());
                }
                let mut merged = snapshot
                    .tables
                    .get(TableKind::Resources)
                    .map(<[_]>::to_vec)
                    .unwrap_or_default();
                merged.extend(rows.iter().cloned());
                // Re-sort into the order `cad-db` itself re-encodes on the next
                // snapshot, so this archive is byte-canonical and a reload → save
                // of it writes the same table. The order is the map key's:
                // `(BundledKind, String)` — the kind's **declaration** order
                // (`font`, `xref`, `image`; deliberately not alphabetical, where
                // `image` would come before `xref`), then the key.
                merged.sort_by(|left, right| row_id(left).cmp(&row_id(right)));
                snapshot.tables.set(TableKind::Resources, merged);
                Ok(())
            },
            &scope,
        )?;
        Ok(inserted)
    }

    /// The canonical `tables/resources.json` rows for `resources`.
    ///
    /// `cad-db` owns the row shape (D4), so the rows are read back from a
    /// throwaway database that holds exactly these entries instead of being
    /// written a second time here. That database is content-free — no entities,
    /// tables, textures or variables — so its snapshot costs one row per
    /// resource, not a pass over the document. (The row codec is crate-private in
    /// `cad-db`; a hand-written copy here is exactly the drift ADR-0040 D4's
    /// BLOCKER was about.)
    fn resource_table_rows(
        &self,
        resources: &[&BundledResource],
    ) -> CadResult<Vec<serde_json::Value>> {
        let document = self
            .application
            .workspace
            .documents
            .get(&self.document_id)
            .ok_or_else(|| CadError::InvalidInput("no document is open".into()))?;
        let mut builder = DrawingDatabaseBuilder::new(document.drawing.id());
        for resource in resources {
            builder.insert_resource(
                resource.kind,
                &resource.key,
                &resource.name,
                Arc::clone(&resource.bytes),
            )?;
            builder.set_resource_source(
                resource.kind,
                &resource.key,
                resource.source.as_deref(),
            )?;
        }
        let scratch = builder.finish()?;
        let snapshot = scratch
            .snapshot(
                document.units.clone(),
                document.identity.clone(),
                Vec::new(),
                Revision(0),
            )
            .map_err(|error| {
                CadError::Unsupported(format!("{}: {}", error.marker, error.detail))
            })?;
        Ok(snapshot
            .tables
            .get(TableKind::Resources)
            .map(<[_]>::to_vec)
            .unwrap_or_default())
    }

    /// Apply one document-level edit through snapshot → hydrate (see the module
    /// docs), as one revision, and accumulate its dirty scope.
    fn rewrite_document(
        &mut self,
        edit: impl FnOnce(&mut DocumentSnapshot) -> CadResult<()>,
        scope: &cad_db::ChangeScope,
    ) -> CadResult<()> {
        let mut snapshot = self.snapshot_document()?;
        edit(&mut snapshot)?;
        // One revision, exactly like any other commit: the next save's
        // `saved_revision` must describe these bytes.
        snapshot.revision = Revision(snapshot.revision.0.saturating_add(1));
        let rebuilt = DrawingDatabase::from_snapshot(&snapshot).map_err(|error| {
            CadError::Unsupported(format!("{}: {}", error.marker, error.detail))
        })?;
        let document = self
            .application
            .workspace
            .documents
            .get_mut(&self.document_id)
            .ok_or_else(|| CadError::InvalidInput("no document is open".into()))?;
        // Content is identical (`from_snapshot` is the database's own fixed
        // point), so ids, counters and the element rows are unchanged; only the
        // document-level fields this method was given differ.
        document.drawing = Arc::new(rebuilt);
        self.dirty_scope.merge(scope);
        // The rendering/representation keys are derived from the drawing; a new
        // revision is a new content identity, so the session republishes.
        self.session.generation = self.session.generation.saturating_add(1);
        Ok(())
    }
}

/// The `(kind, key)` a `tables/resources.json` row is ordered by.
///
/// Mirrors the map key `cad-db` encodes from (`BTreeMap<(BundledKind, String), _>`):
/// the enum's declaration order first, then the key string.
fn row_id(row: &serde_json::Value) -> (Option<BundledKind>, &str) {
    (
        row.get("kind")
            .and_then(serde_json::Value::as_str)
            .and_then(BundledKind::parse),
        row.get("key")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default(),
    )
}
