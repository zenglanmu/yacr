//! Third-party SHX test faces: located at run time, never at compile time.
//!
//! `complex.shx` (like the rest of the `mlightcad/cad-data` catalogue) is
//! third-party and **not committed** here — see `docs/fonts.md`. It is downloaded
//! by `scripts/fetch-fonts.sh` into generated directories (host packaging, the
//! Web/Android bundles) and into the shared `FONT_CACHE_DIR` cache, so a clean
//! clone has no copy on disk. A compile-time `include_bytes!` therefore made
//! `cargo check --all-targets` fail on a fresh checkout; these helpers read the
//! face only when a test actually runs, and the contract tests skip loudly (with
//! [`missing_face_hint`]) when this checkout never staged it.
//!
//! Stage the face into a generated directory with:
//!
//! ```text
//! FONTS=complex.shx bash scripts/fetch-fonts.sh apps/app-android/font-assets
//! ```

use std::env;
use std::fs;
use std::path::PathBuf;

/// The diagnosable skip message for a face this checkout never staged.
pub fn missing_face_hint(name: &str) -> String {
    format!(
        "{name} was not staged: this checkout has no copy of the third-party face, \
         so the SHX contract path is skipped. Stage it with \
         `FONTS={name} bash scripts/fetch-fonts.sh apps/app-android/font-assets` \
         (or point YACR_TEST_FONT_DIR at a directory that already holds it)."
    )
}

/// The shared download cache, matching `scripts/fetch-fonts.sh`'s default.
fn cache_dir() -> Option<PathBuf> {
    if let Some(dir) = env::var_os("FONT_CACHE_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    if let Some(dir) = env::var_os("XDG_CACHE_HOME") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir).join("yacr/fonts"));
        }
    }
    env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache/yacr/fonts"))
}

/// Candidate directories, in lookup order: an explicit test override first, then
/// the generated host-bundle directories, then the shared download cache.
fn candidate_dirs() -> Vec<PathBuf> {
    // `env!("CARGO_MANIFEST_DIR")` is `crates/cad-app`, so `../..` is the root.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut dirs = Vec::new();
    if let Some(dir) = env::var_os("YACR_TEST_FONT_DIR") {
        if !dir.is_empty() {
            dirs.push(PathBuf::from(dir));
        }
    }
    // A missing override keeps searching the defaults rather than panicking.
    dirs.push(root.join("apps/app-android/font-assets"));
    dirs.push(root.join("apps/app-web/dist-ui/fonts"));
    dirs.push(root.join("apps/desktop-ui/dist/fonts"));
    if let Some(dir) = cache_dir() {
        dirs.push(dir);
    }
    dirs
}

/// The bytes of `name` from the first candidate directory that holds a non-empty
/// file, or `None` when this checkout has not staged the face.
pub fn shx_face(name: &str) -> Option<Vec<u8>> {
    for dir in candidate_dirs() {
        if let Ok(bytes) = fs::read(dir.join(name)) {
            if !bytes.is_empty() {
                return Some(bytes);
            }
        }
    }
    None
}
