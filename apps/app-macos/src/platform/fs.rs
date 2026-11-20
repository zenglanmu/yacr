//! The macOS carrier's file-writing half: one atomic replace, one `.bak`.
//!
//! This is the POSIX sequence the Linux and Windows carriers also implement, in
//! their own file layer, because the three apps are separate binary crates and
//! the shared `cad-*` layers must not own a filesystem policy:
//!
//! ```text
//! temp in the target directory -> write -> sync_all
//!   -> hard-link (or copy) the previous file to `<name>.bak`
//!   -> rename the temp over the target (atomic on APFS/HFS+)
//!   -> best-effort fsync of the parent directory
//! ```
//!
//! A failure before the rename leaves the old bytes in place: the temp file is
//! removed and the target is untouched. The backup is taken **before** the
//! rename, never by renaming the target away, because a window in which the
//! document does not exist at all is worse than no backup.
//!
//! `keep_backup` distinguishes a document save (ADR-0041 D4: overwriting the
//! opened source is allowed, and `.bak` is what protects the previous bytes) from
//! the config/preference writes, which ask for no backup.
//!
//! Target-independent on purpose: the whole contract is unit-tested on a
//! non-macOS host, where only the AppKit panels need the real target.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use cad_domain::{CadError, CadResult};

/// Distinguishes concurrent temp names within one process.
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Write `bytes` to `path` atomically, optionally keeping the previous bytes as
/// `<name>.bak`.
///
/// A backup that cannot be created does **not** fail the save: the rename is
/// still atomic and the user's edit is the thing at risk. It is reported on
/// stderr instead of being swallowed, because "saved with no backup" and "saved"
/// are different contracts.
///
/// `pub(crate)` so the recovery store ([`crate::recovery_store`]) uses this one
/// implementation instead of a second, subtly different one: a carrier-private
/// snapshot needs the same temp → sync → rename sequence as a document save, and
/// two copies could disagree about durability.
pub(crate) fn atomic_replace(path: &Path, bytes: &[u8], keep_backup: bool) -> CadResult<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("yacr");
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(".{stem}.{}.{sequence}.tmp", std::process::id()));
    // The backup must describe the file we are about to replace, so it is taken
    // before the rename and skipped for a first save (nothing to back up).
    if keep_backup && path.exists() {
        if let Err(reason) = backup_previous(path) {
            eprintln!(
                "yacr: warning: saved {} without a .bak ({reason})",
                path.display()
            );
        }
    }
    if let Err(error) = write_then_rename(&tmp, path, bytes) {
        // Never leave a partial temp file behind, whichever step failed.
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    // Best effort: the rename already published the bytes, so a directory fsync
    // failure is not a write failure (it only weakens crash durability).
    if let Ok(directory) = std::fs::File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

/// Refresh `path.bak` from the current `path`.
///
/// A hard link is preferred: it is atomic, costs no extra bytes and shares the
/// inode, so the backup is byte-identical by construction. Filesystems without
/// hard links (some network/removable volumes) fall back to a copy.
fn backup_previous(path: &Path) -> Result<(), String> {
    let backup = path.with_extension(match path.extension().and_then(|ext| ext.to_str()) {
        Some(extension) => format!("{extension}.bak"),
        None => "bak".to_string(),
    });
    // A stale backup must not block a fresh one: `hard_link` refuses an existing
    // destination, and `copy` would silently keep the older bytes if it did not
    // truncate.
    let _ = std::fs::remove_file(&backup);
    match std::fs::hard_link(path, &backup) {
        Ok(()) => Ok(()),
        Err(link_error) => std::fs::copy(path, &backup)
            .map(|_| ())
            .map_err(|copy_error| format!("link: {link_error}; copy: {copy_error}")),
    }
}

/// The fallible half of [`atomic_replace`]: create, write and sync the temp file,
/// then rename it over the target. Every error names the path it failed on.
fn write_then_rename(tmp: &Path, path: &Path, bytes: &[u8]) -> CadResult<()> {
    let mut file = std::fs::File::create(tmp).map_err(|error| {
        CadError::InvalidInput(format!(
            "create temp file {} failed: {error}",
            tmp.display()
        ))
    })?;
    file.write_all(bytes).map_err(|error| {
        CadError::InvalidInput(format!("write {} failed: {error}", tmp.display()))
    })?;
    file.sync_all().map_err(|error| {
        CadError::InvalidInput(format!("sync {} failed: {error}", tmp.display()))
    })?;
    drop(file);
    std::fs::rename(tmp, path).map_err(|error| {
        CadError::InvalidInput(format!(
            "rename {} to {} failed: {error}",
            tmp.display(),
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A fresh directory under the system temp dir (no `tempfile` dependency in
    /// this crate); removed again when the test ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> TempDir {
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "yacr-macos-fs-{label}-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temp dir");
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn replace_keeps_the_previous_bytes_as_bak_and_publishes_the_new_ones() {
        let temp = TempDir::new("replace");
        let target = temp.0.join("plan.ymodel");
        std::fs::write(&target, b"old bytes").expect("seed");

        atomic_replace(&target, b"new bytes", true).expect("written");

        assert_eq!(std::fs::read(&target).unwrap(), b"new bytes");
        assert_eq!(
            std::fs::read(temp.0.join("plan.ymodel.bak")).unwrap(),
            b"old bytes",
            "the backup names the *previous* document"
        );
        // No temp file survives a successful replace.
        let leftovers: Vec<_> = std::fs::read_dir(&temp.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
    }

    #[test]
    fn a_first_save_writes_no_backup_and_no_backup_means_no_bak() {
        let temp = TempDir::new("first");
        let target = temp.0.join("plan.ymodel");
        atomic_replace(&target, b"first bytes", true).expect("written");
        assert_eq!(std::fs::read(&target).unwrap(), b"first bytes");
        assert!(
            !temp.0.join("plan.ymodel.bak").exists(),
            "there was no previous document to back up"
        );

        // `keep_backup = false` (config/preference writes) never creates one.
        let pref = temp.0.join("preference.json");
        atomic_replace(&pref, b"{\"a\":1}", false).expect("written");
        atomic_replace(&pref, b"{\"a\":2}", false).expect("rewritten");
        assert!(!temp.0.join("preference.json.bak").exists());
        assert_eq!(std::fs::read(&pref).unwrap(), b"{\"a\":2}");
    }

    #[test]
    fn a_failed_write_leaves_the_target_untouched() {
        let temp = TempDir::new("failed");
        // A file name at the POSIX NAME_MAX boundary: the target itself is
        // creatable, but the temp name (`.<stem>.<pid>.<seq>.tmp`) is not, so the
        // failure happens before the rename on every POSIX filesystem.
        let name = format!("{}.ymodel", "a".repeat(240));
        let target = temp.0.join(name);
        std::fs::write(&target, b"old bytes").expect("seed");

        let error = atomic_replace(&target, b"new bytes", true).expect_err("must fail");
        assert!(
            error.to_string().contains("create temp file"),
            "failure names the step: {error}"
        );
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"old bytes",
            "a failed write must not touch the target"
        );
    }
}
