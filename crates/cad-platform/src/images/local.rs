//! Local (native) image loading shared by desktop hosts and the CLI.
//!
//! Kept out of wasm builds, which have no filesystem and resolve resources
//! through the browser network stack instead.
//!
//! Split out of `images.rs` (code-size gate): the public path is unchanged
//! (`cad_platform::images::local::*`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cad_domain::{CadError, CadResult};
use cad_resources::{
    decode_image, image_resource_key, is_safe_reference, DecodedImageCache, ResourceKey,
    ResourceLimits,
};

use super::*;

/// Candidate `images/` directories for an executable path, most specific
/// first: next to the binary, next to its `bin/` parent, under a macOS
/// `.app` bundle's `Contents/Resources/`, then the working directory.
///
/// Mirrors [`super::super::fonts::local::font_dir_candidates`] so a packaged
/// client finds a sibling image pack the same way it finds its fonts.
pub fn image_dir_candidates(exe: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = exe.parent() {
        out.push(dir.join("images"));
        if let Some(parent) = dir.parent() {
            out.push(parent.join("images"));
            // macOS `.app` layout: `Contents/MacOS/<exe>` with the image
            // package at `Contents/Resources/images`. Harmless on other
            // platforms (the path simply does not exist).
            out.push(parent.join("Resources").join("images"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        out.push(cwd.join("images"));
    }
    out
}

/// Resolve the image directory to use.
///
/// An explicit path must exist (a typo is an error, not a silent fallback);
/// otherwise the first existing candidate for `exe` is used, or `None`.
pub fn resolve_image_dir(explicit: Option<&Path>, exe: &Path) -> CadResult<Option<PathBuf>> {
    match explicit {
        Some(dir) => {
            if dir.is_dir() {
                Ok(Some(dir.to_path_buf()))
            } else {
                Err(CadError::InvalidInput(format!(
                    "image directory does not exist: {}",
                    dir.display()
                )))
            }
        }
        None => Ok(image_dir_candidates(exe)
            .into_iter()
            .find(|dir| dir.is_dir())),
    }
}

/// Resolve one logical image key to an existing file under `roots`.
///
/// The reference is validated with `cad-resources`' [`is_safe_reference`]
/// (absolute paths, Windows drive letters, URL schemes and `..` escapes are
/// rejected) before any filesystem access; the joined candidate must also
/// stay under its root as defence in depth. Returns `Ok(None)` when the key
/// is safe but no root holds the file, so the caller reports it unresolved.
///
/// The stored key is lowercased ([`image_resource_key`]) but the filesystem
/// may be case-sensitive, so after an exact miss each root is retried with a
/// **bounded, component-by-component case-insensitive scan**. The scan never
/// follows a symlink, never accepts `..`/empty path segments and only returns
/// a regular file, so it cannot escape the root.
pub fn resolve_image_file(roots: &[PathBuf], key: &str) -> CadResult<Option<PathBuf>> {
    if key.chars().any(char::is_control) || !is_safe_reference(key) {
        return Err(CadError::InvalidInput(format!(
            "image reference is not a safe relative key: {key}"
        )));
    }
    // Canonical relative components (empty and `.` dropped), matching the
    // normalisation `image_resource_key` applies to the cached key.
    let folded = key.replace('\\', "/");
    let components: Vec<&str> = folded
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();
    if components.is_empty() {
        return Err(CadError::InvalidInput(format!(
            "image reference is not a safe relative key: {key}"
        )));
    }
    for root in roots {
        let candidate = root.join(key);
        if !candidate.starts_with(root) {
            continue;
        }
        if candidate.is_file() {
            return Ok(Some(candidate));
        }
        if let Some(found) = case_insensitive_candidate(root, &components) {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

/// Resolve `components` under `root` by matching each path component
/// case-insensitively.
///
/// Only directory entries are considered (never a constructed path), a
/// symlink entry is skipped, and the final entry must be a regular file.
/// This keeps a hostile stored key from traversing `..`, a symlink or an
/// absolute path out of the allowed root.
fn case_insensitive_candidate(root: &Path, components: &[&str]) -> Option<PathBuf> {
    let mut current = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        let last = index + 1 == components.len();
        let mut matched = None;
        for entry in std::fs::read_dir(&current).ok()?.flatten() {
            let file_type = entry.file_type().ok()?;
            if file_type.is_symlink() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !name.eq_ignore_ascii_case(component) {
                continue;
            }
            if last && !file_type.is_file() {
                continue;
            }
            if !last && !file_type.is_dir() {
                continue;
            }
            matched = Some(entry.path());
            break;
        }
        current = matched?;
    }
    if current.starts_with(root) && current.is_file() {
        Some(current)
    } else {
        None
    }
}

/// Resolve, decode and cache the requested images.
///
/// `drawing_dir` is searched first (a drawing's own folder wins over an
/// explicit pack), then `images_dir`. Each key is reduced to the same
/// path-preserving [`image_resource_key`] the representation provider looks
/// up, so a decoded image actually becomes a texture primitive rather than an
/// outline-only frame and two files with the same base name stay distinct.
/// A missing/undecodable/over-budget key is recorded in the report and never
/// inserted as a placeholder.
///
/// This is the **file-only** pass. Use
/// [`load_image_cache_for_document`] whenever the database may own mesh
/// textures as well: this function cannot see them (they are not files).
pub fn load_image_cache(
    requested: &[String],
    drawing_dir: Option<&Path>,
    images_dir: Option<&Path>,
    limits: &ResourceLimits,
) -> CadResult<(Arc<DecodedImageCache>, ImageLoadReport)> {
    let roots = roots(drawing_dir, images_dir);
    let requested = dedup(requested);
    let mut cache = DecodedImageCache::new();
    let mut report = ImageLoadReport {
        requested: requested.clone(),
        ..ImageLoadReport::default()
    };
    resolve_files(&roots, &requested, &mut cache, &mut report, limits);
    report.bytes = cache.used_bytes();
    Ok((Arc::new(cache), report))
}

/// Resolve, decode and cache a document's images: its own textures first,
/// then the file-based references (ADR-0038 D10).
///
/// **Document textures** (`database.textures()`) are decoded from the
/// database bytes with the same [`ResourceLimits`] and inserted under
/// `ResourceKey(key)` — the exact key a textured `RenderBatch` carries, so
/// `Renderer::upload_mesh_textures` finds them. There is no filesystem access
/// and no safe-reference resolution for them: the bytes belong to the
/// document (D2/D3), so a key can only be missing because the document does
/// not carry it (a `cad-db` validation concern, not a path concern).
///
/// An undecodable or over-budget texture is recorded by key in
/// [`ImageLoadReport::failed`] and simply stays out of the cache; the load
/// itself still succeeds, because the renderer must be able to draw the rest
/// of the drawing untextured and report the missing key instead of failing
/// the whole open. Nothing is ever substituted by a placeholder image.
///
/// **File references** are then resolved exactly as
/// [`load_image_cache`] does, skipping any key the document already supplied
/// as a texture (so a document texture is never overwritten by a same-named
/// file). With no document textures the report is identical, item for item,
/// to [`load_image_cache`]'s.
///
/// The report's `requested` lists the document textures first (their decode
/// order) and then the file keys, deduplicated as before.
pub fn load_image_cache_for_document(
    database: &DrawingDatabase,
    requested: &[String],
    drawing_dir: Option<&Path>,
    images_dir: Option<&Path>,
    limits: &ResourceLimits,
) -> CadResult<(Arc<DecodedImageCache>, ImageLoadReport)> {
    let roots = roots(drawing_dir, images_dir);
    let mut cache = DecodedImageCache::new();
    let mut report = ImageLoadReport::default();

    // The document's own textures, first (D10), in key order.
    let mut document_keys: Vec<String> = Vec::new();
    for (key, bytes) in database.textures() {
        document_keys.push(key.to_string());
        report.requested.push(key.to_string());
        // `cad-db` already refuses a non-canonical key on insert, so this is
        // defence in depth for a database built by another path: an unsafe
        // key is reported and never inserted, but never guessed at either.
        if !cad_domain::is_valid_texture_key(key) {
            report
                .failed
                .push(format!("{key}: cad:texture.key: not a valid texture key"));
            continue;
        }
        match decode_image(bytes, limits) {
            Ok(image) => match cache.insert(ResourceKey(key.to_string()), image, limits) {
                Ok(()) => report.loaded.push(key.to_string()),
                Err(issue) => report.failed.push(format!("{key}: {}", issue.code)),
            },
            Err(issue) => report.failed.push(format!("{key}: {}", issue.code)),
        }
    }

    // File references, skipping a key the document already contributed. The
    // comparison also uses the normalised form (the cache identity), so a
    // differently spelled file reference can never replace a document
    // texture under the same `ResourceKey`.
    let requested = dedup(requested);
    let file_keys: Vec<String> = requested
        .into_iter()
        .filter(|key| !document_supplies(&document_keys, key))
        .collect();
    report.requested.extend(file_keys.iter().cloned());
    resolve_files(&roots, &file_keys, &mut cache, &mut report, limits);

    report.bytes = cache.used_bytes();
    Ok((Arc::new(cache), report))
}

/// Candidate roots, drawing directory first.
fn roots(drawing_dir: Option<&Path>, images_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(dir) = drawing_dir {
        roots.push(dir.to_path_buf());
    }
    if let Some(dir) = images_dir {
        roots.push(dir.to_path_buf());
    }
    roots
}

/// Whether the document's texture table already answers this file key, by
/// exact spelling or by the normalised `ResourceKey` identity.
fn document_supplies(document_keys: &[String], key: &str) -> bool {
    if document_keys.iter().any(|existing| existing == key) {
        return true;
    }
    image_resource_key(key).is_some_and(|resource| {
        document_keys
            .iter()
            .any(|existing| existing == resource.as_str())
    })
}

/// Resolve the file-based keys into `cache`, recording every outcome.
fn resolve_files(
    roots: &[PathBuf],
    keys: &[String],
    cache: &mut DecodedImageCache,
    report: &mut ImageLoadReport,
    limits: &ResourceLimits,
) {
    for key in keys {
        match resolve_image_file(roots, key) {
            Ok(Some(path)) => {
                let bytes = match std::fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        report.failed.push(format!("{key}: read failed: {error}"));
                        continue;
                    }
                };
                match decode_image(&bytes, limits) {
                    Ok(image) => {
                        // Key by the path-preserving image key so two files
                        // with the same base name stay distinct. The reference
                        // already passed `is_safe_reference` in
                        // `resolve_image_file`; a `None` here is defensive.
                        let Some(resource) = image_resource_key(key) else {
                            report.unresolved.push(key.clone());
                            continue;
                        };
                        match cache.insert(resource, image, limits) {
                            Ok(()) => report.loaded.push(key.clone()),
                            Err(issue) => {
                                report.failed.push(format!("{key}: {}", issue.code));
                            }
                        }
                    }
                    Err(issue) => report.failed.push(format!("{key}: {}", issue.code)),
                }
            }
            // Unsafe references are not attempts at all; both they and a
            // clean miss stay explicit unresolved keys.
            Ok(None) | Err(_) => report.unresolved.push(key.clone()),
        }
    }
}

fn dedup(keys: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for key in keys {
        if !out.iter().any(|existing| existing == key) {
            out.push(key.clone());
        }
    }
    out
}
