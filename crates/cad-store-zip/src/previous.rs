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
//! Failure to read the previous archive is
//! [`cad_domain::ymodel::markers::FOREIGN_ENTRIES_UNREADABLE`], never a save that
//! quietly drops what it could not understand.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use zip::ZipArchive;

use cad_db::StoreError;
use cad_domain::ymodel::markers;

use crate::json::{malformed, Sha256Sum};
use crate::manifest::{EntryRecord, Manifest};
use crate::MANIFEST_ENTRY;

/// What one previous archive says about its entries.
#[derive(Debug, Clone, Default)]
pub(crate) struct PreviousArchive {
    /// The foreign entries, sorted by path.
    foreign: Vec<String>,
    /// The source manifest's records, for the paths it described.
    records: BTreeMap<String, EntryRecord>,
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
        Ok(PreviousArchive { foreign, records })
    }

    /// The paths to copy, sorted.
    pub(crate) fn foreign(&self) -> &[String] {
        &self.foreign
    }

    /// A source record that can be reused verbatim.
    ///
    /// Only a record with a compressed-byte hash is reusable: that is the one the
    /// next reader will check instead of decompressing, and the value is only
    /// true because the compressed bytes are copied unchanged.
    pub(crate) fn reusable(&self, path: &str) -> Option<&EntryRecord> {
        self.records
            .get(path)
            .filter(|record| record.sha256_c.is_some())
    }
}

/// Read [`PreviousArchive::foreign`] without keeping the archive open.
pub(crate) fn open(bytes: &[u8]) -> Result<ZipArchive<Cursor<&[u8]>>, StoreError> {
    ZipArchive::new(Cursor::new(bytes)).map_err(unreadable)
}

/// The manifest record for one entry, computed from the bytes themselves.
///
/// Needed when a foreign entry is **not** described by the source manifest: the
/// next archive still has to record a `sha256`/`len`/`crc` for it, and inventing
/// those is not an option. The pass is streaming — the entry is never buffered,
/// so a preserved source file of any size is fine.
///
/// `sha256_c` stays `None`. The compressed bytes cannot be hashed through the zip
/// crate's public API, and the reader's rule already covers that case: with no
/// `sha256_c` it verifies by decompressing and checking `sha256`/`len`
/// (`crate::read`). Recording a value that was not computed would be worse than
/// recording none.
pub(crate) fn measure(bytes: &[u8], path: &str) -> Result<EntryRecord, StoreError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(unreadable)?;
    let (crc, len) = {
        let file = archive.by_name(path).map_err(unreadable)?;
        (file.crc32(), file.size())
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
        sha256,
        sha256_c: None,
        len,
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
}
