//! The entries this build does not model, and how they survive a save
//! (ADR-0037 D11).
//!
//! Two cases reach here:
//!
//! 1. an entry the previous archive's manifest does **not** list, or lists with
//!    `"foreign": true` — another writer's data, or a class this build has no
//!    code for. It is never parsed;
//! 2. the bytes to copy it from.
//!
//! The store therefore keeps the previous archive's **bytes** when one has
//! foreign entries ([`crate::ZipDocumentStore::with_previous`]) and opens it here
//! twice: once to learn what is foreign, once while writing to copy each entry
//! with `raw_copy_file` (compressed bytes carried over, so a foreign blob cannot
//! be re-compressed into a slightly different archive).
//!
//! The same scan also answers the raw-copy question for **modelled** entries: a
//! shard the commit did not touch can be carried over at the compressed-byte
//! level too (ADR-0037 D11 Phase 2), using the source manifest's `shard_map` and
//! entry record. Only entries the source manifest describes as non-foreign are
//! eligible; everything else in the archive remains foreign data. A copied entry
//! **inherits its source record verbatim** (codec, `sha256`, `sha256_c`, `len`,
//! `len_c`, `crc`): `raw_copy_file` carries the stored payload over unchanged, so
//! every field still describes the bytes in the new archive. A foreign entry the
//! source manifest does not describe is measured once here and recorded as
//! [`Codec::Opaque`].
//!
//! Failure to read the previous archive is
//! [`cad_domain::ymodel::markers::FOREIGN_ENTRIES_UNREADABLE`], never a save that
//! quietly drops what it could not understand.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use zip::ZipArchive;

use cad_db::StoreError;
use cad_domain::ymodel::markers;

use crate::json::{malformed, Sha256Sum};
use crate::manifest::{Codec, EntryRecord, Manifest};
use crate::MANIFEST_ENTRY;

/// What one previous archive says about its entries.
#[derive(Debug, Clone, Default)]
pub(crate) struct PreviousArchive {
    /// The foreign entries, sorted by path.
    foreign: Vec<String>,
    /// The source manifest's records, for the paths it described.
    records: BTreeMap<String, EntryRecord>,
    /// Every entry path the source manifest lists in `shard_map`.
    shards: BTreeSet<String>,
}

impl PreviousArchive {
    /// Scan an archive for the entries this build must carry over.
    ///
    /// # Errors
    ///
    /// [`markers::FOREIGN_ENTRIES_UNREADABLE`] when the bytes are not a zip, when
    /// `manifest.json` is missing, or when it does not parse. A caller holding a
    /// previous archive it cannot read must not proceed with a save, because
    /// dropping its unknown entries is exactly what D11 forbids.
    pub(crate) fn scan(bytes: &[u8]) -> Result<PreviousArchive, StoreError> {
        let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(unreadable)?;
        let names = entry_names(&mut archive)?;
        let manifest = read_manifest(&mut archive)?;

        let mut foreign = Vec::new();
        for name in names {
            if name == MANIFEST_ENTRY {
                continue;
            }
            match manifest.entry(&name) {
                Some(record) if !record.foreign => {}
                _ => foreign.push(name),
            }
        }
        foreign.sort();

        let records = manifest
            .entries
            .iter()
            .map(|record| (record.path.clone(), record.clone()))
            .collect();
        let shards = manifest.shard_map.values().flatten().cloned().collect();
        Ok(PreviousArchive {
            foreign,
            records,
            shards,
        })
    }

    /// The paths to copy, sorted.
    pub(crate) fn foreign(&self) -> &[String] {
        &self.foreign
    }

    /// Whether the source manifest lists `path` as one of its element shards.
    ///
    /// `shard_map` is the authority for what an archive holds, but it is not the
    /// authority for what this build can copy: [`Self::record`] must also describe
    /// the entry, and a path listed only in `shard_map` without a record is left
    /// to be re-encoded.
    pub(crate) fn lists_shard(&self, path: &str) -> bool {
        self.shards.contains(path)
    }

    /// The source manifest's record for a modelled (non-foreign) entry.
    ///
    /// A shard copied from the source archive carries its stored payload over
    /// unchanged (`raw_copy_file`), so the record's `sha256`/`sha256_c`/`len`/
    /// `len_c`/`crc` all still describe the copied entry and are inherited
    /// verbatim by the next save. The next reader verifies them (`crate::read`),
    /// which is why a corrupt source record surfaces as a load failure instead of
    /// being trusted.
    pub(crate) fn record(&self, path: &str) -> Option<&EntryRecord> {
        self.records.get(path).filter(|record| !record.foreign)
    }

    /// The source record of any entry a raw copy may take, foreign or modelled.
    ///
    /// A foreign entry is described by the source manifest only when it was
    /// listed there (as foreign); an unlisted one is measured from the bytes
    /// instead. Either way the record describes the very payload
    /// `raw_copy_file` copies, so it is reused verbatim rather than re-derived.
    pub(crate) fn record_for_copy(&self, path: &str) -> Option<&EntryRecord> {
        self.records.get(path)
    }
}

/// Read [`PreviousArchive::foreign`] without keeping the archive open.
pub(crate) fn open(bytes: &[u8]) -> Result<ZipArchive<Cursor<&[u8]>>, StoreError> {
    ZipArchive::new(Cursor::new(bytes)).map_err(unreadable)
}

/// The manifest record for one entry, measured from the bytes themselves.
///
/// Needed when a foreign entry is **not** described by the source manifest: the
/// next archive still has to record what describes it, and inventing values is not
/// an option. The `sha256` pass is streaming — the entry is never buffered, so a
/// preserved source file of any size is fine.
///
/// The record is [`Codec::Opaque`] with `sha256_c: None`: this entry is copied,
/// never decoded, so this build does not claim a stored-payload digest it has not
/// computed. `len` is the entry's logical size and `len_c` its stored size, both
/// from the central directory (and both preserved by `raw_copy_file`).
pub(crate) fn measure(bytes: &[u8], path: &str) -> Result<EntryRecord, StoreError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(unreadable)?;
    let (crc, len, len_c) = {
        let file = archive.by_name(path).map_err(unreadable)?;
        (file.crc32(), file.size(), file.compressed_size())
    };
    let sha256 = {
        let mut file = archive.by_name(path).map_err(unreadable)?;
        let mut hasher = Sha256Sum::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(unreadable)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        hasher.hex()
    };
    Ok(EntryRecord {
        path: path.to_string(),
        codec: Codec::Opaque,
        sha256,
        sha256_c: None,
        len,
        len_c,
        crc,
        foreign: true,
    })
}

/// Every entry name, in archive order and validated as UTF-8.
fn entry_names(archive: &mut ZipArchive<Cursor<&[u8]>>) -> Result<Vec<String>, StoreError> {
    let mut names = Vec::with_capacity(archive.len());
    for name in archive.file_names() {
        names.push(name.map_err(unreadable)?.into_owned());
    }
    Ok(names)
}

/// The previous archive's `manifest.json`.
fn read_manifest(archive: &mut ZipArchive<Cursor<&[u8]>>) -> Result<Manifest, StoreError> {
    let mut file = archive.by_name(MANIFEST_ENTRY).map_err(unreadable)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(unreadable)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(unreadable)?;
    Manifest::from_json_value(&value).map_err(unreadable)
}

/// Any failure while reading what must be preserved.
pub(crate) fn unreadable(error: impl std::fmt::Display) -> StoreError {
    StoreError::new(
        markers::FOREIGN_ENTRIES_UNREADABLE,
        format!("cannot read the previous archive: {error}"),
    )
}

/// A foreign entry that would collide with a live one.
pub(crate) fn conflict(path: &str) -> StoreError {
    malformed(
        "foreign entry",
        format!("{path:?} collides with an entry this save writes"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_previous_archive_that_is_not_a_zip_is_unreadable() {
        let error = PreviousArchive::scan(b"not a zip").expect_err("garbage");
        assert_eq!(error.marker, markers::FOREIGN_ENTRIES_UNREADABLE);
    }

    #[test]
    fn measuring_an_unreadable_entry_is_an_error_not_a_guess() {
        let error = measure(b"not a zip", "raw/x.bin").expect_err("garbage");
        assert_eq!(error.marker, markers::FOREIGN_ENTRIES_UNREADABLE);
    }

    #[test]
    fn a_measured_foreign_entry_is_opaque_and_records_both_lengths() {
        use std::io::Write as _;

        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            zip.start_file(
                "raw/x.bin",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .expect("start");
            zip.write_all(b"hello").expect("write");
            zip.finish().expect("finish");
        }
        let bytes = buffer.into_inner();
        let record = measure(&bytes, "raw/x.bin").expect("measure");
        assert_eq!(record.codec, Codec::Opaque);
        assert_eq!(
            record.sha256_c, None,
            "an opaque entry has no stored digest"
        );
        assert!(record.foreign);
        assert_eq!(record.len, 5);
        assert_eq!(record.len_c, 5);
    }
}
