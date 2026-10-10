//! Host-side raster-image resolution: request → resolve → decode → cache.
//!
//! Spec v2.0 §7.3, §16.2: the importer emits a raster image's file as a
//! *logical* resource key (`SemanticGeometry::Image.file`), never a platform
//! path. The core (`cad-resources`) turns bytes into a decoded RGBA8
//! [`DecodedImage`]; the host is the only layer allowed to touch the
//! filesystem. This module holds the parts the desktop and CLI hosts share so
//! the two cannot drift apart:
//!
//! 1. [`requested_images`] reads, read-only, the image keys a drawing
//!    references.
//! 2. [`local::load_image_cache`] resolves each key under a drawing directory
//!    and an optional explicit images directory, reads the bytes, decodes them
//!    with `cad-resources` and inserts them into a [`DecodedImageCache`].
//!
//! Resolution is deliberately defensive: an absolute path, a `..` escape, a
//! URL or an empty key is rejected before any read (`is_safe_reference`), and a
//! joined candidate must stay under its root. A missing or undecodable key is
//! recorded verbatim in the [`ImageLoadReport`] and **never** replaced by a
//! placeholder image.

use cad_db::DrawingDatabase;
use cad_domain::SemanticGeometry;

/// Outcome of one image loading pass.
///
/// Failures are recorded verbatim instead of being folded into a success;
/// callers surface them so a missing image is visible in diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageLoadReport {
    /// Logical image keys the drawing asked for (deduplicated, in first-seen
    /// order).
    pub requested: Vec<String>,
    /// Keys successfully decoded and inserted into the cache.
    pub loaded: Vec<String>,
    /// Requested keys with no readable file under any candidate root, or with a
    /// reference that is unsafe to attempt at all.
    pub unresolved: Vec<String>,
    /// `<key>: <reason>` for each requested image that resolved to a file but
    /// could not be decoded or inserted (unsupported format, malformed bytes,
    /// over budget, read error).
    pub failed: Vec<String>,
    /// Decoded bytes currently held by the cache (RGBA8 payloads).
    pub bytes: usize,
}

impl ImageLoadReport {
    /// Whether no images were requested (the drawing references no rasters).
    pub fn is_empty(&self) -> bool {
        self.requested.is_empty() && self.loaded.is_empty() && self.failed.is_empty()
    }

    /// One-line, platform-neutral summary for diagnostics.
    pub fn summary(&self) -> String {
        format!(
            "requested={} loaded={} unresolved={} failed={} bytes={}",
            self.requested.len(),
            self.loaded.len(),
            self.unresolved.len(),
            self.failed.len(),
            self.bytes,
        )
    }
}

/// Raster-image keys a drawing references, read from the public database API.
///
/// This only collects the logical `file` key of `Image` geometry (recursing into
/// `Compound` sub-geometries). It performs no filesystem access and no path
/// resolution: a hostile drawing can therefore only supply names, which the
/// loader later rejects if they are not safe relative references. Names that
/// resolve to nothing remain explicit in the loading report.
///
/// The database models no image-kind `Style` table, so unlike [`super::fonts::requested_fonts`]
/// there are no `Style::resource_keys` to add here; if such a table is added the
/// collection must be extended in the same place.
pub fn requested_images(database: &DrawingDatabase) -> Vec<String> {
    let mut out = Vec::new();
    for entity in database.entities() {
        collect_images(&entity.geometry, &mut out);
    }
    out
}

fn collect_images(geometry: &SemanticGeometry, out: &mut Vec<String>) {
    match geometry {
        SemanticGeometry::Image {
            file: Some(file), ..
        } => {
            if !out.iter().any(|key| key == file) {
                out.push(file.clone());
            }
        }
        SemanticGeometry::Compound(parts) => {
            for part in parts {
                collect_images(part, out);
            }
        }
        _ => {}
    }
}

/// Local (native) image loading shared by desktop hosts and the CLI.
///
/// Kept out of wasm builds, which have no filesystem and resolve resources
/// through the browser network stack instead.
#[cfg(not(target_arch = "wasm32"))]
pub mod local {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use cad_domain::{CadError, CadResult};
    use cad_resources::{
        decode_image, image_resource_key, is_safe_reference, DecodedImageCache, ResourceLimits,
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
    pub fn resolve_image_file(roots: &[PathBuf], key: &str) -> CadResult<Option<PathBuf>> {
        if !is_safe_reference(key) {
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
        }
        Ok(None)
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
    pub fn load_image_cache(
        requested: &[String],
        drawing_dir: Option<&Path>,
        images_dir: Option<&Path>,
        limits: &ResourceLimits,
    ) -> CadResult<(Arc<DecodedImageCache>, ImageLoadReport)> {
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Some(dir) = drawing_dir {
            roots.push(dir.to_path_buf());
        }
        if let Some(dir) = images_dir {
            roots.push(dir.to_path_buf());
        }

        let requested = dedup(requested);
        let mut cache = DecodedImageCache::new();
        let mut report = ImageLoadReport {
            requested: requested.clone(),
            ..ImageLoadReport::default()
        };
        for key in &requested {
            match resolve_image_file(&roots, key) {
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
        report.bytes = cache.used_bytes();
        Ok((Arc::new(cache), report))
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_db::{DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
    use cad_domain::{DatabaseId, EntityId, LayerId, ObjectId, Point3, Revision, SpaceId};

    fn p(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    fn image_geometry(file: Option<&str>) -> SemanticGeometry {
        SemanticGeometry::Image {
            origin: p(0.0, 0.0),
            u: p(1.0, 0.0),
            v: p(0.0, 1.0),
            pixels: [10.0, 10.0],
            file: file.map(str::to_string),
            clip: None,
            visible: true,
        }
    }

    fn entity(id: u128, geometry: SemanticGeometry, draw_order: i64) -> DbEntity {
        DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbRasterImage".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(id),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry,
            draw_order,
        }
    }

    fn database_with_images() -> DrawingDatabase {
        let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
        builder
            .insert_layer(Layer {
                id: LayerId(0),
                name: "0".into(),
                visible: true,
            })
            .unwrap();
        builder
            .insert_entity(entity(1, image_geometry(Some("logo.png")), 0))
            .unwrap();
        // A second reference to the same file must be reported once.
        builder
            .insert_entity(entity(2, image_geometry(Some("logo.png")), 1))
            .unwrap();
        // A nested compound image is still collected.
        builder
            .insert_entity(entity(
                3,
                SemanticGeometry::Compound(vec![image_geometry(Some("nested.jpg"))]),
                2,
            ))
            .unwrap();
        // An image with no key contributes nothing.
        builder
            .insert_entity(entity(4, image_geometry(None), 3))
            .unwrap();
        builder.finish().unwrap()
    }

    #[test]
    fn collects_deduplicated_image_file_keys_including_compounds() {
        let db = database_with_images();
        assert_eq!(
            requested_images(&db),
            vec!["logo.png".to_string(), "nested.jpg".to_string()]
        );
    }

    #[test]
    fn image_report_summary_is_stable() {
        let report = ImageLoadReport {
            requested: vec!["a.png".into()],
            loaded: vec!["a.png".into()],
            unresolved: Vec::new(),
            failed: Vec::new(),
            bytes: 16,
        };
        assert!(!report.is_empty());
        assert_eq!(
            report.summary(),
            "requested=1 loaded=1 unresolved=0 failed=0 bytes=16"
        );
        assert!(ImageLoadReport::default().is_empty());
    }

    /// A 2x2 RGBA PNG, generated so the test needs no committed binary fixture.
    fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(rgba).unwrap();
        }
        out
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn scratch(label: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("yacr-image-test-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn valid_png_decodes_and_lands_in_the_cache_under_its_sanitized_key() {
        use cad_resources::{ResourceKey, ResourceLimits};
        let dir = scratch("valid");
        let pixels: Vec<u8> = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 10, 20, 30, 255,
        ];
        std::fs::write(dir.join("Logo.PNG"), encode_png(2, 2, &pixels)).unwrap();

        let (cache, report) = local::load_image_cache(
            &["Logo.PNG".to_string()],
            Some(&dir),
            None,
            &ResourceLimits::default(),
        )
        .unwrap();

        assert_eq!(report.requested, vec!["Logo.PNG".to_string()]);
        assert_eq!(report.loaded, vec!["Logo.PNG".to_string()]);
        assert!(report.unresolved.is_empty());
        assert!(report.failed.is_empty());
        assert_eq!(report.bytes, 16);
        let key = ResourceKey::sanitize("Logo.PNG");
        assert_eq!(key.as_str(), "logo.png");
        let decoded = cache.get(&key).expect("decoded image is cached");
        assert_eq!((decoded.width, decoded.height), (2, 2));
        assert_eq!(&decoded.rgba[0..4], &[255, 0, 0, 255]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn missing_file_is_reported_and_never_inserted() {
        use cad_resources::ResourceLimits;
        let dir = scratch("missing");
        let (cache, report) = local::load_image_cache(
            &["absent.png".to_string()],
            Some(&dir),
            None,
            &ResourceLimits::default(),
        )
        .unwrap();
        assert_eq!(report.unresolved, vec!["absent.png".to_string()]);
        assert!(report.loaded.is_empty());
        assert!(report.failed.is_empty());
        assert!(cache.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn undecodable_file_is_reported_as_failed_not_loaded() {
        use cad_resources::ResourceLimits;
        let dir = scratch("undecodable");
        std::fs::write(dir.join("broken.png"), b"not really a png").unwrap();
        let (cache, report) = local::load_image_cache(
            &["broken.png".to_string()],
            Some(&dir),
            None,
            &ResourceLimits::default(),
        )
        .unwrap();
        assert!(report.unresolved.is_empty());
        assert!(report.loaded.is_empty());
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].starts_with("broken.png: "));
        assert!(cache.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn path_escape_is_rejected_before_any_read() {
        use cad_resources::ResourceLimits;
        let parent = scratch("escape");
        let drawing = parent.join("drawing");
        std::fs::create_dir_all(&drawing).unwrap();
        // A real file outside the drawing directory that must never be loaded.
        std::fs::write(parent.join("secret.png"), encode_png(1, 1, &[9, 9, 9, 255])).unwrap();

        let (cache, report) = local::load_image_cache(
            &["../secret.png".to_string(), "/etc/passwd".to_string()],
            Some(&drawing),
            None,
            &ResourceLimits::default(),
        )
        .unwrap();
        assert_eq!(
            report.unresolved,
            vec!["../secret.png".to_string(), "/etc/passwd".to_string()]
        );
        assert!(report.loaded.is_empty());
        assert!(cache.is_empty());
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn drawing_directory_wins_over_the_explicit_images_directory() {
        use cad_resources::{ResourceKey, ResourceLimits};
        let drawing = scratch("root-drawing");
        let pack = scratch("root-pack");
        // Same logical key in both roots: the drawing-relative file must win.
        std::fs::write(drawing.join("logo.png"), encode_png(1, 1, &[1, 2, 3, 255])).unwrap();
        std::fs::write(pack.join("logo.png"), encode_png(1, 1, &[4, 5, 6, 255])).unwrap();

        let (cache, report) = local::load_image_cache(
            &["logo.png".to_string()],
            Some(&drawing),
            Some(&pack),
            &ResourceLimits::default(),
        )
        .unwrap();
        assert_eq!(report.loaded, vec!["logo.png".to_string()]);
        let decoded = cache
            .get(&ResourceKey::sanitize("logo.png"))
            .expect("cached");
        assert_eq!(&decoded.rgba[0..4], &[1, 2, 3, 255]);
        let _ = std::fs::remove_dir_all(&drawing);
        let _ = std::fs::remove_dir_all(&pack);
    }

    /// Two references with the same base name but different directories must
    /// produce distinct keys and both survive in the cache (R6: no silent
    /// collision).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn same_base_name_in_two_directories_keeps_distinct_keys() {
        use cad_resources::{image_resource_key, ResourceLimits};
        let dir = scratch("collision");
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::create_dir_all(dir.join("b")).unwrap();
        std::fs::write(
            dir.join("a").join("logo.png"),
            encode_png(1, 1, &[1, 2, 3, 255]),
        )
        .unwrap();
        std::fs::write(
            dir.join("b").join("logo.png"),
            encode_png(1, 1, &[4, 5, 6, 255]),
        )
        .unwrap();

        let (cache, report) = local::load_image_cache(
            &["a/logo.png".to_string(), "b/logo.png".to_string()],
            Some(&dir),
            None,
            &ResourceLimits::default(),
        )
        .unwrap();

        assert_eq!(
            report.loaded,
            vec!["a/logo.png".to_string(), "b/logo.png".to_string()]
        );
        assert_eq!(cache.len(), 2, "both textures must be cached separately");
        let a = image_resource_key("a/logo.png").unwrap();
        let b = image_resource_key("b/logo.png").unwrap();
        assert_ne!(a, b);
        assert_eq!(&cache.get(&a).unwrap().rgba[0..4], &[1, 2, 3, 255]);
        assert_eq!(&cache.get(&b).unwrap().rgba[0..4], &[4, 5, 6, 255]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn candidates_prefer_the_binary_side_images_dir() {
        let exe = std::path::Path::new("/opt/yacr/bin/yacr-linux");
        let candidates = local::image_dir_candidates(exe);
        assert_eq!(
            candidates[0],
            std::path::PathBuf::from("/opt/yacr/bin/images")
        );
        assert_eq!(candidates[1], std::path::PathBuf::from("/opt/yacr/images"));
        let app_exe = std::path::Path::new("/opt/Yacr.app/Contents/MacOS/yacr-macos");
        let app_candidates = local::image_dir_candidates(app_exe);
        assert_eq!(
            app_candidates[2],
            std::path::PathBuf::from("/opt/Yacr.app/Contents/Resources/images")
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn explicit_missing_image_dir_is_an_error_not_a_silent_fallback() {
        let missing = std::path::Path::new("/definitely/not/an/image/dir");
        let result =
            local::resolve_image_dir(Some(missing), std::path::Path::new("/opt/yacr/bin/yacr"));
        assert!(matches!(result, Err(cad_domain::CadError::InvalidInput(_))));
    }
}
