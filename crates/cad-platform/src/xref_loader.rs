//! Host-side carriers for external-reference (XREF) file access.
//!
//! `cad-resources`/`cad-import-acadrust` decide *whether* a reference path is
//! allowed and *what* to do with the bytes; the concrete carrier lives here.
//! Native hosts read real files (the CLI/headless tool included), while a wasm
//! carrier without a host bridge reports "no filesystem" explicitly with
//! [`NoXrefSource`] instead of faking an empty drawing.

use std::path::Path;
use std::sync::Arc;

use cad_resources::{XrefFileSource, XrefSourceError};

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

/// A carrier for runtimes with no filesystem/browser bridge.
///
/// `Ok(None)` is the explicit "no filesystem" answer: the importer records
/// every reference as `browser-unavailable`, never as a silently empty block.
/// A host that cannot read referenced files (wasm without a bridge) must use
/// this rather than claiming a missing file.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoXrefSource;

impl XrefFileSource for NoXrefSource {
    fn read(&self, _path: &Path) -> Result<Option<Arc<[u8]>>, XrefSourceError> {
        Ok(None)
    }
}

/// Native filesystem carrier restricted to a set of allowed roots.
///
/// Every read canonicalizes both the root and the candidate and requires the
/// canonical candidate to be an existing **file** under the canonical root.
/// Canonicalization follows symlinks, so a link *inside* a root that points
/// outside it is refused: lexical containment alone would let an attacker (or a
/// stale drawing) escape the allowed tree.
#[cfg(not(target_arch = "wasm32"))]
pub struct FsXrefSource {
    roots: Vec<PathBuf>,
}

#[cfg(not(target_arch = "wasm32"))]
impl FsXrefSource {
    /// Build a source restricted to `roots`. Empty root entries are ignored.
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots: roots
                .into_iter()
                .filter(|root| !root.as_os_str().is_empty())
                .collect(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl XrefFileSource for FsXrefSource {
    fn read(&self, path: &Path) -> Result<Option<Arc<[u8]>>, XrefSourceError> {
        let mut outside_roots = false;
        for root in &self.roots {
            // A root that does not exist (or cannot be resolved) cannot vouch
            // for anything; skip it rather than widening the policy.
            let Ok(canonical_root) = root.canonicalize() else {
                continue;
            };
            // An absolute reference resolves as given. A path that already
            // starts with the root is used as-is: re-joining it would produce
            // `root/root/...` when the caller's drawing directory was relative.
            // Only a truly root-relative path is joined under the root.
            let candidate = if path.is_absolute() || path.starts_with(root) {
                path.to_path_buf()
            } else {
                root.join(path)
            };
            // A missing/unresolvable candidate stays "not found" below; it is
            // never turned into an empty successful read.
            let Ok(canonical) = candidate.canonicalize() else {
                continue;
            };
            if !canonical.starts_with(&canonical_root) {
                // It resolved to a real path, just not one we may read.
                outside_roots = true;
                continue;
            }
            let metadata = std::fs::metadata(&canonical).map_err(|error| {
                XrefSourceError::Unreadable(format!(
                    "xref metadata failed for {}: {error}",
                    path.display()
                ))
            })?;
            if !metadata.is_file() {
                // An existing directory/socket under a root is not an xref.
                continue;
            }
            let bytes = std::fs::read(&canonical).map_err(|error| {
                XrefSourceError::Unreadable(format!(
                    "xref read failed for {}: {error}",
                    path.display()
                ))
            })?;
            return Ok(Some(Arc::from(bytes.into_boxed_slice())));
        }
        if outside_roots {
            Err(XrefSourceError::Refused(format!(
                "xref canonical path outside allowed roots: {}",
                path.display()
            )))
        } else {
            Err(XrefSourceError::Missing(format!(
                "xref file not found: {}",
                path.display()
            )))
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "yacr-xref-loader-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// A directory under `target/` reached by a *relative* path from the test's
    /// current directory. Used for the relative-root regression.
    fn relative_temp_dir(label: &str) -> (PathBuf, PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let relative = PathBuf::from("target").join(format!(
            "yacr-xref-relative-{label}-{}-{nanos}",
            std::process::id()
        ));
        let absolute = std::env::current_dir()
            .expect("current dir")
            .join(&relative);
        fs::create_dir_all(&absolute).expect("create relative temp dir");
        (relative, absolute)
    }

    #[test]
    fn loads_relative_and_absolute_paths_under_the_root() {
        let root = temp_dir("under-root");
        let file = root.join("ext.dwg");
        fs::write(&file, [1u8, 2, 3]).expect("write xref");
        let source = FsXrefSource::new(vec![root.clone()]);

        let relative = Path::new("ext.dwg");
        assert_eq!(
            source.read(relative).expect("relative read"),
            Some(Arc::from([1u8, 2, 3].as_slice()))
        );
        assert_eq!(
            source.read(&file).expect("absolute read"),
            Some(Arc::from([1u8, 2, 3].as_slice()))
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn path_already_under_a_relative_root_is_not_rejoined() {
        let (relative, absolute) = relative_temp_dir("no-rejoin");
        fs::write(absolute.join("ext.dwg"), [4u8]).expect("write xref");
        let source = FsXrefSource::new(vec![relative.clone()]);

        // The path already contains the root; joining it again would look under
        // `root/root/ext.dwg` and fail.
        let path = relative.join("ext.dwg");
        assert_eq!(
            source.read(&path).expect("path under relative root"),
            Some(Arc::from([4u8].as_slice()))
        );

        fs::remove_dir_all(&absolute).ok();
    }

    #[test]
    fn missing_file_is_missing_not_external_unsupported() {
        let root = temp_dir("missing");
        let source = FsXrefSource::new(vec![root.clone()]);
        let error = source
            .read(Path::new("does-not-exist.dwg"))
            .expect_err("missing file must be an error");
        assert!(
            matches!(&error, XrefSourceError::Missing(text) if text.contains("xref file not found")),
            "unexpected error: {error:?}"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn path_outside_the_root_is_refused_not_missing() {
        let root = temp_dir("outside-root");
        let other = temp_dir("outside-other");
        let file = other.join("other.dwg");
        fs::write(&file, [9u8]).expect("write outside xref");
        let source = FsXrefSource::new(vec![root.clone()]);

        let error = source
            .read(&file)
            .expect_err("outside path must be refused");
        assert!(
            matches!(&error, XrefSourceError::Refused(text)
                if text.contains("xref canonical path outside allowed roots")),
            "unexpected error: {error:?}"
        );

        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&other).ok();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escaping_the_root_is_refused() {
        let root = temp_dir("symlink-root");
        let other = temp_dir("symlink-other");
        let target = other.join("secret.dwg");
        fs::write(&target, [7u8; 4]).expect("write target");
        let link = root.join("escape.dwg");
        std::os::unix::fs::symlink(&target, &link).expect("create symlink");
        let source = FsXrefSource::new(vec![root.clone()]);

        let error = source
            .read(&link)
            .expect_err("symlink escaping the root must be refused");
        assert!(
            matches!(&error, XrefSourceError::Refused(text)
                if text.contains("xref canonical path outside allowed roots")),
            "unexpected error: {error:?}"
        );

        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&other).ok();
    }

    #[test]
    fn empty_file_returns_empty_bytes_not_none() {
        let root = temp_dir("empty");
        let file = root.join("empty.dwg");
        fs::write(&file, []).expect("write empty xref");
        let source = FsXrefSource::new(vec![root.clone()]);

        let loaded = source.read(&file).expect("empty read is a success");
        let loaded = loaded.expect("an existing file is Some, never None");
        assert!(loaded.is_empty());

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn empty_roots_are_ignored() {
        let source = FsXrefSource::new(vec![PathBuf::new()]);
        let error = source
            .read(Path::new("ext.dwg"))
            .expect_err("no usable root means not found");
        assert!(
            matches!(&error, XrefSourceError::Missing(text) if text.contains("xref file not found")),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn no_source_reports_no_filesystem() {
        assert_eq!(
            NoXrefSource.read(Path::new("ext.dwg")).expect("no source"),
            None
        );
    }
}
