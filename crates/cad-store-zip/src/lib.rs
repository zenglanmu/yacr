//! The `.ymodel` archive store: zip container + sharded JSON (ADR-0037 D11/D12).
//!
//! This crate is the only implementation of [`cad_db::DocumentStore`]. Its whole
//! job is **bytes ⇄ [`DocumentSnapshot`]**: it never sees a
//! [`cad_db::DrawingDatabase`], never touches a path, and never does the atomic
//! replace, `.bak`, temp naming or mtime conflict check of ADR-0037 D12 — those
//! belong to the carrier adapter next to the file handle, and are the reason
//! [`cad_db::ByteSink`] exists. Everything the *archive* must guarantee is here.
//!
//! # Container layout
//!
//! ```text
//! manifest.json                 -- the table of contents; always the LAST entry
//! document.json                 -- DocumentHeader (units, identity, id counters)
//! schema/classes.json           -- the class registry materialised from CAD_DOMAIN
//! tables/models.json            -- Vec<ModelRecord>
//! tables/<table>.json           -- one per present TableKind (cad_db::table_entry)
//! elements/<model>-<shard>.json -- elements of one shard, sorted by id
//! textures/<key>                -- document base-colour textures (ADR-0038 D4)
//! raw/<anything>                -- foreign/preserved entries (never parsed)
//! ```
//!
//! `manifest.json` is deliberately **not** listed in its own `entries`: a
//! self-referential hash has no fixed point. Its integrity is the zip CRC.
//!
//! # Integrity
//!
//! Every entry the build models records, in the manifest:
//!
//! * `sha256` — lowercase hex of the **uncompressed** bytes (the format
//!   contract: any reader can recompute it);
//! * `len` — the uncompressed length;
//! * `crc` — the zip CRC-32 of the uncompressed bytes (cross-checked against the
//!   central directory on read);
//! * `sha256_c` — lowercase hex of the **compressed** bytes, or `null`.
//!
//! `sha256_c` is `Option` on purpose. Encoding streams each entry through the zip
//! writer, so the compressed bytes of an entry it *encodes* never exist as a
//! buffer the store could hash: computing `sha256_c` would mean buffering the whole
//! archive first, which the chunked-sink rule (ADR-0037 D16, the web memory
//! budget) forbids. Freshly encoded entries therefore write `"sha256_c": null`.
//! Raw-copied entries (a foreign blob, or an untouched shard in Phase 2) carry
//! their compressed bytes over unchanged, but this build still cannot hash those
//! bytes — `zip` keeps its raw reader crate-private — so a copied shard records
//! `null` rather than claim a value it did not compute. (A foreign entry whose
//! source manifest already recorded `sha256_c` keeps that value: it describes the
//! very bytes being copied.) Every entry is verified on read by decompressing and
//! checking `sha256`/`len`/CRC; a copy whose source record was corrupt therefore
//! fails the open instead of being trusted.
//!
//! # Determinism
//!
//! Two writes of the same snapshot produce byte-identical archives: entries are
//! emitted in sorted path order, every zip entry gets [`zip::DateTime::default`],
//! JSON entries are always `Deflated` and foreign copies are byte-identical.
//! There is one apparent exception — `saved_at` — and it is handled by not being
//! a clock: see [`ZipDocumentStore::with_saved_at`].
//!
//! # Foreign entries
//!
//! An entry this build does not model (not listed in the previous manifest, or
//! listed with `foreign: true`) is **never parsed, never re-encoded and never
//! dropped**. It is copied with [`zip::ZipWriter::raw_copy_file`] on the next
//! save, which needs the previous archive: give it to the store with
//! [`ZipDocumentStore::with_previous`], or let [`DocumentStore::read`] do it —
//! a read that finds foreign entries keeps that archive so the next write can
//! preserve them. If a foreign entry cannot be read back, the save fails with
//! [`cad_domain::ymodel::markers::FOREIGN_ENTRIES_UNREADABLE`] rather than
//! producing an archive that silently lost data (ADR-0037 D11).
//!
//! A preserved entry is still *checked*, just never interpreted: the manifest
//! records its zip CRC and length, and opening the archive again verifies both
//! against the central directory, so a corrupted preserved blob is refused
//! instead of being carried forward. The payload itself is only compared when the
//! source manifest already described it (`sha256_c`, copied verbatim); an entry
//! that nobody described is measured once (`sha256`, `len`, `crc`) so the next
//! archive records real values rather than placeholders.
//!
//! # Budget
//!
//! [`ZipDocumentStore::with_max_bytes`] caps the cumulative written bytes (the
//! web adapter sets 256 MB, ADR-0037 D16). The check runs per chunk and calls
//! [`cad_db::ByteSink::abort`] the moment the budget is exceeded, so a browser
//! never has to allocate the oversized archive first.

mod crc32;
mod json;
mod manifest;
mod models;
mod previous;
mod read;
mod schema;
mod write;

use cad_db::{
    ByteSink, ByteSource, ChangeScope, DocumentSnapshot, DocumentStore, LoadedDocument, SaveReport,
    StoreError,
};

/// The archive's table of contents.
pub const MANIFEST_ENTRY: &str = "manifest.json";

/// The `manifest.generator` string this build writes.
pub(crate) const GENERATOR: &str = concat!("yacr/", env!("CARGO_PKG_VERSION"));

/// A [`DocumentStore`] over zip + sharded JSON.
///
/// # Configuration
///
/// * [`ZipDocumentStore::with_max_bytes`] — the serialization budget.
/// * [`ZipDocumentStore::with_previous`] — the archive whose foreign entries
///   must survive this save (see the crate docs).
/// * [`ZipDocumentStore::with_document_uuid`] / [`with_saved_at`] — the two
///   manifest fields the store cannot derive honestly; leave them unset and the
///   store stays a pure function of the snapshot.
///
/// [`with_saved_at`]: ZipDocumentStore::with_saved_at
#[derive(Debug, Clone, Default)]
pub struct ZipDocumentStore {
    max_bytes: Option<u64>,
    previous: Option<Vec<u8>>,
    open_revision: Option<u64>,
    document_uuid: Option<String>,
    saved_at: Option<String>,
}

impl ZipDocumentStore {
    /// A store with no budget, no previous archive and no timestamps.
    pub fn new() -> ZipDocumentStore {
        ZipDocumentStore::default()
    }

    /// Cap the number of bytes this store may hand to the sink.
    ///
    /// Exceeding it aborts the sink mid-stream and returns
    /// [`cad_domain::ymodel::markers::PERSISTENCE_TOO_LARGE`]. `None` (the
    /// default) means no limit; native carriers set none because the file system
    /// already refuses an oversized write, and the web adapter sets 256 MB
    /// (ADR-0037 D16).
    pub fn with_max_bytes(mut self, max_bytes: Option<u64>) -> ZipDocumentStore {
        self.max_bytes = max_bytes;
        self
    }

    /// The bytes of the archive this write copies unchanged entries from.
    ///
    /// Foreign entries **must** be carried over from it, and element shards the
    /// commit did not touch (`dirty`) are copied from it too rather than being
    /// re-encoded. Without previous bytes every entry is encoded, which is the
    /// first-save case.
    ///
    /// [`DocumentStore::read`] sets this automatically when the opened archive
    /// held entries this build does not model, so the usual carrier never calls
    /// it. It exists for the case where the previous archive was read by another
    /// store instance in the same session, and for a carrier that keeps the
    /// archive to make later saves incremental. Passing bytes that do not parse as
    /// a `.ymodel` archive makes `write` fail with
    /// [`cad_domain::ymodel::markers::FOREIGN_ENTRIES_UNREADABLE`]: dropping the
    /// unknown entries silently would be worse than refusing the save.
    pub fn with_previous(mut self, previous: Option<Vec<u8>>) -> ZipDocumentStore {
        self.previous = previous;
        self
    }

    /// Pin `manifest.document_uuid`.
    ///
    /// When unset, the store derives a UUID-shaped identifier from the
    /// snapshot's [`cad_domain::DocumentIdentity`] — deterministically, so two
    /// writes of one snapshot stay byte-identical. That derivation is *stable
    /// across saves*, which is what D3 asks for between a document and its
    /// saved copy; Save-As mints a new identity anyway, and a carrier that wants
    /// to pin a federation GUID sets it here.
    pub fn with_document_uuid(mut self, document_uuid: Option<String>) -> ZipDocumentStore {
        self.document_uuid = document_uuid;
        self
    }

    /// Pin `manifest.open_revision`, the import baseline (ADR-0037 D2).
    ///
    /// The baseline **never advances**: it is the revision the document had when
    /// it was imported, and `revision == open_revision` is the mechanical "this
    /// is still the source bytes" predicate behind L0 passthrough. The snapshot
    /// does not carry it (it lives beside the file in the carrier), so the store
    /// has to be told. [`DocumentStore::read`] sets it from the archive, which
    /// makes a load → save cycle preserve it.
    ///
    /// When never set, the store writes `u64::MAX` (`write::NO_IMPORT_BASELINE`),
    /// which no real revision equals — a document whose baseline is unknown
    /// therefore never claims L0 passthrough. Defaulting to the snapshot's own
    /// revision would do the opposite and is the B1 bug the ADR pins: an edited
    /// document would claim to still be its source file.
    pub fn with_open_revision(mut self, open_revision: Option<u64>) -> ZipDocumentStore {
        self.open_revision = open_revision;
        self
    }

    /// Pin `manifest.saved_at` (an opaque, caller-formatted timestamp).
    ///
    /// The store has no clock, and taking one here would break the
    /// byte-identical determinism above for no gain: `saved_at` is carrier
    /// metadata, and [`DocumentStore::read`] keeps whatever the archive said so a
    /// load → save cycle does not rewrite it.
    pub fn with_saved_at(mut self, saved_at: Option<String>) -> ZipDocumentStore {
        self.saved_at = saved_at;
        self
    }

    /// The budget currently configured.
    pub fn max_bytes(&self) -> Option<u64> {
        self.max_bytes
    }

    /// The previous archive currently held for foreign-entry preservation.
    pub fn previous_archive(&self) -> Option<&[u8]> {
        self.previous.as_deref()
    }
}

impl DocumentStore for ZipDocumentStore {
    fn read(&mut self, source: &mut dyn ByteSource) -> Result<LoadedDocument, StoreError> {
        let bytes = source.read_all()?;
        let parsed = read::parse(&bytes)?;
        // Keep the archive only when it is needed: a document with no foreign
        // entries must not pay for a second copy of its own bytes.
        self.previous = if parsed.loaded.manifest.foreign_entries.is_empty() {
            None
        } else {
            Some(bytes)
        };
        // Preserve the manifest fields the store cannot derive, so a load → save
        // round trip does not re-mint them.
        self.open_revision = Some(parsed.loaded.manifest.open_revision.0);
        self.document_uuid = Some(parsed.loaded.manifest.document_uuid.clone());
        self.saved_at = parsed.saved_at;
        Ok(parsed.loaded)
    }

    fn write(
        &mut self,
        snapshot: &DocumentSnapshot,
        dirty: &ChangeScope,
        sink: &mut dyn ByteSink,
    ) -> Result<SaveReport, StoreError> {
        // The live entry set is
        // `shards ∪ tables_present ∪ models ∪ header ∪ classes ∪ foreign`.
        // `dirty` decides which shards may be raw-copied from `self.previous`
        // instead of re-encoded (ADR-0037 D11 Phase 2); everything else is
        // rewritten, so an entry that is no longer live cannot survive.
        write::encode(
            self.max_bytes,
            self.previous.as_deref(),
            dirty,
            self.open_revision,
            self.document_uuid.as_deref(),
            self.saved_at.as_deref(),
            snapshot,
            sink,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A carrier holds the store behind `Box<dyn DocumentStore>` and moves it
    // between threads: the handle must be `Send + Sync` (ADR-0037 D1).
    #[allow(dead_code)]
    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn the_store_handle_is_send_and_sync() {
        assert_send_sync::<ZipDocumentStore>();
    }

    #[test]
    fn configuration_is_chainable_and_readable() {
        let store = ZipDocumentStore::new()
            .with_max_bytes(Some(1024))
            .with_document_uuid(Some("11111111-2222-8333-8444-555555555555".into()))
            .with_saved_at(Some("2026-11-18T00:00:00Z".into()));
        assert_eq!(store.max_bytes(), Some(1024));
        assert_eq!(store.previous_archive(), None);
        let store = store.with_previous(Some(vec![1, 2, 3]));
        assert_eq!(store.previous_archive(), Some(&[1u8, 2, 3][..]));
        // Clearing a pinned value is expressible.
        let store = store.with_document_uuid(None);
        assert_eq!(store.document_uuid, None);
    }
}
